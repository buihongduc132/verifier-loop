//! CLI command definitions (tasks.md §10) for both binaries.
//!
//! `verifier-loop` (jewilo):   `NEW "<goal>" [--context]`, `RESUME <goalId> [--fix "…"]`.
//! `verifier-verdict` (jewije): `approve`, `reject --notes "…"`.
//! Bin targets live in `src/bin/`; this module holds the shared clap command structs.

// TODO §10: wire clap commands to lib logic (RED then GREEN, separate fresh teammates).
