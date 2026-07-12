//! `verifier-verdict` (aliased `jewije`) — V*'s interface (approve / reject).
//!
//! tasks.md §7 / verdict-registration spec. Verifiers register their verdict by invoking
//! this binary with `approve` or `reject --notes "..."`. Identity is resolved from the
//! `VERIFIER_LOOP_*` env (D2): env always wins over any argument. The store root comes
//! from `VERIFIER_LOOP_HOME` (defaulting to `~/.verifier-loop`).
//!
//! Exit codes:
//!   * success → prints `Verdict registered`, exit 0.
//!   * failure → prints an error to stderr, exits non-zero (notes-required, already-final,
//!     missing identity, goal/store missing).

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use verifier_loop::verdict::{self, VerdictError};

/// `VERIFIER_LOOP_HOME` overrides the store root; otherwise `~/.verifier-loop`.
const ENV_HOME: &str = "VERIFIER_LOOP_HOME";
/// Identity env vars (D2). Env always wins over args; missing env → fail closed.
const ENV_GOAL_ID: &str = "VERIFIER_LOOP_GOAL_ID";
const ENV_VERIFIER_ID: &str = "VERIFIER_LOOP_VERIFIER_ID";
const ENV_ROUND: &str = "VERIFIER_LOOP_ROUND";
/// Verifier signing secret (verdict-registration MODIFIED spec). A 64-hex Ed25519
/// signing key whose deriving pubkey must match the slot's pinned verifier-pubkey.json.
/// Required when the slot has a pinned pubkey; refused (Unauthenticated) if supplied
/// for a slot without one.
const ENV_VERIFIER_SECRET: &str = "VERIFIER_LOOP_VERIFIER_SECRET";
const DEFAULT_HOME_DIR: &str = ".verifier-loop";

#[derive(Debug, Parser)]
#[command(
    name = "verifier-verdict",
    about = "Register a verifier verdict (approve / reject --notes)."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Register an APPROVE verdict for this verifier's slot.
    ///
    /// `--notes` (or `-n`) is OPTIONAL on approve (design D1). When supplied and
    /// non-empty, the notes are stored on the verdict record; when omitted or empty,
    /// behavior is unchanged (legacy approve). Reject keeps `--notes` required.
    Approve {
        /// Optional approval evidence/notes. Trimmed; empty/whitespace -> no notes key.
        #[arg(long, short = 'n')]
        notes: Option<String>,
    },
    /// Register a REJECT verdict; `--notes` is required (non-empty).
    Reject {
        /// Required: the reason for rejection. Must be non-empty.
        #[arg(long)]
        notes: String,
    },
}

fn main() {
    // Initialize tracing (fail-open, design D5). jewije runs inside the spawned V*
    // process; VERIFIER_LOOP_TRACE_ID (set by the spawning jewilo) is picked up by
    // the receipt layer at append time and by tracing spans via the subscriber.
    let _ = verifier_loop::observe::init(None);
    let cli = Cli::parse();
    let code = match run(&cli) {
        Ok(()) => {
            println!("Verdict registered");
            0
        }
        Err(msg) => {
            eprintln!("{msg}");
            1
        }
    };
    std::process::exit(code);
}

fn run(cli: &Cli) -> Result<(), String> {
    let root = resolve_home()?;
    let goal_id = resolve_required(ENV_GOAL_ID, "goal id")?;
    let verifier_id = resolve_required(ENV_VERIFIER_ID, "verifier id")?;
    let round = resolve_round()?;

    // Verdict registration span (lifecycle-tracing spec). The traceId comes from
    // VERIFIER_LOOP_TRACE_ID (propagated by the spawning jewilo) or is empty when
    // jewije is invoked manually — the receipt layer records whichever is active.
    let trace_id = verifier_loop::observe::trace_id_from_env();
    let kind = match cli.command {
        Cmd::Approve { .. } => "approve",
        Cmd::Reject { .. } => "reject",
    };
    let _span = tracing::info_span!(
        "jewije.register",
        goalId = %goal_id,
        verifierId = %verifier_id,
        round = round,
        traceId = %trace_id.as_deref().unwrap_or(""),
        kind = kind,
    )
    .entered();

    // Resolve the optional signing secret. A missing/empty secret is legal only for
    // slots in the legacy (unsigned) regime — see the regime gate below.
    let secret_hex = std::env::var(ENV_VERIFIER_SECRET)
        .ok()
        .filter(|s| !s.is_empty());
    let signing_key = match secret_hex.as_deref() {
        Some(h) => Some(
            verifier_loop::crypto::signing_key_from_hex(h)
                .map_err(|e| format!("unauthenticated: invalid verifier secret: {e}"))?,
        ),
        None => None,
    };

    // Regime gate: the slot's pinned verifier pubkey presence determines whether a
    // secret is required. Pinned + no secret, or secret + no pin, are both
    // Unauthenticated (fail closed). Both absent → legacy unsigned path. Both present
    // (and matching) → signed path.
    let pinned = verdict::read_pinned_pubkey(&root, &goal_id, &verifier_id, round)
        .map_err(|e| e.to_string())?;

    let result = match (&cli.command, pinned, signing_key.as_ref()) {
        (Cmd::Approve { ref notes }, None, None) => {
            verdict::register_approve(&root, &goal_id, &verifier_id, round, notes.as_deref())
        }
        (Cmd::Approve { ref notes }, Some(_), Some(sk)) => verdict::register_signed_approve(
            &root,
            &goal_id,
            &verifier_id,
            round,
            notes.as_deref(),
            sk,
        ),
        (Cmd::Approve { .. }, _, None) => Err(VerdictError::Unauthenticated(
            "verifier secret missing; set $VERIFIER_LOOP_VERIFIER_SECRET".to_string(),
        )),
        (Cmd::Approve { .. }, None, Some(_)) => Err(VerdictError::Unauthenticated(
            "no pinned verifier pubkey for this slot".to_string(),
        )),
        (Cmd::Reject { ref notes }, None, None) => {
            verdict::register_reject(&root, &goal_id, &verifier_id, round, notes)
        }
        (Cmd::Reject { ref notes }, Some(_), Some(sk)) => {
            verdict::register_signed_reject(&root, &goal_id, &verifier_id, round, notes, sk)
        }
        (Cmd::Reject { .. }, _, None) => Err(VerdictError::Unauthenticated(
            "verifier secret missing; set $VERIFIER_LOOP_VERIFIER_SECRET".to_string(),
        )),
        (Cmd::Reject { .. }, None, Some(_)) => Err(VerdictError::Unauthenticated(
            "no pinned verifier pubkey for this slot".to_string(),
        )),
    };

    result
        .map(|()| {
            // Record a verdict-registered event in the per-goal trace.jsonl (trace-export
            // spec). Fail-open: a write error is swallowed inside append_trace_event.
            let status = match cli.command {
                Cmd::Approve { .. } => "APPROVE",
                Cmd::Reject { .. } => "REJECT",
            };
            let _ = verifier_loop::observe::append_trace_event(
                &root,
                &goal_id,
                "info",
                "jewije.registered",
                serde_json::json!({
                    "verifierId": verifier_id,
                    "round": round,
                    "status": status,
                }),
            );
        })
        .map_err(map_verdict_error)
}

/// Map a `VerdictError` to a user-facing stderr string. Each arm fails closed.
fn map_verdict_error(e: VerdictError) -> String {
    match e {
        VerdictError::NotesRequired => "reject requires non-empty --notes".to_string(),
        VerdictError::AlreadyFinal => "verdict is already final; cannot be overwritten".to_string(),
        VerdictError::GoalNotFound => format!("goal not found: {e}"),
        other => other.to_string(),
    }
}

/// Resolve the store root from `VERIFIER_LOOP_HOME` or the default `~/.verifier-loop`.
fn resolve_home() -> Result<PathBuf, String> {
    if let Some(v) = std::env::var_os(ENV_HOME) {
        return Ok(PathBuf::from(v));
    }
    match dirs_home() {
        Some(h) => Ok(h.join(DEFAULT_HOME_DIR)),
        None => Err(format!("{ENV_HOME} is unset and $HOME is not available")),
    }
}

/// Resolve a required identity value from env (env wins; there is no arg override).
fn resolve_required(env_key: &str, label: &str) -> Result<String, String> {
    std::env::var(env_key).map_err(|_| format!("{label} not set (expected ${env_key})"))
}

/// Parse the round from `VERIFIER_LOOP_ROUND` (u32).
fn resolve_round() -> Result<u32, String> {
    std::env::var(ENV_ROUND)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("round not set or invalid (expected $ {ENV_ROUND}=<u32>)"))
}

/// Minimal `$HOME` resolution without pulling in the `dirs` crate (zero new deps).
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
