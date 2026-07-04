//! Hash-chained receipt log (tasks.md §5 / receipt-log spec, design.md D4).
//!
//! Every successful verdict write appends ONE chained line to
//! `goals/<goalId>/receipt-log.jsonl`. Each line is a compact JSON object whose
//! `entryHash` chains over the previous line's `entryHash` via `prevHash`.
//!
//! Canonical entry-hash contract (PINNED by the RED test `tests/receipt.rs`):
//!
//! ```text
//! entryHash = lowercase_hex( SHA256( prevHash + "|" + seq + "|" + kind + "|" + verdictId + "|" + status ) )
//! ```
//!
//! where `seq` is the decimal `u64` (no leading zeros), `kind ∈ {"approve","reject"}`,
//! `status ∈ {"APPROVE","REJECT"}`, and the first entry's `prevHash` is the empty string.
//!
//! Fail-closed: a NULL verdict never writes an entry; a missing store yields no log;
//! any in-place edit of `status` / `prevHash` / `seq` is detectable via [`verify_chain`].

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::goal;

/// On-disk receipt log filename, sibling to `goal.json` / `signature.json`.
pub const RECEIPT_LOG_FILE: &str = "receipt-log.jsonl";

/// One immutable, hash-chained line in the per-goal receipt log.
///
/// On-disk keys are camelCase (`verdictId`, `prevHash`, `entryHash`, `signedBy`) per the
/// receipt-log spec contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptEntry {
    /// 1-based sequence number (line count at append time + 1).
    pub seq: u64,
    /// Verdict kind: `"approve"` or `"reject"`.
    pub kind: String,
    /// Verifier slot id that produced this entry (e.g. `"v1"`).
    pub verdict_id: String,
    /// Verifier-CLI status: `"APPROVE"` or `"REJECT"`.
    pub status: String,
    /// Chain link: the previous entry's `entry_hash`, or `""` for the first entry.
    pub prev_hash: String,
    /// `lowercase_hex(SHA256(prevHash|seq|kind|verdictId|status))`.
    pub entry_hash: String,
    /// First 16 hex of the signer's pubkey id (provenance).
    pub signed_by: String,
}

/// Errors raised by the receipt log layer. All fail-closed.
#[derive(Debug, thiserror::Error)]
pub enum ReceiptError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("chain broken at seq {seq}: {reason}")]
    ChainBreak { seq: u64, reason: String },
}

/// Resolve the on-disk receipt log path: `<root>/goals/<goal_id>/receipt-log.jsonl`.
pub fn receipt_log_path(root: &Path, goal_id: &str) -> PathBuf {
    goal::goal_dir(root, goal_id).join(RECEIPT_LOG_FILE)
}

/// Append a single chained entry to the goal's receipt log.
///
/// * Reads the current head (last `entry_hash`, or `""` if absent).
/// * `seq = (line count) + 1`.
/// * Computes `entry_hash` via the pinned canonical form.
/// * Writes one compact JSON line + `\n`.
///
/// Returns the newly-appended entry's `entry_hash` (the new chain head).
pub fn append_receipt(
    root: &Path,
    goal_id: &str,
    kind: &str,
    verdict_id: &str,
    status: &str,
    signed_by: &str,
) -> Result<String, ReceiptError> {
    let log_path = receipt_log_path(root, goal_id);

    // 1. Current head + 2. seq = line_count + 1.
    let (prev_hash, line_count) = read_head_and_count(&log_path);
    let seq = line_count + 1;

    // 3 + 4. Canonical form + entry hash.
    let entry_hash = compute_entry_hash(&prev_hash, seq, kind, verdict_id, status);

    // 5. Build the entry.
    let entry = ReceiptEntry {
        seq,
        kind: kind.to_string(),
        verdict_id: verdict_id.to_string(),
        status: status.to_string(),
        prev_hash: prev_hash.clone(),
        entry_hash: entry_hash.clone(),
        signed_by: signed_by.to_string(),
    };

    // 6. Append one compact JSON line + '\n'.
    let line = serde_json::to_string(&entry)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    writeln!(file, "{line}")?;

    // 7. Return the new chain head.
    Ok(entry_hash)
}

/// Read the chain head: the `entry_hash` of the LAST line, or `""` if the log is absent
/// or empty.
pub fn read_receipt_head(root: &Path, goal_id: &str) -> String {
    let log_path = receipt_log_path(root, goal_id);
    read_head_and_count(&log_path).0
}

/// Read and parse the entire receipt log, one `ReceiptEntry` per non-empty line.
pub fn read_receipt_log(root: &Path, goal_id: &str) -> Result<Vec<ReceiptEntry>, ReceiptError> {
    let log_path = receipt_log_path(root, goal_id);
    if !log_path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&log_path)?;
    let mut entries = Vec::new();
    for line in raw.split('\n') {
        if line.is_empty() {
            continue;
        }
        let entry: ReceiptEntry = serde_json::from_str(line)?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Verify an in-memory chain: every entry's `entry_hash` must match a recomputation from
/// `{prev_hash, seq, kind, verdict_id, status}`, and every entry's `prev_hash` must equal
/// the previous entry's `entry_hash` (or `""` for the first entry).
///
/// Returns `Err` on the first mismatch.
pub fn verify_chain(entries: &[ReceiptEntry]) -> Result<(), ReceiptError> {
    let mut prev_entry_hash = String::new();
    for entry in entries {
        // Chain link: prev_hash must equal the previous entry's entry_hash (or "" for i=0).
        if entry.prev_hash != prev_entry_hash {
            return Err(ReceiptError::ChainBreak {
                seq: entry.seq,
                reason: format!(
                    "prevHash does not link to the previous entry's entryHash (expected `{}`)",
                    if prev_entry_hash.is_empty() {
                        "(empty)".to_string()
                    } else {
                        prev_entry_hash.clone()
                    }
                ),
            });
        }

        // Recompute and compare.
        let recomputed =
            compute_entry_hash(&entry.prev_hash, entry.seq, &entry.kind, &entry.verdict_id, &entry.status);
        if recomputed != entry.entry_hash {
            return Err(ReceiptError::ChainBreak {
                seq: entry.seq,
                reason: "recomputed entryHash does not match stored entryHash".to_string(),
            });
        }

        prev_entry_hash = entry.entry_hash.clone();
    }
    Ok(())
}

/// Pinned canonical form: `lowercase_hex(SHA256(prevHash|seq|kind|verdictId|status))`.
fn compute_entry_hash(prev_hash: &str, seq: u64, kind: &str, verdict_id: &str, status: &str) -> String {
    let input = format!("{prev_hash}|{seq}|{kind}|{verdict_id}|{status}");
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    hex::encode(h.finalize())
}

/// Read the log file and return `(last_entry_hash, non_empty_line_count)`.
///
/// If the file does not exist, returns `("", 0)`.
fn read_head_and_count(log_path: &Path) -> (String, u64) {
    if !log_path.exists() {
        return (String::new(), 0);
    }
    let raw = match std::fs::read_to_string(log_path) {
        Ok(s) => s,
        Err(_) => return (String::new(), 0),
    };
    let mut count = 0u64;
    let mut head = String::new();
    for line in raw.split('\n') {
        if line.is_empty() {
            continue;
        }
        count += 1;
        match serde_json::from_str::<ReceiptEntry>(line) {
            Ok(entry) => head = entry.entry_hash,
            Err(_) => {
                // Unparseable trailing bytes: leave the head as the last good one.
            }
        }
    }
    (head, count)
}
