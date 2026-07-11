// add-round-recovery (SHAPE-1) — RECOVER + STATUS + GoalLock.
// RED phase: written first, against the openspec change
//   openspec/changes/add-round-recovery/specs/{round-recovery,goal-status,goal-lifecycle}/spec.md
// BEFORE the `round_recover` module exists. Every assertion here is expected to FAIL
// (compile error: module absent) until the GREEN impl lands.
//
// Implements locked decisions LD3-LD11 from
//   flow/findings/round-recovery/2026-07-12-locked-decisions.yaml
//
// SHAPE-1 contract under test:
//   * GoalLock: exclusive flock on goals/<goalId>/.lock (LD5). Second concurrent
//     acquire in the same process fails with GoalBusy. Drop releases the lock so a
//     later acquire succeeds. A crashed process leaves the lock released (advisory).
//   * status: read-only probe (no lock). state/needs derived per design §4.3 (LD7).
//   * recover: wait-only (LD8/LD10/LD11). Polls verdict.json for the current round;
//     reuses consensus::evaluate + compute_hash + write_completion unchanged. Never
//     spawns, kills, re-renders, or re-captures. Dead-null -> RESUME guidance.
//
// These tests seed real goal dirs + verdict slots (no real backend; verdicts are written
// directly to disk to simulate an orphan verifier writing a signed verdict).

use std::fs;
use std::path::Path;
use std::time::Duration;

use verifier_loop::{consensus, goal, round_recover, store, verdict};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Seed a fresh goal dir under a tempdir store root. Returns (TempDir, goalId).
fn fresh_goal() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), "ship the thing", None).unwrap();
    (dir, goal_id)
}

/// Default config (n=2, m=2).
fn cfg() -> store::Config {
    store::Config::default()
}

/// Pre-create a slot dir with a null verdict + meta, mirroring what spawn does.
fn seed_null_slot(root: &Path, goal_id: &str, round: u32, vid: &str) {
    let vdir = goal::goal_dir(root, goal_id)
        .join(goal::ROUNDS_DIR)
        .join(round.to_string())
        .join(vid);
    fs::create_dir_all(&vdir).unwrap();
    if !vdir.join(verdict::VERDICT_FILE).exists() {
        fs::write(
            vdir.join(verdict::VERDICT_FILE),
            r#"{"status":null}"#,
        )
        .unwrap();
    }
    fs::write(
        vdir.join("meta.json"),
        r#"{"turnsUsed":0}"#,
    )
    .unwrap();
}

/// A real signed APPROVE — mint the slot's pinned pubkey and sign under it, exactly like
/// a verifier process does via jewije. This produces a verdict that passes the signature
/// gate in consensus::evaluate.
fn signed_approve(
    root: &Path,
    goal_id: &str,
    round: u32,
    vid: &str,
) -> verdict::VerdictRecord {
    let sk = verdict::mint_and_pin_pubkey(root, goal_id, vid, round).unwrap();
    verdict::register_signed_approve(root, goal_id, vid, round, None, &sk).unwrap();
    verdict::read_verdict(root, goal_id, vid, round).unwrap_or(verdict::VerdictRecord {
        status: verdict::VerdictStatus::Null,
        notes: None,
        registered_at: None,
        signature: None,
        pubkey_id: None,
    })
}

/// A real signed REJECT with notes.
fn signed_reject(
    root: &Path,
    goal_id: &str,
    round: u32,
    vid: &str,
    notes: &str,
) -> verdict::VerdictRecord {
    let sk = verdict::mint_and_pin_pubkey(root, goal_id, vid, round).unwrap();
    verdict::register_signed_reject(root, goal_id, vid, round, notes, &sk).unwrap();
    verdict::read_verdict(root, goal_id, vid, round).unwrap_or(verdict::VerdictRecord {
        status: verdict::VerdictStatus::Null,
        notes: None,
        registered_at: None,
        signature: None,
        pubkey_id: None,
    })
}

// ===========================================================================
// GoalLock (LD5)
// ===========================================================================

#[test]
fn goal_lock_second_concurrent_acquire_in_same_process_fails_busy() {
    // LD5: an exclusive lock is held for its duration. A second acquire on the same
    // goal (different file handle, same process) must fail with GoalBusy rather than
    // silently proceeding (which would race double-mint / session-file corruption).
    let (dir, goal_id) = fresh_goal();
    let _g1 = round_recover::GoalLock::acquire_exclusive(dir.path(), &goal_id).unwrap();
    let second = round_recover::GoalLock::acquire_exclusive(dir.path(), &goal_id);
    assert!(
        matches!(second, Err(round_recover::RoundRecoverError::GoalBusy)),
        "second concurrent acquire must fail with GoalBusy, got: {second:?}"
    );
}

#[test]
fn goal_lock_drop_releases_so_later_acquire_succeeds() {
    // The lock must not poison the goal: once the guard drops, a fresh acquire works.
    let (dir, goal_id) = fresh_goal();
    {
        let _g = round_recover::GoalLock::acquire_exclusive(dir.path(), &goal_id).unwrap();
    } // guard dropped here
    let again = round_recover::GoalLock::acquire_exclusive(dir.path(), &goal_id);
    assert!(again.is_ok(), "after drop, a fresh acquire must succeed: {again:?}");
}

// ===========================================================================
// STATUS (LD7)
// ===========================================================================

#[test]
fn status_needs_done_state_consensus_pass_when_completion_exists() {
    let (dir, goal_id) = fresh_goal();
    let round = 1u32;
    signed_approve(dir.path(), &goal_id, round, "v1");
    signed_approve(dir.path(), &goal_id, round, "v2");

    // Drive consensus to pass so completion.json is written (reuse the real path).
    let v1 = verdict::read_verdict(dir.path(), &goal_id, "v1", round).unwrap();
    let v2 = verdict::read_verdict(dir.path(), &goal_id, "v2", round).unwrap();
    let verdicts = vec![("v1".to_string(), v1), ("v2".to_string(), v2)];
    let result = consensus::evaluate(dir.path(), &goal_id, round, &verdicts, 2, 2);
    assert!(result.passed);
    let salt = store::salt_in(dir.path()).unwrap();
    let head = verifier_loop::receipt::read_receipt_head(dir.path(), &goal_id);
    let hash = consensus::compute_hash(
        &salt,
        &goal_id,
        "sig",
        round,
        &result.matching_verdicts,
        "2026-07-12T00:00:00Z",
        &head,
    );
    consensus::write_completion(dir.path(), &goal_id, &result, round, &hash, "2026-07-12T00:00:00Z")
        .unwrap();

    let st = round_recover::status(dir.path(), &goal_id, &cfg()).unwrap();
    assert_eq!(st.state, round_recover::GoalState::ConsensusPass);
    assert_eq!(st.needs, round_recover::GoalNeeds::Done);
}

#[test]
fn status_needs_recover_state_in_progress_with_a_null_slot() {
    // LD7: ≥1 null slot + no completion => needs="recover", state="in_progress".
    let (dir, goal_id) = fresh_goal();
    let round = 1u32;
    signed_approve(dir.path(), &goal_id, round, "v1");
    seed_null_slot(dir.path(), &goal_id, round, "v2");

    let st = round_recover::status(dir.path(), &goal_id, &cfg()).unwrap();
    assert_eq!(st.needs, round_recover::GoalNeeds::Recover);
    assert_eq!(st.state, round_recover::GoalState::InProgress);
    // slots surfaced
    let ids: Vec<&str> = st.slots.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"v1"));
    assert!(ids.contains(&"v2"));
}

#[test]
fn status_needs_resume_state_consensus_fail_when_all_non_null_below_n() {
    // LD7: every slot non-null, below n, no completion => needs="resume", state="consensus_fail".
    let (dir, goal_id) = fresh_goal();
    let round = 1u32;
    signed_approve(dir.path(), &goal_id, round, "v1");
    signed_reject(dir.path(), &goal_id, round, "v2", "missing tests");

    let st = round_recover::status(dir.path(), &goal_id, &cfg()).unwrap();
    assert_eq!(st.needs, round_recover::GoalNeeds::Resume);
    assert_eq!(st.state, round_recover::GoalState::ConsensusFail);
}

#[test]
fn status_state_new_before_slots_exist() {
    // A fresh goal whose round-1 slots were never pre-created => state="new".
    let (dir, goal_id) = fresh_goal();
    let st = round_recover::status(dir.path(), &goal_id, &cfg()).unwrap();
    assert_eq!(st.state, round_recover::GoalState::New);
}

// ===========================================================================
// RECOVER (LD8 / LD10 / LD11 / LD3)
// ===========================================================================

/// A short timeout so the dead-null test does not hang the suite.
const RECOVER_TEST_TIMEOUT: Duration = Duration::from_millis(500);

#[test]
fn recover_harvests_verdict_that_becomes_non_null_mid_poll() {
    // LD8: a null slot whose (still-running orphan) verifier writes a signed APPROVE
    // mid-poll => recover observes it, re-evaluates, and writes completion.json.
    let (dir, goal_id) = fresh_goal();
    let round = 1u32;
    signed_approve(dir.path(), &goal_id, round, "v1");
    // v2 starts null (orphan still running). We pre-mint its key so the background
    // writer can sign under the pinned pubkey exactly like a real verifier.
    let sk_v2 = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v2", round).unwrap();
    seed_null_slot(dir.path(), &goal_id, round, "v2");

    // Simulate the orphan writing its signed APPROVE shortly after recover starts.
    let root = dir.path().to_path_buf();
    let gid = goal_id.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        let _ = verdict::register_signed_approve(&root, &gid, "v2", round, None, &sk_v2);
    });

    let outcome = round_recover::recover(
        dir.path(),
        &goal_id,
        &cfg(),
        Duration::from_secs(5),
    )
    .unwrap();

    match outcome {
        round_recover::RecoverOutcome::ConsensusPassed(_) => {
            // completion.json must now exist.
            let comp = goal::goal_dir(dir.path(), &goal_id).join(consensus::COMPLETION_FILE);
            assert!(comp.exists(), "completion.json must be written on a passed recover");
        }
        other => panic!("expected ConsensusPassed, got {other:?}"),
    }
}

#[test]
fn recover_dead_null_slot_times_out_with_resume_guidance() {
    // LD8/LD11: a null slot whose orphan has died => after the timeout, recover returns
    // StillNullAfter referencing RESUME, and NO completion.json is written.
    let (dir, goal_id) = fresh_goal();
    let round = 1u32;
    signed_approve(dir.path(), &goal_id, round, "v1");
    seed_null_slot(dir.path(), &goal_id, round, "v2");

    let outcome = round_recover::recover(dir.path(), &goal_id, &cfg(), RECOVER_TEST_TIMEOUT).unwrap();
    match outcome {
        round_recover::RecoverOutcome::StillNullAfter { guidance, .. } => {
            assert!(
                guidance.to_lowercase().contains("resume"),
                "guidance must point to RESUME: {guidance}"
            );
        }
        other => panic!("expected StillNullAfter, got {other:?}"),
    }
    let comp = goal::goal_dir(dir.path(), &goal_id).join(consensus::COMPLETION_FILE);
    assert!(!comp.exists(), "no completion.json on a dead-null recover");
}

#[test]
fn recover_round_decided_no_consensus_does_not_wait_full_timeout() {
    // LD8: every slot non-null but below n => decided-failed; return promptly without
    // waiting the full timeout.
    let (dir, goal_id) = fresh_goal();
    let round = 1u32;
    signed_approve(dir.path(), &goal_id, round, "v1");
    signed_reject(dir.path(), &goal_id, round, "v2", "missing tests");

    let start = std::time::Instant::now();
    let outcome = round_recover::recover(dir.path(), &goal_id, &cfg(), Duration::from_secs(10)).unwrap();
    let elapsed = start.elapsed();
    match outcome {
        round_recover::RecoverOutcome::RoundDecidedNoConsensus => {}
        other => panic!("expected RoundDecidedNoConsensus, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(5),
        "decided-failed must NOT wait the full timeout (elapsed {elapsed:?})"
    );
    let comp = goal::goal_dir(dir.path(), &goal_id).join(consensus::COMPLETION_FILE);
    assert!(!comp.exists(), "no completion.json on a decided-failed recover");
}

#[test]
fn recover_does_not_spawn_or_re_render() {
    // LD10/LD11 structural guarantee: recover's signature takes NO prompt and NO
    // snapshot — it cannot re-render or re-capture. (This is a compile-time contract;
    // the test exists to pin the public API shape so a later change cannot quietly add
    // a prompt/snapshot param.) We exercise the happy path and assert no initial-prompt
    // file was written for the current round by recover.
    let (dir, goal_id) = fresh_goal();
    let round = 1u32;
    signed_approve(dir.path(), &goal_id, round, "v1");
    let sk_v2 = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v2", round).unwrap();
    seed_null_slot(dir.path(), &goal_id, round, "v2");

    let root = dir.path().to_path_buf();
    let gid = goal_id.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        let _ = verdict::register_signed_approve(&root, &gid, "v2", round, None, &sk_v2);
    });

    let _ = round_recover::recover(dir.path(), &goal_id, &cfg(), Duration::from_secs(5)).unwrap();

    // recover must not have written any prompt file for the round.
    let round_dir = goal::goal_dir(dir.path(), &goal_id)
        .join(goal::ROUNDS_DIR)
        .join(round.to_string());
    let wrote_prompt = any_file_named(&round_dir, "initial-prompt.txt");
    assert!(
        !wrote_prompt,
        "recover must NOT write initial-prompt.txt (frozen-snapshot invariant LD10)"
    );
}

/// Recursively check whether any file named `name` exists under `dir` (no extra deps).
fn any_file_named(dir: &Path, name: &str) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if any_file_named(&path, name) {
                return true;
            }
        } else if path.file_name().and_then(|s| s.to_str()) == Some(name) {
            return true;
        }
    }
    false
}
