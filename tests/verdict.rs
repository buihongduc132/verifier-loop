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

// ===========================================================================
// Pinned verifier pubkey (verifier-identity spec)
// RED phase for add-verifier-tamper-hardening §2 (tasks.md). These tests demand
// the new `mint_and_pin_pubkey` / `read_pinned_pubkey` / `VerifierPubkeyFile` /
// `pubkey_path` API on `verifier_loop::verdict`. They will FAIL TO COMPILE until
// the GREEN team adds that API — that IS RED.
// ===========================================================================

use verifier_loop::crypto;
use verifier_loop::verdict::VerifierPubkeyFile;

/// Mint a keypair into a fresh goal's v1 slot and return (TempDir, goal_id) so each
/// test below has an isolated store. Mirrors `fresh_goal_with_null_verdict` but does
/// NOT pre-create a verdict.json (the pubkey mint must succeed on an empty slot).
fn fresh_goal_for_pubkey() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), "build it", None).unwrap();
    (dir, goal_id)
}

#[test]
fn mint_and_pin_pubkey_writes_file_before_returning() {
    let (dir, goal_id) = fresh_goal_for_pubkey();

    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1)
        .expect("mint_and_pin_pubkey must succeed on a fresh slot");

    // File MUST exist at the pinned location.
    let file = verdict::pubkey_path(dir.path(), &goal_id, "v1", 1).join("verifier-pubkey.json");
    assert!(file.exists(), "pinned pubkey file must exist at {file:?}");

    // On-disk schema: {pubkey: <64 hex>, mintedAt: <iso>}.
    let raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    let pubkey_hex = raw["pubkey"].as_str().expect("pubkey field must be a hex string");
    assert_eq!(
        pubkey_hex.len(),
        64,
        "pubkey must be the 64-hex encoding of a 32-byte Ed25519 verifying key"
    );
    assert!(
        hex::decode(pubkey_hex).is_ok(),
        "pubkey must be valid hex"
    );
    let minted_at = raw["mintedAt"].as_str().expect("mintedAt must be a string");
    assert!(!minted_at.is_empty(), "mintedAt must be populated");

    // Returned signing key's verifying_key() MUST equal the pinned pubkey bytes.
    let pinned_vk = crypto::verifying_key_from_hex(pubkey_hex).unwrap();
    let returned_vk = sk.verifying_key();
    assert_eq!(
        crypto::verifying_key_to_hex(&returned_vk),
        pubkey_hex,
        "returned signing key must correspond to the pinned verifying key"
    );
    assert_eq!(
        crypto::verifying_key_to_hex(&pinned_vk),
        crypto::verifying_key_to_hex(&returned_vk),
    );
}

#[test]
fn mint_and_pin_pubkey_second_call_on_same_slot_fails() {
    let (dir, goal_id) = fresh_goal_for_pubkey();

    let _first = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let second = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1);

    let err = second.expect_err("second mint on the same slot must fail closed (immutable)");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("pin") || msg.contains("exists") || msg.contains("final"),
        "second-mint error must name the immutability reason; got: {err}"
    );
}

#[test]
fn mint_and_pin_pubkey_distinct_keys_across_verifiers() {
    let (dir, goal_id) = fresh_goal_for_pubkey();

    verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v2", 1).unwrap();

    let read = |vid: &str| -> String {
        let file = verdict::pubkey_path(dir.path(), &goal_id, vid, 1).join("verifier-pubkey.json");
        let raw: VerifierPubkeyFile = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        raw.pubkey
    };

    let pk_v1 = read("v1");
    let pk_v2 = read("v2");
    assert_ne!(
        pk_v1, pk_v2,
        "distinct verifier slots MUST mint distinct keypairs (fresh per slot)"
    );
}

#[test]
fn read_pinned_pubkey_returns_none_when_absent() {
    let (dir, goal_id) = fresh_goal_for_pubkey();

    let result = verdict::read_pinned_pubkey(dir.path(), &goal_id, "v1", 1)
        .expect("read on a slot without a pinned pubkey must be Ok(None), not an error");
    assert!(
        result.is_none(),
        "absent verifier-pubkey.json must resolve to None (caller treats as Unauthenticated)"
    );
}

#[test]
fn read_pinned_pubkey_returns_some_when_present() {
    let (dir, goal_id) = fresh_goal_for_pubkey();

    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let expected_hex = crypto::verifying_key_to_hex(&sk.verifying_key());

    let key = verdict::read_pinned_pubkey(dir.path(), &goal_id, "v1", 1)
        .expect("read on a minted slot must be Ok")
        .expect("minted slot must read back Some(key)");

    assert_eq!(
        crypto::verifying_key_to_hex(&key),
        expected_hex,
        "read_pinned_pubkey must return exactly the key that was minted"
    );
}

// ===========================================================================
// Signed verdict record (signed-verdict-record spec)
// RED phase for add-verifier-tamper-hardening §3. Demands `signature` + `pubkeyId`
// fields on `VerdictRecord` and a `verify_record` function. FAILS TO COMPILE until
// the GREEN team adds them — that IS RED.
// ===========================================================================

/// Build a genuine signed APPROVE record using the given signing key. The signature
/// covers the canonical bytes of {status:APPROVE, notes:None, registeredAt, goalId,
/// verifierId, round} exactly as the GREEN team's `verify_record` will recompute.
fn signed_approve_record(
    sk: &crypto::SigningKey,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
    registered_at: &str,
) -> verdict::VerdictRecord {
    let vk = sk.verifying_key();
    let canon = crypto::canonical_record_bytes(
        "APPROVE",
        None,
        registered_at,
        goal_id,
        verifier_id,
        round,
    );
    let sig = crypto::sign(&canon, sk);
    verdict::VerdictRecord {
        status: verdict::VerdictStatus::Approve,
        notes: None,
        registered_at: Some(registered_at.to_string()),
        signature: Some(hex::encode(&sig)),
        pubkey_id: Some(crypto::pubkey_id(&vk)),
    }
}

#[test]
fn approve_record_carries_signature_and_pubkey_id() {
    // Hex shapes only — correctness of the signature is exercised by verify_record tests.
    let sig_128 = "a".repeat(128);
    let pub_id_16 = "b".repeat(16);

    let rec = verdict::VerdictRecord {
        status: verdict::VerdictStatus::Approve,
        notes: None,
        registered_at: Some("2026-07-04T12:00:00+00:00".to_string()),
        signature: Some(sig_128.clone()),
        pubkey_id: Some(pub_id_16.clone()),
    };

    let j = serde_json::to_string(&rec).unwrap();
    assert!(
        j.contains(&format!("\"signature\":\"{sig_128}\"")),
        "serialized record must carry signature verbatim: {j}"
    );
    assert!(
        j.contains(&format!("\"pubkeyId\":\"{pub_id_16}\"")),
        "serialized record must carry pubkeyId (camelCase) verbatim: {j}"
    );
}

#[test]
fn null_placeholder_has_no_signature_fields() {
    let rec = verdict::VerdictRecord {
        status: verdict::VerdictStatus::Null,
        notes: None,
        registered_at: None,
        signature: None,
        pubkey_id: None,
    };

    let j = serde_json::to_string(&rec).unwrap();
    assert!(!j.contains("signature"), "null placeholder must omit signature: {j}");
    assert!(!j.contains("pubkeyId"), "null placeholder must omit pubkeyId: {j}");
    assert!(!j.contains("pubkey_id"), "no snake_case leak: {j}");
}

#[test]
fn verify_record_accepts_genuine_signature() {
    let (dir, goal_id) = fresh_goal_for_pubkey();
    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let vk = sk.verifying_key();

    let iso = "2026-07-04T12:00:00+00:00";
    let rec = signed_approve_record(&sk, &goal_id, "v1", 1, iso);

    verdict::verify_record(&rec, Some(&vk), &goal_id, "v1", 1)
        .expect("a genuine signature over the canonical bytes must verify Ok(())");
}

#[test]
fn verify_record_rejects_edited_status() {
    let (dir, goal_id) = fresh_goal_for_pubkey();
    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let vk = sk.verifying_key();

    let iso = "2026-07-04T12:00:00+00:00";
    let mut rec = signed_approve_record(&sk, &goal_id, "v1", 1, iso);
    // Tamper: flip status to REJECT without re-signing.
    rec.status = verdict::VerdictStatus::Reject;

    let err = verdict::verify_record(&rec, Some(&vk), &goal_id, "v1", 1)
        .expect_err("edited status must invalidate the signature");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("bad") || msg.contains("signature") || msg.contains("mismatch"),
        "edited-status error must be BadSignature-shaped; got: {err}"
    );
}

#[test]
fn verify_record_rejects_edited_notes() {
    let (dir, goal_id) = fresh_goal_for_pubkey();
    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let vk = sk.verifying_key();

    let iso = "2026-07-04T12:00:00+00:00";
    let mut rec = signed_approve_record(&sk, &goal_id, "v1", 1, iso);
    // Tamper: add notes after signing.
    rec.notes = Some("late addition".to_string());

    let err = verdict::verify_record(&rec, Some(&vk), &goal_id, "v1", 1)
        .expect_err("added notes must invalidate the signature");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("bad") || msg.contains("signature") || msg.contains("mismatch"),
        "edited-notes error must be BadSignature-shaped; got: {err}"
    );
}

#[test]
fn verify_record_rejects_edited_registered_at() {
    let (dir, goal_id) = fresh_goal_for_pubkey();
    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let vk = sk.verifying_key();

    let iso = "2026-07-04T12:00:00+00:00";
    let mut rec = signed_approve_record(&sk, &goal_id, "v1", 1, iso);
    // Tamper: flip registered_at after signing.
    rec.registered_at = Some("2026-07-04T23:59:59+00:00".to_string());

    let err = verdict::verify_record(&rec, Some(&vk), &goal_id, "v1", 1)
        .expect_err("edited registeredAt must invalidate the signature");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("bad") || msg.contains("signature") || msg.contains("mismatch"),
        "edited-registeredAt error must be BadSignature-shaped; got: {err}"
    );
}

#[test]
fn verify_record_rejects_identity_mismatch() {
    let (dir, goal_id) = fresh_goal_for_pubkey();
    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let vk = sk.verifying_key();

    let iso = "2026-07-04T12:00:00+00:00";
    // Signed for v1 — but presented as v2's verdict.
    let rec = signed_approve_record(&sk, &goal_id, "v1", 1, iso);

    let err = verdict::verify_record(&rec, Some(&vk), &goal_id, "v2", 1)
        .expect_err("identity mismatch (verifierId) must invalidate the signature");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("bad") || msg.contains("signature") || msg.contains("mismatch"),
        "identity-mismatch error must be BadSignature-shaped; got: {err}"
    );
}

#[test]
fn verify_record_wrong_pubkey_when_pinned_missing() {
    let (dir, goal_id) = fresh_goal_for_pubkey();
    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();

    let iso = "2026-07-04T12:00:00+00:00";
    let rec = signed_approve_record(&sk, &goal_id, "v1", 1, iso);

    // No pinned pubkey supplied -> cannot be trusted even though the signature is valid.
    let err = verdict::verify_record(&rec, None, &goal_id, "v1", 1)
        .expect_err("missing pinned pubkey must fail closed");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("pubkey") || msg.contains("untrusted") || msg.contains("pin"),
        "missing-pinned-pubkey error must name pubkey/untrusted/pin; got: {err}"
    );
}

#[test]
fn verify_record_wrong_pubkey_when_pubkey_id_mismatch() {
    let (dir, goal_id) = fresh_goal_for_pubkey();
    let sk_a = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let vk_a = sk_a.verifying_key();

    // A different keypair whose pubkeyId is NOT vk_a's.
    let other = crypto::generate_keypair();
    let vk_b = other.verifying;

    let iso = "2026-07-04T12:00:00+00:00";
    let mut rec = signed_approve_record(&sk_a, &goal_id, "v1", 1, iso);
    // Lie about the pubkeyId: claim it belongs to vk_b while the signature is vk_a's.
    rec.pubkey_id = Some(crypto::pubkey_id(&vk_b));

    let err = verdict::verify_record(&rec, Some(&vk_a), &goal_id, "v1", 1)
        .expect_err("pubkeyId mismatch must fail closed even if the signature itself is valid");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("pubkey") || msg.contains("wrong") || msg.contains("mismatch"),
        "pubkeyId-mismatch error must name the wrong-pubkey reason; got: {err}"
    );
}

#[test]
fn verify_record_untrusted_for_null_status() {
    let (dir, goal_id) = fresh_goal_for_pubkey();
    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let vk = sk.verifying_key();

    // A null-status record, even with a signature attached, is non-matching by spec.
    let rec = verdict::VerdictRecord {
        status: verdict::VerdictStatus::Null,
        notes: None,
        registered_at: None,
        signature: Some("a".repeat(128)),
        pubkey_id: Some(crypto::pubkey_id(&vk)),
    };

    let err = verdict::verify_record(&rec, Some(&vk), &goal_id, "v1", 1)
        .expect_err("null-status record must NOT verify even if a signature is set");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("untrusted") || msg.contains("null") || msg.contains("not") || msg.contains("status"),
        "null-status error must be Untrusted-shaped; got: {err}"
    );
}

// ===========================================================================
// Secret-required verdict registration (verdict-registration MODIFIED spec)
// RED phase for add-verifier-tamper-hardening §4 (tasks.md). The `jewije` binary
// now MUST read VERIFIER_LOOP_VERIFIER_SECRET (hex Ed25519 signing key), derive the
// verifying pubkey, compare to the slot's pinned verifier-pubkey.json, and FAIL CLOSED
// (VerdictError::Unauthenticated) on absence / non-match. On success the written
// verdict.json is SIGNED (signature + pubkeyId) and verifies against the pinned key.
//
// These tests drive the BUILT `verifier-verdict` binary via assert_cmd (the real
// user-facing contract). They will FAIL today because:
//   * bin/verifier_verdict.rs does not yet read VERIFIER_LOOP_VERIFIER_SECRET, so
//     approve-without-secret currently SUCCEEDS (must fail closed).
//   * the written verdict.json carries no signature (must be signed).
// ===========================================================================

/// Build a fresh temp store + goal with a pre-created null verdict slot AND a pinned
/// pubkey for v1, returning (TempDir, goal_id, signing_key_hex). The signing key hex
/// is what a verifier process would receive as VERIFIER_LOOP_VERIFIER_SECRET.
fn fresh_goal_with_pinned_v1(round: u32) -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), "build it", None).unwrap();

    // Pre-create the null verdict placeholder (mirrors the spawn layer).
    let vdir = verdict::verdict_path(dir.path(), &goal_id, "v1", round);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(vdir.join(verdict::VERDICT_FILE), r#"{"status":null}"#).unwrap();

    // Pin a real pubkey and capture the signing key hex.
    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", round)
        .expect("mint_and_pin_pubkey must succeed on a fresh slot");
    let secret_hex = crypto::signing_key_to_hex(&sk);
    (dir, goal_id, secret_hex)
}

/// A verdict slot file path under a temp store.
fn slot_verdict_file(root: &Path, goal_id: &str, vid: &str, round: u32) -> PathBuf {
    verdict::verdict_path(root, goal_id, vid, round).join(verdict::VERDICT_FILE)
}

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Scenario: Missing signing secret fails closed
// ---------------------------------------------------------------------------

/// jewije approve with identity env but NO VERIFIER_LOOP_VERIFIER_SECRET must exit
/// non-zero, write NO verdict.json change (slot stays null), and surface an
/// unauthenticated/secret-shaped error on stderr.
#[test]
fn jewije_approve_without_secret_fails_closed() {
    let (dir, goal_id, _secret) = fresh_goal_with_pinned_v1(1);
    let slot = slot_verdict_file(dir.path(), &goal_id, "v1", 1);

    let assert = Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        // VERIFIER_LOOP_VERIFIER_SECRET deliberately NOT set.
        .arg("approve")
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_lowercase();
    assert!(
        stderr.contains("secret") || stderr.contains("unauthenticated") || stderr.contains("auth"),
        "stderr must name the missing-secret / unauthenticated reason; got: {stderr}"
    );

    // The slot must remain null — no verdict written.
    let rec = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    assert_eq!(rec.status, verdict::VerdictStatus::Null, "no write on missing secret");
    // And no stray signature/pubkeyId appear.
    assert!(rec.signature.is_none(), "no signature may be written without a secret");
    assert!(rec.pubkey_id.is_none(), "no pubkeyId may be written without a secret");
    // Sanity: the file itself was not turned into an APPROVE record on disk.
    let raw = fs::read_to_string(&slot).unwrap();
    assert!(!raw.contains("APPROVE"), "raw slot must not have been mutated: {raw}");
}

// ---------------------------------------------------------------------------
// Scenario: Signing secret that does not match the pinned pubkey fails closed
// ---------------------------------------------------------------------------

/// jewije approve with a VERIFIER_LOOP_VERIFIER_SECRET whose pubkey does NOT equal
/// the slot's pinned verifier-pubkey.json must exit non-zero, leave the slot null,
/// and surface an unauthenticated/pubkey/pin-shaped error.
#[test]
fn jewije_approve_with_wrong_secret_fails_closed() {
    let (dir, goal_id, _correct_secret) = fresh_goal_with_pinned_v1(1);

    // Mint a SECOND, unrelated keypair whose pubkey is NOT the slot's pinned key.
    let wrong = crypto::generate_keypair();
    let wrong_hex = crypto::signing_key_to_hex(&wrong.signing);

    let assert = Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &wrong_hex)
        .arg("approve")
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_lowercase();
    assert!(
        stderr.contains("unauthenticated")
            || stderr.contains("pubkey")
            || stderr.contains("pin")
            || stderr.contains("mismatch"),
        "stderr must name the wrong-secret / wrong-pubkey reason; got: {stderr}"
    );

    // Slot must be unchanged (still null placeholder, no APPROVE written).
    let rec = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    assert_eq!(
        rec.status,
        verdict::VerdictStatus::Null,
        "wrong-secret approve must not mutate the slot"
    );
    assert!(rec.signature.is_none(), "no signature written on wrong-secret");
}

// ---------------------------------------------------------------------------
// Scenario: First verdict registers and is signed (APPROVE)
// ---------------------------------------------------------------------------

/// jewije approve with the CORRECT secret must exit 0, write a verdict.json whose
/// status is APPROVE, carries a 128-hex signature and 16-hex pubkeyId, and whose
/// signature verifies against the pinned verifying key.
#[test]
fn jewije_approve_with_correct_secret_writes_signed_verdict() {
    let (dir, goal_id, secret) = fresh_goal_with_pinned_v1(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret)
        .arg("approve")
        .assert()
        .success()
        .stdout(predicates::str::contains("Verdict registered"));

    // On-disk schema: status=APPROVE, signature (128 hex), pubkeyId (16 hex).
    let slot = slot_verdict_file(dir.path(), &goal_id, "v1", 1);
    let raw: Value = serde_json::from_str(&fs::read_to_string(&slot).unwrap()).unwrap();
    assert_eq!(raw["status"], Value::String(APPROVE.into()));
    let sig = raw["signature"].as_str().expect("signature must be present on signed verdict");
    assert_eq!(
        sig.len(),
        128,
        "signature must be the 128-hex encoding of a 64-byte Ed25519 signature; got len {}",
        sig.len()
    );
    hex::decode(sig).expect("signature must be valid hex");
    let pub_id = raw["pubkeyId"].as_str().expect("pubkeyId must be present");
    assert_eq!(pub_id.len(), 16, "pubkeyId must be the 16-hex prefix; got len {}", pub_id.len());

    // The written record MUST verify against the slot's pinned pubkey.
    let pinned_vk = verdict::read_pinned_pubkey(dir.path(), &goal_id, "v1", 1)
        .expect("read pinned pubkey")
        .expect("pinned pubkey must be present after fresh_goal_with_pinned_v1");
    let record = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    verdict::verify_record(&record, Some(&pinned_vk), &goal_id, "v1", 1)
        .expect("the written verdict must verify against the pinned verifying key");
}

// ---------------------------------------------------------------------------
// Scenario: First verdict registers and is signed (REJECT + notes)
// ---------------------------------------------------------------------------

/// jewije reject --notes 'reason' with the CORRECT secret must exit 0, write a
/// REJECT record carrying the notes, with a signature that verifies.
#[test]
fn jewije_reject_with_correct_secret_writes_signed_verdict_with_notes() {
    let (dir, goal_id, secret) = fresh_goal_with_pinned_v1(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret)
        .args(["reject", "--notes", "reason: missing tests"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Verdict registered"));

    let slot = slot_verdict_file(dir.path(), &goal_id, "v1", 1);
    let raw: Value = serde_json::from_str(&fs::read_to_string(&slot).unwrap()).unwrap();
    assert_eq!(raw["status"], Value::String(REJECT.into()));
    assert_eq!(
        raw["notes"].as_str(),
        Some("reason: missing tests"),
        "notes must be persisted verbatim"
    );
    assert_eq!(
        raw["signature"].as_str().map(str::len),
        Some(128),
        "signed REJECT must carry a 128-hex signature"
    );
    assert_eq!(
        raw["pubkeyId"].as_str().map(str::len),
        Some(16),
        "signed REJECT must carry a 16-hex pubkeyId"
    );

    // Signature must verify against the pinned pubkey.
    let pinned_vk = verdict::read_pinned_pubkey(dir.path(), &goal_id, "v1", 1)
        .unwrap()
        .expect("pinned pubkey present");
    let record = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    verdict::verify_record(&record, Some(&pinned_vk), &goal_id, "v1", 1)
        .expect("signed REJECT must verify against the pinned verifying key");
}

// ---------------------------------------------------------------------------
// Scenario: reject without notes is still refused (existing behavior preserved)
// ---------------------------------------------------------------------------

/// Even WITH the correct secret, reject --notes '' (empty) must fail. The secret gate
/// does not bypass the notes-required check (existing behavior preserved).
#[test]
fn jewije_reject_without_notes_fails() {
    let (dir, goal_id, secret) = fresh_goal_with_pinned_v1(1);

    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret)
        .args(["reject", "--notes", ""])
        .assert()
        .failure();

    // Slot stays null.
    let rec = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    assert_eq!(rec.status, verdict::VerdictStatus::Null, "empty-notes reject must not write");
}

// ---------------------------------------------------------------------------
// Scenario: Second verdict for the same slot is rejected (AlreadyFinal)
// ---------------------------------------------------------------------------

/// After a successful signed APPROVE, a second jewije approve on the same slot must
/// fail (AlreadyFinal) and leave the original signed verdict byte-for-byte unchanged.
#[test]
fn jewije_second_verdict_on_same_slot_fails_already_final() {
    let (dir, goal_id, secret) = fresh_goal_with_pinned_v1(1);

    // First verdict: signed APPROVE via the correct secret.
    Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret)
        .arg("approve")
        .assert()
        .success();

    // Snapshot the signed verdict bytes AFTER the first write so we can prove the
    // second attempt does not mutate it.
    let slot = slot_verdict_file(dir.path(), &goal_id, "v1", 1);
    let first_bytes = fs::read_to_string(&slot).unwrap();
    assert!(first_bytes.contains("signature"), "first verdict must be signed (RED if unsigned)");

    // Second attempt — even with the same correct secret — must fail (AlreadyFinal).
    let assert = Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret)
        .arg("approve")
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_lowercase();
    assert!(
        stderr.contains("already") || stderr.contains("final"),
        "second verdict must surface AlreadyFinal; got: {stderr}"
    );

    // The original signed verdict must be byte-for-byte unchanged.
    let after_bytes = fs::read_to_string(&slot).unwrap();
    assert_eq!(
        first_bytes, after_bytes,
        "already-final slot must NOT be mutated by a second attempt"
    );
}

// ---------------------------------------------------------------------------
// Scenario: Null-slot first-fill without the pinned secret fails closed
// ---------------------------------------------------------------------------

/// A goal dir whose slot has NO pinned verifier-pubkey.json (a pre-change goal layout)
/// must reject any jewije approve, regardless of which secret is supplied. The pinned
/// pubkey is the trust anchor — no anchor => Unauthenticated.
#[test]
fn jewije_approve_on_slot_without_pinned_pubkey_fails_closed() {
    // Build a fresh goal + null slot WITHOUT minting a pinned pubkey.
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), "build it", None).unwrap();
    let vdir = verdict::verdict_path(dir.path(), &goal_id, "v1", 1);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(vdir.join(verdict::VERDICT_FILE), r#"{"status":null}"#).unwrap();
    assert!(
        !verdict::pubkey_path(dir.path(), &goal_id, "v1", 1).join(verdict::PUBKEY_FILE).exists(),
        "test precondition: no pinned pubkey for this slot"
    );

    // Any secret — even a freshly-minted valid one — must fail because there is no
    // pinned anchor to match against.
    let arbitrary = crypto::generate_keypair();
    let arbitrary_hex = crypto::signing_key_to_hex(&arbitrary.signing);

    let assert = Command::cargo_bin("verifier-verdict")
        .unwrap()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &arbitrary_hex)
        .arg("approve")
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_lowercase();
    assert!(
        stderr.contains("unauthenticated")
            || stderr.contains("pubkey")
            || stderr.contains("pin"),
        "approve on a slot without a pinned pubkey must surface an unauthenticated/pubkey-shaped error; got: {stderr}"
    );

    // The slot stays null.
    let rec = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    assert_eq!(rec.status, verdict::VerdictStatus::Null, "slot must remain null");
}
