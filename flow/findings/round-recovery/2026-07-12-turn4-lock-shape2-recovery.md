# Explore Turn 4 — lock shape 2 (kill orphan → resume by SID → nudge)

> Date: 2026-07-12
> Phase: lock decisions
> Status: decided → 2 locks, 1 closed thread

## [user] (verbatim — trust chain L1)

> 2: OK; wrinkle: no problem ;

## [assistant]

Solid. Shape 2 it is — kill orphan, resume by SID, nudge.

**Recovery primitive, final shape:**

```
RECOVER <goalId>
  │
  ▼
round = state.current_round     ← NOT incremented
  │
  ▼
for each slot v1..vN in round:
  ├─ verdict non-null  → KEEP
  └─ verdict null:
       ├─ orphan alive (pid file)? → kill → resume by SID → verdict-nudge → gather
       └─ no orphan                → spawn fresh → gather
  │
  ▼
evaluate consensus (unchanged)
  ├─ PASS → hash → done
  └─ FAIL → "round complete, n/m not met; use RESUME for round N+1"
```

**New artifacts** (cheap):
- `rounds/N/vN/pid` — written at spawn time, before gather
- `meta.json` gets SID written immediately (already structured for it; just move the write earlier)

**What stays untouched:** `consensus::evaluate`, completion hash inputs, audit trail, fail-closed invariants. Verdict file remains the resumption contract.

**Boundary vs RESUME:** RECOVER = same round, salvage work, no new context. RESUME = round N+1, fresh snapshot, fix-notes. Clean split.

Ready to capture this as an openspec change proposal? Suggest name: `add-round-recovery`. Want me to draft it?

## Status at end of turn 4

Locked: shape 2 (LD1), wrinkle accepted (LD2). Final recovery primitive frozen. New artifacts named (pid file, early meta SID). RECOVER/RESUME boundary crisp. Explore effectively concluded → ready for proposal/tasks. Next: step 10 persist (this run), then step 20 gotcha verify, then step 50 to-tasks.
