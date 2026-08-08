# add-dynamic-round-pipeline

## Why

`jewilo` today runs a single-phase consensus: spawn `m` verifiers of the SAME backend,
gather, count APPROVEs, hash on `n`/`m`. Every round is homogeneous — there is no notion
of a "dump" (broad, fast) verifier vs a "smart" (thorough) verifier, no confirmation step
after a gate passes, and no escalation when the gate keeps passing but the smart check
keeps vetoing.

The dynamic-round-pipeline explore (`flow/findings/2026-07-20-dynamic-round-pipeline/`)
locked **30 decisions (LD1–LD30)** across 5 turns + 2 resolution passes. All design
threads are resolved; the only open thread was **OT6** ("capture as OpenSpec proposal?"),
which the user deferred until now. This change captures OT6 and implements the pipeline.

The pipeline adds two execution paths driven by a single global config + a per-goal
runtime escalation counter:

```
PL-D (default):   Gate(m D, thr=n) → Confirm(confirmCount S, thr=confirmCount) → APPROVE
PL-E (escalated): Mixed(⌊m/2⌋ D + ⌈m/2⌉ S, thr=m) → Final(confirmCount S, thr=confirmCount) → APPROVE
```

After `escaThreshold` consecutive (Gate-pass ∧ Confirm-reject) cycles AND `m ≥ 2`, the
goal flips to PL-E for subsequent invocations. Output format encodes the path:
`<phaseApproves>+<phaseApproves>[+...]/<m>` (e.g. `2+1/2` for PL-D pass, `1+1+1/2` for
PL-E pass). The completion hash covers the union of ALL matching verdicts across all
sub-phases, with a `phaseId` axis added to bind phase order into the tamper-evident
receipt.

## What Changes

### New capabilities

- **`dynamic-pipeline`** — Phase/Pipeline abstractions (`Phase { dump_count,
  smart_count, threshold }`, `Vec<Phase>` executor), the PL-D/PL-E pipeline constructors
  derived from config, the `esca` escalation counter lifecycle, the `<phases>/<m>` output
  format, and the `pipeline`/`escalationDepth` completion.json metadata fields.

### Modified capabilities

- **`goal-lifecycle`** — `Config` gains 6 new fields (`dumpAdapter`, `smartAdapter`,
  `confirmCount`, `escaThreshold`, `escaMaxRetries`, plus validation). All pipeline
  fields are snapshotted into `goal.json` at NEW. `state.json` gains `escaCount` +
  `currentPhase` + `verifierIdVersion`. Config validation (LD28) rejects degenerate
  configs at parse time.
- **`verifier-spawn`** — Slot paths gain a `phaseId` axis
  (`rounds/<round>/<phaseId>/<vid>/`). `SpawnInput` carries `Vec<Adapter>` so the Mixed
  phase can mix dump + smart adapters in ONE spawn call. `VERIFIER_LOOP_PHASE` env var
  propagates phase to children. Verifier ids become role-prefixed + monotonic per
  invocation (`d1..`, `s1..`).
- **`consensus-check`** — `MatchingVerdict` gains `phaseId`. Hash inputs sort matching
  verdicts by `(phaseId, verifierId)`. Consensus evaluation runs once per phase; the
  pipeline executor short-circuits on reject.
- **`completion-proof`** — `completion.json` gains `pipeline: "PL-D"|"PL-E"` (metadata,
  non-hash) + `escalationDepth: u32`. Receipt log append is mandatory for EVERY verdict
  across ALL sub-phases.
- **`verifier-prompt`** — `collect_prior_reject_notes` walks `phaseId`-ordered history
  within `current_round` (not just `round < current_round`), so within-invocation
  sub-phases see earlier sub-phases' REJECT notes (OT10 fix).

## Impact

- **Config schema**: 6 new fields; `deny_unknown_fields` retained; legacy `backend`
  becomes alias for `dumpAdapter` (LD19 precedence). Config without `dumpAdapter` uses
  `backend`; both present + no `verifiers[]` → hard error (ambiguous).
- **Hash determinism**: `MatchingVerdict` schema gains `phaseId`. **Historical receipts
  re-derive identically** because phaseId is added to old receipts as the empty/legacy
  phase during audit recompute (migration via `verifierIdVersion`).
- **Output contract**: denominator stays `m` (LD27 — backward compat with existing
  `did not reach {}/{}` messages). The `+` segments are new but additive.
- **Tests**: new e2e test (`tests/dynamic_pipeline_e2e.rs`) covering PL-D pass, PL-D
  gate reject, PL-D confirm reject (esca++), PL-E activation, PL-E mixed pass + final
  pass, and the hash-covers-all-phases invariant.
- **Docs**: `AGENTS.md`, `README.md`, `USAGE.md` updated with pipeline config examples
  + output format table.

## Migration / compatibility

- `verifierIdVersion: 0` (legacy `v{i+1}`) for goals created before this change; `1`
  (`d/s` scheme) for new goals. `verifier_id()` checks the version. Old goals continue
  to RESUME under the v-namespace; new goals use d/s.
- Goals in flight at deploy time finish under the OLD single-phase pipeline
  (`pipeline: "PL-D"` with Confirm degenerated to no-op when `confirmCount` is unset →
  behaves exactly as today).
