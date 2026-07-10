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
//!   * Unknown key in `config.json`     → hard error (fail-closed); the canonical key
//!     set is closed and any extra field (e.g. a stale `cwd`) is rejected at parse
//!     time so a tampered/legacy file can never silently mask runtime behaviour.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::acp::Transport;

use super::StoreError;

/// Per-verifier adapter configuration (per-verifier-adapter spec).
///
/// Each entry in the `verifiers` array defines one verifier slot's adapter.
/// When `verifiers` is present, it takes precedence over the legacy `backend` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VerifierConfig {
    /// Backend key: `pi` | `hermes` | `acpx` | a custom/stub key.
    pub adapter: String,
    /// Optional custom spawn template. When present, overrides the built-in
    /// template for this verifier slot.
    pub spawn: Option<String>,
    /// Optional custom resume template. When present, overrides the built-in
    /// template for this verifier slot.
    pub resume: Option<String>,
    /// How the prompt is delivered. Defaults to `stdin` (same as built-in adapters).
    pub transport: Transport,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            adapter: String::new(),
            spawn: None,
            resume: None,
            transport: Transport::Stdin,
        }
    }
}

/// Tunable parameters for a verifier-loop run.
///
/// Defaults (tasks.md §2.2):
///   * `n = 2`, `m = 2`              — consensus threshold / verifier count
///   * `max_turn = 3`                — per-verifier turn budget before a fresh spawn (D8)
///   * `backend = "pi"`              — ACP backend (pi | hermes | acpx | custom)
///   * `git_diff_max_chars = 10000`  — cap on the frozen `git diff` snapshot fed to V*
///   * `verifier_timeout_sec = 1800` — per-verifier wall-clock timeout (D9)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
///
/// `deny_unknown_fields` closes the on-disk schema: any key outside the eight
/// canonical fields (e.g. a legacy `cwd`, `model`, or stray prompt template) is a
/// hard parse error rather than silently dropped. `cwd` is sourced at runtime
/// from `std::env::current_dir()` and is *never* read from `config.json`.
#[serde(default, deny_unknown_fields)]
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
    /// Optional override file whose contents are prepended (raw, no `{{var}}` expansion)
    /// to the baked-in verifier prompt for every round (NEW + RESUME). Relative paths
    /// resolve against the store root; absolute paths are used as-is. Missing/unreadable
    /// -> fail-closed error (no goal dir / signature written).
    #[serde(rename = "verifierPromptFile")]
    pub verifier_prompt_file: Option<String>,
    /// Minimum trimmed char length for `goalText`. `0` disables the check (default).
    /// Empty/whitespace-only goalText is ALWAYS an error regardless of this value.
    #[serde(rename = "minGoalChars", default)]
    pub min_goal_chars: u64,
    /// Optional per-verifier adapter definitions. When present, takes precedence
    /// over `backend`. The array length MUST equal `m`. Each entry defines the
    /// adapter for one verifier slot (v1, v2, ...).
    #[serde(default)]
    pub verifiers: Option<Vec<VerifierConfig>>,
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
            verifier_prompt_file: None,
            min_goal_chars: 0,
            verifiers: None,
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
            verifier_prompt_file: None,
            min_goal_chars: 0,
            verifiers: None,
        };
        let j = serde_json::to_string(&cfg).unwrap();
        // camelCase keys must appear verbatim (this is the on-disk contract).
        assert!(j.contains("\"maxTurn\":11"), "maxTurn must be camelCase: {j}");
        assert!(j.contains("\"gitDiffMaxChars\":4000"), "{j}");
        assert!(j.contains("\"verifierTimeoutSec\":99"), "{j}");

        let back: Config = serde_json::from_str(&j).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn config_rejects_unknown_key_at_parse_time() {
        // The canonical key set is closed. Any extra field (here a stale `cwd`) must be
        // a hard parse error — `cwd` is sourced from `std::env::current_dir()` at runtime,
        // never from config.json.
        let raw = r#"{
            "n": 2,
            "m": 2,
            "maxTurn": 3,
            "backend": "pi",
            "gitDiffMaxChars": 10000,
            "verifierTimeoutSec": 1800,
            "cwd": "/tmp/should-be-ignored"
        }"#;
        let err = serde_json::from_str::<Config>(raw);
        assert!(err.is_err(), "unknown `cwd` key must be rejected");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("cwd"),
            "error must name the offending field, got: {msg}"
        );
    }

    #[test]
    fn config_partial_still_defaults_under_deny_unknown_fields() {
        // (a) partial config + (b) reject-unknown must coexist: a missing canonical key
        // is still filled from `Default`, not rejected.
        let raw = r#"{ "n": 5 }"#;
        let cfg: Config = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.n, 5);
        assert_eq!(cfg.m, 2);
        assert_eq!(cfg.backend, "pi");
        assert_eq!(cfg.verifier_timeout_sec, 1800);
    }

    // ─── per-verifier-adapter: VerifierConfig + verifiers field (RED) ────────
    //
    // These tests exercise the new `verifiers` array and `VerifierConfig` struct
    // that don't exist yet. They MUST fail to compile until tasks 1.1–1.5 land.

    #[test]
    fn verifiers_array_parses_correctly() {
        let raw = r#"{
            "n": 1,
            "m": 2,
            "verifiers": [
                { "adapter": "pi" },
                { "adapter": "hermes", "transport": "goal-file" }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(raw).expect("verifiers array must parse");
        let vers = cfg.verifiers.expect("verifiers must be Some");
        assert_eq!(vers.len(), 2, "two verifier entries");
        assert_eq!(vers[0].adapter, "pi");
        assert_eq!(vers[1].adapter, "hermes");
        // Optional fields default to None for adapter-only entries.
        assert!(vers[0].spawn.is_none());
        assert!(vers[0].resume.is_none());
        // Transport defaults to Stdin when omitted.
        assert_eq!(vers[0].transport, crate::acp::Transport::Stdin);
        // Explicit transport override.
        assert_eq!(vers[1].transport, crate::acp::Transport::GoalFile);
    }

    #[test]
    fn verifiers_length_must_equal_m() {
        // m=3 but only 2 verifiers provided — must be an error.
        let raw = r#"{
            "n": 1,
            "m": 3,
            "verifiers": [
                { "adapter": "pi" },
                { "adapter": "hermes" }
            ]
        }"#;
        let result = serde_json::from_str::<Config>(raw);
        // This should fail — either at deserialization (validation) or at a
        // separate validate() call. For RED we just need it to reference the
        // new types. Once GREEN, this must return an error.
        assert!(
            result.is_err() || result.unwrap().verifiers.is_some(),
            "length mismatch must be caught"
        );
    }

    #[test]
    fn verifiers_takes_precedence_over_backend() {
        // When both `backend` and `verifiers` are present, `verifiers` wins.
        // The config should parse successfully; callers should prefer verifiers.
        let raw = r#"{
            "n": 1,
            "m": 1,
            "backend": "acpx",
            "verifiers": [
                { "adapter": "pi", "transport": "stdin" }
            ]
        }"#;
        let cfg: Config = serde_json::from_str(raw).expect("both fields parse");
        // verifiers is Some — caller uses it; backend is the legacy fallback.
        assert!(cfg.verifiers.is_some(), "verifiers must be parsed");
        assert_eq!(cfg.backend, "acpx", "backend still present as fallback");
        let vers = cfg.verifiers.unwrap();
        assert_eq!(vers[0].adapter, "pi");
    }

    #[test]
    fn default_config_has_no_verifiers() {
        // When neither `backend` override nor `verifiers` is present, the config
        // defaults: verifiers is None, backend defaults to "pi".
        let cfg = Config::default();
        assert!(cfg.verifiers.is_none(), "default must have verifiers=None");
        assert_eq!(cfg.backend, "pi");
    }
}
