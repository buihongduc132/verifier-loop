---
name: audit-verifier-loop-runs
description: Audit verifier-loop run logs in time window, group problems, triage tooling vs model responsibility
argument-hint: "[hours] [goal-id]"
---

$ARGUMENTS

---

> to verify , check logs of the run ; then group them to problem that we should fix ; if the model do not works by itself ; callsout , it is other responsibility (../llm-configuration )

---

# GOAL (DOD)

Reliability report for verifier-loop runs in window W. Done when ALL:
1. Run inventory table: every goal touched in W — subject, rounds, verdicts, result/hash.
2. Integrity verified: `jewilo AUDIT` exit 0 for every goal w/ completion.json.
3. Problems grouped [P1..Pn], each w/ log evidence (file path + numbers).
4. Each problem TRIAGED: verifier-loop tooling bug (fix here) OR model/adapter misbehavior (CALLOUT → `../llm-configuration`, do NOT fix here).
5. Verdict: fail-closed invariants hold or not (NULL never → APPROVE, missing store → no hash, edit → hash mismatch).
No claim without log evidence. Cannot prove → do not claim.

# STEPS

## 1. Scope
- W = `$1` hours if present, else 24.
- `$2` = goal ID → single-goal deep-dive (skip window filter).
- Store root `~/.verifier-loop` (respect `VERIFIER_LOOP_HOME` if set).
- Repo = verifier-loop cwd (code refs below relative to it).

## 2. Inventory runs
- Window by mtime, NOT dir-name order (goal IDs = UUIDs, not chronological):
  `find ~/.verifier-loop/goals -maxdepth 2 -newermt "<utc-cutoff>" -printf '%T@ %p\n' | sort -rn`
- Per goal dir read:
  - `goal.json` — createdAt, goalText (first ~120 chars), config snapshot (n/m/maxTurn/verifierTimeoutSec/adapters). Snapshot = creation-time truth, live config.json may differ.
  - `state.json` — currentRound, escalation state.
  - `completion.json` — latest pass only: hash, fullDigest, matchingVerdicts, pipeline, roundNumber.
  - `trace.jsonl` — FULL lifecycle: pipeline.start / jewije.registered (per verdict, APPROVE|REJECT) / pipeline.passed | rejected (rejection text inline). Single source of truth for history.

## 3. Per-verifier slot stats
- Files: `rounds/<r>/<phase>/<vid>/{meta.json,verdict.json,stderr.txt,verifier-pubkey.json}`.
- `meta.json`: sid, turnsUsed, nudgeAttempts, recoveryAttempts, compactionObserved.
- `verdict.json`: status APPROVE | REJECT | missing/null.
- Compute per goal + overall: max turnsUsed vs maxTurn cap; null-verdict rate; nudge ratio (nudges ≈ turns?); REJECT reason extracts.

## 4. Wall-clock vs timeout
- Python over trace.jsonl: delta pipeline.start → each jewije.registered. Flag any > verifierTimeoutSec.
- KNOWN GAP: timeout enforced PER-NUDGE-ATTEMPT, not per-verifier wall-clock (`src/spawn/orchestrator.rs` — `tokio::time::sleep(timeout)` inside per-attempt select, lines ~313/775/818; config doc in `src/store/config.rs` says "per-verifier wall-clock"). Verdicts 35–215 min observed under 1800s config. Doc/behavior mismatch = tooling problem [P]; long wall-clocks NOT automatically timeout breach.

## 5. Integrity
- For each goal w/ completion.json: `jewilo AUDIT <goalId>` — exit 0 = valid (creation-time n/m re-checked, hash recomputed vs stored fullDigest). Non-zero = CRITICAL.
- `receipt-log.jsonl`: check signature-failure / tamper entries.

## 6. Health + live
- `health.jsonl`: count `{"event":"unhealthy"}` in W. Cooldown rule: >3 events / rolling 1h → cooldown mode → fallback hash `<mmddyy>-ffffff` (`src/health/mod.rs`).
- `ps aux | grep -E 'jewilo|jewije'` — live runs. DO NOT kill. Note orphaned drivers (goal mid-round, driver process gone).

## 7. Group problems [P#]
Each: cause class + evidence. Known classes (non-exhaustive):
- Timeout semantics doc/behavior gap (step 4).
- Null-verdict slots → wasted rounds. Diagnose via slot `stderr.txt` + turnsUsed.
- `health.jsonl` blind spots: external driver kill invisible; nudge-exhausted-to-null NOT logged unhealthy.
- Abandoned goals: REJECTs never remediated (round count frozen, no driver).
- Integrity failure (AUDIT non-zero) — CRITICAL, escalate immediately.
- Double-pass: goal passed rN then re-ran rN+1, completion.json latest-wins. Not a bug; audit-trail lives in trace.jsonl. Note only.

## 8. Ownership triage (CRITICAL)
For EACH [P#], classify:
- **Tooling** (fix in verifier-loop repo): timeout code, health logging, store format, orchestrator, hash/signature, docs drift.
- **Model/adapter** (do NOT fix here → CALLOUT): verifier never self-drives (nudges ≈ turns, needs constant nudging); null verdict w/ turnsUsed>0 (model stalled, no crash); REJECT notes like "cannot verify claims / evidence missing" (model failed investigation, not code broken); fabricated claims caught by verifier (model quality). These = model/prompt/adapter config → `../llm-configuration` (sibling repo). State verbatim: "model behavior — other responsibility, ../llm-configuration".

## 9. Output
- Runs table: goal short-ID, subject, rounds, result (hash | REJECT | running).
- Problems [P#] w/ evidence + triage owner.
- Callouts [CA#] = llm-configuration handoffs.
- Invariant verdict + unhealthy/cooldown status.

# WORKS — follow
- trace.jsonl = lifecycle truth incl. rejection text. Use it for history; completion.json only latest.
- meta.json = per-slot effort evidence.
- `jewilo AUDIT` = free tamper re-verify. Run on every completed goal, every audit.
- `find -newermt` mtime = correct windowing.
- `jewilo STATUS <goalId>` = current state probe (read-only, no lock).

# NOT-WORKS — avoid
- DO NOT trust health.jsonl completeness (blind spots, step 7).
- DO NOT sort goal dirs by name for chronology.
- DO NOT grep JSON logs for plain "fail" — parse JSON.
- DO NOT kill live jewilo/jewije processes.
- DO NOT propose model-behavior fixes inside verifier-loop repo — hand to ../llm-configuration.
- DO NOT treat wall-clock > verifierTimeoutSec as bug without checking per-attempt semantics first.

# TIPS
- Python one-liner: `datetime.fromisoformat(ts)` deltas for wall-clock.
- Goal short-ID = first 8 hex of UUID.
- Null-slot post-mortem: read slot `stderr.txt` first.
- Output `--json` mode exists on both binaries for machine parsing (`flow/usecases/programmatic-json-output.md`).

# REFERENCES
- `AGENTS.md` — module map, config schema, fail-closed invariants, hash formula.
- `src/health/mod.rs` — unhealthy detection + cooldown.
- `src/spawn/orchestrator.rs` — timeout/nudge/gather, stderr.txt capture.
- `src/store/config.rs` — config schema + defaults.
- `THREAT-MODEL.md` — tamper model limits.
- `flow/usecases/programmatic-json-output.md` — JSON envelope.
- `../llm-configuration` — model/adapter/prompt config responsibility (handoff target).

# MISTAKES / LESSONS
- 2026-08-16 audit: 12/55 null slots but 0 health events — blind spot confirmed. Check trace.jsonl, not health.jsonl alone.
- 215-min verdict under 1800s config — per-attempt timeout semantics; do not misreport as "timeout ignored".
- Double-pass goal (r3+r4): completion.json overwritten; history only in trace.jsonl.
- `ls goals/ | head` looked chronological — UUID order ≠ time order. Always mtime.
- REJECT "cannot verify claims" ≠ code broken — model investigation failure → llm-configuration.
