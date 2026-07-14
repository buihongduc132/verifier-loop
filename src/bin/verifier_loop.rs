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
//!
//! `--json` output mode (`add-json-output-mode`): when the global `--json` flag is set,
//! every success / failure site routes through `cli::json_output::Output` so stdout carries
//! exactly ONE JSON envelope object per process invocation (design D0/D1). Legacy
//! free-text lines never leak onto stdout under `--json`. Human-readable diagnostics stay
//! on stderr in BOTH modes. On-disk artifacts, hash inputs, signature verification, and
//! exit codes are byte-identical / identical with and without `--json`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;
use clap::Parser;

use verifier_loop::cli::json_output::{JsonEnvelope, Output, RejectionBreakdown};
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

/// Coarse process outcome. `Failure` always maps to a non-zero exit code. Every site that
/// has already emitted its own (JSON or human) output returns one of these instead of
/// propagating a raw `Err(String)` so the top-level handler never double-prints.
enum Outcome {
    Success,
    Failure,
}

fn main() -> ExitCode {
    // Initialize tracing (fail-open, design D5): errors are swallowed and logged
    // to stderr; a broken logger never blocks a verdict or hash. Store root is
    // resolved lazily inside run() — pass None here so init wires the stderr + OTLP
    // layers; the per-goal JSONL file layer resolves its path from env at write time.
    let _ = verifier_loop::observe::init(None);
    let cli = VerifierLoopCli::parse();
    let output = if cli.json {
        Output::Json
    } else {
        Output::Human
    };
    let outcome = run(&cli, output);
    // Flush + shut down the OTLP tracer before exit so in-flight spans are not
    // lost (design D3). No-op when OTLP is not configured / feature off.
    verifier_loop::observe::shutdown();
    match outcome {
        Outcome::Success => ExitCode::SUCCESS,
        Outcome::Failure => ExitCode::FAILURE,
    }
}

fn run(cli: &VerifierLoopCli, output: Output) -> Outcome {
    let command = command_name(&cli.command);
    let root = match resolve_home() {
        Ok(r) => r,
        Err(msg) => return emit_error(&output, command, None, None, &msg),
    };
    let config = match verifier_loop::store::Config::load_in(&root) {
        Ok(c) => c,
        Err(e) => {
            return emit_error(&output, command, None, None, &format!("config: {e}"));
        }
    };
    // Load the custom verifier-prompt preamble (if configured) BEFORE any goal dir / signature
    // is written, so a missing/unreadable file fails closed with NO side effects.
    let prepend = match load_verifier_prompt_file(&root, config.verifier_prompt_file.as_deref()) {
        Ok(p) => p,
        Err(msg) => return emit_error(&output, command, None, None, &msg),
    };
    match cli.command {
        VerifierLoopCmd::New {
            ref goal,
            ref context,
        } => run_new(
            &root,
            &config,
            goal,
            context.as_deref(),
            prepend.as_deref(),
            &output,
        ),
        VerifierLoopCmd::Resume {
            ref goal_id,
            ref fix,
        } => run_resume(
            &root,
            &config,
            goal_id,
            fix.as_deref(),
            prepend.as_deref(),
            &output,
        ),
        VerifierLoopCmd::Recover { ref goal_id } => run_recover(&root, &config, goal_id, &output),
        VerifierLoopCmd::Status { ref goal_id } => run_status(&root, &config, goal_id, &output),
        VerifierLoopCmd::Stats { ref goal_id } => {
            run_simple_json_passthrough(&output, command, run_stats(&root, goal_id))
        }
        VerifierLoopCmd::Audit { ref goal_id } => {
            let res = run_audit(&root, goal_id);
            run_simple_json_passthrough(&output, command, res)
        }
    }
}

/// `NEW`: create the goal, spawn round 1, evaluate, print hash or rejection.
fn run_new(
    root: &Path,
    config: &verifier_loop::store::Config,
    goal_text: &str,
    context: Option<&str>,
    prepend: Option<&str>,
    output: &Output,
) -> Outcome {
    // Validate goalText BEFORE any goal dir / signature is written (fail-closed).
    if let Err(msg) = validate_goal_text(goal_text, config.min_goal_chars) {
        return emit_error(output, "new", None, None, &msg);
    }

    let goal_id = match verifier_loop::goal::new(root, goal_text, context) {
        Ok(id) => id,
        Err(e) => return emit_error(output, "new", None, None, &format!("NEW failed: {e}")),
    };
    // LD5: hold the exclusive goal lock for the whole round (spawn+gather+evaluate).
    // Acquired AFTER goal::new creates the goal dir (the lock file lives under it).
    let _lock = match acquire_goal_lock(root, &goal_id) {
        Ok(l) => l,
        Err(msg) => return emit_error(output, "new", Some(&goal_id), None, &msg),
    };
    let round = 1u32;
    // Under Human mode the legacy `goalId: <id>` line is the first stdout line. Under
    // JSON it is suppressed (the id rides inside the envelope instead).
    if matches!(output, Output::Human) {
        println!("goalId: {goal_id}");
    }
    run_round(root, config, &goal_id, round, None, RoundKind::New, prepend, output)
}

/// `RESUME`: increment the round, append fix notes, respawn, evaluate.
fn run_resume(
    root: &Path,
    config: &verifier_loop::store::Config,
    goal_id: &str,
    fix: Option<&str>,
    prepend: Option<&str>,
    output: &Output,
) -> Outcome {
    // LD5: hold the exclusive goal lock for the whole round.
    let _lock = match acquire_goal_lock(root, goal_id) {
        Ok(l) => l,
        Err(msg) => return emit_error(output, "resume", Some(goal_id), None, &msg),
    };
    // LD3 symmetric warning: if the current round has null verdicts and no completion,
    // suggest RECOVER first (the user may have meant to harvest in-flight verdicts).
    // RESUME still proceeds — it is the explicit escape hatch.
    warn_if_round_is_recoverable(root, config, goal_id);
    let round = match verifier_loop::goal::resume(root, goal_id, fix) {
        Ok(r) => r,
        Err(e) => return emit_error(output, "resume", Some(goal_id), None, &format!("RESUME failed: {e}")),
    };
    run_round(
        root,
        config,
        goal_id,
        round,
        fix,
        RoundKind::Resume,
        prepend,
        output,
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
    output: &Output,
) -> Outcome {
    let command = "recover";
    // LD3: if the round already reached consensus, there is nothing to recover — warn and
    // succeed without polling (the user likely meant RESUME N+1). If the round is already
    // decided-but-failed (needs=Resume), fail fast with the same guidance instead of
    // acquiring the lock + doing redundant disk reads only to return
    // RoundDecidedNoConsensus. Only needs=Recover (null slots or interrupted-pass) is
    // worth polling.
    let st = match round_recover::status(root, goal_id, config) {
        Ok(s) => s,
        Err(e) => return emit_error(output, command, Some(goal_id), None, &format!("STATUS: {e}")),
    };
    match st.needs {
        round_recover::GoalNeeds::Done => {
            let msg = format!(
                "round {} already reached consensus; use `jewilo RESUME {goal_id}` to start a new round",
                st.round
            );
            eprintln!("{msg}");
            let env = envelope(command, true)
                .with_goal(goal_id)
                .with_round(st.round)
                .with_status("already-done")
                .with_needs(GoalNeeds::Done);
            print_success(output, env, &format!("round {} already reached consensus", st.round));
            return Outcome::Success;
        }
        round_recover::GoalNeeds::Resume => {
            let msg = format!(
                "round {} is decided but did not reach {}/{} consensus; \
                 run `jewilo RESUME {goal_id}` for a fresh round",
                st.round, config.n, config.m
            );
            eprintln!("{msg}");
            let env = envelope(command, false)
                .with_goal(goal_id)
                .with_round(st.round)
                .with_status("rejected")
                .with_needs(GoalNeeds::Resume)
                .with_error(&format!("round {} rejected", st.round));
            print_error(output, env, &format!("round {} rejected", st.round));
            return Outcome::Failure;
        }
        round_recover::GoalNeeds::Recover => {}
    }

    let timeout = std::time::Duration::from_secs(config.verifier_timeout_sec.max(1));
    let outcome = match round_recover::recover(root, goal_id, config, timeout) {
        Ok(o) => o,
        Err(e) => return emit_error(output, command, Some(goal_id), Some(st.round), &format!("RECOVER: {e}")),
    };
    match outcome {
        RecoverOutcome::ConsensusPassed(hash) => {
            // fullDigest is not carried by the outcome; read it from the just-written
            // completion.json so the envelope matches the NEW/RESUME success shape.
            let full_digest = read_completion_full_digest(root, goal_id);
            let env = envelope(command, true)
                .with_goal(goal_id)
                .with_round(st.round)
                .with_status("consensus-passed")
                .with_hash(&hash)
                .maybe_with_full_digest(full_digest.as_deref());
            print_success(output, env, &hash);
            Outcome::Success
        }
        RecoverOutcome::RoundDecidedNoConsensus => {
            let msg = format!(
                "round {} is decided but did not reach {}/{} consensus; \
                 run `jewilo RESUME {goal_id}` for a fresh round",
                st.round, config.n, config.m
            );
            eprintln!("{msg}");
            let env = envelope(command, false)
                .with_goal(goal_id)
                .with_round(st.round)
                .with_status("rejected")
                .with_needs(GoalNeeds::Resume)
                .with_error(&format!("round {} rejected", st.round));
            print_error(output, env, &format!("round {} rejected", st.round));
            Outcome::Failure
        }
        RecoverOutcome::StillNullAfter {
            null_slots,
            guidance,
        } => {
            let msg = format!(
                "round {} still has null verdict slots ({}); {guidance}",
                st.round,
                null_slots.join(", ")
            );
            eprintln!("{msg}");
            let breakdown = RejectionBreakdown::from_unsorted(
                Vec::new(),
                null_slots.clone(),
                Vec::new(),
            );
            let env = envelope(command, false)
                .with_goal(goal_id)
                .with_round(st.round)
                .with_status("recover-null-after-timeout")
                .with_rejection(breakdown)
                .with_error(&format!(
                    "round {} not recoverable (null slots after timeout)",
                    st.round
                ));
            print_error(output, env, &format!(
                "round {} not recoverable (null slots after timeout)",
                st.round
            ));
            Outcome::Failure
        }
    }
}

/// `STATUS <goalId>`: read-only machine-readable goal state (LD7). Prints one JSON object
/// to stdout. Takes NO goal lock (a status probe must never block on a long round).
///
/// Under `--json` the legacy body is wrapped in the standard envelope: `round`, `state`,
/// `needs` are lifted to the top level and the body's `slots` array is exposed as
/// `verdicts`. Under Human mode the bare body is printed byte-identical to before this
/// change.
fn run_status(
    root: &Path,
    config: &verifier_loop::store::Config,
    goal_id: &str,
    output: &Output,
) -> Outcome {
    let st = match round_recover::status(root, goal_id, config) {
        Ok(s) => s,
        Err(e) => return emit_error(output, "status", Some(goal_id), None, &format!("STATUS: {e}")),
    };
    match output {
        Output::Human => {
            // Byte-identical legacy body (pretty JSON, no envelope wrapper).
            let body = match serde_json::to_string_pretty(&st) {
                Ok(b) => b,
                Err(e) => return emit_error(output, "status", Some(goal_id), None, &format!("STATUS serialize: {e}")),
            };
            println!("{body}");
            Outcome::Success
        }
        Output::Json => {
            // Lift round/state/needs to the envelope; pass the body's `slots` as `verdicts`.
            let body: serde_json::Value = match serde_json::to_value(&st) {
                Ok(v) => v,
                Err(e) => return emit_error(output, "status", Some(goal_id), None, &format!("STATUS serialize: {e}")),
            };
            let env = envelope("status", true)
                .with_goal(goal_id)
                .with_round_value(body.get("round").cloned())
                .with_state_value(body.get("state").cloned())
                .with_needs_value(body.get("needs").cloned())
                .with_verdicts(body.get("slots").cloned());
            print_success(output, env, "");
            Outcome::Success
        }
    }
}

/// `STATS <goalId>`: read-only aggregate of ALL stored JSON for a goal run (intention
/// 2026-07-14). Prints one JSON object to stdout. Takes NO goal lock; never spawns
/// verifiers (a stats probe must never block on a long round).
fn run_stats(root: &Path, goal_id: &str) -> Result<(), String> {
    let stats = verifier_loop::stats::collect_stats(root, goal_id)
        .map_err(|e| format!("STATS: {e}"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&stats).map_err(|e| format!("STATS serialize: {e}"))?
    );
    Ok(())
}

/// `AUDIT <goalId>`: read-only post-hoc audit of the final completion against the
/// creation-time config requirement (intention 2026-07-14). Prints a JSON report to
/// stdout; exits 0 if valid, non-zero otherwise. Takes NO goal lock; never spawns.
fn run_audit(root: &Path, goal_id: &str) -> Result<(), String> {
    let report = verifier_loop::stats::audit(root, goal_id).map_err(|e| format!("AUDIT: {e}"))?;
    let json = serde_json::to_string_pretty(&report).map_err(|e| format!("AUDIT serialize: {e}"))?;
    // Always print the report so the caller sees the reason even on an invalid audit.
    println!("{json}");
    if report.valid {
        Ok(())
    } else {
        Err("audit: completion does not match the creation-time requirement".to_string())
    }
}

/// STATS / AUDIT are out of scope for the `--json` envelope (no spec scenario); under
/// Human mode they keep their bare-JSON stdout + stderr-error behavior. Under `--json`
/// the legacy bare body is still printed (consumers tolerate extra fields); a top-level
/// `Err` is surfaced through the error envelope + non-zero exit.
fn run_simple_json_passthrough(
    output: &Output,
    command: &str,
    res: Result<(), String>,
) -> Outcome {
    match res {
        Ok(()) => Outcome::Success,
        Err(msg) => emit_error(output, command, None, None, &msg),
    }
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

impl RoundKind {
    fn command(self) -> &'static str {
        match self {
            RoundKind::New => "new",
            RoundKind::Resume => "resume",
        }
    }
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
    output: &Output,
) -> Outcome {
    let command = kind.command();
    let fail = |msg: String| -> Outcome {
        emit_error(output, command, Some(goal_id), Some(round), &msg)
    };
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
        let env = envelope(command, true)
            .with_goal(goal_id)
            .with_round(round)
            .with_status("cooldown-fallback")
            .with_hash(&fb);
        print_success(output, env, &fb);
        return Outcome::Success;
    }

    let record = match verifier_loop::goal::load(root, goal_id) {
        Ok(r) => r,
        Err(e) => return fail(format!("goal load: {e}")),
    };
    tracing::debug!(fields = ?record.goal_text.len(), "goal loaded");

    // Frozen artifact snapshot (§9): captured once per round from cwd. Fails closed if cwd
    // is not a git work tree (V* must never receive a silently empty snapshot). The
    // fileEditTimes block is capped to Config.file_edit_times_max_chars (D1).
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => return fail(format!("cwd: {e}")),
    };
    let snapshot = match verifier_loop::prompt::capture_snapshot_with(
        &cwd,
        config.git_diff_max_chars,
        config.file_edit_times_max_chars,
    ) {
        Ok(s) => s,
        Err(e) => return fail(format!("snapshot capture failed: {e}")),
    };

    // Cap the --context input to Config.context_max_chars (D3).
    let context_capped: Option<String> = record
        .context
        .as_deref()
        .map(|c| verifier_loop::prompt::cap_context(c, config.context_max_chars).0);

    let adapter = match resolve_adapter(config) {
        Ok(a) => a,
        Err(msg) => return fail(msg),
    };

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
        };
        let rendered = match rendered {
            Ok(r) => r,
            Err(e) => return fail(format!("prompt render failed: {e}")),
        };
        let rendered = verifier_loop::prompt::prepend_custom(rendered, prepend);
        // Feature b (intention 2026-07-14): build the prompt dynamically by collecting
        // ALL prior REJECT verdict notes for this goal and appending them so the verifier
        // sees the rejection history and can verify fixes. No-op when there are no prior
        // rejects (e.g. round 1, or all-prior-APPROVE).
        let prior_reject_notes =
            verifier_loop::prompt::collect_prior_reject_notes(root, goal_id, round);
        let rendered = verifier_loop::prompt::append_prior_reject_notes(&rendered, &prior_reject_notes);
        if let Err(e) = verifier_loop::prompt::write_initial_prompt(&goal_root, goal_id, &vid, round, &rendered) {
            return fail(format!("initial-prompt persist failed: {e}"));
        }
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
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return fail(format!("runtime: {e}")),
    };
    let input = verifier_loop::spawn::SpawnInput {
        root,
        goal_id,
        round,
        config,
        prompt: &prompt,
        adapter: &adapter,
    };
    let runs = match rt.block_on(async {
        let _spawn_span = tracing::info_span!("jewilo.spawn", m = config.m).entered();
        match kind {
            RoundKind::New => verifier_loop::spawn::spawn_round(input).await,
            RoundKind::Resume => verifier_loop::spawn::spawn_resume(input).await,
        }
    }) {
        Ok(r) => r,
        Err(e) => return fail(format!("spawn failed: {e}")),
    };

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
        let rec = match verdict::read_verdict(root, goal_id, &vid, round) {
            Ok(r) => r,
            Err(e) => return fail(format!("verdict read {vid}: {e}")),
        };
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
        let salt = match verifier_loop::store::salt_in(root) {
            Ok(s) => s,
            Err(e) => return fail(format!("salt: {e}")),
        };
        let sig_record: verifier_loop::goal::SignatureRecord = {
            let raw = match std::fs::read_to_string(
                goal_root.join(verifier_loop::goal::SIGNATURE_FILE),
            ) {
                Ok(s) => s,
                Err(e) => return fail(format!("signature read: {e}")),
            };
            match serde_json::from_str(&raw) {
                Ok(r) => r,
                Err(e) => return fail(format!("signature parse: {e}")),
            }
        };
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
        if let Err(e) = verifier_loop::consensus::write_completion(
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
        ) {
            return fail(format!("completion write: {e}"));
        }
        tracing::info!(matchedAt = %matched_at, "consensus reached");
        let _ = verifier_loop::observe::append_trace_event(
            root,
            goal_id,
            "info",
            "jewilo.consensus.passed",
            serde_json::json!({ "matchedAt": matched_at, "hash": hash.short_hash() }),
        );
        let env = envelope(command, true)
            .with_goal(goal_id)
            .with_round(round)
            .with_status("consensus-passed")
            .with_hash(hash.short_hash())
            .with_full_digest(hash.full_digest());
        print_success(output, env, hash.short_hash());
        Outcome::Success
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
        // These human-readable lines stay on stderr in BOTH modes (design: only stdout
        // shape changes between modes).
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
        // Under --json emit a single rejection envelope on stdout. Arrays are sorted by
        // verifierId ascending via `RejectionBreakdown::from_unsorted` (design D5).
        let breakdown = RejectionBreakdown::from_unsorted(
            result.rejection.reject_notes.clone(),
            result.rejection.null_verifiers.clone(),
            result.rejection.signature_failures.clone(),
        );
        let env = envelope(command, false)
            .with_goal(goal_id)
            .with_round(round)
            .with_status("rejected")
            .with_rejection(breakdown)
            .with_error(&format!("round {round} rejected"));
        print_error(output, env, &format!("round {round} rejected"));
        Outcome::Failure
    }
}

// ---------------------------------------------------------------------------
// `--json` envelope helpers
// ---------------------------------------------------------------------------

/// Map a parsed subcommand to its envelope `command` string.
fn command_name(cmd: &VerifierLoopCmd) -> &'static str {
    match cmd {
        VerifierLoopCmd::New { .. } => "new",
        VerifierLoopCmd::Resume { .. } => "resume",
        VerifierLoopCmd::Recover { .. } => "recover",
        VerifierLoopCmd::Status { .. } => "status",
        VerifierLoopCmd::Stats { .. } => "stats",
        VerifierLoopCmd::Audit { .. } => "audit",
    }
}

/// A small string sentinel used to carry the `needs` value onto the envelope. Mirrors the
/// `GoalNeeds` snake_case serialization ("done" | "recover" | "resume").
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum GoalNeeds {
    Done,
    Recover,
    Resume,
}

impl GoalNeeds {
    fn as_str(self) -> &'static str {
        match self {
            GoalNeeds::Done => "done",
            GoalNeeds::Recover => "recover",
            GoalNeeds::Resume => "resume",
        }
    }
}

/// Builder wrapper around `JsonEnvelope` so call sites stay readable. All optional fields
/// start `None`; the `.with_*` setters fill only what a given path carries.
struct EnvBuilder {
    inner: JsonEnvelope,
}

#[allow(dead_code)]
impl EnvBuilder {
    fn with_goal(mut self, goal_id: &str) -> Self {
        self.inner.goal_id = Some(goal_id.to_string());
        self
    }
    fn with_round(mut self, round: u32) -> Self {
        self.inner.round = Some(round);
        self
    }
    fn with_round_value(mut self, round: Option<serde_json::Value>) -> Self {
        if let Some(serde_json::Value::Null) | None = round {
            return self;
        }
        if let Some(n) = round.and_then(|v| v.as_u64()) {
            self.inner.round = Some(n as u32);
        }
        self
    }
    fn with_status(mut self, status: &str) -> Self {
        self.inner.status = Some(status.to_string());
        self
    }
    fn with_hash(mut self, hash: &str) -> Self {
        self.inner.hash = Some(hash.to_string());
        self
    }
    fn with_full_digest(mut self, digest: &str) -> Self {
        self.inner.full_digest = Some(digest.to_string());
        self
    }
    fn maybe_with_full_digest(mut self, digest: Option<&str>) -> Self {
        self.inner.full_digest = digest.map(|s| s.to_string());
        self
    }
    fn with_needs(mut self, needs: GoalNeeds) -> Self {
        self.inner.needs = Some(needs.as_str().to_string());
        self
    }
    fn with_needs_value(mut self, needs: Option<serde_json::Value>) -> Self {
        if let Some(n @ serde_json::Value::String(_)) = needs {
            self.inner.needs = Some(n.to_string().trim_matches('"').to_string());
        }
        self
    }
    fn with_state_value(mut self, state: Option<serde_json::Value>) -> Self {
        if let Some(s @ serde_json::Value::String(_)) = state {
            self.inner.state = Some(s.to_string().trim_matches('"').to_string());
        }
        self
    }
    fn with_rejection(mut self, br: RejectionBreakdown) -> Self {
        self.inner.rejection = Some(br);
        self
    }
    fn with_verdicts(mut self, v: Option<serde_json::Value>) -> Self {
        self.inner.verdicts = v;
        self
    }
    fn with_error(mut self, err: &str) -> Self {
        self.inner.error = Some(err.to_string());
        self
    }
}

/// Start a new envelope builder with the always-present `ok` + `command` fields.
fn envelope(command: &str, ok: bool) -> EnvBuilder {
    EnvBuilder {
        inner: JsonEnvelope {
            ok,
            command: command.to_string(),
            goal_id: None,
            round: None,
            verifier_id: None,
            status: None,
            hash: None,
            full_digest: None,
            needs: None,
            rejection: None,
            verdicts: None,
            state: None,
            error: None,
        },
    }
}

/// Print a successful result via the formatter. `human_line` is the legacy stdout line
/// (used verbatim under Human mode; ignored under Json).
fn print_success(output: &Output, env: EnvBuilder, human_line: &str) {
    output.print_success(&env.inner, human_line, &mut std::io::stdout());
}

/// Print a failed result via the formatter. The human-readable `human_err` mirrors to
/// stderr under Json; under Human it is the only stderr line. The structured envelope
/// goes to stdout under Json.
fn print_error(output: &Output, env: EnvBuilder, human_err: &str) {
    // The formatter's `print_error<W>` takes a single type parameter for both writers;
    // stdout and stderr are distinct concrete types, so erase them behind `Box<dyn Write>`.
    let mut out: Box<dyn Write> = Box::new(std::io::stdout());
    let mut err: Box<dyn Write> = Box::new(std::io::stderr());
    output.print_error(&env.inner, human_err, &mut out, &mut err);
}

/// Top-level error emitter used by `run()` for setup-phase failures (store / config /
/// prompt-file) and by subcommands for their own fatal errors. Always returns `Failure`.
fn emit_error(
    output: &Output,
    command: &str,
    goal_id: Option<&str>,
    round: Option<u32>,
    msg: &str,
) -> Outcome {
    let mut b = envelope(command, false).with_error(msg);
    if let Some(g) = goal_id {
        b = b.with_goal(g);
    }
    if let Some(r) = round {
        b = b.with_round(r);
    }
    print_error(output, b, msg);
    Outcome::Failure
}

/// Best-effort read of `fullDigest` from a goal's `completion.json`. Returns `None` when
/// the file is absent or unreadable (used by RECOVER success, which only carries the
/// short hash in its outcome).
fn read_completion_full_digest(root: &Path, goal_id: &str) -> Option<String> {
    let path = verifier_loop::goal::goal_dir(root, goal_id)
        .join(verifier_loop::consensus::COMPLETION_FILE);
    let raw = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("fullDigest").and_then(|d| d.as_str()).map(|s| s.to_string())
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

