//! Config loader (tasks.md §2.2, goal-lifecycle spec; dynamic-pipeline D1).
//!
//! `~/.verifier-loop/config.json` carries the tunable parameters that gate spawning,
//! consensus, and the frozen diff fed to verifiers. The on-disk shape uses camelCase keys
//! (`maxTurn`, `gitDiffMaxChars`, `verifierTimeoutSec`, `dumpAdapter`, `escaThreshold`)
//! matching the spec; the in-memory shape uses idiomatic snake_case Rust fields via
//! `#[serde(rename)]`.
//!
//! ## dynamic-pipeline extension (D1, LD19/LD23/LD28/LD30)
//!
//! Six new fields gate the PL-D/PL-E pipeline:
//!   * `dumpAdapter`    — adapter for the Dump (D) role (LD1). `None` → fall back to `backend`.
//!   * `smartAdapter`   — adapter for the Smart (S) role (LD1). `None` → fall back to `backend`.
//!   * `confirmCount`   — Smart-verifier count for Confirm/Final phases (default 1).
//!   * `escaThreshold`  — consecutive Gate-pass/Confirm-reject count to flip to PL-E (default 2,
//!                        LD4; `0` disables escalation per LD21).
//!   * `escaMaxRetries` — PL-E cycle cap before hard-fail (default 3, LD21).
//!
//! Precedence (LD19): `dumpAdapter`/`smartAdapter` win over legacy `backend`; setting BOTH
//! `backend` AND `dumpAdapter` is a hard error (ambiguous). All fields are snapshotted at
//! NEW time via [`Config::snapshot`] (LD23).
//!
//! Semantics:
//!   * Missing `config.json`            → fully defaulted [`Config`].
//!   * Partial `config.json`            → present fields honoured, missing fields defaulted.
//!   * Malformed `config.json`          → hard error (fail-closed); never silently defaulted.
//!   * Degenerate config (n=0, n>m, …)  → hard error (LD28, fail-closed at parse time).
//!   * Unknown key in `config.json`     → hard error (fail-closed); the canonical key
//!     set is closed and any extra field (e.g. a stale `cwd`) is rejected at parse
//!     time so a tampered/legacy file can never silently mask runtime behaviour.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::StoreError;

/// Tunable parameters for a verifier-loop run (plus dynamic-pipeline extension, D1).
///
/// Defaults (tasks.md §2.2 + dynamic-pipeline D1):
///   * `n = 2`, `m = 2`              — consensus threshold / verifier count
///   * `max_turn = 3`                — per-verifier turn budget before a fresh spawn (D8)
///   * `backend = "pi"`              — ACP backend (pi | hermes | acpx | custom)
///   * `git_diff_max_chars = 10000`  — cap on the frozen `git diff` snapshot fed to V*
///   * `verifier_timeout_sec = 1800` — per-verifier wall-clock timeout (D9)
///   * `confirm_count = 1`           — Smart-verifier count for Confirm/Final (LD19)
///   * `esca_threshold = 2`          — consecutive Gate-pass/Confirm-reject to flip PL-E (LD4)
///   * `esca_max_retries = 3`        — PL-E cycle cap (LD21)
///
/// `deny_unknown_fields` closes the on-disk schema: any key outside the canonical fields
/// (e.g. a legacy `cwd`, `model`, or stray prompt template) is a hard parse error rather
/// than silently dropped. `cwd` is sourced at runtime from `std::env::current_dir()` and
/// is *never* read from `config.json`.
///
/// Per-field `#[serde(default = "...")]` is used INSTEAD of container `#[serde(default)]`
/// so that a missing `backend` parses to `""` (not `Config::default().backend = "pi"`).
/// This lets the LD19 ambiguity check (reject when both `backend` AND `dumpAdapter` are
/// explicitly set) distinguish user-set from defaulted. The no-`config.json` path still
/// returns `Config::default()` with `backend = "pi"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Consensus threshold — minimum APPROVE verdicts required to pass (n of m).
    #[serde(default = "default_n")]
    pub n: u32,
    /// Number of verifiers spawned per round.
    #[serde(default = "default_m")]
    pub m: u32,
    /// Per-verifier turn budget; once exhausted the session is spawned fresh (D8/§6).
    #[serde(rename = "maxTurn", default = "default_max_turn")]
    pub max_turn: u32,
    /// ACP backend key: `pi` | `hermes` | `acpx` | a custom-adapter key. Legacy alias
    /// for `dump_adapter` when `dump_adapter` is unset (LD19 precedence #3). Defaults to
    /// `""` when absent from `config.json` so the LD19 ambiguity check can distinguish
    /// user-set from defaulted.
    #[serde(default)]
    pub backend: String,
    /// Cap on the frozen `git diff` snapshot handed to each verifier (chars).
    #[serde(rename = "gitDiffMaxChars", default = "default_git_diff_max_chars")]
    pub git_diff_max_chars: u64,
    /// Per-verifier wall-clock timeout in seconds (D9). A timeout leaves a null verdict.
    #[serde(rename = "verifierTimeoutSec", default = "default_verifier_timeout_sec")]
    pub verifier_timeout_sec: u64,
    /// Optional override file whose contents are prepended (raw, no `{{var}}` expansion)
    /// to the baked-in verifier prompt for every round (NEW + RESUME). Relative paths
    /// resolve against the store root; absolute paths are used as-is. Missing/unreadable
    /// -> fail-closed error (no goal dir / signature written).
    #[serde(rename = "verifierPromptFile", default)]
    pub verifier_prompt_file: Option<String>,
    /// Minimum trimmed char length for `goalText`. `0` disables the check (default).
    /// Empty/whitespace-only goalText is ALWAYS an error regardless of this value.
    #[serde(rename = "minGoalChars", default)]
    pub min_goal_chars: u64,
    /// Byte cap on the `fileEditTimes` block (scoped to changed files). When the
    /// changed-files block exceeds this cap it is truncated with an indicator.
    /// Prompt-bloat fix D1 (default 8000).
    #[serde(
        rename = "fileEditTimesMaxChars",
        default = "default_file_edit_times_max_chars"
    )]
    pub file_edit_times_max_chars: u64,
    /// Char cap on the `--context` input. Over-cap context is truncated with an
    /// indicator. Prompt-bloat fix D3 (default 20000).
    #[serde(rename = "contextMaxChars", default = "default_context_max_chars")]
    pub context_max_chars: u64,
    /// Rendered-prompt byte budget. When the total rendered prompt exceeds this, a
    /// per-section warning is emitted to stderr (does NOT block spawn). Prompt-bloat
    /// fix D4 (default 50000).
    #[serde(rename = "promptBudgetBytes", default = "default_prompt_budget_bytes")]
    pub prompt_budget_bytes: u64,
    // --- dynamic-pipeline extension (D1, LD19) ---
    /// Adapter for the Dump (D) role (LD1). `None` → fall back to `backend` (LD19 #3).
    #[serde(rename = "dumpAdapter")]
    pub dump_adapter: Option<String>,
    /// Adapter for the Smart (S) role (LD1). `None` → fall back to `backend` (LD19 #3).
    #[serde(rename = "smartAdapter")]
    pub smart_adapter: Option<String>,
    /// Smart-verifier count for the Confirm and Final phases (LD19, default 1).
    #[serde(rename = "confirmCount", default = "default_confirm_count")]
    pub confirm_count: u32,
    /// Consecutive Gate-pass/Confirm-reject count that flips the goal to PL-E (LD4,
    /// default 2). `0` disables escalation (LD21).
    #[serde(rename = "escaThreshold", default = "default_esca_threshold")]
    pub esca_threshold: u32,
    /// PL-E cycle cap before the goal hard-fails with "escalation exhaustion" (LD21,
    /// default 3).
    #[serde(rename = "escaMaxRetries", default = "default_esca_max_retries")]
    pub esca_max_retries: u32,
}

fn default_file_edit_times_max_chars() -> u64 {
    8_000
}

fn default_context_max_chars() -> u64 {
    20_000
}

fn default_prompt_budget_bytes() -> u64 {
    50_000
}

fn default_confirm_count() -> u32 {
    1
}

fn default_esca_threshold() -> u32 {
    2
}

fn default_esca_max_retries() -> u32 {
    3
}

// Per-field serde defaults for the legacy fields (used instead of container
// `#[serde(default)]` so a missing `backend` parses to `""`).
fn default_n() -> u32 {
    2
}
fn default_m() -> u32 {
    2
}
fn default_max_turn() -> u32 {
    3
}
fn default_git_diff_max_chars() -> u64 {
    10_000
}
fn default_verifier_timeout_sec() -> u64 {
    1800
}

/// Frozen snapshot of the pipeline-relevant config fields, written into `goal.json` at
/// NEW time so live `config.json` edits do not affect in-flight goals (LD23).
///
/// Carries every field that influences pipeline composition or escalation so that a
/// receipt can be re-derived deterministically from the snapshot alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalSnapshot {
    pub n: u32,
    pub m: u32,
    pub backend: String,
    pub dump_adapter: Option<String>,
    pub smart_adapter: Option<String>,
    pub confirm_count: u32,
    pub esca_threshold: u32,
    pub esca_max_retries: u32,
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
            file_edit_times_max_chars: 8_000,
            context_max_chars: 20_000,
            prompt_budget_bytes: 50_000,
            dump_adapter: None,
            smart_adapter: None,
            confirm_count: 1,
            esca_threshold: 2,
            esca_max_retries: 3,
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

    /// Resolve the Dump-role adapter name (LD19 precedence: dump_adapter → backend).
    ///
    /// `dump_adapter` wins when set; otherwise the legacy `backend` field is the alias.
    /// The ambiguity case (both set) is rejected at parse time, so by the time a `Config`
    /// exists this resolver is unambiguous.
    pub fn resolve_dump_adapter(&self) -> String {
        self.dump_adapter
            .clone()
            .unwrap_or_else(|| self.backend.clone())
    }

    /// Resolve the Smart-role adapter name (LD19 precedence: smart_adapter → backend).
    ///
    /// `smart_adapter` wins when set; otherwise it defaults to `backend` (so a config
    /// with only `backend` behaves as today: all verifiers same adapter, Confirm phase
    /// degenerate but valid).
    pub fn resolve_smart_adapter(&self) -> String {
        self.smart_adapter
            .clone()
            .unwrap_or_else(|| self.backend.clone())
    }

    /// Produce a frozen snapshot of the pipeline-relevant fields for `goal.json` (LD23).
    ///
    /// Live `config.json` edits after NEW do not affect in-flight goals because every
    /// field that influences pipeline composition is captured here.
    pub fn snapshot(&self) -> GoalSnapshot {
        GoalSnapshot {
            n: self.n,
            m: self.m,
            backend: self.backend.clone(),
            dump_adapter: self.dump_adapter.clone(),
            smart_adapter: self.smart_adapter.clone(),
            confirm_count: self.confirm_count,
            esca_threshold: self.esca_threshold,
            esca_max_retries: self.esca_max_retries,
        }
    }

    /// Non-fatal validation warnings (LD15: escaThreshold ignored when m < 2).
    ///
    /// Returns a list of human-readable warning strings. The caller may emit them to
    /// stderr. These NEVER block a goal — they document benign misconfigs.
    pub fn validate_and_warn(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.m < 2 && self.esca_threshold > 0 {
            warnings.push(format!(
                "escaThreshold ({}) ignored when m < 2 (LD15): PL-E cannot escalate at m={}",
                self.esca_threshold, self.m
            ));
        }
        warnings
    }

    /// Fail-closed validation (LD28). Returns `Ok(())` iff the config is non-degenerate.
    ///
    /// Rules:
    ///   * `1 ≤ n ≤ m`  — reject `n=0` (vacuous-pass Gate) and `n>m` (impossible).
    ///   * `m ≥ 1`      — reject `m=0` (empty pipeline).
    ///   * `confirm_count ≥ 1`.
    ///   * `esca_threshold ≥ 0` (u32 guarantees this; documented for LD21).
    ///   * NOT both `backend` AND `dump_adapter` set (LD19 ambiguity).
    fn validate(&self) -> Result<(), StoreError> {
        if self.n == 0 {
            return Err(StoreError::Json(format!(
                "invalid config: n=0 creates a vacuous-pass Gate; need 1 ≤ n ≤ m (LD28)"
            )));
        }
        if self.n > self.m {
            return Err(StoreError::Json(format!(
                "invalid config: n ({}) > m ({}) is impossible; need 1 ≤ n ≤ m (LD28)",
                self.n, self.m
            )));
        }
        if self.m == 0 {
            return Err(StoreError::Json(format!(
                "invalid config: m=0 produces an empty pipeline; need m ≥ 1 (LD28)"
            )));
        }
        if self.confirm_count == 0 {
            return Err(StoreError::Json(format!(
                "invalid config: confirmCount=0 is degenerate; need confirmCount ≥ 1 (LD28)"
            )));
        }
        // LD19: both backend AND dump_adapter set is ambiguous (no verifiers[] to break the tie).
        // `backend` defaults to "" when absent via #[serde(default)], so only treat a non-empty
        // backend as "set".
        if !self.backend.is_empty() && self.dump_adapter.is_some() {
            return Err(StoreError::Json(format!(
                "invalid config: both backend ({}) AND dumpAdapter ({:?}) set is ambiguous; \
                 use one or the other, or add per-slot verifiers[] (LD19)",
                self.backend, self.dump_adapter
            )));
        }
        Ok(())
    }
}

/// Loads `config.json` from `<root>/config.json`, applying defaults for any missing file
/// or missing field. A present-but-malformed OR degenerate file is a hard error
/// (fail-closed, LD28).
pub fn load_config_in(root: &Path) -> Result<Config, StoreError> {
    let path = root.join("config.json");
    if !path.exists() {
        return Ok(Config::default());
    }

    let raw = std::fs::read_to_string(&path)?;
    let cfg: Config = serde_json::from_str(&raw).map_err(|e| StoreError::Json(e.to_string()))?;
    cfg.validate()?;
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
        assert_eq!(
            (d.git_diff_max_chars, d.verifier_timeout_sec),
            (10_000, 1800)
        );
        // dynamic-pipeline defaults (D1)
        assert_eq!((d.confirm_count, d.esca_threshold, d.esca_max_retries), (1, 2, 3));
        assert_eq!(d.dump_adapter, None);
        assert_eq!(d.smart_adapter, None);
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
            file_edit_times_max_chars: 8_000,
            context_max_chars: 20_000,
            prompt_budget_bytes: 50_000,
            dump_adapter: Some("acpx".into()),
            smart_adapter: Some("pi".into()),
            confirm_count: 4,
            esca_threshold: 5,
            esca_max_retries: 6,
        };
        let j = serde_json::to_string(&cfg).unwrap();
        // camelCase keys must appear verbatim (this is the on-disk contract).
        assert!(
            j.contains("\"maxTurn\":11"),
            "maxTurn must be camelCase: {j}"
        );
        assert!(j.contains("\"gitDiffMaxChars\":4000"), "{j}");
        assert!(j.contains("\"verifierTimeoutSec\":99"), "{j}");
        // dynamic-pipeline camelCase (D1)
        assert!(j.contains("\"dumpAdapter\":\"acpx\""), "{j}");
        assert!(j.contains("\"smartAdapter\":\"pi\""), "{j}");
        assert!(j.contains("\"confirmCount\":4"), "{j}");
        assert!(j.contains("\"escaThreshold\":5"), "{j}");
        assert!(j.contains("\"escaMaxRetries\":6"), "{j}");

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
        assert_eq!(cfg.backend, ""); // struct-level #[serde(default)] → String::default ()
        assert_eq!(cfg.verifier_timeout_sec, 1800);
        assert_eq!(cfg.confirm_count, 1);
    }

    #[test]
    fn resolve_dump_adapter_prefers_dump_adapter_over_backend() {
        let cfg = Config {
            dump_adapter: Some("acpx".into()),
            backend: "pi".into(),
            ..Config::default()
        };
        assert_eq!(cfg.resolve_dump_adapter(), "acpx");
    }

    #[test]
    fn resolve_smart_adapter_falls_back_to_backend() {
        let cfg = Config {
            smart_adapter: None,
            backend: "hermes".into(),
            ..Config::default()
        };
        assert_eq!(cfg.resolve_smart_adapter(), "hermes");
    }

    #[test]
    fn snapshot_captures_all_pipeline_fields() {
        let cfg = Config {
            n: 3,
            m: 3,
            backend: "pi".into(),
            dump_adapter: Some("hermes".into()),
            smart_adapter: Some("acpx".into()),
            confirm_count: 2,
            esca_threshold: 4,
            esca_max_retries: 5,
            ..Config::default()
        };
        let s = cfg.snapshot();
        assert_eq!(s.n, 3);
        assert_eq!(s.m, 3);
        assert_eq!(s.dump_adapter.as_deref(), Some("hermes"));
        assert_eq!(s.smart_adapter.as_deref(), Some("acpx"));
        assert_eq!(s.confirm_count, 2);
        assert_eq!(s.esca_threshold, 4);
        assert_eq!(s.esca_max_retries, 5);
    }
}
