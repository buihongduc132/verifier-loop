# Explore Turn 5 — OT7 collapsed by LD2, OT6 deferred

> Date: 2026-07-21
> Phase: explore (ospx:explore continuation)
> Status: OT7 resolved (superseded by LD2); OT6 deferred by user. 1 thread remains.

## Context

After the findings-resolve-threads sub-action (which auto-decided OT3/OT4/OT5/OT8
as LD13–LD16 and escalated OT6/OT7), user reviewed the open-threads and pushed back
on OT7. The pushback was sharp: OT7 had already been answered by LD2.

## [user] (verbatim — trust chain L1)

> OT7 , isnt the point is each of them is the ROUND , and EACH round will always see the PREVIOUS one? Why do keep concerning this when I am explicitly COLLAPSED the scenario into ROUND; 
> Ot6: not yet;

## [assistant]

**You're right — I was over-engineering.** LD2 already collapsed it: smart phase *is*
an ordinary round. Ordinary rounds already receive prior-round verdict notes via
`prompt::collect_prior_reject_notes` (called in `src/bin/verifier_loop.rs::run_round`,
~line 370). So smart phase automatically gets dump phase's notes. No new mechanic, no
design fork.

- **OT7** → superseded by LD2. Resolved.
- **OT6** → stays open.

## Resolution detail

The threat-model framing I introduced in turn 1 ("reviewer-of-dump vs independent
committee") was false dichotomy under LD2. The collapse is total:

- LD2 says smart phase = ordinary round lifecycle.
- Ordinary rounds already chain prior-round notes (`collect_prior_reject_notes` +
  `append_prior_reject_notes`, `src/prompt/`).
- Therefore smart phase's prompt is constructed the same way as any RESUME round — it
  sees every prior phase's verdict notes.
- No new prompt-construction branch, no threat-model fork, no design choice.

The existing mechanic in `src/bin/verifier_loop.rs::run_round`:

```rust
// Feature b (intention 2026-07-14): build the prompt dynamically by collecting
// ALL prior REJECT verdict notes for this goal and appending them so the verifier
// sees the rejection history and can verify fixes. No-op when there are no prior
// rejects (e.g. round 1, or all-prior-APPROVE).
let prior_reject_notes =
    verifier_loop::prompt::collect_prior_reject_notes(root, goal_id, round);
let rendered = verifier_loop::prompt::append_prior_reject_notes(&rendered, &prior_reject_notes);
```

This already does exactly what OT7 was asking about — for the REJECT case. The only
delta the pipeline adds: also surface prior-phase APPROVE notes (or at minimum the
fact of prior-phase pass) so smart knows "dump already approved" not just "prior
rejected". That's a minor prompt-rendering tweak inside the existing collect/append
helpers, NOT a new threat model.

## Status at end of turn 5

- OT7 → resolved (superseded by LD2, user re-collapse in this turn).
- OT6 → still open (user said "not yet" to OpenSpec capture).
- All 12 user-locked decisions (LD1–LD12) + 4 auto-decided (LD13–LD16) intact.
- 7/8 threads resolved. 1 remains (OT6 — process/intent).

Implementation-readiness: **still gated on OT6** (no OpenSpec change exists). Once
user says "yes, capture as proposal", the design is fully specified and ready for
`/opsx:new` → proposal + design + specs + tasks → TDD red/green.

→ next: awaiting user on OT6.
