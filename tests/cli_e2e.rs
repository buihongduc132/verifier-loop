// tasks.md §10 — End-to-end NEW / RESUME wiring (consensus-check + completion-proof +
// goal-lifecycle + verifier-spawn + verdict-registration + verifier-prompt specs).
// RED phase: written first, against the spec, BEFORE the `verifier-loop` bin wiring exists.
// The scaffold bin prints an identity line and exits 0 without spawning anything, so every
// assertion here is expected to FAIL until §10 GREEN lands.
//
// Determinism strategy (NO real `pi`): a STUB backend. `config.json` sets `backend: "stub"`
// (an unknown backend key) and the test exports `VERIFIER_LOOP_BACKEND_CMD=<abs script path>`.
// The CLI resolves the stub to a custom adapter whose command is just the script path (no
// `{prompt}` placeholder → the orchestrator's whitespace split yields a single argv element,
// identical to the proven §5 spawn_orchestrator test pattern).
//
// The stub script:
//   1. Emits a fixed ACP JSON stream (`session` + `agent_end`) so the orchestrator captures
//      a SID and a final output, and writes `final-output.txt`.
//   2. Runs `verifier-verdict` (the cargo-built jewije) to register the verdict, inheriting
//      `VERIFIER_LOOP_HOME` + the identity env vars injected by the spawn layer. Whether it
//      approves or rejects is driven by `VERIFIER_LOOP_STUB_VERDICT` (default approve).
//
// Every test creates a tempdir, `git init`s it (the frozen snapshot requires a work tree),
// points `VERIFIER_LOOP_HOME` at it, writes a `config.json`, writes the stub script, and
// drives the built `verifier-loop` binary via assert_cmd. No real network, no real pi.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

/// Absolute path to a cargo-built binary.
fn bin(name: &str) -> PathBuf {
    assert_cmd::cargo::cargo_bin(name)
}

/// `verifier-loop` binary built by cargo (target/debug) — for assert-style (success) tests.
fn vl_bin() -> Command {
    Command::cargo_bin("verifier-loop").unwrap()
}

/// Absolute path to the built `verifier-verdict` binary (baked into the stub script).
fn verdict_bin_path() -> PathBuf {
    bin("verifier-verdict")
}

/// Run `verifier-loop` as a raw subprocess and return its full output regardless of exit
/// status. Used by failure-path tests (assert_cmd's `.unwrap()` asserts success, which we
/// explicitly want to AVOID when the round is expected to reject).
fn run_vl_raw(
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
    let verdict = verdict_bin_path();
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
# Register the verdict. Identity + home come from the env injected/inheritied by the spawn layer.
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
    // The frozen snapshot (§9) requires a git work tree; capture_snapshot fails closed otherwise.
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

    // A git identity is required for `git status`/`diff` not to warn, and for a clean
    // work tree snapshot. Commit a placeholder so the repo has a valid HEAD.
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

/// Parse a `goal.json` from a goal directory.
fn read_goal_json(home: &Path, goal_id: &str) -> Value {
    let p = home.join("goals").join(goal_id).join("goal.json");
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

/// Read the completion hash from stdout (last `vl:` line).
fn hash_from_stdout(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("vl:").map(|h| format!("vl:{h}")))
}

// ---------------------------------------------------------------------------
// Scenario 1 — NEW happy path (n=m=1, stub approves) produces a vl: hash + goal dir.
// ---------------------------------------------------------------------------

#[test]
fn new_with_approving_stub_produces_hash_and_goal_dir() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let mut cmd = vl_bin();
    let out = cmd
        .arg("NEW")
        .arg("implement the verifier-loop CLI")
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", &stub)
        .current_dir(home) // cwd = git work tree for the frozen snapshot
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "NEW exited {}: {stderr}", out.status);

    let hash = hash_from_stdout(&stdout).expect("a vl: hash was printed on pass");
    assert!(
        predicate::str::is_match("^vl:[0-9a-f]{40}$")
            .unwrap()
            .eval(&hash),
        "hash must be vl:<40 hex>: {hash}"
    );

    // Locate the goal dir by scanning goals/ (goalId is a random UUID).
    let goals_dir = home.join("goals");
    let goal_ids: Vec<String> = fs::read_dir(&goals_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(goal_ids.len(), 1, "exactly one goal created");
    let goal_id = &goal_ids[0];
    let gdir = goals_dir.join(goal_id);

    // goal.json + signature.json present.
    assert!(gdir.join("goal.json").exists(), "goal.json written");
    assert!(gdir.join("signature.json").exists(), "signature.json written");

    let v1 = gdir.join("rounds").join("1").join("v1");
    // verdict.json status APPROVE (written by the stub via verifier-verdict).
    let verdict: Value =
        serde_json::from_str(&fs::read_to_string(v1.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "APPROVE", "v1 verdict APPROVE: {verdict}");
    assert!(verdict["registeredAt"].is_string(), "registeredAt present");

    // meta.json + final-output.txt + initial-prompt.txt populated by spawn/prompt layers.
    assert!(v1.join("meta.json").exists(), "meta.json written");
    let final_out = fs::read_to_string(v1.join("final-output.txt")).unwrap();
    assert!(final_out.contains("stub final output"), "final-output captured: {final_out}");
    assert!(v1.join("initial-prompt.txt").exists(), "initial-prompt persisted");

    // completion.json mirrors the printed hash.
    let completion: Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("completion.json")).unwrap()).unwrap();
    assert_eq!(completion["hash"], hash, "completion.json hash matches stdout");
    assert_eq!(completion["goalId"].as_str().unwrap(), goal_id);
    assert_eq!(completion["roundNumber"], 1);
    assert!(completion["matchedAt"].is_string());
}

// ---------------------------------------------------------------------------
// Scenario 2 — NEW fail path (n=m=1, stub rejects): no hash, no completion.json.
// ---------------------------------------------------------------------------

#[test]
fn new_with_rejecting_stub_exits_non_zero_and_no_hash() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "a goal that will be rejected"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "rejected round must exit non-zero: {stderr}"
    );
    assert!(hash_from_stdout(&stdout).is_none(), "no hash on failure: {stdout}");

    let goals_dir = home.join("goals");
    let goal_id = fs::read_dir(&goals_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    assert!(
        !home.join("goals").join(&goal_id).join("completion.json").exists(),
        "no completion.json on failure"
    );

    let verdict: Value = serde_json::from_str(&fs::read_to_string(
        home.join("goals")
            .join(&goal_id)
            .join("rounds")
            .join("1")
            .join("v1")
            .join("verdict.json"),
    )
    .unwrap())
    .unwrap();
    assert_eq!(verdict["status"], "REJECT");
    assert!(verdict["notes"].as_str().unwrap().contains("no proof"));
}

// ---------------------------------------------------------------------------
// Scenario 3 — RESUME reject→pass: round 1 REJECT, RESUME → round 2 APPROVE → hash.
// ---------------------------------------------------------------------------

#[test]
fn resume_after_reject_produces_hash_on_second_round() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    // Round 1: reject.
    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "goal needing a fix round"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );
    assert!(!out.status.success(), "round 1 rejects");

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();

    // Snapshot goal.json + signature.json bytes BEFORE resume (immutability proof).
    let gdir = home.join("goals").join(&goal_id);
    let goal_before = fs::read(gdir.join("goal.json")).unwrap();
    let sig_before = fs::read(gdir.join("signature.json")).unwrap();

    // Round 2: approve via RESUME.
    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["RESUME", &goal_id, "--fix", "added missing tests"],
        &[],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "round 2 must pass: {stderr}\nstdout={stdout}"
    );

    let hash = hash_from_stdout(&stdout).expect("hash on round 2");
    assert!(predicate::str::is_match("^vl:[0-9a-f]{40}$").unwrap().eval(&hash));

    // Round 2 directory exists with an APPROVE verdict.
    let v2 = gdir.join("rounds").join("2").join("v1");
    let verdict: Value =
        serde_json::from_str(&fs::read_to_string(v2.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "APPROVE");

    // completion.json on round 2.
    let completion: Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("completion.json")).unwrap()).unwrap();
    assert_eq!(completion["roundNumber"], 2);
    assert_eq!(completion["hash"].as_str().unwrap(), hash);

    // goal.json + signature.json byte-for-byte unchanged across the RESUME.
    assert_eq!(
        fs::read(gdir.join("goal.json")).unwrap(),
        goal_before,
        "goal.json immutable across RESUME"
    );
    assert_eq!(
        fs::read(gdir.join("signature.json")).unwrap(),
        sig_before,
        "signature.json immutable across RESUME"
    );
}

// ---------------------------------------------------------------------------
// Scenario 4 — audit recomputation + tamper invalidation (fail-closed).
// ---------------------------------------------------------------------------

#[test]
fn hash_recomputes_and_tamper_breaks_it() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let mut cmd = vl_bin();
    let out = cmd
        .arg("NEW")
        .arg("tamper-evidence goal")
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", &stub)
        .current_dir(home)
        .unwrap();
    assert!(out.status.success());

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    let gdir = home.join("goals").join(&goal_id);

    let completion: Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("completion.json")).unwrap()).unwrap();
    let stored_hash = completion["hash"].as_str().unwrap().to_string();

    // Recompute via the public consensus API and compare (audit reproducibility).
    let salt = fs::read_to_string(home.join(".salt")).unwrap();
    let goal: Value = serde_json::from_str(&fs::read_to_string(gdir.join("goal.json")).unwrap()).unwrap();
    let sig: Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("signature.json")).unwrap()).unwrap();
    let goal_sig = sig["signature"].as_str().unwrap();
    let round = completion["roundNumber"].as_u64().unwrap() as u32;
    let matched_at = completion["matchedAt"].as_str().unwrap();

    // Reconstruct MatchingVerdicts from completion.json.
    let mvs: Vec<verifier_loop::consensus::MatchingVerdict> = completion["matchingVerdicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| verifier_loop::consensus::MatchingVerdict {
            verifier_id: v["verifierId"].as_str().unwrap().to_string(),
            registered_at: v["registeredAt"].as_str().unwrap().to_string(),
        })
        .collect();

    let recomputed = verifier_loop::consensus::compute_hash(
        salt.trim(),
        &goal_id,
        goal_sig,
        round,
        &mvs,
        matched_at,
    );
    assert_eq!(recomputed, stored_hash, "audit recomputes the stored hash");

    // Tamper 1: edit goalText → signature recomputation differs → hash differs.
    let mut tampered_goal = goal.clone();
    tampered_goal["goalText"] = serde_json::Value::String("tampered!".into());
    let tampered_goal_text = tampered_goal["goalText"].as_str().unwrap();
    let created_at = goal["createdAt"].as_str().unwrap();
    let tampered_sig = verifier_loop::goal::compute_signature(salt.trim(), tampered_goal_text, created_at);
    let tampered_hash = verifier_loop::consensus::compute_hash(
        salt.trim(),
        &goal_id,
        &tampered_sig,
        round,
        &mvs,
        matched_at,
    );
    assert_ne!(
        tampered_hash, stored_hash,
        "tampered goalText must break the hash"
    );

    // Tamper 2: edit a matching verdict's registeredAt → hash differs.
    let mut tampered_mvs = mvs.clone();
    tampered_mvs[0].registered_at = "1999-01-01T00:00:00Z".to_string();
    let tampered_v_hash = verifier_loop::consensus::compute_hash(
        salt.trim(),
        &goal_id,
        goal_sig,
        round,
        &tampered_mvs,
        matched_at,
    );
    assert_ne!(
        tampered_v_hash, stored_hash,
        "tampered verdict must break the hash"
    );
}

// ---------------------------------------------------------------------------
// Scenario 5 — missing/unusable store fails closed (no hash, non-zero exit).
// ---------------------------------------------------------------------------

#[test]
fn new_with_home_pointing_at_a_file_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let home_file = dir.path().join("not_a_dir");
    fs::write(&home_file, "x").unwrap();

    let stub = stub_script(dir.path());

    let out = run_vl_raw(dir.path(), &home_file, &stub, &["NEW", "goal that cannot be created"], &[]);
    assert!(!out.status.success(), "must fail closed when home is a file");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(hash_from_stdout(&stdout).is_none(), "no hash when store unusable");
}

// ---------------------------------------------------------------------------
// Scenario 6 — goal context option round-trips into goal.json.
// ---------------------------------------------------------------------------

#[test]
fn new_with_context_records_context_in_goal_json() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let mut cmd = vl_bin();
    let out = cmd
        .arg("NEW")
        .arg("contextual goal")
        .arg("--context")
        .arg("ticket #99")
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", &stub)
        .current_dir(home)
        .unwrap();
    assert!(out.status.success());

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    let goal = read_goal_json(home, &goal_id);
    assert_eq!(goal["context"], "ticket #99");
}
