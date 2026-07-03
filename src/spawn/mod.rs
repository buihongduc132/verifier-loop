//! Verifier spawn orchestration + session reuse (tasks.md §5,§6, verifier-spawn spec).
//!
//! Parallel non-blocking spawn of m verifiers (D7) via `tokio::process::Command` +
//! `tokio::select!`; injected identity env (D2); per-verifier timeout -> NULL verdict (D9);
//! gather barrier; pre-create `verdict.json` status:null + `meta.json`. On RESUME reuse SID
//! up to maxTurn else fresh spawn with archived prior SID (D8).

// TODO §5,§6: orchestrator + reuse (RED then GREEN, separate fresh teammates).
