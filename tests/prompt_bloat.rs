// tasks.md §1, §3 (D1, D3, D4) — Prompt bloat fixes.
// RED phase: written first, against the spec, BEFORE any implementation.
//
// verifier-prompt MODIFIED + ADDED requirements:
//   * fileEditTimes scoped to changed files only (git status --porcelain, not ls-files).
//   * fileEditTimes block byte-capped (fileEditTimesMaxChars, default 8000).
//   * --context input capped (contextMaxChars, default 20000).
//   * Rendered-prompt budget warning when over promptBudgetBytes (default 50000).
//
// API targets for the GREEN author (documented here so the tests pin the contract):
//   * Config.file_edit_times_max_chars: u64  (serde "fileEditTimesMaxChars", default 8000)
//   * Config.context_max_chars: u64          (serde "contextMaxChars", default 20000)
//   * Config.prompt_budget_bytes: u64        (serde "promptBudgetBytes", default 50000)
//   * pub fn capture_file_edit_times(cwd: &Path, max_chars: u64) -> Result<String, PromptError>
//       (today: private, 1-arg, uses `git ls-files`. GREEN: pub, 2-arg, uses `git status
//        --porcelain`, truncates over-cap block with an indicator.)
//   * pub fn cap_context(context: &str, max_chars: u64) -> (String, bool)
//       (today: does not exist. GREEN: truncates --context to max_chars with indicator.)
//   * pub fn budget_warning(rendered: &str, budget: usize) -> Option<String>
//       (today: does not exist. GREEN: returns Some(breakdown) when rendered > budget,
//        None otherwise. Does NOT block spawn.)
//
// Every test below FAILS today (compile error for missing fields/fns, or assertion
// failure for the changed-files scoping which uses the existing capture_snapshot API).

use std::fs;
use std::path::Path;

use verifier_loop::{prompt, store::Config};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn run(cwd: &Path, prog: &str, args: &[&str]) -> String {
    let out = std::process::Command::new(prog)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("running {prog}: {e}"));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Build a throwaway git repo with `tracked` committed files, then modify `changed` of
/// them so `git status --porcelain` lists exactly `changed` entries.
fn temp_git_repo_with(tracked: usize, changed: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    run(p, "git", &["init", "-q"]);
    run(p, "git", &["config", "user.email", "t@t.t"]);
    run(p, "git", &["config", "user.name", "t"]);
    for i in 0..tracked {
        fs::write(p.join(format!("file_{i:03}.txt")), "initial\n").unwrap();
    }
    run(p, "git", &["add", "."]);
    run(p, "git", &["commit", "-q", "-m", "init"]);
    // Modify the first `changed` tracked files.
    for i in 0..changed {
        fs::write(p.join(format!("file_{i:03}.txt")), "changed\n").unwrap();
    }
    dir
}

// ===========================================================================
// Group 1 (D1) — fileEditTimes scoped to changed files only
// ===========================================================================

// ---------------------------------------------------------------------------
// §1.1 RED: capture_file_edit_times returns entries only for changed files.
// Uses the existing public capture_snapshot API. Today capture_file_edit_times uses
// `git ls-files` and emits ALL 100 tracked files → assertion FAILS.
// ---------------------------------------------------------------------------

#[test]
fn capture_file_edit_times_scoped_to_changed_files() {
    let dir = temp_git_repo_with(100, 3);
    let snap = prompt::capture_snapshot(dir.path(), 100_000).unwrap();

    let entries: Vec<&str> = snap.file_edit_times.lines().filter(|l| !l.is_empty()).collect();
    // After GREEN: only the 3 changed files appear. Today: all 100 tracked files.
    assert_eq!(
        entries.len(),
        3,
        "fileEditTimes must list ONLY changed files (git status --porcelain), not all \
         tracked files; got {} entries:\n{}",
        entries.len(),
        snap.file_edit_times
    );
    // Each entry must correspond to a changed file (file_000, file_001, file_002).
    for e in &entries {
        let path = e.split(':').next().unwrap_or("");
        assert!(
            path.starts_with("file_00"),
            "changed-file entry expected, got: {e}"
        );
    }
}

// ---------------------------------------------------------------------------
// §1.2 RED: fileEditTimesMaxChars truncates the block when exceeded.
// Today capture_file_edit_times is PRIVATE + 1-arg → compile error.
// ---------------------------------------------------------------------------

#[test]
fn file_edit_times_byte_capped() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    run(p, "git", &["init", "-q"]);
    run(p, "git", &["config", "user.email", "t@t.t"]);
    run(p, "git", &["config", "user.name", "t"]);
    // Create + commit many tracked files, then change ALL of them so the changed-files
    // block is large.
    for i in 0..200 {
        fs::write(p.join(format!("f_{i:03}.txt")), "x\n").unwrap();
    }
    run(p, "git", &["add", "."]);
    run(p, "git", &["commit", "-q", "-m", "init"]);
    for i in 0..200 {
        fs::write(p.join(format!("f_{i:03}.txt")), "changed\n").unwrap();
    }

    // GREEN target: pub fn capture_file_edit_times(cwd, max_chars). A 50-byte cap must
    // truncate the >50-byte block with an indicator.
    let block = prompt::capture_file_edit_times(p, 50).expect("changed-files block captures");
    assert!(
        block.len() <= 50 + 64,
        "byte-capped block must be near the 50-char cap, got {} bytes: {block}",
        block.len()
    );
    assert!(
        block.contains("truncated") || block.contains("…"),
        "over-cap block must carry a truncation indicator: {block}"
    );
}

// ===========================================================================
// Group 1/3 — new Config fields (D1/D3/D4)
// ===========================================================================

// ---------------------------------------------------------------------------
// §1.4/§3.3 RED: Config::default() has the three new bloat caps with spec defaults.
// Today these fields do not exist → compile error.
// ---------------------------------------------------------------------------

#[test]
fn config_defaults_for_bloat_caps() {
    let d = Config::default();
    assert_eq!(
        d.file_edit_times_max_chars, 8_000,
        "fileEditTimesMaxChars default must be 8000"
    );
    assert_eq!(
        d.context_max_chars, 20_000,
        "contextMaxChars default must be 20000"
    );
    assert_eq!(
        d.prompt_budget_bytes, 50_000,
        "promptBudgetBytes default must be 50000"
    );
}

// ---------------------------------------------------------------------------
// §1.4/§3.3 RED: new fields round-trip through serde with camelCase keys.
// Today fields do not exist → compile error.
// ---------------------------------------------------------------------------

#[test]
fn config_bloat_caps_round_trip_camel_case() {
    let cfg = Config {
        file_edit_times_max_chars: 1_234,
        context_max_chars: 5_678,
        prompt_budget_bytes: 9_012,
        ..Config::default()
    };
    let j = serde_json::to_string(&cfg).unwrap();
    assert!(j.contains("\"fileEditTimesMaxChars\":1234"), "camelCase: {j}");
    assert!(j.contains("\"contextMaxChars\":5678"), "camelCase: {j}");
    assert!(j.contains("\"promptBudgetBytes\":9012"), "camelCase: {j}");

    let back: Config = serde_json::from_str(&j).unwrap();
    assert_eq!(back.file_edit_times_max_chars, 1_234);
    assert_eq!(back.context_max_chars, 5_678);
    assert_eq!(back.prompt_budget_bytes, 9_012);
}

// ===========================================================================
// Group 3 (D3) — --context byte cap
// ===========================================================================

// ---------------------------------------------------------------------------
// §3.1 RED: cap_context truncates over-cap context with indicator.
// Today cap_context does not exist → compile error.
// ---------------------------------------------------------------------------

#[test]
fn cap_context_truncates_when_over_cap() {
    let big = "x".repeat(30_000);
    let (capped, truncated) = prompt::cap_context(&big, 20_000);
    assert!(truncated, "over-cap context must be flagged truncated");
    assert!(
        capped.len() <= 20_000 + 64,
        "capped context must be near the 20000-char cap, got {}",
        capped.len()
    );
    assert!(
        capped.contains("truncated") || capped.contains("…"),
        "capped context must carry an indicator: tail = …{}",
        &capped[capped.len().saturating_sub(60)..]
    );
}

#[test]
fn cap_context_passes_through_when_under_cap() {
    let small = "y".repeat(100);
    let (capped, truncated) = prompt::cap_context(&small, 20_000);
    assert!(!truncated, "under-cap context must not be truncated");
    assert_eq!(capped, small, "under-cap context passes through unchanged");
}

// ===========================================================================
// Group 3 (D4) — rendered-prompt budget warning
// ===========================================================================

// ---------------------------------------------------------------------------
// §3.2 RED: budget_warning returns a breakdown when rendered > budget.
// Today budget_warning does not exist → compile error.
// ---------------------------------------------------------------------------

#[test]
fn budget_warning_returns_breakdown_when_over() {
    // A rendered prompt well over the 50_000 budget.
    let rendered = "z".repeat(120_000);
    let warning = prompt::budget_warning(&rendered, 50_000);
    assert!(
        warning.is_some(),
        "over-budget prompt must yield a warning (Some)"
    );
    let text = warning.unwrap();
    assert!(
        text.contains("120000") || text.contains("120,000") || text.contains("50000"),
        "warning must cite sizes; got: {text}"
    );
    // The warning does NOT block — it is informational. (Caller decides.)
}

#[test]
fn budget_warning_returns_none_when_under() {
    let rendered = "z".repeat(30_000);
    let warning = prompt::budget_warning(&rendered, 50_000);
    assert!(
        warning.is_none(),
        "under-budget prompt must yield no warning (None)"
    );
}
