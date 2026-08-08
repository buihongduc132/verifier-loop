# Resistance Evaluation — Shape-1 Round Recovery (wait-only, never kill)

> Date: 2026-07-12  
> Evaluator: path-a-resistance  
> Context: task #7 — quantify IMPLEMENTATION + PRODUCT + THREAT-MODEL + FAILURE-MODE resistance of shape-1 versus status quo (no recovery at all).  
> Status: provisional (pending user re-lock on LD1)

---

## 1. IMPLEMENTATION resistance

Shape-1 DEFINITION: `RECOVER` waits only for already-live orphan children to write their `verdict.json`; it never kills any child; slots whose orphan is dead and whose verdict is still null fall through to the user message "run `RESUME N+1`". No secrets are persisted; no fresh spawn is attempted for pinned-but-null slots.

### 1.1 Required code changes

| Location | What shape-1 needs | Approx LOC delta | Notes |
|---|---|---|---|
| `src/cli/mod.rs` | Add `Recover { goal_id }` variant to `VerifierLoopCmd` | ~6 | Mirrors existing `Resume` variant. |
| `src/bin/verifier_loop.rs` | Dispatch `Recover` to a new `run_recover` function | ~15 | Shared validation + store resolution; no prompt rendering. |
| `src/goal/mod.rs` | Possibly expose `current_round` (already public); no state mutation needed | ~0 | Shape-1 does NOT increment `current_round` (it is same-round). |
| `src/spawn/orchestrator.rs` | New `shape_1_recover_poll` function | ~60–80 | Reads slot meta, loops over `verdict.json` read, polls with timeout, reports slot status. No `Command::spawn`, no `mint_verifier_secret`. |
| Tests | Hermetic `recover` tests | ~80–120 | Mock orphan process by writing verdict after delay; test null/dead fallthrough. |

### 1.2 New modules

- None required. Shape-1 can live entirely in `src/bin/verifier_loop.rs` plus a helper in `src/spawn/orchestrator.rs` (or `src/spawn/mod.rs`). It does not justify a new crate module.

### 1.3 Does it touch verdict / consensus / crypto?

No. The existing primitives are read-only or reused as-is:

- `verdict::read_verdict` — already exists, read-only.
- `consensus::evaluate` — reused unchanged; shape-1 simply calls it after the poll window closes.
- `verdict::mint_and_pin_pubkey` / `verdict::verify_signature` — **not called at all** in shape-1; the pinned pubkey from the original spawn is preserved because no new spawn occurs.
- `crypto` — untouched; no new key minting or signing.
- `receipt` / completion hash — unchanged.

### 1.4 Implementation summary

Shape-1 is essentially a **wait + poll verdict.json loop** followed by a normal consensus evaluation. It is the smallest recovery primitive that is compatible with the landed tamper-hardening (no secret persistence, no new key minting, no first-write-wins collision on pinned slots).

**Estimated total LOC delta:** ~150–220 including tests; ~60–100 net production code.

**Implementation resistance score: 2/10** (low friction; mostly new CLI wiring and a polling helper).

---

## 2. PRODUCT resistance — when shape-1 fails to salvage a slot

Shape-1 only wins when an orphan child process is still alive and eventually writes a signed `verdict.json`. It cannot help when the signer is dead. Real scenarios:

### (a) TTY Ctrl-C — children die with jewilo

- **What happens:** Interactive terminal `Ctrl-C` sends `SIGINT` to the whole process group (pgrp). Both `jewilo` and its children receive it; if backends do not ignore SIGINT, they die too. The slots become pinned-but-null with no live signer.
- **Can shape-1 salvage?** No. The children are dead, and the signing secret is gone.
- **Can ANY approach salvage?** No — unless the secret is persisted to disk (G4-2 threat-model regression), or unless a supervisor/cgroup keeps the children alive. If the signer is dead, no valid signature can be produced for that pinned slot.
- **Prevalence:** Common for interactive debugging / local use, but less common in the target deployment (outer agent driving `jewilo`).

### (b) Non-TTY SIGTERM from outer agent — orphans alive

- **What happens:** An outer agent (e.g. a CI step, a wrapper, or a scheduler) sends `SIGTERM` to `jewilo` only. Children are reparented and keep running. The verifier timeout still applies from the child's own clock.
- **Can shape-1 salvage?** Yes, if the poll window is long enough to cover the remaining time until the orphan writes its verdict. This is the case shape-1 is designed for.
- **Prevalence:** This is the scenario that motivated recovery in the first place. In automated/scheduled deployments, SIGTERM-to-parent is arguably at least as common as TTY Ctrl-C, and perhaps more common for long-running CI agents. However, if the outer agent uses a process-group kill (`kill -INT <pgid>`), `docker stop`, or cgroup-scoped termination, it collapses to scenario (a).
- **Caveat:** G3-3 (SIGPIPE on jewilo exit) means some orphans may die from broken stdout/stderr pipes before they can finish, depending on backend SIGPIPE handling. So even scenario (b) is not guaranteed.

### (c) Child crash / OOM while jewilo is alive

- **What happens:** A child process crashes or is OOM-killed before writing a verdict. Slot is pinned-but-null; the signing secret died with the child.
- **Can shape-1 salvage?** No. The signer is dead.
- **Could secrets-persist help?** Partially. If the secret were on disk, a fresh process could sign a verdict for the pinned slot. But this introduces G4-2 (threat-model regression) and is explicitly excluded by the task's premise ("NEVER persisted to disk"). Even then, a fresh spawn would require a fresh prompt/render and snapshot TTL consistency, which is its own product friction (the task notes this).

### 2.1 Summary of salvageability

| Scenario | Salvageable by shape-1? | Salvageable by any no-persist approach? | Prevalence estimate |
|---|---|---|---|
| TTY Ctrl-C | No | No | Common locally; rare in outer-agent deployments |
| Non-TTY SIGTERM to jewilo only | Yes (if orphan survives + finishes) | No other approach needed | Common in CI/scheduled wrappers; uncertain due to SIGPIPE caveat |
| Child crash / OOM | No | No (without secret persistence) | Unpredictable; depends on backend stability |
| Cgroup/systemd kill (reaps all) | No | No | Common in containerized/systemd deployments |
| `docker stop` / no-forward PID 1 | Maybe (children survive) | Maybe, but same as shape-1 | Container-dependent |

**Product resistance score: 6/10** — shape-1 only helps in the SIGTERM-to-parent case, which is real but narrow and complicated by SIGPIPE/cgroup/supervisor behavior. It does nothing for the most common local failure (Ctrl-C) or for child crashes.

---

## 3. THREAT-MODEL resistance

Shape-1 preserves `THREAT-MODEL.md` almost verbatim. The key scope is quoted below:

> **A process that has write access to `~/.verifier-loop/` AND can read a V\*'s environment (i.e. `VERIFIER_LOOP_VERIFIER_SECRET`) can still forge that V\*'s verdict.**

Shape-1 does not change this:

- It does not persist the signing secret to disk.
- It does not spawn new children for pinned slots.
- It does not change the signature-verification gate in `consensus::evaluate`.
- It does not introduce a new way to produce a signed verdict without the original child.

The only place where shape-1 might require a documentation note is in the **operational guidance** section: the same guidance that says "Treat same-box jewilo as coordination + audit, not prevention" can add a sentence that `RECOVER` is a same-round coordination convenience, not a mechanism that restores a lost signing secret. No new threat model or new attacker capability is introduced.

**THREAT-MODEL resistance score: 1/10** — essentially zero friction; the existing model already accounts for shape-1's behavior.

---

## 4. FAILURE MODE / user-visible behavior

When shape-1 cannot salvage a slot, the behavior is:

1. `RECOVER` loads the current round and polls each slot for a configurable window (e.g. `min(verifier_timeout_sec, 60s)`).
2. For any slot that is still null and has no live orphan (or the orphan has died), the slot is left null.
3. `consensus::evaluate` is called with the current verdicts.
4. If consensus still fails, the CLI prints:
   - the same rejection summary as a normal round (`round N did not reach n/m consensus`),
   - a list of null verifiers that could not be recovered,
   - and an explicit recommendation: `run 'jewilo RESUME <goalId>' for round N+1 with fresh verifiers and fresh keys.`

### 4.1 Is this acceptable?

Yes, with caveats:

- It is **honest**: the user is told exactly what happened and what to do next.
- It is **fail-closed**: no unsigned or unauthenticated verdict is counted.
- It is **consistent with the rest of the design**: RESUME is already the recovery mechanism for failed rounds; shape-1 is just a same-round salvage attempt.
- Caveat: if the user expected `RECOVER` to be a magic fix, they will be disappointed. But that is a documentation issue, not a correctness issue.

**FAILURE-MODE resistance score: 2/10** — the UX is clean and the failure path is already the product's normal recovery path.

---

## 5. Overall scores and verdict

| Dimension | Score (1 = least resistance, 10 = most resistance) | Rationale |
|---|---|---|
| IMPLEMENTATION | 2 | Small LOC, no new modules, no crypto/consensus/verdict changes. |
| PRODUCT | 6 | Only the SIGTERM-to-parent orphan case is recovered; Ctrl-C, crash, OOM, cgroup kills all fail. |
| THREAT-MODEL | 1 | Preserves `THREAT-MODEL.md` verbatim; no new secrets, no new trust assumptions. |
| FAILURE-MODE UX | 2 | Clear user-visible behavior, fail-closed, falls back to existing `RESUME` path. |

### One-paragraph verdict

Shape-1 is the path of least resistance among the recovery designs that respect the landed per-verifier Ed25519 tamper-hardening. Implementation is straightforward: a new CLI subcommand, a polling helper that reads `verdict.json`, and a normal `consensus::evaluate` call — no changes to the signature, pinning, or consensus layers. Threat-model resistance is essentially perfect because the signing secret is never persisted and no new spawn is attempted for a pinned slot. The real cost is product resistance: shape-1 only salvages the scenario where `jewilo` is killed by a single-process SIGTERM but the children remain alive and healthy. It does nothing for the more common TTY Ctrl-C, child crashes, OOMs, or cgroup-scoped kills. When it cannot recover, the user-visible outcome is a clean rejection and a prompt to `RESUME N+1`, which is acceptable because it matches the product's existing fail-closed recovery path. If the goal is a robust recovery feature, shape-1 is a narrow band-aid; if the goal is a recovery feature that does not compromise security, shape-1 is the best available option without revisiting the "never persist secrets" constraint.

---

## Open follow-ups (not part of this task)

- Decide whether to reopen LD1 and lock shape-1, or to persist secrets (G4-2) and accept the threat-model regression.
- If shape-1 is selected, confirm the poll window and the exact user-visible message for the `RECOVER → RESUME N+1` fallthrough.
- Determine whether to detect "live orphan" via PID file only, or via `kill(pid, 0)` + `/proc/<pid>/comm` validation (PID recycling safety from G4-3).
