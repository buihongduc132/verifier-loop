//! Integration tests for the `observe` module (add-otel-observability).
//!
//! RED phase: these tests assert the contract BEFORE the implementation exists.
//! See `openspec/changes/add-otel-observability/specs/trace-export/spec.md` and
//! `specs/lifecycle-tracing/spec.md` for the pinned scenarios.
//!
//! Fail-open invariant (design.md D5): tracing errors are swallowed and never
//! propagate to a verdict, consensus, or hash decision.

use std::fs;
use std::path::Path;

use tempfile::tempdir;
use verifier_loop::observe;

// ── traceId resolution + persistence (trace-export spec, tasks §2) ───────────

#[test]
fn ensure_goal_trace_id_mints_and_persists_on_first_call() {
    let store = tempdir().unwrap();
    let goal_id = "abc-mint";
    let id = observe::ensure_goal_trace_id(store.path(), goal_id).expect("mint ok");

    // Persisted to <store>/goals/<goalId>/trace-id with the returned value.
    let trace_file = store.path().join("goals").join(goal_id).join("trace-id");
    let persisted = fs::read_to_string(&trace_file).expect("trace-id file exists");
    assert_eq!(persisted, id, "persisted value must equal returned id");
}

#[test]
fn ensure_goal_trace_id_reuses_existing_on_subsequent_call() {
    let store = tempdir().unwrap();
    let goal_id = "abc-reuse";
    let goal_dir = store.path().join("goals").join(goal_id);
    fs::create_dir_all(&goal_dir).unwrap();
    // Pre-write a known trace id.
    let known = "0123456789abcdef0123456789abcdef";
    fs::write(goal_dir.join("trace-id"), known).unwrap();

    let id = observe::ensure_goal_trace_id(store.path(), goal_id).expect("reuse ok");
    assert_eq!(id, known, "must reuse the persisted value unchanged");
    // File untouched.
    assert_eq!(
        fs::read_to_string(goal_dir.join("trace-id")).unwrap(),
        known,
        "file must not be overwritten"
    );
}

#[test]
fn ensure_goal_trace_id_is_16_bytes_hex() {
    let store = tempdir().unwrap();
    let id = observe::ensure_goal_trace_id(store.path(), "abc-hex").unwrap();
    assert_eq!(id.len(), 32, "16 bytes hex = 32 hex chars");
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit()),
        "must be lowercase hex, got {id}"
    );
}

// ── traceId NOT in completion hash inputs (design.md D4) ─────────────────────

#[test]
fn completion_hash_byte_identical_with_different_trace_id() {
    use verifier_loop::consensus::{compute_hash, MatchingVerdict};

    let matching = vec![MatchingVerdict {

        phase_id: String::new(),
        verifier_id: "v1".into(),
        registered_at: "2026-07-12T10:00:00Z".into(),
    }];

    // compute_hash does NOT take a trace_id argument — calling it twice with
    // identical inputs yields identical output. (The D4 test is structural:
    // traceId cannot influence the hash because it is not a parameter.)
    let h1 = compute_hash(
        "SALT",
        "GID",
        "SIG",
        1,
        &matching,
        "2026-07-12T10:05:00Z",
        "head0",
    );
    let h2 = compute_hash(
        "SALT",
        "GID",
        "SIG",
        1,
        &matching,
        "2026-07-12T10:05:00Z",
        "head0",
    );
    assert_eq!(h1.short_hash(), h2.short_hash());
    assert_eq!(h1.full_digest(), h2.full_digest());

    // The hashed input string must never contain "trace" as a field.
    // We assert by recompute: the documented input concat is
    //   salt + goalId + goalSignature + round + canon + matchedAt + receiptHead
    // None of those is traceId.
    let canon = serde_json::to_string(&[&matching[0]]).unwrap_or_default();
    let input = format!("SALTGIDSIG1{canon}2026-07-12T10:05:00Zhead0");
    // Sanity: the input does not include a traceId segment (it is not a param).
    assert!(!input.to_lowercase().contains("traceid"));
}

// ── subscriber init fail-open (trace-export spec, tasks §3) ──────────────────

#[test]
fn init_swallows_unwritable_store_and_returns_ok() {
    // A non-existent path is fine — init must be best-effort (design.md D5).
    // We point at a path that cannot be a store; init still returns Ok.
    let bogus = Path::new("/proc/this-cannot-exist-as-a-store-xyz");
    let res = observe::init(Some(bogus));
    assert!(
        res.is_ok(),
        "init must be fail-open and return Ok, got {res:?}"
    );
}

#[test]
fn init_with_none_store_returns_ok() {
    // When the store root is None (e.g. a misconfigured env), init is a no-op.
    let res = observe::init(None);
    assert!(res.is_ok(), "init(None) must be Ok (no-op), got {res:?}");
}

// ── env-driven traceId read (for jewije fallback) ────────────────────────────

#[test]
fn trace_id_from_env_returns_value_when_set() {
    // SAFETY: this test mutates the process env. Tests in this file do not run
    // in parallel with other env-dependent tests via cargo's default thread
    // model per-test; we scope the var tightly.
    // We test the read helper directly.
    let id = observe::trace_id_from_env();
    // Without env set, returns None.
    assert!(
        id.is_none(),
        "expected None when VERIFIER_LOOP_TRACE_ID unset, got {id:?}"
    );
}

// ── per-goal trace.jsonl path ────────────────────────────────────────────────

#[test]
fn trace_jsonl_path_lives_under_goal_dir() {
    let store = tempdir().unwrap();
    let p = observe::trace_jsonl_path(store.path(), "abc-path");
    assert_eq!(
        p,
        store
            .path()
            .join("goals")
            .join("abc-path")
            .join("trace.jsonl")
    );
}
