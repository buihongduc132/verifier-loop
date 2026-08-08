// add-json-output-mode — Blocker B RED tests for STATS / AUDIT under `--json`.
//
// These are the RED regression tests for the BLOCKER B defect: under `--json`,
// `jewilo STATS` / `jewilo AUDIT` MUST emit exactly ONE envelope object on stdout
// (carrying the report body), NOT a bare/duplicate JSON object. The companion
// default-mode tests pin the legacy bare-JSON byte-identity.
//
// TDD: written FIRST, against the spec. All four are expected to FAIL on the current
// (pre-fix) code: STATS/AUDIT bypass the envelope under --json, and AUDIT-invalid prints
// TWO objects (the bare report + the error envelope). The GREEN fix routes both through
// `emit_report` so exactly one root object is emitted.
//
// Source of truth: openspec/changes/add-json-output-mode/specs/json-output/spec.md
//
// Determinism strategy mirrors tests/json_output_jewilo.rs (stub backend + tempdir home).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn bin(name: &str) -> PathBuf {
    assert_cmd::cargo::cargo_bin(name)
}

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

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

fn stub_script(dir: &Path) -> PathBuf {
    let verdict = bin("verifier-verdict");
    let v = verdict.to_string_lossy();
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
  reject) "{v}" reject --notes "stub rejection: no proof produced" ;;
  *)      "{v}" approve ;;
esac
"#,
        ),
    )
}

fn seed_workdir(dir: &Path, n: u32, m: u32) {
    let git_ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "-q"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(git_ok, "git init failed in tempdir");
    let cfg = serde_json::json!({
        "n": n, "m": m, "maxTurn": 3, "backend": "stub",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    fs::write(dir.join("config.json"), cfg.to_string()).unwrap();
    fs::write(dir.join(".gitkeep"), "").unwrap();
    for (k, val) in [("user.email", "t@e.com"), ("user.name", "T")] {
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", k, val])
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

fn count_top_level_json_roots(stdout: &str) -> usize {
    let mut depth: i64 = 0;
    let mut roots = 0usize;
    let mut in_string = false;
    let mut prev = '\0';
    for ch in stdout.chars() {
        if in_string {
            if ch == '"' && prev != '\\' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => in_string = true,
                '{' => {
                    if depth == 0 {
                        roots += 1;
                    }
                    depth += 1;
                }
                '}' => {
                    if depth > 0 {
                        depth -= 1;
                    }
                }
                _ => {}
            }
        }
        prev = ch;
    }
    roots
}

fn parse_json_envelope(stdout: &str) -> Value {
    let roots = count_top_level_json_roots(stdout);
    assert_eq!(
        roots, 1,
        "expected exactly ONE JSON root object on stdout, found {roots}.\nstdout:\n{stdout}"
    );
    let start = stdout.find('{').expect("at least one '{' present");
    let mut depth: i64 = 0;
    let mut in_string = false;
    let mut prev = '\0';
    let mut end = start;
    for (i, ch) in stdout[start..].char_indices() {
        if in_string {
            if ch == '"' && prev != '\\' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        prev = ch;
    }
    let slice = &stdout[start..end];
    serde_json::from_str(slice).unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\n---\n{slice}"))
}

fn seed_completed_goal(home: &Path, stub: &Path, goal_text: &str) -> String {
    let out = run_vl_raw(home, home, stub, &["NEW", goal_text], &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "seed NEW must reach consensus; stderr:\n{stderr}"
    );
    let mut goal_ids: Vec<String> = fs::read_dir(home.join("goals"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(goal_ids.len(), 1, "exactly one goal created in seed NEW");
    goal_ids.pop().unwrap()
}

// ---------------------------------------------------------------------------
// Test 2 — `jewilo --json STATS <id>` emits exactly ONE envelope with report.
// ---------------------------------------------------------------------------

#[test]
fn jewilo_stats_json_single_envelope_with_report() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 1, 1);
    let stub = stub_script(home);
    let goal_id = seed_completed_goal(home, &stub, "stats --json single envelope goal");

    // STATS is read-only; the stub path is unused but supplied for harness uniformity.
    let out = run_vl_raw(home, home, &stub, &["--json", "STATS", &goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "STATS --json must exit 0; stderr:\n{stderr}"
    );

    // Exactly ONE root object (BLOCKER B: no duplicate / bare object).
    assert_eq!(
        count_top_level_json_roots(&stdout),
        1,
        "STATS --json must emit exactly ONE JSON object; got: {stdout}"
    );
    let env = parse_json_envelope(&stdout);
    assert_eq!(env["ok"], true, "envelope.ok must be true: {env}");
    assert_eq!(
        env["command"].as_str(),
        Some("stats"),
        "envelope.command must be \"stats\": {env}"
    );
    assert_eq!(
        env["goalId"].as_str(),
        Some(goal_id.as_str()),
        "envelope.goalId must be set: {env}"
    );
    // The stats body rides inside the `report` field.
    assert!(
        env.get("report").is_some(),
        "envelope must carry a `report` field with the stats body: {env}"
    );
    // Sanity: the report carries the goal record.
    assert_eq!(
        env["report"]["goal"]["goalId"].as_str(),
        Some(goal_id.as_str()),
        "report.goal.goalId must match: {env}"
    );
    // Success envelopes carry NO status field (STATS/AUDIT use `report` instead).
    assert!(
        env.get("status").is_none(),
        "STATS success envelope must NOT carry `status` (it uses `report`): {env}"
    );
    assert!(
        env.get("error").is_none(),
        "STATS success envelope must NOT carry `error`: {env}"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — `jewilo STATS <id>` (no --json) is byte-identical: bare pretty JSON, no envelope.
// ---------------------------------------------------------------------------

#[test]
fn jewilo_stats_default_bare_json_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 1, 1);
    let stub = stub_script(home);
    let goal_id = seed_completed_goal(home, &stub, "stats default bare json goal");

    let out = run_vl_raw(home, home, &stub, &["STATS", &goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "STATS must exit 0; stderr:\n{stderr}");

    // The entire stdout must be a SINGLE bare JSON object — no envelope wrapper.
    assert_eq!(
        count_top_level_json_roots(&stdout.trim_end()),
        1,
        "default STATS must emit exactly ONE bare JSON object: {stdout}"
    );
    let body: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("default STATS stdout must be a single bare JSON object: {e}\n{stdout}")
    });
    // No envelope wrapper fields.
    assert!(
        body.get("ok").is_none(),
        "default STATS must NOT have an `ok` envelope field: {body}"
    );
    assert!(
        body.get("command").is_none(),
        "default STATS must NOT have a `command` envelope field: {body}"
    );
    assert!(
        body.get("report").is_none(),
        "default STATS must NOT wrap the body in a `report` field: {body}"
    );
    // The bare body carries the goal record.
    assert_eq!(
        body["goal"]["goalId"].as_str(),
        Some(goal_id.as_str()),
        "default STATS body carries the goal: {body}"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — `jewilo --json AUDIT <id-without-completion>` emits exactly ONE envelope
// with ok:false, command:"audit", report, AND error. (Catches the duplicate-object bug.)
// ---------------------------------------------------------------------------

#[test]
fn jewilo_audit_json_invalid_single_envelope_with_report_and_error() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 1, 1);
    let stub = stub_script(home);

    // NEW with REJECT: no consensus → no completion.json.
    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "audit --json invalid no completion goal"],
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

    let out = run_vl_raw(home, home, &stub, &["--json", "AUDIT", &goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !out.status.success(),
        "AUDIT on an invalid completion must exit NON-zero; stderr:\n{stderr}"
    );

    // The BLOCKER B bug: this used to print TWO objects (the bare report + the error
    // envelope). Assert exactly ONE.
    assert_eq!(
        count_top_level_json_roots(&stdout),
        1,
        "AUDIT --json must emit exactly ONE JSON object even on invalid; got: {stdout}"
    );
    let env = parse_json_envelope(&stdout);
    assert_eq!(env["ok"], false, "invalid-audit envelope.ok must be false: {env}");
    assert_eq!(
        env["command"].as_str(),
        Some("audit"),
        "envelope.command must be \"audit\": {env}"
    );
    assert_eq!(
        env["goalId"].as_str(),
        Some(goal_id.as_str()),
        "envelope.goalId must be set: {env}"
    );
    // The audit report must ride inside the envelope (NOT as a separate object).
    assert!(
        env.get("report").is_some(),
        "envelope must carry the audit `report` body: {env}"
    );
    assert_eq!(
        env["report"]["valid"].as_bool(),
        Some(false),
        "report.valid must be false: {env}"
    );
    // And the error string must be present.
    assert!(
        env.get("error").is_some(),
        "envelope must carry an `error` string on the invalid path: {env}"
    );
    let err = env["error"].as_str().unwrap();
    assert!(
        err.contains("audit"),
        "error must name the audit failure: {err}"
    );
}

// ---------------------------------------------------------------------------
// Test 5 — `jewilo AUDIT <id-without-completion>` (no --json): bare report on stdout +
// error on stderr; non-zero exit. Legacy byte-identity.
// ---------------------------------------------------------------------------

#[test]
fn jewilo_audit_default_bare_report_plus_stderr_error() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 1, 1);
    let stub = stub_script(home);

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "audit default invalid no completion goal"],
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

    let out = run_vl_raw(home, home, &stub, &["AUDIT", &goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !out.status.success(),
        "AUDIT on an invalid completion must exit NON-zero; stderr:\n{stderr}"
    );

    // Exactly ONE bare JSON object on stdout (the audit report); no envelope wrapper.
    assert_eq!(
        count_top_level_json_roots(&stdout.trim_end()),
        1,
        "default AUDIT stdout must be a single bare JSON report object: {stdout}"
    );
    let body: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("default AUDIT stdout must be a single bare JSON object: {e}\n{stdout}")
    });
    assert!(
        body.get("ok").is_none(),
        "default AUDIT must NOT have an `ok` envelope field: {body}"
    );
    assert!(
        body.get("command").is_none(),
        "default AUDIT must NOT have a `command` envelope field: {body}"
    );
    assert_eq!(
        body["valid"].as_bool(),
        Some(false),
        "the bare report must mark the audit invalid: {body}"
    );
    // The human-readable error rides on stderr only (legacy behavior).
    assert!(
        stderr.to_lowercase().contains("audit"),
        "default AUDIT stderr must carry the human-readable audit error: {stderr}"
    );
}
