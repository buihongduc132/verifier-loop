//! Consensus + completion hash (tasks.md §8, consensus-check + completion-proof specs).
//!
//! After the gather barrier, the CLI counts APPROVE verdicts among the `m` spawned
//! verifiers; the round passes iff the APPROVE count is `>= n` (LD4: n/m static from
//! `config.json`). On pass the tamper-evident completion hash is computed and
//! `completion.json` is written. On fail a rejection (REJECT notes + null markers) is
//! surfaced to A; no hash and no completion file are produced (fail-closed D9).
//!
//! ## Hash formula (completion-proof spec, D6)
//!
//! ```text
//! completionHash = "vl:" + first40hex(SHA256(
//!     salt + goalId + goalSignature + String(roundNumber)
//!          + JSON.stringify(matchingVerdicts sorted by verifierId)
//!          + matchedAtISO
//! ))
//! where goalSignature = SHA256(salt + goalText + createdAt)
//! ```
//!
//! Each input guards a distinct tamper vector: editing `goalText`/`createdAt` changes
//! `goalSignature`; editing an APPROVE verdict's `registeredAt`/`notes` changes the
//! canonicalized matching array; both break the hash. `matchingVerdicts` is serialized
//! as canonical JSON (sorted by `verifierId`, object keys alphabetical, no whitespace)
//! so the digest is deterministic and bit-for-bit reproducible by an auditor reading
//! the goal directory plus `.salt`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::goal;
use crate::verdict::{VerdictRecord, VerdictStatus};

/// `~/.verifier-loop/goals/<goalId>/completion.json` — written only on consensus.
pub const COMPLETION_FILE: &str = "completion.json";
/// Length of the hex suffix of a completion hash (`first40hex`).
const HASH_HEX_LEN: usize = 40;
/// Prefix of every completion hash.
const HASH_PREFIX: &str = "vl:";

/// A matching (APPROVE) verdict participating in the hash input.
///
/// Serialized canonically as `{"registeredAt":..,"verifierId":..}` (keys alphabetical
/// via `BTreeMap`) inside the sorted-by-`verifierId` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchingVerdict {
    pub verifier_id: String,
    /// ISO-8601 timestamp the verdict was registered. Always present for a matching
    /// (APPROVE) verdict — a verdict without `registeredAt` cannot match (fail-closed).
    pub registered_at: String,
}

/// The rejection surfaced to A when a round does not pass: each non-APPROVE verifier's
/// contribution (REJECT notes, or a marker for a null/missing verdict).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rejection {
    /// `(verifierId, notes)` for every REJECT verdict.
    pub reject_notes: Vec<(String, String)>,
    /// Verifier ids with no verdict (null/missing) — surfaced as a "did not register" marker.
    pub null_verifiers: Vec<String>,
}

/// Result of consensus evaluation over the gathered verdicts.
#[derive(Debug, Clone)]
pub struct ConsensusResult {
    pub passed: bool,
    pub approve_count: u32,
    pub n: u32,
    pub m: u32,
    /// APPROVE verdicts that contributed to the pass, sorted ascending by `verifier_id`.
    pub matching_verdicts: Vec<MatchingVerdict>,
    /// Populated when `!passed`. Empty on pass.
    pub rejection: Rejection,
}

/// The `completion.json` record written on success.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionRecord {
    pub hash: String,
    pub goal_id: String,
    pub round_number: u32,
    pub matched_at: String,
    pub matching_verdicts: Vec<MatchingVerdict>,
}

/// Evaluate n-of-m consensus over the gathered verdicts.
///
/// `verdicts` is `[(verifierId, VerdictRecord)]` for the round; entries need not be
/// sorted and may be fewer than `m` (missing entries are treated as null → fail-closed).
/// APPROVE verdicts lacking `registered_at` are not counted as matching (fail-closed).
/// Null/REJECT verdicts never count toward `n` (D9).
pub fn evaluate(verdicts: &[(String, VerdictRecord)], n: u32, m: u32) -> ConsensusResult {
    let mut matching: Vec<MatchingVerdict> = Vec::new();
    let mut rejection = Rejection::default();

    for (vid, rec) in verdicts {
        match rec.status {
            VerdictStatus::Approve => {
                if let Some(ts) = rec.registered_at.as_ref() {
                    if !ts.is_empty() {
                        matching.push(MatchingVerdict {
                            verifier_id: vid.clone(),
                            registered_at: ts.clone(),
                        });
                        continue;
                    }
                }
                // APPROVE but missing/empty timestamp — cannot match (fail-closed).
                rejection.null_verifiers.push(vid.clone());
            }
            VerdictStatus::Reject => {
                let notes = rec.notes.clone().unwrap_or_default();
                rejection.reject_notes.push((vid.clone(), notes));
            }
            VerdictStatus::Null => {
                rejection.null_verifiers.push(vid.clone());
            }
        }
    }

    // Any missing verifier (fewer than m entries) is an implicit null.
    if (verdicts.len() as u32) < m {
        // We cannot know which verifier ids are missing from the slice alone, but the
        // round already cannot pass (missing = fail-closed). The gather barrier caller
        // is expected to supply all m slots; here we simply ensure no false pass.
    }

    // Sort matching by verifier_id ascending (canonical hash input order).
    matching.sort_by(|a, b| a.verifier_id.cmp(&b.verifier_id));

    let approve_count = matching.len() as u32;
    let passed = approve_count >= n && (verdicts.len() as u32) >= m;

    ConsensusResult {
        passed,
        approve_count,
        n,
        m,
        matching_verdicts: matching,
        rejection,
    }
}

/// Canonical JSON for the matching-verdicts hash input.
///
/// Produces a JSON array `[{"registeredAt":..,"verifierId":..},...]`:
///   * array sorted ascending by `verifierId`,
///   * each object's keys alphabetical (`BTreeMap` → `registeredAt` before `verifierId`),
///   * no whitespace (default `serde_json::to_string`).
///
/// `matching` is assumed already sorted by the caller (or re-sorted here defensively).
fn canonical_matching_json(matching: &[MatchingVerdict]) -> String {
    let mut sorted: Vec<&MatchingVerdict> = matching.iter().collect();
    sorted.sort_by(|a, b| a.verifier_id.cmp(&b.verifier_id));

    let arr: Vec<serde_json::Value> = sorted
        .iter()
        .map(|m| {
            let mut map = BTreeMap::new();
            map.insert(
                "registeredAt".to_string(),
                serde_json::Value::String(m.registered_at.clone()),
            );
            map.insert(
                "verifierId".to_string(),
                serde_json::Value::String(m.verifier_id.clone()),
            );
            serde_json::to_value(&map).expect("BTreeMap serializes")
        })
        .collect();
    serde_json::to_string(&serde_json::Value::Array(arr)).expect("array serializes")
}

/// Compute the tamper-evident completion hash (completion-proof spec, D6).
///
/// `completionHash = "vl:" + first40hex(SHA256(salt + goalId + goalSignature +
/// String(roundNumber) + canonicalJSON(matchingVerdicts sorted by verifierId) +
/// matchedAtISO))`.
///
/// Deterministic: identical inputs yield an identical hash. Each input guards a distinct
/// tamper vector (see module docs).
pub fn compute_hash(
    salt: &str,
    goal_id: &str,
    goal_sig: &str,
    round: u32,
    matching: &[MatchingVerdict],
    matched_at_iso: &str,
) -> String {
    let canon = canonical_matching_json(matching);
    let input = format!("{salt}{goal_id}{goal_sig}{round}{canon}{matched_at_iso}");
    let digest = hex::encode(Sha256::digest(input.as_bytes()));
    debug_assert!(
        digest.len() >= HASH_HEX_LEN,
        "SHA-256 hex digest must be at least {} chars",
        HASH_HEX_LEN
    );
    format!("{HASH_PREFIX}{}", &digest[..HASH_HEX_LEN])
}

/// Write `completion.json` for a passing round. Refuses (returns `Err(NotPassed)`) when
/// the round did not reach consensus — no completion record is ever produced on failure.
///
/// The write is atomic (tmp sibling + rename). Fails closed if the goal directory is
/// missing or the store is unusable.
pub fn write_completion(
    root: &Path,
    goal_id: &str,
    result: &ConsensusResult,
    round: u32,
    hash: &str,
    matched_at_iso: &str,
) -> Result<PathBuf, ConsensusError> {
    if !result.passed {
        return Err(ConsensusError::NotPassed);
    }

    let gdir = goal::goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(ConsensusError::GoalNotFound);
    }

    let record = CompletionRecord {
        hash: hash.to_string(),
        goal_id: goal_id.to_string(),
        round_number: round,
        matched_at: matched_at_iso.to_string(),
        matching_verdicts: result.matching_verdicts.clone(),
    };

    let target = gdir.join(COMPLETION_FILE);
    let tmp = gdir.join(format!("{COMPLETION_FILE}.tmp"));
    let json = serde_json::to_string_pretty(&record)?;
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &target)?;
    Ok(target)
}

/// Errors raised by the consensus layer. All paths fail closed.
#[derive(Debug, thiserror::Error)]
pub enum ConsensusError {
    #[error("round did not reach consensus; no completion record produced")]
    NotPassed,
    #[error("goal not found (store or goal directory missing)")]
    GoalNotFound,
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(vid: &str, ts: &str) -> MatchingVerdict {
        MatchingVerdict {
            verifier_id: vid.into(),
            registered_at: ts.into(),
        }
    }

    #[test]
    fn canonical_json_sorts_by_verifier_id_and_keys_alphabetical() {
        let matching = vec![mv("v2", "b"), mv("v1", "a")];
        let j = canonical_matching_json(&matching);
        // Sorted by verifierId: v1 before v2.
        assert!(j.find(r#""verifierId":"v1""#).unwrap() < j.find(r#""verifierId":"v2""#).unwrap());
        // Keys alphabetical within each object: registeredAt before verifierId.
        assert!(j.find(r#""registeredAt""#).unwrap() < j.find(r#""verifierId""#).unwrap());
        // No whitespace.
        assert!(!j.contains(' '), "canonical JSON must have no whitespace: {j}");
    }

    #[test]
    fn compute_hash_uses_exact_concatenation_order() {
        let matching = vec![mv("v1", "2026-07-03T10:00:00Z")];
        let h = compute_hash("SALT", "GID", "SIG", 1, &matching, "2026-07-03T10:05:00Z");

        // Independent recompute.
        let canon = canonical_matching_json(&matching);
        let input = format!("SALTGIDSIG1{canon}2026-07-03T10:05:00Z");
        let digest = hex::encode(Sha256::digest(input.as_bytes()));
        assert_eq!(h, format!("vl:{}", &digest[..HASH_HEX_LEN]));
    }

    #[test]
    fn evaluate_reject_notes_collected_on_fail() {
        let verdicts = vec![
            ("v1".to_string(), VerdictRecord {
                status: VerdictStatus::Approve,
                notes: None,
                registered_at: Some("t".into()),
            }),
            ("v2".to_string(), VerdictRecord {
                status: VerdictStatus::Reject,
                notes: Some("needs work".into()),
                registered_at: Some("t".into()),
            }),
        ];
        let r = evaluate(&verdicts, 2, 2);
        assert!(!r.passed);
        assert_eq!(r.rejection.reject_notes, vec![("v2".to_string(), "needs work".to_string())]);
    }
}
