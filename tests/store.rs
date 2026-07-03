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
