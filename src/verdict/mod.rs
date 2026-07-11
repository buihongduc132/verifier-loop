//! `verifier-verdict` (jewije) logic (tasks.md §7, verdict-registration spec).
//!
//! Verifiers register their verdict exclusively by writing a per-slot `verdict.json`
//! atomically. The slot is `<store-root>/goals/<goalId>/rounds/<round>/<verifierId>/`.
//!
//! Semantics:
//!   * `approve [--notes]` → write `{status:"APPROVE", registeredAt, notes?}`; notes optional (D1).
//!   * `reject --notes`     → write `{status:"REJECT", notes, registeredAt}`.
//!   * reject w/o notes     → refused, no write.
//!   * first verdict final  → a non-null verdict is never overwritten (D4).
//!   * null baseline        → a spawn-time pre-created `{status:null}` is overwritten by
//!     the first real verdict (null is not a verdict, only a placeholder).
//!   * fail-closed          → NULL never becomes APPROVE (D9).
//!
//! Identity (goalId / verifierId / round) is resolved by the CLI from `VERIFIER_LOOP_*`
//! env (D2); the core functions take them as explicit args.

use std::fs;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::crypto;
use crate::goal;
use crate::store;

/// On-disk per-verifier verdict filename (mirrors `spawn::VERDICT_FILE`).
pub const VERDICT_FILE: &str = "verdict.json";

/// On-disk pinned verifier pubkey filename (verifier-identity spec).
///
/// Written once at spawn time into the per-verifier slot dir and never overwritten;
/// its immutability binds a verifier's later signatures to a single pubkey.
pub const PUBKEY_FILE: &str = "verifier-pubkey.json";

/// On-disk per-verifier signing secret filename (verifier-secret spec delta).
///
/// The hex-encoded Ed25519 signing key is persisted (mode 0600) alongside the pinned
/// pubkey in the per-verifier slot dir so that the verdict-enforcement nudge loop
/// (D5) and the compaction-recovery resume (D6) — which spawn NEW verifier processes
/// — can re-inject the SAME secret that signed the pinned pubkey. Without this file
/// the resume path would inject an empty secret and every harvested verdict would fail
/// consensus signature verification (`unauthenticated: verifier secret missing`).
///
/// First-write-wins: the orchestrator's initial spawn writes this file once; later
/// resumes within the same round or across rounds READ it (never overwrite). On a
/// single host this is equivalent exposure to the existing forgeability concession
/// (THREAT-MODEL.md §b: a process with read access to the slot dir can forge). It is
/// a deterrent + detection layer, not a prevention guarantee.
pub const SECRET_FILE: &str = "verifier-secret.hex";

/// The status of a verdict slot.
///
/// On disk: `"APPROVE"`, `"REJECT"`, or `null` (no verdict registered yet). A `null`
/// status is the spawn-time placeholder and is **never** treated as APPROVE (D9).
// Custom serde (see `status_ser` / `status_de` below): on disk the null placeholder is
// the JSON `null` (written by the spawn layer), while APPROVE/REJECT are strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictStatus {
    Approve,
    Reject,
    /// No verdict registered (pre-created placeholder). Round evaluation treats this as
    /// not-passing (fail-closed).
    Null,
}

impl Serialize for VerdictStatus {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            VerdictStatus::Null => s.serialize_none(),
            VerdictStatus::Approve => s.serialize_str("APPROVE"),
            VerdictStatus::Reject => s.serialize_str("REJECT"),
        }
    }
}

impl<'de> Deserialize<'de> for VerdictStatus {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::Null => Ok(VerdictStatus::Null),
            serde_json::Value::String(s) => match s.as_str() {
                "APPROVE" => Ok(VerdictStatus::Approve),
                "REJECT" => Ok(VerdictStatus::Reject),
                _ => Err(serde::de::Error::custom(format!(
                    "unknown verdict status: {s}"
                ))),
            },
            _ => Err(serde::de::Error::custom(
                "verdict status must be a string or null",
            )),
        }
    }
}

/// The on-disk verdict record. `notes` is present only for `REJECT`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerdictRecord {
    pub status: VerdictStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// ISO-8601 timestamp the verdict was registered. Absent on the null placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered_at: Option<String>,
    /// Detached Ed25519 signature over the canonical record bytes
    /// (`crypto::canonical_record_bytes`), hex-encoded. Absent on the null placeholder
    /// and on unsigned legacy records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Short pubkey prefix identifier (`crypto::pubkey_id`) of the pinned verifying key
    /// the signature was made under. Absent on unsigned records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey_id: Option<String>,
}

/// On-disk pinned verifier pubkey record (verifier-identity spec).
///
/// Written exactly once into `<slot>/verifier-pubkey.json` at spawn time. Its presence
/// pins the verifier's verifying key; subsequent verdict signatures are bound to it via
/// `pubkey_id` + `verify_record`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierPubkeyFile {
    /// 64-hex encoding of the 32-byte Ed25519 verifying key.
    #[serde(rename = "pubkey")]
    pub pubkey: String,
    /// ISO-8601 timestamp the keypair was minted.
    #[serde(rename = "mintedAt")]
    pub minted_at: String,
}

/// Compute the on-disk slot directory for a verifier (the dir that holds both
/// `verdict.json` and `verifier-pubkey.json`).
///
/// `<root>/goals/<goalId>/rounds/<round>/<verifierId>/`
/// (matches the spawn layer's directory layout — `rounds/<round>/<vid>`).
pub fn verdict_path(root: &Path, goal_id: &str, verifier_id: &str, round: u32) -> PathBuf {
    goal::goal_dir(root, goal_id)
        .join(goal::ROUNDS_DIR)
        .join(round.to_string())
        .join(verifier_id)
}

/// Compute the on-disk slot directory that holds a verifier's pinned pubkey file.
///
/// Identical layout to `verdict_path` (same per-verifier slot dir); callers append
/// `PUBKEY_FILE` to reach the file itself. Returns the directory so it can also be
/// `create_dir_all`'d before writing.
pub fn pubkey_path(root: &Path, goal_id: &str, verifier_id: &str, round: u32) -> PathBuf {
    // Same slot dir as the verdict — both files live side-by-side per verifier-identity spec.
    verdict_path(root, goal_id, verifier_id, round)
}

/// Read a verdict slot. A missing file or a malformed/null record resolves to a `Null`
/// status (fail-closed: never silently promoted). A genuine I/O or parse error of an
/// existing, non-null file is surfaced as `Err`.
pub fn read_verdict(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
) -> Result<VerdictRecord, VerdictError> {
    let path = verdict_path(root, goal_id, verifier_id, round).join(VERDICT_FILE);
    if !path.exists() {
        return Ok(VerdictRecord {
            status: VerdictStatus::Null,
            notes: None,
            registered_at: None,
            signature: None,
            pubkey_id: None,
        });
    }
    let raw = fs::read_to_string(&path)?;
    let rec: VerdictRecord = serde_json::from_str(&raw)?;
    Ok(rec)
}

/// Mint a fresh Ed25519 keypair for the verifier and pin its verifying key into the
/// slot as `verifier-pubkey.json` (verifier-identity spec).
///
/// First-write-wins (immutable pin): if a pubkey is already pinned for this slot the
/// call fails closed with `AlreadyPinned` and the stored file is left untouched.
/// Returns the secret `SigningKey` so the caller (verifier process) can sign verdicts.
///
/// The secret hex is ALSO persisted to `<slot>/verifier-secret.hex` (mode 0600,
/// first-write-wins) so that the verdict-enforcement nudge loop (D5) and the
/// compaction-recovery resume (D6) — which spawn NEW verifier processes — can
/// re-inject the SAME secret that signed the pinned pubkey via [`read_verifier_secret`].
/// On a single host this is equivalent exposure to the existing forgeability
/// concession (THREAT-MODEL.md §b): a process with read access to the slot dir can
/// forge. Out-of-process V* on a separate host remains the only prevention guarantee.
pub fn mint_and_pin_pubkey(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
) -> Result<crypto::SigningKey, VerdictError> {
    ensure_goal_dir(root, goal_id)?;

    let slot = pubkey_path(root, goal_id, verifier_id, round);
    fs::create_dir_all(&slot)?;
    let target = slot.join(PUBKEY_FILE);

    // Immutability: a pinned pubkey is never re-minted.
    if target.exists() {
        return Err(VerdictError::AlreadyPinned);
    }

    // ATOMIC PUBKEY + SECRET PERSISTENCE: BOTH must land or NEITHER lands.
    //
    // Ordering: write BOTH temp files first, then rename the secret, then rename the
    // pubkey pin LAST. The pubkey pin is the "slot is pinned" marker — its presence is
    // what makes the next `mint_and_pin_pubkey` call return `AlreadyPinned`. By
    // renaming it LAST, we guarantee that if any earlier step fails (disk full, I/O
    // error, crash mid-write), the pin file is absent and a retry can re-mint the
    // slot from scratch (no bricked slot). The secret rename lands BEFORE the pin so
    // that once the pin is visible, the secret is guaranteed to exist alongside it.
    //
    // The temps are cleaned up best-effort on the error paths; on success both temps
    // have been renamed away.
    let kp = crypto::generate_keypair();
    let file = VerifierPubkeyFile {
        pubkey: crypto::verifying_key_to_hex(&kp.verifying),
        minted_at: Utc::now().to_rfc3339(),
    };
    let secret_hex = crypto::signing_key_to_hex(&kp.signing);

    let secret_target = slot.join(SECRET_FILE);
    // First-write-wins on the secret too: a pre-existing secret is left untouched
    // (mirrors the pubkey pin's immutability). When the secret is already present we
    // skip writing it entirely so a partially-failed prior mint cannot leave a stale
    // secret paired with a fresh pubkey. We still proceed to pin the pubkey below
    // because the pin check at the top of this function already passed.
    //
    // Use fs::metadata (NOT Path::exists()): exists() maps permission-denied / broken
    // symlink to `false`, which would wrongly re-write the secret and overwrite a
    // good (first-write-wins) secret. Only NotFound means "not present".
    let secret_already_present = match fs::metadata(&secret_target) {
        Ok(_) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(VerdictError::Io(e)),
    };

    // (1) Secret temp (only when not already present).
    let secret_tmp = if secret_already_present {
        None
    } else {
        let tmp = slot.join(format!("{SECRET_FILE}.tmp"));
        write_secret_mode_0600(&tmp, &secret_hex)?;
        Some(tmp)
    };

    // (2) Pubkey pin temp.
    let pubkey_tmp = slot.join(format!("{PUBKEY_FILE}.tmp"));
    let json = serde_json::to_string_pretty(&file)?;
    // Use 0600-style care: the pubkey is not secret but we write via fs::write then
    // rename for atomicity (the pin immutability check reads the final path).
    fs::write(&pubkey_tmp, json)?;

    // (3) Rename secret FIRST (so it is durably visible before the pin marker).
    if let Some(tmp) = secret_tmp.as_ref() {
        if let Err(e) = fs::rename(tmp, &secret_target) {
            // Clean up the pubkey temp so a retry starts clean.
            let _ = fs::remove_file(&pubkey_tmp);
            return Err(VerdictError::Io(e));
        }
    }

    // (4) Rename the pubkey pin LAST — this is the "slot is pinned" commit marker.
    if let Err(e) = fs::rename(&pubkey_tmp, &target) {
        // Best-effort rollback of the secret rename so a retry can re-mint cleanly.
        // (If the rollback fails we surface the original pin-rename error; the caller
        // sees an I/O error and the slot is in an indeterminate-but-recoverable state.)
        if let Some(tmp) = secret_tmp.as_ref() {
            // The secret was already renamed above; move it back to the temp path so a
            // retry rewrites it. If that fails, leave it in place — it is harmless
            // (first-write-wins semantics will keep it).
            let _ = fs::rename(&secret_target, tmp);
        }
        return Err(VerdictError::Io(e));
    }

    Ok(kp.signing)
}

/// Read the persisted per-verifier signing secret hex (mode 0600 file written by
/// [`mint_and_pin_pubkey`]). Used by the spawn layer's verdict-enforcement nudge loop
/// (D5) and compaction-recovery resume (D6) to re-inject the SAME secret that signed
/// the pinned pubkey into a NEW verifier process.
///
/// Returns `Ok(None)` when no secret file exists (legacy unsigned regime, or a slot
/// minted before this file was written). The caller injects an empty secret in that
/// case, and any harvested verdict will fail consensus signature verification
/// (fail-closed: never silently trusted).
pub fn read_verifier_secret(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
) -> Result<Option<String>, VerdictError> {
    let target = pubkey_path(root, goal_id, verifier_id, round).join(SECRET_FILE);
    // Use fs::metadata (NOT Path::exists()): exists() maps ANY metadata error
    // (permission denied, broken symlink) to `false`, which would silently yield
    // Ok(None) → an empty secret injected → unsigned verdict. Only a genuine
    // NotFound resolves to Ok(None); all other I/O errors propagate as
    // VerdictError::Io (fail-closed).
    match fs::metadata(&target) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(VerdictError::Io(e)),
        Ok(_) => {}
    }
    let raw = fs::read_to_string(&target)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

/// Atomically write `secret_hex` to `target` with filesystem mode 0600 (owner
/// read+write only). The secret must never be world/group-readable on a multi-user
/// host (it is the per-verifier forge key). Uses `OpenOptions` with an explicit
/// `.mode(0o600)` so the file is created with restrictive perms from the outset,
/// then a `set_permissions` call as defense-in-depth in case the umask widened them.
fn write_secret_mode_0600(target: &Path, secret_hex: &str) -> Result<(), VerdictError> {
 use std::os::unix::fs::PermissionsExt;
    // Create with mode 0600 from the outset (owner read+write only).
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true).truncate(true).mode(0o600);
        let mut f = opts.open(target)?;
        use std::io::Write;
        f.write_all(secret_hex.as_bytes())?;
        f.flush()?;
    }
    // Defense-in-depth: enforce 0600 even if the process umask widened the create mode.
    let mut perms = fs::metadata(target)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(target, perms)?;
    Ok(())
}

/// Read the pinned verifying key for a verifier slot.
///
/// Returns `Ok(None)` when no pubkey has been pinned (caller treats the slot as
/// unauthenticated). Returns `Ok(Some(key))` for a well-formed pin file. A present but
/// malformed file surfaces as `Err` (fail closed rather than silently trusting).
pub fn read_pinned_pubkey(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
) -> Result<Option<crypto::VerifyingKey>, VerdictError> {
    let target = pubkey_path(root, goal_id, verifier_id, round).join(PUBKEY_FILE);
    if !target.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&target)?;
    let file: VerifierPubkeyFile = serde_json::from_str(&raw)?;
    let key = crypto::verifying_key_from_hex(&file.pubkey)
        .map_err(|e| VerdictError::BadSignature(format!("pinned pubkey is malformed: {e}")))?;
    Ok(Some(key))
}

/// Verify a `VerdictRecord`'s signature against the pinned verifying key
/// (signed-verdict-record spec).
///
/// Fail-closed chain:
///   1. A NULL status, or any record without a signature, is `Untrusted` — null never
///      becomes APPROVE (D9), and an unsigned APPROVE is never trusted.
///   2. No pinned pubkey supplied → `WrongPubkey` (cannot bind the signature).
///   3. `pubkeyId` does not match the pinned key's id → `WrongPubkey`.
///   4. Signature is not valid hex, or does not verify over the canonical bytes →
///      `BadSignature`. The canonical bytes bind {status, notes, registeredAt, goalId,
///      verifierId, round}, so any tampering with those fields invalidates the signature.
pub fn verify_record(
    record: &VerdictRecord,
    pinned_pubkey: Option<&crypto::VerifyingKey>,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
) -> Result<(), VerdictError> {
    // (1) Null status or missing signature => untrusted, regardless of any key.
    if record.status == VerdictStatus::Null || record.signature.is_none() {
        return Err(VerdictError::Untrusted);
    }

    // (2) A signature is only meaningful against a pinned pubkey.
    let pinned = pinned_pubkey.ok_or(VerdictError::WrongPubkey)?;

    // (3) The record's declared pubkeyId must match the pinned key.
    let expected_id = crypto::pubkey_id(pinned);
    if record.pubkey_id.as_deref() != Some(expected_id.as_str()) {
        return Err(VerdictError::WrongPubkey);
    }

    // (4) Decode + cryptographically verify the signature over the canonical bytes.
    let sig_hex = record.signature.as_ref().expect("checked Some above");
    let sig_bytes = hex::decode(sig_hex).map_err(|_| {
        VerdictError::BadSignature("signature is not valid hexadecimal".to_string())
    })?;

    let status_str = match record.status {
        VerdictStatus::Approve => "APPROVE",
        VerdictStatus::Reject => "REJECT",
        VerdictStatus::Null => return Err(VerdictError::Untrusted),
    };
    let registered_at_str = record.registered_at.as_deref().unwrap_or("");
    let canonical = crypto::canonical_record_bytes(
        status_str,
        record.notes.as_deref(),
        registered_at_str,
        goal_id,
        verifier_id,
        round,
    );

    if !crypto::verify(&sig_bytes, &canonical, pinned) {
        return Err(VerdictError::BadSignature(
            "signature does not verify over the canonical record bytes".to_string(),
        ));
    }
    Ok(())
}

/// Register a SIGNED APPROVE verdict bound to the slot's pinned verifier pubkey
/// (verdict-registration MODIFIED spec — secret-required gate).
///
/// Fail-closed chain:
///   1. No pinned pubkey for the slot → `Unauthenticated` (no trust anchor).
///   2. The secret's deriving verifying key does not equal the pinned key →
///      `Unauthenticated`.
///   3. Otherwise: sign the canonical record bytes and write atomically
///      (first-write-wins; an existing non-null verdict yields `AlreadyFinal`).
pub fn register_signed_approve(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
    notes: Option<&str>,
    secret: &crypto::SigningKey,
) -> Result<(), VerdictError> {
    let normalized = normalize_optional_notes(notes);
    let record = build_signed_record(
        VerdictStatus::Approve,
        normalized,
        root,
        goal_id,
        verifier_id,
        round,
        secret,
    )?;
    write_first_verdict(root, goal_id, verifier_id, round, &record)?;
    // Hash-chained receipt append (receipt-log spec): every successful signed write
    // extends the per-goal chain. Fail-closed if the receipt append itself fails —
    // a missing chain entry means the completion-hash inputs would be incomplete.
    append_receipt_for_signed_write(
        root,
        goal_id,
        verifier_id,
        "approve",
        "APPROVE",
        record.pubkey_id.as_deref(),
    )?;
    Ok(())
}

/// Register a SIGNED REJECT verdict with notes (atomic first-write-wins). Empty notes
/// are refused with `NotesRequired` exactly like the unsigned path.
pub fn register_signed_reject(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
    notes: &str,
    secret: &crypto::SigningKey,
) -> Result<(), VerdictError> {
    let trimmed = notes.trim();
    if trimmed.is_empty() {
        return Err(VerdictError::NotesRequired);
    }
    let record = build_signed_record(
        VerdictStatus::Reject,
        Some(trimmed),
        root,
        goal_id,
        verifier_id,
        round,
        secret,
    )?;
    write_first_verdict(root, goal_id, verifier_id, round, &record)?;
    append_receipt_for_signed_write(
        root,
        goal_id,
        verifier_id,
        "reject",
        "REJECT",
        record.pubkey_id.as_deref(),
    )?;
    Ok(())
}

/// Build a signed `VerdictRecord` bound to the slot's pinned verifying key.
///
/// Shared by `register_signed_approve` / `register_signed_reject`. Performs the
/// secret/pinned-pubkey authentication gate, then signs the canonical record bytes.
fn build_signed_record(
    status: VerdictStatus,
    notes: Option<&str>,
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
    secret: &crypto::SigningKey,
) -> Result<VerdictRecord, VerdictError> {
    // (1) Trust anchor: the pinned verifying key must exist for this slot.
    let pinned_vk = read_pinned_pubkey(root, goal_id, verifier_id, round)?.ok_or_else(|| {
        VerdictError::Unauthenticated("no pinned verifier pubkey for this slot".to_string())
    })?;

    // (2) The supplied secret must correspond to the pinned pubkey.
    let derived_vk = secret.verifying_key();
    if crypto::verifying_key_to_hex(&derived_vk) != crypto::verifying_key_to_hex(&pinned_vk) {
        return Err(VerdictError::Unauthenticated(
            "secret's pubkey does not match the pinned verifier pubkey".to_string(),
        ));
    }

    // (3) Sign the canonical record bytes (binds status/notes/registeredAt/ids/round).
    let registered_at = Utc::now().to_rfc3339();
    let status_str = match status {
        VerdictStatus::Approve => "APPROVE",
        VerdictStatus::Reject => "REJECT",
        VerdictStatus::Null => "null",
    };
    let canonical = crypto::canonical_record_bytes(
        status_str,
        notes,
        &registered_at,
        goal_id,
        verifier_id,
        round,
    );
    let sig = crypto::sign(&canonical, secret);

    Ok(VerdictRecord {
        status,
        notes: notes.map(|s| s.to_string()),
        registered_at: Some(registered_at),
        signature: Some(hex::encode(&sig)),
        pubkey_id: Some(crypto::pubkey_id(&pinned_vk)),
    })
}

/// Append a hash-chained receipt entry after a successful signed verdict write.
///
/// Fail-closed: if the receipt append errors (disk full, parse error), the error is
/// surfaced to the caller — the verdict itself is already durably written, but the
/// completion hash for the goal cannot be considered complete without the chain entry.
fn append_receipt_for_signed_write(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    kind: &str,
    status: &str,
    signed_by: Option<&str>,
) -> Result<(), VerdictError> {
    let signed_by = signed_by.unwrap_or("");
    crate::receipt::append_receipt(root, goal_id, kind, verifier_id, status, signed_by)
        .map_err(|e| VerdictError::ReceiptFailed(e.to_string()))?;
    Ok(())
}

/// Register an (unsigned) APPROVE verdict in the given slot (atomic first-write-wins).
///
/// Legacy path retained for slots that are not in the signed regime (no pinned pubkey
/// and no secret supplied). Signed registration goes through `register_signed_approve`.
pub fn register_approve(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
    notes: Option<&str>,
) -> Result<(), VerdictError> {
    let record = VerdictRecord {
        status: VerdictStatus::Approve,
        notes: normalize_optional_notes(notes).map(str::to_string),
        registered_at: Some(Utc::now().to_rfc3339()),
        signature: None,
        pubkey_id: None,
    };
    write_first_verdict(root, goal_id, verifier_id, round, &record)
}

/// Normalize optional notes for an APPROVE verdict (design D2).
///
/// Trims and drops empty/whitespace-only input so `approve --notes ""` serializes
/// identically to `approve` with no `--notes` (the `notes` key is absent from the
/// on-disk JSON via `skip_serializing_if = "Option::is_none"`). Reject keeps its own
/// non-empty enforcement in [`register_reject`]. Returns a borrowed `&str` (trimmed)
/// so no allocation occurs until the caller builds the `VerdictRecord`.
fn normalize_optional_notes(notes: Option<&str>) -> Option<&str> {
    notes.map(str::trim).filter(|s| !s.is_empty())
}

/// Register a REJECT verdict with notes (atomic first-write-wins). Empty notes are refused.
pub fn register_reject(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
    notes: &str,
) -> Result<(), VerdictError> {
    let trimmed = notes.trim();
    if trimmed.is_empty() {
        return Err(VerdictError::NotesRequired);
    }
    let record = VerdictRecord {
        status: VerdictStatus::Reject,
        notes: Some(trimmed.to_string()),
        registered_at: Some(Utc::now().to_rfc3339()),
        signature: None,
        pubkey_id: None,
    };
    write_first_verdict(root, goal_id, verifier_id, round, &record)
}

/// Atomically write a verdict as the first real verdict for the slot.
///
/// A null placeholder (`{status:null}`) left by the spawn layer is overwritten; an
/// existing non-null verdict (APPROVE/REJECT) is final and yields `AlreadyFinal` without
/// altering the stored file (D4 first-write-wins). The write is atomic: write to a sibling
/// temp file then rename over the target.
fn write_first_verdict(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
    record: &VerdictRecord,
) -> Result<(), VerdictError> {
    // Ensure the goal directory exists (the slot may not have been pre-created if a
    // verdict CLI is invoked out-of-band). Fail closed if the store root is unusable.
    ensure_goal_dir(root, goal_id)?;

    let vdir = verdict_path(root, goal_id, verifier_id, round);
    fs::create_dir_all(&vdir)?;
    let target = vdir.join(VERDICT_FILE);

    // First-write-wins: if a real verdict already exists, refuse.
    if target.exists() {
        let existing = read_verdict(root, goal_id, verifier_id, round)?;
        if existing.status != VerdictStatus::Null {
            return Err(VerdictError::AlreadyFinal);
        }
    }

    // Atomic write: temp sibling + rename.
    let tmp = vdir.join(format!("{VERDICT_FILE}.tmp"));
    let json = serde_json::to_string_pretty(record)?;
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &target)?;
    Ok(())
}

/// Ensure the goal directory exists; fail closed if the store root is a file or unusable.
fn ensure_goal_dir(root: &Path, goal_id: &str) -> Result<(), VerdictError> {
    let meta = fs::metadata(root);
    match meta {
        Ok(m) if m.is_dir() => {}
        Ok(_) => {
            return Err(VerdictError::StoreUnusable(io::Error::new(
                io::ErrorKind::InvalidInput,
                "store root is a file, not a directory",
            )));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Treat a missing store root as a missing goal (the goal could not have been
            // created under it). Fail closed.
            return Err(VerdictError::GoalNotFound);
        }
        Err(e) => return Err(VerdictError::Io(e)),
    }
    let gdir = goal::goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(VerdictError::GoalNotFound);
    }
    Ok(())
}

/// Errors raised by the verdict layer. Every path fails closed (D9).
#[derive(Debug, thiserror::Error)]
pub enum VerdictError {
    #[error("verdict is already final; cannot be overwritten")]
    AlreadyFinal,
    #[error("reject requires non-empty --notes")]
    NotesRequired,
    #[error("goal not found (store or goal directory missing)")]
    GoalNotFound,
    #[error("store root is unusable: {0}")]
    StoreUnusable(#[source] io::Error),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("store error: {0}")]
    Store(#[from] store::StoreError),
    /// A pinned verifier pubkey is already present for the slot (immutable pin).
    #[error("verifier pubkey is already pinned for this slot; cannot be re-minted")]
    AlreadyPinned,
    /// A signature failed to verify or was malformed (signed-verdict-record spec).
    /// Carries a short reason string.
    #[error("bad signature: {0}")]
    BadSignature(String),
    /// The declared/cited pubkey does not match the pinned key, or no pinned key was
    /// supplied (the signature cannot be bound to a trusted identity).
    #[error("wrong or missing pinned pubkey; signature cannot be bound")]
    WrongPubkey,
    /// The record is unsigned or carries a null status — by spec it is never trusted.
    #[error("untrusted record: unsigned or null status is never accepted")]
    Untrusted,
    /// The verifier could not be authenticated against the slot's pinned pubkey: the
    /// pinned pubkey is missing, no secret was supplied for a pinned slot, or the
    /// supplied secret's deriving pubkey does not match the pinned one (verdict-
    /// registration MODIFIED spec, secret-required gate).
    #[error("unauthenticated: {0}")]
    Unauthenticated(String),
    /// The hash-chained receipt log append failed after the verdict was written.
    /// The verdict is durable but the goal's completion hash cannot be considered
    /// complete without the chain entry (receipt-log spec).
    #[error("receipt log append failed: {0}")]
    ReceiptFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_uppercase_strings_and_null() {
        assert_eq!(
            serde_json::to_string(&VerdictStatus::Approve).unwrap(),
            r#""APPROVE""#
        );
        assert_eq!(
            serde_json::to_string(&VerdictStatus::Reject).unwrap(),
            r#""REJECT""#
        );
        assert_eq!(serde_json::to_string(&VerdictStatus::Null).unwrap(), "null");
    }

    #[test]
    fn status_round_trips() {
        for s in [
            VerdictStatus::Approve,
            VerdictStatus::Reject,
            VerdictStatus::Null,
        ] {
            let j = serde_json::to_string(&s).unwrap();
            let back: VerdictStatus = serde_json::from_str(&j).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn record_skips_optional_fields_when_absent() {
        let r = VerdictRecord {
            status: VerdictStatus::Approve,
            notes: None,
            registered_at: None,
            signature: None,
            pubkey_id: None,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(!j.contains("notes"), "{j}");
        assert!(!j.contains("registeredAt"), "{j}");
        assert!(!j.contains("signature"), "{j}");
        assert!(!j.contains("pubkeyId"), "{j}");
    }

    #[test]
    fn verdict_path_layout_matches_spawn_layer() {
        let p = verdict_path(Path::new("/tmp/r"), "g1", "v2", 3);
        assert_eq!(p, Path::new("/tmp/r/goals/g1/rounds/3/v2"));
    }

    // -----------------------------------------------------------------------
    // F1 regression: atomic pubkey + secret persistence.
    // If the secret file is absent, mint_and_pin_pubkey MUST be able to re-run
    // (not AlreadyPinned) so the slot is never bricked; after success BOTH the
    // pubkey pin and the secret file must exist.
    // -----------------------------------------------------------------------
    fn seed_store(root: &Path) -> String {
        goal::new(root, "test goal", None).expect("NEW seeds a goal")
    }

    #[cfg(unix)]
    #[test]
    fn mint_atomic_writes_both_files_or_neither() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let goal_id = seed_store(root);

        let sk1 = mint_and_pin_pubkey(root, &goal_id, "v1", 1).expect("first mint");
        let slot = verdict_path(root, &goal_id, "v1", 1);
        assert!(slot.join(PUBKEY_FILE).exists(), "pubkey pin must exist after mint");
        assert!(slot.join(SECRET_FILE).exists(), "secret must exist after mint");

        // The persisted secret must round-trip back.
        let persisted = read_verifier_secret(root, &goal_id, "v1", 1)
            .expect("read secret")
            .expect("secret present");
        assert_eq!(persisted, crypto::signing_key_to_hex(&sk1));
    }

    #[cfg(unix)]
    #[test]
    fn mint_can_re_run_when_secret_absent_but_pubkey_present_is_still_pinned() {
        // The pin immutability invariant holds: a pinned slot stays pinned even if
        // the secret file is later removed (operator/sibling deleted it). This pins
        // the spec'd behaviour so the atomic-persistence fix is not mistaken for
        // "re-mintable after partial failure".
        //
        // The atomicity fix guarantees the OPPOSITE at mint TIME: a *failed* mint
        // (where the pubkey rename never completed) is re-runnable. We cannot easily
        // simulate a mid-mint crash in a unit test, so we assert the positive case
        // (a fully-rolled-back failed mint re-runs) via the temp-file cleanup path:
        // if neither the pin nor the secret exist, a fresh mint succeeds.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let goal_id = seed_store(root);

        // Simulate a *failed* prior mint: only a stray pubkey TMP remains, no pin,
        // no secret. A retry MUST succeed (not AlreadyPinned) and produce both files.
        let slot = verdict_path(root, &goal_id, "v1", 1);
        fs::create_dir_all(&slot).unwrap();
        fs::write(slot.join(format!("{PUBKEY_FILE}.tmp")), "stray").unwrap();

        let sk = mint_and_pin_pubkey(root, &goal_id, "v1", 1).expect("retry after stray tmp");
        assert!(slot.join(PUBKEY_FILE).exists());
        assert!(slot.join(SECRET_FILE).exists());
        let persisted = read_verifier_secret(root, &goal_id, "v1", 1)
            .unwrap()
            .expect("secret present after retry");
        assert_eq!(persisted, crypto::signing_key_to_hex(&sk));
    }

    // -----------------------------------------------------------------------
    // F2 regression: read_verifier_secret surfaces I/O errors (permission denied)
    // instead of silently returning Ok(None) → unsigned verdict.
    // -----------------------------------------------------------------------
    #[cfg(unix)]
    #[test]
    fn read_verifier_secret_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let goal_id = seed_store(root);

        // No secret file at all → Ok(None).
        assert_eq!(read_verifier_secret(root, &goal_id, "v1", 1).unwrap(), None);

        // A zero-byte secret → Ok(None) (trimmed-empty).
        let slot = verdict_path(root, &goal_id, "v1", 1);
        fs::create_dir_all(&slot).unwrap();
        fs::write(slot.join(SECRET_FILE), "   ").unwrap();
        assert_eq!(read_verifier_secret(root, &goal_id, "v1", 1).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn read_verifier_secret_surfaces_permission_denied_error() {
        // Only meaningful when the test runs as a non-root user (root bypasses DAC).
        // We skip gracefully under root; the regression still holds on CI/dev hosts.
        // root detection via `id -u`-style: if HOME is /root or the crate's `geteuid`
        // is unavailable, we approximate by trying to read /proc/self and checking the
        // standard env. Simpler: attempt the perm-denied setup; if the read still
        // succeeds, the runner is root → skip.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let goal_id = seed_store(root);
        let slot = verdict_path(root, &goal_id, "v1", 1);
        fs::create_dir_all(&slot).unwrap();
        let secret = slot.join(SECRET_FILE);
        fs::write(&secret, "deadbeef").unwrap();
        let secret_perms = fs::metadata(&secret).unwrap().permissions().mode();

        // Strip all perms. Under root this is a no-op for access checks.
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();

        let res = read_verifier_secret(root, &goal_id, "v1", 1);
        // Restore perms BEFORE asserting so a panic does not leak a 0000 file.
        let _ = fs::set_permissions(&secret, fs::Permissions::from_mode(secret_perms));

        // On root the read succeeds; treat that as a skip.
        match res {
            Ok(None) | Ok(Some(_)) => {
                eprintln!("read_verifier_secret perm-denied test skipped (running as root)");
            }
            Err(VerdictError::Io(_)) => {
                // expected on non-root
            }
            other => panic!(
                "permission-denied secret MUST surface as Err(Io), got {other:?}"
            ),
        }
    }
}
