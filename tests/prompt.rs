// tasks.md §9 — Verifier prompt rendering (blind + frozen artifacts).
// RED phase: written first, against the spec, before implementation.
//
// Covers verifier-prompt spec scenarios + LD11/LD12/LD13/LD14/LD24:
//   * Template vars: goalId, verifierId, round, prevRound, goalText, context, fixNotes,
//     prevNotes, cwd, gitStatus, fileEditTimes, gitDiff, gitDiffMaxChars, {{process.env.*}}.
//   * Frozen snapshot captured at spawn (cwd, git status --porcelain, file edit times,
//     git diff truncated to gitDiffMaxChars default 10000).
//   * Diff truncation with indicator when over the cap.
//   * Snapshot consistency within a round (2 verifiers same round = byte-identical).
//   * initial-prompt.txt persisted per verifier before spawn.
//   * BLINDNESS (critical): rendered prompt MUST NOT contain round number (unless opted-in
//     via template var), other verdicts, n/m config, or the completion hash.
//   * Resume template: {{fixNotes}} (A's --fix text), {{prevNotes}} (own prior notes).
//     Omit var -> not shown. No boolean flag; template IS config (LD24).
//   * Null template -> baked-in default incl verifier policy text.
//   * {{process.env.TICKET_URL}} interpolation.

use std::fs;
use std::path::Path;

use verifier_loop::prompt::{
    self, PromptVars, Snapshot, INITIAL_PROMPT_FILE,
};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn vars_default() -> PromptVars<'static> {
    PromptVars {
        goal_id: "goal-123",
        verifier_id: "v1",
        round: 3,
        prev_round: None,
        goal_text: "build the thing",
        context: Some("optional context"),
        fix_notes: None,
        prev_notes: None,
        cwd: "/repo",
        git_status: " M src/lib.rs\n",
        file_edit_times: "src/lib.rs:1234567\n",
        git_diff: "diff --git a/src/lib.rs b/src/lib.rs\n+fn new() {}\n",
        git_diff_max_chars: 10_000,
        truncated: false,
    }
}

/// Build a real throwaway git repo for snapshot capture tests.
fn temp_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    run(p, "git", &["init", "-q"]);
    run(p, "git", &["config", "user.email", "t@t.t"]);
    run(p, "git", &["config", "user.name", "t"]);
    fs::write(p.join("file.txt"), "initial\n").unwrap();
    run(p, "git", &["add", "."]);
    run(p, "git", &["commit", "-q", "-m", "init"]);
    // make a dirty diff
    fs::write(p.join("file.txt"), "changed\n").unwrap();
    dir
}

fn run(cwd: &Path, prog: &str, args: &[&str]) -> String {
    let out = std::process::Command::new(prog)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("running {prog}: {e}"));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ---------------------------------------------------------------------------
// Template variable substitution
// ---------------------------------------------------------------------------

#[test]
fn template_vars_substituted() {
    let v = vars_default();
    let out = prompt::render(Some("{{goalId}}|{{verifierId}}|{{goalText}}"), &v).unwrap();
    assert_eq!(out, "goal-123|v1|build the thing");
}

#[test]
fn all_snapshot_vars_substituted() {
    let v = vars_default();
    let out = prompt::render(
        Some("[{{cwd}}][{{gitStatus}}][{{fileEditTimes}}][{{gitDiff}}][{{gitDiffMaxChars}}]"),
        &v,
    )
    .unwrap();
    assert!(out.contains("[/repo]"), "cwd: {out}");
    assert!(out.contains("[ M src/lib.rs\n]"), "gitStatus: {out}");
    assert!(out.contains("[src/lib.rs:1234567\n]"), "fileEditTimes: {out}");
    assert!(out.contains("[diff --git"), "gitDiff: {out}");
    assert!(out.contains("[10000]"), "gitDiffMaxChars: {out}");
}

#[test]
fn context_substituted_when_present() {
    let v = vars_default();
    let out = prompt::render(Some("ctx={{context}}"), &v).unwrap();
    assert_eq!(out, "ctx=optional context");
}

#[test]
fn unknown_var_renders_empty_no_panic() {
    let v = vars_default();
    let out = prompt::render(Some("a={{totallyUnknown}}b"), &v).unwrap();
    assert_eq!(out, "a=b", "unknown vars must disappear, not stay as {{...}}");
}

// ---------------------------------------------------------------------------
// Env var interpolation
// ---------------------------------------------------------------------------

#[test]
fn env_var_interpolated() {
    std::env::set_var("VL_TEST_TICKET_URL", "https://t.example/JIRA-1");
    let v = vars_default();
    let out = prompt::render(Some("ticket={{process.env.VL_TEST_TICKET_URL}}"), &v).unwrap();
    assert_eq!(out, "ticket=https://t.example/JIRA-1");
    std::env::remove_var("VL_TEST_TICKET_URL");
}

#[test]
fn missing_env_var_renders_empty() {
    std::env::remove_var("VL_DEFINITELY_UNSET_VAR");
    let v = vars_default();
    let out = prompt::render(Some("[{{process.env.VL_DEFINITELY_UNSET_VAR}}]"), &v).unwrap();
    assert_eq!(out, "[]", "missing env var must render empty, not error");
}

// ---------------------------------------------------------------------------
// Null template -> baked-in default with verifier policy
// ---------------------------------------------------------------------------

#[test]
fn null_template_uses_baked_default_with_policy() {
    let v = vars_default();
    let out = prompt::render(None, &v).unwrap();
    // Default embeds the verifier detective policy text.
    assert!(
        out.contains("Verifier") || out.contains("detective") || out.contains("ZERO trust"),
        "default must embed verifier policy text; got: {out}"
    );
    // And still carries the goal.
    assert!(out.contains("build the thing"), "default must render goalText: {out}");
    assert!(out.contains("v1"), "default must render verifierId: {out}");
}

#[test]
fn default_template_const_contains_policy() {
    let t = prompt::default_template();
    assert!(
        t.contains("Verifier") || t.contains("detective") || t.contains("ZERO trust"),
        "default_template const must embed policy"
    );
    assert!(t.contains("{{goalText}}"), "default must reference {{goalText}}");
    assert!(t.contains("{{verifierId}}"), "default must reference {{verifierId}}");
}

// ---------------------------------------------------------------------------
// Diff truncation
// ---------------------------------------------------------------------------

#[test]
fn diff_truncated_with_indicator_when_over_cap() {
    let big = "x".repeat(500);
    let v = PromptVars {
        git_diff: &big,
        git_diff_max_chars: 100,
        truncated: false,
        ..vars_default()
    };
    let out = prompt::render(Some("{{gitDiff}}"), &v).unwrap();
    assert!(
        out.len() < big.len(),
        "truncated render must be shorter than the source"
    );
    assert!(
        out.contains("truncated") || out.contains("…"),
        "truncation indicator must be present: {out}"
    );
}

#[test]
fn snapshot_diff_truncated_flag_and_indicator() {
    let dir = temp_git_repo();
    // Produce a large UNSTAGED change on a tracked file so `git diff` emits it.
    let mut content = fs::read_to_string(dir.path().join("file.txt")).unwrap();
    content.push_str(&"z".repeat(5_000));
    fs::write(dir.path().join("file.txt"), content).unwrap();
    // do NOT stage — git diff (unstaged) must surface it.

    let snap = prompt::capture_snapshot(dir.path(), 100).unwrap();
    assert!(snap.truncated, "snapshot over cap must set truncated=true");
    assert!(snap.git_diff.len() <= 100 + 64, "git_diff must be near the cap: {}", snap.git_diff.len());
    assert!(
        snap.git_diff.contains("truncated") || snap.git_diff.contains("…"),
        "truncated snapshot diff must carry indicator: {}",
        snap.git_diff
    );
}

#[test]
fn snapshot_diff_not_truncated_when_under_cap() {
    let dir = temp_git_repo();
    let snap = prompt::capture_snapshot(dir.path(), 10_000).unwrap();
    assert!(!snap.truncated, "under-cap snapshot must set truncated=false");
    assert!(
        !snap.git_diff.contains("truncated") && !snap.git_diff.contains("…"),
        "under-cap diff must NOT carry truncation indicator: {}",
        snap.git_diff
    );
}

// ---------------------------------------------------------------------------
// Snapshot content + consistency within a round (LD11)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_records_cwd_git_status_and_file_edit_times() {
    let dir = temp_git_repo();
    let snap = prompt::capture_snapshot(dir.path(), 10_000).unwrap();
    assert_eq!(snap.cwd, dir.path().to_string_lossy(), "snapshot cwd must equal repo");
    assert!(
        snap.git_status.contains("file.txt"),
        "git_status must reflect porcelain output: {}",
        snap.git_status
    );
    assert!(
        !snap.file_edit_times.is_empty(),
        "file_edit_times must be captured: {}",
        snap.file_edit_times
    );
}

#[test]
fn snapshot_consistent_within_round() {
    let dir = temp_git_repo();
    let a = prompt::capture_snapshot(dir.path(), 10_000).unwrap();
    let b = prompt::capture_snapshot(dir.path(), 10_000).unwrap();
    assert_eq!(a.git_status, b.git_status, "same round = identical git_status");
    assert_eq!(a.git_diff, b.git_diff, "same round = identical git_diff");
    assert_eq!(a.file_edit_times, b.file_edit_times, "same round = identical file_edit_times");
}

#[test]
fn snapshot_in_non_git_repo_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let res = prompt::capture_snapshot(dir.path(), 10_000);
    assert!(res.is_err(), "capture_snapshot in a non-git repo must fail closed, not silently empty");
}

// ---------------------------------------------------------------------------
// BLINDNESS (critical) — LD12 / LD13
// ---------------------------------------------------------------------------

#[test]
fn blindness_default_hides_round_number() {
    // Round 3. The default template must NOT surface the round number anywhere.
    let v = vars_default(); // round = 3
    let out = prompt::render(None, &v).unwrap();
    assert!(
        !out.contains("Round 3") && !out.contains("round: 3") && !out.contains("round=3")
            && !out.contains("Round:3"),
        "default must not reveal round number; got: {out}"
    );
}

#[test]
fn blindness_round_visible_only_when_template_opts_in() {
    let v = vars_default();
    let out = prompt::render(Some("current round = {{round}}"), &v).unwrap();
    assert_eq!(out, "current round = 3", "opted-in round var must interpolate");
}

#[test]
fn blindness_no_hash_or_threshold_in_output() {
    let v = vars_default();
    let out = prompt::render(None, &v).unwrap();
    assert!(!out.contains("vl:"), "default must not contain completion hash prefix");
    assert!(
        !out.contains("n/m") && !out.contains("2/2") && !out.contains("2 of 2"),
        "default must not reveal n/m threshold: {out}"
    );
}

#[test]
fn blindness_no_other_verdicts_leak() {
    // PromptVars has no field for other verdicts — structurally enforced.
    // Verify the rendered output of the default template has no APPROVE/REJECT markers
    // that would imply knowledge of peer verdicts.
    let v = vars_default();
    let out = prompt::render(None, &v).unwrap();
    assert!(
        !out.contains("V2 APPROVE") && !out.contains("verifier v2") && !out.contains("peer verdict"),
        "default must not reference other verifiers' verdicts: {out}"
    );
}

// ---------------------------------------------------------------------------
// Resume template (LD24): fixNotes + prevNotes, template IS config
// ---------------------------------------------------------------------------

#[test]
fn resume_fix_notes_interpolated() {
    let v = PromptVars {
        fix_notes: Some("fixed the off-by-one in consensus"),
        ..vars_default()
    };
    let out = prompt::render_resume(Some("fixes:\n{{fixNotes}}"), &v).unwrap();
    assert!(out.contains("fixed the off-by-one in consensus"), "{out}");
}

#[test]
fn resume_omits_prev_notes_when_var_absent() {
    let v = PromptVars {
        prev_notes: Some("my own prior secret notes"),
        ..vars_default()
    };
    // Template does NOT mention {{prevNotes}} -> prior notes must NOT appear.
    let out = prompt::render_resume(Some("resume: {{goalText}}"), &v).unwrap();
    assert!(
        !out.contains("my own prior secret notes"),
        "prev notes must not leak when template omits {{prevNotes}}: {out}"
    );
}

#[test]
fn resume_includes_own_prev_notes_when_var_present() {
    let v = PromptVars {
        prev_notes: Some("my round-1 notes"),
        ..vars_default()
    };
    let out = prompt::render_resume(Some("you previously said: {{prevNotes}}"), &v).unwrap();
    assert!(out.contains("my round-1 notes"), "{out}");
}

#[test]
fn resume_prev_notes_are_own_only_not_peer() {
    // prevNotes is documented as this verifier's OWN prior notes (LD13).
    // There is no field for peer notes in PromptVars — verify by construction.
    let v = PromptVars {
        prev_notes: Some("MY notes"),
        ..vars_default()
    };
    let out = prompt::render_resume(Some("{{prevNotes}}"), &v).unwrap();
    assert_eq!(out, "MY notes");
}

#[test]
fn null_resume_template_uses_baked_default() {
    let v = vars_default();
    let out = prompt::render_resume(None, &v).unwrap();
    assert!(
        out.contains("Verifier") || out.contains("detective") || out.contains("ZERO trust"),
        "default resume must embed policy: {out}"
    );
    assert!(out.contains("build the thing"), "default resume must render goalText: {out}");
}

#[test]
fn default_resume_template_const_exists_and_has_placeholders() {
    let t = prompt::default_resume_template();
    assert!(t.contains("{{goalText}}"), "resume default must reference goalText");
    assert!(t.contains("{{fixNotes}}"), "resume default must reference fixNotes");
}

// ---------------------------------------------------------------------------
// initial-prompt.txt persistence (spec: stored per verifier before spawn)
// ---------------------------------------------------------------------------

#[test]
fn write_initial_prompt_persists_file_in_slot() {
    let dir = tempfile::tempdir().unwrap();
    let goal_root = dir.path();
    let rendered = "RENDERED PROMPT BODY";
    let path = prompt::write_initial_prompt(goal_root, "goal-7", "v1", 1, rendered).unwrap();
    assert_eq!(path.file_name().unwrap(), INITIAL_PROMPT_FILE);
    assert!(path.exists(), "initial-prompt.txt must exist after write");
    assert_eq!(fs::read_to_string(&path).unwrap(), rendered, "content must match rendered");
    // Slot path layout: rounds/<round>/<verifierId>/initial-prompt.txt
    assert!(
        path.starts_with(goal_root.join("rounds").join("1").join("v1")),
        "path layout must be rounds/<round>/<verifierId>/initial-prompt.txt: {}",
        path.display()
    );
}

#[test]
fn write_initial_prompt_overwrites_on_resume_same_slot() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    prompt::write_initial_prompt(root, "g", "v1", 1, "first").unwrap();
    prompt::write_initial_prompt(root, "g", "v1", 1, "second").unwrap();
    assert_eq!(
        fs::read_to_string(root.join("rounds").join("1").join("v1").join(INITIAL_PROMPT_FILE))
            .unwrap(),
        "second",
        "rewrite in same slot must overwrite"
    );
}

// ---------------------------------------------------------------------------
// Round-trip: vars + None template -> rendered -> persisted (integration)
// ---------------------------------------------------------------------------

#[test]
fn render_then_persist_full_flow() {
    let dir = tempfile::tempdir().unwrap();
    let v = vars_default();
    let rendered = prompt::render(None, &v).unwrap();
    let path = prompt::write_initial_prompt(dir.path(), "goal-123", "v1", 1, &rendered).unwrap();
    let on_disk = fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, rendered);
    assert!(on_disk.contains("build the thing"));
}

// ---------------------------------------------------------------------------
// Snapshot struct is serializable for goal-dir audit
// ---------------------------------------------------------------------------

#[test]
fn snapshot_is_serializable() {
    let s = Snapshot {
        cwd: "/r".into(),
        git_status: " M x\n".into(),
        file_edit_times: "x:1\n".into(),
        git_diff: "diff".into(),
        git_diff_max_chars: 10_000,
        truncated: false,
    };
    let j = serde_json::to_string(&s).unwrap();
    assert!(j.contains("\"cwd\":\"/r\""), "{j}");
    assert!(j.contains("\"gitDiffMaxChars\":10000"), "camelCase on disk: {j}");
    let back: Snapshot = serde_json::from_str(&j).unwrap();
    assert_eq!(back.cwd, "/r");
}
