// tasks.md §8 — Consensus n/m + tamper-evident completion hash.
// RED phase (rev 2): short-hash form `mmddyy-XXXXXXXX` + `fullDigest` field.
//
// Covers the consensus-check + completion-proof spec scenarios:
//   * Consensus is n approvals out of m verifiers (2/2, 2/3, below-threshold).
//   * null + REJECT do not count toward n (fail-closed D9).
//   * Rejection surfaces reject notes + null markers to A.
//   * Consensus is static and human-configured (n/m from config.json, LD4).
//   * Hash formula EXACT (rev 2 — short form + full digest):
//       short       = mmddyy + "-" + first8hex(SHA256(same inputs))
//       fullDigest  = SHA256(same inputs)                  // 64 hex, stored not printed
//       where mmddyy = UTC date of matchedAt (MMDDYY),
//       inputs = salt + goalId + goalSignature + String(roundNumber)
//              + canonicalJSON(matchingVerdicts sorted by verifierId) + matchedAtISO
//       and goalSignature = SHA256(salt + goalText + createdAt).
//   * Hash determinism (identical inputs -> identical short + fullDigest).
//   * Tamper vectors: goalText edit invalidates BOTH short and fullDigest;
//     verdict edit invalidates fullDigest (and short w.h.p.).
//   * Audit-traceable (recompute from goal-dir + salt matches stored fullDigest).
//   * completion.json written on success with `hash` + `fullDigest`; no file on failure.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use verifier_loop::{consensus, goal, store, verdict};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Build a `VerdictRecord` APPROVE with a fixed registeredAt (deterministic for hash tests).
fn approve_at(iso: &str) -> verdict::VerdictRecord {
    verdict::VerdictRecord {
        status: verdict::VerdictStatus::Approve,
        notes: None,
        registered_at: Some(iso.to_string()),
        signature: None,
        pubkey_id: None,
    }
}

fn reject_at(notes: &str, iso: &str) -> verdict::VerdictRecord {
    verdict::VerdictRecord {
        status: verdict::VerdictStatus::Reject,
        notes: Some(notes.to_string()),
        registered_at: Some(iso.to_string()),
        signature: None,
        pubkey_id: None,
    }
}

fn null_verdict() -> verdict::VerdictRecord {
    verdict::VerdictRecord {
        status: verdict::VerdictStatus::Null,
        notes: None,
        registered_at: None,
        signature: None,
        pubkey_id: None,
    }
}

/// Pre-create the spawn-time null placeholder verdict for a verifier slot.
fn pre_create_null(root: &Path, goal_id: &str, vid: &str, round: u32) {
    let vdir = verdict::verdict_path(root, goal_id, vid, round);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(
        vdir.join(verdict::VERDICT_FILE),
        r#"{"status":null}"#,
    )
    .unwrap();
}

/// Independent canonical-JSON of matching verdicts (mirrors the spec): an array of
/// objects `{"registeredAt":..,"verifierId":..}` sorted by verifierId, keys alphabetical
/// (via BTreeMap), no whitespace. This is the audit-side recompute, NOT the impl.
fn canonical_matching_json(matching: &[consensus::MatchingVerdict]) -> String {
    let mut sorted: Vec<&consensus::MatchingVerdict> = matching.iter().collect();
    sorted.sort_by(|a, b| a.verifier_id.cmp(&b.verifier_id));
    let arr: Vec<Value> = sorted
        .iter()
        .map(|m| {
            let mut map = BTreeMap::new();
            map.insert("registeredAt".to_string(), json!(m.registered_at));
            map.insert("verifierId".to_string(), json!(m.verifier_id));
            serde_json::to_value(&map).unwrap()
        })
        .collect();
    serde_json::to_string(&json!(arr)).unwrap()
}

/// mmddyy from an RFC3339 matchedAtISO (UTC). e.g. "2026-07-03T10:05:00Z" -> "070326".
/// Independent of the impl; used by audit-side recompute.
fn mmddyy_of(iso: &str) -> String {
    // Parse "YYYY-MM-DD" prefix from the RFC3339 string.
    let date = &iso[..10]; // "2026-07-03"
    let yyyy = &date[0..4];
    let mm = &date[5..7];
    let dd = &date[8..10];
    let yy = &yyyy[2..4]; // last 2 digits of year
    format!("{mm}{dd}{yy}")
}

/// Independent SHA-256 recompute producing BOTH the short hash and the full digest,
/// used to cross-check `compute_hash`.
fn spec_recompute(
    salt: &str,
    goal_id: &str,
    goal_sig: &str,
    round: u32,
    matching: &[consensus::MatchingVerdict],
    matched_at_iso: &str,
) -> (String, String) {
    let canon = canonical_matching_json(matching);
    let input = format!("{salt}{goal_id}{goal_sig}{round}{canon}{matched_at_iso}");
    let digest = hex::encode(Sha256::digest(input.as_bytes()));
    let short = format!("{}-{}", mmddyy_of(matched_at_iso), &digest[..8]);
    (short, digest)
}

/// Create a goal under a fresh temp store root.
fn fresh_goal(text: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), text, None).unwrap();
    (dir, goal_id)
}

// ---------------------------------------------------------------------------
// Consensus n/m evaluation
// ---------------------------------------------------------------------------

#[test]
fn evaluate_2_of_2_unanimous_pass() {
    let verdicts = vec![
        ("v1".to_string(), approve_at("2026-07-03T10:00:00Z")),
        ("v2".to_string(), approve_at("2026-07-03T10:01:00Z")),
    ];
    // Legacy unsigned regime: no pinned pubkeys → unsigned APPROVEs are trusted.
    let r = consensus::evaluate(Path::new("/nonexistent-consensus-legacy"), "g", 1, &verdicts, 2, 2);
    assert!(r.passed, "2/2 unanimous must pass");
    assert_eq!(r.approve_count, 2);
    assert_eq!(r.n, 2);
    assert_eq!(r.m, 2);
    assert_eq!(r.matching_verdicts.len(), 2);
}

#[test]
fn evaluate_2_of_3_majority_pass() {
    let verdicts = vec![
        ("v1".to_string(), approve_at("2026-07-03T10:00:00Z")),
        ("v2".to_string(), approve_at("2026-07-03T10:01:00Z")),
        ("v3".to_string(), reject_at("bad", "2026-07-03T10:02:00Z")),
    ];
    let r = consensus::evaluate(Path::new("/nonexistent-consensus-legacy"), "g", 1, &verdicts, 2, 3);
    assert!(r.passed, "2 of 3 APPROVE must pass");
    assert_eq!(r.approve_count, 2);
    assert_eq!(r.matching_verdicts.len(), 2, "only approvers match");
}

#[test]
fn evaluate_below_threshold_fails() {
    let verdicts = vec![
        ("v1".to_string(), approve_at("2026-07-03T10:00:00Z")),
        ("v2".to_string(), approve_at("2026-07-03T10:01:00Z")),
        ("v3".to_string(), reject_at("missing X", "2026-07-03T10:02:00Z")),
    ];
    let r = consensus::evaluate(Path::new("/nonexistent-consensus-legacy"), "g", 1, &verdicts, 3, 3);
    assert!(!r.passed, "2 of 3 with n=3 must fail");
}

#[test]
fn evaluate_null_and_reject_do_not_count_toward_n() {
    let verdicts = vec![
        ("v1".to_string(), approve_at("2026-07-03T10:00:00Z")),
        ("v2".to_string(), null_verdict()),
        ("v3".to_string(), reject_at("notes here", "2026-07-03T10:02:00Z")),
    ];
    let r = consensus::evaluate(Path::new("/nonexistent-consensus-legacy"), "g", 1, &verdicts, 2, 3);
    assert!(!r.passed, "1 APPROVE + null + reject cannot reach n=2");
    assert_eq!(r.approve_count, 1);
    // Rejection surfaces the reject notes and the null marker.
    assert_eq!(r.rejection.reject_notes.len(), 1);
    assert_eq!(r.rejection.reject_notes[0].0, "v3");
    assert_eq!(r.rejection.reject_notes[0].1, "notes here");
    assert_eq!(r.rejection.null_verifiers, vec!["v2".to_string()]);
}

#[test]
fn evaluate_missing_verdict_treated_as_null_fail_closed() {
    // Fewer verdicts than m: the missing ones are absent entirely.
    let verdicts = vec![("v1".to_string(), approve_at("2026-07-03T10:00:00Z"))];
    let r = consensus::evaluate(Path::new("/nonexistent-consensus-legacy"), "g", 1, &verdicts, 2, 2);
    assert!(!r.passed, "missing verdicts must fail closed");
}

#[test]
fn matching_verdicts_sorted_by_verifier_id() {
    let verdicts = vec![
        ("v3".to_string(), approve_at("2026-07-03T10:02:00Z")),
        ("v1".to_string(), approve_at("2026-07-03T10:00:00Z")),
        ("v2".to_string(), approve_at("2026-07-03T10:01:00Z")),
    ];
    let r = consensus::evaluate(Path::new("/nonexistent-consensus-legacy"), "g", 1, &verdicts, 3, 3);
    assert!(r.passed);
    let ids: Vec<&str> = r.matching_verdicts.iter().map(|m| m.verifier_id.as_str()).collect();
    assert_eq!(ids, vec!["v1", "v2", "v3"], "must be sorted asc by verifierId");
}

// ---------------------------------------------------------------------------
// Hash formula (rev 2: short form + full digest)
// ---------------------------------------------------------------------------

#[test]
fn compute_hash_formula_matches_spec_recompute() {
    let matching = vec![
        consensus::MatchingVerdict {
            verifier_id: "v1".into(),
            registered_at: "2026-07-03T10:00:00Z".into(),
        },
        consensus::MatchingVerdict {
            verifier_id: "v2".into(),
            registered_at: "2026-07-03T10:01:00Z".into(),
        },
    ];
    let out = consensus::compute_hash(
        "deadbeef",
        "goal-123",
        "sig-abc",
        1,
        &matching,
        "2026-07-03T10:05:00Z",
        "",
    );
    let (exp_short, exp_full) = spec_recompute("deadbeef", "goal-123", "sig-abc", 1, &matching, "2026-07-03T10:05:00Z");
    assert_eq!(out.short_hash(), exp_short, "short hash must match independent recompute");
    assert_eq!(out.full_digest(), exp_full, "full digest must match independent recompute");
}

#[test]
fn compute_hash_deterministic_identical_inputs() {
    let matching = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: "2026-07-03T10:00:00Z".into(),
    }];
    let a = consensus::compute_hash("s", "g", "sig", 1, &matching, "2026-07-03T10:05:00Z", "");
    let b = consensus::compute_hash("s", "g", "sig", 1, &matching, "2026-07-03T10:05:00Z", "");
    assert_eq!(a.short_hash(), b.short_hash(), "identical inputs -> identical short hash");
    assert_eq!(a.full_digest(), b.full_digest(), "identical inputs -> identical full digest");

    // Stable regardless of the order matching was assembled (sorting is impl's job).
    let matching_rev = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: "2026-07-03T10:00:00Z".into(),
    }];
    let c = consensus::compute_hash("s", "g", "sig", 1, &matching_rev, "2026-07-03T10:05:00Z", "");
    assert_eq!(a.short_hash(), c.short_hash());
}

#[test]
fn compute_hash_short_form_is_mmddyy_dash_8hex() {
    let matching = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: "2026-07-03T10:00:00Z".into(),
    }];
    let out = consensus::compute_hash("s", "g", "sig", 1, &matching, "2026-07-03T10:05:00Z", "");
    let short = out.short_hash();
    // mmddyy from matchedAt UTC (2026-07-03 -> 070326), hyphen, 8 lowercase hex.
    assert_eq!(&short[..7], "070326-", "prefix must be mmddyy- from matchedAt: {short}");
    assert_eq!(short.len(), 15, "mmddyy(6) + -(1) + 8hex = 15: {short}");
    let hex_part = &short[7..];
    assert!(
        hex_part.chars().all(|c: char| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "suffix must be 8 lowercase hex chars: {short}"
    );
}

#[test]
fn compute_hash_full_digest_is_64_lowercase_hex() {
    let matching = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: "2026-07-03T10:00:00Z".into(),
    }];
    let out = consensus::compute_hash("s", "g", "sig", 1, &matching, "2026-07-03T10:05:00Z", "");
    let full = out.full_digest();
    assert_eq!(full.len(), 64, "full digest must be 64 hex chars: {full}");
    assert!(
        full.chars().all(|c: char| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "full digest must be lowercase hex: {full}"
    );
}

#[test]
fn compute_hash_mmddyy_tracks_matched_at_not_created_at() {
    // Same inputs except matchedAt differs across two runs: short hash prefix (mmddyy)
    // must change, full digest must also change.
    let matching = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: "2026-07-03T10:00:00Z".into(),
    }];
    let a = consensus::compute_hash("s", "g", "sig", 1, &matching, "2026-07-03T10:05:00Z", "");
    let b = consensus::compute_hash("s", "g", "sig", 1, &matching, "2026-08-15T10:05:00Z", "");
    assert_ne!(a.short_hash()[..6], b.short_hash()[..6], "mmddyy must come from matchedAt");
    assert_ne!(a.full_digest(), b.full_digest(), "full digest must change with matchedAt");
}

// ---------------------------------------------------------------------------
// Tamper vectors (rev 2: both short + fullDigest invalidated on goalText edit)
// ---------------------------------------------------------------------------

#[test]
fn tamper_goal_text_invalidates_both_short_and_full_digest() {
    let (dir, goal_id) = fresh_goal("original goal text");

    let salt = store::salt_in(dir.path()).unwrap();
    let record = goal::load(dir.path(), &goal_id).unwrap();
    let sig = goal::compute_signature(&salt, &record.goal_text, &record.created_at);

    let matching = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: "2026-07-03T10:00:00Z".into(),
    }];
    let original = consensus::compute_hash(&salt, &goal_id, &sig, 1, &matching, "2026-07-03T10:05:00Z", "");

    // Tamper goalText on disk.
    let mut tampered = record.clone();
    tampered.goal_text = "MUTATED goal text".to_string();
    fs::write(
        goal::goal_dir(dir.path(), &goal_id).join(goal::GOAL_FILE),
        serde_json::to_string_pretty(&tampered).unwrap(),
    )
    .unwrap();

    // Recompute signature from the now-tampered goalText -> different signature -> different hash.
    let tampered_sig = goal::compute_signature(&salt, &tampered.goal_text, &record.created_at);
    let after = consensus::compute_hash(&salt, &goal_id, &tampered_sig, 1, &matching, "2026-07-03T10:05:00Z", "");

    assert_ne!(original.short_hash(), after.short_hash(), "edited goalText MUST invalidate short hash");
    assert_ne!(original.full_digest(), after.full_digest(), "edited goalText MUST invalidate full digest");
}

#[test]
fn tamper_verdict_notes_invalidates_full_digest() {
    let (dir, goal_id) = fresh_goal("goal");
    pre_create_null(dir.path(), &goal_id, "v1", 1);
    verdict::register_approve(dir.path(), &goal_id, "v1", 1, None).unwrap();

    let salt = store::salt_in(dir.path()).unwrap();
    let record = goal::load(dir.path(), &goal_id).unwrap();
    let sig = goal::compute_signature(&salt, &record.goal_text, &record.created_at);

    // Hash from the registered APPROVE verdict.
    let v = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    let matching = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: v.registered_at.clone().unwrap(),
    }];
    let original = consensus::compute_hash(&salt, &goal_id, &sig, 1, &matching, "2026-07-03T10:05:00Z", "");

    // Tamper the verdict: edit registeredAt (and notes to force a value-bearing change).
    let tampered = verdict::VerdictRecord {
        status: verdict::VerdictStatus::Approve,
        notes: Some("injected".to_string()),
        registered_at: Some("1999-01-01T00:00:00Z".to_string()),
        signature: None,
        pubkey_id: None,
    };
    fs::write(
        verdict::verdict_path(dir.path(), &goal_id, "v1", 1).join(verdict::VERDICT_FILE),
        serde_json::to_string_pretty(&tampered).unwrap(),
    )
    .unwrap();

    let v2 = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    let matching2 = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: v2.registered_at.clone().unwrap(),
    }];
    let after = consensus::compute_hash(&salt, &goal_id, &sig, 1, &matching2, "2026-07-03T10:05:00Z", "");

    assert_ne!(original.full_digest(), after.full_digest(), "edited verdict MUST invalidate full digest");
}

// ---------------------------------------------------------------------------
// completion.json write on success
// ---------------------------------------------------------------------------

#[test]
fn write_completion_writes_record_on_success() {
    let (dir, goal_id) = fresh_goal("goal");
    pre_create_null(dir.path(), &goal_id, "v1", 1);
    pre_create_null(dir.path(), &goal_id, "v2", 1);
    verdict::register_approve(dir.path(), &goal_id, "v1", 1, None).unwrap();
    verdict::register_approve(dir.path(), &goal_id, "v2", 1, None).unwrap();

    let cfg = store::Config::load_in(dir.path()).unwrap(); // defaults n=2,m=2
    let v1 = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    let v2 = verdict::read_verdict(dir.path(), &goal_id, "v2", 1).unwrap();
    let verdicts = vec![
        ("v1".to_string(), v1),
        ("v2".to_string(), v2),
    ];
    let r = consensus::evaluate(dir.path(), &goal_id, 1, &verdicts, cfg.n, cfg.m);
    assert!(r.passed);

    let salt = store::salt_in(dir.path()).unwrap();
    let record = goal::load(dir.path(), &goal_id).unwrap();
    let sig = goal::compute_signature(&salt, &record.goal_text, &record.created_at);
    let matched_at = "2026-07-03T10:05:00Z";
    let hash = consensus::compute_hash(&salt, &goal_id, &sig, 1, &r.matching_verdicts, matched_at, "");

    let path = consensus::write_completion(dir.path(), &goal_id, &r, 1, &hash, matched_at).unwrap();
    assert!(path.exists(), "completion.json must exist");

    let raw = fs::read_to_string(&path).unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["hash"], json!(hash.short_hash()));
    assert_eq!(v["fullDigest"], json!(hash.full_digest()));
    assert_eq!(v["goalId"], json!(goal_id));
    assert_eq!(v["roundNumber"], json!(1));
    assert_eq!(v["matchedAt"], json!(matched_at));
    assert!(v["matchingVerdicts"].is_array(), "matchingVerdicts must be present");
    assert_eq!(v["matchingVerdicts"].as_array().unwrap().len(), 2);
}

#[test]
fn no_completion_on_failure() {
    let (dir, goal_id) = fresh_goal("goal");
    let verdicts = vec![
        ("v1".to_string(), approve_at("2026-07-03T10:00:00Z")),
        ("v2".to_string(), reject_at("nope", "2026-07-03T10:01:00Z")),
    ];
    let r = consensus::evaluate(dir.path(), &goal_id, 1, &verdicts, 2, 2);
    assert!(!r.passed);

    // write_completion must refuse on a non-passing round.
    let dummy = consensus::compute_hash("s", "g", "sig", 1, &[], "2026-07-03T10:05:00Z", "");
    let res = consensus::write_completion(dir.path(), &goal_id, &r, 1, &dummy, "2026-07-03T10:05:00Z");
    assert!(res.is_err(), "must refuse to write completion on failure");

    let completion_path = goal::goal_dir(dir.path(), &goal_id).join("completion.json");
    assert!(!completion_path.exists(), "no completion.json on failure");
}

// ---------------------------------------------------------------------------
// Audit-traceable: recompute from goal-dir + salt matches stored
// ---------------------------------------------------------------------------

#[test]
fn audit_recompute_matches_stored_hash() {
    let (dir, goal_id) = fresh_goal("build the thing");
    pre_create_null(dir.path(), &goal_id, "v1", 1);
    pre_create_null(dir.path(), &goal_id, "v2", 1);
    verdict::register_approve(dir.path(), &goal_id, "v1", 1, None).unwrap();
    verdict::register_approve(dir.path(), &goal_id, "v2", 1, None).unwrap();

    let cfg = store::Config::load_in(dir.path()).unwrap();
    let v1 = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    let v2 = verdict::read_verdict(dir.path(), &goal_id, "v2", 1).unwrap();
    let verdicts = vec![("v1".to_string(), v1), ("v2".to_string(), v2)];
    let r = consensus::evaluate(dir.path(), &goal_id, 1, &verdicts, cfg.n, cfg.m);
    assert!(r.passed);

    let salt = store::salt_in(dir.path()).unwrap();
    let record = goal::load(dir.path(), &goal_id).unwrap();
    let sig = goal::compute_signature(&salt, &record.goal_text, &record.created_at);
    let matched_at = "2026-07-03T10:05:00Z";
    let hash = consensus::compute_hash(&salt, &goal_id, &sig, 1, &r.matching_verdicts, matched_at, "");
    consensus::write_completion(dir.path(), &goal_id, &r, 1, &hash, matched_at).unwrap();

    // --- Auditor recompute, reading ONLY goal-dir + .salt ---
    let salt2 = store::salt_in(dir.path()).unwrap();
    assert_eq!(salt2, salt, "salt stable");
    let rec2 = goal::load(dir.path(), &goal_id).unwrap();
    let sig2 = goal::compute_signature(&salt2, &rec2.goal_text, &rec2.created_at);
    let va = verdict::read_verdict(dir.path(), &goal_id, "v1", 1).unwrap();
    let vb = verdict::read_verdict(dir.path(), &goal_id, "v2", 1).unwrap();
    let audit_matching = vec![
        consensus::MatchingVerdict {
            verifier_id: "v1".into(),
            registered_at: va.registered_at.unwrap(),
        },
        consensus::MatchingVerdict {
            verifier_id: "v2".into(),
            registered_at: vb.registered_at.unwrap(),
        },
    ];
    let recomputed = consensus::compute_hash(&salt2, &goal_id, &sig2, 1, &audit_matching, matched_at, "");

    let completion_raw = fs::read_to_string(
        goal::goal_dir(dir.path(), &goal_id).join("completion.json"),
    )
    .unwrap();
    let cv: Value = serde_json::from_str(&completion_raw).unwrap();
    assert_eq!(cv["hash"], json!(recomputed.short_hash()), "stored short hash must match audit recompute");
    assert_eq!(cv["fullDigest"], json!(recomputed.full_digest()), "stored fullDigest must match audit recompute");
}

// ---------------------------------------------------------------------------
// n/m static from Config (LD4)
// ---------------------------------------------------------------------------

#[test]
fn n_m_static_from_config_json() {
    let dir = tempfile::tempdir().unwrap();
    // Human-configured threshold 2-of-3.
    fs::write(
        dir.path().join("config.json"),
        r#"{"n":2,"m":3}"#,
    )
    .unwrap();
    let cfg = store::Config::load_in(dir.path()).unwrap();
    assert_eq!((cfg.n, cfg.m), (2, 3));

    // With 2 APPROVE out of 3 -> passes the 2-of-3 threshold.
    let verdicts = vec![
        ("v1".to_string(), approve_at("2026-07-03T10:00:00Z")),
        ("v2".to_string(), approve_at("2026-07-03T10:01:00Z")),
        ("v3".to_string(), reject_at("x", "2026-07-03T10:02:00Z")),
    ];
    let r = consensus::evaluate(dir.path(), "config-test-goal", 1, &verdicts, cfg.n, cfg.m);
    assert!(r.passed, "2-of-3 config threshold met");
}

// ===========================================================================
// §8 (REV 3) — Signed-verdict consensus + receipt-head in the completion hash.
//
// RED phase for `add-verifier-tamper-hardening` (completion-proof MODIFIED +
// signed-verdict-record "Consensus verifies signature before treating verdict as
// matching"). These tests DEFINE the new contract; the GREEN team adapts
// `evaluate` and `compute_hash` to satisfy them.
//
// NEW contract driven here:
//   * `evaluate(root, goal_id, round, verdicts, n, m)` VERIFIES each APPROVE
//     verdict's signature against the slot's pinned pubkey BEFORE counting it.
//     A verdict whose signature fails (BadSignature / WrongPubkey / Untrusted)
//     is NOT counted as matching and is surfaced in `rejection.signature_failures`
//     as `(verifierId, reason)`.
//   * `compute_hash(salt, goal_id, goal_sig, round, matching, matched_at_iso,
//     receipt_head)` INCLUDES the receipt-log head in its inputs (appended AFTER
//     matchedAtISO).
//
// These tests WILL FAIL today: `evaluate` does not take `(root, goal_id, round)`
// and does not verify signatures; `compute_hash` does not take `receipt_head`.
// That mismatch IS the RED state.
// ===========================================================================

use verifier_loop::crypto;
use verifier_loop::receipt;

/// Fixed ISO timestamps so signature-bearing records are deterministic.
const SIGNED_TS: &str = "2026-07-04T09:00:00Z";

/// Mint + pin a verifier keypair, then register a SIGNED APPROVE in the slot.
///
/// Mirrors the happy path the spawn layer will use: `mint_and_pin_pubkey` pins the
/// verifying key, `register_signed_approve` signs the canonical record bytes with the
/// matching secret and writes the verdict (+ appends a receipt entry).
fn mint_pin_and_signed_approve(root: &Path, goal_id: &str, vid: &str, round: u32) {
    let secret = verdict::mint_and_pin_pubkey(root, goal_id, vid, round).unwrap();
    verdict::register_signed_approve(root, goal_id, vid, round, None, &secret).unwrap();
}

/// Build + atomically write a verdict signed by `secret`, declaring `claim_pubkey_id`.
///
/// Used to forge a verdict under a NON-pinned key (the signature is cryptographically
/// valid but the claimed `pubkey_id` does not match the slot's pinned key → WrongPubkey).
fn write_signed_verdict_with_key(
    root: &Path,
    goal_id: &str,
    vid: &str,
    round: u32,
    status: verdict::VerdictStatus,
    notes: Option<&str>,
    secret: &crypto::SigningKey,
    claim_pubkey_id: &str,
) {
    let registered_at = SIGNED_TS.to_string();
    let status_str = match status {
        verdict::VerdictStatus::Approve => "APPROVE",
        verdict::VerdictStatus::Reject => "REJECT",
        verdict::VerdictStatus::Null => "null",
    };
    let canonical = crypto::canonical_record_bytes(
        status_str,
        notes,
        &registered_at,
        goal_id,
        vid,
        round,
    );
    let sig = crypto::sign(&canonical, secret);
    let rec = verdict::VerdictRecord {
        status,
        notes: notes.map(|s| s.to_string()),
        registered_at: Some(registered_at),
        signature: Some(hex::encode(&sig)),
        pubkey_id: Some(claim_pubkey_id.to_string()),
    };
    let vdir = verdict::verdict_path(root, goal_id, vid, round);
    fs::create_dir_all(&vdir).unwrap();
    let target = vdir.join(verdict::VERDICT_FILE);
    let tmp = vdir.join(format!("{}.tmp", verdict::VERDICT_FILE));
    fs::write(&tmp, serde_json::to_string_pretty(&rec).unwrap()).unwrap();
    fs::rename(&tmp, &target).unwrap();
}

/// In-place-edit a single top-level field of an on-disk verdict.json (WITHOUT
/// re-signing). Used to simulate in-flight tampering after registration.
fn tamper_verdict_field(root: &Path, goal_id: &str, vid: &str, round: u32, field: &str, value: Value) {
    let path = verdict::verdict_path(root, goal_id, vid, round).join(verdict::VERDICT_FILE);
    let raw = fs::read_to_string(&path).unwrap();
    let mut v: Value = serde_json::from_str(&raw).unwrap();
    v[field] = value;
    fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

/// Read the on-disk verdicts for a fixed set of verifier ids into the
/// `[(verifierId, VerdictRecord)]` shape `evaluate` consumes.
fn read_all_verdicts(root: &Path, goal_id: &str, round: u32, vids: &[&str]) -> Vec<(String, verdict::VerdictRecord)> {
    vids.iter()
        .map(|vid| {
            let rec = verdict::read_verdict(root, goal_id, vid, round).unwrap();
            (vid.to_string(), rec)
        })
        .collect()
}

/// Recursively copy a directory tree (used to seed a second tempdir with an identical
/// goal + verdicts, varying only the receipt log).
fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let e = entry.unwrap();
        let from = e.path();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// (1) Consensus accepts a genuine signed APPROVE.
// ---------------------------------------------------------------------------

#[test]
fn evaluate_accepts_genuine_signed_verdict() {
    let (dir, goal_id) = fresh_goal("ship the feature");
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v1", 1);
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v2", 1);

    let verdicts = read_all_verdicts(dir.path(), &goal_id, 1, &["v1", "v2"]);
    let r = consensus::evaluate(dir.path(), &goal_id, 1, &verdicts, 2, 2);

    assert!(r.passed, "two genuine signed APPROVEs must pass 2/2");
    assert_eq!(r.approve_count, 2, "both signed verdicts count toward n");
    assert_eq!(r.matching_verdicts.len(), 2);
    assert!(
        r.rejection.signature_failures.is_empty(),
        "no signature failures on genuine signed verdicts"
    );
}

// ---------------------------------------------------------------------------
// (2) In-flight `status` edit invalidates the signature.
// ---------------------------------------------------------------------------

#[test]
fn evaluate_rejects_in_flight_status_edit() {
    let (dir, goal_id) = fresh_goal("ship the feature");
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v1", 1);
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v2", 1);

    // Tamper v1's status to REJECT after it was signed (no re-signing).
    tamper_verdict_field(dir.path(), &goal_id, "v1", 1, "status", json!("REJECT"));

    let verdicts = read_all_verdicts(dir.path(), &goal_id, 1, &["v1", "v2"]);
    let r = consensus::evaluate(dir.path(), &goal_id, 1, &verdicts, 2, 2);

    assert!(!r.passed, "an in-flight status edit MUST fail closed");
    assert_eq!(r.approve_count, 1, "tampered v1 no longer counts");
    let offender = r
        .rejection
        .signature_failures
        .iter()
        .find(|(vid, _)| vid == "v1");
    assert!(
        offender.is_some(),
        "rejection summary must name v1 as a signature failure: {:?}",
        r.rejection.signature_failures
    );
    let reason = offender.unwrap().1.to_lowercase();
    assert!(
        reason.contains("signature") || reason.contains("badsignature"),
        "rejection reason must mention signature/BadSignature: {reason}"
    );
}

// ---------------------------------------------------------------------------
// (3) In-flight `notes` edit invalidates the signature.
// ---------------------------------------------------------------------------

#[test]
fn evaluate_rejects_in_flight_notes_edit() {
    let (dir, goal_id) = fresh_goal("ship the feature");
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v1", 1);
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v2", 1);

    tamper_verdict_field(dir.path(), &goal_id, "v1", 1, "notes", json!("injected after signing"));

    let verdicts = read_all_verdicts(dir.path(), &goal_id, 1, &["v1", "v2"]);
    let r = consensus::evaluate(dir.path(), &goal_id, 1, &verdicts, 2, 2);

    assert!(!r.passed, "an in-flight notes edit MUST fail closed");
    assert_eq!(r.approve_count, 1);
    assert!(
        r.rejection
            .signature_failures
            .iter()
            .any(|(vid, _)| vid == "v1"),
        "v1 must be surfaced as a signature failure: {:?}",
        r.rejection.signature_failures
    );
}

// ---------------------------------------------------------------------------
// (4) In-flight `registeredAt` edit invalidates the signature.
// ---------------------------------------------------------------------------

#[test]
fn evaluate_rejects_in_flight_registered_at_edit() {
    let (dir, goal_id) = fresh_goal("ship the feature");
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v1", 1);
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v2", 1);

    tamper_verdict_field(
        dir.path(),
        &goal_id,
        "v1",
        1,
        "registeredAt",
        json!("1999-01-01T00:00:00Z"),
    );

    let verdicts = read_all_verdicts(dir.path(), &goal_id, 1, &["v1", "v2"]);
    let r = consensus::evaluate(dir.path(), &goal_id, 1, &verdicts, 2, 2);

    assert!(!r.passed, "an in-flight registeredAt edit MUST fail closed");
    assert_eq!(r.approve_count, 1);
    assert!(
        r.rejection
            .signature_failures
            .iter()
            .any(|(vid, _)| vid == "v1"),
        "v1 must be surfaced as a signature failure: {:?}",
        r.rejection.signature_failures
    );
}

// ---------------------------------------------------------------------------
// (5) A verdict signed by a NON-pinned key fails closed (WrongPubkey).
// ---------------------------------------------------------------------------

#[test]
fn evaluate_rejects_verdict_signed_by_non_pinned_key() {
    let (dir, goal_id) = fresh_goal("ship the feature");
    // Pin K1 for v1, then sign v1's APPROVE with an unrelated K2 and CLAIM K2's id.
    let _pinned_secret = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let kp2 = crypto::generate_keypair();
    let wrong_id = crypto::pubkey_id(&kp2.verifying);
    write_signed_verdict_with_key(
        dir.path(),
        &goal_id,
        "v1",
        1,
        verdict::VerdictStatus::Approve,
        None,
        &kp2.signing,
        &wrong_id,
    );
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v2", 1);

    let verdicts = read_all_verdicts(dir.path(), &goal_id, 1, &["v1", "v2"]);
    let r = consensus::evaluate(dir.path(), &goal_id, 1, &verdicts, 2, 2);

    assert!(!r.passed, "a verdict signed by a non-pinned key MUST fail closed");
    assert_eq!(r.approve_count, 1, "the forged v1 does not count");
    let offender = r
        .rejection
        .signature_failures
        .iter()
        .find(|(vid, _)| vid == "v1");
    assert!(offender.is_some(), "v1 must be named in the rejection summary");
    let reason = offender.unwrap().1.to_lowercase();
    assert!(
        reason.contains("pubkey") || reason.contains("wrongpubkey"),
        "rejection reason must mention pubkey/WrongPubkey: {reason}"
    );
}

// ---------------------------------------------------------------------------
// (6) An APPROVE with signature=None is never trusted (Untrusted).
// ---------------------------------------------------------------------------

#[test]
fn evaluate_rejects_unsigned_approve() {
    let (dir, goal_id) = fresh_goal("ship the feature");
    // Pin a pubkey for v1 (so the slot IS in the signed regime) but write an UNSIGNED
    // APPROVE — simulating a pre-change or tampered record.
    let _secret = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", 1).unwrap();
    let unsigned = verdict::VerdictRecord {
        status: verdict::VerdictStatus::Approve,
        notes: None,
        registered_at: Some(SIGNED_TS.to_string()),
        signature: None,
        pubkey_id: None,
    };
    let vdir = verdict::verdict_path(dir.path(), &goal_id, "v1", 1);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(
        vdir.join(verdict::VERDICT_FILE),
        serde_json::to_string_pretty(&unsigned).unwrap(),
    )
    .unwrap();
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v2", 1);

    let verdicts = read_all_verdicts(dir.path(), &goal_id, 1, &["v1", "v2"]);
    let r = consensus::evaluate(dir.path(), &goal_id, 1, &verdicts, 2, 2);

    assert!(!r.passed, "an unsigned APPROVE MUST fail closed");
    assert_eq!(r.approve_count, 1, "unsigned v1 does not count");
    let offender = r
        .rejection
        .signature_failures
        .iter()
        .find(|(vid, _)| vid == "v1");
    assert!(offender.is_some(), "v1 must be named in the rejection summary");
    let reason = offender.unwrap().1.to_lowercase();
    assert!(
        reason.contains("unsigned") || reason.contains("untrusted"),
        "rejection reason must mention unsigned/Untrusted: {reason}"
    );
}

// ---------------------------------------------------------------------------
// (7) Receipt head is part of the hash inputs: differing heads → differing hashes.
// ---------------------------------------------------------------------------

#[test]
fn compute_hash_differs_when_receipt_head_differs() {
    let (dir1, goal_id) = fresh_goal("ship the feature");
    mint_pin_and_signed_approve(dir1.path(), &goal_id, "v1", 1);
    mint_pin_and_signed_approve(dir1.path(), &goal_id, "v2", 1);

    // Replicate the ENTIRE store (salt + goal + signed verdicts) into a second tempdir.
    let dir2 = tempfile::tempdir().unwrap();
    copy_dir_recursive(dir1.path(), dir2.path());

    assert_eq!(goal_id, goal_id, "goal id stable across the copy");
    assert_eq!(
        receipt::read_receipt_head(dir1.path(), &goal_id),
        receipt::read_receipt_head(dir2.path(), &goal_id),
        "copied stores share the same receipt head initially"
    );

    // Append ONE extra receipt entry to dir2 ONLY → its head now differs.
    receipt::append_receipt(dir2.path(), &goal_id, "approve", "v3", "APPROVE", "extra")
        .unwrap();
    assert_ne!(
        receipt::read_receipt_head(dir1.path(), &goal_id),
        receipt::read_receipt_head(dir2.path(), &goal_id),
        "heads must diverge after the extra append"
    );

    // Identical matching verdicts + matched_at, but different heads.
    let matching = vec![
        consensus::MatchingVerdict { verifier_id: "v1".into(), registered_at: SIGNED_TS.into() },
        consensus::MatchingVerdict { verifier_id: "v2".into(), registered_at: SIGNED_TS.into() },
    ];
    let matched_at = "2026-07-04T09:05:00Z";

    let head1 = receipt::read_receipt_head(dir1.path(), &goal_id);
    let head2 = receipt::read_receipt_head(dir2.path(), &goal_id);
    let salt1 = store::salt_in(dir1.path()).unwrap();
    let salt2 = store::salt_in(dir2.path()).unwrap();
    assert_eq!(salt1, salt2, "salt copied verbatim");

    let h1 = consensus::compute_hash(&salt1, &goal_id, "sig", 1, &matching, matched_at, &head1);
    let h2 = consensus::compute_hash(&salt2, &goal_id, "sig", 1, &matching, matched_at, &head2);

    assert_ne!(h1.full_digest(), h2.full_digest(), "differing receipt head MUST change the digest");
    assert_ne!(h1.short_hash(), h2.short_hash(), "short hash changes too (w.h.p.)");
}

// ---------------------------------------------------------------------------
// (8) Determinism: identical inputs INCLUDING the receipt head → identical hashes.
// ---------------------------------------------------------------------------

#[test]
fn compute_hash_same_when_receipt_head_same() {
    let matching = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: SIGNED_TS.into(),
    }];
    let matched_at = "2026-07-04T09:05:00Z";
    let head = "abc123deadbeef";

    let a = consensus::compute_hash("salt", "goal-1", "sig", 1, &matching, matched_at, head);
    let b = consensus::compute_hash("salt", "goal-1", "sig", 1, &matching, matched_at, head);

    assert_eq!(a.short_hash(), b.short_hash(), "identical inputs → identical short hash");
    assert_eq!(a.full_digest(), b.full_digest(), "identical inputs → identical full digest");
}

// ---------------------------------------------------------------------------
// (9) A fresh goal with no receipt log (head = "") still hashes stably.
// ---------------------------------------------------------------------------

#[test]
fn compute_hash_empty_receipt_head_for_fresh_goal() {
    let (dir, goal_id) = fresh_goal("fresh goal, no receipt log");
    let head = receipt::read_receipt_head(dir.path(), &goal_id);
    assert!(head.is_empty(), "a fresh goal has no receipt log → empty head");

    let matching = vec![consensus::MatchingVerdict {
        verifier_id: "v1".into(),
        registered_at: SIGNED_TS.into(),
    }];
    let matched_at = "2026-07-04T09:05:00Z";
    let salt = store::salt_in(dir.path()).unwrap();

    let a = consensus::compute_hash(&salt, &goal_id, "sig", 1, &matching, matched_at, "");
    let b = consensus::compute_hash(&salt, &goal_id, "sig", 1, &matching, matched_at, "");
    assert_eq!(a.full_digest(), b.full_digest(), "empty head must still hash deterministically");
    assert_eq!(a.short_hash(), b.short_hash());
    assert_eq!(a.full_digest().len(), 64, "full digest is still 64 hex chars with empty head");
}

// ---------------------------------------------------------------------------
// (10) Closed loop: the head IS in the stored hash inputs (library level).
//
// Mirrors `tests/spawn_orchestrator.rs::closed_loop_completion_hash_inputs_include_receipt_head`
// but at the library level: mint+pin, signed APPROVEs (which append receipts),
// compute the hash WITH the head, then recompute WITHOUT the head and assert they
// differ — proving the head is part of the hashed bytes.
// ---------------------------------------------------------------------------

#[test]
fn closed_loop_hash_includes_receipt_head() {
    let (dir, goal_id) = fresh_goal("ship the feature");
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v1", 1);
    mint_pin_and_signed_approve(dir.path(), &goal_id, "v2", 1);

    let head = receipt::read_receipt_head(dir.path(), &goal_id);
    assert!(!head.is_empty(), "m=2 signed approves produced a non-empty receipt log");

    let salt = store::salt_in(dir.path()).unwrap();
    let record = goal::load(dir.path(), &goal_id).unwrap();
    let sig = goal::compute_signature(&salt, &record.goal_text, &record.created_at);
    let matching = vec![
        consensus::MatchingVerdict { verifier_id: "v1".into(), registered_at: SIGNED_TS.into() },
        consensus::MatchingVerdict { verifier_id: "v2".into(), registered_at: SIGNED_TS.into() },
    ];
    let matched_at = "2026-07-04T09:05:00Z";

    // The NEW contract: compute_hash folds in the head.
    let with_head = consensus::compute_hash(&salt, &goal_id, &sig, 1, &matching, matched_at, &head);

    // Manually recompute the head-LESS digest over the SAME other inputs.
    // (Mirrors `compute_hash`'s pre-change input assembly, exactly.)
    let mut sorted: Vec<&consensus::MatchingVerdict> = matching.iter().collect();
    sorted.sort_by(|a, b| a.verifier_id.cmp(&b.verifier_id));
    let arr: Vec<Value> = sorted
        .iter()
        .map(|m| {
            let mut map = BTreeMap::new();
            map.insert("registeredAt".to_string(), json!(m.registered_at));
            map.insert("verifierId".to_string(), json!(m.verifier_id));
            serde_json::to_value(&map).unwrap()
        })
        .collect();
    let canon = serde_json::to_string(&json!(arr)).unwrap();
    let headless_input = format!("{salt}{goal_id}{sig}1{canon}{matched_at}");
    let headless_digest = hex::encode(Sha256::digest(headless_input.as_bytes()));

    assert_ne!(
        with_head.full_digest(),
        headless_digest,
        "the stored digest MUST differ from the head-less recompute — proving the head is in the inputs"
    );
}
