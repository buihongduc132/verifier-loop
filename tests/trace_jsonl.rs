//! Integration tests for the per-goal JSONL trace file (trace-export spec, tasks §4).

use std::fs;

use serde_json::Value;
use tempfile::tempdir;
use verifier_loop::observe;

#[test]
fn append_trace_event_creates_per_goal_file() {
    let store = tempdir().unwrap();
    let goal = "g-create";
    // First append creates the file with a JSON line.
    observe::append_trace_event(
        store.path(),
        goal,
        "info",
        "jewilo.round.start",
        serde_json::json!({"kind": "NEW"}),
    )
    .expect("append ok");

    let path = observe::trace_jsonl_path(store.path(), goal);
    assert!(path.exists(), "trace.jsonl must be created on first append");
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.ends_with('\n'), "file must end with newline");
    let line: Value = serde_json::from_str(raw.trim()).unwrap();
    // Required camelCase keys.
    assert!(line.get("timestamp").is_some(), "must have timestamp");
    assert_eq!(line.get("level").and_then(|v| v.as_str()), Some("info"));
    // traceId is minted by append_trace_event via ensure_goal_trace_id → non-empty.
    let tid = line.get("traceId").and_then(|v| v.as_str()).unwrap();
    assert!(
        !tid.is_empty(),
        "traceId must be non-empty (minted on first append)"
    );
    assert_eq!(line.get("goalId").and_then(|v| v.as_str()), Some(goal));
    assert_eq!(
        line.get("event").and_then(|v| v.as_str()),
        Some("jewilo.round.start")
    );
    // No snake_case keys.
    let obj = line.as_object().unwrap();
    assert!(
        !obj.keys().any(|k| k.contains('_')),
        "no snake_case keys; got {:?}",
        obj.keys().collect::<Vec<_>>()
    );
}

#[test]
fn append_trace_event_appends_not_truncates() {
    let store = tempdir().unwrap();
    let goal = "g-append";
    for i in 0..3 {
        observe::append_trace_event(
            store.path(),
            goal,
            "info",
            &format!("e{i}"),
            serde_json::json!({"i": i}),
        )
        .unwrap();
    }
    let raw = fs::read_to_string(observe::trace_jsonl_path(store.path(), goal)).unwrap();
    let count = raw.lines().filter(|l| !l.is_empty()).count();
    assert_eq!(count, 3, "three appends → three lines (not truncated)");
}

#[test]
fn append_trace_event_with_trace_id_records_it() {
    let store = tempdir().unwrap();
    let goal = "g-tid";
    observe::append_trace_event(store.path(), goal, "info", "e", serde_json::json!({})).unwrap();
    // Pre-populate a trace-id and append again — the new line carries it.
    observe::ensure_goal_trace_id(store.path(), goal).unwrap();
    let tid = fs::read_to_string(store.path().join("goals").join(goal).join("trace-id")).unwrap();
    observe::append_trace_event(store.path(), goal, "info", "e2", serde_json::json!({})).unwrap();
    let raw = fs::read_to_string(observe::trace_jsonl_path(store.path(), goal)).unwrap();
    let last_line: Value = serde_json::from_str(raw.lines().last().unwrap()).unwrap();
    assert_eq!(
        last_line.get("traceId").and_then(|v| v.as_str()),
        Some(tid.as_str()),
        "traceId must be read from the persisted trace-id file"
    );
}

#[test]
fn append_trace_event_best_effort_on_unwritable_store() {
    // Pointing at a non-creatable path: the append fails but returns Ok(()) — fail-open.
    // (We cannot easily make a read-only dir across all CI envs, so assert the helper
    // returns Ok even on a bogus nested path that mkdir fails to create.)
    let res = observe::append_trace_event(
        std::path::Path::new("/proc/cannot/create/here"),
        "x",
        "info",
        "e",
        serde_json::json!({}),
    );
    assert!(
        res.is_ok(),
        "append must be fail-open (swallow io errors), got {res:?}"
    );
}
