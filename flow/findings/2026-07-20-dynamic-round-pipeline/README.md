# dynamic-round-pipeline

> Date range: 2026-07-20 → 2026-07-21
> Status: explore-ongoing (1 thread deferred OT6; 14 new gotcha-coverage threads OT9-OT22)

## Topics

### dynamic-round-pipeline (2026-07-20)

Explored adding a phased consensus pipeline to `jewilo`: default Gate (m dump
verifiers, threshold n) → on unanimous pass, Confirm (smart verifier). After 2
consecutive Gate-pass/Confirm-reject, escalate Gate to Mixed mode (half dump +
half smart, smart rounds up), then a Final smart. Reframed from cheap/expensive
(cost-irrelevant) to **Dump (D)** vs **Smart (S)**. Locked 12 decisions (LD1–LD12):
dump/smart framing, smart=round-of-review, 1a/1b/1c sub-round naming, 2-consecutive
escalation rule, single-cmd, `<D>+<S>/<n>` output, hash over chain, global config,
dynamic Mixed via floor/ceil, generic m, rot-proof abstractions, Esc→esca rename.
Introduced **Phase** (`{dump,smart,threshold}`) + **Pipeline** (`Vec<Phase>`) as
rot-proof primitives; D/S are adapter refs. 4-field config extension
(`dumpAdapter`, `smartAdapter`, `confirmCount`, `escaT`). 8 threads raised; 7
resolved (OT1/OT2 superseded by LD4, OT3-5/OT8 auto-decided as LD13-16, OT7
superseded by LD2 in turn 5). 1 remains (OT6 — OpenSpec capture, deferred by
user "not yet").

### ot7-collapse + ot6-defer (2026-07-21, turn 5)

User pushed back on OT7 (smart visibility threat model): LD2 already collapsed
smart phase into ordinary round, and ordinary rounds already chain prior-round
notes via `prompt::collect_prior_reject_notes` (`src/bin/verifier_loop.rs::run_round`).
So smart automatically sees dump phase's notes — no threat-model fork. OT7 →
superseded by LD2. OT6 (OpenSpec capture) deferred by user ("not yet"). Design
is now fully specified; only gate to implementation is OT6.

### gotcha-coverage (2026-07-21, step 20)

4 reviewer sub-agents reviewed all 16 LDs + OT6 + 2 turn conclusions. 56 gotchas
found (2×Rank5, 12×Rank4, 19×Rank3, 11×Rank2, 1×Rank1). Grouped into 14 composite
threads (OT9-OT22). **Critical finding**: OT10 (2×Rank5) INVALIDATES Turn 5's
conclusion — the claim "smart automatically gets dump notes, no new mechanic" is
wrong for within-invocation sub-phases. `collect_prior_reject_notes` parses dir
names as u32 ("1a" fails) AND filters `round >= current_round` (same in one
invocation). A note-passing mechanism IS required. No user-locked decisions
invalidated; design is still viable but needs the OT9-OT22 resolutions folded
into the OpenSpec proposal before implementation.

## Pick up next time

1. `2026-07-20-locked-decisions.yaml` — 16 decisions (LD1-12 user-locked, LD13-16 auto); the spec proposal MUST honor all.
2. `2026-07-20-open-threads.yaml` — 7 resolved, 1 open (OT6).
3. `2026-07-20-turn3-dynamic-generic.md` — the rot-proof Phase/Pipeline abstraction (canonical design shape).
4. `2026-07-20-turn2-reframe-dump-smart.md` — the DD/DS pipeline diagrams + output format table.
5. `2026-07-21-turn5-ot7-collapsed.md` — the LD2-collapse argument that kills OT7.
6. Open decision: OT6 — when ready, capture as OpenSpec proposal (`/opsx:new` or `/opsx:ff`).
