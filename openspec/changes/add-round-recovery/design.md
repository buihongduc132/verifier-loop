# Design — add-round-recovery (SHAPE-1)

> Source of truth for the locked decisions:
> [`flow/findings/round-recovery/2026-07-12-locked-decisions.yaml`](../../../flow/findings/round-recovery/2026-07-12-locked-decisions.yaml)
> (LD1 superseded by LD8; LD2 moot; LD3–LD11 live). This design **implements** those
> decisions; it does not re-decide them.

## 1. Problem

`jewilo` blocks at a gather barrier while `m` verifier backends run. If jewilo is killed
mid-round, the children are orphaned. Today:

- There is no way to harvest the in-flight verdicts → the only path is `RESUME N+1`,
  which discards them and mints fresh keys.
- No machine-readable state → an outer agent cannot tell "round N needs RECOVER" from
  "consensus-fail, needs RESUME N+1" from "already done."
- No mutual exclusion → a concurrent `NEW`/`RESUME`/`RECOVER` corrupts the session
  files and double-mints pubkeys (`AlreadyPinned`).

## 2. Selected path: SHAPE-1 (LD8) — wait-only recovery

SHAPE-1 = **never kill the signer, never re-render, never fresh-spawn inside RECOVER.**

```
                  jewilo killed mid-round N
                            │
                            ▼
            ┌───────────────────────────────────┐
            │  orphan verifiers still running   │
            │  (each holds the only valid       │
            │   signing key in its env)         │
            └───────────────┬───────────────────┘
                            │  user / agent runs:
                            │  jewilo RECOVER <goalId>
                            ▼
            ┌───────────────────────────────────┐
            │  RECOVER (SHAPE-1):               │
            │   • flock the goal dir (LD5)      │
            │   • read each slot's verdict.json │
            │   • POLL up to verifierTimeoutSec │
            │     for verdicts to appear        │
            │   • re-run consensus::evaluate    │
            │     (UNCHANGED — verdict file is  │
            │      the resumption contract)     │
            │   • on pass → write completion    │
            │   • on null remaining → guidance: │
            │     "run RESUME N+1"              │
            └───────────────────────────────────┘
```

**Why not shape-2 (kill orphan → resume-by-SID → nudge)?** It is unimplementable against
landed tamper-hardening (LD1 INVALIDATED by G5-1): the per-slot signing secret is minted
by `mint_and_pin_pubkey`, returned to the orchestrator, injected into the child env
**only**, and never persisted. Killing the orphan destroys the only valid signer; a
resumed `pi --session` carries no `VERIFIER_LOOP_VERIFIER_SECRET`; a fresh spawn hits
`AlreadyPinned`. A pinned-but-null slot with no LIVE signer can never produce a countable
verdict. Persisting secrets (path-b) would widen forging from "write+env-read" to
"store-read during open window" — a threat-model regression (LD8 tradeoff). SHAPE-1's only
cost is honesty: dead-but-null slots must advance to `RESUME N+1`.

## 3. Decisions implemented (LD3–LD11)

| LD | Decision | Where |
|----|----------|-------|
| LD3 | `RECOVER <goalId>` separate command; symmetric precondition warnings on RECOVER ("use RESUME") and RESUME ("use RECOVER first") | `round_recover` + cli |
| LD4 | vocab: `recover` (cross-process) vs `compaction_recover` (within-round); module prefix `round_recover` | module name, docs |
| LD5 | exclusive `flock` on `goals/<goalId>/.lock` for NEW/RESUME/RECOVER full duration; concurrent → exit non-zero "goal busy" | `round_recover::GoalLock` + goal layer |
| LD6 | kill-behavior axes (signal-source, supervisor) — **documentation only** (no kill in SHAPE-1) | design.md §5, AGENTS note |
| LD7 | `STATUS <goalId>` JSON subcommand: `{round, state, needs, slots:[{id,verdict,hasLiveOrphan?}]}` | `round_recover::status` |
| LD8 | SHAPE-1: poll live orphans; dead-null → RESUME N+1; no secret persistence | `round_recover::recover` |
| LD9 | no PID file, no kill step | (absence — nothing to implement) |
| LD10 | RECOVER never re-renders / re-captures | `recover` reads no prompt/snapshot |
| LD11 | no snapshot TTL — RECOVER never fresh-spawns | (absence) |

## 4. Component design

### 4.1 `GoalLock` (LD5)

An RAII guard wrapping an exclusive `flock` on `goals/<goalId>/.lock`.

```rust
pub struct GoalLock { /* file handle held for the guard's lifetime */ }
impl GoalLock {
    /// Acquire an exclusive lock; blocks until held. On unrecoverable lock
    /// contention beyond `GoalBusy`, returns Err so the caller exits non-zero.
    pub fn acquire_exclusive(root, goal_id) -> Result<GoalLock, RoundRecoverError>;
}
impl Drop for GoalLock { /* unlock + close */ }
```

- Built on `fs4::fs_std::FileExt::lock_exclusive` (same crate the receipt log uses →
  one locking idiom in the repo).
- Holds a `File` handle; `Drop` calls `unlock` (best-effort) then closes. This guarantees
  the lock is released even on early-return / panic, so a crashed `RECOVER` does not
  poison the goal.
- `NEW`, `RESUME`, and `RECOVER` each acquire this guard at entry and hold it for the
  whole operation. `STATUS` is read-only and does **not** take the lock (a status probe
  must never block on a long-running round) — it reads the on-disk state atomically per
  file (each verdict.json/meta.json read is independent).

### 4.2 `recover` (LD3/LD8/LD10/LD11)

```rust
pub fn recover(root, goal_id, config, timeout) -> Result<RecoverOutcome, RoundRecoverError>;
```

Algorithm (SHAPE-1, wait-only):

1. `GoalLock::acquire_exclusive` (LD5).
2. Read the current round from `state.json`.
3. Loop until `verifierTimeoutSec` (the same per-verifier budget, reused):
   a. Read every slot's `verdict.json` for the current round.
   b. Run `consensus::evaluate` over them.
   c. If `evaluate.passed` → write `completion.json`, return `RecoverOutcome::ConsensusPassed(hash)`.
   d. If every slot is non-null (each is APPROVE/REJECT, none null) → the round is
      decided but failed; return `RecoverOutcome::RoundDecidedNoConsensus`.
   e. Else (≥1 null slot, not yet passed) → sleep `RECOVER_POLL_INTERVAL` (default 2s),
      re-poll.
4. On timeout: return `RecoverOutcome::StillNullAfter { null_slots, guidance }` where
   `guidance = "run RESUME N+1 for fresh slots and fresh keys"`.

**Invariants preserved (LD8/LD10/LD11):**
- Never calls `spawn_round` / `spawn_resume` → no fresh keys, no re-render, no re-capture.
- Never reads the prompt or snapshot.
- Never kills a process (LD9).
- Reuses `consensus::evaluate` unchanged → the verdict file is the resumption contract;
  the completion-hash formula and inputs are untouched.
- A NULL verdict is never promoted (fail-closed D9 preserved).

### 4.3 `status` (LD7)

```rust
pub fn status(root, goal_id, config) -> Result<GoalStatus, RoundRecoverError>;
```

Reads (no lock) and emits:

```jsonc
{
  "goalId": "...",
  "round": 2,
  "state": "in_progress",      // new | in_progress | consensus_pass | consensus_fail
  "needs": "recover",          // recover | resume | done
  "slots": [
    { "id": "v1", "verdict": "APPROVE" },
    { "id": "v2", "verdict": null }
  ]
}
```

`needs` derivation:
- `completion.json` exists → `"done"`.
- ≥1 null slot and no completion → `"recover"` (a live orphan may still emit).
- every slot non-null, no completion → `"resume"` (round decided, failed).
- (state `new` only before the first spawn of the round.)

`hasLiveOrphan`: SHAPE-1 deliberately does **not** track PIDs (LD9), so liveness is
inferred only indirectly (a slot with `meta.json` present but a null verdict *may* have a
live orphan). The machine contract therefore keys on `verdict` (the durable signal), and
`needs` is derived from null-vs-decided. This keeps `STATUS` PID-free and CVE-free.

### 4.4 Symmetric precondition warnings (LD3)

- `RECOVER` on a round where `completion.json` already exists → warn "round already
  reached consensus; use `RESUME N+1` for a new round" and exit 0 (nothing to recover).
- `RESUME` on a round that has null verdicts and no completion → warn "round N has null
  verdicts; consider `RECOVER <goalId>` first to harvest in-flight verdicts" (but still
  proceed — RESUME is the user's explicit escape hatch).

## 5. Kill-behavior axes (LD6 — documentation, no code)

The kill-behavior table from the exploration gains two axes (no implementation — SHAPE-1
has no kill step):

| jewilo death | signal source | supervisor | child outcome |
|---|---|---|---|
| TTY Ctrl-C | TTY-generated (pgrp broadcast) | raw process | children reaped with jewilo |
| `kill -INT <pid>` | `kill()`-delivered (single pid) | raw process | children **orphaned** |
| `kill -TERM` under systemd | (any) | cgroup-scoped | all descendants reaped |
| container PID-1 SIGTERM | (any) | non-forwarding PID-1 | children **survive** even Ctrl-C |

`SIGPIPE`-on-pipe-close is a third orphan-death vector. RECOVER tolerates all of these:
it only observes whether `verdict.json` becomes non-null, never whether a process is alive.

## 6. Threat model (no regression)

SHAPE-1 changes **nothing** about the security surface compared to the landed
tamper-hardening:

- The per-slot Ed25519 signing secret is still minted by `mint_and_pin_pubkey` at spawn,
  injected into the child env **only**, and never persisted. RECOVER never mints, never
  reads, never needs it.
- A verdict counts only if its signature verifies against the slot's pinned pubkey
  (`consensus::evaluate` → `verify_record`, unchanged). So an orphan that writes a valid
  signed verdict after jewilo dies is legitimate (it holds the secret); a forged verdict
  without the secret fails the signature gate as before.
- The frozen-snapshot-per-round invariant holds (LD10): RECOVER never re-renders, so the
  snapshot a verifier saw at spawn is the snapshot its verdict is bound to.
- No `PID` file (LD9) → no PID-reuse / wrapper-vs-grandchild / PID-file-CVE surface.

The only new durable artifact is the `.lock` file (empty, advisory). It contains no
secrets and is created idempotently; a stale `.lock` is harmless (advisory locks release on
process exit / crash).

## 7. Concurrency & fail-closed

- **`flock` mutual exclusion (LD5)** prevents: concurrent `RECOVER`/`RESUME` session-file
  corruption; double-mint `AlreadyPinned` races; double-spawn of the same round. A second
  invocation exits non-zero with `"goal <id> busy; another NEW/RESUME/RECOVER in progress"`.
- **Fail-closed (D9) preserved**: RECOVER reuses `consensus::evaluate`, which never counts
  a NULL or unsigned verdict. A null slot after timeout → no hash, no completion; the user
  is told to `RESUME N+1`.
- **Receipt log**: RECOVER does not append to the receipt log itself (verdicts are written
  by the verifier processes via `verifier-verdict`, which already chains). RECOVER only
  reads the head for the completion hash, exactly as `run_round` does today.

## 8. Non-goals (out of scope)

- Secret persistence (LD8 rejected path-b).
- PID file / orphan liveness detection / kill primitive (LD9 — no kill step).
- Snapshot TTL / staleness (LD11 — no fresh-spawn in RECOVER).
- Re-rendering the prompt or re-capturing the snapshot in RECOVER (LD10).
- OT1 audit subcommand, OT2 per-verifier maxTurn refresh, OT6 fan-out scouts (existing
  non-goals from `add-verifier-loop-cli/design.md`).
