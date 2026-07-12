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

/// Read the completion hash from stdout (last `mmddyy-XXXXXXXX` line).
fn hash_from_stdout(stdout: &str) -> Option<String> {
    stdout.lines().rev().find_map(|l| {
        let l = l.trim();
        // mmddyy-XXXXXXXX: 6 digits, hyphen, 8 hex.
        if l.len() == 15
            && l[6..7] == *"-"
            && l[..6].chars().all(|c: char| c.is_ascii_digit())
            && l[7..]
                .chars()
                .all(|c: char| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            Some(l.to_string())
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Scenario 1 — NEW happy path (n=m=1, stub approves) produces a mmddyy-XXXXXXXX hash + goal dir.
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

    let hash = hash_from_stdout(&stdout).expect("a mmddyy-XXXXXXXX hash was printed on pass");
    assert!(
        predicate::str::is_match("^[0-9]{6}-[0-9a-f]{8}$")
            .unwrap()
            .eval(&hash),
        "hash must be mmddyy-XXXXXXXX: {hash}"
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
    assert!(
        gdir.join("signature.json").exists(),
        "signature.json written"
    );

    let v1 = gdir.join("rounds").join("1").join("v1");
    // verdict.json status APPROVE (written by the stub via verifier-verdict).
    let verdict: Value =
        serde_json::from_str(&fs::read_to_string(v1.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["status"], "APPROVE",
        "v1 verdict APPROVE: {verdict}"
    );
    assert!(verdict["registeredAt"].is_string(), "registeredAt present");

    // meta.json + final-output.txt + initial-prompt.txt populated by spawn/prompt layers.
    assert!(v1.join("meta.json").exists(), "meta.json written");
    let final_out = fs::read_to_string(v1.join("final-output.txt")).unwrap();
    assert!(
        final_out.contains("stub final output"),
        "final-output captured: {final_out}"
    );
    assert!(
        v1.join("initial-prompt.txt").exists(),
        "initial-prompt persisted"
    );

    // completion.json mirrors the printed hash.
    let completion: Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("completion.json")).unwrap()).unwrap();
    assert_eq!(
        completion["hash"], hash,
        "completion.json hash matches stdout"
    );
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
    assert!(
        hash_from_stdout(&stdout).is_none(),
        "no hash on failure: {stdout}"
    );

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
        !home
            .join("goals")
            .join(&goal_id)
            .join("completion.json")
            .exists(),
        "no completion.json on failure"
    );

    let verdict: Value = serde_json::from_str(
        &fs::read_to_string(
            home.join("goals")
                .join(&goal_id)
                .join("rounds")
                .join("1")
                .join("v1")
                .join("verdict.json"),
        )
        .unwrap(),
    )
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
    assert!(predicate::str::is_match("^[0-9]{6}-[0-9a-f]{8}$")
        .unwrap()
        .eval(&hash));

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
    let goal: Value =
        serde_json::from_str(&fs::read_to_string(gdir.join("goal.json")).unwrap()).unwrap();
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

    let receipt_head = verifier_loop::receipt::read_receipt_head(home, &goal_id);
    let recomputed = verifier_loop::consensus::compute_hash(
        salt.trim(),
        &goal_id,
        goal_sig,
        round,
        &mvs,
        matched_at,
        &receipt_head,
    );
    assert_eq!(
        recomputed.short_hash(),
        stored_hash,
        "audit recomputes the stored short hash"
    );
    assert_eq!(
        recomputed.full_digest(),
        completion["fullDigest"].as_str().unwrap(),
        "audit recomputes the stored fullDigest"
    );

    // Tamper 1: edit goalText → signature recomputation differs → hash differs.
    let mut tampered_goal = goal.clone();
    tampered_goal["goalText"] = serde_json::Value::String("tampered!".into());
    let tampered_goal_text = tampered_goal["goalText"].as_str().unwrap();
    let created_at = goal["createdAt"].as_str().unwrap();
    let tampered_sig =
        verifier_loop::goal::compute_signature(salt.trim(), tampered_goal_text, created_at);
    let tampered_hash = verifier_loop::consensus::compute_hash(
        salt.trim(),
        &goal_id,
        &tampered_sig,
        round,
        &mvs,
        matched_at,
        &receipt_head,
    );
    assert_ne!(
        tampered_hash.short_hash(),
        stored_hash,
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
        &receipt_head,
    );
    assert_ne!(
        tampered_v_hash.short_hash(),
        stored_hash,
        "tampered verdict must break the hash"
    );
    assert_ne!(
        tampered_v_hash.full_digest(),
        recomputed.full_digest(),
        "tampered verdict must break the full digest"
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

    let out = run_vl_raw(
        dir.path(),
        &home_file,
        &stub,
        &["NEW", "goal that cannot be created"],
        &[],
    );
    assert!(
        !out.status.success(),
        "must fail closed when home is a file"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        hash_from_stdout(&stdout).is_none(),
        "no hash when store unusable"
    );
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

// ---------------------------------------------------------------------------
// RED phase (task #10) — verifierPromptFile + minGoalChars config features.
// Expected to FAIL until the GREEN implementation wires the two new keys.
//   * (a) verifierPromptFile set -> initial-prompt.txt = <file> + "\n---\n" + baked-in
//         (raw static text, NO {{var}} expansion).
//   * (b) the same prepend happens on RESUME.
//   * (c) empty/whitespace goalText -> non-zero exit, no goal dir.
//   * (d) minGoalChars > trimmed goalText length -> non-zero exit, no goal dir.
//   * (e) verifierPromptFile pointing at a MISSING file -> non-zero exit, no goal dir.
//   * (f) verifierPromptFile absent -> today's baked-in-only behavior (no change).
// ---------------------------------------------------------------------------

/// Seed a workdir + custom config.json (including the new camelCase keys), returning the
/// stub script path. `prompt_file_body` writes the file at `custom-prompt.md` inside `home`
/// when Some; used for (a)/(b) where the file exists and is raw static text.
fn seed_workdir_with_config(
    dir: &Path,
    n: u32,
    m: u32,
    extra_config: serde_json::Value,
    prompt_file_body: Option<&str>,
) -> PathBuf {
    let git_ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "-q"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(git_ok, "git init failed in tempdir");

    if let Some(body) = prompt_file_body {
        fs::write(dir.join("custom-prompt.md"), body).unwrap();
    }

    let mut cfg = serde_json::json!({
        "n": n,
        "m": m,
        "maxTurn": 3,
        "backend": "stub",
        "gitDiffMaxChars": 1000,
        "verifierTimeoutSec": 10,
    });
    if let serde_json::Value::Object(map) = &mut cfg {
        if let serde_json::Value::Object(extra) = extra_config {
            for (k, v) in extra {
                map.insert(k.clone(), v.clone());
            }
        }
    }
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

/// Returns true if at least one goal dir was created under <home>/goals.
fn any_goal_dir(home: &Path) -> bool {
    home.join("goals")
        .read_dir()
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

#[test]
fn new_with_verifier_prompt_file_prepends_custom_text_to_initial_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // Raw static text with literal {{goalText}} braces — must NOT be expanded.
    let custom = "CUSTOM VERIFIER PREAMBLE v1\nRemember: {{goalText}} must stay literal.\n";
    let stub = seed_workdir_with_config(
        home,
        1,
        1,
        serde_json::json!({ "verifierPromptFile": "custom-prompt.md" }),
        Some(custom),
    );

    let mut cmd = vl_bin();
    let out = cmd
        .arg("NEW")
        .arg("implement the verifier-loop CLI")
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", &stub)
        .current_dir(home)
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "NEW exited {}: {stderr}", out.status);

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    let prompt = fs::read_to_string(
        home.join("goals")
            .join(&goal_id)
            .join("rounds")
            .join("1")
            .join("v1")
            .join("initial-prompt.txt"),
    )
    .unwrap();

    // (1) Custom file contents come FIRST.
    assert!(
        prompt.starts_with(custom),
        "initial-prompt must start with the custom verifierPromptFile contents; got:\n{prompt}"
    );
    // (2) Then a `---` separator, then the baked-in default template.
    let sep = format!("{custom}---\n");
    assert!(
        prompt.starts_with(&sep),
        "separator `\n---\n` between custom file and baked-in default missing; got:\n{prompt}"
    );
    // (3) Raw static text — no {{var}} expansion inside the custom portion.
    assert!(
        prompt.contains("{{goalText}}"),
        "custom file must be RAW STATIC text (no {{var}} expansion): {prompt}"
    );
    // (4) Design D2 (override semantics): when a custom verifierPromptFile is set, the
    // built-in VERIFIER_POLICY block MUST be ABSENT — the custom file REPLACES it, not
    // supplements it. Asserting absence here pins the spec's mutual-exclusivity rule.
    assert!(
        !prompt.contains("Verifier Detective Policy (canonical, from verifier-loop skill)"),
        "built-in policy heading MUST be absent when a custom verifierPromptFile is set (D2 override semantics): {prompt}"
    );
    assert!(
        !prompt.contains("<_unfold.md>"),
        "built-in policy marker `<_unfold.md>` MUST be absent when a custom verifierPromptFile is set (D2): {prompt}"
    );
}

#[test]
fn resume_with_verifier_prompt_file_prepends_custom_text_to_initial_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let custom = "RESUME CUSTOM PREAMBLE\n";
    let stub = seed_workdir_with_config(
        home,
        1,
        1,
        serde_json::json!({ "verifierPromptFile": "custom-prompt.md" }),
        Some(custom),
    );

    // Round 1: reject, so we can RESUME.
    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "goal needing a resume round"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );
    assert!(!out.status.success(), "round 1 must reject");

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();

    // Round 2: RESUME (approve).
    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["RESUME", &goal_id, "--fix", "added missing tests"],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "RESUME must pass: {stderr}");

    let prompt = fs::read_to_string(
        home.join("goals")
            .join(&goal_id)
            .join("rounds")
            .join("2")
            .join("v1")
            .join("initial-prompt.txt"),
    )
    .unwrap();

    assert!(
        prompt.starts_with(custom),
        "RESUME initial-prompt must start with custom verifierPromptFile contents: {prompt}"
    );
    assert!(
        prompt.starts_with(&format!("{custom}---\n")),
        "separator `\n---\n` between custom file and baked-in resume default missing: {prompt}"
    );
    // Design D2 (override semantics): the built-in policy MUST be absent on RESUME too
    // when a custom verifierPromptFile is set.
    assert!(
        !prompt.contains("Verifier Detective Policy (canonical, from verifier-loop skill)"),
        "built-in policy heading MUST be absent on RESUME when a custom verifierPromptFile is set (D2): {prompt}"
    );
    assert!(
        !prompt.contains("<_unfold.md>"),
        "built-in policy marker `<_unfold.md>` MUST be absent on RESUME when a custom verifierPromptFile is set (D2): {prompt}"
    );
}

#[test]
fn new_with_empty_or_whitespace_goal_text_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir_with_config(
        home,
        1,
        1,
        serde_json::json!({}), // minGoalChars absent -> 0, but empty/whitespace is ALWAYS an error
        None,
    );

    let out = run_vl_raw(home, home, &stub, &["NEW", "   \t  "], &[]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !out.status.success(),
        "whitespace-only goalText must exit non-zero: {stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("goal") || stderr.to_lowercase().contains("empty"),
        "stderr should name the empty-goal failure clearly: {stderr}"
    );
    assert!(
        !any_goal_dir(home),
        "no goal dir / signature may be written on empty goalText"
    );
}

#[test]
fn new_with_goal_below_min_goal_chars_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub =
        seed_workdir_with_config(home, 1, 1, serde_json::json!({ "minGoalChars": 50 }), None);

    // 10-char goalText, well under minGoalChars=50.
    let out = run_vl_raw(home, home, &stub, &["NEW", "0123456789"], &[]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !out.status.success(),
        "goalText shorter than minGoalChars must exit non-zero: {stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("min")
            || stderr.to_lowercase().contains("short")
            || stderr.to_lowercase().contains("goal"),
        "stderr should explain the min-goal-chars failure: {stderr}"
    );
    assert!(
        !any_goal_dir(home),
        "no goal dir / signature may be written when goalText is below minGoalChars"
    );
}

#[test]
fn new_with_missing_verifier_prompt_file_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // Point verifierPromptFile at a path that does NOT exist (and pass NO prompt body so the
    // file is never written). The run must fail closed with a clear error and write nothing.
    let stub = seed_workdir_with_config(
        home,
        1,
        1,
        serde_json::json!({ "verifierPromptFile": "does-not-exist-prompt.md" }),
        None,
    );

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "implement the verifier-loop CLI"],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !out.status.success(),
        "missing verifierPromptFile must exit non-zero: {stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("prompt") || stderr.to_lowercase().contains("file"),
        "stderr should name the missing-prompt-file failure: {stderr}"
    );
    assert!(
        !any_goal_dir(home),
        "no goal dir / signature may be written when verifierPromptFile is missing"
    );
}

#[test]
fn new_without_verifier_prompt_file_keeps_baked_in_default_only() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // No verifierPromptFile key at all -> today's behavior.
    let stub = seed_workdir_with_config(home, 1, 1, serde_json::json!({}), None);

    let mut cmd = vl_bin();
    let out = cmd
        .arg("NEW")
        .arg("implement the verifier-loop CLI")
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", &stub)
        .current_dir(home)
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "NEW exited {}: {stderr}", out.status);

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    let prompt = fs::read_to_string(
        home.join("goals")
            .join(&goal_id)
            .join("rounds")
            .join("1")
            .join("v1")
            .join("initial-prompt.txt"),
    )
    .unwrap();

    // No custom preamble present: the initial prompt must equal exactly the baked-in default
    // render (no leading custom block, no leading `---`).
    assert!(
        !prompt.starts_with("CUSTOM VERIFIER PREAMBLE"),
        "no custom preamble expected when verifierPromptFile is absent: {prompt}"
    );
    // The baked-in default template starts with the identity line referencing verifierId/goalId.
    assert!(
        prompt.starts_with("You are verifier"),
        "absent verifierPromptFile must keep the baked-in-only render: {prompt}"
    );
}

// ---------------------------------------------------------------------------
// RED phase (cwd-runtime-source) — config.json dead keys must fail-closed at
// parse time, and the frozen snapshot's cwd must reflect the RUNTIME cwd
// (std::env::current_dir), NEVER any config.json value.
//
// 6a: jewilo NEW with a `cwd` key in config.json MUST exit non-zero (the key
//     is rejected; cwd is runtime-derived). Currently FAILS because Config
//     lacks #[serde(deny_unknown_fields)] so the cwd key is silently ignored
//     and the run proceeds to exit 0.
// 6b: snapshot.cwd == the runtime dir from which jewilo was invoked. Already
//     true today (cwd is sourced from std::env::current_dir), so this is a
//     regression guard that must STAY green.
// ---------------------------------------------------------------------------

#[test]
fn jewilo_new_fails_closed_when_config_has_cwd_key() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // Seed a valid worktree BUT inject the dead `cwd` key (pointing at a wrong path).
    let stub = seed_workdir_with_config(
        home,
        1,
        1,
        serde_json::json!({ "cwd": "/nonexistent/wrong/config/path" }),
        None,
    );
    // Append the cwd key by rewriting config.json (seed_workdir_with_config merges extras).
    // (The helper already inserted it via extra_config above.)

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "regression: config.json must not carry a cwd key"],
        &[],
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "jewilo NEW MUST fail-closed when config.json contains a `cwd` key (cwd is runtime-derived). \
         Got exit {:?}. stderr:\n{stderr}",
        out.status.code()
    );
    assert!(
        stderr.to_lowercase().contains("cwd"),
        "error message must explain cwd is not a valid config key / is runtime-derived: {stderr}"
    );
    // No goal dir / signature written (fail-closed before any side effect).
    assert!(
        !any_goal_dir(home),
        "no goal dir must be created when config.json has a dead `cwd` key"
    );
}

#[test]
fn snapshot_cwd_is_runtime_dir_regardless_of_anything_else() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir_with_config(home, 1, 1, serde_json::json!({}), None);

    // Invoke jewilo from a SPECIFIC runtime dir (the worktree root = home here).
    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "regression: snapshot cwd must be runtime"],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "NEW exited {}: {stderr}", out.status);

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    let prompt = fs::read_to_string(
        home.join("goals")
            .join(&goal_id)
            .join("rounds")
            .join("1")
            .join("v1")
            .join("initial-prompt.txt"),
    )
    .unwrap();

    // The frozen snapshot embeds the runtime cwd. assert_cmd's current_dir(home) sets it.
    assert!(
        prompt.contains(home.to_str().unwrap()),
        "snapshot cwd must equal the RUNTIME dir ({:?}), not any config value. prompt:\n{prompt}",
        home
    );
}
