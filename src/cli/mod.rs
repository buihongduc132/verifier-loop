//! CLI command definitions (tasks.md §10) for both binaries.
//!
//! * `verifier-loop` (`jewilo`):   `NEW "<goal>" [--context]`, `RESUME <goalId> [--fix "…"]`,
//!   `RECOVER <goalId>`, `STATUS <goalId>`.
//! * `verifier-verdict` (`jewije`): `approve`, `reject --notes "…"` (defined inline in its
//!   own bin; this module holds the `verifier-loop` shared command structs so the bin stays
//!   a thin dispatch layer over the lib).
//!
//! Bin targets live in `src/bin/`; this module holds the clap command structs consumed by
//! the `verifier-loop` bin. Identity / store resolution (env wins) lives in the bin.

use clap::{Parser, Subcommand};

pub mod json_output;

/// `verifier-loop` (jewilo) top-level CLI.
#[derive(Debug, Parser)]
#[command(
    name = "verifier-loop",
    bin_name = "verifier-loop",
    version,
    about = "Spawn verifiers, gather verdicts, and produce a tamper-evident completion hash."
)]
pub struct VerifierLoopCli {
    /// Machine-readable JSON output mode (`add-json-output-mode`, design D2). Global so it
    /// parses both before AND after the subcommand: `jewilo --json NEW <goal>` and
    /// `jewilo NEW <goal> --json` both work.
    #[arg(long, short = 'j', global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: VerifierLoopCmd,
}

/// `verifier-loop` subcommands.
#[derive(Debug, Subcommand)]
pub enum VerifierLoopCmd {
    /// Create a new immutable goal, spawn round 1, evaluate n/m consensus.
    #[command(name = "NEW")]
    New {
        /// The goal text (immutable once written to goal.json).
        goal: String,
        /// Optional context annotation recorded into goal.json.
        #[arg(long)]
        context: Option<String>,
    },
    /// Resume a goal: increment the round, append fix notes, respawn verifiers.
    #[command(name = "RESUME")]
    Resume {
        /// The goalId (UUID) to resume.
        goal_id: String,
        /// Optional fix notes appended to the new round's fix-notes.json.
        #[arg(long)]
        fix: Option<String>,
    },
    /// Cross-process round recovery (SHAPE-1): wait for already-emitted verdicts from the
    /// current round and re-evaluate consensus. Does NOT spawn, kill, re-render, or
    /// re-capture. Use after jewilo was killed/interrupted mid-round (add-round-recovery).
    #[command(name = "RECOVER")]
    Recover {
        /// The goalId (UUID) to recover.
        goal_id: String,
    },
    /// Read-only machine-readable goal state: round, state, needs, and per-slot verdicts
    /// (add-round-recovery LD7). Does not take the goal lock; never blocks.
    #[command(name = "STATUS")]
    Status {
        /// The goalId (UUID) to inspect.
        goal_id: String,
    },
    /// Read-only aggregate of ALL stored JSON for a goal run: goal record, creation-time
    /// config snapshot, per-round verdicts, completion, health, and durations. Does not
    /// take the goal lock; never blocks or spawns verifiers (intention 2026-07-14).
    #[command(name = "STATS")]
    Stats {
        /// The goalId (UUID) to inspect.
        goal_id: String,
    },
    /// Read-only post-hoc audit: verifies the final completion TRULY matches the
    /// creation-time config requirement (n/m verdict match + hash recompute). Prints a
    /// JSON report and exits 0 if valid, non-zero otherwise. Does not take the goal lock
    /// or spawn verifiers (intention 2026-07-14).
    #[command(name = "AUDIT")]
    Audit {
        /// The goalId (UUID) to audit.
        goal_id: String,
    },
}
