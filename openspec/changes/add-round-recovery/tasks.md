# Implementation Tasks — add-round-recovery (SHAPE-1)

**TDD discipline (per AGENTS.md):** every group = RED test first (fresh author) → GREEN
impl (different fresh author) → coverage gate `cargo llvm-cov --fail-under-lines 80`.
Never the same author for RED + GREEN of a group. Implements locked decisions LD3–LD11
from `flow/findings/round-recovery/2026-07-12-locked-decisions.yaml`.

## 1. GoalLock — exclusive goal mutual exclusion (LD5)

- [x] 1.1 RED: `tests/round_recover.rs` — `acquire_exclusive` then a second
      `acquire_exclusive` in the SAME process (different file handle) fails with
      `GoalBusy` / contention. Also: a `GoalLock` `Drop` releases so a second acquire
      after drop succeeds.
- [x] 1.2 RED: `tests/cli_e2e.rs` (or round_recover integration) — a second concurrent
      `jewilo RESUME` subprocess while the first holds the lock exits non-zero with
      "goal busy" on stderr.
- [x] 1.3 GREEN: add `src/round_recover/mod.rs` with `GoalLock` RAII guard over
      `fs4::fs_std::FileExt::lock_exclusive` on `goals/<goalId>/.lock`. Add
      `RoundRecoverError::GoalBusy`. Register `pub mod round_recover;` in `src/lib.rs`.
- [x] 1.4 Coverage gate on touched files.

## 2. STATUS — machine-readable goal state (LD7)

- [x] 2.1 RED: `tests/round_recover.rs` — `round_recover::status` returns
      `state="consensus_pass"`, `needs="done"` when `completion.json` exists.
- [x] 2.2 RED: `status` returns `needs="recover"`, `state="in_progress"` when ≥1 null
      slot and no completion.
- [x] 2.3 RED: `status` returns `needs="resume"`, `state="consensus_fail"` when every
      slot non-null and below `n`, no completion.
- [x] 2.4 RED: `status` returns `state="new"` before the round's slots exist.
- [x] 2.5 GREEN: implement `status()` reading `state.json`, per-slot `verdict.json`, and
      `completion.json`; derive `state`/`needs` per design §4.3. No lock taken.
- [x] 2.6 Coverage gate.

## 3. RECOVER — wait-only round recovery (LD3/LD8/LD10/LD11)

- [x] 3.1 RED: `recover` returns `ConsensusPassed(hash)` and writes `completion.json`
      when a null slot's `verdict.json` becomes a signed APPROVE mid-poll (simulate by
      writing the verdict file from a background task).
- [x] 3.2 RED: `recover` returns `StillNullAfter` with `RESUME` guidance when the timeout
      elapses with a slot still null; asserts NO `completion.json` written.
- [x] 3.3 RED: `recover` returns `RoundDecidedNoConsensus` promptly (without waiting the
      full timeout) when every slot is non-null but below `n`.
- [x] 3.4 RED: `recover` on a goal whose `completion.json` already exists is a no-op
      warning that exits 0 (LD3 symmetric warning) — exercised at the CLI layer.
- [x] 3.5 GREEN: implement `recover()` per design §4.2 — flock, loop/poll, reuse
      `consensus::evaluate` + `compute_hash` + `write_completion` unchanged. Poll interval
      `RECOVER_POLL_INTERVAL_SECS` (default 2). Timeout = `config.verifier_timeout_sec`.
- [x] 3.6 GREEN: assert recover does NOT call spawn, does NOT read the prompt/snapshot
      (structural: the function signature takes no prompt/snapshot).
- [x] 3.7 Coverage gate.

## 4. CLI wiring (LD3)

- [x] 4.1 RED: `jewilo STATUS <goalId>` prints a single JSON object with the documented
      fields; exits 0.
- [x] 4.2 RED: `jewilo RECOVER <goalId>` on a harvestable round prints the short hash and
      exits 0; on a dead-null round exits non-zero with RESUME guidance.
- [x] 4.3 RED: `jewilo RECOVER <goalId>` on a complete round warns + exits 0 (LD3).
- [x] 4.4 RED: `jewilo RESUME <goalId>` on a round with a null slot warns about RECOVER
      (LD3) but proceeds.
- [x] 4.5 RED: a second concurrent `jewilo RESUME` exits non-zero "goal busy" (LD5).
- [x] 4.6 GREEN: add `Status { goal_id }` and `Recover { goal_id }` variants to
      `VerifierLoopCmd`; add `run_status`/`run_recover` in `src/bin/verifier_loop.rs`;
      take the `GoalLock` in `run_new`/`run_resume`/`run_recover`.
- [x] 4.7 Coverage gate + full `cargo test` green.

## 5. Docs + e2e

- [x] 5.1 Update module map + design pointers in `AGENTS.md` for `round_recover` /
      RECOVER / STATUS.
- [x] 5.2 E2E smoke: `jewilo-dev RECOVER` after a killed round harvests the verdict and
      reaches consensus (manual run in the worktree, captured in the PR body).
