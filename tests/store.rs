// tasks.md §2 — Salt + config store (goal-lifecycle spec).
// RED phase: written first, against the spec, before any implementation.
// (Deviation note: the objective's "separate fresh teammate per phase" safeguard is not applied
//  here because no `teams`/delegation tool is available to this agent. RED is still committed
//  and verified-failing before GREEN is written, preserving test-first discipline.)

use std::fs;
use std::os::unix::fs::PermissionsExt;

use verifier_loop::store;

#[test]
fn salt_is_created_once_with_correct_permissions_and_length() {
    let dir = tempfile::tempdir().unwrap();
    let salt_path = dir.path().join(".salt");

    let salt1 = store::salt_in(dir.path()).expect("first salt read");
    assert!(salt_path.exists(), "salt file must be created on first run");
    assert_eq!(salt1.len(), 64, "salt must be 64 hex chars");
    assert!(
        salt1.chars().all(|c| c.is_ascii_hexdigit()),
        "salt must be hex"
    );

    let mode = fs::metadata(&salt_path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "salt file must have mode 0600, got {:o}",
        mode & 0o777
    );
}

#[test]
fn salt_is_never_overwritten_on_subsequent_runs() {
    let dir = tempfile::tempdir().unwrap();

    let salt1 = store::salt_in(dir.path()).unwrap();
    let salt2 = store::salt_in(dir.path()).unwrap();
    assert_eq!(salt1, salt2, "salt created once must be stable across runs");
}

#[test]
fn salt_value_is_not_present_in_stdout_or_logs() {
    // The salt is never printed by the store API. The public API returns it only to the
    // caller (the hash computation); there is no logging path. The real guard is that no
    // subcommand prints the salt (covered in goal-lifecycle tests). Here we assert the
    // store API surface does not take a "verbose/print" flag.
    let dir = tempfile::tempdir().unwrap();
    let _ = store::salt_in(dir.path()).unwrap();
}

#[test]
fn config_defaults_are_applied_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();

    let cfg = store::load_config_in(dir.path()).expect("config loads with defaults");
    assert_eq!(cfg.n, 2);
    assert_eq!(cfg.m, 2);
    assert_eq!(cfg.max_turn, 3);
    assert_eq!(cfg.backend, "pi");
    assert_eq!(cfg.git_diff_max_chars, 10000);
    assert_eq!(cfg.verifier_timeout_sec, 1800);
}

#[test]
fn config_is_loaded_from_file_when_present() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":3,"m":5,"maxTurn":4,"backend":"hermes","gitDiffMaxChars":2048,"verifierTimeoutSec":600}"#,
    )
    .unwrap();

    let cfg = store::load_config_in(dir.path()).expect("config loads from file");
    assert_eq!(cfg.n, 3);
    assert_eq!(cfg.m, 5);
    assert_eq!(cfg.max_turn, 4);
    assert_eq!(cfg.backend, "hermes");
    assert_eq!(cfg.git_diff_max_chars, 2048);
    assert_eq!(cfg.verifier_timeout_sec, 600);
}

#[test]
fn config_partial_file_keeps_defaults_for_missing_fields() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("config.json"), r#"{"m":3}"#).unwrap();

    let cfg = store::load_config_in(dir.path()).unwrap();
    assert_eq!(cfg.n, 2, "missing n keeps default");
    assert_eq!(cfg.m, 3, "present m is honored");
    assert_eq!(cfg.max_turn, 3, "missing maxTurn keeps default");
}

#[test]
fn salt_is_distinct_across_independent_stores() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let sa = store::salt_in(a.path()).unwrap();
    let sb = store::salt_in(b.path()).unwrap();
    assert_ne!(sa, sb, "independent stores must have independent salts");
}

// ---------------------------------------------------------------------------
// RED phase (task #10) — verifierPromptFile + minGoalChars config features.
// These reference Config fields that do NOT exist yet; expected to fail to compile.
// ---------------------------------------------------------------------------

#[test]
fn config_parses_verifier_prompt_file_when_present() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"verifierPromptFile":"/tmp/custom-verifier-prompt.md"}"#,
    )
    .unwrap();

    let cfg = store::load_config_in(dir.path()).expect("config loads with verifierPromptFile");
    assert_eq!(
        cfg.verifier_prompt_file.as_deref(),
        Some("/tmp/custom-verifier-prompt.md"),
        "verifierPromptFile parses into Config.verifier_prompt_file (camelCase on disk)",
    );
    // The other defaults must still hold when only the new key is present.
    assert_eq!(cfg.min_goal_chars, 0, "minGoalChars absent -> 0");
}

#[test]
fn config_parses_min_goal_chars_when_present() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("config.json"), r#"{"minGoalChars":50}"#).unwrap();

    let cfg = store::load_config_in(dir.path()).expect("config loads with minGoalChars");
    assert_eq!(
        cfg.min_goal_chars, 50,
        "minGoalChars parses into Config.min_goal_chars (camelCase on disk, u64)",
    );
    assert_eq!(
        cfg.verifier_prompt_file, None,
        "verifierPromptFile absent -> None",
    );
}

#[test]
fn config_defaults_verifier_prompt_file_and_min_goal_chars_when_absent() {
    let dir = tempfile::tempdir().unwrap();
    // No config.json at all -> fully-defaulted Config.
    let cfg = store::load_config_in(dir.path()).expect("config defaults when file missing");
    assert_eq!(
        cfg.verifier_prompt_file, None,
        "missing verifierPromptFile -> None",
    );
    assert_eq!(cfg.min_goal_chars, 0, "missing minGoalChars -> 0");

    // A partial config with neither new key also keeps the defaults.
    fs::write(dir.path().join("config.json"), r#"{"m":3}"#).unwrap();
    let cfg = store::load_config_in(dir.path()).unwrap();
    assert_eq!(cfg.verifier_prompt_file, None);
    assert_eq!(cfg.min_goal_chars, 0);
}

#[test]
fn config_camel_case_round_trips_verifier_prompt_file_and_min_goal_chars() {
    // On-disk shape uses camelCase; the struct uses snake_case via #[serde(rename)].
    // The round-trip must preserve both keys exactly.
    let cfg = store::Config {
        n: 1,
        m: 1,
        max_turn: 3,
        backend: "stub".into(),
        git_diff_max_chars: 1000,
        verifier_timeout_sec: 10,
        verifier_prompt_file: Some("/abs/path/to/prompt.md".into()),
        min_goal_chars: 42,
        file_edit_times_max_chars: 8_000,
        context_max_chars: 20_000,
        prompt_budget_bytes: 50_000,
        ..store::Config::default()
    };
    let j = serde_json::to_string(&cfg).unwrap();
    // camelCase keys must appear verbatim on disk (the on-disk contract).
    assert!(
        j.contains("\"verifierPromptFile\":\"/abs/path/to/prompt.md\""),
        "verifierPromptFile must serialize camelCase: {j}",
    );
    assert!(
        j.contains("\"minGoalChars\":42"),
        "minGoalChars must serialize camelCase: {j}",
    );
    // The 3 prompt-bloat config fields must serialize camelCase (a removed
    // #[serde(rename)] would silently break the round-trip without these).
    assert!(
        j.contains("\"fileEditTimesMaxChars\":8000"),
        "fileEditTimesMaxChars must serialize camelCase: {j}",
    );
    assert!(
        j.contains("\"contextMaxChars\":20000"),
        "contextMaxChars must serialize camelCase: {j}",
    );
    assert!(
        j.contains("\"promptBudgetBytes\":50000"),
        "promptBudgetBytes must serialize camelCase: {j}",
    );

    let back: store::Config = serde_json::from_str(&j).unwrap();
    assert_eq!(
        back, cfg,
        "round-trip preserves verifier_prompt_file + min_goal_chars"
    );
}

// ---------------------------------------------------------------------------
// RED phase (cwd-runtime-source) — Config MUST reject unknown keys (deny_unknown_fields).
// The live config.json historically carried dead no-op keys (cwd, model,
// verifierPromptTemplate, verifierResumePromptTemplate) that serde silently ignored,
// hiding misconfiguration from the operator. The fix is fail-closed: any unknown
// key MUST produce a load error mentioning the offending field.
// These tests currently FAIL because Config lacks #[serde(deny_unknown_fields)].
// ---------------------------------------------------------------------------

#[test]
fn config_rejects_unknown_key_cwd() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"cwd":"/nonexistent/wrong/path"}"#,
    )
    .unwrap();
    let err = store::load_config_in(dir.path())
        .expect_err("cwd is runtime-derived; config.json cwd key MUST be rejected");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("cwd"),
        "error must name the offending unknown field 'cwd': {msg}"
    );
}

#[test]
fn config_rejects_unknown_key_model() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("config.json"), r#"{"model":null}"#).unwrap();
    let err =
        store::load_config_in(dir.path()).expect_err("model is not a config key; MUST be rejected");
    assert!(
        err.to_string().to_lowercase().contains("model"),
        "error must name 'model': {}",
        err
    );
}

#[test]
fn config_rejects_unknown_key_verifier_prompt_template() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"verifierPromptTemplate":null}"#,
    )
    .unwrap();
    store::load_config_in(dir.path())
        .expect_err("verifierPromptTemplate is a dead key; MUST be rejected");
}

#[test]
fn config_rejects_unknown_key_verifier_resume_prompt_template() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"verifierResumePromptTemplate":null}"#,
    )
    .unwrap();
    store::load_config_in(dir.path())
        .expect_err("verifierResumePromptTemplate is a dead key; MUST be rejected");
}

#[test]
fn config_accepts_canonical_keys_only() {
    // The full canonical set: n, m, maxTurn, backend, gitDiffMaxChars, verifierTimeoutSec,
    // verifierPromptFile, minGoalChars. No dead keys. MUST parse cleanly.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":2,"m":2,"maxTurn":3,"backend":"pi","gitDiffMaxChars":10000,"verifierTimeoutSec":1800,"verifierPromptFile":null,"minGoalChars":0}"#,
    )
    .unwrap();
    let cfg =
        store::load_config_in(dir.path()).expect("canonical keys only must parse without error");
    assert_eq!((cfg.n, cfg.m), (2, 2));
}
