//! Integration tests for traceId propagation (add-otel-observability, design D2/D4).
//!
//! RED phase for tasks §6. Pins:
//!   * receipt-log entries record `traceId` (camelCase) but it is EXCLUDED from
//!     `entryHash` (design D4) — two entries identical except traceId produce the
//!     same entryHash.
//!   * `jewilo` spawn injects `VERIFIER_LOOP_TRACE_ID` into every V* child env
//!     (verifier-spawn MODIFIED spec).
//!   * manual `jewije` without the env var mints a fallback traceId that is NOT
//!     persisted to trace-id (trace-export spec).
//!
//! These tests mutate `VERIFIER_LOOP_TRACE_ID` in the process env. Because cargo
//! runs unit/integration tests in parallel threads within one process, a process-
//! wide env var is shared across tests. We serialize these tests with a module-
//! level mutex so the set/remove sequence is atomic per test. (This is the same
//! constraint that motivates the `serial_test` crate; we inline a minimal mutex.)

use std::fs;
use std::sync::Mutex;

use tempfile::tempdir;
use verifier_loop::receipt;

/// Process-wide lock: only one env-mutating trace test runs at a time.
static ENV_LOCK: Mutex<()> = Mutex::new(());

// ── traceId recorded on receipt entry, EXCLUDED from entryHash (design D4) ────

#[test]
fn receipt_entry_records_trace_id_when_env_set() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("VERIFIER_LOOP_TRACE_ID", "aaaabbbbccccdddd1111222233334444");
    let store = tempdir().unwrap();
    std::fs::create_dir_all(store.path().join("goals").join("g-rec")).unwrap();

    let head = receipt::append_receipt(
        store.path(),
        "g-rec",
        "approve",
        "v1",
        "APPROVE",
        "pubkey-abc",
    )
    .expect("append ok");
    assert!(!head.is_empty(), "entryHash must be non-empty");

    let entries = receipt::read_receipt_log(store.path(), "g-rec").unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(
        e.trace_id.as_deref(),
        Some("aaaabbbbccccdddd1111222233334444"),
        "receipt entry must record the active traceId from env"
    );

    std::env::remove_var("VERIFIER_LOOP_TRACE_ID");
}

#[test]
fn trace_id_excluded_from_entry_hash() {
    let _guard = ENV_LOCK.lock().unwrap();
    let store = tempdir().unwrap();
    std::fs::create_dir_all(store.path().join("goals").join("g-excl")).unwrap();

    // Two appends with DIFFERENT traceId but identical canonical fields.
    std::env::set_var("VERIFIER_LOOP_TRACE_ID", "11111111111111111111111111111111");
    let head1 =
        receipt::append_receipt(store.path(), "g-excl", "approve", "v1", "APPROVE", "pk").unwrap();
    std::env::remove_var("VERIFIER_LOOP_TRACE_ID");

    // Second store to get a fresh chain with a different traceId.
    let store2 = tempdir().unwrap();
    std::fs::create_dir_all(store2.path().join("goals").join("g-excl")).unwrap();
    std::env::set_var("VERIFIER_LOOP_TRACE_ID", "22222222222222222222222222222222");
    let head2 =
        receipt::append_receipt(store2.path(), "g-excl", "approve", "v1", "APPROVE", "pk").unwrap();
    std::env::remove_var("VERIFIER_LOOP_TRACE_ID");

    assert_eq!(
        head1, head2,
        "entryHash must NOT depend on traceId (design D4); identical canonical fields → identical hash"
    );
}

#[test]
fn receipt_entry_trace_id_optional_when_env_unset() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var("VERIFIER_LOOP_TRACE_ID");
    let store = tempdir().unwrap();
    std::fs::create_dir_all(store.path().join("goals").join("g-opt")).unwrap();

    receipt::append_receipt(store.path(), "g-opt", "approve", "v1", "APPROVE", "pk").unwrap();

    let entries = receipt::read_receipt_log(store.path(), "g-opt").unwrap();
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].trace_id.is_none(),
        "traceId must be None/absent when env unset (backward compat)"
    );

    // The serialized JSON line must NOT contain a traceId key (skip_serializing_if).
    let raw = std::fs::read_to_string(
        store
            .path()
            .join("goals")
            .join("g-opt")
            .join("receipt-log.jsonl"),
    )
    .unwrap();
    assert!(
        !raw.contains("traceId"),
        "no traceId key when env unset; got: {raw}"
    );
}
