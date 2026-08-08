//! Verifier-loop health self-awareness + cooldown mode (intention 2026-07-14 feature a).
//!
//! The underlying issue this module addresses: when a verifier sub-agent cannot produce a
//! result (no SID + no final output) OR exits with a non-success exit code, the spawn
//! layer leaves a null verdict (fail-closed) — correct per-run, but with no memory across
//! runs. Repeated backend failures across successive rounds/goals stall the main driving
//! process indefinitely. This module provides that memory.
//!
//! ## Model
//!
//! * [`is_run_unhealthy`] classifies a single [`crate::spawn::VerifierRun`]: unhealthy iff
//!   it produced no usable result (no SID AND no final output) OR the child exited with a
//!   non-success exit code OR it timed out.
//! * [`record_unhealthy_at`] / [`record_unhealthy`] append a timestamped event to the
//!   store-wide `health.jsonl` log.
//! * [`in_cooldown`] returns true iff MORE THAN 3 unhealthy events fall within a rolling
//!   1-hour window ending at `now` (configurable via [`cooldown_threshold`] /
//!   [`cooldown_window`]).
//! * [`fallback_hash_at`] / [`fallback_hash`] produce the non-blocking fallback hash
//!   `<mmddyy>-ffffff` returned in cooldown mode instead of spawning verifiers.
//!
//! ## Fail-closed vs cooldown
//!
//! Cooldown does NOT weaken the fail-closed invariants (NULL verdict never → APPROVE,
//! etc.). It is a *non-blocking* fallback: rather than spawning verifiers that will
//! almost certainly fail again and leave nulls, the CLI returns a recognizable fallback
//! hash so the outer driving process is not completely blocked. The `ffffff` suffix makes
//! the fallback unmistakable (a real consensus hash is `mmddyy-XXXXXXXX`, 8 mixed hex).

use std::fs;
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};

use crate::spawn::VerifierRun;

/// Store-wide unhealthy-event log filename (one JSON line per event).
pub const HEALTH_LOG: &str = "health.jsonl";

/// Cooldown trips when the unhealthy-event count within the rolling window EXCEEDS this
/// threshold (i.e. strictly more than 3 → the 4th within the window trips it).
pub const COOLDOWN_THRESHOLD: usize = 3;

/// Rolling cooldown window. Events older than this relative to `now` do not count.
pub const COOLDOWN_WINDOW_SECS: i64 = 3600; // 1 hour

/// Cooldown fallback hash suffix. Visually distinct from any real 8-hex digest.
pub const FALLBACK_SUFFIX: &str = "ffffff";

/// Classify a verifier run as unhealthy.
///
/// Unhealthy iff any of:
///   * timed out (killed by `verifierTimeoutSec`),
///   * produced NO usable result (no SID AND no final output), OR
///   * the child exited with a non-success (non-zero) exit code.
///
/// A run that captured a SID or final output AND exited zero is healthy even if stderr
/// was present (a chatty but functioning backend).
pub fn is_run_unhealthy(run: &VerifierRun) -> bool {
    if run.timed_out {
        return true;
    }
    // No usable result: backend produced neither a session id nor any final output.
    let no_result = run.sid.is_none() && run.final_output.is_none();
    if no_result {
        return true;
    }
    // Non-success exit code (a crash / explicit failure).
    matches!(run.exit_code, Some(code) if code != 0)
}

/// Append an unhealthy event to the store's `health.jsonl` with an explicit timestamp.
/// Creates the file (and parent dirs) if absent. Each event is one JSON line:
/// `{"event":"unhealthy","at":"<rfc3339>"}`.
pub fn record_unhealthy_at(root: &Path, at: DateTime<Utc>) -> Result<(), std::io::Error> {
    let line = serde_json::json!({
        "event": "unhealthy",
        "at": at.to_rfc3339(),
    });
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(HEALTH_LOG))?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Append an unhealthy event timestamped at the current wall-clock time.
pub fn record_unhealthy(root: &Path) -> Result<(), std::io::Error> {
    record_unhealthy_at(root, Utc::now())
}

/// Cooldown threshold (events). Trips when the in-window count is **strictly greater**
/// than this value. Defaults to [`COOLDOWN_THRESHOLD`] (3 → 4 trips).
pub fn cooldown_threshold() -> usize {
    COOLDOWN_THRESHOLD
}

/// Cooldown rolling window length. Defaults to [`COOLDOWN_WINDOW_SECS`] (1 hour).
pub fn cooldown_window() -> Duration {
    Duration::seconds(COOLDOWN_WINDOW_SECS)
}

/// Returns true iff the store is in cooldown at `now`: strictly more than
/// [`cooldown_threshold`] unhealthy events fall within `[now - cooldown_window, now]`.
///
/// A missing or unreadable `health.jsonl` is treated as zero events (not cooldown).
pub fn in_cooldown(root: &Path, now: DateTime<Utc>) -> bool {
    count_recent(root, now) > cooldown_threshold()
}

/// The non-blocking fallback hash `<mmddyy>-ffffff` for `at` (UTC date of `at`).
///
/// The prefix follows the existing completion-hash `mmddyy` convention
/// (`MMDDYY` of the UTC date); the `ffffff` suffix marks it as a cooldown fallback,
/// visually distinct from a real `mmddyy-XXXXXXXX` consensus hash.
pub fn fallback_hash_at(at: DateTime<Utc>) -> String {
    format!("{}-{}", at.format("%m%d%y"), FALLBACK_SUFFIX)
}

/// The cooldown fallback hash for the current wall-clock time.
pub fn fallback_hash() -> String {
    fallback_hash_at(Utc::now())
}

/// Count unhealthy events within the rolling window ending at `now`. Best-effort: a
/// missing/malformed log line is skipped, never fatal.
fn count_recent(root: &Path, now: DateTime<Utc>) -> usize {
    let window_start = now - cooldown_window();
    let Ok(raw) = fs::read_to_string(root.join(HEALTH_LOG)) else {
        return 0;
    };
    let mut count = 0usize;
    for line in raw.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_hash_format_and_day_stability() {
        let a = DateTime::parse_from_rfc3339("2026-07-14T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let b = DateTime::parse_from_rfc3339("2026-07-14T23:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(fallback_hash_at(a), "071426-ffffff");
        assert_eq!(fallback_hash_at(a), fallback_hash_at(b));
        // Day rollover changes the prefix.
        let c = DateTime::parse_from_rfc3339("2026-07-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(fallback_hash_at(c), "071526-ffffff");
    }
}
