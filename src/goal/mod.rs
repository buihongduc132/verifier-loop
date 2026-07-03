//! Goal lifecycle (tasks.md §3, goal-lifecycle spec).
//!
//! `NEW "<goal>" [--context]`  -> goalId, immutable goal.json, signature.json (D5).
//! `RESUME <id> [--fix "…"]`   -> increment round, append fix-notes.json, goal untouched.
//! Missing store / missing goal -> fail closed, no hash.

// TODO §3: NEW + RESUME + immutability signature (RED then GREEN, separate fresh teammates).
