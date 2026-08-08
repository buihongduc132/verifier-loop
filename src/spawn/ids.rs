//! Verifier-id scheme for the dynamic-round-pipeline (LD16, LD26).
//!
//! Two versions (LD26 migration gate):
//!   * `0` — legacy `v{idx+1}` (single-phase goals created before dynamic-pipeline).
//!   * `1` — role-prefixed + monotonic-per-invocation `d{..}` / `s{..}`.
//!
//! ## v1 id table (canonical pipelines)
//!
//! ```text
//! Gate    (m D):              d1       .. d_m
//! Confirm (k S):              s1       .. s_k
//! Mixed   (⌊m/2⌋ D + ⌈m/2⌉ S): d_{m+1}  .. d_{m+⌊m/2⌋} ; s_{k+1} .. s_{k+⌈m/2⌉}
//! Final   (k S):              s_{k+⌈m/2⌉+1} .. s_{k+⌈m/2⌉+k}
//! ```
//!
//! Monotonic per-invocation: the dump counter and smart counter each only increment
//! within one invocation, guaranteeing no collisions regardless of phase composition.

use crate::pipeline::{Phase, PhaseRole};
use crate::store::Config;

/// Legacy v0 id: `v{idx+1}`.
pub fn verifier_id_legacy(idx: usize) -> String {
    format!("v{}", idx + 1)
}

/// v1 role-prefixed id for a single slot.
///
/// `Gate` / `Mixed` (dump side) → `d{idx+1}`; `Confirm` / `Final` (smart side) →
/// `s{idx+1}`. `Mixed` and `Final` are treated by their role family here (dump vs smart);
/// the offset (which counter to continue from) is resolved by
/// [`verifier_ids_for_phase_role`].
pub fn verifier_id_role(_version: u8, role: PhaseRole, idx: usize) -> String {
    match role {
        // Dump-family roles use the `d` prefix.
        PhaseRole::Gate | PhaseRole::Mixed => format!("d{}", idx + 1),
        // Smart-family roles use the `s` prefix.
        PhaseRole::Confirm | PhaseRole::Final => format!("s{}", idx + 1),
    }
}

/// Dispatch on `version`: `0` → legacy `v{idx+1}`, `1` → role-prefixed.
pub fn verifier_id(version: u8, role: PhaseRole, idx: usize) -> String {
    match version {
        0 => verifier_id_legacy(idx),
        _ => verifier_id_role(version, role, idx),
    }
}

/// Extract the role family from a verifier id (LD16).
///
/// `d{n}` → `Some(Gate)` (dump family), `s{n}` → `Some(Confirm)` (smart family), `v{n}`
/// → `None` (legacy, no role prefix).
pub fn role_from_verifier_id(vid: &str) -> Option<PhaseRole> {
    if let Some(rest) = vid.strip_prefix('d') {
        if rest.parse::<u32>().is_ok() {
            return Some(PhaseRole::Gate);
        }
    }
    if let Some(rest) = vid.strip_prefix('s') {
        if rest.parse::<u32>().is_ok() {
            return Some(PhaseRole::Confirm);
        }
    }
    None
}

/// Starting index (1-based) for the dump slots of a phase, derived from the phase ROLE
/// and config (LD26 monotonic continuation).
///
///   * `Gate`   → dump starts at 1.
///   * `Mixed`  → dump continues after Gate: starts at `m + 1`.
///   * `Confirm`/`Final` → no dump slots (start is irrelevant).
fn dump_id_start(phase_role: PhaseRole, cfg: &Config) -> u32 {
    match phase_role {
        PhaseRole::Gate => 1,
        PhaseRole::Mixed => cfg.m.saturating_add(1),
        PhaseRole::Confirm | PhaseRole::Final => 1,
    }
}

/// Starting index (1-based) for the smart slots of a phase (LD26).
///
///   * `Confirm` → smart starts at 1.
///   * `Mixed`   → smart continues after Confirm: starts at `confirm_count + 1`.
///   * `Final`   → smart continues after Mixed smart: starts at
///     `confirm_count + ceil(m/2) + 1`.
fn smart_id_start(phase_role: PhaseRole, cfg: &Config) -> u32 {
    let mixed_smart = cfg.m - (cfg.m / 2); // == ceil(m/2)
    match phase_role {
        PhaseRole::Confirm => 1,
        PhaseRole::Mixed => cfg.confirm_count.saturating_add(1),
        PhaseRole::Final => cfg
            .confirm_count
            .saturating_add(mixed_smart)
            .saturating_add(1),
        PhaseRole::Gate => 1,
    }
}

/// The dump-side or smart-side ids for a phase, selected by `query_role`.
///
/// `query_role = Gate` selects the dump ids; `query_role = Confirm` selects the smart ids.
/// (Mixed/Final as `query_role` behave like their family: Mixed→dump, Final→smart.)
pub fn verifier_ids_for_phase_role(
    phase: &Phase,
    cfg: &Config,
    query_role: PhaseRole,
) -> Vec<String> {
    match query_role {
        // Dump-family query.
        PhaseRole::Gate | PhaseRole::Mixed => {
            let start = dump_id_start(phase.role, cfg);
            (0..phase.dump_count)
                .map(|i| format!("d{}", start + i))
                .collect()
        }
        // Smart-family query.
        PhaseRole::Confirm | PhaseRole::Final => {
            let start = smart_id_start(phase.role, cfg);
            (0..phase.smart_count)
                .map(|i| format!("s{}", start + i))
                .collect()
        }
    }
}

/// All verifier ids for a phase (dump ids first, then smart ids), v1 scheme.
pub fn verifier_ids_for_phase(phase: &Phase, cfg: &Config) -> Vec<String> {
    let mut ids = verifier_ids_for_phase_role(phase, cfg, PhaseRole::Gate);
    ids.extend(verifier_ids_for_phase_role(phase, cfg, PhaseRole::Confirm));
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{default_pipeline, escalation_pipeline};
    use crate::store::Config;

    fn cfg(n: u32, m: u32, confirm_count: u32) -> Config {
        Config {
            n,
            m,
            confirm_count,
            ..Config::default()
        }
    }

    #[test]
    fn gate_ids() {
        let c = cfg(2, 3, 1);
        let gate = &default_pipeline(&c)[0];
        assert_eq!(
            verifier_ids_for_phase(gate, &c),
            vec!["d1", "d2", "d3"]
        );
    }

    #[test]
    fn mixed_continues_after_gate_and_confirm() {
        // m=4, k=1: Gate d1..d4, Confirm s1, Mixed dump d5..d6, Mixed smart s2..s3.
        let c = cfg(2, 4, 1);
        let mixed = &escalation_pipeline(&c)[0];
        assert_eq!(
            verifier_ids_for_phase_role(mixed, &c, PhaseRole::Gate),
            vec!["d5", "d6"],
            "Mixed dump continues after Gate"
        );
        assert_eq!(
            verifier_ids_for_phase_role(mixed, &c, PhaseRole::Confirm),
            vec!["s2", "s3"],
            "Mixed smart continues after Confirm"
        );
    }

    #[test]
    fn final_continues_after_mixed_smart() {
        // m=4, k=2: Confirm s1,s2; Mixed smart s3,s4; Final s5,s6.
        let c = cfg(2, 4, 2);
        let final_phase = &escalation_pipeline(&c)[1];
        assert_eq!(
            verifier_ids_for_phase(final_phase, &c),
            vec!["s5", "s6"],
            "Final continues after Mixed smart"
        );
    }

    #[test]
    fn no_collisions_pl_d() {
        let c = cfg(2, 3, 2);
        let mut all: Vec<String> = Vec::new();
        for phase in &default_pipeline(&c) {
            all.extend(verifier_ids_for_phase(phase, &c));
        }
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "collisions: {:?}", all);
    }

    #[test]
    fn no_collisions_pl_e() {
        let c = cfg(2, 4, 1);
        let mut all: Vec<String> = Vec::new();
        for phase in &escalation_pipeline(&c) {
            all.extend(verifier_ids_for_phase(phase, &c));
        }
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "collisions: {:?}", all);
    }

    #[test]
    fn role_extraction() {
        assert_eq!(role_from_verifier_id("d1"), Some(PhaseRole::Gate));
        assert_eq!(role_from_verifier_id("s1"), Some(PhaseRole::Confirm));
        assert_eq!(role_from_verifier_id("v1"), None);
    }
}
