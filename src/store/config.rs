//! Config loader (tasks.md §2.2, goal-lifecycle spec).
//!
//! `~/.verifier-loop/config.json` carries the tunable parameters that gate spawning,
//! consensus, and the frozen diff fed to verifiers. The on-disk shape uses camelCase keys
//! (`maxTurn`, `gitDiffMaxChars`, `verifierTimeoutSec`) matching the spec; the in-memory
//! shape uses idiomatic snake_case Rust fields via `#[serde(rename)]`.
//!
//! Semantics:
//!   * Missing `config.json`            → fully defaulted [`Config`].
//!   * Partial `config.json`            → present fields honoured, missing fields defaulted.
//!   * Malformed `config.json`          → hard error (fail-closed); never silently defaulted.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::StoreError;

/// Tunable parameters for a verifier-loop run.
///
/// Defaults (tasks.md §2.2):
///   * `n = 2`, `m = 2`              — consensus threshold / verifier count
///   * `max_turn = 3`                — per-verifier turn budget before a fresh spawn (D8)
///   * `backend = "pi"`              — ACP backend (pi | hermes | acpx | custom)
///   * `git_diff_max_chars = 10000`  — cap on the frozen `git diff` snapshot fed to V*
///   * `verifier_timeout_sec = 1800` — per-verifier wall-clock timeout (D9)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Consensus threshold — minimum APPROVE verdicts required to pass (n of m).
    pub n: u32,
    /// Number of verifiers spawned per round.
    pub m: u32,
    /// Per-verifier turn budget; once exhausted the session is spawned fresh (D8/§6).
    #[serde(rename = "maxTurn")]
    pub max_turn: u32,
    /// ACP backend key: `pi` | `hermes` | `acpx` | a custom-adapter key.
    pub backend: String,
    /// Cap on the frozen `git diff` snapshot handed to each verifier (chars).
    #[serde(rename = "gitDiffMaxChars")]
    pub git_diff_max_chars: u64,
    /// Per-verifier wall-clock timeout in seconds (D9). A timeout leaves a null verdict.
    #[serde(rename = "verifierTimeoutSec")]
    pub verifier_timeout_sec: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            n: 2,
            m: 2,
            max_turn: 3,
            backend: "pi".to_string(),
            git_diff_max_chars: 10_000,
            verifier_timeout_sec: 1800,
        }
    }
}

impl Config {
    /// Loads the config for the given store root.
    ///
    /// Thin associated wrapper over [`load_config_in`] so callers may write either
    /// `Config::load_in(root)` or `load_config_in(root)`.
    pub fn load_in(root: &Path) -> Result<Self, StoreError> {
        load_config_in(root)
    }
}

/// Loads `config.json` from `<root>/config.json`, applying defaults for any missing file
/// or missing field. A present-but-malformed file is a hard error (fail-closed).
pub fn load_config_in(root: &Path) -> Result<Config, StoreError> {
    let path = root.join("config.json");
    if !path.exists() {
        return Ok(Config::default());
    }

    let raw = std::fs::read_to_string(&path)?;
    let cfg: Config = serde_json::from_str(&raw)
        .map_err(|e| StoreError::Json(e.to_string()))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    // Behavioural coverage of the public surface lives in the integration test
    // `tests/store.rs` (the §2 RED→GREEN contract). These unit tests pin a couple of
    // invariants that the integration test does not directly assert, to keep coverage
    // honest on the helper paths.

    use super::*;

    #[test]
    fn default_matches_spec_constants() {
        let d = Config::default();
        assert_eq!((d.n, d.m, d.max_turn), (2, 2, 3));
        assert_eq!(d.backend, "pi");
        assert_eq!((d.git_diff_max_chars, d.verifier_timeout_sec), (10_000, 1800));
    }

    #[test]
    fn config_round_trips_through_serde_json_camel_case() {
        let cfg = Config {
            n: 7,
            m: 9,
            max_turn: 11,
            backend: "hermes".into(),
            git_diff_max_chars: 4_000,
            verifier_timeout_sec: 99,
        };
        let j = serde_json::to_string(&cfg).unwrap();
        // camelCase keys must appear verbatim (this is the on-disk contract).
        assert!(j.contains("\"maxTurn\":11"), "maxTurn must be camelCase: {j}");
        assert!(j.contains("\"gitDiffMaxChars\":4000"), "{j}");
        assert!(j.contains("\"verifierTimeoutSec\":99"), "{j}");

        let back: Config = serde_json::from_str(&j).unwrap();
        assert_eq!(back, cfg);
    }
}
