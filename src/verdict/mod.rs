//! `verifier-verdict` (jewije) logic (tasks.md §7, verdict-registration spec).
//!
//! Verifiers register their verdict exclusively by writing a per-slot `verdict.json`
//! atomically. The slot is `<store-root>/goals/<goalId>/rounds/<round>/<verifierId>/`.
//!
//! Semantics:
//!   * `approve`            → write `{status:"APPROVE", registeredAt}`.
//!   * `reject --notes`     → write `{status:"REJECT", notes, registeredAt}`.
//!   * reject w/o notes     → refused, no write.
//!   * first verdict final  → a non-null verdict is never overwritten (D4).
//!   * null baseline        → a spawn-time pre-created `{status:null}` is overwritten by
//!                            the first real verdict (null is not a verdict, only a placeholder).
//!   * fail-closed          → NULL never becomes APPROVE (D9).
//!
//! Identity (goalId / verifierId / round) is resolved by the CLI from `VERIFIER_LOOP_*`
//! env (D2); the core functions take them as explicit args.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::goal;
use crate::store;

/// On-disk per-verifier verdict filename (mirrors `spawn::VERDICT_FILE`).
pub const VERDICT_FILE: &str = "verdict.json";

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
            _ => Err(serde::de::Error::custom("verdict status must be a string or null")),
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
}

/// Compute the on-disk path for a verifier's verdict file.
///
/// `<root>/goals/<goalId>/rounds/<round>/<verifierId>/verdict.json`
/// (matches the spawn layer's directory layout — `rounds/<round>/<vid>`).
pub fn verdict_path(root: &Path, goal_id: &str, verifier_id: &str, round: u32) -> PathBuf {
    goal::goal_dir(root, goal_id)
        .join(goal::ROUNDS_DIR)
        .join(round.to_string())
        .join(verifier_id)
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
        });
    }
    let raw = fs::read_to_string(&path)?;
    let rec: VerdictRecord = serde_json::from_str(&raw)?;
    Ok(rec)
}

/// Register an APPROVE verdict in the given slot (atomic first-write-wins).
pub fn register_approve(
    root: &Path,
    goal_id: &str,
    verifier_id: &str,
    round: u32,
) -> Result<(), VerdictError> {
    let record = VerdictRecord {
        status: VerdictStatus::Approve,
        notes: None,
        registered_at: Some(Utc::now().to_rfc3339()),
    };
    write_first_verdict(root, goal_id, verifier_id, round, &record)
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
        assert_eq!(
            serde_json::to_string(&VerdictStatus::Null).unwrap(),
            "null"
        );
    }

    #[test]
    fn status_round_trips() {
        for s in [VerdictStatus::Approve, VerdictStatus::Reject, VerdictStatus::Null] {
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
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(!j.contains("notes"), "{j}");
        assert!(!j.contains("registeredAt"), "{j}");
    }

    #[test]
    fn verdict_path_layout_matches_spawn_layer() {
        let p = verdict_path(Path::new("/tmp/r"), "g1", "v2", 3);
        assert_eq!(p, Path::new("/tmp/r/goals/g1/rounds/3/v2"));
    }
}
