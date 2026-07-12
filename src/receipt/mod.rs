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

use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::goal;

/// On-disk receipt log filename, sibling to `goal.json` / `signature.json`.
pub const RECEIPT_LOG_FILE: &str = "receipt-log.jsonl";

/// One immutable, hash-chained line in the per-goal receipt log.
///
/// On-disk keys are camelCase (`verdictId`, `prevHash`, `entryHash`, `signedBy`,
/// `traceId`) per the receipt-log spec contract.
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
    /// Active per-goal trace id (observability metadata, add-otel-observability D2).
    /// Sourced from `VERIFIER_LOOP_TRACE_ID` at append time. EXCLUDED from
    /// `entry_hash` (design D4) — it is not tamper-evident evidence, only an audit
    /// pivot to the span trail. `None` when the env var is unset (backward-compat
    /// with pre-change logs); omitted from the JSON line via skip_serializing_if.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
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
    /// Failed to acquire the exclusive append lock (e.g. held by another process
    /// for too long, or lock FS not supported). Fail-closed: do not append
    /// without the lock, since that would race concurrent writers.
    #[error("receipt log lock error: {0}")]
    Lock(String),
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

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Open (create if needed) then acquire an EXCLUSIVE lock for the full
    // read-head + append cycle. This closes the TOCTOU race flagged in PR#5:
    // without the lock, two concurrent verifier processes could both read the
    // same (prev_hash, line_count), emit duplicate seq numbers, and break the
    // chain. The lock is advisory (flock on Unix, LockFileEx on Windows) — it
    // relies on every writer cooperating via this code path.
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&log_path)?;
    file.lock_exclusive()
        .map_err(|e| ReceiptError::Lock(e.to_string()))?;

    // SAFETY: from here until unlock, no other cooperating writer can enter.
    let result = append_receipt_locked(&file, kind, verdict_id, status, signed_by);

    // Always release the lock, even on error (don't poison the slot).
    let _ = file.unlock();

    result
}

/// Append under an already-held exclusive lock. Reads head+count from `file`,
/// computes the new entry, appends one line, flushes.
fn append_receipt_locked(
    file: &std::fs::File,
    kind: &str,
    verdict_id: &str,
    status: &str,
    signed_by: &str,
) -> Result<String, ReceiptError> {
    // 1. Current head + 2. seq = line_count + 1. Errors now propagate (PR#5):
    // a transient read failure on a non-empty log MUST NOT be swallowed into
    // ("", 0), which would corrupt the chain by restarting at seq=1.
    let (prev_hash, line_count) = read_head_and_count(file)?;
    let seq = line_count + 1;

    // 3 + 4. Canonical form + entry hash.
    let entry_hash = compute_entry_hash(&prev_hash, seq, kind, verdict_id, status);

    // 5. Build the entry. The traceId is observability metadata (design D2/D4):
    // read from env at append time, EXCLUDED from `entry_hash` (computed above
    // from the canonical tuple only). None when env unset → omitted from JSON.
    let trace_id = crate::observe::trace_id_from_env();
    let entry = ReceiptEntry {
        seq,
        kind: kind.to_string(),
        verdict_id: verdict_id.to_string(),
        status: status.to_string(),
        prev_hash: prev_hash.clone(),
        entry_hash: entry_hash.clone(),
        signed_by: signed_by.to_string(),
        trace_id,
    };

    // 6. Append one compact JSON line + '\n'. `file` was opened with append(true)
    // so the OS positions the write at end-of-file atomically per write(2).
    let line = serde_json::to_string(&entry)?;
    let mut writer = std::io::BufWriter::new(file);
    writeln!(writer, "{line}")?;
    writer.flush()?;

    // 7. Return the new chain head.
    Ok(entry_hash)
}

/// Read the chain head: the `entry_hash` of the LAST line, or `""` if the log is absent
/// or empty.
pub fn read_receipt_head(root: &Path, goal_id: &str) -> String {
    let log_path = receipt_log_path(root, goal_id);
    // Open without creating; if absent, head is empty. A transient read error
    // is logged away here for back-compat (read_receipt_head is a read-only
    // best-effort helper used by consensus/auditors), but read_receipt_log
    // (the parsing variant) propagates errors. See PR#5.
    match std::fs::File::open(&log_path) {
        Ok(f) => read_head_and_count(&f).map(|(h, _)| h).unwrap_or_default(),
        Err(_) => String::new(),
    }
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
        let recomputed = compute_entry_hash(
            &entry.prev_hash,
            entry.seq,
            &entry.kind,
            &entry.verdict_id,
            &entry.status,
        );
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
fn compute_entry_hash(
    prev_hash: &str,
    seq: u64,
    kind: &str,
    verdict_id: &str,
    status: &str,
) -> String {
    let input = format!("{prev_hash}|{seq}|{kind}|{verdict_id}|{status}");
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    hex::encode(h.finalize())
}

/// Read the log file and return `(last_entry_hash, non_empty_line_count)`.
///
/// If the file does not exist, returns `("", 0)`.
/// I/O errors propagate (PR#5): previously a read failure was silently
/// mapped to `("", 0)`, which would corrupt the chain by restarting at seq=1.
fn read_head_and_count(file: &std::fs::File) -> Result<(String, u64), std::io::Error> {
    let mut raw = String::new();
    // Seek to start in case the file was opened in append mode (append-mode
    // writes go to EOF, but reads start at the current offset which may be EOF
    // on some platforms).
    use std::io::{Read, Seek, SeekFrom};
    let mut f = file;
    f.seek(SeekFrom::Start(0))?;
    f.read_to_string(&mut raw)?;
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
    Ok((head, count))
}
