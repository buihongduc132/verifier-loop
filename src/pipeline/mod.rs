//! Dynamic-round-pipeline abstractions (dynamic-pipeline D2; LD11, LD22, LD9, LD15).
//!
//! Rot-proof design: `Phase` is the atomic execution unit. Gate/Confirm/Mixed/Final are
//! NOT separate code paths — they are constructed by [`default_pipeline`] /
//! [`escalation_pipeline`] and executed by a shape-agnostic runner (Phase 3, executor).
//!
//! ## Pipelines
//!
//! ```text
//! PL-D (default):   Gate(m D, n) → Confirm(confirmCount S, confirmCount)
//! PL-E (escalated): Mixed(⌊m/2⌋ D + ⌈m/2⌉ S, m) → Final(confirmCount S, confirmCount)
//! ```
//!
//! Adding a future phase type changes a pipeline constructor, NOT the executor.

pub mod esca;
pub mod executor;

use crate::store::Config;

use serde::{Deserialize, Serialize};

/// Sub-phase identity within one invocation (LD3, LD17, LD18): `"1a"`, `"1b"`, ...
///
/// Distinct from the top-level `round: u32` (the RESUME counter). Two sub-phases in the
/// same invocation share the same `round` but have different `PhaseId`s. Canonical order
/// is alphabetical (`"1a" < "1b" < "1c" < "1d"`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PhaseId(String);

impl PhaseId {
    /// Construct from a string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The canonical letter-suffixed phaseId as `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PhaseId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for PhaseId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for PhaseId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Semantic role label for a phase (LD11). Gate/Confirm/Mixed/Final are labels, not
/// code paths — the executor is role-agnostic. Used for id-scheme offset derivation
/// (`spawn::verifier_ids_for_phase`) and audit display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PhaseRole {
    /// PL-D first phase: m dump verifiers, threshold n.
    Gate,
    /// PL-D second phase: confirmCount smart verifiers, unanimity.
    Confirm,
    /// PL-E first phase: floor(m/2) dump + ceil(m/2) smart, threshold m (unanimity, LD22).
    Mixed,
    /// PL-E second phase: confirmCount smart verifiers, unanimity.
    Final,
}

/// A generic phase in a pipeline (LD11). The executor spawns `dump_count + smart_count`
/// verifiers and requires `threshold` APPROVEs to pass.
///
/// `role` is a label for id-offset derivation + display; it does NOT fork execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    /// Letter-suffixed sub-phase identity (LD3): `"1a"`, `"1b"`, ...
    pub id: PhaseId,
    /// Semantic role label (drives verifier-id offset derivation).
    pub role: PhaseRole,
    /// Dump-role verifier count in this phase.
    pub dump_count: u32,
    /// Smart-role verifier count in this phase.
    pub smart_count: u32,
    /// APPROVE threshold for this phase to pass.
    ///   * Gate   → n
    ///   * Confirm → confirmCount
    ///   * Mixed  → m (unanimity, LD22)
    ///   * Final  → confirmCount
    pub threshold: u32,
}

impl Phase {
    /// Total verifiers spawned in this phase = `dump_count + smart_count`.
    pub fn total_verifiers(&self) -> u32 {
        self.dump_count + self.smart_count
    }
}

/// Construct the default pipeline (PL-D): Gate(m D, n) → Confirm(confirmCount S, confirmCount).
///
/// Gate runs first with `m` dump verifiers and threshold `n`. If it passes, Confirm runs
/// with `confirmCount` smart verifiers (unanimity). A sub-phase REJECT is a hard reject
/// for the invocation (LD13).
pub fn default_pipeline(cfg: &Config) -> Vec<Phase> {
    vec![
        Phase {
            id: PhaseId::new("1a"),
            role: PhaseRole::Gate,
            dump_count: cfg.m,
            smart_count: 0,
            threshold: cfg.n,
        },
        Phase {
            id: PhaseId::new("1b"),
            role: PhaseRole::Confirm,
            dump_count: 0,
            smart_count: cfg.confirm_count,
            threshold: cfg.confirm_count,
        },
    ]
}

/// Construct the escalation pipeline (PL-E): Mixed(⌊m/2⌋ D + ⌈m/2⌉ S, m) →
/// Final(confirmCount S, confirmCount).
///
/// Mixed composition is derived from `m` via formula (LD9, never hardcoded): `floor(m/2)`
/// dump + `ceil(m/2)` smart. Mixed threshold = `m` (unanimity, LD22 — corrected from
/// turn3's `n`). Final runs only if Mixed passes.
pub fn escalation_pipeline(cfg: &Config) -> Vec<Phase> {
    let mixed_dump = cfg.m / 2;
    let mixed_smart = cfg.m - mixed_dump; // == ceil(m / 2)
    vec![
        Phase {
            id: PhaseId::new("1a"),
            role: PhaseRole::Mixed,
            dump_count: mixed_dump,
            smart_count: mixed_smart,
            threshold: cfg.m, // unanimity (LD22)
        },
        Phase {
            id: PhaseId::new("1b"),
            role: PhaseRole::Final,
            dump_count: 0,
            smart_count: cfg.confirm_count,
            threshold: cfg.confirm_count,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(n: u32, m: u32, confirm_count: u32) -> Config {
        Config {
            n,
            m,
            confirm_count,
            ..Config::default()
        }
    }

    #[test]
    fn default_pipeline_shape() {
        let pl = default_pipeline(&cfg(2, 3, 1));
        assert_eq!(pl.len(), 2);
        assert_eq!(pl[0].role, PhaseRole::Gate);
        assert_eq!(pl[1].role, PhaseRole::Confirm);
        assert_eq!(pl[0].dump_count, 3);
        assert_eq!(pl[0].threshold, 2);
        assert_eq!(pl[1].smart_count, 1);
        assert_eq!(pl[1].threshold, 1);
    }

    #[test]
    fn escalation_pipeline_mixed_formula_table() {
        for (m, d, s) in [(2u32, 1u32, 1u32), (3, 1, 2), (4, 2, 2), (5, 2, 3)] {
            let pl = escalation_pipeline(&cfg(2, m, 1));
            let mixed = &pl[0];
            assert_eq!((mixed.dump_count, mixed.smart_count), (d, s));
            assert_eq!(mixed.threshold, m, "Mixed threshold = m (unanimity)");
        }
    }

    #[test]
    fn m1_mixed_degenerates() {
        let pl = escalation_pipeline(&cfg(1, 1, 1));
        let mixed = &pl[0];
        assert_eq!((mixed.dump_count, mixed.smart_count), (0, 1));
    }

    #[test]
    fn phase_id_order_alphabetical() {
        let mut ids = vec![
            PhaseId::new("1d"),
            PhaseId::new("1a"),
            PhaseId::new("1c"),
            PhaseId::new("1b"),
        ];
        ids.sort();
        assert_eq!(
            ids.iter().map(|i| i.as_str()).collect::<Vec<_>>(),
            vec!["1a", "1b", "1c", "1d"]
        );
    }
}
