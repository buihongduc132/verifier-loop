// RED phase (task: resume --notes append-only goal notes + NEW --init-prompt-file).
//
// End-to-end CLI tests exercising two new `jewilo` flags:
//   1. `NEW --init-prompt-file <path>` — read the goal text from a file instead of a
//      positional arg. `goal.json` `goalText` must equal the file contents.
//   2. `RESUME <id> [--notes "..."]` — append-only goal notes. Each `--notes` value is
//      appended as its own line to `goals/<id>/goal-notes.json`; goal.json /
//      signature.json are unchanged; the verifier prompt carries the original goalText
//      followed by each appended note on its own line.
//
// These are expected to FAIL until the GREEN teammate:
//   - adds `--init-prompt-file` to the `NEW` clap variant (making the positional `goal`
//     optional when the flag is supplied, and failing closed when NEITHER is supplied),
//   - adds `--notes` (repeatable) to the `RESUME` clap variant,
//   - wires both into the goal layer + prompt render path.
//
// Determinism uses the same STUB backend pattern as tests/cli_e2e.rs (NO real `pi`).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
use serde_json::Value;

fn bin(name: &str) -> PathBuf {
    assert_cmd::cargo::cargo_bin(name)
}

fn vl_bin() -> Command {
    Command::cargo_bin("verifier-loop").unwrap()
}

fn run_vl_raw(
    cwd: &Path,
    home: &Path,
    stub: &Path,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Output {
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

fn verdict_bin_path() -> PathBuf {
    bin("verifier-verdict")
}

fn stub_script(dir: &Path) -> PathBuf {
    let verdict = verdict_bin_path();
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
        .arg("-C").arg(dir)
        .args(["config", "user.email", "test@example.com"]).status();
    let _ = std::process::Command::new("git")
        .arg("-C").arg(dir)
        .args(["config", "user.name", "Test"]).status();
    let _ = std::process::Command::new("git")
        .arg("-C").arg(dir).args(["add", "."]).status();
    let _ = std::process::Command::new("git")
        .arg("-C").arg(dir).args(["commit", "-q", "-m", "seed"]).status();

    stub_script(dir)
}

fn only_goal_id(home: &Path) -> String {
    fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned()
}

fn read_goal_json(home: &Path, goal_id: &str) -> Value {
    let p = home.join("goals").join(goal_id).join("goal.json");
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

// ===========================================================================
// NEW --init-prompt-file
// ===========================================================================

#[test]
fn new_with_init_prompt_file_uses_file_contents_as_goal_text() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let prompt_file = home.join("init-goal.txt");
    let body = "implement the verifier-loop CLI from a file";
    fs::write(&prompt_file, body).unwrap();

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &[
            "NEW",
            "--init-prompt-file",
            prompt_file.to_str().unwrap(),
        ],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "NEW with --init-prompt-file must succeed: {stderr}");

    let goal_id = only_goal_id(home);
    let goal = read_goal_json(home, &goal_id);
    assert_eq!(
        goal["goalText"], body,
        "goal.json goalText must equal the --init-prompt-file contents: {goal}"
    );
}

#[test]
fn new_with_init_prompt_file_missing_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &[
            "NEW",
            "--init-prompt-file",
            home.join("does-not-exist.txt").to_str().unwrap(),
        ],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "NEW with a non-existent --init-prompt-file must exit non-zero: {stderr}"
    );
    assert!(
        !home.join("goals").read_dir().map(|mut it| it.next().is_some()).unwrap_or(false),
        "no goal dir may be created when the init-prompt-file is missing"
    );
}

#[test]
fn new_without_goal_or_init_prompt_file_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    // NEITHER a positional goal NOR --init-prompt-file is supplied. clap must reject this
    // (non-zero exit) before any goal dir is written.
    let out = run_vl_raw(home, home, &stub, &["NEW"], &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "NEW with neither goal nor --init-prompt-file must exit non-zero: {stderr}"
    );
    assert!(
        !home.join("goals").read_dir().map(|mut it| it.next().is_some()).unwrap_or(false),
        "no goal dir may be created when no goal source is supplied"
    );
}

#[test]
fn new_with_init_prompt_file_trims_trailing_newline() {
    // A prompt file commonly ends with a trailing newline. The goalText must not carry
    // stray whitespace-only artifacts that bloat the verifier prompt; the implementation
    // should trim a single trailing newline (shell `echo` convention) but keep interior
    // newlines intact.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let prompt_file = home.join("init-goal.txt");
    let body = "multi\nline\ngoal body";
    fs::write(&prompt_file, format!("{body}\n")).unwrap();

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "--init-prompt-file", prompt_file.to_str().unwrap()],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "NEW exited {}: {stderr}", out.status);

    let goal = read_goal_json(home, &only_goal_id(home));
    assert_eq!(
        goal["goalText"], body,
        "a single trailing newline should be trimmed, interior newlines preserved: {goal}"
    );
}

// ===========================================================================
// RESUME --notes (append-only goal notes)
// ===========================================================================

#[test]
fn resume_with_notes_appends_each_note_to_goal_notes_file() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    // Round 1: reject so we can RESUME.
    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "goal that will be resumed with notes"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );
    assert!(!out.status.success(), "round 1 must reject");
    let goal_id = only_goal_id(home);

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &[
            "RESUME",
            &goal_id,
            "--notes",
            "first appended note",
            "--notes",
            "second appended note",
        ],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "RESUME with --notes must complete the round: {stderr}"
    );

    let notes_path = home.join("goals").join(&goal_id).join("goal-notes.json");
    assert!(notes_path.exists(), "goal-notes.json must be created by RESUME --notes");

    let stored: Value = serde_json::from_str(&fs::read_to_string(&notes_path).unwrap()).unwrap();
    let arr = stored["notes"].as_array().expect("notes is an array");
    assert_eq!(arr.len(), 2, "each --notes value is a separate entry: {stored}");
    assert_eq!(arr[0], "first appended note");
    assert_eq!(arr[1], "second appended note");
}

#[test]
fn resume_with_notes_does_not_mutate_goal_json_or_signature_json() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "immutable goal text"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );
    assert!(!out.status.success(), "round 1 must reject");
    let goal_id = only_goal_id(home);
    let gdir = home.join("goals").join(&goal_id);

    let goal_before = fs::read(gdir.join("goal.json")).unwrap();
    let sig_before = fs::read(gdir.join("signature.json")).unwrap();

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["RESUME", &goal_id, "--notes", "a note appended to the goal"],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "RESUME must pass: {stderr}");

    let goal_after = fs::read(gdir.join("goal.json")).unwrap();
    let sig_after = fs::read(gdir.join("signature.json")).unwrap();
    assert_eq!(
        goal_before, goal_after,
        "goal.json must be byte-identical after RESUME --notes (no strip/update of the initial goal)"
    );
    assert_eq!(
        sig_before, sig_after,
        "signature.json must be byte-identical after RESUME --notes"
    );
}

#[test]
fn resume_with_notes_across_rounds_appends_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    // Round 1: reject.
    run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "goal with multiple note rounds"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );
    let goal_id = only_goal_id(home);

    // Round 2: RESUME with a note, reject again.
    run_vl_raw(
        home,
        home,
        &stub,
        &["RESUME", &goal_id, "--notes", "note from round 2"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );

    // Round 3: RESUME with another note.
    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["RESUME", &goal_id, "--notes", "note from round 3"],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "round 3 must pass: {stderr}");

    let notes_path = home.join("goals").join(&goal_id).join("goal-notes.json");
    let stored: Value = serde_json::from_str(&fs::read_to_string(&notes_path).unwrap()).unwrap();
    let arr = stored["notes"].as_array().unwrap();
    assert_eq!(
        arr.len(),
        2,
        "notes from round 2 AND round 3 must both be present (append-only across rounds): {stored}"
    );
    assert_eq!(arr[0], "note from round 2");
    assert_eq!(arr[1], "note from round 3");
}

#[test]
fn resume_notes_appear_concatenated_in_verifier_prompt() {
    // The verifier prompt must carry the original goalText FOLLOWED BY each appended note
    // on its own line (auto-concat in the code logic — the on-disk goal.json stays
    // immutable; only the rendered prompt sees the concatenation).
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    let original_goal = "ORIGINAL GOAL TEXT marker";
    run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", original_goal],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );
    let goal_id = only_goal_id(home);

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &[
            "RESUME",
            &goal_id,
            "--notes",
            "NOTE ALPHA marker",
            "--notes",
            "NOTE BETA marker",
        ],
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

    // The original goal text is present.
    assert!(
        prompt.contains(original_goal),
        "prompt must contain the original goalText: {prompt}"
    );
    // Each appended note is present, each on its own line in the goal block.
    assert!(
        prompt.contains("NOTE ALPHA marker"),
        "prompt must contain the first appended note: {prompt}"
    );
    assert!(
        prompt.contains("NOTE BETA marker"),
        "prompt must contain the second appended note: {prompt}"
    );
    // The two notes appear on separate lines (auto-concat joins them with newlines).
    assert!(
        prompt.contains("NOTE ALPHA marker\n") && prompt.contains("NOTE BETA marker"),
        "notes must be newline-joined in the prompt"
    );
}

#[test]
fn resume_without_notes_does_not_create_goal_notes_file() {
    // Backward compatibility: RESUME without --notes must NOT create an empty
    // goal-notes.json (no behavior change for existing users).
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir(home, 1, 1);

    run_vl_raw(
        home,
        home,
        &stub,
        &["NEW", "goal resumed without notes"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );
    let goal_id = only_goal_id(home);

    run_vl_raw(
        home,
        home,
        &stub,
        &["RESUME", &goal_id, "--fix", "just a fix note"],
        &[],
    );

    let notes_path = home.join("goals").join(&goal_id).join("goal-notes.json");
    assert!(
        !notes_path.exists(),
        "RESUME without --notes must not create goal-notes.json (backward compatible)"
    );
}
