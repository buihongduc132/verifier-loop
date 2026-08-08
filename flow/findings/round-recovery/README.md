# Round Recovery

> Date range: 2026-07-12 → 2026-07-12
> Status: explore-ongoing (2 user-locked, BOTH CHALLENGED by gotcha-coverage; 10 open threads)

## Topics

### round-recovery (2026-07-12)
Explored how jewilo should resume the CURRENT round after being killed/interrupted (vs spawning a new round). Two kill scenarios: A (TTY SIGINT → children die with jewilo), B (non-TTY SIGTERM → verifiers orphaned). Core insight (later qualified): verdict file = resumption contract; consensus::evaluate + hash unchanged; meta.json purely SID-reuse hint. Probed pi: SID on stdout line 1 (t≈0), session file survives kill, concurrent same-SID = silent race. User locked shape 2 (kill orphan → resume-by-SID → <2KB nudge).

### gotcha-coverage (2026-07-12, two sub-agents)
Reviewer-a (safety) + reviewer-b (secret lifecycle). **Bombshell: LD1 shape-2 is INVALIDATED against landed tamper-hardening** — the per-slot signing secret is never persisted; killing the orphan destroys the only valid signer; fresh spawn hits `AlreadyPinned`. So a pinned-but-null slot with no LIVE signer can never produce a countable verdict. Both user-locks (LD1, LD2) are now challenged; NOT auto-superseded (user-locked) — await user relock. 10 open threads (OT5 = secret-lifecycle is the load-bearing one). Reframed recovery: shape-2 only viable if secrets persist (threat-model regression); else RECOVER degrades to shape-1 (wait-for-verdict, never kill the signer) + RECOVER→RESUME fallthrough for dead slots.

## Pick up next time
1. `2026-07-12-turn4b-gotcha-secret-lifecycle.md` — G5-1 invalidates LD1. Read FIRST.
2. `2026-07-12-open-threads.yaml` OT5 — load-bearing. Forces a new shape decision.
3. `2026-07-12-turn4a-gotcha-recovery-safety.md` — G1/G3/G7 (kill safety, concurrency) still apply even if shape-1 is chosen.
4. **Decision needed from user:** reopen LD1? Lock shape-1 (wait, never kill signer) + RECOVER→RESUME fallthrough? Or persist secrets (threat-model regression, G4-2)?
5. After LD1 relocked + OT5-OT9 closed → draft `add-round-recovery` openspec change. Step 50 (to-tasks) BLOCKED until threads close.
