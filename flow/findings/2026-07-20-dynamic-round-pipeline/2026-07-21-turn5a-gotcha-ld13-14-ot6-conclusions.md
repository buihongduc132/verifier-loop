# Appendix to 2026-07-21-turn5-ot7-collapsed.md) — gotcha coverage

> Gotcha coverage for: Turn 5 (./2026-07-21-turn5-ot7-collapsed.md)
> Sub-agent: reviewer (parallel batch)
> Items reviewed: LD13, LD14, OT6, Turn 1 & 5 conclusions
> Also relevant to: Turn 1, Turn 3, Turn 4 (cross-turn references in body)

---


# Gotcha Batch 4 — LD13, LD14, OT6, Turn 1 & Turn 5 conclusions

> Date: 2026-07-21
> Scope: dynamic-round-pipeline explore — gotcha hunt only (no re-analysis)

---

## Rank 5 — Sophisticated

### G4-1 [LD13 + Turn 5] Sub-phase round-dir naming breaks `collect_prior_reject_notes`

**Gotcha**: LD3 mandates letter-suffixed sub-round dirs (`1a`, `1b`, `1c`). `collect_prior_reject_notes` (`src/prompt/mod.rs:573`) parses dir names as `u32` — `"1a".parse::<u32>()` fails → silently skipped. The smart phase in sub-phase `1b` cannot see dump-phase REJECT notes from `1a`, even if dump partially rejected before the gate passed.

**Severity**: 5 (Sophisticated) — invisible data loss; the function returns empty string, no error. The Turn 5 conclusion ("smart automatically gets dump phase's notes") is **wrong** for within-invocation sub-phases.

**Evidence**: `src/prompt/mod.rs:573` — `let Ok(round) = name_str.parse::<u32>() else { continue; };`

**Mitigation**: Either (a) parse sub-round dirs with a `SubRound` type that handles `"{num}{letter}"` format, or (b) within a single invocation, pass dump-phase notes directly to the smart phase via an in-memory channel rather than relying on disk-scanned prior-round notes. Option (b) is cleaner since sub-phases are not separate rounds.

---

### G4-2 [Turn 5] `current_round` filter blocks same-invocation note collection

**Gotcha**: Even if round dirs were numeric, `collect_prior_reject_notes` filters `round >= current_round` (`src/prompt/mod.rs:576`). Within a single `jewilo NEW` invocation, `current_round = 1` for ALL sub-phases (Gate, Confirm, Mixed, Final). Gate's notes live in round 1 → `1 >= 1` → filtered out. Smart phase sees nothing.

**Severity**: 5 (Sophisticated) — same invisible data loss as G4-1 but via a different mechanism. The Turn 5 conclusion assumes sub-phases are "ordinary rounds" but ordinary rounds are separate invocations with incrementing `current_round`. Sub-phases within one invocation share the same `current_round`.

**Evidence**: `src/goal/mod.rs:144` — `state.current_round` increments only on RESUME, not on sub-phase transitions. `src/prompt/mod.rs:576` — `if round >= current_round { continue; }`.

**Mitigation**: Sub-phases need their own note-passing mechanism that doesn't depend on the cross-round `collect_prior_reject_notes` path. This IS a "design fork" — contradicting the Turn 5 conclusion.

---

## Rank 4 — Significant

### G4-3 [LD13] Health cooldown cascade within single invocation

**Gotcha**: The pipeline runs Gate (dump) → Confirm (smart) in one invocation. If dump verifiers are unhealthy (timeout/crash), those events record to `health.jsonl`. If 3+ dump runs are unhealthy in the rolling window, the Confirm phase inherits cooldown mode and returns a fallback hash (`<mmddyy>-ffffff`) instead of spawning the smart verifier. The smart phase is penalized for the dump phase's failures — even though the smart backend may be perfectly healthy.

**Severity**: 4 (Significant) — a flaky dump backend blocks the smart confirmation path. The operator sees a fallback hash and may not realize the smart verifier never ran.

**Evidence**: `src/health/mod.rs` — cooldown is store-wide, not per-backend. `src/bin/verifier_loop.rs` — health check runs once at the top of `run_round`, not per-sub-phase.

**Mitigation**: Either (a) scope health tracking per-backend/adapter so dump failures don't trip smart's cooldown, or (b) reset/re-evaluate health state between sub-phases within the same invocation.

---

### G4-4 [LD14] Pipeline field doesn't capture escalation depth

**Gotcha**: LD14 records `"PL-D"` or `"PL-E"` as binary metadata. But PL-E encompasses multiple paths: Gate→Confirm→Mixed→Final (1 escalation), Gate→Confirm→Mixed→Confirm→Mixed→Final (2 escalations), etc. Audit cannot distinguish a 1-escalation PL-E from a 5-escalation PL-E. The `pipeline` field loses the escalation history that LD4's consecutive-counter mechanism produces.

**Severity**: 4 (Significant) — audit trail is incomplete. Two completions with identical `"PL-E"` may have very different verification rigor.

**Evidence**: LD4 escalation rule allows unbounded escalation cycles (2× consecutive dumpPass+smartReject → Mixed → if pass → Final). The binary PL-D/PL-E field collapses all escalation paths.

**Mitigation**: Add an `escalation_count` or `pipeline_path` field (e.g., `"PL-E:G-C-M-F"`) to capture the exact sub-phase sequence. Alternatively, the trace.jsonl already records each sub-phase — document that audit should cross-reference trace.jsonl for path detail.

---

## Rank 3 — Moderate

### G4-5 [Turn 1] Cost model ignores conditional probability

**Gotcha**: The cost model `2C + pE` vs `E` assumes dump-phase pass rate `p` is independent of smart-phase outcome. In practice, if dump verifiers are shallow (miss bugs systematically), then `P(smart reject | dump pass)` is high — the worst cost scenario. The model's breakeven `p < 1 - 2C/E` doesn't account for the conditional: you want `p` low AND `P(smart reject | dump pass)` low. If dump is too lenient, you pay `2C + E` on almost every goal (dump passes, smart rejects, RESUME, repeat).

**Severity**: 3 (Moderate) — the cost analysis is directionally correct but misses the key risk: dump verifiers that are fast but useless (high `p`, high conditional smart-reject rate).

**Mitigation**: Measure both `p` (dump pass rate) and `q = P(smart reject | dump pass)` before committing. The effective cost is `2C + p·q·(E + resume_cost)` per goal, not `2C + pE`.

---

### G4-6 [LD13] Rejection output ambiguity in multi-phase pipeline

**Gotcha**: LD13 says "jewilo prints rejection notes + exits non-zero (same as a normal failed round)." But in a multi-phase pipeline, the rejection could come from Confirm (smart), Mixed, or Final — each with different implications. A Confirm reject means "dump agreed but smart disagreed." A Mixed reject means "even the mixed panel couldn't agree." The current rejection output format (`round {round} did not reach {n}/{m} consensus`) doesn't indicate WHICH phase rejected, making it harder for A to understand what to fix.

**Severity**: 3 (Moderate) — A sees "round 1 rejected" but doesn't know if dump caught it (cheap fix) or smart vetoed (subtle fix).

**Evidence**: `src/bin/verifier_loop.rs:530-534` — rejection output uses generic `round {round} did not reach {n}/{m} consensus` format with no phase context.

**Mitigation**: Include the phase name in the rejection output (e.g., `"Confirm phase (1b) rejected: smart verifier s1 REJECT: ..."`) so A knows which phase caught the issue.

---

### G4-7 [LD14] No schema version bump for completion.json

**Gotcha**: Adding `pipeline` to `CompletionRecord` changes the JSON schema. Existing audit tools or scripts that parse completion.json may not expect the new field. While serde's `default` handles deserialization, external tools (shell scripts, jq queries) that assume a fixed field set could break. No `schemaVersion` field exists in `CompletionRecord` to signal the change.

**Severity**: 3 (Moderate) — forward-compat is fine (serde default), but external consumers have no version signal.

**Evidence**: `src/consensus/mod.rs` — `CompletionRecord` has no `schema_version` field. The `trace_id` field was added with `skip_serializing_if` but no version bump.

**Mitigation**: Either add a `schema_version` field to `CompletionRecord`, or document that completion.json consumers must tolerate unknown fields (which is standard JSON practice but worth stating explicitly).

---

## Rank 2 — Minor

### G4-8 [OT6] Explore findings reference rotting code locations

**Gotcha**: The explore findings cite specific line numbers (e.g., "`src/bin/verifier_loop.rs::run_round`, ~line 370"). These line numbers are already stale — the actual `collect_prior_reject_notes` call is at line ~393 in the current file. Without a formal OpenSpec proposal that captures design intent (not code locations), the findings will increasingly disconnect from the codebase as it evolves.

**Severity**: 2 (Minor) — the references are already off by ~23 lines; will worsen.

**Mitigation**: When capturing as OpenSpec proposal, reference function names and module paths, not line numbers.

---

### G4-9 [Turn 1] Cost model ignores latency (the stated secondary goal)

**Gotcha**: The Turn 1 cost model only considers monetary cost (`2C + pE` vs `E`). But the user's original intention explicitly includes "increase speed" — dump verifiers are presumably faster. The model should account for wall-clock time: `2·t_dump + p·t_smart` vs `t_expensive`. A pipeline that costs more money but runs 3× faster may still win.

**Severity**: 2 (Minor) — the monetary cost analysis is valid for its scope; the latency omission is a known simplification.

**Mitigation**: Extend the model to a 2-axis comparison: `(cost, latency)`. The Pareto-optimal choice depends on the operator's priority.

---

## Rank 1 — YAGNI

### G4-10 [OT6] Process question has no technical gotcha

**Gotcha**: OT6 is a process/intent question ("capture as OpenSpec proposal?"). No technical gotcha exists — it's a workflow decision. The only risk is delay: the longer the proposal is deferred, the more the explore findings stale.

**Severity**: 1 (YAGNI) — not a technical concern.

**Mitigation**: Set a deadline for the OT6 decision to prevent indefinite explore state.

---

## Summary

| Rank | Count | Items |
|------|-------|-------|
| 5 | 2 | G4-1, G4-2 |
| 4 | 2 | G4-3, G4-4 |
| 3 | 3 | G4-5, G4-6, G4-7 |
| 2 | 2 | G4-8, G4-9 |
| 1 | 1 | G4-10 |

**Critical finding**: G4-1 and G4-2 together invalidate the Turn 5 conclusion. The claim "smart phase automatically gets dump phase's notes, no new mechanic, no design fork" is **incorrect** for within-invocation sub-phases. Two independent mechanisms (round-dir naming and `current_round` filtering) prevent note collection within a single pipeline invocation. A new note-passing mechanism IS required.
