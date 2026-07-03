//! `verifier-loop` (aliased `jewilo`) — A's interface (NEW / RESUME).
//!
//! tasks.md §10 — CLI wiring + end-to-end. Dispatches on `NEW` / `RESUME`:
//!
//! * `NEW "<goal>" [--context]`  — ensure salt + config, create immutable signed goal,
//!   capture the frozen artifact snapshot, render the verifier prompt, spawn round 1
//!   (§5), gather, evaluate n/m consensus (§8); on pass write `completion.json` and print
//!   the short completion hash (`mmddyy-XXXXXXXX`); on fail print the rejection and exit non-zero.
//! * `RESUME <goalId> [--fix "…"]` — load the goal, increment the round, append fix notes,
//!   re-capture the snapshot, render the resume prompt, spawn_resume (§6), evaluate.
//!
//! Backend resolution: built-in adapters (`pi` / `hermes` / `acpx`) via the §4 layer;
//! any other `config.backend` value (e.g. `"stub"` for hermetic tests, or `"custom"`) is
//! resolved from `VERIFIER_LOOP_BACKEND_CMD` (used for both spawn+resume) or the
//! `VERIFIER_LOOP_SPAWN_CMD` / `VERIFIER_LOOP_RESUME_CMD` pair. This keeps `acp/` untouched
//! while letting deterministic e2e run without a real `pi`.
//!
//! Fail-closed (D9): every error path is explicit; a NULL verdict never becomes APPROVE; a
//! missing store yields no hash. The salt is never printed.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;
use clap::Parser;

use verifier_loop::cli::{VerifierLoopCli, VerifierLoopCmd};
use verifier_loop::verdict::{self, VerdictStatus};

/// Store-root override env (mirrors verifier-verdict). Defaults to `~/.verifier-loop`.
const ENV_HOME: &str = "VERIFIER_LOOP_HOME";
/// Stub/custom backend command override env (spawn+resume). Used when `config.backend`
/// is not a built-in adapter.
const ENV_BACKEND_CMD: &str = "VERIFIER_LOOP_BACKEND_CMD";
const ENV_SPAWN_CMD: &str = "VERIFIER_LOOP_SPAWN_CMD";
const ENV_RESUME_CMD: &str = "VERIFIER_LOOP_RESUME_CMD";
const DEFAULT_HOME_DIR: &str = ".verifier-loop";

fn main() -> ExitCode {
    let cli = VerifierLoopCli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &VerifierLoopCli) -> Result<(), String> {
    let root = resolve_home()?;
    match cli.command {
        VerifierLoopCmd::New {
            ref goal,
            ref context,
        } => run_new(&root, goal, context.as_deref()),
        VerifierLoopCmd::Resume {
            ref goal_id,
            ref fix,
        } => run_resume(&root, goal_id, fix.as_deref()),
    }
}

/// `NEW`: create the goal, spawn round 1, evaluate, print hash or rejection.
fn run_new(root: &Path, goal_text: &str, context: Option<&str>) -> Result<(), String> {
    let goal_id = verifier_loop::goal::new(root, goal_text, context)
        .map_err(|e| format!("NEW failed: {e}"))?;
    let round = 1u32;
    println!("goalId: {goal_id}");
    run_round(root, &goal_id, round, None, RoundKind::New)
}

/// `RESUME`: increment the round, append fix notes, respawn, evaluate.
fn run_resume(root: &Path, goal_id: &str, fix: Option<&str>) -> Result<(), String> {
    let round = verifier_loop::goal::resume(root, goal_id, fix)
        .map_err(|e| format!("RESUME failed: {e}"))?;
    run_round(root, goal_id, round, fix, RoundKind::Resume)
}

#[derive(Clone, Copy)]
enum RoundKind {
    New,
    Resume,
}

/// Shared round driver: snapshot → render → spawn → gather → evaluate → hash/reject.
fn run_round(
    root: &Path,
    goal_id: &str,
    round: u32,
    fix_notes: Option<&str>,
    kind: RoundKind,
) -> Result<(), String> {
    let config = verifier_loop::store::Config::load_in(root).map_err(|e| format!("config: {e}"))?;
    let record = verifier_loop::goal::load(root, goal_id).map_err(|e| format!("goal load: {e}"))?;

    // Frozen artifact snapshot (§9): captured once per round from cwd. Fails closed if cwd
    // is not a git work tree (V* must never receive a silently empty snapshot).
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let snapshot = verifier_loop::prompt::capture_snapshot(&cwd, config.git_diff_max_chars)
        .map_err(|e| format!("snapshot capture failed: {e}"))?;

    let adapter = resolve_adapter(&config)?;

    // Render + persist the verifier prompt per verifier slot (correct audit trail). The
    // spawn layer takes a single prompt per round (its API), so the round's spawned
    // verifiers all receive the v1 render; for the deterministic stub backend this is
    // irrelevant (the stub ignores the prompt), and for real backends verifier identity is
    // additionally conveyed via VERIFIER_LOOP_VERIFIER_ID. See KNOWN LIMITATION below.
    let goal_root = verifier_loop::goal::goal_dir(root, goal_id);
    let m = config.m as usize;
    let mut rendered_prompts: Vec<String> = Vec::with_capacity(m);
    for i in 0..m {
        let vid = verifier_id(i);
        let prev_notes = if matches!(kind, RoundKind::Resume) {
            prev_round_own_notes(root, goal_id, &vid, round)
        } else {
            None
        };
        let vars = verifier_loop::prompt::PromptVars {
            goal_id,
            verifier_id: &vid,
            round,
            prev_round: prev_round_of(round, kind),
            goal_text: &record.goal_text,
            context: record.context.as_deref(),
            fix_notes,
            prev_notes: prev_notes.as_deref(),
            cwd: &snapshot.cwd,
            git_status: &snapshot.git_status,
            file_edit_times: &snapshot.file_edit_times,
            git_diff: &snapshot.git_diff,
            git_diff_max_chars: snapshot.git_diff_max_chars,
            truncated: snapshot.truncated,
        };
        let rendered = match kind {
            RoundKind::New => verifier_loop::prompt::render(None, &vars),
            RoundKind::Resume => verifier_loop::prompt::render_resume(None, &vars),
        }
        .map_err(|e| format!("prompt render failed: {e}"))?;
        verifier_loop::prompt::write_initial_prompt(&goal_root, goal_id, &vid, round, &rendered)
            .map_err(|e| format!("initial-prompt persist failed: {e}"))?;
        rendered_prompts.push(rendered);
    }

    // KNOWN LIMITATION: spawn_round / spawn_resume accept a single prompt per round, so for
    // m>1 every verifier receives the v1 render (verifier identity still arrives via env).
    // The per-verifier initial-prompt.txt files above are correct. A per-verifier spawn API
    // would be a §5 change (out of scope here).
    let prompt = rendered_prompts.first().cloned().unwrap_or_default();

    // Drive the async spawn in a dedicated runtime (the bin is sync).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    let input = verifier_loop::spawn::SpawnInput {
        root,
        goal_id,
        round,
        config: &config,
        prompt: &prompt,
        adapter: &adapter,
    };
    rt.block_on(async {
        match kind {
            RoundKind::New => verifier_loop::spawn::spawn_round(input).await,
            RoundKind::Resume => verifier_loop::spawn::spawn_resume(input).await,
        }
    })
    .map_err(|e| format!("spawn failed: {e}"))?;

    // Gather verdicts for every verifier slot (missing → null → fail-closed).
    let mut verdicts: Vec<(String, verifier_loop::verdict::VerdictRecord)> = Vec::new();
    for i in 0..m {
        let vid = verifier_id(i);
        let rec = verdict::read_verdict(root, goal_id, &vid, round)
            .map_err(|e| format!("verdict read {vid}: {e}"))?;
        verdicts.push((vid, rec));
    }

    let result = verifier_loop::consensus::evaluate(&verdicts, config.n, config.m);
    if result.passed {
        let salt = verifier_loop::store::salt_in(root).map_err(|e| format!("salt: {e}"))?;
        let sig_record: verifier_loop::goal::SignatureRecord = serde_json::from_str(
            &std::fs::read_to_string(goal_root.join(verifier_loop::goal::SIGNATURE_FILE))
                .map_err(|e| format!("signature read: {e}"))?,
        )
        .map_err(|e| format!("signature parse: {e}"))?;
        let matched_at = Utc::now().to_rfc3339();
        let hash = verifier_loop::consensus::compute_hash(
            &salt,
            goal_id,
            &sig_record.signature,
            round,
            &result.matching_verdicts,
            &matched_at,
        );
        verifier_loop::consensus::write_completion(root, goal_id, &result, round, &hash, &matched_at)
            .map_err(|e| format!("completion write: {e}"))?;
        println!("{}", hash.short_hash());
        Ok(())
    } else {
        // Surface the rejection: REJECT notes + null markers (consensus-check spec).
        eprintln!("round {round} did not reach {}/{} consensus", result.approve_count, config.m);
        for (vid, notes) in &result.rejection.reject_notes {
            eprintln!("  {vid} REJECT: {notes}");
        }
        if !result.rejection.null_verifiers.is_empty() {
            eprintln!(
                "  no verdict from: {}",
                result.rejection.null_verifiers.join(", ")
            );
        }
        Err(format!("round {round} rejected"))
    }
}

/// Resolve the backend adapter: built-in (pi/hermes/acpx) first, else a stub/custom
/// command from env. Keeps `acp/` untouched while enabling hermetic e2e.
fn resolve_adapter(
    config: &verifier_loop::store::Config,
) -> Result<verifier_loop::acp::Adapter, String> {
    if let Ok(a) = verifier_loop::acp::adapter_for(&config.backend) {
        return Ok(a);
    }
    let spawn_cmd = std::env::var(ENV_BACKEND_CMD)
        .or_else(|_| std::env::var(ENV_SPAWN_CMD))
        .map_err(|_| {
            format!(
                "unknown backend '{}' and no ${ENV_BACKEND_CMD} / ${ENV_SPAWN_CMD} override set",
                config.backend
            )
        })?;
    let resume_cmd = std::env::var(ENV_RESUME_CMD).unwrap_or_else(|_| spawn_cmd.clone());
    Ok(verifier_loop::acp::Adapter::custom(spawn_cmd, resume_cmd))
}

/// `v1`, `v2`, … mirroring the spawn layer's id scheme.
fn verifier_id(idx: usize) -> String {
    format!("v{}", idx + 1)
}

/// The previous round number for a RESUME, else `None`.
fn prev_round_of(round: u32, kind: RoundKind) -> Option<u32> {
    match kind {
        RoundKind::Resume => Some(round.saturating_sub(1)),
        RoundKind::New => None,
    }
}

/// This verifier's OWN prior-round notes (only meaningful on RESUME; REJECT carries notes).
/// A prior APPROVE / null yields no prev notes (blindness: never a peer's notes).
fn prev_round_own_notes(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
) -> Option<String> {
    let prev = round.checked_sub(1)?;
    let rec = verdict::read_verdict(root, goal_id, verifier_id, prev).ok()?;
    match rec.status {
        VerdictStatus::Reject => rec.notes,
        _ => None,
    }
}

/// Resolve the store root from `VERIFIER_LOOP_HOME` or the default `~/.verifier-loop`.
fn resolve_home() -> Result<PathBuf, String> {
    if let Some(v) = std::env::var_os(ENV_HOME) {
        return Ok(PathBuf::from(v));
    }
    match std::env::var_os("HOME") {
        Some(h) => Ok(PathBuf::from(h).join(DEFAULT_HOME_DIR)),
        None => Err(format!("{ENV_HOME} is unset and $HOME is not available")),
    }
}
