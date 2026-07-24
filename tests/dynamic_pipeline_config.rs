// T1 — Config schema (LD19, LD23, LD28, LD30)
// RED phase: written first, against the spec, before any implementation.
// Tests reference Config fields that do NOT exist yet; expected to fail to compile.

use std::fs;

use verifier_loop::store;

// ---------------------------------------------------------------------------
// T1.1 — Parse 6 new fields (LD19, LD30)
// ---------------------------------------------------------------------------

#[test]
fn config_parses_dump_adapter_when_present() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"dumpAdapter":"hermes"}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads with dumpAdapter");
    assert_eq!(
        cfg.dump_adapter,
        Some("hermes".to_string()),
        "dumpAdapter parses into Config.dump_adapter (camelCase on disk, Option<String>)"
    );
}

#[test]
fn config_parses_smart_adapter_when_present() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"smartAdapter":"acpx"}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads with smartAdapter");
    assert_eq!(
        cfg.smart_adapter,
        Some("acpx".to_string()),
        "smartAdapter parses into Config.smart_adapter (camelCase on disk, Option<String>)"
    );
}

#[test]
fn config_parses_confirm_count_when_present() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"confirmCount":2}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads with confirmCount");
    assert_eq!(
        cfg.confirm_count, 2,
        "confirmCount parses into Config.confirm_count (camelCase on disk, u32)"
    );
}

#[test]
fn config_parses_esca_threshold_when_present() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"escaThreshold":3}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads with escaThreshold");
    assert_eq!(
        cfg.esca_threshold, 3,
        "escaThreshold parses into Config.esca_threshold (camelCase on disk, u32, LD30 full-word)"
    );
}

#[test]
fn config_parses_esca_max_retries_when_present() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"escaMaxRetries":5}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads with escaMaxRetries");
    assert_eq!(
        cfg.esca_max_retries, 5,
        "escaMaxRetries parses into Config.esca_max_retries (camelCase on disk, u32)"
    );
}

#[test]
fn config_defaults_all_6_new_fields_when_absent() {
    // No config.json at all -> fully-defaulted Config.
    let dir = tempfile::tempdir().unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config defaults when file missing");
    assert_eq!(
        cfg.dump_adapter, None,
        "dump_adapter defaults to None when absent"
    );
    assert_eq!(
        cfg.smart_adapter, None,
        "smart_adapter defaults to None when absent"
    );
    assert_eq!(
        cfg.confirm_count, 1,
        "confirm_count defaults to 1 when absent (LD19)"
    );
    assert_eq!(
        cfg.esca_threshold, 2,
        "esca_threshold defaults to 2 when absent (LD19)"
    );
    assert_eq!(
        cfg.esca_max_retries, 3,
        "esca_max_retries defaults to 3 when absent (LD21)"
    );
}

// ---------------------------------------------------------------------------
// T1.2 — Validation (LD28, fail-closed)
// ---------------------------------------------------------------------------

#[test]
fn config_rejects_n_zero_vacuous_pass() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":0,"m":2}"#,
    )
    .unwrap();
    let err = store::load_config_in(dir.path())
        .expect_err("n=0 creates vacuous-pass Gate; MUST be rejected at parse time (LD28)");
    let msg = err.to_string();
    assert!(
        msg.contains("n") || msg.contains("threshold"),
        "error must mention the offending field, got: {msg}"
    );
}

#[test]
fn config_rejects_n_greater_than_m_impossible() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":3,"m":2}"#,
    )
    .unwrap();
    let err = store::load_config_in(dir.path())
        .expect_err("n>m is impossible (more approvals needed than verifiers); MUST be rejected (LD28)");
    let msg = err.to_string();
    assert!(
        msg.contains("n") || msg.contains("m") || msg.contains("threshold"),
        "error must mention the offending field, got: {msg}"
    );
}

#[test]
fn config_rejects_m_zero_empty_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":1,"m":0}"#,
    )
    .unwrap();
    let err = store::load_config_in(dir.path())
        .expect_err("m=0 produces empty pipeline; MUST be rejected at parse time (LD28)");
    let msg = err.to_string();
    assert!(
        msg.contains("m") || msg.contains("verifier"),
        "error must mention the offending field, got: {msg}"
    );
}

#[test]
fn config_rejects_confirm_count_zero() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":2,"m":2,"confirmCount":0}"#,
    )
    .unwrap();
    let err = store::load_config_in(dir.path())
        .expect_err("confirmCount=0 is degenerate; MUST be rejected at parse time (LD28)");
    let msg = err.to_string();
    assert!(
        msg.contains("confirmCount") || msg.contains("confirm"),
        "error must mention the offending field, got: {msg}"
    );
}

#[test]
fn config_accepts_esca_threshold_zero_disabled() {
    // escaThreshold=0 means escalation disabled (LD21). Valid config.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":2,"m":2,"escaThreshold":0}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path())
        .expect("escaThreshold=0 is valid (escalation disabled per LD21)");
    assert_eq!(cfg.esca_threshold, 0);
}

// ---------------------------------------------------------------------------
// T1.3 — Precedence rule (LD19)
// ---------------------------------------------------------------------------

#[test]
fn config_rejects_both_backend_and_dump_adapter_ambiguous() {
    // LD19: "Reject config with both backend AND dumpAdapter present without verifiers[]"
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"backend":"pi","dumpAdapter":"hermes"}"#,
    )
    .unwrap();
    let err = store::load_config_in(dir.path())
        .expect_err("both backend AND dumpAdapter set is ambiguous; MUST be rejected (LD19)");
    let msg = err.to_string();
    assert!(
        msg.contains("backend") || msg.contains("dumpAdapter") || msg.contains("ambiguous"),
        "error must mention the conflict, got: {msg}"
    );
}

#[test]
fn config_backend_alias_for_dump_adapter_when_dump_adapter_unset() {
    // LD19 precedence #3: backend is alias for dumpAdapter when dumpAdapter is unset.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"backend":"hermes"}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads with backend only");
    // backend is the legacy field; dump_adapter is None; but the precedence resolver
    // should treat backend as dumpAdapter when dumpAdapter is unset.
    // This test checks the resolver function, not the raw field.
    let resolved = cfg.resolve_dump_adapter();
    assert_eq!(
        resolved, "hermes",
        "backend is alias for dumpAdapter when dumpAdapter is unset (LD19 precedence #3)"
    );
}

#[test]
fn config_dump_adapter_wins_over_backend_when_both_unset() {
    // LD19 precedence #2: dumpAdapter wins over backend.
    // But we already reject both set (T1.3 above), so this tests the resolver
    // when dumpAdapter is set and backend is default.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"dumpAdapter":"acpx"}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads with dumpAdapter");
    let resolved = cfg.resolve_dump_adapter();
    assert_eq!(
        resolved, "acpx",
        "dumpAdapter wins when set (LD19 precedence #2)"
    );
}

#[test]
fn config_smart_adapter_defaults_to_backend_when_unset() {
    // LD19: smartAdapter defaults to backend when unset.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"backend":"hermes"}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads with backend");
    let resolved = cfg.resolve_smart_adapter();
    assert_eq!(
        resolved, "hermes",
        "smartAdapter defaults to backend when unset (LD19)"
    );
}

// ---------------------------------------------------------------------------
// T1.4 — Snapshot at NEW (LD23)
// ---------------------------------------------------------------------------

#[test]
fn config_snapshot_includes_all_6_new_fields() {
    // LD23: all 6 new fields frozen into goal.json at NEW.
    // This test checks that Config has a snapshot() method that includes the new fields.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":2,"m":2,"dumpAdapter":"hermes","smartAdapter":"acpx","confirmCount":2,"escaThreshold":3,"escaMaxRetries":5}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads");
    let snapshot = cfg.snapshot();
    // snapshot is a GoalSnapshot struct; check it has the new fields.
    assert_eq!(snapshot.dump_adapter, Some("hermes".to_string()));
    assert_eq!(snapshot.smart_adapter, Some("acpx".to_string()));
    assert_eq!(snapshot.confirm_count, 2);
    assert_eq!(snapshot.esca_threshold, 3);
    assert_eq!(snapshot.esca_max_retries, 5);
}

// ---------------------------------------------------------------------------
// T1.5 — m<2 warning (LD15)
// ---------------------------------------------------------------------------

#[test]
fn config_warns_when_m_less_than_2_and_esca_threshold_positive() {
    // LD15: m<2 ∧ escaThreshold>0 → stderr warning, escaThreshold ignored.
    // This test checks that the warning is emitted (we can't easily capture stderr
    // in a unit test, so we check the validation function returns a warning).
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":1,"m":1,"escaThreshold":2}"#,
    )
    .unwrap();
    let cfg = store::load_config_in(dir.path()).expect("config loads (not a hard error)");
    let warnings = cfg.validate_and_warn();
    assert!(
        warnings.iter().any(|w| w.contains("escaThreshold") && w.contains("m < 2")),
        "must warn about escaThreshold ignored when m<2 (LD15), got: {:?}",
        warnings
    );
}

// ---------------------------------------------------------------------------
// T1.6 — camelCase round-trip
// ---------------------------------------------------------------------------

#[test]
fn config_camel_case_round_trips_all_6_new_fields() {
    // On-disk shape uses camelCase; the struct uses snake_case via #[serde(rename)].
    let cfg = store::Config {
        n: 2,
        m: 2,
        max_turn: 3,
        backend: "pi".into(),
        git_diff_max_chars: 10_000,
        verifier_timeout_sec: 1800,
        verifier_prompt_file: None,
        min_goal_chars: 0,
        file_edit_times_max_chars: 8_000,
        context_max_chars: 20_000,
        prompt_budget_bytes: 50_000,
        dump_adapter: Some("hermes".into()),
        smart_adapter: Some("acpx".into()),
        confirm_count: 2,
        esca_threshold: 3,
        esca_max_retries: 5,
    };
    let j = serde_json::to_string(&cfg).unwrap();
    // camelCase keys must appear verbatim on disk.
    assert!(
        j.contains("\"dumpAdapter\":\"hermes\""),
        "dumpAdapter must serialize camelCase: {j}"
    );
    assert!(
        j.contains("\"smartAdapter\":\"acpx\""),
        "smartAdapter must serialize camelCase: {j}"
    );
    assert!(
        j.contains("\"confirmCount\":2"),
        "confirmCount must serialize camelCase: {j}"
    );
    assert!(
        j.contains("\"escaThreshold\":3"),
        "escaThreshold must serialize camelCase: {j}"
    );
    assert!(
        j.contains("\"escaMaxRetries\":5"),
        "escaMaxRetries must serialize camelCase: {j}"
    );

    let back: store::Config = serde_json::from_str(&j).unwrap();
    assert_eq!(back, cfg, "round-trip preserves all 6 new fields");
}
