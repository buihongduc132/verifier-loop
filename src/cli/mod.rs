//! CLI command definitions (tasks.md §10) for both binaries.
//!
//! * `verifier-loop` (`jewilo`):   `NEW "<goal>" [--context]`, `RESUME <goalId> [--fix "…"]`.
//! * `verifier-verdict` (`jewije`): `approve`, `reject --notes "…"` (defined inline in its
//!   own bin; this module holds the `verifier-loop` shared command structs so the bin stays
//!   a thin dispatch layer over the lib).
//!
//! Bin targets live in `src/bin/`; this module holds the clap command structs consumed by
//! the `verifier-loop` bin. Identity / store resolution (env wins) lives in the bin.

use clap::{Parser, Subcommand};

/// `verifier-loop` (jewilo) top-level CLI.
#[derive(Debug, Parser)]
#[command(
    name = "verifier-loop",
    bin_name = "verifier-loop",
    version,
    about = "Spawn verifiers, gather verdicts, and produce a tamper-evident completion hash."
)]
pub struct VerifierLoopCli {
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
}
