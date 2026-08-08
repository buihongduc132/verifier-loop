# Explore Turn 2 — reframe to dump/smart + DD/DS pipelines + naming

> Date: 2026-07-20
> Phase: explore (ospx:explore)
> Status: major reframe — user locked 8 design decisions (LD1–LD8); assistant proposed DD/DS pipelines, output format, hash composition

## [user] (verbatim — trust chain L1)

> reword / rethink all of your next response. DO NOT linger into cheap/expensive; linger into dump/smart. We are not consider about COST there at all. 
> 2. Consider the smart one is just another round of review. but name instead of 1-2-3-4 , it will be 1b 1c 1d or something ; 
> That having exact same mechanic , lifecycle as a normal round. 
> This is the designing now for simplicity; 
> 3. Ok , if it is 2 consequtive dumpPass - smartReject. Then switch to: 1/2 smart ; 1/2 dump; (50% , round up); THEN it still have to have another "smart" one if 2/2 passed (even with 1 smart 1 dump); 
> Now also NAMED / ABBRV each of these concept so that we can easily reference it later ; 
> 2/2 all dump ; half-half 2/2 (dump , smart); the final verdict after unanimous. 
> 4. 1 cmd only. ALSO , the output should shows like 2+1/2 instead of 3/2 or 2/2. To easily determine what kind of jewilo was ran. 
> 5. 2 verifier hash. 2/2 + 1Final as verdict. 
> 6. global. 
> 7. Give me your best suggestion ;

## [assistant]

## Reframe: Dump vs Smart (cost irrelevant)

**Dump (D)** = fast verifier. **Smart (S)** = thorough verifier. That's it.

## Naming

| Concept | Name | Abbr | Composition |
|---------|------|------|-------------|
| Initial 2-dump gate | **Gate** | **G** | 2D |
| Smart confirmation after gate passes | **Confirm** | **C** | 1S |
| Half-half after 2× consecutive G-pass-C-reject | **Mixed** | **M** | 1D+1S |
| Last smart after any unanimous pass | **Final** | **F** | 1S |

**Mode names:**
- **DD** = normal (Gate is 2D)
- **DS** = mixed (Gate is 1D+1S, triggered after 2× consecutive dumpPass-smartReject)

## Pipeline (single `jewilo NEW` / `RESUME` invocation)

```
┌─────────────────────────────────────────────────────────┐
│  MODE: DD (normal)                                      │
│                                                         │
│  [1a Gate: 2D] ──reject──▶ REJECT  output: "<x>/2"     │
│       │ 2/2 pass                                        │
│       ▼                                                 │
│  [1b Confirm: 1S] ──reject──▶ REJECT                   │
│       │ approve              dumpPass-smartReject++      │
│       ▼                                                 │
│     APPROVE   output: "2+1/2"                           │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│  MODE: DS (after 2× consecutive dumpPass-smartReject)   │
│                                                         │
│  [1a Mixed: 1D+1S] ──reject──▶ REJECT                  │
│       │ 2/2 pass                                        │
│       ▼                                                 │
│  [1b Final: 1S] ──reject──▶ REJECT                     │
│       │ approve                                         │
│       ▼                                                 │
│     APPROVE   output: "1+1+1/2"                         │
└─────────────────────────────────────────────────────────┘
```

Each sub-round (1a, 1b, 1c, 1d) has the **exact same lifecycle** as a normal round — spawn, gather, evaluate, hash. Just nested within one invocation instead of requiring RESUME.

## Output format: `<D>+<S>/<n>`

| Path | Output | Meaning |
|------|--------|---------|
| Gate rejected (0 dump pass) | `0/2` | dump caught it immediately |
| Gate rejected (1 dump pass) | `1/2` | partial dump pass |
| Gate pass, Confirm reject | `2+0/2` | dump agreed, smart vetoed |
| Gate pass, Confirm pass | `2+1/2` | normal APPROVE |
| Mixed reject | `0+1/2` or `1+0/2` | mixed mode failed |
| Mixed pass, Final reject | `1+1+0/2` | mixed agreed, final vetoed |
| Mixed pass, Final pass | `1+1+1/2` | mixed-mode APPROVE |

The `+` boundaries tell you exactly which path ran. No ambiguity.

## Hash composition (point 5)

The completion hash inputs = **all verdicts in the chain**, sorted by verifierId:

- DD path: `SHA256(salt + goalId + sig + round + canonicalJSON([v1_D, v2_D, s1_S]) + matchedAt)`
- DS path: `SHA256(salt + goalId + sig + round + canonicalJSON([v1_D, s1_S, s2_S]) + matchedAt)`

The hash always covers the full chain. `completion.json` gets a new field `mode: "DD" | "DS"` so audit knows which path produced it.

## Escalation counter

Tracked in `goal.json` (or a sibling `escalation.json`):

```json
{ "consecutiveDumpPassSmartReject": 2 }
```

- Incremented when Gate passes 2/2 but Confirm rejects
- Reset to 0 when Confirm approves OR when Mixed mode rejects
- When it hits 2 → flip to DS mode for subsequent RESUMEs

## My suggestion (point 7)

**Don't do the "2 consecutive" escalation.** It adds state, it's fragile across RESUMEs, and it creates an asymmetric experience (same goal ID behaves differently depending on history).

**Instead: make the mode a config flag.**

```json
{
  "n": 2, "m": 2,
  "gate": { "dump": 2 },
  "confirm": { "smart": 1 },
  "mixed": { "dump": 1, "smart": 1 },
  "mode": "DD"
}
```

User (or outer agent) sets `mode: "DS"` when they want mixed. No automatic escalation. Simpler state machine, same primitives.

If you DO want auto-escalation, make it a separate config flag: `"autoEscalate": true` — and the counter lives in `state.json` alongside `current_round`. But I'd start without it.

**Also: the "Final" sub-round after Mixed is redundant if Mixed already has 1S.** The smart verifier in Mixed already voted. Adding another smart after unanimous mixed pass is just "smart reviews smart's review" — diminishing returns. I'd drop Final and let Mixed's 2/2 be the terminal consensus.

So my simplified proposal:

```
DD mode: [Gate 2D] → pass → [Confirm 1S] → pass → APPROVE "2+1/2"
DS mode: [Mixed 1D+1S] → pass → APPROVE "1+1/2"   (no Final needed)
```

Two modes, two paths, one hash formula, zero auto-escalation state.

---

Want me to capture this as an OpenSpec proposal? Or keep poking at the design?

## Status at end of turn 2

User locked LD1–LD8 (dump/smart framing, smart=round-of-review, 1a/1b/1c naming, 2-consecutive-escalation rule, single-cmd, output format, hash-includes-all, global config). Assistant proposed DD/DS modes, `<D>+<S>/<n>` output, hash over chain, escalation counter. Assistant suggested (not confirmed by user): drop Final, config-flag over auto-escalation. → next: turn 3 (user wants dynamic + generic + rot-proof).
