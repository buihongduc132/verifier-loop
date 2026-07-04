//! Verifier spawn orchestration + session reuse (tasks.md §5, §6; verifier-spawn spec).
//!
//! §5 — [`spawn_round`] launches `m` verifier processes **concurrently** (every child is
//! spawned via `tokio::process::Command::spawn` *before* any is awaited — so no launch
//! blocks another), pre-creates each `rounds/<round>/<verifierId>/verdict.json`
//! `{status:null}` + `meta.json` `{sid, turnsUsed}` (D9 fail-closed), injects the identity
//! env vars (`VERIFIER_LOOP_GOAL_ID` / `_VERIFIER_ID` / `_ROUND`, D2), enforces a
//! per-verifier `verifierTimeoutSec` kill → null verdict (D9), and gathers all runs at a
//! barrier (D7).
//!
//! §6 — [`spawn_resume`] decides per verifier whether to reuse the prior SID
//! (`turnsUsed < maxTurn` → adapter resume command with `--session <sid>`) or spawn fresh
//! (`turnsUsed >= maxTurn` → fresh command + prior SID archived under its originating
//! round). Round increments; verifierId stays stable (D8).
//!
//! Output parsing is delegated to the shared §4 ACP parser ([`crate::acp`]); only the
//! command rendering + process lifecycle live here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::acp;
use crate::crypto;
use crate::goal;
use crate::store;
use crate::verdict;

/// Subdirectory name for a verifier id under a round directory.
/// Verifier ids are `v1`, `v2`, … `v{m}` (spec: "v1, v2, ...").
fn verifier_id(idx: usize) -> String {
    format!("v{}", idx + 1)
}

/// `verdict.json` written at spawn time — pre-created as `{status:null}` (D9).
pub const VERDICT_FILE: &str = "verdict.json";
/// `meta.json` written at spawn time — `{sid, turnsUsed}` (spec).
pub const META_FILE: &str = "meta.json";
/// `final-output.txt` written after gather if the verifier emitted an `agent_end`.
pub const FINAL_OUTPUT_FILE: &str = "final-output.txt";
/// `archive.json` written under a prior round dir when a session is freshly respawned
/// (§6): records the superseded SID for audit.
pub const ARCHIVE_FILE: &str = "archive.json";

/// Identity env var names injected into every verifier process (D2).
pub const ENV_GOAL_ID: &str = "VERIFIER_LOOP_GOAL_ID";
pub const ENV_VERIFIER_ID: &str = "VERIFIER_LOOP_VERIFIER_ID";
pub const ENV_ROUND: &str = "VERIFIER_LOOP_ROUND";
/// Store-root override propagated to spawned verifiers so `verifier-verdict`
/// (jewije) registers its verdict into the *same* store the orchestrator used —
/// otherwise jewije resolves its own default `$HOME/.verifier-loop` and the verdict
/// write lands in the wrong store, leaving the slot null (no consensus → no hash).
pub const ENV_HOME: &str = "VERIFIER_LOOP_HOME";
/// Per-verifier signing secret (hex) injected so the verifier backend's `jewije`
/// call can register a SIGNED verdict bound to the slot's pinned pubkey (D3,
/// verifier-spawn MODIFIED). The secret is minted by `verdict::mint_and_pin_pubkey`
/// at spawn time and is NEVER persisted to disk by the orchestrator.
pub const ENV_VERIFIER_SECRET: &str = "VERIFIER_LOOP_VERIFIER_SECRET";
/// Path to the `verifier-verdict` (jewije) binary the stub backend should invoke.
/// Spawn resolves this as the sibling of the running `verifier-loop` exe so a stub
/// calling bare `verifier-verdict` from PATH cannot pick up a stale/global install
/// (which would lack the signed-verdict regime and silently produce unsigned
/// verdicts). The stub falls back to PATH lookup if this is unset.
pub const ENV_VERDICT_BIN: &str = "VERIFIER_LOOP_VERDICT_BIN";

/// Inputs to a spawn round. All borrowed; the round is driven to completion synchronously.
#[derive(Debug, Clone, Copy)]
pub struct SpawnInput<'a> {
    pub root: &'a Path,
    pub goal_id: &'a str,
    pub round: u32,
    pub config: &'a store::Config,
    pub prompt: &'a str,
    pub adapter: &'a acp::Adapter,
}

/// A completed verifier run (after the gather barrier).
#[derive(Debug, Clone)]
pub struct VerifierRun {
    pub verifier_id: String,
    /// SID captured from the ACP `session` event, if any. `None` on timeout or missing.
    pub sid: Option<String>,
    /// Final assistant message captured from `agent_end`, if any.
    pub final_output: Option<String>,
    /// True iff the verifier was killed by `verifierTimeoutSec`.
    pub timed_out: bool,
}

/// On-disk per-verifier metadata, written at spawn time and updated after gather.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifierMeta {
    /// `null` until the ACP `session` event is parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    /// Turn budget consumed so far. `0` at pre-create; `1` after a fresh spawn,
    /// `prior + 1` after a reused resume (v1 heuristic — OT2 per-turn refresh deferred).
    pub turns_used: u32,
}

/// Errors raised by the spawn layer. Every path fails closed (D9): a timeout is **not**
/// an error — it is reported as a [`VerifierRun`] with `timed_out = true` and a null
/// verdict left on disk.
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("goal layer error: {0}")]
    Goal(#[from] goal::GoalError),
    #[error("acp parse error: {0}")]
    Acp(#[from] acp::AcpError),
    #[error("verdict layer error: {0}")]
    Verdict(#[from] crate::verdict::VerdictError),
}

// ---------------------------------------------------------------------------
// §5 — fresh spawn round
// ---------------------------------------------------------------------------

/// Spawn `m` verifiers concurrently for a fresh round, gather them, and return their runs.
///
/// Pre-creates `rounds/<round>/<vN>/{verdict.json {status:null}, meta.json}` for each
/// verifier before launching. All processes are launched (via `Command::spawn`) before any
/// is awaited, so launches are non-blocking relative to each other (D7). The function
/// returns only after every process has either completed or timed out (gather barrier).
pub async fn spawn_round(input: SpawnInput<'_>) -> Result<Vec<VerifierRun>, SpawnError> {
    let rounds_dir = round_dir(input.root, input.goal_id, input.round);
    fs::create_dir_all(&rounds_dir)?;

    // Build the launch plan: (verifierId, command, vdir) for each of m verifiers.
    let mut plan: Vec<(String, Command, PathBuf)> = Vec::new();
    for i in 0..input.config.m as usize {
        let vid = verifier_id(i);
        let vdir = rounds_dir.join(&vid);
        fs::create_dir_all(&vdir)?;
        pre_create_verifier_dir(&vdir);

        let mut cmd = build_spawn_command(&input.adapter.spawn, input.prompt);
        let secret_hex = mint_verifier_secret(input.root, input.goal_id, &vid, input.round)?;
        inject_identity_env(
            &mut cmd,
            input.goal_id,
            &vid,
            input.round,
            input.root,
            &secret_hex,
        );
        inject_verifier_verdict_bin(&mut cmd);
        plan.push((vid, cmd, vdir));
    }

    // Launch every child BEFORE awaiting any (non-blocking spawn). Each `spawn()` starts
    // the OS process immediately; awaiting is the gather barrier.
    let mut children: Vec<(String, tokio::process::Child, PathBuf)> = Vec::new();
    for (vid, mut cmd, vdir) in plan {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = cmd.spawn()?;
        children.push((vid, child, vdir));
    }

    gather(input, children).await
}

// ---------------------------------------------------------------------------
// §6 — resume round (reuse SID or fresh spawn + archive)
// ---------------------------------------------------------------------------

/// Resume a round: per verifier, reuse the prior SID (`turnsUsed < maxTurn`) via the
/// adapter resume command, else spawn fresh and archive the prior SID.
///
/// Reads each verifier's prior-round `meta.json` to decide. Round env increments;
/// verifierId is stable across rounds (D8).
pub async fn spawn_resume(input: SpawnInput<'_>) -> Result<Vec<VerifierRun>, SpawnError> {
    debug_assert!(
        input.round >= 1,
        "spawn_resume requires round >= 1 (prev round = round-1)"
    );
    let prev_round = input.round.saturating_sub(1);
    let rounds_dir = round_dir(input.root, input.goal_id, input.round);
    fs::create_dir_all(&rounds_dir)?;

    let mut plan: Vec<(String, Command, PathBuf)> = Vec::new();
    for i in 0..input.config.m as usize {
        let vid = verifier_id(i);
        let vdir = rounds_dir.join(&vid);
        fs::create_dir_all(&vdir)?;

        let prev_vdir = round_dir(input.root, input.goal_id, prev_round).join(&vid);
        let prior = read_meta(&prev_vdir)?;

        let mut cmd;
        let fresh;
        match &prior {
            Some(meta) if meta.turns_used < input.config.max_turn => {
                // Reuse: resume on the prior SID.
                let sid = meta.sid.clone().unwrap_or_default();
                cmd = build_resume_command(&input.adapter.resume, &sid, input.prompt);
                fresh = false;
            }
            _ => {
                // Fresh (exhausted, or no prior meta). Archive the prior SID if present.
                if let Some(meta) = prior.as_ref() {
                    if let Some(sid) = &meta.sid {
                        archive_prior_sid(&prev_vdir, sid)?;
                    }
                }
                cmd = build_spawn_command(&input.adapter.spawn, input.prompt);
                fresh = true;
            }
        }

        // The new round's meta starts at null SID / turnsUsed=0; updated after gather.
        // For a reused session we keep the prior turnsUsed as the baseline so the next
        // resume decision sees the running total.
        let baseline_turns = if fresh {
            0
        } else {
            prior.as_ref().map(|m| m.turns_used).unwrap_or(0)
        };
        pre_create_verifier_dir_with_turns(&vdir, baseline_turns);

        let secret_hex = mint_verifier_secret(input.root, input.goal_id, &vid, input.round)?;
        inject_identity_env(
            &mut cmd,
            input.goal_id,
            &vid,
            input.round,
            input.root,
            &secret_hex,
        );
        inject_verifier_verdict_bin(&mut cmd);
        plan.push((vid, cmd, vdir));
    }

    let mut children: Vec<(String, tokio::process::Child, PathBuf)> = Vec::new();
    for (vid, mut cmd, vdir) in plan {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = cmd.spawn()?;
        children.push((vid, child, vdir));
    }

    gather(input, children).await
}

// ---------------------------------------------------------------------------
// gather barrier (shared by spawn_round + spawn_resume)
// ---------------------------------------------------------------------------

/// Await every launched child with a per-verifier timeout (D9). On timeout the child is
/// killed and the run is marked `timed_out`; the pre-created null verdict is left in
/// place. Captured stdout is parsed for the SID + final output, and `meta.json` +
/// `final-output.txt` are updated accordingly.
async fn gather(
    input: SpawnInput<'_>,
    mut children: Vec<(String, tokio::process::Child, PathBuf)>,
) -> Result<Vec<VerifierRun>, SpawnError> {
    let timeout = Duration::from_secs(input.config.verifier_timeout_sec.max(1));
    let mut runs = Vec::with_capacity(children.len());

    for (vid, mut child, vdir) in children.drain(..) {
        // Take the stdout pipe out of the child and drain it on a background task.
        // Draining concurrently with `wait()` is required because an OS pipe holds only
        // ~64KB: a verifier emitting MBs of ACP events would fill the buffer, block on
        // write, and either hang to the timeout or exit without emitting `agent_end` —
        // leaving a null verdict despite a successful run.
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let drain = tokio::spawn(async move {
            // Best-effort stderr drain so a chatty backend never blocks on a full pipe.
            if let Some(mut p) = stderr_pipe {
                use tokio::io::AsyncReadExt;
                let mut sink = Vec::<u8>::new();
                let _ = p.read_to_end(&mut sink).await;
            }
            let buf = match stdout_pipe {
                Some(mut pipe) => {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    let _ = pipe.read_to_end(&mut buf).await;
                    buf
                }
                None => Vec::new(),
            };
            buf
        });

        let run = tokio::select! {
            biased;
            _ = tokio::time::sleep(timeout) => {
                // Timeout (D9): kill, reap, leave null verdict. No SID / final output.
                let _ = child.start_kill();
                let _ = child.wait().await;
                VerifierRun { verifier_id: vid, sid: None, final_output: None, timed_out: true }
            }
            status = child.wait() => {
                // Child exited; the drain task finishes shortly after (pipe hits EOF).
                let buf = drain.await.unwrap_or_default();
                let stdout = String::from_utf8_lossy(&buf);
                let sid = acp::extract_sid(&stdout);
                let final_output = acp::extract_final_output(&stdout);
                if let Some(text) = &final_output {
                    let _ = fs::write(vdir.join(FINAL_OUTPUT_FILE), text);
                }
                // Status failures still parse whatever stdout was emitted; the null
                // verdict is left in place if no SID/output was captured (fail-closed).
                let _ = status;
                update_meta_after_run(&vdir, sid.as_deref(), &input)?;
                VerifierRun {
                    verifier_id: vid,
                    sid,
                    final_output,
                    timed_out: false,
                }
            }
        };
        runs.push(run);
    }

    Ok(runs)
}

// ---------------------------------------------------------------------------
// helpers — paths, files, env, command building
// ---------------------------------------------------------------------------

/// `goals/<goal_id>/rounds/<round>`.
fn round_dir(root: &Path, goal_id: &str, round: u32) -> PathBuf {
    goal::goal_dir(root, goal_id)
        .join(goal::ROUNDS_DIR)
        .join(round.to_string())
}

/// Write `verdict.json {status:null}` + `meta.json {sid:null, turnsUsed:0}`.
/// Idempotent and best-effort: a pre-existing verdict is NOT overwritten (first-write
/// semantics live in the verdict layer; here we only ensure the null baseline exists).
fn pre_create_verifier_dir(vdir: &Path) {
    pre_create_verifier_dir_with_turns(vdir, 0);
}

fn pre_create_verifier_dir_with_turns(vdir: &Path, baseline_turns: u32) {
    let verdict_path = vdir.join(VERDICT_FILE);
    if !verdict_path.exists() {
        let _ = fs::write(
            &verdict_path,
            serde_json::json!({ "status": serde_json::Value::Null }).to_string(),
        );
    }
    let meta = VerifierMeta {
        sid: None,
        turns_used: baseline_turns,
    };
    let _ = fs::write(
        vdir.join(META_FILE),
        serde_json::to_string(&meta).unwrap_or_else(|_| "{}".into()),
    );
}

/// Read the prior round's `meta.json`, if present (used by resume).
fn read_meta(vdir: &Path) -> Result<Option<VerifierMeta>, SpawnError> {
    let path = vdir.join(META_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    Ok(Some(serde_json::from_str(&raw)?))
}

/// Update `meta.json` after a run with the captured SID and an incremented turn count.
fn update_meta_after_run(
    vdir: &Path,
    sid: Option<&str>,
    input: &SpawnInput<'_>,
) -> Result<(), SpawnError> {
    let existing = read_meta(vdir)?.unwrap_or(VerifierMeta {
        sid: None,
        turns_used: 0,
    });
    let turns_used = existing.turns_used.saturating_add(1).min(input.config.max_turn);
    let updated = VerifierMeta {
        sid: sid.map(str::to_string),
        turns_used,
    };
    fs::write(
        vdir.join(META_FILE),
        serde_json::to_string_pretty(&updated)?,
    )?;
    Ok(())
}

/// Archive a superseded SID under its originating round directory (§6).
fn archive_prior_sid(prev_vdir: &Path, sid: &str) -> Result<(), SpawnError> {
    let archive = serde_json::json!({
        "sid": sid,
        "archivedAt": Utc::now().to_rfc3339(),
        "reason": "maxTurn reached; session spawned fresh"
    });
    fs::write(
        prev_vdir.join(ARCHIVE_FILE),
        serde_json::to_string_pretty(&archive)?,
    )?;
    Ok(())
}

/// Split a rendered command string into argv and build a `tokio::process::Command`.
///
/// Splitting on whitespace matches the §4 contract ("the orchestrator splits the rendered
/// command on whitespace"). The first token is the program; the rest are args. Prompts
/// needing shell quoting are the caller's responsibility (D2 note in adapters.rs).
/// Parse a rendered command string into a `Command`.
///
/// Adapter templates embed the prompt via `{prompt}` substitution inside shell quotes
/// (e.g. `pi -p "{prompt}" --mode json`). The prompt is arbitrary multi-KB text with
/// spaces, newlines, and quotes, so naive `split_whitespace()` would shatter it into
/// thousands of args. We delegate to `sh -c` so the shell handles quoting correctly.
fn build_spawn_command(template: &str, prompt: &str) -> Command {
    let (left, right) = template.split_once("{prompt}")
        .unwrap_or((template, ""));
    let mut left_parts = left.split_whitespace();
    let program = left_parts.next().expect("spawn template has a non-empty program");
    let mut cmd = Command::new(program);
    for a in left_parts {
        cmd.arg(a);
    }
    cmd.arg(prompt);
    for a in right.split_whitespace() {
        cmd.arg(a);
    }
    cmd
}

fn build_resume_command(template: &str, sid: &str, prompt: &str) -> Command {
    let with_sid = template.replace("{sid}", sid);
    build_spawn_command(&with_sid, prompt)
}

/// Mint a fresh Ed25519 keypair for the verifier slot and pin its verifying key into
/// `verifier-pubkey.json` (verifier-spawn MODIFIED D3). Returns the secret signing key
/// encoded as hex so it can be injected into the verifier process env.
///
/// First-write-wins: a pinned slot is left untouched. Spawn is the first caller for a
/// fresh slot, so a prior pin only exists on resume across rounds (a new slot dir per
/// round means this is only an error if the same round is spawned twice, which the
/// orchestrator never does).
fn mint_verifier_secret(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
) -> Result<String, SpawnError> {
    let sk = verdict::mint_and_pin_pubkey(root, goal_id, verifier_id, round)?;
    Ok(crypto::signing_key_to_hex(&sk))
}

/// The env pairs a spawned verifier needs: identity (D2) plus the store root so its
/// `verifier-verdict` call writes into the orchestrator's store (fail-closed: a
/// verdict written to a *different* store would be invisible → null slot → no hash),
/// plus the per-verifier signing secret (D3) so its verdict registration is signed.
fn identity_env_pairs<'a>(
    goal_id: &'a str,
    verifier_id: &'a str,
    round: u32,
    root: &'a Path,
    secret_hex: &'a str,
) -> Vec<(&'static str, std::ffi::OsString)> {
    vec![
        (ENV_GOAL_ID, goal_id.into()),
        (ENV_VERIFIER_ID, verifier_id.into()),
        (ENV_ROUND, round.to_string().into()),
        (ENV_HOME, root.as_os_str().into()),
        (ENV_VERIFIER_SECRET, secret_hex.into()),
    ]
}

/// Inject the identity + store-root + signing-secret env vars into a verifier command
/// (D2 + D3).
fn inject_identity_env(
    cmd: &mut Command,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
    root: &Path,
    secret_hex: &str,
) {
    for (k, v) in identity_env_pairs(goal_id, verifier_id, round, root, secret_hex) {
        cmd.env(k, v);
    }
}

/// Resolve the `verifier-verdict` (jewije) binary that ships beside the running
/// `verifier-loop` exe, and inject its absolute path so the stub backend invokes the
/// matching build (not a stale/global `verifier-verdict` on PATH). Best-effort: if the
/// sibling cannot be resolved (e.g. the orchestrator runs embedded outside a CLI exe),
/// the env var is left unset and the stub falls back to a PATH lookup.
fn inject_verifier_verdict_bin(cmd: &mut Command) {
    if let Some(bin) = sibling_verifier_verdict() {
        cmd.env(ENV_VERDICT_BIN, &bin);
    }
}

/// Locate `verifier-verdict` next to the current executable. Returns `None` if the
/// current exe cannot be resolved, has no parent, or the sibling is absent.
fn sibling_verifier_verdict() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join("verifier-verdict");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_ids_are_one_indexed_v_prefix() {
        assert_eq!(verifier_id(0), "v1");
        assert_eq!(verifier_id(2), "v3");
    }

    #[test]
    fn build_spawn_command_passes_prompt_as_single_arg_without_shell() {
        let template = "pi -p {prompt} --mode json";
        let prompt = "hello `world` $(rm -rf /) \"quoted\"";
        let cmd = build_spawn_command(template, prompt);
        let s = format!("{:?}", cmd.as_std());
        assert!(s.contains("pi"), "program is pi");
        assert!(!s.contains("sh"), "must NOT use sh");
        // The shell-unsafe substring `$(rm -rf /)` must survive inside ONE arg. If the
        // prompt were split on spaces, "$(rm" and "-rf" would be separate args. The
        // Debug format quotes each arg, so the whole prompt appears as one quoted unit.
        assert!(s.contains("$(rm -rf /)"), "prompt body intact as single arg: {s}");
        assert!(s.contains("--mode"), "post-args preserved");
    }

    #[test]
    fn build_resume_command_substitutes_sid_then_prompt() {
        let template = "pi --session {sid} -p {prompt} --mode json";
        let cmd = build_resume_command(template, "abc-123", "hello world");
        let s = format!("{:?}", cmd.as_std());
        assert!(s.contains("abc-123"), "sid substituted: {s}");
        assert!(s.contains("hello world"), "prompt intact: {s}");
    }

    #[test]
    fn meta_round_trips_camel_case() {
        let m = VerifierMeta {
            sid: Some("s".into()),
            turns_used: 2,
        };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"turnsUsed\":2"), "{j}");
        let back: VerifierMeta = serde_json::from_str(&j).unwrap();
        assert_eq!(back.sid.as_deref(), Some("s"));
        assert_eq!(back.turns_used, 2);
    }

    #[test]
    fn meta_without_sid_serializes_without_key() {
        let m = VerifierMeta {
            sid: None,
            turns_used: 0,
        };
        let j = serde_json::to_string(&m).unwrap();
        assert!(!j.contains("sid"), "null sid should be skipped: {j}");
    }

    #[test]
    fn identity_env_pairs_propagate_store_root() {
        let root = Path::new("/tmp/vl-home");
        let pairs = identity_env_pairs("g1", "v1", 2, root, "deadbeef");
        let home = pairs.iter().find(|(k, _)| *k == ENV_HOME);
        assert!(home.is_some(), "VERIFIER_LOOP_HOME must be injected");
        assert_eq!(
            home.unwrap().1.as_os_str(),
            Path::new("/tmp/vl-home").as_os_str(),
            "injected HOME must equal the orchestrator's root"
        );
        // identity vars still present
        assert!(pairs.iter().any(|(k, _)| *k == ENV_GOAL_ID));
        assert!(pairs.iter().any(|(k, _)| *k == ENV_VERIFIER_ID));
        assert!(pairs.iter().any(|(k, _)| *k == ENV_ROUND));
        // D3: per-verifier signing secret is injected.
        let secret = pairs.iter().find(|(k, _)| *k == ENV_VERIFIER_SECRET);
        assert!(secret.is_some(), "VERIFIER_LOOP_VERIFIER_SECRET must be injected");
        assert_eq!(
            secret.unwrap().1.as_os_str(),
            "deadbeef",
            "injected secret must equal the minted signing key hex"
        );
    }

    #[test]
    fn round_dir_layout_matches_goal_layer() {
        let root = Path::new("/tmp/x");
        let d = round_dir(root, "g1", 2);
        assert_eq!(
            d,
            Path::new("/tmp/x/goals/g1/rounds/2")
        );
    }
}
