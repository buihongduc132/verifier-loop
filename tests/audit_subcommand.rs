// Intention 2026-07-14 — `jewilo AUDIT <goalId>` subcommand (RED phase).
//
// `AUDIT` verifies the final completion TRULY matches the creation-time config requirement:
//   * reads creation-time config (n/m) from goal.json (the snapshot, NOT current config.json),
//   * reads completion.json matching verdicts,
//   * verifies matching APPROVE count >= n out of m,
//   * recomputes the completion hash from the stored inputs and compares to stored fullDigest,
//   * prints JSON { valid, requiredN, requiredM, matchingVerdicts, hashRecomputed, hashStored,
//     checks },
//   * exits 0 if valid, non-zero otherwise.
//
// See flow/intentions/2026-07-14_stats-and-audit-subcommands.md.
//
// RED phase: the `AUDIT` variant does NOT exist in `VerifierLoopCmd` yet, so clap rejects
// `AUDIT` as an unknown subcommand and every test here FAILS. The contract pinned here
// drives the GREEN implementation.
//
// Determinism strategy mirrors tests/cli_e2e.rs: a STUB backend script emits a fixed ACP
// stream and calls the built `verifier-verdict` to register APPROVE/REJECT. No real pi.

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
cat <<'ACP'
{{"type":"session","id":"stub-session-id"}}
{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"stub final output"}}]}}],"willRetry":false}}
ACP
case "${{VERIFIER_LOOP_STUB_VERDICT:-approve}}" in
  reject) "{verdict}" reject --notes "stub rejection: no proof produced" ;;
  *)      "{verdict}" approve ;;
esac
"#,
            verdict = verdict.to_string_lossy()
        ),
    )
}

/// Seed a git work tree at `dir` with a `config.json` (stub) and the stub script.
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

/// Run `verifier-loop` as a raw subprocess, returning full output regardless of exit.
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

/// Drive a full NEW to completion (n=m=1, stub approves) and return the goalId + the
/// path to the goal directory.
fn seed_completed_goal(home: &Path, stub: &Path, goal_text: &str) -> (String, PathBuf) {
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
    let goal_id = goal_ids.pop().unwrap();
    let gdir = home.join("goals").join(&goal_id);
    (goal_id, gdir)
}

/// Read the on-disk completion.json for a goal as a JSON Value.
fn read_completion(gdir: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(gdir.join("completion.json")).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// Test 1 — AUDIT reports valid=true when the completion matches the creation-time
// config (n=m=1, one APPROVE). Exit 0.
// ---------------------------------------------------------------------------

#[test]
fn audit_valid_when_completion_matches_config() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let (goal_id, _gdir) =
        seed_completed_goal(home, &stub, "audit: valid completion matches config");

    let out = run_vl(
        home,
        home,
        Path::new("/unused/stub/audit"),
        &["AUDIT", &goal_id],
        &[],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "AUDIT of a valid completion must exit 0. exit={:?}\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}",
        out.status.code()
    );

    let report: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("AUDIT stdout must be valid JSON: parse {e}\n--- stdout ---\n{stdout}")
    });
    eprintln!("AUDIT report:\n{report:#}");

    assert_eq!(
        report["valid"].as_bool(),
        Some(true),
        "AUDIT.valid must be true for a matching completion: {report}"
    );
    assert_eq!(
        report["requiredN"].as_u64(),
        Some(1),
        "AUDIT.requiredN must be the creation-time n=1: {report}"
    );
    assert_eq!(
        report["requiredM"].as_u64(),
        Some(1),
        "AUDIT.requiredM must be the creation-time m=1: {report}"
    );
    assert!(
        report["matchingVerdicts"]
            .as_u64()
            .or_else(|| report["matchingVerdicts"].as_array().map(|a| a.len() as u64))
            .is_some_and(|c| c >= 1),
        "AUDIT.matchingVerdicts must be >=1: {report}"
    );

    // Hash recompute: the recomputed full digest must EQUAL the stored fullDigest.
    let recomputed = report["hashRecomputed"]
        .as_str()
        .unwrap_or_else(|| panic!("AUDIT.hashRecomputed must be present: {report}"));
    let stored = report["hashStored"]
        .as_str()
        .unwrap_or_else(|| panic!("AUDIT.hashStored must be present: {report}"));
    assert_eq!(
        recomputed, stored,
        "AUDIT hashRecomputed must equal hashStored for a valid completion: {report}"
    );
    assert!(
        report["checks"]
            .as_array()
            .is_some_and(|c| !c.is_empty()),
        "AUDIT.checks must be a non-empty array: {report}"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — AUDIT detects a tampered fullDigest: report valid=false (hash mismatch),
// non-zero exit.
// ---------------------------------------------------------------------------

#[test]
fn audit_detects_hash_tamper() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let (goal_id, gdir) =
        seed_completed_goal(home, &stub, "audit: detect fullDigest tamper");

    // Tamper: flip the stored fullDigest so the recomputed hash no longer matches.
    let mut completion = read_completion(&gdir);
    let original_digest = completion["fullDigest"]
        .as_str()
        .expect("completion.json has a fullDigest")
        .to_string();
    // Build a plausible-but-wrong 64-hex digest by flipping the first char.
    let tampered_digest = {
        let first = original_digest.as_bytes()[0];
        let flipped = if first == b'0' { '1' } else { '0' };
        format!("{flipped}{}", &original_digest[1..])
    };
    assert_ne!(
        tampered_digest, original_digest,
        "tampered digest must differ from the original"
    );
    completion["fullDigest"] = serde_json::Value::String(tampered_digest.clone());
    fs::write(
        gdir.join("completion.json"),
        serde_json::to_string_pretty(&completion).unwrap(),
    )
    .unwrap();

    let out = run_vl(
        home,
        home,
        Path::new("/unused/stub/audit"),
        &["AUDIT", &goal_id],
        &[],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "AUDIT of a tampered completion must exit NON-zero. exit={:?}\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}",
        out.status.code()
    );

    // Even on non-zero exit, the JSON report must be emitted to stdout.
    let report: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("AUDIT stdout must be valid JSON even on tamper: parse {e}\n--- stdout ---\n{stdout}")
    });
    eprintln!("AUDIT report (tampered):\n{report:#}");

    assert_eq!(
        report["valid"].as_bool(),
        Some(false),
        "AUDIT.valid must be false when the hash was tampered: {report}"
    );
    // The stored hash is the tampered value; the recomputed one differs from it.
    assert_eq!(
        report["hashStored"].as_str(),
        Some(tampered_digest.as_str()),
        "AUDIT.hashStored must reflect the tampered value: {report}"
    );
    assert_ne!(
        report["hashRecomputed"].as_str(),
        Some(tampered_digest.as_str()),
        "AUDIT.hashRecomputed must NOT match the tampered stored value: {report}"
    );
    // The checks array must surface the hash mismatch explicitly.
    let checks_json = serde_json::to_string(&report["checks"]).unwrap_or_default();
    assert!(
        checks_json.to_lowercase().contains("hash")
            || checks_json.to_lowercase().contains("digest")
            || checks_json.to_lowercase().contains("mismatch"),
        "AUDIT.checks must name the hash/digest mismatch: {checks_json}"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — AUDIT reports valid=false with a "no completion" reason when there is no
// completion.json (stub rejected → no consensus). Non-zero exit.
// ---------------------------------------------------------------------------

#[test]
fn audit_reports_no_completion() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    // NEW with a REJECTING stub: no consensus → no completion.json.
    let out = run_vl(
        home,
        home,
        &stub,
        &["NEW", "audit: no completion on reject"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );
    assert!(
        !out.status.success(),
        "seed NEW with reject must exit non-zero"
    );

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    let gdir = home.join("goals").join(&goal_id);
    assert!(
        !gdir.join("completion.json").exists(),
        "precondition: no completion.json after a rejected round"
    );

    let out = run_vl(
        home,
        home,
        Path::new("/unused/stub/audit"),
        &["AUDIT", &goal_id],
        &[],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "AUDIT with no completion must exit NON-zero. exit={:?}\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}",
        out.status.code()
    );

    let report: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("AUDIT stdout must be valid JSON even with no completion: parse {e}\n--- stdout ---\n{stdout}")
    });
    eprintln!("AUDIT report (no completion):\n{report:#}");

    assert_eq!(
        report["valid"].as_bool(),
        Some(false),
        "AUDIT.valid must be false when there is no completion: {report}"
    );

    // The reason / checks must explicitly name the missing completion.
    let report_json = serde_json::to_string(&report).unwrap_or_default().to_lowercase();
    assert!(
        report_json.contains("no completion") || report_json.contains("completion"),
        "AUDIT report must name the missing-completion reason: {report}"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — AUDIT uses the CREATION-TIME config snapshot (from goal.json), NOT the
// current config.json. After creation with n=m=1, mutate config.json to n=2 m=2; AUDIT
// must STILL use n=1 m=1 (the snapshot), so a single-APPROVE completion stays valid.
// ---------------------------------------------------------------------------

#[test]
fn audit_uses_creation_time_config_not_current() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let (goal_id, gdir) =
        seed_completed_goal(home, &stub, "audit: creation-time config is authoritative");

    // Sanity: the goal snapshot recorded n=m=1.
    let goal_json: Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("goal.json")).unwrap()).unwrap();
    assert_eq!(goal_json["config"]["n"].as_u64(), Some(1));
    assert_eq!(goal_json["config"]["m"].as_u64(), Some(1));

    // Mutate the CURRENT config.json to n=2 m=2 AFTER creation. AUDIT must ignore this.
    let new_cfg = serde_json::json!({
        "n": 2,
        "m": 2,
        "maxTurn": 3,
        "backend": "stub",
        "gitDiffMaxChars": 1000,
        "verifierTimeoutSec": 10
    });
    fs::write(home.join("config.json"), new_cfg.to_string()).unwrap();

    let out = run_vl(
        home,
        home,
        Path::new("/unused/stub/audit"),
        &["AUDIT", &goal_id],
        &[],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "AUDIT must exit 0 using the creation-time n=1 m=1 snapshot (NOT the mutated config.json). \
         exit={:?}\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}",
        out.status.code()
    );

    let report: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("AUDIT stdout must be valid JSON: parse {e}\n--- stdout ---\n{stdout}")
    });
    eprintln!("AUDIT report (creation-time config):\n{report:#}");

    assert_eq!(
        report["valid"].as_bool(),
        Some(true),
        "AUDIT.valid must be true using the creation-time n=1 m=1 snapshot: {report}"
    );
    // The reported requirement MUST be the snapshot (1/1), not the mutated (2/2).
    assert_eq!(
        report["requiredN"].as_u64(),
        Some(1),
        "AUDIT.requiredN must come from goal.json's creation-time snapshot (=1), NOT current config.json: {report}"
    );
    assert_eq!(
        report["requiredM"].as_u64(),
        Some(1),
        "AUDIT.requiredM must come from goal.json's creation-time snapshot (=1), NOT current config.json: {report}"
    );
}
