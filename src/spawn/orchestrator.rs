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
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::acp;
use crate::acp::Transport;
use crate::crypto;
use crate::goal;
use crate::spawn::tempfile::TempPromptFile;
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
/// Per-verifier captured stderr filename. Written whenever the backend emitted any
/// stderr (success or crash) so the user can always inspect why a run failed closed.
pub const STDERR_FILE: &str = "stderr.txt";
/// Maximum bytes of backend stderr retained in RAM + persisted to `stderr.txt`.
/// Only the diagnostic TAIL matters (error messages live at the end of a run), so we
/// keep a bounded tail instead of buffering an unbounded chatty backend into RAM.
/// A run exceeding this is truncated with a `[...truncated N bytes...]` marker.
pub const STDERR_CAP_BYTES: usize = 8 * 1024;
/// `archive.json` written under a prior round dir when a session is freshly respawned
/// (§6): records the superseded SID for audit.
pub const ARCHIVE_FILE: &str = "archive.json";

/// A spawned verifier child + its metadata, used by [`gather`]. Factored into a type
/// alias to keep clippy's `type_complexity` lint happy and the gather signature readable.
type SpawnedChild = (
    String,
    tokio::process::Child,
    PathBuf,
    Option<JoinHandle<io::Result<()>>>,
    Option<TempPromptFile>,
);

/// Minimal verdict-nudge prompt used for within-round verdict enforcement (D5). Contains
/// NO goal/diff/policy text; the resumed session already holds that context. Kept tiny
/// (<2KB) so it never re-triggers compaction.
pub const VERDICT_NUDGE_PROMPT: &str = "You have completed your investigation. Register your verdict NOW via the verifier-verdict CLI:\n\nverifier-verdict approve --notes \"evidence summary: ...\"\nverifier-verdict reject --notes \"what is wrong, with file:line references\"\n\nNo further investigation is needed. Register your verdict.\n";

/// Compaction-aware recovery nudge prompt used for compaction recovery (D6). Contains
/// NO goal/diff/policy text (preserved in the compacted session). Tells the verifier that
/// compaction occurred, its prior investigation is preserved, and it must register the
/// verdict immediately. Kept tiny (<2KB).
pub const COMPACTION_RECOVERY_NUDGE_PROMPT: &str = "The session compacted after your investigation. Your prior reasoning and artifacts are preserved in this resumed session.\n\nRegister your verdict NOW via the verifier-verdict CLI:\n\nverifier-verdict approve --notes \"evidence summary: ...\"\nverifier-verdict reject --notes \"what is wrong, with file:line references\"\n\nNo further investigation is needed. Register your verdict.\n";

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
    /// dynamic-pipeline: override verifier count (defaults to config.m). Used per-phase
    /// to spawn confirmCount (Confirm) or floor/ceil(m/2) (Mixed) verifiers.
    pub verifier_count: Option<usize>,
    /// dynamic-pipeline: override verifier id prefix (defaults to "v"). "d" for dump
    /// phases, "s" for smart phases (LD16).
    pub id_prefix: Option<&'a str>,
    /// dynamic-pipeline: starting index for verifier ids (defaults to 0). Monotonic
    /// continuation so Mixed dump starts at m+1 (LD26).
    pub id_offset: usize,
}

/// A completed verifier run (after the gather barrier).
#[derive(Debug, Clone)]
pub struct VerifierRun {
    pub verifier_id: String,
    /// SID captured from the ACP `session` event, if any. `None` on timeout or missing.
    pub sid: Option<String>,
    /// Final assistant message captured from `agent_end`, if any.
    pub final_output: Option<String>,
    /// Stderr captured from the backend process. Surfaced (not swallowed) so a
    /// crashed backend's error reaches the user instead of a silent null verdict.
    pub stderr: Option<String>,
    /// True iff the verifier was killed by `verifierTimeoutSec`.
    pub timed_out: bool,
    /// Raw exit code of the child process, when it exited normally. `None` on timeout
    /// (killed) or when the OS did not report an exit code (e.g. signaled). Used by the
    /// health layer to flag an unhealthy run (non-success exit code).
    pub exit_code: Option<i32>,
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
    /// Number of within-round verdict-enforcement nudge resumes issued (D5).
    #[serde(default)]
    pub nudge_attempts: u32,
    /// True iff the verifier session stream contained a `type:compaction` event (D6).
    #[serde(default)]
    pub compaction_observed: bool,
    /// Number of compaction-recovery resumes issued (D6). Capped at 1 per slot per round.
    #[serde(default)]
    pub recovery_attempts: u32,
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

/// Write `meta.json` atomically (temp sibling + rename).
fn write_meta(vdir: &Path, meta: &VerifierMeta) -> Result<(), SpawnError> {
    let tmp = vdir.join(format!("{META_FILE}.tmp"));
    fs::write(&tmp, serde_json::to_string_pretty(meta)?)?;
    fs::rename(&tmp, vdir.join(META_FILE))?;
    Ok(())
}

/// Read the current verdict status for a verifier slot. Returns `true` iff the slot has
/// a non-null verdict (APPROVE or REJECT). Used by the nudge/recovery loop to decide
/// whether to stop.
fn slot_has_verdict(root: &Path, goal_id: &str, verifier_id: &str, round: u32) -> bool {
    match verdict::read_verdict(root, goal_id, verifier_id, round) {
        Ok(rec) => rec.status != verdict::VerdictStatus::Null,
        Err(_) => false,
    }
}

/// Read a `VerifierMeta` and report its `turns_used` + `sid`, defaulting to 0 / None if
/// the meta file is missing or malformed. Used by the nudge loop to track budget.
fn slot_meta(vdir: &Path) -> (u32, Option<String>) {
    match read_meta(vdir) {
        Ok(Some(m)) => (m.turns_used, m.sid),
        _ => (0, None),
    }
}

/// Spawn a nudge/resume child: build the adapter's resume command with the given SID,
/// inject identity env, and write the nudge prompt to the child's stdin. Returns the
/// spawned child + its stdin-writer handle. Used by both the verdict-enforcement nudge
/// loop (D5) and the compaction-recovery resume (D6).
async fn spawn_nudge_child(
    input: &SpawnInput<'_>,
    verifier_id: &str,
    sid: &str,
    nudge_prompt: &str,
) -> Result<
    (
        tokio::process::Child,
        Option<JoinHandle<io::Result<()>>>,
        Option<TempPromptFile>,
    ),
    SpawnError,
> {
    // Transport-aware nudge spawn. For `Stdin` the nudge prompt travels on the child's
    // stdin pipe (same as the initial spawn). For `GoalFile` a new tempfile carries the
    // nudge prompt; the guard is returned so the caller holds it until the child is
    // reaped (the file is unlinked on drop, after the child has opened it).
    let goal_file_guard: Option<TempPromptFile> = match input.adapter.transport {
        Transport::GoalFile => Some(TempPromptFile::new(nudge_prompt.as_bytes())?),
        Transport::Stdin => None,
    };
    let goal_file_path = goal_file_guard.as_ref().map(|g| g.path());
    let stdin_config = match input.adapter.transport {
        Transport::Stdin => Stdio::piped(),
        Transport::GoalFile => Stdio::null(),
    };
    let mut cmd = build_resume_command(input.adapter, sid, goal_file_path);
    // Read the persisted per-verifier signing secret (written at initial spawn time
    // by mint_and_pin_pubkey) so the resume child can sign a verdict that verifies
    // against the slot's pinned pubkey. The initial spawn injected the secret into the
    // child's ENV; a nudge/recovery resume is a NEW process and cannot inherit that
    // env, so it MUST read the persisted copy. Re-minting fails (pubkey already
    // pinned + a fresh key wouldn't match the pinned pubkey), so we read instead.
    // Ok(None) on a legacy unsigned slot → inject empty string (fail-closed: any
    // harvested verdict will fail consensus signature verification).
    let secret_hex =
        verdict::read_verifier_secret(input.root, input.goal_id, verifier_id, input.round)?;
    // Resolve the goal trace id for the nudge child too (add-otel-observability D2):
    // a nudge/recovery resume is a NEW process and cannot inherit the initial
    // spawn env, so it must read the persisted trace-id (same as the persisted
    // secret above). Fail-open per D5.
    let trace_id = crate::observe::ensure_goal_trace_id(input.root, input.goal_id).ok();
    inject_identity_env(
        &mut cmd,
        input.goal_id,
        verifier_id,
        input.round,
        input.root,
        secret_hex.as_deref().unwrap_or(""),
        trace_id.as_deref(),
    );
    inject_verifier_verdict_bin(&mut cmd);
    cmd.stdin(stdin_config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;
    let write_handle = match input.adapter.transport {
        Transport::Stdin => spawn_stdin_writer(child.stdin.take(), nudge_prompt.as_bytes()),
        Transport::GoalFile => None,
    };
    Ok((child, write_handle, goal_file_guard))
}

/// Wait for a spawned child (the nudge/resume) with the per-verifier timeout, drain its
/// stdout/stderr, parse SID + final output + compaction, persist artifacts, and update
/// meta. Returns a struct describing the nudge run's outcome. Mirrors the inner block of
/// the main `gather` `child.wait()` arm but does NOT handle GoalFile tempfiles.
struct NudgeOutcome {
    sid: Option<String>,
    final_output: Option<String>,
    stderr: Option<String>,
    timed_out: bool,
    compaction_observed: bool,
    agent_end_seen: bool,
    exit_code: Option<i32>,
}

async fn reap_nudge_child(
    input: &SpawnInput<'_>,
    _verifier_id: &str,
    vdir: &Path,
    mut child: tokio::process::Child,
    write_handle: Option<JoinHandle<io::Result<()>>>,
    nudge_attempts: u32,
    recovery_attempts: u32,
) -> Result<NudgeOutcome, SpawnError> {
    let timeout = Duration::from_secs(input.config.verifier_timeout_sec.max(1));
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    // Drain stdout AND stderr CONCURRENTLY (not sequentially). A sequential read
    // (stdout first, then stderr) can lose stderr data under timing variations:
    // a chatty stderr fills the kernel pipe buffer (~64KB) while stdout is still
    // live, and the child blocks on its next stderr write until timeout. Mirrors
    // the main `gather` drain, which already uses tokio::join! for the same reason.
    let drain = tokio::spawn(async move {
        let stdout_fut = async {
            match stdout_pipe {
                Some(mut pipe) => {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    let _ = pipe.read_to_end(&mut buf).await;
                    buf
                }
                None => Vec::new(),
            }
        };
        let stderr_fut = async {
            match stderr_pipe {
                Some(mut p) => bounded_stderr_tail(&mut p, STDERR_CAP_BYTES).await,
                None => Vec::new(),
            }
        };
        let (buf, stderr_buf) = tokio::join!(stdout_fut, stderr_fut);
        (buf, stderr_buf)
    });

    let outcome = tokio::select! {
        biased;
        _ = tokio::time::sleep(timeout) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            if let Some(h) = write_handle { let _ = h.await; }
            let (_, stderr_buf) = drain.await.unwrap_or_default();
            let stderr_text = {
                let t = String::from_utf8_lossy(&stderr_buf);
                if t.is_empty() { None } else { Some(t.into_owned()) }
            };
            if let Some(text) = &stderr_text {
                let _ = fs::write(vdir.join(STDERR_FILE), text);
            }
            NudgeOutcome {
                sid: None,
                final_output: None,
                stderr: stderr_text,
                timed_out: true,
                compaction_observed: false,
                agent_end_seen: false,
                exit_code: None,
            }
        }
        status = child.wait() => {
            let (stdout_buf, stderr_buf) = drain.await.unwrap_or_default();
            let stdout = String::from_utf8_lossy(&stdout_buf);
            let stderr_text = {
                let t = String::from_utf8_lossy(&stderr_buf);
                if t.is_empty() { None } else { Some(t.into_owned()) }
            };
            if let Some(text) = &stderr_text {
                let _ = fs::write(vdir.join(STDERR_FILE), text);
            }
            let sid = acp::extract_sid(&stdout);
            let final_output = acp::extract_final_output(&stdout);
            let compaction_observed = acp::extract_compaction_observed(&stdout);
            // agent_end_seen iff final_output was captured (agent_end is the source).
            let agent_end_seen = final_output.is_some();
            if let Some(h) = write_handle {
                let _ = h.await;
            }
            if let Some(text) = &final_output {
                let _ = fs::write(vdir.join(FINAL_OUTPUT_FILE), text);
            }
            let _ = &status;
            let nudge_exit_code = status.as_ref().ok().and_then(|s| s.code());
            // Update meta: increment turns and record compaction.
            let existing = read_meta(vdir)?.unwrap_or(VerifierMeta {
                sid: None,
                turns_used: 0,
                nudge_attempts: 0,
                compaction_observed: false,
                recovery_attempts: 0,
            });
            // Persist the ACTIVE loop counts so an intermediate meta write is never
            // stale (a crash mid-loop must leave an accurate audit trail). The caller
            // increments nudge/recovery BEFORE this call; turns_used advances by one
            // per nudge resume.
            let updated = VerifierMeta {
                sid: sid.clone().or(existing.sid),
                turns_used: existing.turns_used.saturating_add(1).min(input.config.max_turn),
                nudge_attempts,
                compaction_observed: existing.compaction_observed || compaction_observed,
                recovery_attempts,
            };
            write_meta(vdir, &updated)?;
            NudgeOutcome {
                sid,
                final_output,
                stderr: stderr_text,
                timed_out: false,
                compaction_observed,
                agent_end_seen,
                exit_code: nudge_exit_code,
            }
        }
    };
    Ok(outcome)
}

/// Persist `meta.json` and write any text to `final-output.txt`. Idempotent.
fn flush_meta(
    vdir: &Path,
    sid: Option<&str>,
    compaction_observed: bool,
    nudge_attempts: u32,
    recovery_attempts: u32,
    _input: &SpawnInput<'_>,
) -> Result<VerifierMeta, SpawnError> {
    let existing = read_meta(vdir)?.unwrap_or(VerifierMeta {
        sid: None,
        turns_used: 0,
        nudge_attempts: 0,
        compaction_observed: false,
        recovery_attempts: 0,
    });
    let updated = VerifierMeta {
        sid: sid.map(str::to_string).or(existing.sid),
        turns_used: existing.turns_used,
        nudge_attempts,
        compaction_observed: existing.compaction_observed || compaction_observed,
        recovery_attempts,
    };
    write_meta(vdir, &updated)?;
    Ok(updated)
}

/// Read-modify-write `meta.json`: advance ONLY the loop counters
/// (`nudge_attempts` / `recovery_attempts` / `compaction_observed`). Leaves `sid` and
/// `turns_used` untouched. Called BEFORE a nudge/recovery child is spawned so that a
/// crash / signal / timeout mid-spawn leaves an accurate audit trail in `meta.json`
/// (the post-run persist in `reap_nudge_child` only fires after the child exits, so
/// without this pre-spawn checkpoint a mid-spawn crash would leave the counters stale).
fn checkpoint_meta(
    vdir: &Path,
    nudge_attempts: u32,
    recovery_attempts: u32,
    compaction_observed: bool,
) -> Result<(), SpawnError> {
    let existing = read_meta(vdir)?.unwrap_or(VerifierMeta {
        sid: None,
        turns_used: 0,
        nudge_attempts: 0,
        compaction_observed: false,
        recovery_attempts: 0,
    });
    let updated = VerifierMeta {
        sid: existing.sid,
        turns_used: existing.turns_used,
        nudge_attempts,
        compaction_observed: existing.compaction_observed || compaction_observed,
        recovery_attempts,
    };
    write_meta(vdir, &updated)?;
    Ok(())
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

    // Resolve the per-goal trace id ONCE (add-otel-observability D1/D2): mint-or-read
    // <store>/goals/<goalId>/trace-id, then propagate to every V* child env so the
    // V* process's `jewije` calls join this round's trace. Fail-open per D5: a
    // resolution error is swallowed (None) — tracing never blocks a verdict.
    let trace_id = crate::observe::ensure_goal_trace_id(input.root, input.goal_id).ok();

    // Build the launch plan: (verifierId, command, vdir, goal_file_guard) for each
    // of m verifiers. The goal_file_guard is `Some` only for `GoalFile` transport;
    // it is held in the plan / children vec so the tempfile lives until the gather
    // barrier reaps the child (design D3 — unlink after the child has opened the file).
    let mut plan: Vec<(String, Command, PathBuf, Option<TempPromptFile>)> = Vec::new();
    let count = input.verifier_count.unwrap_or(input.config.m as usize);
    let prefix = input.id_prefix.unwrap_or("v");
    let offset = input.id_offset;
    for i in 0..count {
        let vid = format!("{}{}", prefix, offset + i + 1);
        let vdir = rounds_dir.join(&vid);
        fs::create_dir_all(&vdir)?;
        pre_create_verifier_dir(&vdir);

        // GoalFile transport: create the tempfile BEFORE building the command so its
        // path can be substituted into `{goalFile}`. On spawn failure (later), the
        // guard drops here via `?` propagation, unlinking the tempfile.
        let goal_file_guard = match input.adapter.transport {
            Transport::GoalFile => Some(TempPromptFile::new(input.prompt.as_bytes())?),
            Transport::Stdin => None,
        };
        let goal_file_path = goal_file_guard.as_ref().map(|g| g.path());

        let mut cmd = build_spawn_command(input.adapter, goal_file_path);
        let secret_hex = mint_verifier_secret(input.root, input.goal_id, &vid, input.round)?;
        inject_identity_env(
            &mut cmd,
            input.goal_id,
            &vid,
            input.round,
            input.root,
            &secret_hex,
            trace_id.as_deref(),
        );
        inject_verifier_verdict_bin(&mut cmd);
        plan.push((vid, cmd, vdir, goal_file_guard));
    }

    // Launch every child BEFORE awaiting any (non-blocking spawn). Each `spawn()` starts
    // the OS process immediately; awaiting is the gather barrier.
    //
    // For `Stdin` transport (design D1/D7): child stdin is piped and the rendered prompt
    // is written by a background task (D4). For `GoalFile`: stdin stays null (§7 will
    // substitute a tempfile path into the argv instead).
    let mut children: Vec<SpawnedChild> = Vec::new();
    for (vid, mut cmd, vdir, goal_file_guard) in plan {
        let stdin_config = match input.adapter.transport {
            Transport::Stdin => Stdio::piped(),
            Transport::GoalFile => Stdio::null(),
        };
        cmd.stdin(stdin_config)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn()?;
        // Take the stdin pipe (if piped) and spawn a background writer that streams the
        // full prompt bytes, then closes the pipe (D4 async write).
        let write_handle = spawn_stdin_writer(child.stdin.take(), input.prompt.as_bytes());
        children.push((vid, child, vdir, write_handle, goal_file_guard));
    }

    // Fresh round: enable verdict-enforcement nudge loop (D5 + D6) only for the
    // stdin transport. The GoalFile transport is used by custom adapters/tests whose
    // stubs are not designed to handle multiple nudge resumes; limiting nudge to the
    // stdin transport (used by all built-in adapters) keeps existing tests intact.
    let enable_nudge = input.adapter.transport == Transport::Stdin;
    gather(input, enable_nudge, children).await
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

    // Resolve the per-goal trace id once (add-otel-observability D1/D2): reuses the
    // value persisted at NEW time, so RESUME joins the same trace as NEW. Fail-open.
    let trace_id = crate::observe::ensure_goal_trace_id(input.root, input.goal_id).ok();

    let mut plan: Vec<(String, Command, PathBuf, Option<TempPromptFile>)> = Vec::new();
    let count = input.verifier_count.unwrap_or(input.config.m as usize);
    let prefix = input.id_prefix.unwrap_or("v");
    let offset = input.id_offset;
    for i in 0..count {
        let vid = format!("{}{}", prefix, offset + i + 1);
        let vdir = rounds_dir.join(&vid);
        fs::create_dir_all(&vdir)?;

        let prev_vdir = round_dir(input.root, input.goal_id, prev_round).join(&vid);
        let prior = read_meta(&prev_vdir)?;

        // GoalFile transport: create the tempfile BEFORE building the command so its
        // path can be substituted into `{goalFile}` (same as spawn_round).
        let goal_file_guard = match input.adapter.transport {
            Transport::GoalFile => Some(TempPromptFile::new(input.prompt.as_bytes())?),
            Transport::Stdin => None,
        };
        let goal_file_path = goal_file_guard.as_ref().map(|g| g.path());

        let mut cmd;
        let fresh;
        match &prior {
            // Reuse: resume on the prior SID, but ONLY when the SID is present and
            // non-empty. A None/empty prior SID (e.g. after a timeout that did not
            // capture the `session` event) must NOT be substituted into `--session {sid}`
            // — the empty substitution would yield `pi --session  --mode json`, where
            // the shell splits the doubled space and pi parses `--mode` as the session
            // name argument, producing the cascading "No session found matching '--mode'"
            // failure (gh #45/#48/#56/#59/#62/#65/#66/#67/#69). Fall through to fresh.
            Some(meta)
                if meta.turns_used < input.config.max_turn
                    && meta.sid.as_deref().is_some_and(|s| !s.is_empty()) =>
            {
                // Reuse: resume on the prior SID (gate above guarantees non-empty).
                let sid = meta.sid.clone().expect("non-empty SID guaranteed by guard");
                cmd = build_resume_command(input.adapter, &sid, goal_file_path);
                fresh = false;
            }
            _ => {
                // Fresh: exhausted, no prior meta, OR prior SID missing/empty.
                // Archive the prior SID only when non-empty (nothing to archive otherwise).
                if let Some(meta) = prior.as_ref() {
                    if let Some(sid) = meta.sid.as_deref() {
                        if !sid.is_empty() {
                            archive_prior_sid(&prev_vdir, sid)?;
                        }
                    }
                }
                cmd = build_spawn_command(input.adapter, goal_file_path);
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
            trace_id.as_deref(),
        );
        inject_verifier_verdict_bin(&mut cmd);
        plan.push((vid, cmd, vdir, goal_file_guard));
    }

    let mut children: Vec<SpawnedChild> = Vec::new();
    for (vid, mut cmd, vdir, goal_file_guard) in plan {
        let stdin_config = match input.adapter.transport {
            Transport::Stdin => Stdio::piped(),
            Transport::GoalFile => Stdio::null(),
        };
        cmd.stdin(stdin_config)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn()?;
        let write_handle = spawn_stdin_writer(child.stdin.take(), input.prompt.as_bytes());
        children.push((vid, child, vdir, write_handle, goal_file_guard));
    }

    // Resume round: verdict-enforcement nudge (D5) + compaction recovery (D6) apply
    // universally — the verifier-spawn spec "Verdict is enforced after child exit" carries
    // no round-type carve-out. A resume-round child that exits with no verdict MUST be
    // re-prompted on the same sid, exactly like a fresh round. The transport guard
    // (Stdin only) matches spawn_round: GoalFile custom adapters are not designed for
    // multiple nudge resumes and are scoped out consistently on both paths.
    let enable_nudge = input.adapter.transport == Transport::Stdin;
    gather(input, enable_nudge, children).await
}

// ---------------------------------------------------------------------------
// gather barrier (shared by spawn_round + spawn_resume)
// ---------------------------------------------------------------------------

/// Await every launched child with a per-verifier timeout (D9). On timeout the child is
/// killed and the run is marked `timed_out`; the pre-created null verdict is left in
/// place. Captured stdout is parsed for the SID + final output, and `meta.json` +
/// `final-output.txt` are updated accordingly.
///
/// For the `Stdin` transport the background stdin-writer's `JoinHandle` is awaited
/// after the child exits (D4). Per design D4/R2, a write error (typically `EPIPE` when
/// the child exits before draining the pipe) is treated as:
///   - **non-fatal** when the child already produced a recognizable ACP stream (a SID
///     was captured), and
///   - **fatal / fail-closed** when no ACP event was parsed (the verdict stays `null`).
///
/// In practice the fail-closed outcome is already guaranteed by the ACP parse: if no
/// `session` event was emitted, `sid` is `None` and the null verdict on disk is left
/// untouched. The write result is checked here only to short-circuit `meta.json` /
/// `final-output.txt` updates when the run is fail-closed (so a crashed verifier leaves
/// no stale SID/output artifacts).
/// Read a stderr pipe keeping only the last `cap` bytes (the diagnostic tail).
/// Errors live at the end of a run; earlier chatter is discarded to bound RAM.
/// If the stream exceeds `cap`, the returned buffer is prefixed with a truncation
/// marker so the user knows output was elided.
async fn bounded_stderr_tail<R: tokio::io::AsyncRead + Unpin>(pipe: &mut R, cap: usize) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut chunk = [0u8; 1024];
    let mut total_seen: u64 = 0;
    let mut tail: Vec<u8> = Vec::new();
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break, // EOF
            Ok(n) => {
                total_seen += n as u64;
                tail.extend_from_slice(&chunk[..n]);
                // Trim to keep only the last `cap` bytes in memory.
                if tail.len() > cap {
                    let excess = tail.len() - cap;
                    tail.drain(..excess);
                }
            }
            Err(_) => break, // best-effort: stop on read error
        }
    }
    if total_seen as usize > cap {
        let marker = format!(
            "[...truncated {} bytes of stderr above the {}-byte cap...]\n",
            total_seen.saturating_sub(cap as u64),
            cap,
        );
        let mut out = marker.into_bytes();
        out.extend_from_slice(&tail);
        out
    } else {
        tail
    }
}

async fn gather(
    input: SpawnInput<'_>,
    enable_nudge: bool,
    mut children: Vec<SpawnedChild>,
) -> Result<Vec<VerifierRun>, SpawnError> {
    let timeout = Duration::from_secs(input.config.verifier_timeout_sec.max(1));
    let mut runs = Vec::with_capacity(children.len());

    for (vid, mut child, vdir, write_handle, _goal_file_guard) in children.drain(..) {
        // Take the stdout pipe out of the child and drain it on a background task.
        // Draining concurrently with `wait()` is required because an OS pipe holds only
        // ~64KB: a verifier emitting MBs of ACP events would fill the buffer, block on
        // write, and either hang to the timeout or exit without emitting `agent_end` —
        // leaving a null verdict despite a successful run.
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let drain = tokio::spawn(async move {
            // Read stdout and stderr CONCURRENTLY (not sequentially). A sequential
            // read (stdout first, then stderr) can lose stderr data under timing
            // variations: if stdout blocks waiting for EOF, stderr may fill the
            // kernel pipe buffer before the drain begins reading it. Reading both
            // concurrently ensures neither pipe overflows.
            let stdout_fut = async {
                match stdout_pipe {
                    Some(mut pipe) => {
                        use tokio::io::AsyncReadExt;
                        let mut buf = Vec::new();
                        let _ = pipe.read_to_end(&mut buf).await;
                        buf
                    }
                    None => Vec::new(),
                }
            };
            let stderr_fut = async {
                match stderr_pipe {
                    Some(mut p) => bounded_stderr_tail(&mut p, STDERR_CAP_BYTES).await,
                    None => Vec::new(),
                }
            };
            let (buf, stderr_buf) = tokio::join!(stdout_fut, stderr_fut);
            (buf, stderr_buf)
        });

        let run = tokio::select! {
            biased;
            _ = tokio::time::sleep(timeout) => {
                // Timeout (D9): kill, reap, leave null verdict. No SID / final output.
                let _ = child.start_kill();
                let _ = child.wait().await;
                // Reap the stdin writer so it is not leaked (its result is irrelevant
                // for a timed-out run — the verdict is already null via timeout).
                if let Some(h) = write_handle { let _ = h.await; }
                // Drain whatever stderr was captured before the kill, for post-mortem.
                let stderr = drain.await.ok().and_then(|(_s, e)| {
                    let t = String::from_utf8_lossy(&e);
                    if t.is_empty() { None } else { Some(t.into_owned()) }
                });
                if let Some(text) = &stderr {
                    let _ = fs::write(vdir.join(STDERR_FILE), text);
                }
                VerifierRun { verifier_id: vid, sid: None, final_output: None, stderr, timed_out: true, exit_code: None }
            }
            status = child.wait() => {
                // Child exited; the drain task finishes shortly after (pipe hits EOF).
                let (stdout_buf, stderr_buf) = drain.await.unwrap_or_default();
                let stdout = String::from_utf8_lossy(&stdout_buf);
                let stderr_text: Option<String> = {
                    let t = String::from_utf8_lossy(&stderr_buf);
                    if t.is_empty() { None } else { Some(t.into_owned()) }
                };
                // Persist stderr whenever present (success or crash) so the user can
                // always inspect backend diagnostics.
                if let Some(text) = &stderr_text {
                    let _ = fs::write(vdir.join(STDERR_FILE), text);
                }
                let sid = acp::extract_sid(&stdout);
                let final_output = acp::extract_final_output(&stdout);
                let compaction_observed = acp::extract_compaction_observed(&stdout);
                let agent_end_seen = final_output.is_some();

                // Await the stdin writer (D4). For `Stdin` transport this surfaces
                // EPIPE if the child exited without draining the pipe; for `GoalFile`
                // there is no writer (`write_handle` is `None`).
                let write_ok = match write_handle {
                    Some(h) => h.await.unwrap_or(Err(io::Error::other("stdin writer join failed"))).is_ok(),
                    None => true,
                };
                let fail_closed = !write_ok && sid.is_none();

                if !fail_closed {
                    if let Some(text) = &final_output {
                        let _ = fs::write(vdir.join(FINAL_OUTPUT_FILE), text);
                    }
                    update_meta_after_run(&vdir, sid.as_deref(), &input)?;
                }
                // `status` is `io::Result<ExitStatus>`; capture the exit code for the
                // health layer (a non-success exit flags the run as unhealthy even if
                // some output was produced). A wait error or signal yields `None`.
                let mut run_exit_code = status.as_ref().ok().and_then(|s| s.code());

                // Verdict-enforcement + compaction-recovery loop (D5 + D6).
                // After the initial child exits, if the slot still has no verdict and
                // turns remain, we issue nudge resumes. If compaction was observed and
                // the session exited without agent_end, we do ONE recovery resume first.
                let mut run_sid = if fail_closed { None } else { sid.clone() };
                let mut run_final = if fail_closed { None } else { final_output.clone() };
                let mut run_stderr = stderr_text;
                let mut nudge_attempts = 0u32;
                let mut recovery_attempts = 0u32;
                let mut any_compaction = compaction_observed;
                let mut nudge_agent_end_seen = agent_end_seen;
                let mut nudge_sid = sid;

                if !fail_closed && enable_nudge && !slot_has_verdict(input.root, input.goal_id, &vid, input.round) {
                    loop {
                        let (turns_used, active_sid) = slot_meta(&vdir);
                        if turns_used >= input.config.max_turn {
                            break;
                        }
                        let do_recovery = any_compaction && !nudge_agent_end_seen && recovery_attempts == 0;
                        if !do_recovery && recovery_attempts > 0 {
                            // Recovery was attempted and failed to harvest a verdict:
                            // fail-closed rather than keep nudging forever.
                            break;
                        }
                        let sid_to_use = active_sid.or(nudge_sid.clone()).unwrap_or_default();
                        if sid_to_use.is_empty() {
                            break; // no resumable session id
                        }

                        if do_recovery {
                            recovery_attempts += 1;
                        }
                        nudge_attempts += 1;

                        // Persist the incremented counters to meta.json BEFORE spawning
                        // the nudge/recovery child. A crash / signal / timeout mid-spawn
                        // would otherwise leave meta.json stale (the post-run persist in
                        // `reap_nudge_child` only fires after the child exits). This makes
                        // the audit-trail claim accurate even on a mid-spawn crash.
                        checkpoint_meta(&vdir, nudge_attempts, recovery_attempts, any_compaction)?;

                        let (nudge_child, nudge_writer, _nudge_guard) =
                            spawn_nudge_child(
                                &input,
                                &vid,
                                &sid_to_use,
                                if do_recovery {
                                    COMPACTION_RECOVERY_NUDGE_PROMPT
                                } else {
                                    VERDICT_NUDGE_PROMPT
                                },
                            )
                            .await?;
                        let nudge = reap_nudge_child(
                            &input,
                            &vid,
                            &vdir,
                            nudge_child,
                            nudge_writer,
                            nudge_attempts,
                            recovery_attempts,
                        )
                        .await?;

                        if nudge.timed_out {
                            break;
                        }
                        // A nudge child that exited carries the latest exit code; track it
                        // so the health layer sees the most recent backend health signal.
                        if let Some(code) = nudge.exit_code {
                            run_exit_code = Some(code);
                        }
                        if let Some(s) = nudge.sid {
                            nudge_sid = Some(s);
                            run_sid = nudge_sid.clone();
                        }
                        if let Some(out) = nudge.final_output {
                            run_final = Some(out);
                        }
                        if let Some(stderr) = nudge.stderr {
                            run_stderr = Some(stderr);
                        }
                        any_compaction = any_compaction || nudge.compaction_observed;
                        nudge_agent_end_seen = nudge_agent_end_seen || nudge.agent_end_seen;

                        if slot_has_verdict(input.root, input.goal_id, &vid, input.round) {
                            break;
                        }
                    }
                }

                // Persist final bookkeeping into meta.json.
                let _final_meta = flush_meta(
                    &vdir,
                    run_sid.as_deref(),
                    any_compaction,
                    nudge_attempts,
                    recovery_attempts,
                    &input,
                )?;

                VerifierRun {
                    verifier_id: vid,
                    sid: if fail_closed { None } else { run_sid },
                    final_output: if fail_closed { None } else { run_final },
                    stderr: run_stderr,
                    timed_out: false,
                    exit_code: run_exit_code,
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
        nudge_attempts: 0,
        compaction_observed: false,
        recovery_attempts: 0,
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
    // Tolerate older meta.json without the new fields (default them).
    let mut val: serde_json::Value = serde_json::from_str(&raw)?;
    if let Some(obj) = val.as_object_mut() {
        obj.entry("nudgeAttempts").or_insert(serde_json::json!(0));
        obj.entry("compactionObserved")
            .or_insert(serde_json::json!(false));
        obj.entry("recoveryAttempts")
            .or_insert(serde_json::json!(0));
    }
    Ok(Some(serde_json::from_value(val)?))
}

/// Update `meta.json` after a run with the captured SID, an incremented turn count, and
/// the compaction/verdict-enforcement bookkeeping.
fn update_meta_after_run(
    vdir: &Path,
    sid: Option<&str>,
    input: &SpawnInput<'_>,
) -> Result<VerifierMeta, SpawnError> {
    let existing = read_meta(vdir)?.unwrap_or(VerifierMeta {
        sid: None,
        turns_used: 0,
        nudge_attempts: 0,
        compaction_observed: false,
        recovery_attempts: 0,
    });
    let turns_used = existing
        .turns_used
        .saturating_add(1)
        .min(input.config.max_turn);
    let updated = VerifierMeta {
        sid: sid.map(str::to_string).or(existing.sid),
        turns_used,
        nudge_attempts: existing.nudge_attempts,
        compaction_observed: existing.compaction_observed,
        recovery_attempts: existing.recovery_attempts,
    };
    write_meta(vdir, &updated)?;
    Ok(updated)
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

/// Build a `Command` from a rendered template string according to the adapter's
/// [`Transport`] (fix-spawn-argv-overflow design D1/D5/D7; prompt-transport spec).
///
/// * `Stdin` — the prompt travels on the child's stdin pipe, NOT in argv. The
///   template MUST NOT contain `{prompt}` (validated at config load for custom
///   adapters); it is split on whitespace into program + args verbatim. `goal_file`
///   is ignored.
/// * `GoalFile` — the prompt was written to a tempfile by [`TempPromptFile`]; its
///   absolute path is substituted for every `{goalFile}` placeholder (design D5:
///   single substitution, replace all occurrences). The substituted template is split
///   on whitespace into program + args. `goal_file` MUST be `Some` for this transport.
fn build_command_from_template(
    template: &str,
    transport: Transport,
    goal_file: Option<&Path>,
) -> Command {
    match transport {
        Transport::Stdin => {
            // Stdin transport: NO prompt bytes in argv. Split the template on
            // whitespace into program + args. The prompt is piped to stdin by
            // [`spawn_stdin_writer`] after the child is spawned.
            split_into_command(template)
        }
        Transport::GoalFile => {
            // GoalFile transport: substitute the tempfile path into {goalFile}, then
            // split on whitespace. The path is short and shell-safe in argv (no quoting
            // needed). `goal_file` is required for this transport.
            let path = goal_file.unwrap_or_else(|| {
                panic!("GoalFile transport requires a goal_file path (TempPromptFile)")
            });
            let rendered = template.replace("{goalFile}", &path.to_string_lossy());
            split_into_command(&rendered)
        }
    }
}

/// Split a (already-substituted) command template on whitespace into a `Command`:
/// first token is the program, the rest are args. Shared by both transports — neither
/// places prompt bytes in argv.
fn split_into_command(template: &str) -> Command {
    let mut parts = template.split_whitespace();
    let program = parts
        .next()
        .expect("spawn template has a non-empty program");
    let mut cmd = Command::new(program);
    for a in parts {
        cmd.arg(a);
    }
    cmd
}

/// Build the spawn `Command` for an adapter (transport-aware). See
/// [`build_command_from_template`] for the per-transport argv construction. For
/// `GoalFile`, `goal_file` must be the path of the tempfile created by the caller.
fn build_spawn_command(adapter: &acp::Adapter, goal_file: Option<&Path>) -> Command {
    build_command_from_template(&adapter.spawn, adapter.transport, goal_file)
}

/// Build the resume `Command` for an adapter: substitute `{sid}` into the resume
/// template, then delegate to [`build_command_from_template`] with the adapter's
/// transport and (for `GoalFile`) the tempfile path.
fn build_resume_command(adapter: &acp::Adapter, sid: &str, goal_file: Option<&Path>) -> Command {
    let with_sid = adapter.resume.replace("{sid}", sid);
    build_command_from_template(&with_sid, adapter.transport, goal_file)
}

/// Spawn a background task that writes the full prompt bytes to the child's stdin
/// pipe, then closes it (design D4 — async stdin write so a slow-reading child never
/// stalls parallel spawns). Returns `None` when there is no stdin pipe (e.g. the
/// `GoalFile` transport, which sets `stdin = Stdio::null()`).
///
/// The returned `JoinHandle` resolves to `io::Result<()>`. A `BrokenPipe` (`EPIPE`)
/// error is expected when the child exits before draining the pipe; [`gather`] treats
/// it as non-fatal when ACP output was produced, and as fail-closed otherwise (D4/R2).
fn spawn_stdin_writer(
    stdin: Option<tokio::process::ChildStdin>,
    prompt: &[u8],
) -> Option<JoinHandle<io::Result<()>>> {
    let mut stdin = stdin?;
    let prompt = prompt.to_vec();
    Some(tokio::spawn(async move {
        if let Err(e) = stdin.write_all(&prompt).await {
            // The pipe may still be flushed up to the kernel buffer; attempt a
            // graceful shutdown but surface the original write error.
            let _ = stdin.shutdown().await;
            return Err(e);
        }
        // Close the write end so the child sees EOF.
        let _ = stdin.shutdown().await;
        Ok(())
    }))
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
/// plus the per-verifier signing secret (D3) so its verdict registration is signed,
/// plus the per-goal trace id (add-otel-observability D2) so the V* process's
/// `jewije` calls join the spawning round's trace.
fn identity_env_pairs<'a>(
    goal_id: &'a str,
    verifier_id: &'a str,
    round: u32,
    root: &'a Path,
    secret_hex: &'a str,
    trace_id: Option<&'a str>,
) -> Vec<(&'static str, std::ffi::OsString)> {
    let mut pairs = vec![
        (ENV_GOAL_ID, goal_id.into()),
        (ENV_VERIFIER_ID, verifier_id.into()),
        (ENV_ROUND, round.to_string().into()),
        (ENV_HOME, root.as_os_str().into()),
        (ENV_VERIFIER_SECRET, secret_hex.into()),
    ];
    if let Some(tid) = trace_id {
        pairs.push((crate::observe::ENV_TRACE_ID, tid.into()));
    }
    pairs
}

/// Inject the identity + store-root + signing-secret + trace-id env vars into a
/// verifier command (D2 + D3 + add-otel-observability D2).
fn inject_identity_env(
    cmd: &mut Command,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
    root: &Path,
    secret_hex: &str,
    trace_id: Option<&str>,
) {
    for (k, v) in identity_env_pairs(goal_id, verifier_id, round, root, secret_hex, trace_id) {
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

// ---------------------------------------------------------------------------
// §7 — goal-file transport helpers live in [`crate::spawn::tempfile`] (TempPromptFile
// RAII guard + sweep_stale_tempfiles). The orchestrator only constructs the guard and
// substitutes its path into `{goalFile}`; the guard's `Drop` handles unlinking.
// ---------------------------------------------------------------------------

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
        // GoalFile transport: substitutes the tempfile path into {goalFile}. The prompt
        // itself NEVER touches argv (it lives in the tempfile). The Stdin transport
        // path is covered by tests/spawn_stdin_transport.rs; the goal-file integration
        // is covered by tests/spawn_goal_file_transport.rs. This unit test pins the
        // argv-shape contract for the {goalFile} substitution.
        let tmp = tempfile::tempdir().unwrap();
        let goal_path = tmp.path().join("fake-goal.txt");
        fs::write(&goal_path, b"irrelevant").unwrap();
        let adapter = acp::Adapter {
            spawn: "pi --goal-file {goalFile} --mode json".into(),
            resume: "pi --goal-file {goalFile} --mode json".into(),
            transport: Transport::GoalFile,
        };
        let cmd = build_spawn_command(&adapter, Some(&goal_path));
        let s = format!("{:?}", cmd.as_std());
        assert!(s.contains("pi"), "program is pi");
        assert!(!s.contains("sh"), "must NOT use sh");
        assert!(
            s.contains("--goal-file"),
            "pre-placeholder args preserved: {s}"
        );
        assert!(
            s.contains(goal_path.to_str().unwrap()),
            "the {{goalFile}} placeholder must be substituted with the tempfile path: {s}"
        );
        assert!(
            !s.contains("{goalFile}"),
            "no literal {{goalFile}} token may survive substitution: {s}"
        );
        assert!(s.contains("--mode"), "post-args preserved");
    }

    #[test]
    fn build_resume_command_substitutes_sid_then_prompt() {
        // GoalFile transport: {sid} and {goalFile} are both substituted. The prompt
        // itself is in the tempfile, not argv.
        let tmp = tempfile::tempdir().unwrap();
        let goal_path = tmp.path().join("fake-goal-resume.txt");
        fs::write(&goal_path, b"irrelevant").unwrap();
        let adapter = acp::Adapter {
            spawn: "pi --goal-file {goalFile} --mode json".into(),
            resume: "pi --session {sid} --goal-file {goalFile} --mode json".into(),
            transport: Transport::GoalFile,
        };
        let cmd = build_resume_command(&adapter, "abc-123", Some(&goal_path));
        let s = format!("{:?}", cmd.as_std());
        assert!(s.contains("abc-123"), "sid substituted: {s}");
        assert!(
            s.contains(goal_path.to_str().unwrap()),
            "goalFile path substituted: {s}"
        );
        assert!(!s.contains("{sid}"), "no literal {{sid}} survives: {s}");
        assert!(
            !s.contains("{goalFile}"),
            "no literal {{goalFile}} survives: {s}"
        );
    }

    #[test]
    fn meta_round_trips_camel_case() {
        let m = VerifierMeta {
            sid: Some("s".into()),
            turns_used: 2,
            nudge_attempts: 1,
            compaction_observed: true,
            recovery_attempts: 0,
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
            nudge_attempts: 0,
            compaction_observed: false,
            recovery_attempts: 0,
        };
        let j = serde_json::to_string(&m).unwrap();
        assert!(!j.contains("sid"), "null sid should be skipped: {j}");
    }

    #[test]
    fn identity_env_pairs_propagate_store_root() {
        let root = Path::new("/tmp/vl-home");
        let pairs = identity_env_pairs("g1", "v1", 2, root, "deadbeef", None);
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
        assert!(
            secret.is_some(),
            "VERIFIER_LOOP_VERIFIER_SECRET must be injected"
        );
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
        assert_eq!(d, Path::new("/tmp/x/goals/g1/rounds/2"));
    }

    // F2 (cubic r2): checkpoint_meta persists the loop counters BEFORE the nudge child
    // is spawned so a crash mid-spawn leaves an accurate audit trail. A unit test on the
    // helper pins the behaviour: counters advance and sid/turns_used are untouched
    // (the post-run reap path owns those fields).
    #[test]
    fn checkpoint_meta_persists_counters_without_advancing_turns_or_sid() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path();
        pre_create_verifier_dir(vdir);

        // Seed an existing sid + turns_used so we can prove checkpoint leaves them alone.
        let seeded = VerifierMeta {
            sid: Some("pre-existing-sid".into()),
            turns_used: 3,
            nudge_attempts: 0,
            compaction_observed: false,
            recovery_attempts: 0,
        };
        write_meta(vdir, &seeded).unwrap();

        // Simulate the loop having just incremented nudge_attempts + recovery_attempts
        // and about to spawn a child that gets killed mid-spawn.
        checkpoint_meta(vdir, 2, 1, true).unwrap();

        let m = read_meta(vdir)
            .unwrap()
            .expect("meta.json present after checkpoint");
        assert_eq!(m.nudge_attempts, 2, "nudge_attempts persisted pre-spawn");
        assert_eq!(
            m.recovery_attempts, 1,
            "recovery_attempts persisted pre-spawn"
        );
        assert!(m.compaction_observed, "compaction_observed OR-ed into meta");
        // sid + turns_used MUST be untouched (reap owns them).
        assert_eq!(m.sid.as_deref(), Some("pre-existing-sid"));
        assert_eq!(
            m.turns_used, 3,
            "turns_used must NOT advance at checkpoint time"
        );
    }
}
