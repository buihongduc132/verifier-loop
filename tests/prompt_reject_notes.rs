// Dynamic verifier prompt from previous REJECT notes (intention 2026-07-14 feature b).
// RED phase: written FIRST, against the intent contract, BEFORE any implementation.
//
// Contract:
//   * Collect the REJECT verdict notes across ALL prior rounds of a goal (every
//     verifier slot, every round < current).
//   * Render those notes into the current verifier prompt as an appended section so the
//     verifier sees prior rejections and can verify the fixes.

use std::fs;

use verifier_loop::goal;
use verifier_loop::prompt;
use verifier_loop::verdict::{self, VerdictRecord, VerdictStatus};

/// Seed a goal + config and return the goal id.
fn seed_goal(root: &std::path::Path, goal_text: &str) -> String {
    let cfg = serde_json::json!({
        "m": 1,
        "n": 1,
        "maxTurn": 3,
        "verifierTimeoutSec": 5,
        "backend": "stub",
        "minGoalChars": 0,
        "gitDiffMaxChars": 1000,
    });
    fs::write(root.join("config.json"), cfg.to_string()).unwrap();
    goal::new(root, goal_text, None).expect("NEW seeds a goal")
}

/// Write a REJECT verdict into a slot with notes.
fn write_reject(root: &std::path::Path, goal_id: &str, vid: &str, round: u32, notes: &str) {
    let dir = verifier_loop::verdict::verdict_path(root, goal_id, vid, round);
    fs::create_dir_all(&dir).unwrap();
    let rec = VerdictRecord {
        status: VerdictStatus::Reject,
        notes: Some(notes.into()),
        registered_at: Some(chrono::Utc::now().to_rfc3339()),
        signature: None,
        pubkey_id: None,
    };
    fs::write(
        dir.join(verdict::VERDICT_FILE),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();
}

#[test]
fn collect_prior_reject_notes_gathers_all_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let gid = seed_goal(root, "ship feature x");

    // Round 1: v1 REJECT "missing tests", v2 REJECT "no docs"
    write_reject(root, &gid, "v1", 1, "missing tests");
    write_reject(root, &gid, "v2", 1, "no docs");
    // Round 2: v1 REJECT "tests flaky"
    write_reject(root, &gid, "v1", 2, "tests flaky");
    // Round 3 (current): should collect rounds 1 + 2 only.
    let notes = prompt::collect_prior_reject_notes(root, &gid, 3);
    assert!(
        notes.contains("missing tests"),
        "must include round-1 v1 notes: {notes:?}"
    );
    assert!(
        notes.contains("no docs"),
        "must include round-1 v2 notes: {notes:?}"
    );
    assert!(
        notes.contains("tests flaky"),
        "must include round-2 v1 notes: {notes:?}"
    );
}

#[test]
fn collect_prior_reject_notes_excludes_current_round() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let gid = seed_goal(root, "ship feature y");

    write_reject(root, &gid, "v1", 1, "early issue");
    write_reject(root, &gid, "v1", 2, "current round should be excluded");
    let notes = prompt::collect_prior_reject_notes(root, &gid, 2);
    assert!(
        notes.contains("early issue"),
        "round-1 notes included: {notes:?}"
    );
    assert!(
        !notes.contains("current round should be excluded"),
        "current round notes must NOT be included: {notes:?}"
    );
}

#[test]
fn collect_prior_reject_notes_empty_when_no_rejects() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let gid = seed_goal(root, "ship feature z");
    let notes = prompt::collect_prior_reject_notes(root, &gid, 1);
    assert!(
        notes.trim().is_empty(),
        "no prior rejects => empty notes section: {notes:?}"
    );
}

#[test]
fn collect_prior_reject_notes_ignores_approve_and_null() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let gid = seed_goal(root, "ship feature w");

    // v1 APPROVE (should be ignored), v2 REJECT (should be collected).
    let v1_dir = verifier_loop::verdict::verdict_path(root, &gid, "v1", 1);
    fs::create_dir_all(&v1_dir).unwrap();
    let approve = VerdictRecord {
        status: VerdictStatus::Approve,
        notes: None,
        registered_at: Some(chrono::Utc::now().to_rfc3339()),
        signature: None,
        pubkey_id: None,
    };
    fs::write(
        v1_dir.join(verdict::VERDICT_FILE),
        serde_json::to_string(&approve).unwrap(),
    )
    .unwrap();
    write_reject(root, &gid, "v2", 1, "only this must appear");

    let notes = prompt::collect_prior_reject_notes(root, &gid, 2);
    assert!(
        notes.contains("only this must appear"),
        "reject notes collected: {notes:?}"
    );
    assert!(
        !notes.to_lowercase().contains("approve"),
        "approve verdicts must not contribute: {notes:?}"
    );
}

#[test]
fn append_reject_notes_section_appears_in_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let gid = seed_goal(root, "ship feature a");

    write_reject(root, &gid, "v1", 1, "round one feedback");
    let notes = prompt::collect_prior_reject_notes(root, &gid, 2);
    let rendered = "BASE PROMPT BODY";
    let with_notes = prompt::append_prior_reject_notes(rendered, &notes);
    assert!(
        with_notes.starts_with(rendered),
        "original prompt must be preserved as a prefix"
    );
    assert!(
        with_notes.contains("round one feedback"),
        "reject notes must be appended: {with_notes}"
    );
    assert!(
        with_notes.len() > rendered.len(),
        "appended prompt must be longer than the base"
    );
}

#[test]
fn append_reject_notes_noop_when_empty() {
    let rendered = "BASE PROMPT BODY";
    let with_notes = prompt::append_prior_reject_notes(rendered, "");
    assert_eq!(
        with_notes, rendered,
        "empty notes must leave the prompt unchanged"
    );
}
