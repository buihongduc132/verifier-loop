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
    Approve,
    /// Register a REJECT verdict; `--notes` is required (non-empty).
    Reject {
        /// Required: the reason for rejection. Must be non-empty.
        #[arg(long)]
        notes: String,
    },
}

fn main() {
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

    let result = match cli.command {
        Cmd::Approve => verdict::register_approve(&root, &goal_id, &verifier_id, round),
        Cmd::Reject { ref notes } => {
            verdict::register_reject(&root, &goal_id, &verifier_id, round, notes)
        }
    };

    result.map_err(|e| match e {
        VerdictError::NotesRequired => "reject requires non-empty --notes".to_string(),
        VerdictError::AlreadyFinal => "verdict is already final; cannot be overwritten".to_string(),
        VerdictError::GoalNotFound => format!("goal not found: {e}"),
        other => other.to_string(),
    })
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
    std::env::var(env_key)
        .map_err(|_| format!("{label} not set (expected ${env_key})"))
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
