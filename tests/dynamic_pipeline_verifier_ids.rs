// T4 — d/s verifierIds (LD16, LD26)
// RED phase: written first, against the spec, before any implementation.
// Tests reference id-scheme functions that do NOT exist yet; expected to fail to compile.

use verifier_loop::pipeline;
use verifier_loop::spawn;
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
// T4.1 — Gate phase ids (m dump verifiers): d1..d_m
// ---------------------------------------------------------------------------

#[test]
fn gate_phase_dump_ids_are_d1_through_dm() {
    // LD10, LD16, LD26: Gate has m dump verifiers, ids d1..d_m.
    let cfg = test_cfg(2, 3, 1);
    let pl = pipeline::default_pipeline(&cfg);
    let gate = &pl[0];
    let ids = spawn::verifier_ids_for_phase(gate, &cfg);
    assert_eq!(
        ids,
        vec!["d1".to_string(), "d2".to_string(), "d3".to_string()],
        "Gate m=3: ids must be d1, d2, d3 (LD16, LD10)"
    );
}

#[test]
fn gate_phase_ids_for_m2() {
    let cfg = test_cfg(2, 2, 1);
    let pl = pipeline::default_pipeline(&cfg);
    let gate = &pl[0];
    let ids = spawn::verifier_ids_for_phase(gate, &cfg);
    assert_eq!(
        ids,
        vec!["d1".to_string(), "d2".to_string()],
        "Gate m=2: ids must be d1, d2"
    );
}

// ---------------------------------------------------------------------------
// T4.2 — Confirm phase ids (k smart verifiers): s1..s_k
// ---------------------------------------------------------------------------

#[test]
fn confirm_phase_smart_ids_are_s1_through_sk() {
    // LD16, LD26: Confirm has confirmCount smart verifiers, ids s1..s_k.
    let cfg = test_cfg(2, 2, 1);
    let pl = pipeline::default_pipeline(&cfg);
    let confirm = &pl[1];
    let ids = spawn::verifier_ids_for_phase(confirm, &cfg);
    assert_eq!(
        ids,
        vec!["s1".to_string()],
        "Confirm confirmCount=1: ids must be s1"
    );
}

#[test]
fn confirm_phase_ids_for_k2() {
    let cfg = test_cfg(2, 2, 2);
    let pl = pipeline::default_pipeline(&cfg);
    let confirm = &pl[1];
    let ids = spawn::verifier_ids_for_phase(confirm, &cfg);
    assert_eq!(
        ids,
        vec!["s1".to_string(), "s2".to_string()],
        "Confirm confirmCount=2: ids must be s1, s2"
    );
}

// ---------------------------------------------------------------------------
// T4.3 — Mixed phase ids (monotonic continuation, LD26)
// Mixed: d_{m+1}..d_{m+⌊m/2⌋}, s_{k+1}..s_{k+⌈m/2⌉}
// ---------------------------------------------------------------------------

#[test]
fn mixed_phase_dump_ids_continue_after_gate() {
    // LD26: Monotonic per-invocation. Mixed dump ids continue after Gate.
    // Gate: d1..d_m. Mixed: d_{m+1}..d_{m+⌊m/2⌋}.
    let cfg = test_cfg(2, 4, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl[0];
    // m=4: Gate used d1..d4. Mixed dump = floor(4/2)=2 → d5, d6
    let dump_ids = spawn::verifier_ids_for_phase_role(mixed, &cfg, pipeline::PhaseRole::Gate);
    assert_eq!(
        dump_ids,
        vec!["d5".to_string(), "d6".to_string()],
        "Mixed m=4: dump ids must continue after Gate (d5, d6), not restart (LD26)"
    );
}

#[test]
fn mixed_phase_smart_ids_continue_after_confirm() {
    // LD26: Mixed smart ids continue after Confirm.
    // Confirm: s1..s_k. Mixed smart: s_{k+1}..s_{k+⌈m/2⌉}.
    let cfg = test_cfg(2, 4, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl[0];
    // m=4, k=1: Confirm used s1. Mixed smart = ceil(4/2)=2 → s2, s3
    let smart_ids = spawn::verifier_ids_for_phase_role(mixed, &cfg, pipeline::PhaseRole::Confirm);
    assert_eq!(
        smart_ids,
        vec!["s2".to_string(), "s3".to_string()],
        "Mixed m=4, k=1: smart ids must continue after Confirm (s2, s3), not restart (LD26)"
    );
}

#[test]
fn mixed_phase_ids_for_m3() {
    // m=3, k=1: Gate d1,d2,d3. Mixed dump=floor(3/2)=1 → d4. Mixed smart=ceil(3/2)=2 → s2,s3.
    let cfg = test_cfg(2, 3, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mixed = &pl[0];
    let dump_ids = spawn::verifier_ids_for_phase_role(mixed, &cfg, pipeline::PhaseRole::Gate);
    let smart_ids = spawn::verifier_ids_for_phase_role(mixed, &cfg, pipeline::PhaseRole::Confirm);
    assert_eq!(
        dump_ids,
        vec!["d4".to_string()],
        "Mixed m=3: dump ids continue after Gate (d4)"
    );
    assert_eq!(
        smart_ids,
        vec!["s2".to_string(), "s3".to_string()],
        "Mixed m=3: smart ids continue after Confirm (s2, s3)"
    );
}

// ---------------------------------------------------------------------------
// T4.4 — Final phase ids (continue after Mixed smart)
// Final: s_{k+⌈m/2⌉+1}..s_{k+⌈m/2⌉+k}
// ---------------------------------------------------------------------------

#[test]
fn final_phase_smart_ids_continue_after_mixed_smart() {
    // LD26: Final smart ids continue after Mixed smart.
    // Confirm: s1..s_k. Mixed smart: s_{k+1}..s_{k+⌈m/2⌉}. Final: s_{k+⌈m/2⌉+1}..
    let cfg = test_cfg(2, 4, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let final_phase = &pl[1];
    // m=4, k=1: Confirm s1. Mixed smart s2,s3. Final: s4
    let ids = spawn::verifier_ids_for_phase(final_phase, &cfg);
    assert_eq!(
        ids,
        vec!["s4".to_string()],
        "Final m=4, k=1: smart ids continue after Mixed (s4), not restart (LD26)"
    );
}

#[test]
fn final_phase_ids_for_k2() {
    // m=4, k=2: Confirm s1,s2. Mixed smart s3,s4. Final: s5,s6
    let cfg = test_cfg(2, 4, 2);
    let pl = pipeline::escalation_pipeline(&cfg);
    let final_phase = &pl[1];
    let ids = spawn::verifier_ids_for_phase(final_phase, &cfg);
    assert_eq!(
        ids,
        vec!["s5".to_string(), "s6".to_string()],
        "Final m=4, k=2: smart ids continue after Mixed (s5, s6)"
    );
}

// ---------------------------------------------------------------------------
// T4.5 — No collisions within an invocation (LD16, LD26)
// ---------------------------------------------------------------------------

#[test]
fn no_id_collisions_within_pl_d_invocation() {
    // PL-D: Gate d1..d_m + Confirm s1..s_k. No overlap between d and s namespaces.
    let cfg = test_cfg(2, 3, 2);
    let pl = pipeline::default_pipeline(&cfg);
    let mut all_ids = Vec::new();
    for phase in &pl {
        all_ids.extend(spawn::verifier_ids_for_phase(phase, &cfg));
    }
    let mut sorted = all_ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        all_ids.len(),
        "no id collisions in PL-D invocation: {:?}",
        all_ids
    );
}

#[test]
fn no_id_collisions_within_pl_e_invocation() {
    // PL-E: Mixed (d_{m+1}.. + s_{k+1}..) + Final (s_{k+⌈m/2⌉+1}..).
    // No overlap. Monotonic counters ensure uniqueness (LD26).
    let cfg = test_cfg(2, 4, 1);
    let pl = pipeline::escalation_pipeline(&cfg);
    let mut all_ids = Vec::new();
    for phase in &pl {
        all_ids.extend(spawn::verifier_ids_for_phase(phase, &cfg));
    }
    let mut sorted = all_ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        all_ids.len(),
        "no id collisions in PL-E invocation: {:?}",
        all_ids
    );
}

// ---------------------------------------------------------------------------
// T4.6 — Version migration (LD26: v0 = legacy, v1 = d/s)
// ---------------------------------------------------------------------------

#[test]
fn verifier_id_v0_uses_legacy_v_namespace() {
    // LD26: version 0 = legacy v{i+1} scheme.
    let id = spawn::verifier_id_legacy(0);
    assert_eq!(id, "v1", "v0: index 0 → v1 (legacy)");

    let id = spawn::verifier_id_legacy(2);
    assert_eq!(id, "v3", "v0: index 2 → v3 (legacy)");
}

#[test]
fn verifier_id_v1_uses_ds_scheme() {
    // LD26: version 1 = d/s scheme.
    let id = spawn::verifier_id_role(1, pipeline::PhaseRole::Gate, 0);
    assert_eq!(id, "d1", "v1: Gate index 0 → d1");

    let id = spawn::verifier_id_role(1, pipeline::PhaseRole::Confirm, 0);
    assert_eq!(id, "s1", "v1: Confirm index 0 → s1");

    let id = spawn::verifier_id_role(1, pipeline::PhaseRole::Gate, 2);
    assert_eq!(id, "d3", "v1: Gate index 2 → d3");

    let id = spawn::verifier_id_role(1, pipeline::PhaseRole::Confirm, 2);
    assert_eq!(id, "s3", "v1: Confirm index 2 → s3");
}

#[test]
fn verifier_id_dispatches_on_version() {
    // verifier_id(version, role, idx) dispatches on version.
    let v0_id = spawn::verifier_id(0, pipeline::PhaseRole::Gate, 0);
    let v1_id = spawn::verifier_id(1, pipeline::PhaseRole::Gate, 0);
    assert_eq!(v0_id, "v1", "version 0 → legacy v namespace");
    assert_eq!(v1_id, "d1", "version 1 → d/s scheme");
}

// ---------------------------------------------------------------------------
// T4.7 — role prefix extraction (LD16)
// ---------------------------------------------------------------------------

#[test]
fn role_prefix_from_verifier_id() {
    // LD16: role-prefixed ids (d1, s1). Extract role for audit/display.
    assert_eq!(
        spawn::role_from_verifier_id("d1"),
        Some(pipeline::PhaseRole::Gate),
        "d1 → Gate role"
    );
    assert_eq!(
        spawn::role_from_verifier_id("s1"),
        Some(pipeline::PhaseRole::Confirm),
        "s1 → Confirm role"
    );
    assert_eq!(
        spawn::role_from_verifier_id("v1"),
        None,
        "v1 (legacy) → None (no role prefix)"
    );
}
