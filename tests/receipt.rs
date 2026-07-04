// tasks.md §5 — Receipt log (receipt-log spec, design.md D4).
// RED phase: written first, against the spec, before any implementation.
//
// Covers the receipt-log spec scenarios:
//   * Every successful verdict write appends ONE chained line to receipt-log.jsonl
//   * Subsequent entry chains the previous entry's entryHash via prevHash
//   * append_receipt returns the new chain head (last entry's entryHash)
//   * read_receipt_head returns "" for a fresh goal, else the last entry's entryHash
//   * verify_chain recomputes entryHash from {seq,kind,verdictId,status} and detects edits
//   * Trailing-line deletion is detected by head mismatch
//
// Canonical entry-hash contract (PINNED by this test, design.md D4):
//   entryHash = lowercase_hex( SHA256( prevHash + "|" + seq + "|" + kind + "|" + verdictId + "|" + status ) )
// where seq is the decimal u64 (no leading zeros), kind ∈ {"approve","reject"},
// status ∈ {"APPROVE","REJECT"}, verdictId is the verifier slot id (e.g. "v1").
// The first entry's prevHash is the empty string "".

use std::fs;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use verifier_loop::goal;
use verifier_loop::receipt;

/// Recompute the pinned canonical entry hash so the tests are self-validating
/// and the GREEN team has a concrete, deterministic contract to match.
fn expected_entry_hash(prev_hash: &str, seq: u64, kind: &str, verdict_id: &str, status: &str) -> String {
    let input = format!("{prev_hash}|{seq}|{kind}|{verdict_id}|{status}");
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    hex::encode(h.finalize())
}

/// Seed a fresh goal dir under a tempdir store root. Returns (TempDir, goalId).
fn fresh_goal() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), "ship the thing", None).unwrap();
    (dir, goal_id)
}

const APPROVE: &str = "APPROVE";
const REJECT: &str = "REJECT";

#[test]
fn append_first_entry_chains_from_empty_prev_hash() {
    let (dir, goal_id) = fresh_goal();

    let signed_by = "abcd1234abcd1234"; // first 16 hex of a pubkey id
    let head = receipt::append_receipt(
        dir.path(),
        &goal_id,
        "approve",
        "v1",
        APPROVE,
        signed_by,
    )
    .expect("first append must succeed");

    let entries = receipt::read_receipt_log(dir.path(), &goal_id).unwrap();
    assert_eq!(entries.len(), 1, "exactly one entry after first append");

    let e = &entries[0];
    assert_eq!(e.seq, 1, "seq must be 1-based");
    assert_eq!(e.kind, "approve");
    assert_eq!(e.verdict_id, "v1");
    assert_eq!(e.status, APPROVE);
    assert_eq!(e.prev_hash, "", "first entry prevHash MUST be the empty string");
    assert_eq!(e.signed_by, signed_by);

    let expected_hash = expected_entry_hash("", 1, "approve", "v1", APPROVE);
    assert_eq!(
        e.entry_hash, expected_hash,
        "entryHash must be SHA256(prevHash|seq|kind|verdictId|status)"
    );
    assert_eq!(
        head, e.entry_hash,
        "returned head must equal the appended entry's entryHash"
    );
}

#[test]
fn append_second_entry_chains_previous_entry_hash() {
    let (dir, goal_id) = fresh_goal();

    receipt::append_receipt(dir.path(), &goal_id, "approve", "v1", APPROVE, "aa11bb22cc33dd44")
        .unwrap();
    receipt::append_receipt(dir.path(), &goal_id, "approve", "v2", APPROVE, "ee55ff6677889900")
        .unwrap();

    let entries = receipt::read_receipt_log(dir.path(), &goal_id).unwrap();
    assert_eq!(entries.len(), 2);

    let first = &entries[0];
    let second = &entries[1];

    assert_eq!(second.seq, 2);
    assert_eq!(
        second.prev_hash,
        first.entry_hash,
        "second entry prevHash MUST equal first entry entryHash"
    );

    let expected_second_hash =
        expected_entry_hash(&first.entry_hash, 2, "approve", "v2", APPROVE);
    assert_eq!(
        second.entry_hash, expected_second_hash,
        "entryHash MUST chain over the previous entry's entryHash"
    );
}

#[test]
fn append_returns_new_chain_head() {
    let (dir, goal_id) = fresh_goal();

    let head1 = receipt::append_receipt(
        dir.path(), &goal_id, "approve", "v1", APPROVE, "aa11bb22cc33dd44"
    ).unwrap();
    let head2 = receipt::append_receipt(
        dir.path(), &goal_id, "approve", "v2", APPROVE, "ee55ff6677889900"
    ).unwrap();

    let entries = receipt::read_receipt_log(dir.path(), &goal_id).unwrap();
    assert_eq!(head1, entries[0].entry_hash);
    assert_eq!(head2, entries[1].entry_hash);
    assert_ne!(head1, head2, "two distinct entries must have distinct hashes");
}

#[test]
fn read_receipt_head_returns_empty_when_log_absent() {
    let (dir, goal_id) = fresh_goal();
    // No appends yet — log must not exist.
    assert_eq!(
        receipt::read_receipt_head(dir.path(), &goal_id),
        "",
        "head MUST be empty string when no receipt log exists"
    );
}

#[test]
fn read_receipt_head_returns_last_entry_hash() {
    let (dir, goal_id) = fresh_goal();

    receipt::append_receipt(dir.path(), &goal_id, "approve", "v1", APPROVE, "aa11bb22cc33dd44")
        .unwrap();
    let head2 = receipt::append_receipt(
        dir.path(), &goal_id, "approve", "v2", APPROVE, "ee55ff6677889900"
    ).unwrap();

    let head = receipt::read_receipt_head(dir.path(), &goal_id);
    assert_eq!(head, head2, "head MUST be the entryHash of the LAST appended line");
}

#[test]
fn verify_chain_accepts_genuine_chain() {
    let (dir, goal_id) = fresh_goal();

    receipt::append_receipt(dir.path(), &goal_id, "approve", "v1", APPROVE, "aa11bb22cc33dd44")
        .unwrap();
    receipt::append_receipt(dir.path(), &goal_id, "approve", "v2", APPROVE, "ee55ff6677889900")
        .unwrap();

    let entries = receipt::read_receipt_log(dir.path(), &goal_id).unwrap();
    // A genuine, unmutated chain MUST verify cleanly.
    receipt::verify_chain(&entries).expect("genuine chain must verify");
}

#[test]
fn verify_chain_detects_mid_log_status_edit() {
    let (dir, goal_id) = fresh_goal();

    receipt::append_receipt(dir.path(), &goal_id, "approve", "v1", APPROVE, "aa11bb22cc33dd44")
        .unwrap();
    receipt::append_receipt(dir.path(), &goal_id, "approve", "v2", APPROVE, "ee55ff6677889900")
        .unwrap();

    let mut entries = receipt::read_receipt_log(dir.path(), &goal_id).unwrap();
    assert_eq!(entries.len(), 2);

    // Simulate a retroactive edit: flip entry[0].status from APPROVE to REJECT in memory,
    // WITHOUT recomputing its stored entryHash (exactly what an attacker who edits the
    // status field on disk would leave behind).
    entries[0].status = REJECT.to_string();

    // verify_chain recomputes entryHash internally and MUST detect that the recomputed
    // hash no longer matches the stored entryHash.
    let result = receipt::verify_chain(&entries);
    assert!(
        result.is_err(),
        "mid-log status edit MUST be detected: recomputed entryHash must differ from stored entryHash"
    );
}

#[test]
fn verify_chain_detects_broken_prev_hash_link() {
    let (dir, goal_id) = fresh_goal();

    receipt::append_receipt(dir.path(), &goal_id, "approve", "v1", APPROVE, "aa11bb22cc33dd44")
        .unwrap();
    receipt::append_receipt(dir.path(), &goal_id, "approve", "v2", APPROVE, "ee55ff6677889900")
        .unwrap();

    let mut entries = receipt::read_receipt_log(dir.path(), &goal_id).unwrap();

    // Simulate an attacker re-writing entry[0] (status + recomputed entryHash) but leaving
    // entry[1].prevHash pointing at the OLD entryHash. The link MUST break.
    entries[0].status = REJECT.to_string();
    entries[0].entry_hash = expected_entry_hash(&entries[0].prev_hash, entries[0].seq, &entries[0].kind, &entries[0].verdict_id, &entries[0].status);
    // entries[1].prev_hash still points at the original entry[0].entryHash.

    let result = receipt::verify_chain(&entries);
    assert!(
        result.is_err(),
        "broken prevHash link MUST be detected: entry[1].prevHash must equal recomputed entry[0].entryHash"
    );
}

#[test]
fn verify_chain_detects_trailing_deletion_via_head_mismatch() {
    let (dir, goal_id) = fresh_goal();

    receipt::append_receipt(dir.path(), &goal_id, "approve", "v1", APPROVE, "aa11bb22cc33dd44")
        .unwrap();
    let head_2 = receipt::append_receipt(
        dir.path(), &goal_id, "approve", "v2", APPROVE, "ee55ff6677889900"
    ).unwrap();

    // Capture the head that would be folded into a completion hash while the log is intact.
    assert_eq!(receipt::read_receipt_head(dir.path(), &goal_id), head_2);

    // Simulate trailing-line deletion: truncate the file to its first line only.
    let log_path = receipt_log_path(dir.path(), &goal_id);
    let original = fs::read_to_string(&log_path).unwrap();
    let first_line = original.lines().next().expect("log must have at least one line");
    fs::write(&log_path, format!("{first_line}\n")).unwrap();

    // After the deletion, the recomputed head MUST differ from the head folded into the
    // (hypothetically already-stored) completion hash — this is the deletion-detection path.
    let head_after_deletion = receipt::read_receipt_head(dir.path(), &goal_id);
    assert_ne!(
        head_after_deletion, head_2,
        "trailing-line deletion MUST change the chain head (else deletion is undetectable)"
    );

    // And the surviving single-entry chain MUST still verify on its own.
    let survivors = receipt::read_receipt_log(dir.path(), &goal_id).unwrap();
    assert_eq!(survivors.len(), 1);
    receipt::verify_chain(&survivors).expect("surviving chain must still be internally consistent");
}

#[test]
fn append_uses_pipe_separated_canonical_fields() {
    let (dir, goal_id) = fresh_goal();

    let signed_by = "0123456789abcdef";
    receipt::append_receipt(dir.path(), &goal_id, "approve", "v1", APPROVE, signed_by).unwrap();

    let entries = receipt::read_receipt_log(dir.path(), &goal_id).unwrap();
    let e = &entries[0];

    // PIN the canonical form explicitly so the GREEN team cannot drift.
    // entryHash = lowercase_hex( SHA256( prevHash + "|" + seq + "|" + kind + "|" + verdictId + "|" + status ) )
    let mut h = Sha256::new();
    h.update(format!("{}|{}|{}|{}|{}", e.prev_hash, e.seq, e.kind, e.verdict_id, e.status).as_bytes());
    let pinned = hex::encode(h.finalize());

    assert_eq!(
        e.entry_hash, pinned,
        "entryHash MUST be hex(SHA256(prevHash|seq|kind|verdictId|status)) — pipe-separated, no whitespace"
    );
    assert_eq!(pinned.len(), 64, "SHA256 hex digest must be 64 chars");
    assert!(pinned.chars().all(|c| c.is_ascii_lowercase()), "digest must be lowercase hex");
}

#[test]
fn append_creates_log_file_if_absent() {
    let (dir, goal_id) = fresh_goal();
    let log_path = receipt_log_path(dir.path(), &goal_id);

    assert!(!log_path.exists(), "log file MUST NOT exist before the first append");

    receipt::append_receipt(dir.path(), &goal_id, "approve", "v1", APPROVE, "aa11bb22cc33dd44")
        .expect("append must create the log file");

    assert!(log_path.exists(), "log file MUST be created by the first append");
}

#[test]
fn log_file_is_jsonl_one_object_per_line() {
    let (dir, goal_id) = fresh_goal();

    receipt::append_receipt(dir.path(), &goal_id, "approve", "v1", APPROVE, "aa11bb22cc33dd44")
        .unwrap();
    receipt::append_receipt(dir.path(), &goal_id, "reject", "v2", REJECT, "ee55ff6677889900")
        .unwrap();

    let log_path = receipt_log_path(dir.path(), &goal_id);
    let raw = fs::read_to_string(&log_path).unwrap();

    // Split into non-empty lines (tolerate a single trailing newline).
    let lines: Vec<&str> = raw.split('\n').filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 2, "exactly one JSON object per append");

    for (i, line) in lines.iter().enumerate() {
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line {i} must be valid JSON: {e}\nraw: {line}"));

        // Required keys per the spec contract.
        for key in ["seq", "kind", "verdictId", "status", "prevHash", "entryHash", "signedBy"] {
            assert!(
                v.get(key).is_some(),
                "line {i} is missing required key `{key}` (camelCase on disk)"
            );
        }
        // seq is a positive integer.
        let seq = v.get("seq").and_then(|s| s.as_u64());
        assert!(seq.is_some() && seq.unwrap() >= 1, "seq must be a positive integer");
    }

    // camelCase must be honored on disk (serde rename).
    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert!(first.get("verdictId").is_some(), "on-disk key MUST be `verdictId` (camelCase)");
    assert!(first.get("prevHash").is_some(), "on-disk key MUST be `prevHash` (camelCase)");
    assert!(first.get("entryHash").is_some(), "on-disk key MUST be `entryHash` (camelCase)");
    assert!(first.get("signedBy").is_some(), "on-disk key MUST be `signedBy` (camelCase)");
}

/// Resolve the on-disk receipt log path: `<root>/goals/<goal_id>/receipt-log.jsonl`.
///
/// Mirrors the layout the GREEN team is required to implement (per-goal, sibling to
/// goal.json / signature.json).
fn receipt_log_path(root: &Path, goal_id: &str) -> std::path::PathBuf {
    goal::goal_dir(root, goal_id).join("receipt-log.jsonl")
}
