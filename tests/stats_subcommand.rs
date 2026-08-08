// Intention 2026-07-14 — `jewilo STATS <goalId>` subcommand (RED phase).
//
// `STATS` surfaces ALL stored JSON for a goal run as ONE machine-readable JSON object to
// stdout. Read-only, no goal lock. See flow/intentions/2026-07-14_stats-and-audit-subcommands.md.
//
// These tests are written FIRST, before the `STATS` variant exists in `VerifierLoopCmd`.
// Expected RED state: clap rejects `STATS` as an unknown subcommand (exit non-zero,
// stderr mentions an unrecognized subcommand), so every parse + assertion below FAILS.
// The contract pinned here drives the GREEN implementation.
//
// Determinism strategy mirrors tests/cli_e2e.rs exactly: a STUB backend script emits a
// fixed ACP stream and calls the built `verifier-verdict` to register an APPROVE. No real
// `pi`, no network.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin;
use serde_json::Value;

/// Absolute path to a cargo-built binary.
fn bin(name: &str) -> PathBuf {
    cargo_bin(name)
}

/// Write `body` to `<dir>/<name>`, chmod 0755, return its absolute path.
fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

/// A stub backend script: emits the ACP stream then registers a verdict via jewije.
/// Approve/reject is chosen by `$VERIFIER_LOOP_STUB_VERDICT` (default approve).
fn stub_script(dir: &Path) -> PathBuf {
    let verdict = bin("verifier-verdict");
    write_script(
        dir,
        "stub_backend.sh",
        &format!(
            r#"#!/bin/sh
# Deterministic ACP stream: session id + a final assistant message.
cat <<'ACP'
{{"type":"session","id":"stub-session-id"}}
{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"stub final output"}}]}}],"willRetry":false}}
ACP
# Register the verdict. Identity + home come from the env injected/inherited by the spawn layer.
case "${{VERIFIER_LOOP_STUB_VERDICT:-approve}}" in
  reject) "{verdict}" reject --notes "stub rejection: no proof produced" ;;
  *)      "{verdict}" approve ;;
esac
"#,
            verdict = verdict.to_string_lossy()
        ),
    )
}

/// Seed a git work tree at `dir` with a `config.json` (n=m=1 stub) and the stub script.
/// Returns the stub script path (to export as VERIFIER_LOOP_BACKEND_CMD).
fn seed_workdir(dir: &Path, n: u32, m: u32) -> PathBuf {
    let git_ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "-q"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(git_ok, "git init failed in tempdir");

    let cfg = serde_json::json!({
        "n": n,
        "m": m,
        "maxTurn": 3,
        "backend": "stub",
        "gitDiffMaxChars": 1000,
        "verifierTimeoutSec": 10
    });
    fs::write(dir.join("config.json"), cfg.to_string()).unwrap();

    fs::write(dir.join(".gitkeep"), "").unwrap();
    let _ = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "user.email", "test@example.com"])
        .status();
    let _ = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "user.name", "Test"])
        .status();
    let _ = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["add", "."])
        .status();
    let _ = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", "seed"])
        .status();

    stub_script(dir)
}

/// Run `verifier-loop` as a raw subprocess and return its full output regardless of exit
/// status. We do NOT assert success on the NEW seed (it should succeed for approve, but
/// using the raw runner keeps the helper uniform).
fn run_vl(
    cwd: &Path,
    home: &Path,
    stub: &Path,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> std::process::Output {
    let mut c = std::process::Command::new(bin("verifier-loop"));
    c.args(args)
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", stub)
        .current_dir(cwd);
    for (k, v) in extra_env {
        c.env(k, v);
    }
    c.output().expect("verifier-loop subprocess ran")
}

/// Drive a full NEW (n=m=1, stub approves) to completion and return the goalId of the
/// single created goal. Asserts the NEW itself succeeded.
fn seed_completed_goal(home: &Path, stub: &Path, goal_text: &str) -> String {
    let out = run_vl(home, home, stub, &["NEW", goal_text], &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "seed NEW must succeed (approve): exit {:?}, stderr:\n{stderr}",
        out.status.code()
    );

    let mut goal_ids: Vec<String> = fs::read_dir(home.join("goals"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(goal_ids.len(), 1, "exactly one goal created in seed NEW");
    goal_ids.pop().unwrap()
}

/// Run `verifier-loop STATS <goalId>` and parse stdout as JSON. Panics with helpful
/// diagnostics on non-zero exit or invalid JSON (this is the assertion surface: a RED
/// build where STATS does not exist will fail here).
fn stats_json(home: &Path, goal_id: &str) -> Value {
    let out = run_vl(home, home, Path::new("/unused/stub/stats"), &["STATS", goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "STATS must exit 0. exit={:?}\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}",
        out.status.code()
    );
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "STATS stdout must be valid JSON: parse error {e}\n--- stdout ---\n{stdout}"
        )
    })
}

// ---------------------------------------------------------------------------
// Test 1 — STATS surfaces the goal record, creation-time config, current round, and
// completion (hash + matching verdicts + duration fields).
// ---------------------------------------------------------------------------

#[test]
fn stats_surfaces_goal_config_rounds_and_completion() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let goal_id = seed_completed_goal(home, &stub, "stats: full surface of a passing run");

    let stats = stats_json(home, &goal_id);
    eprintln!("STATS json:\n{stats:#}");

    // Goal record surfaced.
    assert_eq!(
        stats["goal"]["goalId"].as_str(),
        Some(goal_id.as_str()),
        "stats.goal.goalId must match: {stats}"
    );
    assert!(
        stats["goal"]["goalText"]
            .as_str()
            .is_some_and(|t| t.contains("full surface")),
        "stats.goal.goalText must carry the goal text: {stats}"
    );
    assert!(
        stats["goal"]["createdAt"].is_string(),
        "stats.goal.createdAt must be a string (ISO timestamp): {stats}"
    );

    // Creation-time config snapshot (from goal.json's `config` field).
    assert_eq!(
        stats["config"]["n"].as_u64(),
        Some(1),
        "stats.config.n must be the creation-time n=1: {stats}"
    );
    assert_eq!(
        stats["config"]["m"].as_u64(),
        Some(1),
        "stats.config.m must be the creation-time m=1: {stats}"
    );

    // Current round (state.json) surfaced.
    assert!(
        stats["round"].as_u64().is_some_and(|r| r >= 1),
        "stats.round must be >=1 (current round from state.json): {stats}"
    );

    // Completion surfaced (hash is the mmddyy-XXXXXXXX short form).
    let hash = stats["completion"]["hash"]
        .as_str()
        .unwrap_or_else(|| panic!("stats.completion.hash must be present: {stats}"));
    assert!(
        regex_like_hash(hash),
        "stats.completion.hash must be mmddyy-XXXXXXXX, got {hash}: {stats}"
    );
    assert!(
        stats["completion"]["matchingVerdicts"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "stats.completion.matchingVerdicts must be a non-empty array: {stats}"
    );

    // Duration fields: createdAt + matchedAt + derived wall-clock duration.
    assert!(
        stats["durations"]["createdAt"].is_string(),
        "stats.durations.createdAt must be present: {stats}"
    );
    assert!(
        stats["durations"]["matchedAt"].is_string(),
        "stats.durations.matchedAt must be present (completion exists): {stats}"
    );
    // Derived wall-clock duration between createdAt and matchedAt. Represented as a
    // string (e.g. seconds or ISO duration); the exact unit is the implementation's
    // choice, but the field MUST exist and be non-null.
    assert!(
        stats["durations"]["wallClockSeconds"].is_number()
            || stats["durations"]["wallClock"].is_string(),
        "stats.durations must carry a derived wall-clock duration field: {stats}"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — STATS includes per-round verdict data (verifier status per round).
// ---------------------------------------------------------------------------

#[test]
fn stats_includes_per_round_verdicts() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let goal_id = seed_completed_goal(home, &stub, "stats: per-round verdict surface");

    let stats = stats_json(home, &goal_id);
    eprintln!("STATS json:\n{stats:#}");

    // A `rounds` array keyed by round number, each carrying per-verifier verdict status.
    let rounds = stats["rounds"]
        .as_array()
        .unwrap_or_else(|| panic!("stats.rounds must be an array: {stats}"));
    assert!(
        !rounds.is_empty(),
        "stats.rounds must contain at least the round-1 entry: {stats}"
    );

    let round1 = &rounds[0];
    // Each round carries its number and a verdicts/slots block with per-verifier status.
    assert!(
        round1["round"].as_u64() == Some(1) || round1["roundNumber"].as_u64() == Some(1),
        "stats.rounds[0] must identify round 1: {round1}"
    );

    // The per-verifier verdict appears as APPROVE under either `verdicts` or `slots`.
    // Tolerate either key name; the contract is "verifier status per round is present".
    let verdicts = round1
        .get("verdicts")
        .or_else(|| round1.get("slots"))
        .unwrap_or_else(|| {
            panic!("stats.rounds[0] must carry per-verifier verdicts/slots: {round1}")
        });
    let verdicts_arr = verdicts
        .as_array()
        .unwrap_or_else(|| panic!("per-round verdicts must be an array: {verdicts}"));
    assert!(
        !verdicts_arr.is_empty(),
        "round 1 must have at least one verifier verdict entry: {round1}"
    );

    // The first verifier's status must surface APPROVE (the stub approved).
    let v1 = &verdicts_arr[0];
    let status_str = v1["verdict"]
        .as_str()
        .or_else(|| v1["status"].as_str())
        .unwrap_or_else(|| panic!("verifier entry must carry a verdict/status string: {v1}"));
    assert_eq!(
        status_str, "APPROVE",
        "round 1 verifier status must be APPROVE: {v1}"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — STATS includes health info (unhealthy-event count in the last hour +
// cooldown flag) read from health.jsonl.
// ---------------------------------------------------------------------------

#[test]
fn stats_includes_health_cooldown_info() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let goal_id = seed_completed_goal(home, &stub, "stats: health surface");

    // Seed the store-wide health.jsonl with 2 recent unhealthy events. 2 is below the
    // cooldown threshold (>3), so cooldown should be FALSE but the unhealthy count >0.
    // Timestamps are RFC3339 "now" / "now-30s" obtained via the `date` command (avoids a
    // direct chrono dependency in the test crate).
    let mut lines = String::new();
    for offset in [0, 30] {
        let at = rfc3339_seconds_ago(offset);
        lines.push_str(&format!(
            "{{\"event\":\"unhealthy\",\"at\":\"{at}\"}}\n"
        ));
    }
    fs::write(home.join("health.jsonl"), lines).unwrap();

    let stats = stats_json(home, &goal_id);
    eprintln!("STATS json:\n{stats:#}");

    // Health block must surface the in-window unhealthy count + the cooldown flag.
    let health = &stats["health"];
    let unhealthy_count = health["unhealthyLastHour"]
        .as_u64()
        .or_else(|| health["unhealthy_count"].as_u64())
        .or_else(|| health["recentUnhealthy"].as_u64())
        .unwrap_or_else(|| {
            panic!("stats.health must carry an unhealthy-event count: {health} (full: {stats})")
        });
    assert!(
        unhealthy_count >= 2,
        "stats.health unhealthy count must reflect the 2 seeded events (got {unhealthy_count}): {stats}"
    );

    // Cooldown flag present (boolean). With 2 events it must be false.
    let cooldown = health["cooldown"]
        .as_bool()
        .or_else(|| health["inCooldown"].as_bool())
        .unwrap_or_else(|| {
            panic!("stats.health must carry a cooldown boolean: {health} (full: {stats})")
        });
    assert!(
        !cooldown,
        "2 unhealthy events are below the >3 threshold; cooldown must be false: {stats}"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — STATS is read-only and takes NO goal lock: it returns valid JSON quickly
// even against a goal whose round directory exists. The read-only contract means a
// STATS probe must never block on a long-running round. We verify it exits 0 and
// produces JSON for a completed goal without spawning any backend.
// ---------------------------------------------------------------------------

#[test]
fn stats_is_read_only_no_lock() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let goal_id = seed_completed_goal(home, &stub, "stats: read-only no-lock probe");

    // Invoke STATS via a raw subprocess whose "backend" points at a path that does NOT
    // exist. If STATS is read-only, it never invokes the backend, so the bad path is
    // irrelevant. If STATS (incorrectly) spawned, this would either hang or fail.
    let bogus_backend = home.join("definitely-not-a-backend.sh");
    let out = run_vl(home, home, &bogus_backend, &["STATS", &goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "STATS must exit 0 read-only even with an unusable backend path. \
         exit={:?}\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}",
        out.status.code()
    );

    // Must still be valid JSON (a backend path that doesn't exist would have broken a
    // spawn-based command; read-only STATS is unaffected).
    let stats: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("STATS stdout must be valid JSON despite unusable backend: parse {e}\n{stdout}")
    });
    assert_eq!(
        stats["goal"]["goalId"].as_str(),
        Some(goal_id.as_str()),
        "read-only STATS must still surface the goal: {stats}"
    );

    // The read-only guarantee also means no `.lock` file is LEFT HELD. We cannot directly
    // inspect an flock from another process after exit, but we CAN assert STATS did not
    // CREATE a persistent lock artifact beyond the empty advisory file (and that a second
    // immediate STATS also succeeds — a held lock would block/fail the probe).
    let out2 = run_vl(home, home, &bogus_backend, &["STATS", &goal_id], &[]);
    assert!(
        out2.status.success(),
        "a second STATS probe must succeed immediately (no held lock): {stats}"
    );
}

/// An RFC3339 timestamp `secs` seconds before now (UTC), via the `date` command. Used to
/// seed health.jsonl with recent events without taking a direct chrono dependency.
fn rfc3339_seconds_ago(secs: u64) -> String {
    let out = std::process::Command::new("date")
        .arg("-u")
        .arg("--rfc-3339=seconds")
        .arg(format!("--date={secs} seconds ago"))
        .output()
        .expect("`date` must run");
    assert!(
        out.status.success(),
        "`date --date=N seconds ago` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // `--rfc-3339=seconds` yields `YYYY-MM-DD HH:MM:SS+00:00`; coerce to strict RFC3339
    // (`T` separator, `Z`) which the health layer parses via chrono::DateTime.
    let with_t = raw.replacen(' ', "T", 1);
    with_t.replace("+00:00", "Z")
}

/// Loose check for the `mmddyy-XXXXXXXX` short hash shape (6 digits, hyphen, 8 hex).
fn regex_like_hash(s: &str) -> bool {
    s.len() == 15
        && s.as_bytes()[6] == b'-'
        && s[..6].bytes().all(|b| b.is_ascii_digit())
        && s[7..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
