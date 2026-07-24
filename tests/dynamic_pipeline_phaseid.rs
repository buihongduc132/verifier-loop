// T3 — phaseId axis (LD3, LD17, LD18, LD25)
// RED phase: written first, against the spec, before any implementation.
// Tests reference phaseId paths/state/env that do NOT exist yet; expected to fail to compile.

use std::fs;

use verifier_loop::goal;
use verifier_loop::pipeline;

// ---------------------------------------------------------------------------
// T3.1 — Slot path gains phaseId axis (LD17)
// Old: rounds/<round>/<vid>/verdict.json
// New: rounds/<round>/<phaseId>/<vid>/verdict.json
// ---------------------------------------------------------------------------

#[test]
fn slot_path_includes_phase_id_axis() {
    // LD17: Add phase dimension to slot path: rounds/<round>/<phaseId>/<vid>/.
    let path = goal::slot_dir("/store", "goalId123", 1, "1a", "d1");
    assert_eq!(
        path.to_str().unwrap(),
        "/store/goals/goalId123/rounds/1/1a/d1"
    );
}

#[test]
fn slot_path_distinguishes_phases_within_same_round() {
    // Two phases in the same round MUST have distinct slot paths.
    let phase_a = goal::slot_dir("/store", "g1", 1, "1a", "d1");
    let phase_b = goal::slot_dir("/store", "g1", 1, "1b", "s1");
    assert_ne!(
        phase_a, phase_b,
        "phaseId axis must distinguish slots within the same round (LD17)"
    );
}

// ---------------------------------------------------------------------------
// T3.2 — state.json gains currentPhase (LD17, LD18)
// ---------------------------------------------------------------------------

#[test]
fn state_record_has_current_phase_field() {
    // LD17: state.json gains currentPhase field.
    let state = goal::StateRecord {
        current_round: 2,
        current_phase: Some("1b".to_string()),
        esca_count: 0,
        escalation_depth: 0,
        verifier_id_version: 1,
    };
    let j = serde_json::to_string(&state).unwrap();
    assert!(
        j.contains("\"currentPhase\":\"1b\""),
        "state.json must serialize currentPhase camelCase: {j}"
    );
}

#[test]
fn state_record_current_phase_defaults_none_for_legacy() {
    // Legacy goals (v0) have no currentPhase. Default is None.
    let state = goal::StateRecord {
        current_round: 1,
        current_phase: None,
        esca_count: 0,
        escalation_depth: 0,
        verifier_id_version: 0,
    };
    let j = serde_json::to_string(&state).unwrap();
    assert!(
        j.contains("\"currentPhase\":null"),
        "currentPhase defaults to null for legacy goals: {j}"
    );
}

#[test]
fn state_record_has_esca_count_field() {
    // esca counter persisted in state.json (LD4, D5).
    let state = goal::StateRecord {
        current_round: 3,
        current_phase: Some("1a".to_string()),
        esca_count: 2,
        escalation_depth: 0,
        verifier_id_version: 1,
    };
    let j = serde_json::to_string(&state).unwrap();
    assert!(
        j.contains("\"escaCount\":2"),
        "state.json must serialize escaCount camelCase: {j}"
    );
}

#[test]
fn state_record_has_escalation_depth_field() {
    // LD25: escalationDepth counts PL-E cycles.
    let state = goal::StateRecord {
        current_round: 5,
        current_phase: Some("1a".to_string()),
        esca_count: 0,
        escalation_depth: 2,
        verifier_id_version: 1,
    };
    let j = serde_json::to_string(&state).unwrap();
    assert!(
        j.contains("\"escalationDepth\":2"),
        "state.json must serialize escalationDepth camelCase: {j}"
    );
}

#[test]
fn state_record_has_verifier_id_version_field() {
    // LD26: verifierIdVersion (0 = legacy, 1 = d/s scheme).
    let state = goal::StateRecord {
        current_round: 1,
        current_phase: None,
        esca_count: 0,
        escalation_depth: 0,
        verifier_id_version: 1,
    };
    let j = serde_json::to_string(&state).unwrap();
    assert!(
        j.contains("\"verifierIdVersion\":1"),
        "state.json must serialize verifierIdVersion camelCase: {j}"
    );
}

// ---------------------------------------------------------------------------
// T3.3 — VERIFIER_LOOP_PHASE env var (LD17)
// ---------------------------------------------------------------------------

#[test]
fn phase_env_var_constant_exists() {
    // LD17: VERIFIER_LOOP_PHASE env var propagated to V* children.
    assert_eq!(
        goal::PHASE_ENV_VAR,
        "VERIFIER_LOOP_PHASE"
    );
}

// ---------------------------------------------------------------------------
// T3.4 — Hash input includes phaseId (LD25)
// ---------------------------------------------------------------------------

#[test]
fn matching_verdict_has_phase_id_field() {
    // LD25: MatchingVerdict gains phaseId. Hash sorts by (phaseId, verifierId).
    let mv = verifier_loop::consensus::MatchingVerdict {
        phase_id: "1a".to_string(),
        verifier_id: "d1".to_string(),
        registered_at: "2026-07-24T00:00:00Z".to_string(),
    };
    let j = serde_json::to_string(&mv).unwrap();
    assert!(
        j.contains("\"phaseId\":\"1a\""),
        "MatchingVerdict must serialize phaseId camelCase (LD25): {j}"
    );
}

#[test]
fn matching_verdict_sorts_by_phase_id_then_verifier_id() {
    // LD25: sort by (phaseId, verifierId). Two different phase orderings → different hashes.
    use verifier_loop::consensus::MatchingVerdict;
    let mut verdicts = vec![
        MatchingVerdict {
            phase_id: "1b".to_string(),
            verifier_id: "s1".to_string(),
            registered_at: "2026-07-24T00:00:00Z".to_string(),
        },
        MatchingVerdict {
            phase_id: "1a".to_string(),
            verifier_id: "d1".to_string(),
            registered_at: "2026-07-24T00:00:00Z".to_string(),
        },
        MatchingVerdict {
            phase_id: "1a".to_string(),
            verifier_id: "d2".to_string(),
            registered_at: "2026-07-24T00:00:00Z".to_string(),
        },
    ];
    verdicts.sort_by(|a, b| {
        a.phase_id
            .cmp(&b.phase_id)
            .then(a.verifier_id.cmp(&b.verifier_id))
    });
    assert_eq!(verdicts[0].phase_id, "1a");
    assert_eq!(verdicts[0].verifier_id, "d1");
    assert_eq!(verdicts[1].phase_id, "1a");
    assert_eq!(verdicts[1].verifier_id, "d2");
    assert_eq!(verdicts[2].phase_id, "1b");
    assert_eq!(verdicts[2].verifier_id, "s1");
}

// ---------------------------------------------------------------------------
// T3.5 — phaseId order is canonical (LD3: 1a < 1b < 1c < 1d)
// ---------------------------------------------------------------------------

#[test]
fn phase_id_canonical_order_is_alphabetical() {
    // LD3: sub-rounds use letter suffixes. Canonical order = alphabetical.
    let mut ids = vec!["1d", "1a", "1c", "1b"];
    ids.sort();
    assert_eq!(
        ids,
        vec!["1a", "1b", "1c", "1d"],
        "phaseId order must be alphabetical (LD3)"
    );
}

// ---------------------------------------------------------------------------
// T3.6 — phaseId from Phase struct
// ---------------------------------------------------------------------------

#[test]
fn phase_id_type_is_string_like() {
    // PhaseId is a newtype or alias that can be compared and formatted.
    let id = pipeline::PhaseId::from("1a");
    assert_eq!(id.as_str(), "1a");
}

// ---------------------------------------------------------------------------
// T3.7 — slot_dir with legacy phaseId (backward compat)
// ---------------------------------------------------------------------------

#[test]
fn slot_dir_with_legacy_phase_works() {
    // Legacy goals (v0) may not have phaseId. The slot path should handle this.
    // For v0 goals, phaseId is a sentinel (e.g., "0" or the round number).
    let path = goal::slot_dir("/store", "g1", 1, "0", "v1");
    assert!(
        path.to_str().unwrap().contains("0/v1"),
        "legacy slot path must work with sentinel phaseId"
    );
}
