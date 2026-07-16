// RED phase (task: resume --notes append-only goal notes + NEW --init-prompt-file).
//
// Unit-level tests for the new goal-notes library API:
//   * `goal::append_notes(root, goal_id, &[String])` — appends each note as a separate
//     line to `goals/<id>/goal-notes.json` (append-only; NEVER mutates goal.json /
//     signature.json).
//   * `goal::load_notes(root, goal_id) -> Result<Vec<String>, GoalError>` — returns every
//     note ever appended, in insertion order.
//
// These are expected to FAIL until the GREEN teammate implements the two functions in
// `src/goal/mod.rs`. The exact contract these tests pin (so the GREEN teammate knows the
// API shape):
//   - goal-notes.json is a `{"notes": ["line1", "line2", ...]}` JSON object.
//   - Each `append_notes` call pushes onto the same array (true append-only, no overwrite).
//   - goal.json / signature.json are byte-for-byte unchanged across any append_notes call.
//   - There is NO function to strip / remove / update / clear notes — only append + load.

use std::fs;

use verifier_loop::goal;

/// Read `goal-notes.json` as a JSON value, panicking if absent / unparseable.
fn read_goal_notes_json(root: &std::path::Path, goal_id: &str) -> serde_json::Value {
    let p = root
        .join("goals")
        .join(goal_id)
        .join("goal-notes.json");
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn append_notes_creates_goal_notes_file_with_each_note_on_its_own_entry() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = goal::new(root, "the original immutable goal", None).unwrap();

    let notes = vec!["first appended note".to_string(), "second appended note".to_string()];
    goal::append_notes(root, &goal_id, &notes).expect("append_notes must succeed");

    let stored = read_goal_notes_json(root, &goal_id);
    let arr = stored["notes"].as_array().expect("notes is a JSON array");
    assert_eq!(arr.len(), 2, "each note is its own array entry: {stored}");
    assert_eq!(arr[0], "first appended note");
    assert_eq!(arr[1], "second appended note");
}

#[test]
fn append_notes_is_truly_append_only_across_multiple_calls() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = goal::new(root, "g", None).unwrap();

    // First append.
    goal::append_notes(root, &goal_id, &["A".to_string()]).unwrap();
    // Second append (different invocation / round).
    goal::append_notes(root, &goal_id, &["B".to_string()]).unwrap();
    // Third append with TWO notes at once.
    goal::append_notes(
        root,
        &goal_id,
        &["C".to_string(), "D".to_string()],
    )
    .unwrap();

    let loaded = goal::load_notes(root, &goal_id).expect("load_notes succeeds");
    assert_eq!(
        loaded,
        vec!["A", "B", "C", "D"],
        "append_notes must be append-only and preserve insertion order across calls: {loaded:?}"
    );
}

#[test]
fn append_notes_never_mutates_goal_json_or_signature_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = goal::new(root, "frozen goal text", None).unwrap();
    let gdir = root.join("goals").join(&goal_id);

    let goal_before = fs::read(gdir.join("goal.json")).unwrap();
    let sig_before = fs::read(gdir.join("signature.json")).unwrap();

    goal::append_notes(root, &goal_id, &["a note that must not touch the goal".to_string()]).unwrap();

    let goal_after = fs::read(gdir.join("goal.json")).unwrap();
    let sig_after = fs::read(gdir.join("signature.json")).unwrap();
    assert_eq!(
        goal_before, goal_after,
        "goal.json must be byte-identical after append_notes (initial goal is immutable)"
    );
    assert_eq!(
        sig_before, sig_after,
        "signature.json must be byte-identical after append_notes"
    );
}

#[test]
fn load_notes_returns_empty_vec_when_no_notes_appended() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = goal::new(root, "no notes yet", None).unwrap();

    let loaded = goal::load_notes(root, &goal_id).expect("load_notes on a fresh goal succeeds");
    assert!(
        loaded.is_empty(),
        "load_notes must return an empty Vec (not an error) when no notes exist: {loaded:?}"
    );
}

#[test]
fn load_notes_fails_closed_for_missing_goal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let result = goal::load_notes(root, "nonexistent-goal-id");
    assert!(
        result.is_err(),
        "load_notes must fail closed when the goal does not exist"
    );
}

#[test]
fn append_notes_fails_closed_for_missing_goal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let result = goal::append_notes(root, "nonexistent-goal-id", &["x".to_string()]);
    assert!(
        result.is_err(),
        "append_notes must fail closed when the goal does not exist"
    );
}

#[test]
fn signature_still_validates_after_append_notes() {
    // Notes must NOT be part of the signature input, so the creation-time signature must
    // still validate after notes are appended.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = goal::new(root, "signed goal", None).unwrap();

    goal::append_notes(root, &goal_id, &["note".to_string()]).unwrap();

    assert!(
        goal::verify_signature(root, &goal_id).is_ok(),
        "verify_signature must still pass after append_notes (notes are not a hash input)"
    );
}
