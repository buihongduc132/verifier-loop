# Explore Turn 3 — make Mixed dynamic + generic m + rot-proof abstractions

> Date: 2026-07-20
> Phase: explore (ospx:explore)
> Status: user locked LD9–LD11 (dynamic Mixed, generic m, rot-proof); assistant introduced Phase/Pipeline abstractions, derived formulas, m=3 examples, m=1 edge case

## [user] (verbatim — trust chain L1)

> MAKE THE mixed to be dynamic calculate ; Not a fixed number; 
> 
> Also , make the 2D / 1D+1S to be more generic , because it is not always 2D initially , it could be 3/3 (3D) or something ; 
> 
> Remember: These number are configurable; Also , make the concept that will not rot.

## [assistant]

## Refined abstractions (rot-proof)

### Core concepts

| Concept | Abbr | Definition |
|---------|------|------------|
| **Dump verifier** | **D** | Adapter/profile config — fast, catches obvious problems |
| **Smart verifier** | **S** | Adapter/profile config — thorough, catches subtle problems |
| **Phase** | **P** | Generic execution unit: `{ role, count, threshold }` — "spawn N verifiers of role R, need T to approve" |
| **Pipeline** | **PL** | Ordered sequence of phases |
| **Escalation counter** | **Esc** | Tracks consecutive Gate-pass-Confirm-reject |
| **Escalation threshold** | **EscT** | Config value; when Esc ≥ EscT → switch pipeline |

D and S are **adapter references**, not hardcoded models. "Dump" and "Smart" are semantic labels; the actual backend/model/profile comes from config. If you rename your adapters tomorrow, the concepts don't rot.

### Two pipelines

```
DEFAULT PIPELINE (PL-D):
  P-Gate  { role: D, count: m,    threshold: n }
  P-Confirm { role: S, count: k,    threshold: k }   ← runs only if Gate passes

ESCALATION PIPELINE (PL-E):  (activated when Esc ≥ EscT)
  P-Mixed { role: D+S, count: m, threshold: n }   ← dynamic composition
  P-Final { role: S, count: k, threshold: k }     ← runs only if Mixed passes
```

### Dynamic Mixed calculation

Mixed composition is **derived from `m`**, never hardcoded:

```
mixedDump  = floor(m / 2)
mixedSmart = ceil(m / 2)
mixedTotal = m  (same as Gate — preserves threshold n)
```

| m | Mixed dump | Mixed smart |
|---|-----------|-------------|
| 2 | 1 | 1 |
| 3 | 1 | 2 |
| 4 | 2 | 2 |
| 5 | 2 | 3 |

### Config schema

```json
{
  "n": 2,
  "m": 2,
  "dumpAdapter": "pi",
  "smartAdapter": "hermes",
  "confirmCount": 1,
  "escalationThreshold": 2
}
```

No `gate`/`confirm`/`mixed`/`final` sub-objects needed — they're **derived**:

| Phase | role | count | threshold |
|-------|------|-------|-----------|
| Gate | D | m | n |
| Confirm | S | confirmCount | confirmCount |
| Mixed | ⌊m/2⌋ D + ⌈m/2⌉ S | m | n |
| Final | S | confirmCount | confirmCount |

Four new config fields (`dumpAdapter`, `smartAdapter`, `confirmCount`, `escalationThreshold`). Everything else is computed. Adding a 5th phase type later = add one derived rule, not a schema rewrite.

### Pipeline diagrams (m=3 example)

```
PL-D (default):
┌──────────────────────────┐
│ P-Gate: 3D, threshold=2  │──<n pass──▶ REJECT
│                          │
│  ≥2/3 approve            │
└──────────┬───────────────┘
           │ pass
┌──────────▼───────────────┐
│ P-Confirm: 1S, thr=1     │──reject──▶ REJECT  (Esc++)
│                          │
│  1/1 approve             │  Esc=0 on pass
└──────────┬───────────────┘
           │ pass
         APPROVE


PL-E (escalation, Esc ≥ EscT):
┌──────────────────────────┐
│ P-Mixed: 1D+2S, thr=2   │──<n pass──▶ REJECT  (Esc reset)
│                          │
│  ≥2/3 approve            │
└──────────┬───────────────┘
           │ pass
┌──────────▼───────────────┐
│ P-Final: 1S, thr=1       │──reject──▶ REJECT
│                          │
│  1/1 approve             │
└──────────┬───────────────┘
           │ pass
         APPROVE
```

### Output format: `<phases>/<n>`

Each `+` segment = one phase's pass count. Segments labeled by role:

```
PL-D paths:
  Gate rejected         →  "1/2"          (1D passed, below threshold)
  Gate pass, Conf rej   →  "2+0/2"       (2D passed, 0S passed)
  Gate pass, Conf pass  →  "2+1/2"       ← normal APPROVE

PL-E paths (m=3):
  Mixed rejected        →  "1+1/2"       (1D+1S passed, below threshold)
  Mixed pass, Final rej →  "1+2+0/2"    (1D+2S passed, 0S final)
  Mixed pass, Final pass→  "1+2+1/2"    ← escalation APPROVE
```

The `+` boundaries encode the pipeline path. Sum of segments = total approvals. Denominator = n (threshold). Unambiguous.

### Hash composition

Same formula, verdicts = **union of all matching verdicts across all phases**:

```
inputs = salt + goalId + goalSignature + String(round)
       + canonicalJSON(allMatchingVerdicts sorted by verifierId)
       + matchedAtISO
```

`completion.json` gains one field: `pipeline: "PL-D" | "PL-E"` — audit knows which path.

### Escalation counter lifecycle

```
Gate pass + Confirm reject  →  Esc++
Confirm pass                →  Esc = 0
Mixed pass + Final reject   →  Esc = 0
Mixed reject                →  Esc = 0
```

Stored in `state.json` alongside `current_round`. Survives RESUME.

### Edge case: m=1

```
floor(1/2) = 0 dump, ceil(1/2) = 1 smart
```

Mixed = 1S. That's identical to Confirm. So for m=1, PL-E degenerates: Mixed ≡ Confirm, Final ≡ Confirm. **Suggestion: require m ≥ 2 for escalation to activate.** If m=1 and Esc ≥ EscT, just keep running PL-D (Gate 1D → Confirm 1S). No Mixed/Final. Add a config validation: `escalationThreshold` is ignored when `m < 2`.

### My suggestion on "concepts that won't rot"

The rot-proof move is making **Phase** the atomic unit. Don't hardcode Gate/Confirm/Mixed/Final as separate code paths. Instead:

```rust
struct Phase {
    dump_count: u32,
    smart_count: u32,
    threshold: u32,
}

fn default_pipeline(cfg: &Config) -> Vec<Phase> { ... }
fn escalation_pipeline(cfg: &Config) -> Vec<Phase> { ... }
```

The executor runs `Vec<Phase>` sequentially, short-circuiting on reject. Adding a new phase type = changing the pipeline constructor, not the executor. The concepts (D, S, Phase, Pipeline) are stable; the compositions are config-derived.

---

Ready to capture as OpenSpec proposal? Or keep refining?

## Status at end of turn 3

User locked LD9 (Mixed dynamic via floor/ceil), LD10 (generic m, not hardcoded 2D), LD11 (rot-proof concepts). Assistant introduced Phase/Pipeline abstractions, derived Mixed formula, minimal 4-field config schema, `pipeline` field for completion.json, m=1 edge case rule, Vec<Phase> executor pattern. Open: drop-Final, auto-escalation vs config flag, m=1 handling confirmation. → next: turn 4 (rename Esc→esca).
