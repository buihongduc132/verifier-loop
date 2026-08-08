// E2E wiring for health cooldown + dynamic reject-notes prompt (intention 2026-07-14).
// Exercises the CLI bin integration: the cooldown check returns the fallback hash, and a
// RESUME after a prior REJECT round appends the prior reject notes into the new prompt.
//
// Determinism strategy mirrors tests/cli_e2e.rs: a STUB backend + git work tree tempdir.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin;

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

/// A stub backend that always APPROVES.
fn approve_stub(dir: &Path) -> PathBuf {
    let verdict = bin("verifier-verdict");
    write_script(
        dir,
        "approve_stub.sh",
        &format!(
            r#"#!/bin/sh
cat <<'ACP'
{{"type":"session","id":"s1"}}
{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"ok"}}]}}],"willRetry":false}}
ACP
"{verdict}" approve
"#,
            verdict = verdict.to_string_lossy()
        ),
    )
}

/// A stub backend that always REJECTS with a fixed note.
fn reject_stub(dir: &Path) -> PathBuf {
    let verdict = bin("verifier-verdict");
    write_script(
        dir,
        "reject_stub.sh",
        &format!(
            r#"#!/bin/sh
cat <<'ACP'
{{"type":"session","id":"s1"}}
{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"nope"}}]}}],"willRetry":false}}
ACP
"{verdict}" reject --notes "round-one rejection evidence missing"
"#,
            verdict = verdict.to_string_lossy()
        ),
    )
}

/// Seed a git work tree at `dir` with a config.json.
fn seed_workdir(dir: &Path, n: u32, m: u32) {
    let git_ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "-q"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(git_ok, "git init failed");
    let cfg = serde_json::json!({
        "n": n, "m": m, "maxTurn": 3, "backend": "stub",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    fs::write(dir.join("config.json"), cfg.to_string()).unwrap();
    fs::write(dir.join(".gitkeep"), "").unwrap();
    for (k, v) in [
        ("user.email", "t@e.com"),
        ("user.name", "T"),
    ] {
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", k, v])
            .status();
    }
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
}

/// Run `verifier-loop` raw, returning its output.
fn run_vl(
    cwd: &Path,
    home: &Path,
    stub: &Path,
    args: &[&str],
) -> std::process::Output {
    std::process::Command::new(bin("verifier-loop"))
        .args(args)
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", stub)
        .current_dir(cwd)
        .output()
        .expect("verifier-loop ran")
}

/// A fallback hash is `<mmddyy>-ffffff` (13 chars: 6 digits, hyphen, 6 f's).
fn is_fallback_hash(s: &str) -> bool {
    let s = s.trim();
    s.len() == 13
        && s[6..7] == *"-"
        && s[..6].chars().all(|c: char| c.is_ascii_digit())
        && s[7..] == *"ffffff"
}

#[test]
fn cooldown_returns_fallback_hash_after_threshold_unhealthy_events() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 1, 1);
    let stub = approve_stub(home);

    // Seed health.jsonl with >3 unhealthy events (the cooldown threshold is "more than 3").
    // Each line is a JSON event with an RFC3339 timestamp at "now".
    let now = chrono::Utc::now().to_rfc3339();
    let mut log = String::new();
    for _ in 0..4 {
        log.push_str(&format!(
            "{{\"event\":\"unhealthy\",\"at\":\"{now}\"}}\n"
        ));
    }
    fs::write(home.join("health.jsonl"), log).unwrap();

    let out = run_vl(home, home, &stub, &["NEW", "ship it"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("STDOUT: {stdout}");
    eprintln!("STDERR: {}", String::from_utf8_lossy(&out.stderr));

    // The last non-empty stdout line should be the fallback hash.
    let last_line = stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .last()
        .unwrap_or("");
    assert!(
        is_fallback_hash(last_line),
        "expected cooldown fallback hash `<mmddyy>-ffffff`, got: {last_line:?}"
    );
}

#[test]
fn no_cooldown_when_under_threshold_runs_normal_round() {
    // With ZERO unhealthy events, the round must run normally (NOT return the ffffff
    // fallback). The stub approves, so we expect a real mmddyy-XXXXXXXX hash.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 1, 1);
    let stub = approve_stub(home);

    let out = run_vl(home, home, &stub, &["NEW", "implement feature x"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    eprintln!("STDOUT: {stdout}\nSTDERR: {stderr}");
    // Find the real hash anywhere in stdout (mmddyy-XXXXXXXX), or assert it did NOT
    // return the fallback. The key contract: under-threshold never yields ffffff.
    let has_fallback = stdout
        .lines()
        .any(|l| is_fallback_hash(l.trim()));
    assert!(
        !has_fallback,
        "under-threshold must NOT return fallback hash: {stdout}"
    );
    // And there must be a real 15-char hash somewhere in stdout.
    let has_real_hash = stdout.lines().any(|l| {
        let l = l.trim();
        l.len() == 15
            && l[6..7] == *"-"
            && l[..6].chars().all(|c: char| c.is_ascii_digit())
            && l[7..]
                .chars()
                .all(|c: char| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    });
    assert!(
        has_real_hash,
        "under-threshold approving stub must produce a real hash: {stdout}\n{stderr}"
    );
}

#[test]
fn resume_appends_prior_round_reject_notes_into_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 2);
    let reject = reject_stub(home);
    let approve = approve_stub(home);

    // Round 1: REJECT (records a prior reject note). Both verifiers reject.
    let out1 = run_vl(home, home, &reject, &["NEW", "ship feature"]);
    let goal_id = String::from_utf8_lossy(&out1.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("goalId: ").map(|s| s.to_string()))
        .expect("goalId line printed");

    // Round 2: the v1 round-2 initial-prompt.txt MUST contain the round-1 reject note
    // (feature b: dynamic prompt from prior reject notes), regardless of pass/reject.
    let _out2 = run_vl(
        home,
        home,
        &approve,
        &["RESUME", &goal_id, "--fix", "added the missing evidence"],
    );

    let prompt_path = home
        .join("goals")
        .join(&goal_id)
        .join("rounds")
        .join("2")
        .join("v1")
        .join("initial-prompt.txt");
    let prompt = fs::read_to_string(&prompt_path).expect("round-2 prompt written");
    assert!(
        prompt.contains("round-one rejection evidence missing"),
        "round-2 prompt must contain the prior round-1 reject note: {prompt}"
    );
    assert!(
        prompt.contains("added the missing evidence"),
        "round-2 prompt must contain the fix notes too: {prompt}"
    );
}
