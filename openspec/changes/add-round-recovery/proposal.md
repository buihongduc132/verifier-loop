# add-round-recovery

## Why

`jewilo` (the `verifier-loop` binary) is a long-running orchestrator: it spawns `m`
verifier backend processes, then blocks at a gather barrier until every child exits or
times out. When the driving agent (or a supervisor) **kills jewilo mid-round** — a TTY
`SIGINT`, a non-TTY `SIGTERM` from `kill()`, a cgroup supervisor restart, or an OOM — the
already-spawned verifier children are orphaned. Today there is **no way to harvest the
verdicts those orphans are still computing**: jewilo has no `RECOVER` command, no
machine-readable `STATUS`, and no mutual exclusion to prevent two concurrent operations
from corrupting the same goal. The only path is `RESUME N+1`, which increments the round,
re-captures the snapshot, re-renders the prompt, and mints **fresh** signing keys —
discarding any in-flight verdict the orphans are about to write.

The round-recovery exploration (`flow/findings/round-recovery/`, 2026-07-12) resolved all
10 open threads and produced 11 locked decisions (LD1 superseded by LD8; LD2 moot;
LD3–LD11 live). The least-resistance path selected is **SHAPE-1** (LD8): a wait-only
recovery primitive that never kills a live signer, never persists secrets, never
re-renders the prompt or re-captures the snapshot, and degrades honestly to `RESUME N+1`
for dead-but-null slots. This change implements SHAPE-1 plus the supporting `STATUS`
contract (LD7), `flock` mutual exclusion (LD5), and symmetric precondition warnings (LD3).

## What Changes

### New capabilities

- **`round-recovery`** — a `RECOVER <goalId>` top-level command (LD3) that, after an
  interrupted round, **waits** for already-emitted verdicts from live orphan verifier
  processes and re-evaluates consensus. It does NOT spawn, kill, re-render, or re-capture
  (LD8/LD10/LD11). Dead-but-null slots fall through to user-visible guidance: "run
  `RESUME N+1` for fresh slots and fresh keys."
- **`goal-status`** — a `STATUS <goalId>` top-level command (LD7) emitting
  machine-readable JSON describing the goal's round, state, what it `needs`
  (`recover`|`resume`|`done`), and per-slot verdict + liveness.

### Modified capabilities

- **`goal-lifecycle`** — `NEW`, `RESUME`, and `RECOVER` now take an **exclusive `flock`**
  on `store_dir/goals/<goalId>/.lock` for their full duration (LD5). A concurrent
  invocation exits non-zero with a clear "goal busy" message. `RESUME` on a round with
  null verdicts warns "use `RECOVER` first"; `RECOVER` on a complete/consensus round
  warns "use `RESUME`" (LD3 symmetric warnings).

### Vocabulary (LD4)

- `recover` = cross-process round recovery (this change, module prefix `round_recover`).
- `compaction_recover` = within-round same-process recovery (already landed, lives in the
  spawn orchestrator). The undefined term "process-recovery" is dropped.

## Impact

- **Code**: new `src/round_recover` module (RECOVER + STATUS + flock guard); `src/goal`
  (flock on the goal dir for NEW/RESUME); `src/cli` + `src/bin/verifier_loop.rs`
  (two new subcommands + precondition warnings).
- **APIs**: two new CLI subcommands (`RECOVER`, `STATUS`). No change to the completion-hash
  formula or the verdict/consensus layers — `RECOVER` reuses `consensus::evaluate`
  unchanged (the verdict file is the resumption contract; LD8).
- **Dependencies**: none new (`fs4` advisory locking is already a dependency, used by the
  receipt log).
- **Security**: **no threat-model regression.** Per-slot signing secrets remain
  unpersisted (live only in the original child env). RECOVER never mints keys, never
  re-renders, never kills — so the frozen-snapshot-per-round invariant and the
  signed-verdict-record regime are preserved verbatim. See `design.md` §Threat model.
- **Out of scope**: PID files (LD9 — no kill step, so no detection needed); secret
  persistence (LD8 rejected path-b); snapshot TTL (LD11 — RECOVER never fresh-spawns);
  OT1 audit subcommand, OT2 per-verifier maxTurn refresh, OT6 fan-out scouts
  (already-design.md non-goals).
