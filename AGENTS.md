# AGENTS.md — verifier-loop repo

Single source of truth pointers for any agent (human or CLI) working in this repo.

## What this is

A Rust implementation of the `verifier-loop` skill's contract as two out-of-process CLIs
(`verifier-loop`/`jewilo` and `verifier-verdict`/`jewije`) that produce a tamper-evident
completion hash on n/m verifier consensus. See [`README.md`](README.md).

## Design source of truth (read FIRST)

- **Specs (behavioural contract):** [`openspec/changes/add-verifier-loop-cli/specs/`](openspec/changes/add-verifier-loop-cli/specs/)
  - `goal-lifecycle`, `verifier-spawn`, `verdict-registration`, `consensus-check`,
    `completion-proof`, `verifier-prompt`
- **Design decisions D0–D10 + risks:** [`openspec/changes/add-verifier-loop-cli/design.md`](openspec/changes/add-verifier-loop-cli/design.md)
- **Locked decisions LD1–LD27 + rationale:** [`flow/explore/2026-07-03-locked-decisions.yaml`](flow/explore/2026-07-03-locked-decisions.yaml), [`flow/explore/`](flow/explore/)
- **Language choice (why Rust):** [`flow/findings/2026-07-03-language-choice.md`](flow/findings/2026-07-03-language-choice.md)
- **Implementation roadmap:** [`openspec/changes/add-verifier-loop-cli/tasks.md`](openspec/changes/add-verifier-loop-cli/tasks.md) §1–§11
- **ACP sample fixtures:** [`flow/fixtures/`](flow/fixtures/)
- **Round recovery (SHAPE-1):** [`openspec/changes/add-round-recovery/`](openspec/changes/add-round-recovery/)
  (specs `round-recovery` + `goal-status` + `goal-lifecycle` delta; locked decisions LD3–LD11 in
  [`flow/findings/round-recovery/2026-07-12-locked-decisions.yaml`](flow/findings/round-recovery/2026-07-12-locked-decisions.yaml)).
  `recover` = cross-process round recovery (the `RECOVER` primitive + `STATUS` probe +
  `GoalLock` mutual exclusion); distinct from `compaction_recover` (within-round same-process).

## Module map

| `src/` module | tasks.md | spec |
|---------------|----------|------|
| `store/`   | §2 | goal-lifecycle (salt + config) |
| `goal/`    | §3 | goal-lifecycle (NEW/RESUME/immutability) |
| `acp/`     | §4 | verifier-spawn (ACP parser + adapters) |
| `spawn/`   | §5,§6 | verifier-spawn (orchestration + reuse) |
| `verdict/` | §7 | verdict-registration |
| `consensus/` | §8 | consensus-check + completion-proof |
| `prompt/`  | §9 | verifier-prompt |
| `round_recover/` | add-round-recovery | cross-process round recovery (RECOVER + STATUS + GoalLock) |
| `observe/` | add-otel-observability | lifecycle-tracing + trace-export (per-goal traceId + trace.jsonl + opt-in OTLP) |
| `health/`  | 2026-07-14 health-cooldown | unhealthy-run detection (`health.jsonl`) + cooldown fallback hash (`<mmddyy>-ffffff`) |
| `stats/`  | 2026-07-14 stats-audit | run introspection (`STATS`) + completion audit (`AUDIT`) |
| `cli/`     | §10 | wiring |

Both binaries (`jewilo`, `jewije`) support a global `--json` flag emitting one stable
camelCase envelope object on stdout (machine-readable contract; default output unchanged) —
see `## JSON output mode (--json)` in [`README.md`](README.md) and
[`flow/usecases/programmatic-json-output.md`](flow/usecases/programmatic-json-output.md).

## Observability / tracing (add-otel-observability)

The full `jewilo`/`jewije` lifecycle is observable via structured tracing:
- **Per-goal traceId**: `<store>/goals/<goalId>/trace-id` — minted at NEW, reused across RESUME, propagated to every V* child env (`VERIFIER_LOOP_TRACE_ID`) so `jewije` verdict registrations join the spawning round's trace.
- **Per-goal `trace.jsonl`**: newline-delimited JSON lifecycle events under `<store>/goals/<goalId>/trace.jsonl` (round start, consensus pass/reject, verdict registered). camelCase keys.
- **Opt-in OTLP**: build with `--features otel` + set `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT` to ship spans to a collector. Default builds link NO OpenTelemetry deps.
- **Level/format env**: `VERIFIER_LOOP_LOG` (default `info`), `VERIFIER_LOOP_LOG_FORMAT` (`text` legacy | `json` structured).

**Critical invariants (design D4/D5):**
- `traceId` is **metadata, NOT a completion-hash or receipt-`entryHash` input** — the hash inputs are byte-identical with and without tracing.
- Tracing is **fail-open**: any error in the observe layer is swallowed and never propagates to a verdict, consensus, or hash decision.

## TDD discipline (hard constraint)

Every feature group follows strict RED-then-GREEN:
1. A **fresh** teammate authors the failing (RED) test against the spec.
2. A **different fresh** teammate authors the minimal GREEN implementation.
3. Coverage gate `>=80%` lines per new src file before the group is marked done.

Never implement without a test first. Never have the same author write both RED and GREEN for a group.

## Coverage gate

```bash
cargo llvm-cov --fail-under-lines 80
```

## Health self-awareness + cooldown (2026-07-14)

The `jewilo` CLI is self-aware of backend health. A verifier run is **unhealthy** when it
times out, produces no usable result (no SID + no final output), OR the child exits
non-zero. Unhealthy events are appended to `<store>/health.jsonl` (one JSON line per
event, `{"event":"unhealthy","at":"<rfc3339>"}`).

If **more than 3** unhealthy events occur within a rolling **1-hour** window, `jewilo`
enters **cooldown mode**: instead of spawning verifiers (which would almost certainly fail
again and leave nulls), it immediately returns a recognizable **fallback hash** of the form
`<mmddyy>-ffffff`. This does NOT weaken fail-closed invariants — it returns a clearly
marked fallback (6 `f`s, vs a real 8-hex consensus hash), never an APPROVE or a real hash.
See `src/health/mod.rs`. Intention: [`flow/intentions/2026-07-14_health-cooldown-and-reject-notes-prompt.md`](flow/intentions/2026-07-14_health-cooldown-and-reject-notes-prompt.md).

The verifier prompt is also built **dynamically from prior REJECT notes** —
`prompt::collect_prior_reject_notes` gathers every REJECT verdict's notes across all prior
rounds of the goal and `prompt::append_prior_reject_notes` appends them under a
`# Prior rejection notes` heading so the verifier sees the rejection history and can
verify fixes against it.

## Run introspection + completion audit (2026-07-14)

Two read-only `jewilo` subcommands surface run statistics and verify a final completion
truly matches the requirement. Neither takes the goal lock or spawns verifiers.

- **`STATS <goalId>`** — aggregates EVERYTHING currently stored as JSON for a goal into one
  machine-readable JSON object to stdout: goal record, creation-time config snapshot (the
  authoritative n/m requirement), current round, per-round verdicts, completion (hash +
  matching verdicts), health (unhealthy-event count + cooldown flag), and durations
  (createdAt, matchedAt, wallClockSeconds). See `src/stats/mod.rs`. Intention:
  [`flow/intentions/2026-07-14_stats-and-audit-subcommands.md`](flow/intentions/2026-07-14_stats-and-audit-subcommands.md).

- **`AUDIT <goalId>`** — post-hoc verification that the final completion truly matches the
  creation-time config requirement. Reads the creation-time n/m from `goal.json` (the
  snapshot taken at `NEW`, NOT the current `config.json`), re-checks the matching APPROVE
  count reaches `n` of `m`, recomputes the completion hash from the stored inputs, and
  compares it to the stored `fullDigest`. Prints a JSON report
  `{ valid, requiredN, requiredM, matchingVerdicts, hashRecomputed, hashStored, checks }`;
  exits 0 if valid, non-zero otherwise. This is the "do these 2/X verdicts truly match the
  requirement recorded at creation time?" audit.

**Config snapshot at creation (single source of truth):** `goal::GoalRecord` carries a
`config: store::Config` field, written into `goal.json` at `NEW`. This is the authoritative
requirement — `config.json` may change later, but `AUDIT` always uses the creation-time
snapshot from `goal.json`.

## jewilo-dev deploy ceremony (for WIP verifier-loop changes)

When iterating on `jewilo` (the `verifier-loop` binary) itself, the WIP build is deployed
as **`jewilo-dev`** so it does NOT shadow the stable `jewilo` on PATH. The ceremony:

1. **Build + deploy WIP as `jewilo-dev`** — `cargo build` then symlink/copy the built
   `verifier-loop` binary to `jewilo-dev` (next to the stable `jewilo`).
2. **Run the normal verifier-loop workflow once with the stable `jewilo`** — confirms the
   stable path is unaffected by the WIP changes.
3. **Run the workflow again with `jewilo-dev`** — exercises the new WIP features end to
   end.

This keeps a known-good `jewilo` on PATH at all times; `jewilo-dev` is the canary.

## Fail-closed invariants (must always hold)

- NULL verdict never → APPROVE.
- Missing store → no hash.
- `goalText` edit → signature mismatch → hash mismatch.
- Verdict edit → hash mismatch.

## Security / threat model

The `add-verifier-tamper-hardening` change adds per-verifier Ed25519 signing keys
(`VERIFIER_LOOP_VERIFIER_SECRET`), pinned pubkeys, signed verdict records, and a
hash-chained receipt log. This is a **deterrent + detection layer**, not a prevention
guarantee: on a single host, a process with write access to `~/.verifier-loop/` AND the
ability to read a V\*'s env can still forge. True prevention requires out-of-process V\*
on a separate host. Read the full model before relying on the completion hash:
[`THREAT-MODEL.md`](THREAT-MODEL.md). (Specs: `openspec/changes/add-verifier-tamper-hardening/specs/`;
design + risks: `openspec/changes/add-verifier-tamper-hardening/design.md`.)

## `.jewilo-*` CWD bloat (standing reminder)

The `jewilo`/`jewije` CLIs do NOT produce `.jewilo-*` files in CWD — they write only to `store_dir` (`~/.verifier-loop/`). Any `.jewilo-*-goal.txt`, `-r<N>-fixnotes.txt`, `-scout-*.md`, `-final-rejection.md`, `-new.sh`, `-resume.sh` in a target repo are produced by the **outer driving agent**, not the binary. These are hygiene artifacts (legit verifier rejections, not tamper). Detail: [`flow/findings/2026-07-09_jewilo-bloat-files-openspec-dashboard.md`](flow/findings/2026-07-09_jewilo-bloat-files-openspec-dashboard.md). **ALWAYS remind the user about this bloat whenever `.jewilo-*` files reappear in any repo's working tree.**

## Branch consolidation (2026-07-09)

- 4 of 5 remote branches are DEAD (squash-merged via PRs #2, #3, #5, #6). Only `consolidate/all-work` has unique work. Audit + merge plan: [`flow/findings/2026-07-09_dead-branch-audit.md`](flow/findings/2026-07-09_dead-branch-audit.md), [`flow/plans/2026-07-09_consolidate-branches-merge-plan.md`](flow/plans/2026-07-09_consolidate-branches-merge-plan.md).

## Branch consolidation (2026-07-09)

5 remote branches assessed: 4 DEAD (squash-merged to main via PRs #2/#3/#5 + absorbed into consolidate), 1 LIVE (`consolidate/all-work`, ~85-90%, clean merge to main, blocked by dirty WT). Analysis + merge plan + dead-branch doc: [`flow/findings/2026-07-09_branch-consolidation-analysis.md`](flow/findings/2026-07-09_branch-consolidation-analysis.md).

## Out of scope (do NOT implement)

Deferred: OT1 audit subcommand, OT2 per-verifier maxTurn refresh, OT3 `chattr +a` hardening,
OT4 skill→wrapper, OT6 fan-out scouts. See design.md Non-Goals.
