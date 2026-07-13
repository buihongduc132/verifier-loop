//! Run introspection + completion audit (intention 2026-07-14).
//!
//! Two read-only surfaces over a goal's stored artifacts:
//!
//! * [`collect_stats`] — aggregates EVERYTHING currently stored as JSON for a goal into one
//!   machine-readable [`serde_json::Value`] (goal record, creation-time config snapshot,
//!   per-round verdicts, completion, health, durations). Powers `jewilo STATS <goalId>`.
//! * [`audit`] — post-hoc verification that the final completion TRULY matches the
//!   creation-time config requirement: reads the creation-time n/m from `goal.json`,
//!   re-checks the matching APPROVE count, recomputes the completion hash from the stored
//!   inputs, and compares it to the stored `fullDigest`. Powers `jewilo AUDIT <goalId>`.
//!
//! Both are read-only and take NO goal lock — a STATS/AUDIT probe must never block on a
//! long-running round (mirrors the STATUS precedent from add-round-recovery LD7).

use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::consensus::{self, CompletionRecord};
use crate::goal::{self, GoalRecord};
use crate::health;
use crate::receipt;
use crate::store;
use crate::verdict::{self, VerdictRecord, VerdictStatus};

/// Errors raised by the stats/audit layer. Read-only; every error fails closed (surfaces
/// to the CLI as a non-zero exit with a message, never a misleading valid report).
#[derive(Debug, thiserror::Error)]
pub enum StatsError {
    #[error("goal not found (store or goal directory missing)")]
    GoalNotFound,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("goal layer error: {0}")]
    Goal(#[from] goal::GoalError),
    #[error("store layer error: {0}")]
    Store(#[from] store::StoreError),
}

// ---------------------------------------------------------------------------
// STATS — aggregate every stored JSON artifact for a goal
// ---------------------------------------------------------------------------

/// Collect every stored JSON artifact for a goal into one machine-readable object.
///
/// Shape (camelCase keys, matching the on-disk contract):
/// ```json
/// {
///   "goal":      { "goalId", "goalText", "context", "createdAt" },
///   "config":    { creation-time config snapshot (n, m, maxTurn, backend, …) },
///   "round":     <current round>,
///   "rounds":    [ { "round", "verdicts": [ { "verifierId", "verdict", "notes"? } ] } ],
///   "completion":{ "hash", "fullDigest", "matchedAt", "matchingVerdicts" } | null,
///   "health":    { "unhealthyLastHour", "cooldown" },
///   "durations": { "createdAt", "matchedAt"?, "wallClockSeconds"? }
/// }
/// ```
///
/// Read-only: takes no lock, spawns nothing. A missing `completion.json` yields
/// `completion: null` (not an error). A missing/empty `health.jsonl` yields a zero-count
/// health block.
pub fn collect_stats(root: &Path, goal_id: &str) -> Result<Value, StatsError> {
    let gdir = goal::goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(StatsError::GoalNotFound);
    }
    let record = goal::load(root, goal_id)?;
    let round = goal::current_round(root, goal_id).unwrap_or(1);

    // Goal block (omit the nested `config` — it is surfaced separately at top level).
    let goal_block = json!({
        "goalId": record.goal_id,
        "goalText": record.goal_text,
        "context": record.context,
        "createdAt": record.created_at,
    });

    // Creation-time config snapshot (the authoritative requirement for AUDIT).
    let config_block = serde_json::to_value(&record.config)?;

    // Per-round verdict data.
    let rounds_block = collect_rounds(root, goal_id, round, record.config.m)?;

    // Completion (may be absent).
    let completion_block = read_completion_record(root, goal_id)?;

    // Health (store-wide health.jsonl).
    let now = Utc::now();
    let unhealthy_last_hour = count_unhealthy_last_hour(root, now);
    let cooldown = health::in_cooldown(root, now);
    let health_block = json!({
        "unhealthyLastHour": unhealthy_last_hour,
        "cooldown": cooldown,
    });

    // Durations.
    let durations_block = build_durations(&record, completion_block.as_ref());

    Ok(json!({
        "goal": goal_block,
        "config": config_block,
        "round": round,
        "rounds": rounds_block,
        "completion": completion_block,
        "health": health_block,
        "durations": durations_block,
    }))
}

/// Collect per-round verdict data for rounds 1..=current.
fn collect_rounds(
    root: &Path,
    goal_id: &str,
    current_round: u32,
    m: u32,
) -> Result<Vec<Value>, StatsError> {
    let rounds_root = goal::goal_dir(root, goal_id).join(goal::ROUNDS_DIR);
    let mut out: Vec<Value> = Vec::new();
    for r in 1..=current_round {
        let mut verdicts: Vec<Value> = Vec::new();
        for i in 0..m as usize {
            let vid = format!("v{}", i + 1);
            let rec = verdict::read_verdict(root, goal_id, &vid, r).unwrap_or(VerdictRecord {
                status: VerdictStatus::Null,
                notes: None,
                registered_at: None,
                signature: None,
                pubkey_id: None,
            });
            let status_str = match rec.status {
                VerdictStatus::Approve => "APPROVE",
                VerdictStatus::Reject => "REJECT",
                VerdictStatus::Null => "null",
            };
            let mut entry = json!({
                "verifierId": vid,
                "verdict": status_str,
            });
            if let Some(notes) = rec.notes.as_deref() {
                entry["notes"] = Value::String(notes.to_string());
            }
            verdicts.push(entry);
        }
        out.push(json!({ "round": r, "verdicts": verdicts }));
    }
    // Best-effort: also surface any round directories beyond `current_round` that exist
    // on disk (e.g. a partially-incremented state) so the report is never incomplete.
    if let Ok(entries) = fs::read_dir(&rounds_root) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Ok(r) = name.parse::<u32>() {
                    if r > current_round {
                        out.push(json!({ "round": r, "verdicts": collect_round_verdicts(root, goal_id, r, m) }));
                    }
                }
            }
        }
    }
    let _ = rounds_root;
    Ok(out)
}

/// Read verdicts for a single round (best-effort, used for extra rounds beyond current).
fn collect_round_verdicts(root: &Path, goal_id: &str, round: u32, m: u32) -> Vec<Value> {
    let mut out = Vec::new();
    for i in 0..m as usize {
        let vid = format!("v{}", i + 1);
        let rec = verdict::read_verdict(root, goal_id, &vid, round).unwrap_or(VerdictRecord {
            status: VerdictStatus::Null,
            notes: None,
            registered_at: None,
            signature: None,
            pubkey_id: None,
        });
        let status_str = match rec.status {
            VerdictStatus::Approve => "APPROVE",
            VerdictStatus::Reject => "REJECT",
            VerdictStatus::Null => "null",
        };
        out.push(json!({ "verifierId": vid, "verdict": status_str }));
    }
    out
}

/// Read `completion.json` for a goal, returning `None` when absent.
fn read_completion_record(root: &Path, goal_id: &str) -> Result<Option<Value>, StatsError> {
    let path = goal::goal_dir(root, goal_id).join(consensus::COMPLETION_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let rec: CompletionRecord = serde_json::from_str(&raw)?;
    Ok(Some(json!({
        "hash": rec.hash,
        "fullDigest": rec.full_digest,
        "matchedAt": rec.matched_at,
        "matchingVerdicts": rec.matching_verdicts,
    })))
}

/// Count unhealthy events in the last hour from the store-wide `health.jsonl`. Mirrors the
/// private `count_recent` in the health module but is duplicated here to keep the stats
/// layer self-contained (the health module's counter is private by design).
fn count_unhealthy_last_hour(root: &Path, now: DateTime<Utc>) -> u64 {
    let window_start = now - health::cooldown_window();
    let Ok(raw) = fs::read_to_string(root.join(health::HEALTH_LOG)) else {
        return 0;
    };
    let mut count = 0u64;
    for line in raw.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if v.get("event").and_then(|e| e.as_str()) == Some("unhealthy") {
                if let Some(at_str) = v.get("at").and_then(|a| a.as_str()) {
                    if let Ok(at) = DateTime::parse_from_rfc3339(at_str) {
                        let at = at.with_timezone(&Utc);
                        if at >= window_start && at <= now {
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    count
}

/// Build the durations block: createdAt, matchedAt (if completion exists), and the derived
/// wall-clock seconds between them.
fn build_durations(record: &GoalRecord, completion: Option<&Value>) -> Value {
    let created_at = &record.created_at;
    let matched_at = completion.and_then(|c| c.get("matchedAt").and_then(|m| m.as_str()));
    let mut block = json!({ "createdAt": created_at });
    if let Some(m) = matched_at {
        block["matchedAt"] = Value::String(m.to_string());
        if let (Ok(c), Ok(m_dt)) = (
            DateTime::parse_from_rfc3339(created_at),
            DateTime::parse_from_rfc3339(m),
        ) {
            let secs = (m_dt.with_timezone(&Utc) - c.with_timezone(&Utc)).num_seconds().max(0);
            block["wallClockSeconds"] = Value::Number(secs.into());
        }
    }
    block
}

// ---------------------------------------------------------------------------
// AUDIT — verify the final completion matches the creation-time requirement
// ---------------------------------------------------------------------------

/// The AUDIT report printed to stdout by `jewilo AUDIT <goalId>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditReport {
    pub valid: bool,
    pub required_n: u32,
    pub required_m: u32,
    /// Number of matching (APPROVE) verdicts recorded in `completion.json`.
    pub matching_verdicts: u32,
    /// Recomputed full 64-hex SHA-256 digest from the stored inputs.
    pub hash_recomputed: String,
    /// Stored `fullDigest` from `completion.json`.
    pub hash_stored: String,
    /// Individual named checks (pass/fail with a reason), for transparency.
    pub checks: Vec<AuditCheck>,
}

/// One named audit check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditCheck {
    pub name: String,
    pub passed: bool,
    /// Why the check failed (absent on pass).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Audit a goal's final completion against its creation-time config requirement.
///
/// Reads the creation-time n/m from `goal.json` (the snapshot, NOT the current
/// `config.json`), reads `completion.json`, verifies the matching APPROVE count reaches
/// `n` out of `m`, recomputes the completion hash from the stored inputs, and compares it
/// to the stored `fullDigest`.
///
/// Returns the report. The caller maps `report.valid` to the process exit code (0 = valid,
/// non-zero = invalid). Read-only: takes no lock, spawns nothing.
pub fn audit(root: &Path, goal_id: &str) -> Result<AuditReport, StatsError> {
    let gdir = goal::goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(StatsError::GoalNotFound);
    }
    let record = goal::load(root, goal_id)?;
    let required_n = record.config.n;
    let required_m = record.config.m;

    let mut checks: Vec<AuditCheck> = Vec::new();
    let mut valid = true;

    // Check 1: completion.json exists.
    let completion_path = gdir.join(consensus::COMPLETION_FILE);
    let completion: CompletionRecord = if !completion_path.exists() {
        checks.push(AuditCheck {
            name: "completionExists".into(),
            passed: false,
            reason: Some("no completion.json — the goal did not reach consensus".into()),
        });
        // Return early with an invalid report; there is nothing else to verify.
        return Ok(empty_invalid_report(required_n, required_m, checks));
    } else {
        let raw = fs::read_to_string(&completion_path)?;
        let rec: CompletionRecord = serde_json::from_str(&raw)?;
        checks.push(AuditCheck {
            name: "completionExists".into(),
            passed: true,
            reason: None,
        });
        rec
    };

    let matching_count = completion.matching_verdicts.len() as u32;

    // Check 2: matching APPROVE count reaches n out of m (the creation-time requirement).
    let count_ok = matching_count >= required_n && matching_count <= required_m;
    checks.push(AuditCheck {
        name: "verdictCountMatchesRequirement".into(),
        passed: count_ok,
        reason: if count_ok {
            None
        } else {
            valid = false;
            Some(format!(
                "matching APPROVE verdicts ({matching_count}) do not satisfy the creation-time requirement n={required_n} of m={required_m}"
            ))
        },
    });

    // Check 3: recompute the completion hash from the stored inputs and compare.
    let salt = store::salt_in(root)?;
    let sig_record: goal::SignatureRecord =
        serde_json::from_str(&fs::read_to_string(gdir.join(goal::SIGNATURE_FILE))?)?;
    let receipt_head = receipt::read_receipt_head(root, goal_id);
    let recomputed = consensus::compute_hash(
        &salt,
        goal_id,
        &sig_record.signature,
        completion.round_number,
        &completion.matching_verdicts,
        &completion.matched_at,
        &receipt_head,
    );
    let hash_ok = recomputed.full_digest() == completion.full_digest;
    checks.push(AuditCheck {
        name: "completionHashMatches".into(),
        passed: hash_ok,
        reason: if hash_ok {
            None
        } else {
            valid = false;
            Some(format!(
                "recomputed fullDigest {} does not match stored fullDigest {} (hash mismatch / tamper)",
                recomputed.full_digest(),
                completion.full_digest
            ))
        },
    });

    Ok(AuditReport {
        valid,
        required_n,
        required_m,
        matching_verdicts: matching_count,
        hash_recomputed: recomputed.full_digest().to_string(),
        hash_stored: completion.full_digest,
        checks,
    })
}

/// Build an invalid report when there is no completion to audit.
fn empty_invalid_report(required_n: u32, required_m: u32, checks: Vec<AuditCheck>) -> AuditReport {
    AuditReport {
        valid: false,
        required_n,
        required_m,
        matching_verdicts: 0,
        hash_recomputed: String::new(),
        hash_stored: String::new(),
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_check_serializes_camel_case() {
        let c = AuditCheck {
            name: "test".into(),
            passed: true,
            reason: None,
        };
        let j = serde_json::to_string(&c).unwrap();
        assert!(j.contains("\"name\""), "{j}");
        assert!(j.contains("\"passed\""), "{j}");
        assert!(!j.contains("reason"), "reason omitted on pass: {j}");
    }

    #[test]
    fn audit_report_shape() {
        let r = AuditReport {
            valid: true,
            required_n: 2,
            required_m: 2,
            matching_verdicts: 2,
            hash_recomputed: "abc".into(),
            hash_stored: "abc".into(),
            checks: vec![AuditCheck {
                name: "completionExists".into(),
                passed: true,
                reason: None,
            }],
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"requiredN\":2"), "{j}");
        assert!(j.contains("\"requiredM\":2"), "{j}");
        assert!(j.contains("\"matchingVerdicts\":2"), "{j}");
        assert!(j.contains("\"hashRecomputed\":\"abc\""), "{j}");
        assert!(j.contains("\"hashStored\":\"abc\""), "{j}");
    }

    #[test]
    fn count_unhealthy_empty_returns_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(count_unhealthy_last_hour(dir.path(), Utc::now()), 0);
    }

    #[test]
    fn build_durations_without_completion_omits_matched_at() {
        let rec = GoalRecord {
            goal_id: "g".into(),
            goal_text: "t".into(),
            context: None,
            created_at: "2026-07-14T10:00:00Z".into(),
            config: store::Config::default(),
        };
        let d = build_durations(&rec, None);
        assert!(d["createdAt"].is_string());
        assert!(d.get("matchedAt").is_none());
        assert!(d.get("wallClockSeconds").is_none());
    }
}
