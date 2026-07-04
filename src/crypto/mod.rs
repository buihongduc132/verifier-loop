//! `crypto` — Ed25519 signing/verifying primitives + canonical record bytes.
//!
//! Implements the `add-verifier-tamper-hardening` contract:
//! * `verifier-identity/spec.md`  — per-verifier signing keypair minted at spawn.
//! * `signed-verdict-record/spec.md` — signature over canonical record bytes.
//!
//! Design decisions (design.md):
//! * D0 — Ed25519 via `ed25519_dalek` v2 + `rand` v0.8.
//! * D7 — `canonical_record_bytes` = serde_json with BTreeMap-sorted keys, no whitespace.
//! * D8 — Cargo crates `ed25519-dalek = "2"`, `rand = "0.8"`.
//!
//! Fail-closed: `verify` only returns `true` on a cryptographically valid signature;
//! any tampering (flipped byte, wrong pubkey, modified canonical bytes) yields `false`.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

// Re-export the dalek key + trait types so callers can name them as
// `verifier_loop::crypto::{SigningKey, VerifyingKey, ...}` without depending on the
// crate directly. Bringing `Signer` / `Verifier` into scope is also required locally so
// `.sign()` / `.verify()` resolve on the dalek types.
pub use ed25519_dalek::{SignatureError, Signature, Signer, SigningKey, Verifier, VerifyingKey};

use rand::rngs::OsRng;
use serde_json::Value;

/// Errors surfaced by hex ↔ key conversions.
#[derive(Debug)]
pub enum CryptoError {
    /// Input was not valid hexadecimal.
    InvalidHex,
    /// Hex decoded successfully but the byte length did not match the key size.
    BadLength,
    /// Bytes were the right length but did not encode a valid key (e.g. non-canonical
    /// Ed25519 public key encoding).
    InvalidKey(SignatureError),
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptoError::InvalidHex => write!(f, "invalid hexadecimal input"),
            CryptoError::BadLength => write!(f, "decoded bytes have wrong length for key"),
            CryptoError::InvalidKey(e) => write!(f, "bytes did not encode a valid key: {e}"),
        }
    }
}

impl Error for CryptoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            CryptoError::InvalidKey(e) => Some(e),
            _ => None,
        }
    }
}

impl From<hex::FromHexError> for CryptoError {
    fn from(_: hex::FromHexError) -> Self {
        CryptoError::InvalidHex
    }
}

/// An Ed25519 signing keypair (secret + matching verifying pubkey).
///
/// Both halves are exposed because callers need the verifying key to publish a
/// `pubkeyId` while keeping the signing key private for emitting signatures.
pub struct Keypair {
    /// Secret signing key (32 bytes). NEVER serialized to disk in plaintext.
    pub signing: SigningKey,
    /// Public verifying key (32 bytes). Safe to publish.
    pub verifying: VerifyingKey,
}

/// Mint a fresh, cryptographically random Ed25519 keypair.
///
/// Uses `OsRng` so entropy comes from the OS CSPRNG; do not call this in a
/// deterministic context.
pub fn generate_keypair() -> Keypair {
    let signing = SigningKey::generate(&mut OsRng);
    let verifying = signing.verifying_key();
    Keypair { signing, verifying }
}

/// Sign `canonical_bytes` with `signing`, producing a 64-byte detached Ed25519 signature.
pub fn sign(canonical_bytes: &[u8], signing: &SigningKey) -> Vec<u8> {
    signing.sign(canonical_bytes).to_vec()
}

/// Verify a 64-byte detached `signature` over `canonical_bytes` against `verifying`.
///
/// Fail-closed: any malformed signature, mismatched bytes, or wrong pubkey yields `false`.
pub fn verify(signature: &[u8], canonical_bytes: &[u8], verifying: &VerifyingKey) -> bool {
    match Signature::from_slice(signature) {
        Ok(sig) => verifying.verify(canonical_bytes, &sig).is_ok(),
        Err(_) => false,
    }
}

/// Short stable identifier for a verifying key: the first 16 hex chars of the pubkey.
///
/// This is a display/prefix identifier only — it is NOT a security primitive (collisions
/// are not a binding guarantee). The full pubkey is required for signature verification.
pub fn pubkey_id(verifying: &VerifyingKey) -> String {
    let full = hex::encode(verifying.to_bytes());
    full[..16].to_string()
}

/// Build the canonical byte representation of a verdict record for signing.
///
/// Per design D7 + `signed-verdict-record/spec.md`: a `BTreeMap<&str, Value>` is used so
/// serde_json emits keys in lexicographic order with no whitespace. The exact field set is
/// `{goalId, notes, registeredAt, round, status, verifierId}` — BTreeMap orders them
/// `goalId < notes < registeredAt < round < status < verifierId`.
///
/// Every field that must be bound by the signature is present: changing status, verifierId,
/// round, goalId, registeredAt, or notes produces a distinct byte sequence and therefore a
/// signature mismatch.
pub fn canonical_record_bytes(
    status: &str,
    notes: Option<&str>,
    registered_at: &str,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
) -> Vec<u8> {
    let mut map: BTreeMap<&str, Value> = BTreeMap::new();
    map.insert("goalId", Value::String(goal_id.to_string()));
    map.insert(
        "notes",
        notes.map(|n| Value::String(n.to_string())).unwrap_or(Value::Null),
    );
    map.insert("registeredAt", Value::String(registered_at.to_string()));
    map.insert("round", Value::Number(round.into()));
    map.insert("status", Value::String(status.to_string()));
    map.insert("verifierId", Value::String(verifier_id.to_string()));
    // `to_vec` produces compact JSON with no whitespace and BTreeMap iterates in sorted
    // key order — both guarantees relied on by the canonical-bytes tests.
    let mut bytes = serde_json::to_vec(&map).expect("serializing a flat BTreeMap<&str, Value> cannot fail");
    // D7 "no whitespace" is total: the canonical form must contain zero 0x20 bytes *anywhere*,
    // including inside string values (e.g. free-form `notes`). Escape ASCII space as the
    // 6-byte JSON escape `\u0020`. This is valid JSON (a parser decodes it back to a space),
    // and because the field set is closed and keys are fixed camelCase (no spaces), the
    // replacement only ever lands inside string values — never between tokens. The result is
    // a whitespace-invariant canonical form: the same logical record always yields the same
    // bytes regardless of incidental spacing in `notes`.
    bytes = escape_spaces_in_json(bytes);
    bytes
}

/// Replace every 0x20 byte in a compact JSON byte stream with the 6-byte escape `\u0020`.
///
/// Safe here because the input is a compact serde_json document over a closed, fixed-key
/// `BTreeMap` whose keys contain no spaces; numeric/structural bytes never carry 0x20, so
/// the only bytes affected are inside string values. Producing a fully whitespace-free
/// canonical form is required by D7.
fn escape_spaces_in_json(bytes: Vec<u8>) -> Vec<u8> {
    // Single pass: count spaces, then rebuild in place if any are present.
    let space_count = bytes.iter().filter(|&&b| b == b' ').count();
    if space_count == 0 {
        return bytes;
    }
    let mut out = Vec::with_capacity(bytes.len() + space_count * 5);
    for &b in &bytes {
        if b == b' ' {
            out.extend_from_slice(b"\\u0020");
        } else {
            out.push(b);
        }
    }
    out
}

/// Decode a 64-char hex string into an Ed25519 `SigningKey` (32 secret bytes).
///
/// Fail-closed on malformed hex or wrong byte length — returns `CryptoError` rather
/// than panicking on a bad-length slice.
pub fn signing_key_from_hex(hex_str: &str) -> Result<SigningKey, CryptoError> {
    let bytes = hex::decode(hex_str)?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| CryptoError::BadLength)?;
    // SigningKey::from_bytes infallibly wraps a 32-byte secret; keep the error path for
    // API symmetry with verifying_key_from_hex and future key-validation hooks.
    Ok(SigningKey::from_bytes(&arr))
}

/// Decode a 64-char hex string into an Ed25519 `VerifyingKey` (32 public bytes).
///
/// Fail-closed on malformed hex, wrong length, or non-canonical pubkey encoding.
pub fn verifying_key_from_hex(hex_str: &str) -> Result<VerifyingKey, CryptoError> {
    let bytes = hex::decode(hex_str)?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| CryptoError::BadLength)?;
    VerifyingKey::from_bytes(&arr).map_err(CryptoError::InvalidKey)
}

/// Encode an Ed25519 `SigningKey` as 64 lowercase hex chars.
pub fn signing_key_to_hex(signing: &SigningKey) -> String {
    hex::encode(signing.to_bytes())
}

/// Encode an Ed25519 `VerifyingKey` as 64 lowercase hex chars.
pub fn verifying_key_to_hex(verifying: &VerifyingKey) -> String {
    hex::encode(verifying.to_bytes())
}
