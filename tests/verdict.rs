// tasks.md §7 — Verifier-verdict CLI (verdict-registration spec).
// RED phase: written first, against the spec, before any implementation.
//
// Covers the verdict-registration spec scenarios:
//   * approve writes a verdict (status APPROVE + registeredAt, prints "Verdict registered", exit 0)
//   * reject requires notes (reject --notes writes REJECT + notes; reject w/o notes refused)
//   * first verdict is final (2nd attempt rejected, stored unchanged)
//   * verdict pre-created as null (forgotten verdict stays null -> round fails)
//   * env-derived slot (VERIFIER_LOOP_* env wins over args)
//
// Identity resolution: goalId / verifierId / round come from VERIFIER_LOOP_GOAL_ID /
// VERIFIER_LOOP_VERIFIER_ID / VERIFIER_LOOP_ROUND. The store root comes from
// VERIFIER_LOOP_HOME (or defaults to ~/.verifier-loop).

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

use verifier_loop::goal;
use verifier_loop::verdict;

const APPROVE: &str = "APPROVE";
const REJECT: &str = "REJECT";

/// Helper: create a goal under a fresh temp store root and pre-create the round-1 v1
/// verifier dir (mirroring what the spawn layer does at spawn time), returning the goalId.
fn fresh_goal_with_null_verdict(round: u32) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), "build it", None).unwrap();

    // Simulate the spawn layer: pre-create rounds/<round>/v1/verdict.json {status:null}.
    let vdir = verdict::verdict_path(dir.path(), &goal_id, "v1", round);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(
        vdir.join(verdict::VERDICT_FILE),
        r#"{"status":null}"#,
    )
    .unwrap();
    (dir, goal_id)
}

fn read_status(root: &Path, goal_id: &str, vid: &str, round: u32) -> Value {
    let rec = verdict::read_verdict(root, goal_id, vid, round).unwrap();
    let v: Value = serde_json::from_str(&serde_json::to_string(&rec.status).unwrap()).unwrap();
    v
}

// ---------------------------------------------------------------------------
// Scenario: Approve writes a verdict
// ---------------------------------------------------------------------------

#[test]
fn approve_writes_verdict_with_status_and_registered_at() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    verdict::register_approve(dir.path(), &goal_id, "v1", 1).unwrap();

    let rec = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    assert_eq!(
        read_status(dir.path(), &goal_id, "v1", 1),
        Value::String(APPROVE.into())
    );
    // registeredAt must be present and non-empty.
    let ts = rec.registered_at.as_deref().expect("registeredAt must be populated");
    assert!(!ts.is_empty(), "registeredAt must be non-empty");
}

#[test]
fn cli_approve_prints_verdict_registered_and_exits_zero() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .arg("approve")
        .assert()
        .success()
        .stdout(predicates::str::contains("Verdict registered"));
}

// ---------------------------------------------------------------------------
// Scenario: Reject requires notes
// ---------------------------------------------------------------------------

#[test]
fn reject_with_notes_writes_verdict_with_notes() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    verdict::register_reject(dir.path(), &goal_id, "v1", 1, "issue 1: missing test").unwrap();

    let rec = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    assert_eq!(
        read_status(dir.path(), &goal_id, "v1", 1),
        Value::String(REJECT.into())
    );
    assert_eq!(rec.notes.as_deref(), Some("issue 1: missing test"));
}

#[test]
fn register_reject_without_notes_is_refused_and_writes_nothing() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    let err = verdict::register_reject(dir.path(), &goal_id, "v1", 1, "").unwrap_err();
    assert!(
        matches!(err, verdict::VerdictError::NotesRequired),
        "empty notes must yield NotesRequired, got {err:?}"
    );

    // Verdict file stays null.
    assert_eq!(
        read_status(dir.path(), &goal_id, "v1", 1),
        Value::Null,
        "no write on refused reject"
    );
}

#[test]
fn cli_reject_without_notes_exits_non_zero_and_writes_nothing() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .args(["reject"])
        .assert()
        .failure();

    assert_eq!(
        read_status(dir.path(), &goal_id, "v1", 1),
        Value::Null,
        "no write when --notes missing"
    );
}

#[test]
fn cli_reject_with_notes_prints_verdict_registered_and_exits_zero() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .args(["reject", "--notes", "issue 1: missing test"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Verdict registered"));

    assert_eq!(
        read_status(dir.path(), &goal_id, "v1", 1),
        Value::String(REJECT.into())
    );
}

// ---------------------------------------------------------------------------
// Scenario: First verdict is final
// ---------------------------------------------------------------------------

#[test]
fn second_verdict_attempt_is_rejected_and_stored_unchanged() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    verdict::register_approve(dir.path(), &goal_id, "v1", 1).unwrap();
    let err = verdict::register_reject(dir.path(), &goal_id, "v1", 1, "too late").unwrap_err();
    assert!(
        matches!(err, verdict::VerdictError::AlreadyFinal),
        "second verdict must be AlreadyFinal, got {err:?}"
    );

    // Stored verdict must remain APPROVE.
    assert_eq!(
        read_status(dir.path(), &goal_id, "v1", 1),
        Value::String(APPROVE.into()),
        "first verdict must be final and unchanged"
    );
}

#[test]
fn cli_second_attempt_exits_non_zero_without_altering_stored_verdict() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    // First verdict via CLI.
    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .arg("approve")
        .assert()
        .success();

    // Second attempt must fail.
    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .args(["reject", "--notes", "nope"])
        .assert()
        .failure();

    assert_eq!(
        read_status(dir.path(), &goal_id, "v1", 1),
        Value::String(APPROVE.into())
    );
}

// ---------------------------------------------------------------------------
// Scenario: Verdict file is pre-created as null (forgotten -> round fails)
// ---------------------------------------------------------------------------

#[test]
fn forgotten_verdict_stays_null_and_round_fails() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    // A verifier that never calls verifier-verdict leaves status:null.
    let rec = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    assert_eq!(
        serde_json::to_value(&rec.status).unwrap(),
        Value::Null,
        "null must never be silently promoted; round is evaluated as not passing"
    );
    assert!(
        !matches!(
            rec.status,
            verdict::VerdictStatus::Approve | verdict::VerdictStatus::Reject
        ),
        "null stays null"
    );
    assert!(matches!(rec.status, verdict::VerdictStatus::Null));
}

// ---------------------------------------------------------------------------
// Scenario: Verifier identity is read from env, not arguments
// ---------------------------------------------------------------------------

#[test]
fn verdict_writes_to_env_derived_slot_regardless_of_args() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    // Env-derived identity (abc / v1 / round 1) — even though no conflicting arg is
    // accepted, the env vars alone must be sufficient to locate the slot.
    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .arg("approve")
        .assert()
        .success();

    // Written to the env-derived slot (goals/<goal_id>/rounds/1/v1/verdict.json).
    let vpath = verdict::verdict_path(dir.path(), &goal_id, "v1", 1);
    let raw: Value = serde_json::from_str(&fs::read_to_string(vpath.join(verdict::VERDICT_FILE)).unwrap()).unwrap();
    assert_eq!(raw["status"], Value::String(APPROVE.into()));
}

#[test]
fn cli_missing_identity_env_exits_non_zero() {
    let (dir, _goal_id) = fresh_goal_with_null_verdict(1);

    // No VERIFIER_LOOP_* identity env -> must fail closed.
    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env_clear()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .arg("approve")
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// CLI error-path coverage (tasks.md §7): NotesRequired / GoalNotFound /
// missing-home. These exercise the bin/verifier_verdict.rs error arms that the
// happy-path CLI tests above leave uncovered.
// ---------------------------------------------------------------------------

/// `reject --notes ""` (empty string, non-null) reaches `register_reject` and is
/// refused with NotesRequired — distinct from omitting `--notes` (which clap rejects
/// before `run()`). Covers the bin's NotesRequired error arm.
#[test]
fn cli_reject_with_empty_notes_string_is_refused() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .args(["reject", "--notes", ""])
        .assert()
        .failure()
        .stderr(predicates::str::contains("reject requires non-empty --notes"));

    // Stored verdict must remain null (no write on refused reject).
    assert_eq!(
        read_status(dir.path(), &goal_id, "v1", 1),
        Value::Null,
        "empty-string notes must not write a verdict"
    );
}

/// An approve against a goal id that does not exist in the store must fail closed with
/// the bin's GoalNotFound error arm.
#[test]
fn cli_approve_for_unknown_goal_id_returns_goal_not_found() {
    let (dir, _goal_id) = fresh_goal_with_null_verdict(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", "goal-does-not-exist")
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .arg("approve")
        .assert()
        .failure()
        .stderr(predicates::str::contains("goal not found"));
}

/// With neither VERIFIER_LOOP_HOME nor HOME set, `resolve_home` must fail closed rather
/// than silently falling back to a non-existent default. Covers the bin's $HOME-unset
/// error arm and the dirs_home() None branch.
#[test]
fn cli_with_home_unset_and_no_home_env_fails_closed() {
    // Remove VERIFIER_LOOP_HOME and HOME individually (not env_clear) so the
    // llvm-cov profiling env (LLVM_PROFILE_FILE) is preserved and the spawned
    // binary's coverage is still merged into the report.
    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env_remove("VERIFIER_LOOP_HOME")
        .env_remove("HOME")
        .env("VERIFIER_LOOP_GOAL_ID", "any-goal")
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .arg("approve")
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "VERIFIER_LOOP_HOME is unset and $HOME is not available",
        ));
}

/// With VERIFIER_LOOP_HOME unset but HOME set, the store root falls back to
/// `$HOME/.verifier-loop`. Covers the bin's `Some(h)` HOME-fallback branch in
/// `resolve_home` (and the `dirs_home()` body).
#[test]
fn cli_with_home_unset_falls_back_to_dot_verifier_loop() {
    let home = tempfile::tempdir().unwrap();
    // Plant a goal directly under the $HOME/.verifier-loop default root so the
    // fallback path is actually resolvable end-to-end.
    let default_root = home.path().join(".verifier-loop");
    fs::create_dir_all(&default_root).unwrap();
    let goal_id = goal::new(&default_root, "build it", None).unwrap();
    let vdir = verdict::verdict_path(&default_root, &goal_id, "v1", 1);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(vdir.join(verdict::VERDICT_FILE), r#"{"status":null}"#).unwrap();

    // VERIFIER_LOOP_HOME deliberately unset; only HOME is provided. env_remove
    // (not env_clear) preserves the llvm-cov profiling env for the subprocess.
    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env_remove("VERIFIER_LOOP_HOME")
        .env("HOME", home.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .arg("approve")
        .assert()
        .success()
        .stdout(predicates::str::contains("Verdict registered"));

    // Written via the $HOME/.verifier-loop fallback root.
    assert_eq!(
        read_status(&default_root, &goal_id, "v1", 1),
        Value::String(APPROVE.into()),
    );
}

// ---------------------------------------------------------------------------
// Atomic first-write-wins (direct API)
// ---------------------------------------------------------------------------

#[test]
fn first_write_wins_is_atomic_across_two_approves() {
    let (dir, goal_id) = fresh_goal_with_null_verdict(1);

    verdict::register_approve(dir.path(), &goal_id, "v1", 1).unwrap();
    let err = verdict::register_approve(dir.path(), &goal_id, "v1", 1).unwrap_err();
    assert!(matches!(err, verdict::VerdictError::AlreadyFinal));
}
