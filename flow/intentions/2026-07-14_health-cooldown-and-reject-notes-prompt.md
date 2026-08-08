# Intention: verifier-loop health self-awareness (cooldown fallback) + dynamic reject-notes prompt

Date: 2026-07-14
Branch: `feat/health-cooldown-prompt`
Worktree: `.worktrees/wt-health-cooldown` (off `origin/main` @ 1752f62)

## a. Self-aware health system + cooldown mode

The underlying issue: when a sub-agent (verifier backend) cannot produce a result OR
exits with a non-success exit code, the spawn layer currently leaves a null verdict
(fail-closed) but has no memory of these failures. Repeated backend failures across
successive rounds/goals therefore stall the main driving process indefinitely.

### Requirement

- Track "unhealthy" verifier runs. A run is **unhealthy** when:
  - it produced NO usable result (no SID captured / no final output), OR
  - the child process exited with a non-zero / non-success exit code.
- Persist a per-store health log (`~/.verifier-loop/health.jsonl`) of unhealthy events
  with timestamps.
- If **more than 3** unhealthy events occur within a rolling **1-hour** window, the
  `jewilo` CLI enters **cooldown mode**.
- In cooldown mode, `jewilo` does NOT spawn verifiers. It immediately returns a fallback
  hash of the form `<date>-ffffff` (the date prefix follows the existing `mmddyy`
  convention from the completion hash, with an `ffffff` suffix to distinguish it from a
  real consensus hash) so the main driving process is not completely blocked.

### Why ffffff

A real completion hash is `mmddyy-XXXXXXXX` (8 hex of a SHA-256). The cooldown fallback
is `<mmddyy>-ffffff` — visually distinguishable (6 f's vs 8 mixed hex), deterministically
recognizable as a non-consensus fallback, and never colliding with a real digest (a real
SHA-256 leading 8 hex starting with `ffffff` is astronomically unlikely and the fallback
deliberately uses the SHORTER `ffffff` token to avoid any ambiguity).

## b. Dynamic verifier prompt from previous REJECT notes

### Requirement

- Build the verifier prompt dynamically by collecting the **previous REJECTED verdict
  notes** (across prior rounds of the same goal).
- Append those notes to the current verifier prompt so the verifier sees prior
  rejections and does not repeat the same rejection cycles / can verify the fixes.

This generalizes the existing per-verifier own-prior-notes (`prevNotes`) mechanism (which
only feeds a single verifier its OWN prior-round notes) to a cross-round aggregation of
ALL prior REJECT notes for the goal.

## TDD discipline

Both features follow strict RED-then-GREEN. Coverage gate `>=80%` lines per new src file
via `cargo llvm-cov --fail-under-lines 80`.

## Deploy ceremony (new — to be recorded in AGENTS.md)

1. Build + deploy the WIP binary as `jewilo-dev` (so it does NOT shadow the stable
   `jewilo` on PATH).
2. Run the normal verifier-loop workflow once with the stable `jewilo`.
3. Run the workflow again with `jewilo-dev` (exercising the new features end to end).

## Out of scope

- Out-of-process health monitoring daemon.
- Auto-recovery / backend restart — cooldown is a non-blocking fallback, not a cure.
