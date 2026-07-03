//! Salt + config store (tasks.md §2, goal-lifecycle spec).
//!
//! `~/.verifier-loop/.salt` — 64 hex chars, mode 0600, created once, never printed.
//! `~/.verifier-loop/config.json` — n, m, maxTurn, backend, gitDiffMaxChars,
//! verifierTimeoutSec, optional prompt/resume templates + custom adapter templates.

// TODO §2: salt creation + config loader (RED then GREEN, separate fresh teammates).
