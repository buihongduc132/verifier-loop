// add-round-recovery — CLI e2e for `RECOVER` + `STATUS` (LD3/LD5/LD7/LD8).
//
// RED phase for Group 4 (CLI wiring). Exercises the built `verifier-loop` binary via
// assert_cmd + a deterministic stub backend (no real pi). Reuses the cli_e2e.rs helper
// idioms: tempdir HOME, git work tree, config.json (backend "stub"), stub script that
// emits an ACP stream then registers a verdict via the built `verifier-verdict`.
//
// Covered scenarios:
//   * STATUS prints a JSON object with goalId/round/state/needs/slots (LD7).
//   * RECOVER on a complete round warns + exits 0 (LD3).
//   * RECOVER on a dead-null round exits non-zero with RESUME guidance (LD8).
//   * RECOVER harvests a verdict that appears on disk mid-poll (LD8) — simulate an
//     orphan writing its signed verdict by minting+signing via the lib between NEW
//     (which left the slot null) and RECOVER.
//   * A second concurrent RESUME exits non-zero "goal busy" (LD5).
//   * RESUME on a round with a null slot warns about RECOVER (LD3).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use verifier_loop::{goal, verdict};

fn bin(name: &str) -> PathBuf {
    assert_cmd::cargo::cargo_bin(name)
}

fn verdict_bin_path() -> PathBuf {
    bin("verifier-verdict")
}

/// Run `verifier-loop` as a raw subprocess; return its full output regardless of exit.
fn run_vl_raw(cwd: &Path, home: &Path, stub: &Path, args: &[&str]) -> std::process::Output {
    let mut c = std::process::Command::new(bin("verifier-loop"));
    c.args(args)
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", stub)
        .current_dir(cwd);
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

/// Stub backend: emits ACP stream then registers a verdict. Reject if
/// `$VERIFIER_LOOP_STUB_VERDICT` is "reject". Approve otherwise.
fn stub_script(dir: &Path) -> PathBuf {
    let verdict = verdict_bin_path();
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

/// A stub that registers a verdict ONLY for v1; v2 stays null (simulating an orphan still
/// running / a slot that has not yet produced a verdict). This lets us drive RECOVER into
/// the dead-null path.
fn stub_only_v1_script(dir: &Path) -> PathBuf {
    let verdict = verdict_bin_path();
    let v = verdict.to_string_lossy();
    write_script(
        dir,
        "stub_only_v1.sh",
        &format!(
            r#"#!/bin/sh
cat <<'ACP'
{{"type":"session","id":"stub-session-id"}}
{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"stub final output"}}]}}],"willRetry":false}}
ACP
# Only v1 registers a verdict; v2 stays null (orphan never finished).
if [ "$VERIFIER_LOOP_VERIFIER_ID" = "v1" ]; then
  "{v}" approve
fi
"#,
        ),
    )
}

/// Seed a git work tree at `dir` with config.json + the named stub. `dir` is used BOTH
/// as the store HOME (config.json lives here) AND as the cwd (git work tree for the frozen
/// snapshot) — mirroring the cli_e2e.rs baseline. Returns the stub path.
fn seed_workdir_with(dir: &Path, n: u32, m: u32, stub: PathBuf) -> PathBuf {
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
    for (k, v) in [("user.email", "t@e.com"), ("user.name", "T")] {
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
    stub
}

/// Extract the goalId printed by `NEW` from stdout.
fn goal_id_from_new(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("goalId: ").map(|s| s.trim().to_string())
    })
}

// ===========================================================================
// STATUS (LD7)
// ===========================================================================

#[test]
fn status_prints_json_with_documented_fields() {
    // One tempdir is BOTH the store HOME (config.json) and the cwd (git work tree).
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir_with(home, 1, 1, stub_script(home));

    // NEW a goal so there is a round with a verdict.
    let out = run_vl_raw(home, home, &stub, &["NEW", "ship it"]);
    assert!(
        out.status.success(),
        "NEW failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let goal_id = goal_id_from_new(&String::from_utf8_lossy(&out.stdout)).unwrap();

    // STATUS must print a single JSON object with the documented fields.
    let out = run_vl_raw(home, home, &stub, &["STATUS", &goal_id]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(json["goalId"], goal_id);
    assert!(json["round"].as_u64().is_some());
    assert!(json["state"].is_string());
    assert!(json["needs"].is_string());
    assert!(json["slots"].is_array());
    // goal-status spec: every slot has an id AND a verdict field, and this goal's single
    // slot approved, so verdict must be the canonical APPROVE string (not just present).
    let slot = &json["slots"][0];
    assert_eq!(
        slot["verdict"], "APPROVE",
        "approved slot must serialize verdict as APPROVE: {slot}"
    );
}

// ===========================================================================
// RECOVER (LD3 / LD8)
// ===========================================================================

#[test]
fn recover_on_complete_round_warns_and_exits_zero() {
    // LD3: RECOVER on a round that already reached consensus is a no-op + warning.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir_with(home, 1, 1, stub_script(home));

    let out = run_vl_raw(home, home, &stub, &["NEW", "ship it"]);
    assert!(out.status.success(), "NEW failed");
    let goal_id = goal_id_from_new(&String::from_utf8_lossy(&out.stdout)).unwrap();

    let out = run_vl_raw(home, home, &stub, &["RECOVER", &goal_id]);
    assert!(
        out.status.success(),
        "RECOVER on complete round must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("resume"),
        "must warn referencing RESUME: {stderr}"
    );
}

#[test]
fn recover_dead_null_round_exits_nonzero_with_resume_guidance() {
    // LD8: a round with a null slot whose orphan never writes => RECOVER times out,
    // exits non-zero, points to RESUME. We use a stub that only approves v1 (n=m=2),
    // leaving v2 null. The recovery timeout is bounded by a small verifierTimeoutSec.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir_with(home, 2, 2, stub_only_v1_script(home));

    let out = run_vl_raw(home, home, &stub, &["NEW", "ship it"]);
    // NEW itself fails (no 2/2 consensus), which is expected.
    let goal_id = goal_id_from_new(&String::from_utf8_lossy(&out.stdout))
        .expect("goalId must be printed even on round failure");

    let out = run_vl_raw(home, home, &stub, &["RECOVER", &goal_id]);
    assert!(
        !out.status.success(),
        "RECOVER on a dead-null round must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("resume"),
        "guidance must point to RESUME: {stderr}"
    );
    // No completion.json for this goal.
    let comp = home.join("goals").join(&goal_id).join("completion.json");
    assert!(!comp.exists(), "no completion.json on dead-null recover");
}

#[test]
fn recover_harvests_signed_verdict_written_mid_poll() {
    // LD8: a null slot whose (simulated) orphan writes a signed APPROVE mid-poll =>
    // RECOVER observes it and reaches consensus. We build the goal + signed slots
    // directly via the lib (we control the pinned keys), then invoke the CLI RECOVER.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // A config.json is needed (n=2,m=2) so RECOVER reads the right threshold.
    let cfg = serde_json::json!({
        "n": 2, "m": 2, "maxTurn": 3, "backend": "stub",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    fs::write(home.join("config.json"), cfg.to_string()).unwrap();

    // Build a goal directly via the lib, round 1.
    let goal_id = verifier_loop::goal::new(home, "ship the thing", None).unwrap();
    let round = 1u32;
    // v1 already approved (signed, pinned) — like an orphan that already finished.
    let sk_v1 = verdict::mint_and_pin_pubkey(home, &goal_id, "v1", round).unwrap();
    verdict::register_signed_approve(home, &goal_id, "v1", round, None, &sk_v1).unwrap();
    // v2 slot: pre-create null verdict + meta (orphan "still running"). We pre-mint v2's
    // key now (spawn would have) so the simulated orphan can sign under the pinned key.
    let sk_v2 = verdict::mint_and_pin_pubkey(home, &goal_id, "v2", round).unwrap();
    let v2_dir = goal::goal_dir(home, &goal_id)
        .join(goal::ROUNDS_DIR)
        .join(round.to_string())
        .join("v2");
    fs::create_dir_all(&v2_dir).unwrap();
    fs::write(v2_dir.join(verdict::VERDICT_FILE), r#"{"status":null}"#).unwrap();
    fs::write(v2_dir.join("meta.json"), r#"{"turnsUsed":0}"#).unwrap();

    // Simulate the orphan writing its signed APPROVE shortly after RECOVER starts. We do
    // this from the same process (the lib call) on a background thread.
    let root = home.to_path_buf();
    let gid = goal_id.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        let _ = verdict::register_signed_approve(&root, &gid, "v2", round, None, &sk_v2);
    });

    // RECOVER must harvest v2's verdict and print a short hash. cwd is irrelevant for
    // RECOVER (no snapshot), but it must be a valid dir.
    let out = run_vl_raw(home, home, &v2_dir, &["RECOVER", &goal_id]);
    assert!(
        out.status.success(),
        "RECOVER should reach consensus; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let hash_line = stdout.lines().rev().find(|l| {
        let l = l.trim();
        l.len() == 15 && l[6..7].to_string() == "-" && l[..6].chars().all(|c| c.is_ascii_digit())
    });
    assert!(
        hash_line.is_some(),
        "RECOVER must print a completion hash; got: {stdout}"
    );
    let comp = home.join("goals").join(&goal_id).join("completion.json");
    assert!(comp.exists(), "completion.json must be written by RECOVER");
}

// ===========================================================================
// Goal lock (LD5) + RESUME warning (LD3)
// ===========================================================================

#[test]
fn concurrent_resume_is_rejected_as_goal_busy() {
    // LD5: while one RESUME holds the goal lock, a second RESUME exits non-zero "goal busy".
    // We hold the lock from this test process via the lib, then invoke the CLI RESUME which
    // must fail fast.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir_with(home, 1, 1, stub_script(home));

    let out = run_vl_raw(home, home, &stub, &["NEW", "ship it"]);
    let goal_id = goal_id_from_new(&String::from_utf8_lossy(&out.stdout)).unwrap();

    // Hold the lock from this process.
    let _lock = verifier_loop::round_recover::GoalLock::acquire_exclusive(home, &goal_id).unwrap();

    // CLI RESUME while the lock is held must fail fast.
    let out = run_vl_raw(home, home, &stub, &["RESUME", &goal_id, "--fix", "x"]);
    assert!(
        !out.status.success(),
        "concurrent RESUME must exit non-zero while the goal is locked"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("busy"),
        "must report goal busy: {stderr}"
    );
}

#[test]
fn resume_warns_when_round_has_null_verdict() {
    // LD3: RESUME on a round with a null slot warns about RECOVER but proceeds.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let stub = seed_workdir_with(home, 2, 2, stub_only_v1_script(home));

    let out = run_vl_raw(home, home, &stub, &["NEW", "ship it"]);
    let goal_id = goal_id_from_new(&String::from_utf8_lossy(&out.stdout)).unwrap();

    // RESUME must warn (round 1 has a null v2). It then proceeds to round 2.
    let out = run_vl_raw(home, home, &stub, &["RESUME", &goal_id, "--fix", "retry"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("recover"),
        "RESUME must warn referencing RECOVER when the prior round has a null slot: {stderr}"
    );
}
