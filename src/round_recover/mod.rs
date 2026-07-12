//! Cross-process round recovery (SHAPE-1) — the `RECOVER` primitive + `STATUS` probe +
//! exclusive goal mutual exclusion (`GoalLock`).
//!
//! Implements locked decisions LD3–LD11 from
//! `flow/findings/round-recovery/2026-07-12-locked-decisions.yaml`.
//!
//! ## Vocabulary (LD4)
//! `recover` = **cross-process** round recovery (this module; a verifier backend kept
//! running after jewilo was killed, and we want to harvest its verdict). This is distinct
//! from `compaction_recover` = **within-round same-process** recovery (compaction fired
//! mid-analysis; the spawn orchestrator resumes the same session). The undefined term
//! "process-recovery" is dropped.
//!
//! ## SHAPE-1 (LD8)
//! `recover()` is **wait-only**: it never spawns, never kills, never re-renders the
//! verifier prompt, never re-captures the working-tree snapshot, and never persists or
//! reads the per-slot signing secret. It polls each slot's `verdict.json` for the current
//! round and re-evaluates consensus with the existing `consensus::evaluate` (the verdict
//! file is the resumption contract — LD8). A dead-but-null slot (orphan died / never
//! finished) degrades honestly to user-visible `RESUME N+1` guidance; no key is minted,
//! no verdict is fabricated, no `completion.json` is written (fail-closed D9 preserved).
//!
//! Why not shape-2 (kill orphan → resume-by-SID → nudge)? It is unimplementable against
//! landed tamper-hardening: the per-slot signing secret lives only in the original child
//! env, so killing the orphan destroys the only valid signer and a resumed session cannot
//! produce a countable verdict. Persisting secrets would regress the threat model. See
//! `openspec/changes/add-round-recovery/design.md`.

use std::fs::OpenOptions;
use std::path::Path;
use std::time::{Duration, Instant};

use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};

use crate::consensus;
use crate::goal;
use crate::store;
use crate::verdict::{self, VerdictRecord, VerdictStatus};

/// Default poll interval for `recover()` while waiting for in-flight verdicts (LD8).
/// Tuned to be responsive without burning a CPU core busy-looping.
pub const RECOVER_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// On-disk advisory lock filename (empty file; advisory `flock`). Sibling to `goal.json`.
pub const GOAL_LOCK_FILE: &str = ".lock";

// ---------------------------------------------------------------------------
// GoalLock (LD5)
// ---------------------------------------------------------------------------

/// RAII guard wrapping an **exclusive** advisory lock on `goals/<goalId>/.lock`.
///
/// `NEW`, `RESUME`, and `RECOVER` each acquire this for their full duration so concurrent
/// state-mutating operations on the same goal cannot race (double-mint `AlreadyPinned`,
/// session-file corruption, double-spawn). The lock is advisory (`flock` on Unix): it
/// relies on every writer cooperating via this type, and it is released by the OS on
/// process exit / crash — so a crashed command never poisons the goal.
///
/// `STATUS` is read-only and deliberately does NOT take this lock (a status probe must
/// never block on a long-running round).
pub struct GoalLock {
    _file: std::fs::File,
}

impl std::fmt::Debug for GoalLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoalLock").finish_non_exhaustive()
    }
}

impl GoalLock {
    /// Acquire an exclusive lock on the goal's `.lock` file, **fail-fast**.
    ///
    /// Per LD5, a concurrent state-mutating operation MUST exit non-zero with a "goal
    /// busy" message rather than blocking on a long-running round. So this uses a
    /// non-blocking `try_lock_exclusive`: if the lock is already held (by another
    /// process, or another handle in this process), it returns `Err(GoalBusy)`
    /// immediately.
    ///
    /// Creates the goal directory + lock file if needed (idempotent). The handle is held
    /// for the guard's lifetime; `Drop` unlocks + closes.
    pub fn acquire_exclusive(root: &Path, goal_id: &str) -> Result<GoalLock, RoundRecoverError> {
        let gdir = goal::goal_dir(root, goal_id);
        // Fail closed if the goal does not exist — `create_dir_all` here would otherwise
        // mint a phantom goal dir (holding only `.lock`) for a bad/unknown goalId. RESUME
        // and RECOVER must only ever lock goals that NEW already created.
        if !gdir.exists() {
            return Err(RoundRecoverError::GoalNotFound);
        }
        let lock_path = gdir.join(GOAL_LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;
        // Only a WouldBlock (lock already held by a cooperating writer) is `GoalBusy`.
        // Any other error (permission denied, lock-FS not supported on NFS, etc.) is a
        // real I/O failure and MUST surface as such — mapping it to `GoalBusy` would tell
        // the operator "another op is in progress" when the real cause is systemic.
        file.try_lock_exclusive().map_err(|e| {
            if e.kind() == std::io::ErrorKind::WouldBlock {
                RoundRecoverError::GoalBusy
            } else {
                RoundRecoverError::Io(e)
            }
        })?;
        Ok(GoalLock { _file: file })
    }
}

impl Drop for GoalLock {
    fn drop(&mut self) {
        // Best-effort unlock; the OS releases on close/exit regardless.
        let _ = self._file.unlock();
    }
}

// ---------------------------------------------------------------------------
// STATUS (LD7)
// ---------------------------------------------------------------------------

/// Lifecycle state of the goal's current round (LD7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    /// Round directory / slots do not yet exist (before the first spawn of the round).
    New,
    /// At least one slot is null and no completion (a live orphan may still emit).
    InProgress,
    /// `completion.json` exists for the current round.
    ConsensusPass,
    /// Every slot non-null but below `n`; no completion.
    ConsensusFail,
}

/// What an outer agent should do next for the goal (LD7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalNeeds {
    /// `completion.json` exists — the goal round is complete.
    Done,
    /// ≥1 null slot, no completion — `RECOVER` may harvest an in-flight verdict.
    Recover,
    /// Every slot non-null, below `n`, no completion — `RESUME N+1` for a fresh round.
    Resume,
}

/// One verifier slot's contribution to the status report (LD7).
///
/// Every slot object ALWAYS carries a `verdict` field — a null/missing slot emits
/// `"verdict": null` (the goal-status spec scenario "STATUS shape" requires each slot to
/// have both an `id` and a `verdict` field, null included).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotStatus {
    /// Verifier id (`v1`, `v2`, …).
    pub id: String,
    /// The slot's verdict status. `VerdictStatus::Null` serializes to JSON `null`, so a
    /// null slot emits `"verdict": null` rather than omitting the key (fail-closed).
    pub verdict: VerdictStatus,
}

/// The machine-readable status object emitted by `STATUS <goalId>` (LD7).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalStatus {
    pub goal_id: String,
    pub round: u32,
    pub state: GoalState,
    pub needs: GoalNeeds,
    pub slots: Vec<SlotStatus>,
}

/// Read-only goal status probe (LD7). Does NOT take the goal lock — a status check must
/// never block on a long-running round. Each file read is independent and atomic.
pub fn status(
    root: &Path,
    goal_id: &str,
    config: &store::Config,
) -> Result<GoalStatus, RoundRecoverError> {
    let round = goal::current_round(root, goal_id)?;
    let round_dir = goal::goal_dir(root, goal_id)
        .join(goal::ROUNDS_DIR)
        .join(round.to_string());

    // Slots exist iff the round directory is populated. Absent => "new".
    let slots_populated = round_dir.is_dir() && any_verifier_dir(&round_dir);

    let mut slots: Vec<SlotStatus> = Vec::new();
    let mut any_null = false;
    let mut raw_approve_count: u32 = 0;
    if slots_populated {
        for i in 0..config.m as usize {
            let vid = verifier_id(i);
            let rec = verdict::read_verdict(root, goal_id, &vid, round).unwrap_or(VerdictRecord {
                status: VerdictStatus::Null,
                notes: None,
                registered_at: None,
                signature: None,
                pubkey_id: None,
            });
            if rec.status == VerdictStatus::Null {
                any_null = true;
                slots.push(SlotStatus {
                    id: vid,
                    verdict: VerdictStatus::Null,
                });
            } else {
                if rec.status == VerdictStatus::Approve {
                    raw_approve_count = raw_approve_count.saturating_add(1);
                }
                slots.push(SlotStatus {
                    id: vid,
                    verdict: rec.status,
                });
            }
        }
    }

    let completion_exists = goal::goal_dir(root, goal_id)
        .join(consensus::COMPLETION_FILE)
        .exists();

    // `needs` derivation (LD7). The raw APPROVE count is a heuristic — STATUS is a
    // read-only probe and does NOT run the signature gate (that is RECOVER's job via
    // consensus::evaluate). So a state reported as potentially-passable here is still
    // authoritatively decided by RECOVER. The key correctness fix: when every slot is
    // non-null AND the raw APPROVE count already reaches `n`, the round WOULD pass —
    // RECOVER can finish it (write completion.json) — so `needs` is `recover`, NOT
    // `resume`. Only a genuinely-decided-failed round (below n) needs `resume`.
    let (state, needs) = if completion_exists {
        (GoalState::ConsensusPass, GoalNeeds::Done)
    } else if !slots_populated {
        (GoalState::New, GoalNeeds::Recover)
    } else if any_null {
        (GoalState::InProgress, GoalNeeds::Recover)
    } else if raw_approve_count >= config.n {
        // All slots decided, APPROVE count reaches n, but no completion yet (the round
        // was interrupted before the gather barrier wrote completion.json). RECOVER can
        // finish it — so this is recoverable, not a failed round.
        (GoalState::InProgress, GoalNeeds::Recover)
    } else {
        (GoalState::ConsensusFail, GoalNeeds::Resume)
    };

    Ok(GoalStatus {
        goal_id: goal_id.to_string(),
        round,
        state,
        needs,
        slots,
    })
}

/// True iff `round_dir` contains any `vN` verifier subdirectory.
fn any_verifier_dir(round_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(round_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with('v') && name[1..].chars().all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// RECOVER (LD8 / LD10 / LD11 / LD3)
// ---------------------------------------------------------------------------

/// The outcome of a `RECOVER` run.
#[derive(Debug, Clone)]
pub enum RecoverOutcome {
    /// Consensus was reached during recovery; the completion hash was written.
    /// Carries the short `mmddyy-XXXXXXXX` hash.
    ConsensusPassed(String),
    /// Every slot is non-null but below `n` — the round is decided but failed.
    /// No completion written; the user should `RESUME N+1`.
    RoundDecidedNoConsensus,
    /// The timeout elapsed with at least one slot still null. Carries the null slot ids
    /// and user-visible guidance (points to `RESUME N+1`).
    StillNullAfter {
        null_slots: Vec<String>,
        guidance: String,
    },
}

/// Wait-only round recovery (SHAPE-1, LD8/LD10/LD11).
///
/// Acquires the exclusive goal lock (LD5), then polls each slot's `verdict.json` for the
/// current round up to `timeout`, re-evaluating consensus after each poll. On a pass it
/// writes `completion.json` exactly as a normal round does, reusing `consensus::evaluate`,
/// `compute_hash`, and `write_completion` unchanged. Never spawns, kills, re-renders, or
/// re-captures (the signature takes neither a prompt nor a snapshot — LD10/LD11). A null
/// slot after timeout degrades to `StillNullAfter` with `RESUME N+1` guidance (no key
/// minted, no verdict fabricated, fail-closed D9).
pub fn recover(
    root: &Path,
    goal_id: &str,
    config: &store::Config,
    timeout: Duration,
) -> Result<RecoverOutcome, RoundRecoverError> {
    recover_with_poll(root, goal_id, config, timeout, RECOVER_POLL_INTERVAL)
}

/// Same as [`recover`] but with an explicit poll interval (used by tests to poll fast).
pub fn recover_with_poll(
    root: &Path,
    goal_id: &str,
    config: &store::Config,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<RecoverOutcome, RoundRecoverError> {
    // LD5: hold the exclusive lock for the full duration (polling included).
    let _lock = GoalLock::acquire_exclusive(root, goal_id)?;

    let round = goal::current_round(root, goal_id)?;
    let deadline = Instant::now() + timeout;

    loop {
        let (verdicts, null_slots) = read_round_verdicts(root, goal_id, round, config);
        let result = consensus::evaluate(root, goal_id, round, &verdicts, config.n, config.m);

        if result.passed {
            let hash = finish_pass(root, goal_id, round, &result)?;
            return Ok(RecoverOutcome::ConsensusPassed(
                hash.short_hash().to_string(),
            ));
        }

        if null_slots.is_empty() {
            // Every slot non-null but below n: the round is decided but failed. Do not
            // wait the full timeout — there is nothing left to harvest.
            return Ok(RecoverOutcome::RoundDecidedNoConsensus);
        }

        let now = Instant::now();
        if now >= deadline {
            return Ok(RecoverOutcome::StillNullAfter {
                null_slots,
                guidance: format!(
                    "round {round} still has null verdict slots after the recovery timeout; \
                     run `jewilo RESUME {goal_id}` for fresh slots and fresh keys"
                ),
            });
        }

        // Clamp the sleep to the remaining time before the deadline so we never overshoot
        // the configured timeout by up to one poll_interval.
        let remaining = deadline.saturating_duration_since(now);
        std::thread::sleep(poll_interval.min(remaining));
    }
}

/// Read every verifier slot's verdict for a round. Returns the `(verdicts, null_slots)`
/// pair. Missing/unreadable slots are treated as null (fail-closed) and named in
/// `null_slots`.
fn read_round_verdicts(
    root: &Path,
    goal_id: &str,
    round: u32,
    config: &store::Config,
) -> (Vec<(String, VerdictRecord)>, Vec<String>) {
    let mut verdicts = Vec::with_capacity(config.m as usize);
    let mut null_slots = Vec::new();
    for i in 0..config.m as usize {
        let vid = verifier_id(i);
        let rec = verdict::read_verdict(root, goal_id, &vid, round).unwrap_or(VerdictRecord {
            status: VerdictStatus::Null,
            notes: None,
            registered_at: None,
            signature: None,
            pubkey_id: None,
        });
        if rec.status == VerdictStatus::Null {
            null_slots.push(vid.clone());
        }
        verdicts.push((vid, rec));
    }
    (verdicts, null_slots)
}

/// Compute + write the completion record for a passed round. Mirrors the bin's
/// `run_round` success path so RECOVER's hash is byte-identical to a normal round's
/// (the verdict file is the resumption contract — LD8).
fn finish_pass(
    root: &Path,
    goal_id: &str,
    round: u32,
    result: &consensus::ConsensusResult,
) -> Result<consensus::CompletionHash, RoundRecoverError> {
    use chrono::Utc;
    let salt = store::salt_in(root)?;
    let goal_root = goal::goal_dir(root, goal_id);
    let sig_record: goal::SignatureRecord = serde_json::from_str(&std::fs::read_to_string(
        goal_root.join(goal::SIGNATURE_FILE),
    )?)?;
    let matched_at = Utc::now().to_rfc3339();
    let receipt_head = crate::receipt::read_receipt_head(root, goal_id);
    let hash = consensus::compute_hash(
        &salt,
        goal_id,
        &sig_record.signature,
        round,
        &result.matching_verdicts,
        &matched_at,
        &receipt_head,
    );
    consensus::write_completion(
        root,
        goal_id,
        result,
        round,
        &hash,
        &matched_at,
        // Record the goal's trace id on completion.json as metadata (NOT a hash
        // input, design D4). Fail-open.
        crate::observe::ensure_goal_trace_id(root, goal_id)
            .ok()
            .as_deref(),
    )?;
    Ok(hash)
}

/// `v1`, `v2`, … mirroring the spawn layer's id scheme.
fn verifier_id(idx: usize) -> String {
    format!("v{}", idx + 1)
}

/// Errors raised by the round-recovery layer. All fail-closed.
#[derive(Debug, thiserror::Error)]
pub enum RoundRecoverError {
    /// LD5: another NEW/RESUME/RECOVER holds the exclusive goal lock.
    #[error("goal is busy; another NEW/RESUME/RECOVER is in progress")]
    GoalBusy,
    #[error("goal not found (store or goal directory missing)")]
    GoalNotFound,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("goal layer error: {0}")]
    Goal(#[from] goal::GoalError),
    #[error("store error: {0}")]
    Store(#[from] store::StoreError),
    #[error("consensus layer error: {0}")]
    Consensus(#[from] consensus::ConsensusError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_ids_are_one_indexed_v_prefix() {
        assert_eq!(verifier_id(0), "v1");
        assert_eq!(verifier_id(3), "v4");
    }

    #[test]
    fn goal_state_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&GoalState::ConsensusPass).unwrap(),
            r#""consensus_pass""#
        );
        assert_eq!(
            serde_json::to_string(&GoalState::InProgress).unwrap(),
            r#""in_progress""#
        );
    }

    #[test]
    fn goal_needs_serializes_snake_case() {
        let j = serde_json::to_string(&GoalNeeds::Recover).unwrap();
        assert_eq!(j, r#""recover""#);
    }
}
