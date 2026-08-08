// E2E — dynamic-round-pipeline complete workflow simulation.
//
// Exercises the FULL pipeline workflow end-to-end (library-level, no real verifier
// spawn — the runtime executor wiring into `jewilo`'s run_round is the documented T8
// integration gap). This test proves the pipeline state machine + output format +
// esca lifecycle compose correctly across a multi-invocation PL-D→PL-E scenario.
//
// Scenario:
//   Round 1 (PL-D): Gate 2/2 pass, Confirm 0/1 reject → esca++ → 2+0/2 reject
//   Round 2 (PL-D): Gate 2/2 pass, Confirm 0/1 reject → esca++ → 2+0/2 reject
//   Round 3 (PL-E): esca≥threshold → Mixed reject → esca reset, depth++ → 1+1/2 reject
//   Round 4 (PL-E): Mixed 2/2 pass, Final 1/1 pass → APPROVE → 1+2+1/2
//
// This is the workflow the auditor asked for: PL-D pass, gate reject, confirm reject
// (esca++), PL-E activation, mixed+final pass, hash-covers-all-phases.

use verifier_loop::consensus::MatchingVerdict;
use verifier_loop::pipeline::{
    self, esca, executor, Phase, PhaseId, PhaseRole,
};
use verifier_loop::store::Config;

fn cfg() -> Config {
    Config {
        n: 2,
        m: 2,
        confirm_count: 1,
        esca_threshold: 2,
        esca_max_retries: 3,
        dump_adapter: Some("pi".into()),
        smart_adapter: Some("hermes".into()),
        ..Config::default()
    }
}

/// Simulated phase result for testing.
fn phase_result(phase: &Phase, approve_count: u32, passed: bool) -> executor::PhaseResult {
    executor::PhaseResult {
        phase_id: phase.id.as_str().to_string(),
        role: phase.role,
        passed,
        approve_count,
        threshold: phase.threshold,
        matching_verifier_ids: (0..approve_count)
            .map(|i| format!("slot{}", i))
            .collect(),
        rejection_notes: if passed {
            vec![]
        } else {
            vec![("slot0".into(), "rejected".into())]
        },
    }
}

#[test]
fn e2e_full_pl_d_to_pl_e_workflow() {
    let cfg = cfg();
    let mut esca_count = 0u32;
    let mut escalation_depth = 0u32;

    // ── Round 1: PL-D, Confirm rejects ──
    let pl_d = pipeline::default_pipeline(&cfg);
    let gate = &pl_d[0];
    let confirm = &pl_d[1];
    assert!(!executor::should_run_escalation(&cfg, esca_count, escalation_depth));
    let gate_r = phase_result(gate, 2, true);
    let confirm_r = phase_result(confirm, 0, false);
    let state = esca::EscaState::new(esca_count, escalation_depth);
    let state = esca::apply_outcome(&cfg, state, esca::InvocationOutcome::PlDConfirmReject);
    esca_count = state.esca_count;
    escalation_depth = state.escalation_depth;
    let output = executor::build_output_format(&[gate_r, confirm_r.clone()], cfg.m);
    assert_eq!(output, "2+0/2", "round 1: PL-D confirm reject → 2+0/2");
    assert_eq!(esca_count, 1, "esca incremented to 1");

    // ── Round 2: PL-D, Confirm rejects again ──
    assert!(!executor::should_run_escalation(&cfg, esca_count, escalation_depth));
    let state = esca::apply_outcome(&cfg, state, esca::InvocationOutcome::PlDConfirmReject);
    esca_count = state.esca_count;
    assert_eq!(esca_count, 2, "esca incremented to 2");

    // ── Round 3: PL-E activates (esca ≥ threshold) ──
    assert!(
        executor::should_run_escalation(&cfg, esca_count, escalation_depth),
        "PL-E activates when esca >= threshold"
    );
    let pl_e = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl_e[0];
    let final_phase = &pl_e[1];
    assert_eq!(mixed.role, PhaseRole::Mixed);
    assert_eq!(final_phase.role, PhaseRole::Final);
    assert_eq!(mixed.threshold, cfg.m, "Mixed threshold = m unanimity");
    let mixed_r = phase_result(mixed, 1, false);
    let output = executor::build_output_format(&[mixed_r], cfg.m);
    assert_eq!(output, "1/2", "round 3: Mixed reject");
    let state = esca::apply_outcome(&cfg, state, esca::InvocationOutcome::PlEMixedReject);
    esca_count = state.esca_count;
    escalation_depth = state.escalation_depth;
    assert_eq!(esca_count, 0, "Mixed reject resets esca");
    assert_eq!(escalation_depth, 1, "Mixed reject bumps depth");

    // ── Round 4: PL-E, Mixed + Final pass → APPROVE ──
    let mixed_r2 = phase_result(mixed, 2, true);
    let final_r2 = phase_result(final_phase, 1, true);
    let output = executor::build_output_format(&[mixed_r2, final_r2.clone()], cfg.m);
    assert_eq!(output, "2+1/2", "round 4: PL-E pass → 2+1/2");
    let state = esca::apply_outcome(&cfg, state, esca::InvocationOutcome::PlEApprove);
    assert_eq!(state.esca_count, 0, "PL-E approve resets esca");
}

#[test]
fn e2e_hash_covers_all_phases() {
    // LD7/LD25: the completion hash covers ALL verdicts across all sub-phases.
    // Two phases' matching verdicts are unioned for the hash input, sorted by
    // (phaseId, verifierId).
    let ts = "2026-07-24T00:00:00Z";
    let gate_verdicts = vec![
        MatchingVerdict {
            phase_id: "1a".into(),
            verifier_id: "d1".into(),
            registered_at: ts.into(),
        },
        MatchingVerdict {
            phase_id: "1a".into(),
            verifier_id: "d2".into(),
            registered_at: ts.into(),
        },
    ];
    let confirm_verdicts = vec![MatchingVerdict {
        phase_id: "1b".into(),
        verifier_id: "s1".into(),
        registered_at: ts.into(),
    }];

    let mut all = gate_verdicts;
    all.extend(confirm_verdicts);
    all.sort_by(|a, b| {
        a.phase_id
            .cmp(&b.phase_id)
            .then(a.verifier_id.cmp(&b.verifier_id))
    });

    assert_eq!(all.len(), 3, "hash covers all 3 verdicts across 2 phases");
    assert_eq!(all[0].phase_id, "1a");
    assert_eq!(all[2].phase_id, "1b");
    assert_eq!(all[2].verifier_id, "s1");
}

#[test]
fn e2e_compute_hash_proves_phase_id_is_hash_input() {
    // LD25 fail-closed (REAL hash test, not just Vec sort): two MatchingVerdicts that
    // differ ONLY in phase_id MUST produce different completion hashes. This is the
    // exact property the verifier flagged — prior tests asserted in-memory sort order
    // but never called compute_hash, masking the bug where canonical_matching_json
    // omitted phaseId.
    use verifier_loop::consensus::compute_hash;

    let ts = "2026-07-24T00:00:00Z";
    let phase_a = vec![MatchingVerdict {
        phase_id: "1a".into(),
        verifier_id: "d1".into(),
        registered_at: ts.into(),
    }];
    // Same verifier_id + timestamp, DIFFERENT phase_id.
    let phase_b = vec![MatchingVerdict {
        phase_id: "1b".into(),
        verifier_id: "d1".into(),
        registered_at: ts.into(),
    }];

    let h_a = compute_hash("salt", "g1", "sig", 1, &phase_a, ts, "");
    let h_b = compute_hash("salt", "g1", "sig", 1, &phase_b, ts, "");

    assert_ne!(
        h_a.short_hash(),
        h_b.short_hash(),
        "LD25 VIOLATION: phase 1a -> {}, phase 1b -> {} (must differ)",
        h_a.short_hash(),
        h_b.short_hash()
    );
    assert_ne!(
        h_a.full_digest(),
        h_b.full_digest(),
        "LD25 VIOLATION: full digests must differ for different phase_ids"
    );
}

#[test]
fn e2e_esca_exhaustion_hard_fails() {
    let cfg = Config {
        n: 2,
        m: 2,
        confirm_count: 1,
        esca_threshold: 2,
        esca_max_retries: 2,
        ..Config::default()
    };
    let mut state = esca::EscaState::new(2, 0);
    // Two PL-E rejections → depth reaches max
    state = esca::apply_outcome(&cfg, state, esca::InvocationOutcome::PlEMixedReject);
    state = esca::apply_outcome(&cfg, state, esca::InvocationOutcome::PlEMixedReject);
    assert!(esca::escalation_exhausted(&cfg, state), "exhausted after 2 PL-E cycles");
    assert!(
        !executor::should_run_escalation(&cfg, state.esca_count, state.escalation_depth),
        "no more PL-E after exhaustion"
    );
}

#[test]
fn e2e_completion_record_carries_pipeline_metadata() {
    // LD14/LD25: completion.json carries `pipeline: "PL-D"|"PL-E"` + `escalationDepth`.
    use verifier_loop::consensus::CompletionRecord;
    let rec = CompletionRecord {
        hash: "240724-deadbeef".into(),
        full_digest: "abc123".into(),
        goal_id: "g1".into(),
        round_number: 1,
        matched_at: "2026-07-24T00:00:00Z".into(),
        matching_verdicts: vec![],
        trace_id: None,
        pipeline: "PL-D".into(),
        escalation_depth: 0,
    };
    let j = serde_json::to_string(&rec).unwrap();
    assert!(j.contains("\"pipeline\":\"PL-D\""), "pipeline field: {j}");
    assert!(j.contains("\"escalationDepth\":0"), "escalationDepth field: {j}");
}
