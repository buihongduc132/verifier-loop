// T11 REAL E2E — dynamic-round-pipeline through the actual CLI.
//
// Invokes the built `verifier-loop` binary via subprocess with a dynamic config
// (dumpAdapter + smartAdapter set) and a STUB backend that calls `verifier-verdict approve`
// (jewije) to register signed verdicts. Verifies:
//   1. `jewilo NEW` with dynamic config dispatches to the pipeline executor (NOT legacy).
//   2. The pipeline runs Gate (1a) → Confirm (1b) sequentially.
//   3. Both phases' verdicts are written with role-prefixed ids (d1, s1).
//   4. completion.json carries `pipeline: "PL-D"` + `escalationDepth: 0`.
//   5. The hash covers BOTH phases (MatchingVerdict has phaseId).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn bin(name: &str) -> PathBuf {
    assert_cmd::cargo::cargo_bin(name)
}

/// Stub backend: emits ACP stream, then registers APPROVE via jewije (signed verdict).
fn write_stub(dir: &std::path::Path) -> PathBuf {
    let verdict = bin("verifier-verdict");
    let script = format!(
        r#"#!/bin/sh
cat <<'ACP'
{{"type":"session","id":"stub-session-id"}}
{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"stub output"}}]}}],"willRetry":false}}
ACP
"{verdict}" approve
"#,
        verdict = verdict.to_string_lossy()
    );
    let path = dir.join("stub_backend.sh");
    fs::write(&path, &script).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

#[test]
fn e2e_cli_dynamic_pipeline_pl_d_pass() {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let repo_path = repo.path();

    // git init + commit so the frozen snapshot works.
    Command::new("git").args(["init", "-q"]).current_dir(repo_path).output().unwrap();
    Command::new("git").args(["config", "user.email", "t@t.com"]).current_dir(repo_path).output().unwrap();
    Command::new("git").args(["config", "user.name", "t"]).current_dir(repo_path).output().unwrap();
    fs::write(repo_path.join("README.md"), "# test").unwrap();
    Command::new("git").args(["add", "-A"]).current_dir(repo_path).output().unwrap();
    Command::new("git").args(["commit", "-q", "-m", "init"]).current_dir(repo_path).output().unwrap();

    // Dynamic config: dumpAdapter + smartAdapter set → is_dynamic_config() = true.
    // n=1, m=1, confirmCount=1 → PL-D: Gate(1D, thr=1) → Confirm(1S, thr=1).
    let config = r#"{"n":1,"m":1,"maxTurn":3,"dumpAdapter":"stub","smartAdapter":"stub","confirmCount":1,"escaThreshold":2,"gitDiffMaxChars":1000,"verifierTimeoutSec":30}"#;
    fs::write(home.path().join("config.json"), config).unwrap();

    let stub = write_stub(home.path());

    let output = Command::new(bin("verifier-loop"))
        .arg("NEW")
        .arg("Test goal for dynamic pipeline e2e")
        .current_dir(repo_path)
        .env("VERIFIER_LOOP_HOME", home.path())
        .env("VERIFIER_LOOP_BACKEND_CMD", &stub)
        .env("VERIFIER_LOOP_SPAWN_CMD", &stub)
        .env("VERIFIER_LOOP_RESUME_CMD", &stub)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("STDOUT: {stdout}");
    eprintln!("STDERR: {stderr}");

    // Find the goal dir.
    let goals_dir = home.path().join("goals");
    let goal_id = fs::read_dir(&goals_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().is_dir())
        .unwrap()
        .file_name()
        .to_string_lossy()
        .to_string();

    let completion_path = goals_dir.join(&goal_id).join("completion.json");
    assert!(
        completion_path.exists(),
        "completion.json must exist after pipeline pass.\nSTDERR: {stderr}"
    );

    let completion: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&completion_path).unwrap()).unwrap();

    // T10: pipeline metadata.
    assert_eq!(
        completion["pipeline"].as_str(),
        Some("PL-D"),
        "pipeline tag must be PL-D: {completion}"
    );
    assert_eq!(
        completion["escalationDepth"].as_u64(),
        Some(0),
        "escalationDepth must be 0"
    );

    // LD25: hash covers BOTH phases with phaseId.
    let mvs = completion["matchingVerdicts"].as_array().unwrap();
    assert!(
        mvs.len() >= 2,
        "hash must cover BOTH phases, got {} verdicts",
        mvs.len()
    );
    let phase_ids: Vec<&str> = mvs.iter().map(|v| v["phaseId"].as_str().unwrap_or("")).collect();
    assert!(
        phase_ids.contains(&"1a") && phase_ids.contains(&"1b"),
        "both 1a + 1b must appear: {phase_ids:?}"
    );

    // Role-prefixed ids (d for dump, s for smart).
    let vids: Vec<&str> = mvs.iter().map(|v| v["verifierId"].as_str().unwrap_or("")).collect();
    assert!(
        vids.iter().any(|v| v.starts_with('d')),
        "Gate must have d-prefixed ids: {vids:?}"
    );
    assert!(
        vids.iter().any(|v| v.starts_with('s')),
        "Confirm must have s-prefixed ids: {vids:?}"
    );

    // Hash present + well-formed.
    let hash = completion["hash"].as_str().unwrap();
    assert!(hash.contains('-'), "hash must be mmddyy-8hex: {hash}");
}
