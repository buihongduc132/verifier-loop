// add-verifier-tamper-hardening — RED phase for src/crypto/ (tasks.md §1.3).
//
// These tests define the contract for the new `verifier_loop::crypto` module. They
// CANNOT COMPILE today because `src/crypto/mod.rs` does not exist (and `crypto` is not
// declared in `src/lib.rs`). That compile failure IS the RED state.
//
// Spec source of truth:
//   - specs/verifier-identity/spec.md  (Per-verifier signing keypair minted at spawn)
//   - specs/signed-verdict-record/spec.md (signature over canonical record bytes)
//   - design.md D0 (Ed25519 via ed25519-dalek v2 + rand v0.8)
//   - design.md D7 (canonical bytes = serde_json BTreeMap-sorted keys, no whitespace)
//   - design.md D8 (crates)
//
// The GREEN team will create src/crypto/mod.rs and declare `pub mod crypto;` in lib.rs.
// API surface demanded here MUST be implemented exactly (re-exports of ed25519_dalek
// types are acceptable as long as the names resolve under verifier_loop::crypto::*).

use verifier_loop::crypto::{
    canonical_record_bytes, generate_keypair, pubkey_id, sign, signing_key_from_hex,
    signing_key_to_hex, verify, verifying_key_from_hex, verifying_key_to_hex,
};

// ---------------------------------------------------------------------------
// D0 — Ed25519 keypair freshness + sign/verify primitives
// ---------------------------------------------------------------------------

#[test]
fn keypair_generate_produces_distinct_keys() {
    let a = generate_keypair();
    let b = generate_keypair();
    assert_ne!(
        verifying_key_to_hex(&a.verifying),
        verifying_key_to_hex(&b.verifying),
        "two generate_keypair() calls must yield distinct verifying pubkeys"
    );
    assert_ne!(
        signing_key_to_hex(&a.signing),
        signing_key_to_hex(&b.signing),
        "two generate_keypair() calls must yield distinct signing keys"
    );
}

#[test]
fn sign_then_verify_round_trips() {
    let kp = generate_keypair();
    let msg = b"canonical record bytes payload";
    let sig = sign(msg, &kp.signing);
    assert!(
        verify(&sig, msg, &kp.verifying),
        "signature produced by sign() must verify against the matching pubkey"
    );
}

#[test]
fn signature_is_64_bytes() {
    // Ed25519 signatures are exactly 64 bytes.
    let kp = generate_keypair();
    let sig = sign(b"any message", &kp.signing);
    assert_eq!(sig.len(), 64, "Ed25519 signature must be 64 bytes, got {}", sig.len());
}

#[test]
fn verify_rejects_flipped_signature_byte() {
    let kp = generate_keypair();
    let msg = b"message";
    let mut sig = sign(msg, &kp.signing);
    sig[0] ^= 0xff; // flip all bits of byte 0
    assert!(
        !verify(&sig, msg, &kp.verifying),
        "a signature with a flipped byte must NOT verify"
    );
}

#[test]
fn verify_rejects_flipped_message_byte() {
    let kp = generate_keypair();
    let mut msg = *b"message payload";
    let sig = sign(&msg, &kp.signing);
    msg[0] ^= 0xff; // flip a message byte after signing
    assert!(
        !verify(&sig, &msg, &kp.verifying),
        "verify must fail when the canonical bytes are modified after signing"
    );
}

#[test]
fn verify_rejects_wrong_pubkey() {
    let kp_a = generate_keypair();
    let kp_b = generate_keypair();
    let msg = b"message";
    let sig = sign(msg, &kp_a.signing);
    assert!(
        !verify(&sig, msg, &kp_b.verifying),
        "a valid signature must NOT verify against a different keypair's pubkey"
    );
}

// ---------------------------------------------------------------------------
// D7 — canonical_record_bytes determinism + layout
// ---------------------------------------------------------------------------

#[test]
fn canonical_record_bytes_is_deterministic_across_calls() {
    // Same logical inputs → byte-identical output, regardless of how many times called.
    let a = canonical_record_bytes("APPROVE", None, "2026-07-04T10:00:00Z", "goal-1", "v1", 1);
    let b = canonical_record_bytes("APPROVE", None, "2026-07-04T10:00:00Z", "goal-1", "v1", 1);
    assert_eq!(a, b, "canonical_record_bytes must be deterministic for identical inputs");
}

#[test]
fn canonical_record_bytes_uses_sorted_keys_and_no_whitespace() {
    // design.md D7: serde_json BTreeMap-sorted keys, no whitespace.
    // BTreeMap orders keys lexicographically: goalId < notes < registeredAt < round < status < verifierId.
    let bytes = canonical_record_bytes(
        "APPROVE",
        Some("notes here"),
        "2026-07-04T10:00:00Z",
        "goal-1",
        "v1",
        1,
    );
    let s = std::str::from_utf8(&bytes).expect("canonical bytes are valid UTF-8 JSON");
    assert!(!s.contains(' '), "canonical JSON must contain no spaces: {s}");
    assert!(!s.contains('\n'), "canonical JSON must contain no newlines: {s}");
    // Assert the keys appear in alphabetical order.
    let pos_goal = s.find("\"goalId\"").expect("goalId key present");
    let pos_notes = s.find("\"notes\"").expect("notes key present");
    let pos_reg = s.find("\"registeredAt\"").expect("registeredAt key present");
    let pos_round = s.find("\"round\"").expect("round key present");
    let pos_status = s.find("\"status\"").expect("status key present");
    let pos_vid = s.find("\"verifierId\"").expect("verifierId key present");
    assert!(pos_goal < pos_notes, "goalId must sort before notes");
    assert!(pos_notes < pos_reg, "notes must sort before registeredAt");
    assert!(pos_reg < pos_round, "registeredAt must sort before round");
    assert!(pos_round < pos_status, "round must sort before status");
    assert!(pos_status < pos_vid, "status must sort before verifierId");
}

#[test]
fn canonical_record_bytes_serializes_none_notes_as_null() {
    let bytes = canonical_record_bytes("APPROVE", None, "2026-07-04T10:00:00Z", "goal-1", "v1", 1);
    let s = std::str::from_utf8(&bytes).unwrap();
    assert!(s.contains("\"notes\":null"), "None notes must serialize as null: {s}");
}

#[test]
fn canonical_record_bytes_distinguishes_approve_from_reject() {
    // A status change MUST change the canonical bytes (signature binds status — guards
    // against in-flight REJECT→APPROVE edits per signed-verdict-record spec).
    let approve = canonical_record_bytes("APPROVE", None, "2026-07-04T10:00:00Z", "goal-1", "v1", 1);
    let reject = canonical_record_bytes("REJECT", Some("nope"), "2026-07-04T10:00:00Z", "goal-1", "v1", 1);
    assert_ne!(approve, reject, "APPROVE and REJECT must produce distinct canonical bytes");
}

#[test]
fn canonical_record_bytes_distinguishes_verifier_ids() {
    // signed-verdict-record spec: copying a verdict signed for v1 into v2's slot must
    // fail signature verification. That requires verifierId to be part of the canonical bytes.
    let v1 = canonical_record_bytes("APPROVE", None, "2026-07-04T10:00:00Z", "goal-1", "v1", 1);
    let v2 = canonical_record_bytes("APPROVE", None, "2026-07-04T10:00:00Z", "goal-1", "v2", 1);
    assert_ne!(v1, v2, "different verifierId must produce distinct canonical bytes (binds identity)");
}

#[test]
fn canonical_record_bytes_distinguishes_rounds() {
    let r1 = canonical_record_bytes("APPROVE", None, "2026-07-04T10:00:00Z", "goal-1", "v1", 1);
    let r2 = canonical_record_bytes("APPROVE", None, "2026-07-04T10:00:00Z", "goal-1", "v1", 2);
    assert_ne!(r1, r2, "different round must produce distinct canonical bytes");
}

// ---------------------------------------------------------------------------
// Hex round-trips + pubkey_id
// ---------------------------------------------------------------------------

#[test]
fn signing_key_hex_round_trips() {
    let kp = generate_keypair();
    let hex = signing_key_to_hex(&kp.signing);
    let back = signing_key_from_hex(&hex).expect("hex round-trips into the same signing key");
    assert_eq!(
        signing_key_to_hex(&back),
        hex,
        "signing_key_from_hex(signing_key_to_hex(sk)) must reproduce the same key"
    );
}

#[test]
fn verifying_key_hex_round_trips() {
    let kp = generate_keypair();
    let hex = verifying_key_to_hex(&kp.verifying);
    let back = verifying_key_from_hex(&hex).expect("hex round-trips into the same verifying key");
    assert_eq!(
        verifying_key_to_hex(&back),
        hex,
        "verifying_key_from_hex(verifying_key_to_hex(vk)) must reproduce the same key"
    );
}

#[test]
fn signing_key_derives_matching_verifying_key() {
    // The verifying key derived from a signing key must be the one we round-tripped through hex.
    // GREEN team must ensure SigningKey -> VerifyingKey derivation is wired (ed25519_dalek does this).
    let kp = generate_keypair();
    let sk_hex = signing_key_to_hex(&kp.signing);
    let sk_back = signing_key_from_hex(&sk_hex).unwrap();
    // Sign with the round-tripped key and verify against the ORIGINAL pubkey.
    let msg = b"round-trip integrity";
    let sig = sign(msg, &sk_back);
    assert!(
        verify(&sig, msg, &kp.verifying),
        "signing key round-tripped through hex must sign messages verifiable by the original pubkey"
    );
}

#[test]
fn pubkey_id_is_first_16_hex_of_pubkey() {
    let kp = generate_keypair();
    let full_hex = verifying_key_to_hex(&kp.verifying);
    let id = pubkey_id(&kp.verifying);
    assert_eq!(id.len(), 16, "pubkey_id must be 16 hex chars, got {} ({})", id.len(), id);
    assert!(
        full_hex.starts_with(&id),
        "pubkey_id must equal the first 16 hex chars of the pubkey; full={full_hex} id={id}"
    );
}

#[test]
fn signing_key_from_hex_rejects_invalid_input() {
    // Fail-closed on malformed hex.
    assert!(signing_key_from_hex("not-hex").is_err(), "garbage hex must error");
    assert!(signing_key_from_hex("").is_err(), "empty hex must error");
    // Wrong length (Ed25519 signing keys are 32 bytes = 64 hex chars).
    assert!(signing_key_from_hex("ab").is_err(), "too-short hex must error");
}

#[test]
fn verifying_key_from_hex_rejects_invalid_input() {
    assert!(verifying_key_from_hex("zz").is_err(), "non-hex must error");
    assert!(verifying_key_from_hex("").is_err(), "empty hex must error");
}
