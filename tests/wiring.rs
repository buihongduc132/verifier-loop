// tasks.md §10 — CLI wiring (clap) for both binaries.
// RED phase: written first, against tasks.md §10 + the CLI subcommand contract, BEFORE
// the `verifier-loop` bin wiring exists. The scaffold bin (src/bin/verifier_loop.rs)
// currently ignores all args and prints an identity line, so every assertion here is
// expected to FAIL until §10 GREEN lands.
//
// Scope of THIS test (wiring only — no spawn, no I/O):
//   * `verifier-loop --help` exits 0 and advertises NEW / RESUME.
//   * `verifier-verdict --help` exits 0 and advertises approve / reject (already wired in §7;
//     asserted here to guard against regressions during §10).
//   * Missing/invalid subcommands and missing required args exit non-zero with a usage hint.
//
// Strategy: assert_cmd against the cargo-built binaries. Fast, hermetic, no temp stores.

use assert_cmd::Command;
use predicates::prelude::*;

/// `verifier-loop --help` lists both subcommands and exits 0.
#[test]
fn verifier_loop_help_lists_subcommands() {
    let mut cmd = Command::cargo_bin("verifier-loop").unwrap();
    cmd.args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("NEW").or(predicate::str::contains("new")))
        .stdout(predicate::str::contains("RESUME").or(predicate::str::contains("resume")));
}

/// `verifier-verdict --help` lists approve / reject and exits 0 (§7 regression guard).
#[test]
fn verifier_verdict_help_lists_subcommands() {
    let mut cmd = Command::cargo_bin("verifier-verdict").unwrap();
    cmd.args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("approve"))
        .stdout(predicate::str::contains("reject"));
}

/// No subcommand at all → non-zero exit + a usage message on stderr.
#[test]
fn no_subcommand_exits_non_zero_with_usage() {
    let mut cmd = Command::cargo_bin("verifier-loop").unwrap();
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("usage").or(predicate::str::contains("Usage")));
}

/// `NEW` with no goal argument → non-zero exit + usage.
#[test]
fn new_without_goal_arg_exits_non_zero() {
    let mut cmd = Command::cargo_bin("verifier-loop").unwrap();
    cmd.args(["NEW"]).assert().failure();
}

/// `RESUME` with no goalId → non-zero exit + usage.
#[test]
fn resume_without_goal_id_exits_non_zero() {
    let mut cmd = Command::cargo_bin("verifier-loop").unwrap();
    cmd.args(["RESUME"]).assert().failure();
}

// ===========================================================================
// §2 RED tests: verifier-verdict approve --notes (CLI wiring)
// fix-approve-notes-and-prompt-merge verdict-registration spec delta + design D1.
//
// The current `Cmd::Approve` is a unit variant that accepts NO arguments, so clap
// rejects `approve --notes "..."` and `approve -n "..."` with `error: unexpected
// argument`. GREEN turns Approve into `Approve { notes: Option<String> }` with the
// `-n` short alias; until then every test below FAILS — that IS RED.
//
// Hermetic: each test scrubs inherited VERIFIER_LOOP_* env (env_clear) before
// re-setting only the vars it needs, so a developer's shell cannot leak a real
// store root / goalId into the assertion.
// ===========================================================================

use std::fs;
use std::path::Path;

use serde_json::Value;
use verifier_loop::goal;
use verifier_loop::verdict;

const APPROVE: &str = "APPROVE";

/// Build a fresh temp store + goal with a pre-created round-1 v1 null verdict slot,
/// mirroring what the spawn layer writes at spawn time. Returns (TempDir, goal_id) so
/// the calling test can point VERIFIER_LOOP_HOME / VERIFIER_LOOP_GOAL_ID at it.
fn fresh_goal_with_null_v1(round: u32) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), "build it", None).unwrap();
    let vdir = verdict::verdict_path(dir.path(), &goal_id, "v1", round);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(vdir.join(verdict::VERDICT_FILE), r#"{"status":null}"#).unwrap();
    (dir, goal_id)
}

/// Read the raw on-disk verdict JSON (preserves the exact key set so the absence of
/// `notes` can be asserted, which `read_verdict`'s `Option<String>` would erase).
fn raw_verdict_json(root: &Path, goal_id: &str, vid: &str, round: u32) -> Value {
    let path = verdict::verdict_path(root, goal_id, vid, round).join(verdict::VERDICT_FILE);
    let raw = fs::read_to_string(&path).unwrap();
    serde_json::from_str(&raw).unwrap()
}

// ---------------------------------------------------------------------------
// §2.1 RED: `verifier-verdict approve --notes "foo"` parses, runs, and stores notes.
// ---------------------------------------------------------------------------

#[test]
fn cli_approve_with_notes_long_flag_stores_notes_and_exits_zero() {
    let (dir, goal_id) = fresh_goal_with_null_v1(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env_clear()
        .env("LLVM_PROFILE_FILE", std::env::var("LLVM_PROFILE_FILE").unwrap_or_default())
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        // NOTE: no VERIFIER_LOOP_VERIFIER_SECRET -> unsigned regime (no pinned pubkey).
        .args(["approve", "--notes", "foo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Verdict registered"));

    // The on-disk verdict MUST be APPROVE with notes "foo".
    let raw = raw_verdict_json(dir.path(), &goal_id, "v1", 1);
    assert_eq!(raw["status"], Value::String(APPROVE.into()));
    assert_eq!(
        raw["notes"].as_str(),
        Some("foo"),
        "--notes value must be stored verbatim on the verdict: {raw}"
    );
}

// ---------------------------------------------------------------------------
// §2.2 RED: `verifier-verdict approve -n "bar"` (short alias) parses identically.
// ---------------------------------------------------------------------------

#[test]
fn cli_approve_with_notes_short_alias_stores_notes() {
    let (dir, goal_id) = fresh_goal_with_null_v1(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env_clear()
        .env("LLVM_PROFILE_FILE", std::env::var("LLVM_PROFILE_FILE").unwrap_or_default())
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .args(["approve", "-n", "bar"])
        .assert()
        .success();

    let raw = raw_verdict_json(dir.path(), &goal_id, "v1", 1);
    assert_eq!(raw["status"], Value::String(APPROVE.into()));
    assert_eq!(
        raw["notes"].as_str(),
        Some("bar"),
        "-n short alias must store notes identically to --notes: {raw}"
    );
}

// ---------------------------------------------------------------------------
// §2.3 RED: `verifier-verdict approve` (no notes) still works and writes no notes key.
// (Regression guard: optional --notes must not break the existing bare-approve path.)
// ---------------------------------------------------------------------------

#[test]
fn cli_approve_without_notes_still_works_and_omits_notes_key() {
    let (dir, goal_id) = fresh_goal_with_null_v1(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env_clear()
        .env("LLVM_PROFILE_FILE", std::env::var("LLVM_PROFILE_FILE").unwrap_or_default())
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .arg("approve")
        .assert()
        .success()
        .stdout(predicate::str::contains("Verdict registered"));

    let raw = raw_verdict_json(dir.path(), &goal_id, "v1", 1);
    assert_eq!(raw["status"], Value::String(APPROVE.into()));
    assert!(
        raw.get("notes").is_none(),
        "bare `approve` must omit the `notes` key entirely (regression): {raw}"
    );
}

// ---------------------------------------------------------------------------
// §2.4 RED: `approve --notes ""` (whitespace) normalizes to no notes key.
// (design D2 / spec scenario "Approve with empty notes normalizes to no notes".)
// ---------------------------------------------------------------------------

#[test]
fn cli_approve_with_whitespace_notes_normalizes_to_no_notes_key() {
    let (dir, goal_id) = fresh_goal_with_null_v1(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env_clear()
        .env("LLVM_PROFILE_FILE", std::env::var("LLVM_PROFILE_FILE").unwrap_or_default())
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .args(["approve", "--notes", "   "])
        .assert()
        .success();

    let raw = raw_verdict_json(dir.path(), &goal_id, "v1", 1);
    assert_eq!(raw["status"], Value::String(APPROVE.into()));
    assert!(
        raw.get("notes").is_none(),
        "whitespace-only --notes must normalize to no notes key: {raw}"
    );
}

// ---------------------------------------------------------------------------
// §2.5 RED: clap no longer emits "unexpected argument --notes" for approve.
// (Hermetic parse-only smoke: even without env, the failure mode must NOT be the
// clap parse error that RED starts with.)
// ---------------------------------------------------------------------------

#[test]
fn cli_approve_notes_long_flag_is_accepted_by_clap() {
    let assert = Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env_clear()
        .env("LLVM_PROFILE_FILE", std::env::var("LLVM_PROFILE_FILE").unwrap_or_default())
        // No identity env on purpose — we only care that clap ACCEPTS the flag.
        .args(["approve", "--notes", "anything"])
        .assert()
        .failure();

    // RED today: stderr says `error: unexpected argument '--notes' found`.
    // GREEN:    clap accepts the flag; the binary fails later on missing env, so the
    //           stderr must NOT contain the "unexpected argument" clap error.
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("--notes"),
        "after GREEN, clap must accept `--notes` on approve; stderr was: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// §2.6 RED regression guard: reject without notes is STILL refused (unchanged).
// (Confirms the approve change did not accidentally relax reject's notes gate.)
// ---------------------------------------------------------------------------

#[test]
fn cli_reject_without_notes_is_still_refused() {
    let (dir, goal_id) = fresh_goal_with_null_v1(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env_clear()
        .env("LLVM_PROFILE_FILE", std::env::var("LLVM_PROFILE_FILE").unwrap_or_default())
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .arg("reject")
        .assert()
        .failure();

    // Slot stays null — reject without notes must never write.
    let raw = raw_verdict_json(dir.path(), &goal_id, "v1", 1);
    assert_eq!(raw["status"], Value::Null, "reject-without-notes must not write: {raw}");
}
