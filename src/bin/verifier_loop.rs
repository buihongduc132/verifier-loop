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
use verifier_loop::health;
use verifier_loop::round_recover::{self, RecoverOutcome, RoundRecoverError};
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
    // Initialize tracing (fail-open, design D5): errors are swallowed and logged
    // to stderr; a broken logger never blocks a verdict or hash. Store root is
    // resolved lazily inside run() — pass None here so init wires the stderr + OTLP
    // layers; the per-goal JSONL file layer resolves its path from env at write time.
    let _ = verifier_loop::observe::init(None);
    let cli = VerifierLoopCli::parse();
    let code = match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    };
    // Flush + shut down the OTLP tracer before exit so in-flight spans are not
    // lost (design D3). No-op when OTLP is not configured / feature off.
    verifier_loop::observe::shutdown();
    code
}

fn run(cli: &VerifierLoopCli) -> Result<(), String> {
    let root = resolve_home()?;
    let config =
        verifier_loop::store::Config::load_in(&root).map_err(|e| format!("config: {e}"))?;
    // Load the custom verifier-prompt preamble (if configured) BEFORE any goal dir / signature
    // is written, so a missing/unreadable file fails closed with NO side effects.
    let prepend = load_verifier_prompt_file(&root, config.verifier_prompt_file.as_deref())?;
    match cli.command {
        VerifierLoopCmd::New {
            ref goal,
            ref context,
        } => run_new(&root, &config, goal, context.as_deref(), prepend.as_deref())?,
        VerifierLoopCmd::Resume {
            ref goal_id,
            ref fix,
        } => run_resume(&root, &config, goal_id, fix.as_deref(), prepend.as_deref())?,
        VerifierLoopCmd::Recover { ref goal_id } => run_recover(&root, &config, goal_id)?,
        VerifierLoopCmd::Status { ref goal_id } => run_status(&root, &config, goal_id)?,
    }
    Ok(())
}

/// `NEW`: create the goal, spawn round 1, evaluate, print hash or rejection.
fn run_new(
    root: &Path,
    config: &verifier_loop::store::Config,
    goal_text: &str,
    context: Option<&str>,
    prepend: Option<&str>,
) -> Result<(), String> {
    // Validate goalText BEFORE any goal dir / signature is written (fail-closed).
    validate_goal_text(goal_text, config.min_goal_chars)?;

    let goal_id = verifier_loop::goal::new(root, goal_text, context)
        .map_err(|e| format!("NEW failed: {e}"))?;
    // LD5: hold the exclusive goal lock for the whole round (spawn+gather+evaluate).
    // Acquired AFTER goal::new creates the goal dir (the lock file lives under it).
    let _lock = acquire_goal_lock(root, &goal_id)?;
    let round = 1u32;
    println!("goalId: {goal_id}");
    run_round(root, config, &goal_id, round, None, RoundKind::New, prepend)
}

/// `RESUME`: increment the round, append fix notes, respawn, evaluate.
fn run_resume(
    root: &Path,
    config: &verifier_loop::store::Config,
    goal_id: &str,
    fix: Option<&str>,
    prepend: Option<&str>,
) -> Result<(), String> {
    // LD5: hold the exclusive goal lock for the whole round.
    let _lock = acquire_goal_lock(root, goal_id)?;
    // LD3 symmetric warning: if the current round has null verdicts and no completion,
    // suggest RECOVER first (the user may have meant to harvest in-flight verdicts).
    // RESUME still proceeds — it is the explicit escape hatch.
    warn_if_round_is_recoverable(root, config, goal_id);
    let round = verifier_loop::goal::resume(root, goal_id, fix)
        .map_err(|e| format!("RESUME failed: {e}"))?;
    run_round(
        root,
        config,
        goal_id,
        round,
        fix,
        RoundKind::Resume,
        prepend,
    )
}

/// `RECOVER <goalId>`: cross-process round recovery (SHAPE-1, LD8). Wait-only: polls
/// the current round's verdicts and re-evaluates consensus without spawning/killing/
/// re-rendering/re-capturing. On pass writes `completion.json` + prints the hash; on
/// dead-null exits non-zero with RESUME N+1 guidance; on already-complete warns + exits 0.
fn run_recover(
    root: &Path,
    config: &verifier_loop::store::Config,
    goal_id: &str,
) -> Result<(), String> {
    // LD3: if the round already reached consensus, there is nothing to recover — warn and
    // succeed without polling (the user likely meant RESUME N+1). If the round is already
    // decided-but-failed (needs=Resume), fail fast with the same guidance instead of
    // acquiring the lock + doing redundant disk reads only to return
    // RoundDecidedNoConsensus. Only needs=Recover (null slots or interrupted-pass) is
    // worth polling.
    let st = round_recover::status(root, goal_id, config).map_err(|e| format!("STATUS: {e}"))?;
    match st.needs {
        round_recover::GoalNeeds::Done => {
            eprintln!(
                "round {} already reached consensus; use `jewilo RESUME {goal_id}` to start a new round",
                st.round
            );
            return Ok(());
        }
        round_recover::GoalNeeds::Resume => {
            eprintln!(
                "round {} is decided but did not reach {}/{} consensus; \
                 run `jewilo RESUME {goal_id}` for a fresh round",
                st.round, config.n, config.m
            );
            return Err(format!("round {} rejected", st.round));
        }
        round_recover::GoalNeeds::Recover => {}
    }

    let timeout = std::time::Duration::from_secs(config.verifier_timeout_sec.max(1));
    let outcome = round_recover::recover(root, goal_id, config, timeout)
        .map_err(|e| format!("RECOVER: {e}"))?;
    match outcome {
        RecoverOutcome::ConsensusPassed(hash) => {
            println!("{hash}");
            Ok(())
        }
        RecoverOutcome::RoundDecidedNoConsensus => {
            eprintln!(
                "round {} is decided but did not reach {}/{} consensus; \
                 run `jewilo RESUME {goal_id}` for a fresh round",
                st.round, config.n, config.m
            );
            Err(format!("round {} rejected", st.round))
        }
        RecoverOutcome::StillNullAfter {
            null_slots,
            guidance,
        } => {
            eprintln!(
                "round {} still has null verdict slots ({}); {guidance}",
                st.round,
                null_slots.join(", ")
            );
            Err(format!(
                "round {} not recoverable (null slots after timeout)",
                st.round
            ))
        }
    }
}

/// `STATUS <goalId>`: read-only machine-readable goal state (LD7). Prints one JSON object
/// to stdout. Takes NO goal lock (a status probe must never block on a long round).
fn run_status(
    root: &Path,
    config: &verifier_loop::store::Config,
    goal_id: &str,
) -> Result<(), String> {
    let st = round_recover::status(root, goal_id, config).map_err(|e| format!("STATUS: {e}"))?;
    // Pretty-printed JSON to stdout; the on-disk contract is camelCase.
    println!(
        "{}",
        serde_json::to_string_pretty(&st).map_err(|e| format!("STATUS serialize: {e}"))?
    );
    Ok(())
}

/// Acquire the exclusive goal lock (LD5). Maps `GoalBusy` to a clear, user-facing message
/// and exits the operation non-zero.
fn acquire_goal_lock(root: &Path, goal_id: &str) -> Result<round_recover::GoalLock, String> {
    round_recover::GoalLock::acquire_exclusive(root, goal_id).map_err(|e| match e {
        RoundRecoverError::GoalBusy => {
            format!("goal {goal_id} busy; another NEW/RESUME/RECOVER is in progress")
        }
        other => format!("goal lock: {other}"),
    })
}

/// LD3 symmetric warning: emit a stderr hint suggesting RECOVER when the current round
/// has null verdicts and no completion (a live orphan may still be about to write one).
/// Non-fatal — RESUME is the user's explicit escape hatch and still proceeds.
fn warn_if_round_is_recoverable(root: &Path, config: &verifier_loop::store::Config, goal_id: &str) {
    if let Ok(st) = round_recover::status(root, goal_id, config) {
        if matches!(st.needs, round_recover::GoalNeeds::Recover) {
            eprintln!(
                "warning: round {} has null verdict slots; consider `jewilo RECOVER {goal_id}` \
                 first to harvest in-flight verdicts before starting a new round",
                st.round
            );
        }
    }
}

#[derive(Clone, Copy)]
enum RoundKind {
    New,
    Resume,
}

/// Shared round driver: snapshot → render → spawn → gather → evaluate → hash/reject.
fn run_round(
    root: &Path,
    config: &verifier_loop::store::Config,
    goal_id: &str,
    round: u32,
    fix_notes: Option<&str>,
    kind: RoundKind,
    prepend: Option<&str>,
) -> Result<(), String> {
    // Top-level round span (add-otel-observability lifecycle-tracing spec). Carries
    // the goal/round/traceId so every nested phase correlates to one round of one goal.
    let trace_id = verifier_loop::observe::ensure_goal_trace_id(root, goal_id).ok();
    let kind_str = match kind {
        RoundKind::New => "NEW",
        RoundKind::Resume => "RESUME",
    };
    // Record a round-start event in the per-goal trace.jsonl (trace-export spec).
    // Fail-open: a write error is swallowed inside append_trace_event.
    let _ = verifier_loop::observe::append_trace_event(
        root,
        goal_id,
        "info",
        "jewilo.round.start",
        serde_json::json!({ "kind": kind_str, "round": round }),
    );
    let round_span = tracing::info_span!(
        "jewilo.round",
        goalId = %goal_id,
        round = round,
        traceId = %trace_id.as_deref().unwrap_or(""),
        kind = %kind_str,
    );
    let _guard = round_span.enter();

    // Health self-awareness (intention 2026-07-14 feature a): if the store is in
    // cooldown (>3 unhealthy verifier runs in the last hour), do NOT spawn verifiers.
    // Instead return the recognizable fallback hash `<mmddyy>-ffffff` so the outer
    // driving process is not completely blocked. This does NOT weaken fail-closed
    // invariants — it returns a clearly-marked fallback, never an APPROVE or a real
    // consensus hash.
    if health::in_cooldown(root, Utc::now()) {
        let fb = health::fallback_hash();
        eprintln!(
            "cooldown: >{} unhealthy verifier runs in the last hour; \
             returning fallback hash {fb} (no verifiers spawned)",
            health::cooldown_threshold()
        );
        let _ = verifier_loop::observe::append_trace_event(
            root,
            goal_id,
            "warn",
            "jewilo.cooldown.fallback",
            serde_json::json!({ "fallbackHash": fb, "round": round }),
        );
        println!("{fb}");
        return Ok(());
    }

    let record = verifier_loop::goal::load(root, goal_id).map_err(|e| format!("goal load: {e}"))?;
    tracing::debug!(fields = ?record.goal_text.len(), "goal loaded");

    // Frozen artifact snapshot (§9): captured once per round from cwd. Fails closed if cwd
    // is not a git work tree (V* must never receive a silently empty snapshot). The
    // fileEditTimes block is capped to Config.file_edit_times_max_chars (D1).
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let snapshot = verifier_loop::prompt::capture_snapshot_with(
        &cwd,
        config.git_diff_max_chars,
        config.file_edit_times_max_chars,
    )
    .map_err(|e| format!("snapshot capture failed: {e}"))?;

    // Cap the --context input to Config.context_max_chars (D3).
    let context_capped: Option<String> = record
        .context
        .as_deref()
        .map(|c| verifier_loop::prompt::cap_context(c, config.context_max_chars).0);

    let adapter = resolve_adapter(config)?;

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
            context: context_capped.as_deref(),
            fix_notes,
            prev_notes: prev_notes.as_deref(),
            cwd: &snapshot.cwd,
            git_status: &snapshot.git_status,
            file_edit_times: &snapshot.file_edit_times,
            git_diff: &snapshot.git_diff,
            git_diff_max_chars: snapshot.git_diff_max_chars,
            truncated: snapshot.truncated,
        };
        // Design D2 (override semantics): when a custom verifierPromptFile is set
        // (`prepend.is_some()`), render the body WITHOUT the built-in VERIFIER_POLICY
        // block, then prepend the custom file. The two policy sources are mutually
        // exclusive — the custom file REPLACES the built-in policy, not supplements it
        // (eliminating the 2x / ~62KB duplication D2 targets). When no custom file is
        // set, the built-in policy template is used as today.
        let has_custom = prepend.is_some();
        let rendered = match (kind, has_custom) {
            (RoundKind::New, false) => verifier_loop::prompt::render(None, &vars),
            (RoundKind::New, true) => verifier_loop::prompt::render(
                Some(verifier_loop::prompt::default_template_no_policy()),
                &vars,
            ),
            (RoundKind::Resume, false) => verifier_loop::prompt::render_resume(None, &vars),
            (RoundKind::Resume, true) => verifier_loop::prompt::render_resume(
                Some(verifier_loop::prompt::default_resume_template_no_policy()),
                &vars,
            ),
        }
        .map_err(|e| format!("prompt render failed: {e}"))?;
        let rendered = verifier_loop::prompt::prepend_custom(rendered, prepend);
        // Feature b (intention 2026-07-14): build the prompt dynamically by collecting
        // ALL prior REJECT verdict notes for this goal and appending them so the verifier
        // sees the rejection history and can verify fixes. No-op when there are no prior
        // rejects (e.g. round 1, or all-prior-APPROVE).
        let prior_reject_notes =
            verifier_loop::prompt::collect_prior_reject_notes(root, goal_id, round);
        let rendered = verifier_loop::prompt::append_prior_reject_notes(&rendered, &prior_reject_notes);
        verifier_loop::prompt::write_initial_prompt(&goal_root, goal_id, &vid, round, &rendered)
            .map_err(|e| format!("initial-prompt persist failed: {e}"))?;
        rendered_prompts.push(rendered);
    }

    // Prompt-budget warning (D4): if the rendered prompt exceeds Config.prompt_budget_bytes,
    // emit a per-section breakdown to stderr. Does NOT block spawn.
    if let Some(warning) = verifier_loop::prompt::budget_warning(
        rendered_prompts.first().map(|s| s.as_str()).unwrap_or(""),
        config.prompt_budget_bytes as usize,
    ) {
        eprint!("{warning}");
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
        config,
        prompt: &prompt,
        adapter: &adapter,
    };
    let runs = rt.block_on(async {
        let _spawn_span = tracing::info_span!("jewilo.spawn", m = config.m).entered();
        match kind {
            RoundKind::New => verifier_loop::spawn::spawn_round(input).await,
            RoundKind::Resume => verifier_loop::spawn::spawn_resume(input).await,
        }
    })
    .map_err(|e| format!("spawn failed: {e}"))?;

    // Health self-awareness (intention 2026-07-14 feature a): record any unhealthy
    // verifier run to the store-wide health.jsonl so repeated backend failures trip
    // cooldown (see the cooldown check at the top of this function). Best-effort: a
    // write error is swallowed (health tracking must never block a verdict).
    let now = Utc::now();
    let unhealthy = runs.iter().filter(|r| health::is_run_unhealthy(r)).count();
    for _ in 0..unhealthy {
        let _ = health::record_unhealthy_at(root, now);
    }

    // Gather verdicts for every verifier slot (missing → null → fail-closed).
    let mut verdicts: Vec<(String, verifier_loop::verdict::VerdictRecord)> = Vec::new();
    for i in 0..m {
        let vid = verifier_id(i);
        let rec = verdict::read_verdict(root, goal_id, &vid, round)
            .map_err(|e| format!("verdict read {vid}: {e}"))?;
        verdicts.push((vid, rec));
    }

    let result =
        verifier_loop::consensus::evaluate(root, goal_id, round, &verdicts, config.n, config.m);
    // Consensus span + result event (lifecycle-tracing spec): records pass/fail +
    // the rejection summary (rejects, nulls, sig failures) under the round span.
    let consensus_span = tracing::info_span!(
        "jewilo.consensus",
        approveCount = result.approve_count,
        n = result.n,
        m = result.m,
    );
    let _cg = consensus_span.enter();
    if result.passed {
        let salt = verifier_loop::store::salt_in(root).map_err(|e| format!("salt: {e}"))?;
        let sig_record: verifier_loop::goal::SignatureRecord = serde_json::from_str(
            &std::fs::read_to_string(goal_root.join(verifier_loop::goal::SIGNATURE_FILE))
                .map_err(|e| format!("signature read: {e}"))?,
        )
        .map_err(|e| format!("signature parse: {e}"))?;
        let matched_at = Utc::now().to_rfc3339();
        let receipt_head = verifier_loop::receipt::read_receipt_head(root, goal_id);
        let hash = verifier_loop::consensus::compute_hash(
            &salt,
            goal_id,
            &sig_record.signature,
            round,
            &result.matching_verdicts,
            &matched_at,
            &receipt_head,
        );
        verifier_loop::consensus::write_completion(
            root,
            goal_id,
            &result,
            round,
            &hash,
            &matched_at,
            // Record the goal's trace id on completion.json as metadata (NOT a hash
            // input, design D4). Fail-open: an unreadable trace-id → None.
            verifier_loop::observe::ensure_goal_trace_id(root, goal_id)
                .ok()
                .as_deref(),
        )
        .map_err(|e| format!("completion write: {e}"))?;
        tracing::info!(matchedAt = %matched_at, "consensus reached");
        let _ = verifier_loop::observe::append_trace_event(
            root,
            goal_id,
            "info",
            "jewilo.consensus.passed",
            serde_json::json!({ "matchedAt": matched_at, "hash": hash.short_hash() }),
        );
        println!("{}", hash.short_hash());
        Ok(())
    } else {
        // Structured rejection event under the consensus span (lifecycle-tracing spec).
        tracing::warn!(
            rejectCount = result.rejection.reject_notes.len(),
            nullCount = result.rejection.null_verifiers.len(),
            sigFailureCount = result.rejection.signature_failures.len(),
            "round rejected"
        );
        let _ = verifier_loop::observe::append_trace_event(
            root,
            goal_id,
            "warn",
            "jewilo.consensus.rejected",
            serde_json::json!({
                "rejectCount": result.rejection.reject_notes.len(),
                "nullCount": result.rejection.null_verifiers.len(),
                "sigFailureCount": result.rejection.signature_failures.len(),
            }),
        );
        // Surface the rejection: REJECT notes + null markers (consensus-check spec).
        eprintln!(
            "round {round} did not reach {}/{} consensus",
            result.approve_count, config.m
        );
        for (vid, notes) in &result.rejection.reject_notes {
            eprintln!("  {vid} REJECT: {notes}");
        }
        if !result.rejection.null_verifiers.is_empty() {
            eprintln!(
                "  no verdict from: {}",
                result.rejection.null_verifiers.join(", ")
            );
            // Surface captured stderr so a crashed backend's error reaches the user
            // instead of a silent null verdict. Truncated to the first 10 lines to
            // avoid flooding the console on a chatty backend.
            for vid in &result.rejection.null_verifiers {
                let stderr_path = verifier_loop::goal::goal_dir(root, goal_id)
                    .join(verifier_loop::goal::ROUNDS_DIR)
                    .join(round.to_string())
                    .join(vid)
                    .join(verifier_loop::spawn::STDERR_FILE);
                if let Ok(text) = std::fs::read_to_string(&stderr_path) {
                    if !text.trim().is_empty() {
                        let preview: String = text.lines().take(10).collect::<Vec<_>>().join("\n");
                        eprintln!("  {vid} stderr:\n{preview}");
                    }
                }
            }
        }
        if !result.rejection.signature_failures.is_empty() {
            eprintln!("  signature failures:");
            for (vid, reason) in &result.rejection.signature_failures {
                eprintln!("    {vid}: {reason}");
            }
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

/// Validates `goal_text` against the empty/whitespace invariant and `min_goal_chars`.
/// Empty/whitespace-only is ALWAYS an error (regardless of `min_goal_chars`). When
/// `min_goal_chars > 0`, a trimmed length below it is an error. Errors out BEFORE any goal
/// dir / signature is written.
fn validate_goal_text(goal_text: &str, min_goal_chars: u64) -> Result<(), String> {
    let trimmed_len = goal_text.trim().chars().count() as u64;
    if trimmed_len == 0 {
        return Err(
            "goal text is empty or whitespace-only; a non-empty goal is required".to_string(),
        );
    }
    if min_goal_chars > 0 && trimmed_len < min_goal_chars {
        return Err(format!(
            "goal text is {trimmed_len} chars, below the configured minGoalChars of {min_goal_chars}"
        ));
    }
    Ok(())
}

/// Loads the custom verifier-prompt preamble file, if configured.
/// Relative paths resolve against `home`; absolute paths are used as-is. A
/// missing/unreadable file is a hard error (fail-closed: no goal dir / signature written).
/// Returns `None` when no `verifierPromptFile` is configured (today's default behavior).
fn load_verifier_prompt_file(
    home: &Path,
    configured: Option<&str>,
) -> Result<Option<String>, String> {
    let rel = match configured {
        Some(p) => p,
        None => return Ok(None),
    };
    let resolved = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        home.join(rel)
    };
    std::fs::read_to_string(&resolved).map(Some).map_err(|e| {
        format!(
            "verifier prompt file '{}' could not be read: {e}",
            resolved.display()
        )
    })
}
