//! Verifier spawn orchestration + session reuse (tasks.md §5,§6, verifier-spawn spec).
//!
//! Parallel non-blocking spawn of m verifiers (D7) via `tokio::process::Command` +
//! `tokio::select!`; injected identity env (D2); per-verifier timeout -> NULL verdict (D9);
//! gather barrier; pre-create `verdict.json` status:null + `meta.json`. On RESUME reuse SID
//! up to maxTurn else fresh spawn with archived prior SID (D8).
//!
//! Implementation lives in [`orchestrator`]; this module re-exports the public surface.

mod orchestrator;

pub use orchestrator::{
    spawn_resume, spawn_round, SpawnError, SpawnInput, VerifierMeta,
    VerifierRun, ARCHIVE_FILE, ENV_GOAL_ID, ENV_ROUND, ENV_VERIFIER_ID, FINAL_OUTPUT_FILE,
    META_FILE, VERDICT_FILE,
};
