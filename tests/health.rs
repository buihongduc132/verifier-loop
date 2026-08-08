// Health self-awareness + cooldown mode (intention 2026-07-14 feature a).
// RED phase: written FIRST, against the intent contract, BEFORE any implementation.
//
// Contract:
//   * A verifier run is "unhealthy" when it produced no usable result (no SID captured
//     AND no final output) OR the child exited with a non-success exit code.
//   * Unhealthy events are appended to `<store>/health.jsonl` with an RFC3339 timestamp.
//   * If MORE THAN 3 unhealthy events occur within a rolling 1-HOUR window, the store is
//     in cooldown.
//   * In cooldown mode the CLI returns a fallback hash `<mmddyy>-ffffff` instead of
//     spawning verifiers (non-blocking fallback for the driving process).

use std::fs;

use verifier_loop::health;

/// Append a fresh unhealthy event with `now` to the store's health log.
fn append_event(root: &std::path::Path, now: chrono::DateTime<chrono::Utc>) {
    health::record_unhealthy_at(root, now).expect("append unhealthy event");
}

#[test]
fn zero_events_is_not_cooldown() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!health::in_cooldown(dir.path(), chrono::Utc::now()));
}

#[test]
fn three_events_within_one_hour_is_not_cooldown() {
    // "more than 3" => 3 itself is NOT cooldown; the 4th within the window trips it.
    let dir = tempfile::tempdir().unwrap();
    let base = chrono::Utc::now();
    append_event(dir.path(), base);
    append_event(dir.path(), base);
    append_event(dir.path(), base);
    assert!(
        !health::in_cooldown(dir.path(), base),
        "exactly 3 events must NOT trip cooldown (needs MORE than 3)"
    );
}

#[test]
fn four_events_within_one_hour_trips_cooldown() {
    let dir = tempfile::tempdir().unwrap();
    let base = chrono::Utc::now();
    for _ in 0..4 {
        append_event(dir.path(), base);
    }
    assert!(
        health::in_cooldown(dir.path(), base),
        "4 events within 1h must trip cooldown"
    );
}

#[test]
fn events_outside_one_hour_window_do_not_count() {
    let dir = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now();
    // 4 events, but all >1h old.
    let stale = now - chrono::Duration::seconds(3700);
    for _ in 0..4 {
        append_event(dir.path(), stale);
    }
    assert!(
        !health::in_cooldown(dir.path(), now),
        "stale events (>1h) must not trip cooldown"
    );
}

#[test]
fn mixed_stale_and_recent_events_count_only_recent() {
    let dir = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now();
    // 4 stale events (>1h) + only 2 recent => not cooldown.
    let stale = now - chrono::Duration::seconds(3700);
    for _ in 0..4 {
        append_event(dir.path(), stale);
    }
    append_event(dir.path(), now);
    append_event(dir.path(), now);
    assert!(!health::in_cooldown(dir.path(), now));
    // Add 2 more recent => 4 recent total => cooldown.
    append_event(dir.path(), now);
    append_event(dir.path(), now);
    assert!(health::in_cooldown(dir.path(), now));
}

#[test]
fn record_unhealthy_writes_jsonl_line_with_timestamp() {
    let dir = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now();
    health::record_unhealthy_at(dir.path(), now).unwrap();
    let log = fs::read_to_string(dir.path().join("health.jsonl")).unwrap();
    let line = log.trim();
    let v: serde_json::Value = serde_json::from_str(line).expect("valid json line");
    assert_eq!(v["event"], "unhealthy");
    assert!(v["at"].is_string(), "at field present");
}

#[test]
fn fallback_hash_is_mmddyy_ffffff_format() {
    // The cooldown fallback hash must be `<mmddyy>-ffffff`.
    let now = chrono::Utc::now();
    let h = health::fallback_hash_at(now);
    assert!(
        h.ends_with("-ffffff"),
        "fallback hash must end with -ffffff: got {h}"
    );
    // The prefix is the mmddyy of `now` (UTC MMDDYY).
    let expected_prefix = now.format("%m%d%y").to_string();
    assert!(
        h.starts_with(&expected_prefix),
        "fallback hash prefix {h} must start with mmddyy {expected_prefix}"
    );
}

#[test]
fn fallback_hash_is_deterministic_per_day() {
    // Two calls within the same UTC day yield the same fallback hash.
    let a = chrono::Utc::now();
    let b = a + chrono::Duration::seconds(10);
    assert_eq!(health::fallback_hash_at(a), health::fallback_hash_at(b));
}

#[test]
fn is_run_unhealthy_flags_no_result_and_bad_exit_code() {
    use verifier_loop::spawn::VerifierRun;
    // No SID, no final output => unhealthy regardless of exit code.
    let no_result = VerifierRun {
        verifier_id: "v1".into(),
        sid: None,
        final_output: None,
        stderr: None,
        timed_out: false,
        exit_code: Some(0),
    };
    assert!(
        health::is_run_unhealthy(&no_result),
        "no SID + no output must be unhealthy even with exit 0"
    );
    // Has SID + output but exited non-zero => unhealthy.
    let bad_exit = VerifierRun {
        verifier_id: "v1".into(),
        sid: Some("s".into()),
        final_output: Some("o".into()),
        stderr: None,
        timed_out: false,
        exit_code: Some(2),
    };
    assert!(
        health::is_run_unhealthy(&bad_exit),
        "non-zero exit must be unhealthy even with output"
    );
    // Timed out => unhealthy.
    let timed = VerifierRun {
        verifier_id: "v1".into(),
        sid: None,
        final_output: None,
        stderr: None,
        timed_out: true,
        exit_code: None,
    };
    assert!(health::is_run_unhealthy(&timed), "timeout must be unhealthy");
}

#[test]
fn is_run_healthy_when_it_produced_result_and_zero_exit() {
    use verifier_loop::spawn::VerifierRun;
    let healthy = VerifierRun {
        verifier_id: "v1".into(),
        sid: Some("s".into()),
        final_output: Some("o".into()),
        stderr: None,
        timed_out: false,
        exit_code: Some(0),
    };
    assert!(!health::is_run_unhealthy(&healthy));
}
