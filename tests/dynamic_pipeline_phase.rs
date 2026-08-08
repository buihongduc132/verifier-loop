// T2 — Phase/Pipeline abstractions (LD11, turn3, LD22)
// RED phase: written first, against the spec, before any implementation.
// Tests reference pipeline types that do NOT exist yet; expected to fail to compile.

use verifier_loop::pipeline;
use verifier_loop::store::Config;

fn test_cfg(n: u32, m: u32, confirm_count: u32) -> Config {
    Config {
        n,
        m,
        max_turn: 3,
        backend: "pi".into(),
        git_diff_max_chars: 10_000,
        verifier_timeout_sec: 1800,
        verifier_prompt_file: None,
        min_goal_chars: 0,
        file_edit_times_max_chars: 8_000,
        context_max_chars: 20_000,
        prompt_budget_bytes: 50_000,
        dump_adapter: None,
        smart_adapter: None,
        confirm_count,
        esca_threshold: 2,
        esca_max_retries: 3,
    }
}

// ---------------------------------------------------------------------------
// T2.1 — default_pipeline (PL-D) shape (LD11, turn3)
// ---------------------------------------------------------------------------

#[test]
fn default_pipeline_has_gate_then_confirm() {
    let cfg = test_cfg(2, 2, 1);
    let pl = pipeline::default_pipeline(&cfg);
    assert_eq!(
        pl.len(),
        2,
        "PL-D has exactly 2 phases: Gate then Confirm"
    );
    assert_eq!(pl[0].role, pipeline::PhaseRole::Gate);
    assert_eq!(pl[1].role, pipeline::PhaseRole::Confirm);
}

#[test]
fn default_pipeline_gate_has_m_dump_zero_smart_threshold_n() {
    let cfg = test_cfg(2, 3, 1);
    let pl = pipeline::default_pipeline(&cfg);
    let gate = &pl[0];
    assert_eq!(gate.role, pipeline::PhaseRole::Gate);
    assert_eq!(
        gate.dump_count, 3,
        "Gate has m dump verifiers (LD10 generic m, not hardcoded 2)"
    );
    assert_eq!(gate.smart_count, 0, "Gate has zero smart verifiers");
    assert_eq!(
        gate.threshold, 2,
        "Gate threshold = n (LD11)"
    );
}

#[test]
fn default_pipeline_confirm_has_zero_dump_confirm_count_smart() {
    let cfg = test_cfg(2, 2, 1);
    let pl = pipeline::default_pipeline(&cfg);
    let confirm = &pl[1];
    assert_eq!(confirm.role, pipeline::PhaseRole::Confirm);
    assert_eq!(confirm.dump_count, 0, "Confirm has zero dump");
    assert_eq!(
        confirm.smart_count, 1,
        "Confirm has confirmCount smart verifiers"
    );
    assert_eq!(
        confirm.threshold, 1,
        "Confirm threshold = confirmCount (unanimity for the smart set)"
    );
}

#[test]
fn default_pipeline_phase_ids_are_letter_suffixed() {
    // LD3: sub-rounds use letter suffixes 1a, 1b, 1c, 1d, NOT integers.
    let cfg = test_cfg(2, 2, 1);
    let pl = pipeline::default_pipeline(&cfg);
    assert_eq!(pl[0].id.as_str(), "1a", "first phase id = 1a (LD3)");
    assert_eq!(pl[1].id.as_str(), "1b", "second phase id = 1b (LD3)");
}

// ---------------------------------------------------------------------------
// T2.2 — escalation_pipeline (PL-E) shape (LD4, turn3, LD9, LD22)
// ---------------------------------------------------------------------------

#[test]
fn escalation_pipeline_has_mixed_then_final() {
    let cfg = test_cfg(2, 2, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    assert_eq!(
        pl.len(),
        2,
        "PL-E has exactly 2 phases: Mixed then Final"
    );
    assert_eq!(pl[0].role, pipeline::PhaseRole::Mixed);
    assert_eq!(pl[1].role, pipeline::PhaseRole::Final);
}

#[test]
fn escalation_pipeline_mixed_has_floor_half_dump_ceil_half_smart() {
    // LD9: Mixed composition derived from m via formula.
    // mixedDump = floor(m/2), mixedSmart = ceil(m/2)
    // m=2 → 1 D + 1 S
    let cfg = test_cfg(2, 2, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl[0];
    assert_eq!(mixed.role, pipeline::PhaseRole::Mixed);
    assert_eq!(mixed.dump_count, 1, "m=2: floor(2/2)=1 dump (LD9)");
    assert_eq!(mixed.smart_count, 1, "m=2: ceil(2/2)=1 smart (LD9)");
    assert_eq!(mixed.dump_count + mixed.smart_count, 2, "Mixed total = m");
}

#[test]
fn escalation_pipeline_mixed_composition_for_m3() {
    // m=3 → floor(3/2)=1 D + ceil(3/2)=2 S
    let cfg = test_cfg(2, 3, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl[0];
    assert_eq!(mixed.dump_count, 1, "m=3: floor(3/2)=1 dump (LD9)");
    assert_eq!(mixed.smart_count, 2, "m=3: ceil(3/2)=2 smart (LD9)");
}

#[test]
fn escalation_pipeline_mixed_composition_for_m5() {
    // m=5 → floor(5/2)=2 D + ceil(5/2)=3 S
    let cfg = test_cfg(2, 5, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl[0];
    assert_eq!(mixed.dump_count, 2, "m=5: floor(5/2)=2 dump (LD9)");
    assert_eq!(mixed.smart_count, 3, "m=5: ceil(5/2)=3 smart (LD9)");
}

#[test]
fn escalation_pipeline_mixed_threshold_is_m_unanimity() {
    // LD22: Mixed threshold = m (unanimity), NOT n.
    let cfg = test_cfg(2, 3, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl[0];
    assert_eq!(
        mixed.threshold, 3,
        "Mixed threshold = m (unanimity), NOT n (LD22). n=2 but threshold must be 3=m"
    );
}

#[test]
fn escalation_pipeline_final_has_confirm_count_smart() {
    let cfg = test_cfg(2, 2, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let final_phase = &pl[1];
    assert_eq!(final_phase.role, pipeline::PhaseRole::Final);
    assert_eq!(final_phase.dump_count, 0, "Final has zero dump");
    assert_eq!(
        final_phase.smart_count, 1,
        "Final has confirmCount smart verifiers"
    );
    assert_eq!(
        final_phase.threshold, 1,
        "Final threshold = confirmCount"
    );
}

#[test]
fn escalation_pipeline_phase_ids_are_letter_suffixed() {
    let cfg = test_cfg(2, 2, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    assert_eq!(pl[0].id.as_str(), "1a", "first phase id = 1a (LD3)");
    assert_eq!(pl[1].id.as_str(), "1b", "second phase id = 1b (LD3)");
}

// ---------------------------------------------------------------------------
// T2.3 — Mixed formula table (LD9, rot-proof — derived, not hardcoded)
// ---------------------------------------------------------------------------

#[test]
fn mixed_composition_table_matches_derived_formula() {
    // The Mixed phase composition is DERIVED from m, never hardcoded (LD9).
    // table from turn3:
    //   m=2 → 1 D + 1 S
    //   m=3 → 1 D + 2 S
    //   m=4 → 2 D + 2 S
    //   m=5 → 2 D + 3 S
    for (m, exp_dump, exp_smart) in [(2, 1, 1), (3, 1, 2), (4, 2, 2), (5, 2, 3)] {
        let cfg = test_cfg(2, m, 1);
        let pl = pipeline::escalation_pipeline(&cfg);
        let mixed = &pl[0];
        assert_eq!(
            mixed.dump_count, exp_dump,
            "m={}: dump_count must be floor(m/2)={}, got {}",
            m, exp_dump, mixed.dump_count
        );
        assert_eq!(
            mixed.smart_count, exp_smart,
            "m={}: smart_count must be ceil(m/2)={}, got {}",
            m, exp_smart, mixed.smart_count
        );
        assert_eq!(
            mixed.dump_count + mixed.smart_count,
            m,
            "m={}: total must equal m",
            m
        );
    }
}

// ---------------------------------------------------------------------------
// T2.4 — m=1 edge case (LD15, turn3)
// ---------------------------------------------------------------------------

#[test]
fn m1_mixed_degenerates_to_confirm_equivalent() {
    // LD15 / turn3: for m=1, Mixed degenerates: floor(1/2)=0 D, ceil(1/2)=1 S.
    // Mixed ≡ Confirm. PL-E is effectively PL-D.
    let cfg = test_cfg(1, 1, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl[0];
    assert_eq!(mixed.dump_count, 0, "m=1: floor(1/2)=0 dump");
    assert_eq!(mixed.smart_count, 1, "m=1: ceil(1/2)=1 smart");
    assert_eq!(
        mixed.threshold, 1,
        "m=1: Mixed threshold = m = 1 (unanimity of 1 verifier)"
    );
}

// ---------------------------------------------------------------------------
// T2.5 — Phase struct invariants (LD11 rot-proof)
// ---------------------------------------------------------------------------

#[test]
fn phase_role_enum_has_all_4_variants() {
    // LD11: PhaseRole is a label enum. Must have Gate | Confirm | Mixed | Final.
    let _gate = pipeline::PhaseRole::Gate;
    let _confirm = pipeline::PhaseRole::Confirm;
    let _mixed = pipeline::PhaseRole::Mixed;
    let _final = pipeline::PhaseRole::Final;
}

#[test]
fn phase_total_verifiers_is_dump_plus_smart() {
    let cfg = test_cfg(2, 4, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl[0];
    let total = mixed.total_verifiers();
    assert_eq!(total, 4, "total_verifiers = dump_count + smart_count");
}
