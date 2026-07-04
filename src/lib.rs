//! `verifier-loop` — out-of-process verifier-loop CLI core library.
//!
//! Two binaries consume this crate:
//! * `verifier-loop` (`jewilo`)  — A's interface (NEW / RESUME / spawn / gather / consensus / hash).
//! * `verifier-verdict` (`jewije`) — V*'s interface (approve / reject).
//!
//! Module map (one module per tasks.md §N group; behaviour lands incrementally via TDD):
//!
//! | module      | tasks.md | spec                | responsibility                                  |
//! |-------------|----------|---------------------|-------------------------------------------------|
//! | [`store`]   | §2       | goal-lifecycle      | salt + config.json store                        |
//! | [`goal`]    | §3       | goal-lifecycle      | goal lifecycle (NEW/RESUME, immutability, sig)  |
//! | [`acp`]     | §4       | verifier-spawn      | shared ACP JSON stream parser + adapters        |
//! | [`spawn`]   | §5,§6    | verifier-spawn      | parallel orchestration + session reuse          |
//! | [`verdict`] | §7       | verdict-registration| verifier-verdict CLI logic                      |
//! | [`consensus`] | §8     | consensus-check / completion-proof | n/m counter + tamper-evident hash |
//! | [`prompt`]  | §9       | verifier-prompt     | blind + frozen-artifact prompt rendering        |
//! | [`cli`]     | §10      | (wiring)            | CLI command definitions for both binaries       |
//! | [`crypto`]  | tamper   | verifier-identity / signed-verdict-record | Ed25519 sign/verify + canonical record bytes |
//!
//! Fail-closed invariants (D9): every error path is explicit (`Result<T,E>`); a NULL verdict
//! never becomes APPROVE; a missing store yields no hash.

// Each module is a stub at scaffolding time (tasks.md §1). Behaviour is added group-by-group
// under strict TDD (RED test by one fresh teammate, GREEN impl by a different fresh teammate).

pub mod acp;
pub mod cli;
pub mod consensus;
pub mod crypto;
pub mod goal;
pub mod prompt;
pub mod spawn;
pub mod store;
pub mod verdict;

/// Crate-level version, surfaced by `--version` on both binaries.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
