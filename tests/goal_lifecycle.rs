// tasks.md §3 — Goal lifecycle (goal-lifecycle spec).
// RED phase: written first against the spec, before implementation.

use std::fs;
use std::path::Path;

use verifier_loop::{goal, store};

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

#[test]
fn new_creates_immutable_signed_goal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let goal_id = goal::new(root, "fix the auth bug", Some("ticket #42")).expect("NEW succeeds");
    assert!(!goal_id.is_empty(), "goalId is non-empty");

    let gdir = root.join("goals").join(&goal_id);
    let goal_json_path = gdir.join("goal.json");
    let sig_path = gdir.join("signature.json");
    assert!(goal_json_path.exists(), "goal.json written");
    assert!(sig_path.exists(), "signature.json written");
    assert!(gdir.join("rounds").exists(), "rounds/ dir created");

    let goal_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&goal_json_path).unwrap()).unwrap();
    assert_eq!(goal_json["goalText"], "fix the auth bug");
    assert_eq!(goal_json["context"], "ticket #42");
    assert!(goal_json["createdAt"].is_string(), "createdAt present");
}

#[test]
fn signature_matches_sha256_of_salt_goaltext_createdat() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let goal_id = goal::new(root, "fix the auth bug", Some("ticket #42")).unwrap();
    let gdir = root.join("goals").join(&goal_id);

    let salt = store::salt_in(root).unwrap();
    let goal_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("goal.json")).unwrap()).unwrap();
    let goal_text = goal_json["goalText"].as_str().unwrap();
    let created_at = goal_json["createdAt"].as_str().unwrap();

    let expected = sha256_hex(&format!("{salt}{goal_text}{created_at}"));

    let sig_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("signature.json")).unwrap()).unwrap();
    assert_eq!(sig_json["signature"].as_str().unwrap(), expected);
}

#[test]
fn editing_goaltext_breaks_signature_recompute() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let goal_id = goal::new(root, "original goal", None).unwrap();
    let gdir = root.join("goals").join(&goal_id);

    // Tamper: change goalText in goal.json.
    let mut goal_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("goal.json")).unwrap()).unwrap();
    goal_json["goalText"] = serde_json::Value::String("tampered goal".into());
    fs::write(
        gdir.join("goal.json"),
        serde_json::to_string_pretty(&goal_json).unwrap(),
    )
    .unwrap();

    // Recompute signature from the (now tampered) goal.json: it must NOT match the stored sig.
    assert!(
        goal::verify_signature(root, &goal_id).is_err(),
        "tampered goalText must fail signature verification"
    );
}

#[test]
fn resume_increments_round_and_appends_fix_notes_without_touching_goal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let goal_id = goal::new(root, "some goal", None).unwrap();
    let gdir = root.join("goals").join(&goal_id);

    let goal_before = fs::read(gdir.join("goal.json")).unwrap();
    let sig_before = fs::read(gdir.join("signature.json")).unwrap();

    let round =
        goal::resume(root, &goal_id, Some("fixed issues 1 and 2")).expect("RESUME succeeds");
    assert_eq!(round, 2, "RESUME after NEW(round 1) must yield round 2");

    let goal_after = fs::read(gdir.join("goal.json")).unwrap();
    let sig_after = fs::read(gdir.join("signature.json")).unwrap();
    assert_eq!(
        goal_before, goal_after,
        "goal.json byte-identical after RESUME"
    );
    assert_eq!(
        sig_before, sig_after,
        "signature.json byte-identical after RESUME"
    );

    let fix_notes_path = gdir.join("rounds").join("2").join("fix-notes.json");
    assert!(
        fix_notes_path.exists(),
        "fix-notes.json written for round 2"
    );
    let fix_notes: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fix_notes_path).unwrap()).unwrap();
    assert_eq!(fix_notes["notes"][0], "fixed issues 1 and 2");
}

#[test]
fn resume_can_be_called_multiple_times_appending_each_time() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = goal::new(root, "g", None).unwrap();

    let r1 = goal::current_round(root, &goal_id).unwrap();
    assert_eq!(r1, 1, "NEW leaves current round at 1");
    let r2 = goal::resume(root, &goal_id, Some("fix A")).unwrap();
    let r3 = goal::resume(root, &goal_id, Some("fix B")).unwrap();
    assert_eq!(r2, 2);
    assert_eq!(r3, 3);

    let fix_notes: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            root.join("goals")
                .join(&goal_id)
                .join("rounds")
                .join("3")
                .join("fix-notes.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(fix_notes["notes"][0], "fix B");
}

#[test]
fn missing_goal_directory_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // No goal created; resume on a nonexistent id must error.
    let res = goal::resume(root, "nonexistent-id", Some("x"));
    assert!(res.is_err(), "RESUME on missing goal must fail closed");
}

#[test]
fn missing_store_yields_no_goal_creation() {
    // If the store root cannot be created (e.g. it is a file, not a dir), NEW fails closed.
    let dir = tempfile::tempdir().unwrap();
    let bad_root = dir.path().join("afile");
    fs::write(&bad_root, "x").unwrap();
    let res = goal::new(&bad_root, "g", None);
    assert!(
        res.is_err(),
        "NEW must fail closed when store root is unusable"
    );
}

#[test]
fn signature_verification_succeeds_for_untampered_goal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = goal::new(root, "clean goal", None).unwrap();
    assert!(goal::verify_signature(root, &goal_id).is_ok());
}

#[test]
fn new_ensures_salt_exists() {
    // NEW must ensure the salt is present (it is an input to signature.json).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert!(!root.join(".salt").exists());
    let _ = goal::new(root, "g", None).unwrap();
    assert!(root.join(".salt").exists(), "NEW ensures .salt is created");
}

#[test]
fn salt_value_never_appears_in_goal_files() {
    let dir = tempfile::tempdir().unwrap();
    let root: &Path = dir.path();
    let goal_id = goal::new(root, "secret goal", None).unwrap();
    let salt = store::salt_in(root).unwrap();
    let gdir = root.join("goals").join(&goal_id);
    for f in ["goal.json", "signature.json"] {
        let content = fs::read_to_string(gdir.join(f)).unwrap();
        assert!(
            !content.contains(&salt),
            "salt value must not appear in {f}"
        );
    }
}
