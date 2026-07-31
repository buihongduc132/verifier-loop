# Explore Turn 1 — initial proposal + cost/state-machine analysis

> Date: 2026-07-20
> Phase: explore (ospx:explore)
> Status: open — user proposal on table, assistant surfaced design questions + cost model + alternatives

## Context

User invoked `/opsx:explore` (Family: ospx, Step 10/70) to think through a new
consensus-pipeline idea: default 2/2, but escalate to a smarter verifier when the
initial 2 verifiers agree unanimously.

## [user] (verbatim — trust chain L1)

> I would like to add this kind of dynamic round; a. default: 2/2. 2/2 with dynamic: if 1/2 approved , it is reject normally ; if 2/2 approved , THEN it will add the other turn at the end. Intention: - I would like to put the initial 2/2 to the dumper model , to reduce cost , increase speed; whenever it is be able to capture problem: GOOD (0/2 , 1/2) ; BUT whenever it is 2/2 , we need to make the addition check by using the SMATER model / profile;

## [assistant]

Grounded the proposal against the actual codebase. Files read (full list in
`references.md`):

- `openspec list --json` — active change inventory
- `openspec/changes/add-verifier-loop-cli/specs/consensus-check/spec.md`
- `src/consensus/mod.rs`, `src/goal/mod.rs`, `src/spawn/mod.rs`,
  `src/spawn/orchestrator.rs`, `src/cli/mod.rs`, `src/store/config.rs`,
  `src/acp/adapters.rs`, `src/bin/verifier_loop.rs`, `src/lib.rs`
- `openspec/changes/per-verifier-adapter/specs/*/spec.md` + `proposal.md`

Load-bearing evidence excerpts (cited in analysis below):

**`consensus-check/spec.md` — the guardrail that conflicts with the proposal:**

> ### Requirement: Consensus is static and human-configured
> The values of `n` and `m` SHALL be read from `config.json` and MUST NOT be dynamically chosen by the CLI or by A at runtime.

**`src/consensus/mod.rs` — hash formula:**

```text
short       = mmddyy + "-" + first8hex(SHA256(inputs))   // displayed, printed
fullDigest  = SHA256(inputs)                              // 64 hex, stored not printed
inputs      = salt + goalId + goalSignature + String(roundNumber)
           + canonicalJSON(matchingVerdicts sorted by verifierId) + matchedAtISO
```

---

Interesting idea — a "cheap gate + expensive confirmation" pattern. Let me ground this in the actual codebase first.

## What you're proposing

A **2-phase consensus pipeline**: phase 1 = 2 cheap verifiers (gate); phase 2 = 1 expensive verifier (confirmation), runs *only* if phase 1 passes unanimously. Cheap catches problems → cheap reject. Cheap agrees → escalate.

## Maps cleanly onto existing arch

The pieces already exist:

```
goal/state.json: current_round (u32, increments on RESUME)
consensus::evaluate(verdicts, n, m) → ConsensusResult{passed, ...}
spawn_round / spawn_resume (per-verifier adapter already supported)
completion hash inputs: salt + goalId + sig + String(roundNumber) + canonicalJSON(matchingVerdicts) + matchedAtISO
```

So the escalation can be modeled as **round N+1 with a different config**, triggered conditionally by round N passing. The hash machinery doesn't care — it just records whichever round reaches consensus.

## ⚠ The spec conflict you'll have to break

Current `consensus-check` spec has an explicit guardrail:

> **Consensus is static and human-configured** — `n` and `m` SHALL be read from `config.json` and MUST NOT be dynamically chosen by the CLI or by A at runtime.

Your idea is *compatible* with the *intent* (the escalation rule is static in config, not chosen at runtime by A) — but it introduces a **second `(n,m)` pair**. So this isn't "add a feature," it's **modify the spec** from "one static threshold" → "a static escalation policy that may chain multiple phases."

Worth asking: *why was that guardrail there?* My read: predictability of the receipt, and to prevent A from gaming the threshold. A chained policy preserves both *if* the whole chain is declared up-front in config + frozen into `goal.json`'s config snapshot (which it already does — per-verifier-adapter change made goal.json snapshot the adapter config).

## State machine

```
              ┌──────────────────┐
   NEW ──────▶│ phase 1: m=2     │   config: {adapter:"pi-dumper"}
              │ v1,v2 CHEAP      │
              └────────┬─────────┘
                       │
                ┌──────▼──────┐
                │ evaluate    │── 0/2, 1/2 ──▶ REJECT (cheap caught it) ✅ $ saved
                │ n=2, m=2    │
                └──────┬──────┘
                       │ 2/2 unanimous
                ┌──────▼──────┐
                │ phase 2: m=1│   config: {adapter:"hermes", model:"opus"}
                │ c1 EXPENSIVE│   ← auto-triggered, same jewilo invocation
                └──────┬──────┘
                       │
                ┌──────▼──────┐
                │ evaluate    │── REJECT/NULL ──▶ REJECT (expensive vetoed)
                │ n=1, m=1    │
                └──────┬──────┘
                       │ 1/1
                ┌──────▼──────┐
                │ completion  │   hash records round N+1 + c1 verdict
                │ hash + json │   (receipt should ALSO log phase-1 2/2 — see below)
                └─────────────┘
```

## 🔥 The counterintuitive bit: this can cost MORE, not less

Your framing assumes "cheap-first = always cheaper." The math says otherwise. Let `p` = probability a goal passes the cheap gate unanimously (2/2):

| Strategy | Cost per goal |
|----------|---------------|
| 1 expensive always | `1·E` |
| 2 cheap + escalate | `2·C + p·E` |

Escalate wins only when `2·C + p·E < 1·E`, i.e. **`p < 1 − 2C/E`**.

Example `C=$1, E=$10` → breakeven at `p < 0.80`.
- If <80% of goals pass cheap 2/2 → escalate saves money ✅
- If >80% pass (work is usually good) → **"1 expensive always" is cheaper** ❌

So the win hinges on **how often cheap verifiers actually catch problems**. If your cheap gate rarely rejects, you're paying 2×C tax on almost every goal *and* still paying E on most. Measure `p` before committing.

## Open design questions (the ones that shape the spec)

1. **Veto semantics**: expensive REJECT → hard reject, or just "trigger normal RESUME N+2"? (Affects whether expensive can deadlock a correct goal.)
2. **Visibility**: does expensive see v1/v2 APPROVE notes, or verify blind? (Reviewer-of-cheap vs independent committee — different threat models.)
3. **Loop risk**: `2/2 cheap pass → expensive reject → fix → 2/2 cheap pass → expensive reject → ...` Could spin forever. Need a rule (max escalations? switch to expensive-only after K?).
4. **Sync vs async**: one `jewilo NEW` blocks through both phases (simple, matches current "one invocation = one decision"), or returns after phase 1 and needs a separate confirm command (adds a new goal state `AwaitingConfirmation` → touches RECOVER/STATUS).
5. **Receipt completeness**: completion hash records round N+1's `matchingVerdicts`. The phase-1 2/2 fact is *lost* unless you add a field (e.g. `priorPhaseConsensus` in `completion.json`). Audit wants this.
6. **Per-goal or global**: escalation policy in `config.json` (global) or per-goal at `NEW` time? Spec currently says config-only.
7. **Tamper**: escalation verifier needs its own pinned pubkey (add-verifier-tamper-hardening). Slot naming — extend `v3` or new namespace `c1`?

## Alternative framings worth weighing

| Variant | Triggers expensive when… | Catches |
|---------|--------------------------|---------|
| **Your proposal** | cheap agrees (2/2 approve) | cheap both wrong same way |
| **Inverse — tiebreak** | cheap *disagrees* (1/2) | cheap split on hard cases |
| **Spot-check** | cheap agrees, with prob `p` (e.g. 10%) | cheap collusion, caps E cost |
| **Reviewer mode** | cheap agrees; expensive sees their notes | cheap shallow-approve |

The inverse (tiebreak) is interesting because it targets the *other* failure mode and has a cleaner cost profile: expensive only runs on disagreement, which is rarer than unanimous-pass in healthy workflows.

## My read

Concept is sound and arch-compatible. Two things to nail down before it's a change:
- **The spec modification** (static single-threshold → static chained-policy) — needs a deliberate spec delta, not a sneaky add.
- **The cost model** — measure `p` (cheap-gate pass rate) on real history first. If `p > 0.8`, the tiebreak variant or "1 expensive always" may serve the cost/speed intent better than escalate-on-agreement.

---

Want me to:
- (a) dig into `consensus::evaluate` + the receipt log to sketch exactly which spec deltas + completion.json fields this needs,
- (b) model the cost/loop-risk more formally (incl. the escalation-deadlock case),
- (c) compare your proposal vs the tiebreak variant against your actual failure modes, or
- (d) keep pulling threads — e.g. how this interacts with the `health` cooldown (cheap dumpers crashing trips cooldown and blocks the expensive phase too)?

No code, no spec files yet — just thinking.

---

**Remaining `ospx` steps (step > 10):** none surfaced in this turn — `ospx.yml` step manifest isn't in the repo; tell me the total step count or the family's step list if you want the >10 enumeration.

## Status at end of turn 1

Proposal on table. Assistant flagged: spec conflict (static-threshold guardrail), cost model caveat (p > 0.8 → escalate loses), 7 open design questions, 4 alternative framings. No decisions locked yet. → next: turn 2 (user reframes).
