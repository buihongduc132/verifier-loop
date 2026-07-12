//! Consensus + completion hash (tasks.md §8, consensus-check + completion-proof specs).
//!
//! After the gather barrier, the CLI counts APPROVE verdicts among the `m` spawned
//! verifiers; the round passes iff the APPROVE count is `>= n` (LD4: n/m static from
//! `config.json`). On pass the tamper-evident completion hash is computed and
//! `completion.json` is written. On fail a rejection (REJECT notes + null markers) is
//! surfaced to A; no hash and no completion file are produced (fail-closed D9).
//!
//! ## Hash formula (rev 2 — completion-proof spec, D6)
//!
//! ```text
//! short       = mmddyy + "-" + first8hex(SHA256(inputs))   // displayed, printed
//! fullDigest  = SHA256(inputs)                              // 64 hex, stored not printed
//! inputs      = salt + goalId + goalSignature + String(roundNumber)
//!            + canonicalJSON(matchingVerdicts sorted by verifierId) + matchedAtISO
//! mmddyy      = UTC date of matchedAt (MMDDYY)
//! goalSignature = SHA256(salt + goalText + createdAt)       // stored full in signature.json
//! ```
//!
//! The short form is the human/agent-facing ID (memorable, invokable); the full digest
//! is the deterministic tamper guard stored in `completion.json` for exact audit recompute.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::goal;
use crate::verdict::{self, VerdictRecord, VerdictStatus};

/// `~/.verifier-loop/goals/<goalId>/completion.json` — written only on consensus.
pub const COMPLETION_FILE: &str = "completion.json";
/// Length of the hex suffix of the short completion hash (`first8hex`).
const HASH_SHORT_HEX_LEN: usize = 8;
/// Length of the full SHA-256 hex digest.
const HASH_FULL_HEX_LEN: usize = 64;

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
/// contribution (REJECT notes, a marker for a null/missing verdict, or a signature
/// failure for a verdict whose signature did not verify against its pinned pubkey).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rejection {
    /// `(verifierId, notes)` for every REJECT verdict.
    pub reject_notes: Vec<(String, String)>,
    /// Verifier ids with no verdict (null/missing) — surfaced as a "did not register" marker.
    pub null_verifiers: Vec<String>,
    /// `(verifierId, reason)` for every APPROVE verdict whose signature failed to verify
    /// against the slot's pinned pubkey (BadSignature / WrongPubkey / Untrusted). Such a
    /// verdict is NOT counted toward `n` (signed-verdict-record "Consensus verifies").
    pub signature_failures: Vec<(String, String)>,
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

/// The `completion.json` record written on success (rev 2).
///
/// `hash` is the short `mmddyy-XXXXXXXX` form (displayed); `fullDigest` is the full
/// 64-hex SHA-256 digest for exact audit recompute (not printed).
///
/// `trace_id` (add-otel-observability D4) is observability metadata: a convenience
/// pivot from the completion record to the goal's span trail. It is NOT a hash
/// input — `hash`/`fullDigest` are computed from the canonical tuple only.
/// `None` when no trace id was resolved (backward-compat); omitted from JSON via
/// skip_serializing_if so old completion.json files still deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionRecord {
    pub hash: String,
    pub full_digest: String,
    pub goal_id: String,
    pub round_number: u32,
    pub matched_at: String,
    pub matching_verdicts: Vec<MatchingVerdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

/// Output of [`compute_hash`]: the short display hash + the full digest.
///
/// The short hash is what A sees and invokes; the full digest is what an auditor
/// compares for exact (non-probabilistic) tamper detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionHash {
    /// `mmddyy-XXXXXXXX` — UTC date of matchedAt + first 8 hex of the digest.
    short: String,
    /// Full 64-hex SHA-256 digest of the same inputs.
    full: String,
}

impl CompletionHash {
    /// The short display hash (`mmddyy-XXXXXXXX`).
    pub fn short_hash(&self) -> &str {
        &self.short
    }
    /// The full 64-hex SHA-256 digest (stored in `completion.json` `fullDigest`).
    pub fn full_digest(&self) -> &str {
        &self.full
    }
}

impl std::fmt::Display for CompletionHash {
    /// Displays the short form (`mmddyy-XXXXXXXX`) — what A sees and invokes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.short)
    }
}

/// Evaluate n-of-m consensus over the gathered verdicts (signed-verdict-record
/// "Consensus verifies signature before treating verdict as matching").
///
/// `verdicts` is `[(verifierId, VerdictRecord)]` for the round; entries need not be
/// sorted and may be fewer than `m` (missing entries are treated as null → fail-closed).
/// APPROVE verdicts lacking `registered_at` are not counted as matching (fail-closed).
/// Null/REJECT verdicts never count toward `n` (D9).
///
/// **Signature gate**: for each APPROVE verdict the slot's pinned pubkey is read via
/// [`verdict::read_pinned_pubkey`].
///   * If a pubkey IS pinned, the verdict's signature MUST verify against it
///     ([`verdict::verify_record`]); otherwise the verdict is recorded in
///     [`Rejection::signature_failures`] as `(verifierId, reason)` and does NOT count.
///   * If NO pubkey is pinned (legacy unsigned regime — e.g. a store populated before
///     signed-verdicts), the APPROVE is trusted as-is (backward-compatible).
///   * A malformed pin file fail-closes the slot (recorded as a `pubkey` failure).
pub fn evaluate(
    root: &Path,
    goal_id: &str,
    round: u32,
    verdicts: &[(String, VerdictRecord)],
    n: u32,
    m: u32,
) -> ConsensusResult {
    let mut matching: Vec<MatchingVerdict> = Vec::new();
    let mut rejection = Rejection::default();

    for (vid, rec) in verdicts {
        match rec.status {
            VerdictStatus::Approve => {
                // APPROVE requires a non-empty registeredAt to match (fail-closed).
                let ts = rec.registered_at.as_deref().filter(|s| !s.is_empty());
                let Some(ts) = ts else {
                    rejection.null_verifiers.push(vid.clone());
                    continue;
                };

                // Signature gate: bind the APPROVE to the slot's pinned pubkey.
                let pinned = match verdict::read_pinned_pubkey(root, goal_id, vid, round) {
                    Ok(opt) => opt,
                    // Malformed pin → cannot trust the slot; fail closed.
                    Err(_) => {
                        rejection.signature_failures.push((
                            vid.clone(),
                            "WrongPubkey: pinned pubkey is unreadable".to_string(),
                        ));
                        continue;
                    }
                };

                if let Some(key) = pinned {
                    // Pinned slot: the signature MUST verify.
                    match verdict::verify_record(rec, Some(&key), goal_id, vid, round) {
                        Ok(()) => matching.push(MatchingVerdict {
                            verifier_id: vid.clone(),
                            registered_at: ts.to_string(),
                        }),
                        Err(err) => {
                            rejection
                                .signature_failures
                                .push((vid.clone(), signature_failure_reason(&err)));
                        }
                    }
                } else {
                    // Legacy unsigned regime: no pinned key → trust the APPROVE.
                    matching.push(MatchingVerdict {
                        verifier_id: vid.clone(),
                        registered_at: ts.to_string(),
                    });
                }
            }
            VerdictStatus::Reject => {
                let notes = rec.notes.clone().unwrap_or_default();
                rejection.reject_notes.push((vid.clone(), notes));

                // Signature gate for REJECT verdicts too: a tampered verdict (e.g.
                // APPROVE flipped to REJECT without re-signing) must surface as a
                // signature failure, not silently pass as a benign reject.
                // Only check when a pinned key exists (legacy unsigned regime is
                // exempt — there is no signature to verify).
                if let Ok(Some(key)) = verdict::read_pinned_pubkey(root, goal_id, vid, round) {
                    if let Err(err) = verdict::verify_record(rec, Some(&key), goal_id, vid, round) {
                        rejection
                            .signature_failures
                            .push((vid.clone(), signature_failure_reason(&err)));
                    }
                }
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

/// Render a human-readable reason for a signature-verification failure, embedding the
/// variant keyword the rejection summary is matched on (`signature`, `pubkey`,
/// `unsigned`/`untrusted`).
fn signature_failure_reason(err: &verdict::VerdictError) -> String {
    use verdict::VerdictError::*;
    match err {
        BadSignature(msg) => {
            format!("BadSignature: signature verification failed ({msg})")
        }
        WrongPubkey => "WrongPubkey: declared pubkey does not match pinned key".to_string(),
        Untrusted => "Untrusted: unsigned or null verdict is never trusted".to_string(),
        other => format!("signature verification failed: {other}"),
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

/// Derive the `mmddyy` prefix (UTC date of `matchedAt`) from an RFC3339 ISO timestamp.
///
/// Returns `MMDDYY` (2-digit month, 2-digit day, 2-digit year). e.g.
/// `"2026-07-03T10:05:00Z"` → `"070326"`.
///
/// Parses the leading `YYYY-MM-DD` of any RFC3339 string; does not require a full
/// datetime parser. Returns the raw prefix slice so the caller controls error policy.
fn mmddyy_of(matched_at_iso: &str) -> String {
    // RFC3339 date prefix is always "YYYY-MM-DD" (10 chars) when present.
    // Defensive: if shorter/malformed, fall back to zeros (the hash stays deterministic;
    // the prefix is only a sortable label, never a tamper guard).
    let bytes = matched_at_iso.as_bytes();
    let (yyyy, mm, dd) = if bytes.len() >= 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        (
            &matched_at_iso[0..4],
            &matched_at_iso[5..7],
            &matched_at_iso[8..10],
        )
    } else {
        ("0000", "00", "00")
    };
    let yy = &yyyy[yyyy.len().saturating_sub(2)..];
    format!("{mm}{dd}{yy}")
}

/// Compute the tamper-evident completion hash (completion-proof MODIFIED, D6 rev 3).
///
/// Produces BOTH:
///   * `short_hash()`  = `mmddyy + "-" + first8hex(SHA256(inputs))` — displayed/printed
///   * `full_digest()` = full 64-hex SHA-256(inputs) — stored in `completion.json`
///
/// where `inputs = salt + goalId + goalSignature + String(roundNumber) +
/// canonicalJSON(matchingVerdicts sorted by verifierId) + matchedAtISO + receiptHead`,
/// and `receiptHead` is the chain tip of the goal's hash-chained receipt log ("" when
/// the log is absent — e.g. a fresh goal).
///
/// Deterministic: identical inputs yield identical short hash and full digest. Each
/// input guards a distinct tamper vector (see module docs). SHA-256 is computed exactly
/// once; short and full always agree on the inputs.
pub fn compute_hash(
    salt: &str,
    goal_id: &str,
    goal_sig: &str,
    round: u32,
    matching: &[MatchingVerdict],
    matched_at_iso: &str,
    receipt_head: &str,
) -> CompletionHash {
    let canon = canonical_matching_json(matching);
    // inputs = salt + goalId + goalSignature + String(roundNumber)
    //        + canonicalJSON(matchingVerdicts) + matchedAtISO + receiptHead
    // The receipt head (chain tip of the goal's hash-chained receipt log) binds the
    // completion proof to the exact sequence of registered verdicts (completion-proof
    // MODIFIED, D6). Empty for a fresh goal with no receipt log.
    let input = format!("{salt}{goal_id}{goal_sig}{round}{canon}{matched_at_iso}{receipt_head}");
    let full = hex::encode(Sha256::digest(input.as_bytes()));
    debug_assert_eq!(
        full.len(),
        HASH_FULL_HEX_LEN,
        "SHA-256 hex digest must be 64 chars"
    );
    let short = format!(
        "{}-{}",
        mmddyy_of(matched_at_iso),
        &full[..HASH_SHORT_HEX_LEN]
    );
    CompletionHash { short, full }
}

/// Write `completion.json` for a passing round. Refuses (returns `Err(NotPassed)`)
/// when the round did not reach consensus — no completion record is ever produced on
/// failure.
///
/// The write is atomic (tmp sibling + rename). Fails closed if the goal directory is
/// missing or the store is unusable. The record carries both the short `hash` and the
/// `full_digest` for exact audit recompute.
pub fn write_completion(
    root: &Path,
    goal_id: &str,
    result: &ConsensusResult,
    round: u32,
    hash: &CompletionHash,
    matched_at_iso: &str,
    trace_id: Option<&str>,
) -> Result<PathBuf, ConsensusError> {
    if !result.passed {
        return Err(ConsensusError::NotPassed);
    }

    let gdir = goal::goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(ConsensusError::GoalNotFound);
    }

    let record = CompletionRecord {
        hash: hash.short_hash().to_string(),
        full_digest: hash.full_digest().to_string(),
        goal_id: goal_id.to_string(),
        round_number: round,
        matched_at: matched_at_iso.to_string(),
        matching_verdicts: result.matching_verdicts.clone(),
        trace_id: trace_id.map(str::to_string),
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
        assert!(
            !j.contains(' '),
            "canonical JSON must have no whitespace: {j}"
        );
    }

    #[test]
    fn compute_hash_uses_exact_concatenation_order() {
        let matching = vec![mv("v1", "2026-07-03T10:00:00Z")];
        let h = compute_hash(
            "SALT",
            "GID",
            "SIG",
            1,
            &matching,
            "2026-07-03T10:05:00Z",
            "head0",
        );

        // Independent recompute — full digest + short form (head appended last).
        let canon = canonical_matching_json(&matching);
        let input = format!("SALTGIDSIG1{canon}2026-07-03T10:05:00Zhead0");
        let digest = hex::encode(Sha256::digest(input.as_bytes()));
        assert_eq!(
            h.full_digest(),
            digest,
            "full digest must match exact concat order"
        );
        assert_eq!(
            h.short_hash(),
            format!("070326-{}", &digest[..HASH_SHORT_HEX_LEN])
        );
    }

    #[test]
    fn evaluate_reject_notes_collected_on_fail() {
        let verdicts = vec![
            (
                "v1".to_string(),
                VerdictRecord {
                    status: VerdictStatus::Approve,
                    notes: None,
                    registered_at: Some("t".into()),
                    signature: None,
                    pubkey_id: None,
                },
            ),
            (
                "v2".to_string(),
                VerdictRecord {
                    status: VerdictStatus::Reject,
                    notes: Some("needs work".into()),
                    registered_at: Some("t".into()),
                    signature: None,
                    pubkey_id: None,
                },
            ),
        ];
        let r = evaluate(
            std::path::Path::new("/nonexistent-consensus-internal-test"),
            "g",
            1,
            &verdicts,
            2,
            2,
        );
        assert!(!r.passed);
        assert_eq!(
            r.rejection.reject_notes,
            vec![("v2".to_string(), "needs work".to_string())]
        );
    }
}
