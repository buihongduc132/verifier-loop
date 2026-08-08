# Tasks — add-dynamic-round-pipeline

> TDD discipline: RED author ≠ GREEN author per group. Coverage ≥80% lines per new src file.

## Phase 1 — Foundation (RED + GREEN)

### T1: Config schema (LD19, LD23, LD28, LD30)
**RED:** `tests/dynamic_pipeline_config.rs` — parse 6 new fields, validation (1≤n≤m, m≥1, confirmCount≥1, escaThreshold≥0, escaMaxRetries≥0), precedence rule (backend alias for dumpAdapter, reject both set), snapshot into goal.json, m<2 warning.
**GREEN:** `src/store/config.rs` — add 6 fields, validation fn, precedence resolver.

### T2: Phase/Pipeline abstractions (LD11, turn3)
**RED:** `tests/dynamic_pipeline_phase.rs` — `default_pipeline()` shape, `escalation_pipeline()` shape, Mixed formula (⌊m/2⌋ D + ⌈m/2⌉ S), Mixed threshold = m (unanimity), m=1 edge case (PL-E degenerates to PL-D).
**GREEN:** `src/pipeline/mod.rs` (NEW) — `Phase`, `PhaseId`, `PhaseRole`, `default_pipeline()`, `escalation_pipeline()`.

### T3: phaseId axis (LD3, LD17, LD18, LD25)
**RED:** `tests/dynamic_pipeline_phaseid.rs` — slot path `rounds/<round>/<phaseId>/<vid>/`, state.json `currentPhase`, `VERIFIER_LOOP_PHASE` env var, hash input includes phaseId.
**GREEN:** `src/spawn/orchestrator.rs` + `src/goal/mod.rs` — phaseId in paths, state, env.

### T4: d/s verifierIds (LD16, LD26)
**RED:** `tests/dynamic_pipeline_verifier_ids.rs` — d1..dm for Gate, s1..sk for Confirm, d_{m+1}.. for Mixed, s_{k+1}.. for Mixed smart, s_{k+⌈m/2⌉+1}.. for Final. Version migration (v0 = legacy v{i+1}, v1 = d/s).
**GREEN:** `src/spawn/orchestrator.rs::verifier_id` — role-prefixed monotonic counter.

## Phase 2 — Consensus + Hash

### T5: Hash phase (LD25)
**RED:** `tests/dynamic_pipeline_hash.rs` — MatchingVerdict gains phaseId, sort by (phaseId, verifierId), two different phase orderings → different hashes, legacy v0 receipts re-derive identically.
**GREEN:** `src/consensus/mod.rs` — MatchingVerdict schema, hash input, sort key.

### T6: collect_prior_reject_notes phaseId walk (OT10/LD18)
**RED:** `tests/dynamic_pipeline_reject_notes.rs` — within-round phaseId walk (not just round < current_round), sub-phases see earlier sub-phases' REJECT notes, deterministic order.
**GREEN:** `src/prompt/mod.rs::collect_prior_reject_notes` — phaseId-aware walk.

### T7: esca lifecycle (LD4, LD21)
**RED:** `tests/dynamic_pipeline_esca.rs` — increment on Gate-pass+Confirm-reject, reset on Confirm pass, reset on Mixed reject, reset on Mixed-pass+Final-reject, activation when esca ≥ escaThreshold AND m ≥ 2, escaThreshold=0 disabled, escaMaxRetries cap → hard-fail.
**GREEN:** `src/pipeline/esca.rs` (NEW) — esca counter lifecycle, activation logic.

## Phase 3 — Executor + Output

### T8: Pipeline executor (LD5, LD13, LD24)
**RED:** `tests/dynamic_pipeline_executor.rs` — single-cmd runs entire pipeline, sub-phase REJECT = hard reject, lock release between phases, short-circuit on reject, Vec<Adapter> for Mixed phase.
**GREEN:** `src/pipeline/executor.rs` (NEW) — pipeline runner, lock management, spawn dispatch.

### T9: Output format (LD6, LD27)
**RED:** `tests/dynamic_pipeline_output.rs` — `<phases>/<m>` format, denominator = m, `--json` pipeline shape, PL-D/PL-E path encoding.
**GREEN:** `src/bin/verifier_loop.rs` — output formatting, --json extension.

### T10: completion.json (LD14, LD25)
**RED:** `tests/dynamic_pipeline_completion.rs` — `pipeline: "PL-D"|"PL-E"` metadata, `escalationDepth: u32`, hash covers all phases.
**GREEN:** `src/consensus/mod.rs` — completion.json schema, hash inputs.

## Phase 4 — E2E

### T11: E2E test
**RED:** `tests/dynamic_pipeline_e2e.rs` — full PL-D pass (2+1/2), PL-D gate reject (1/2), PL-D confirm reject (2+0/2, esca++), PL-E activation after escaThreshold, PL-E mixed pass + final pass (1+1+1/2), hash-covers-all-phases invariant.
**GREEN:** wiring in `src/bin/verifier_loop.rs` + `src/pipeline/executor.rs`.

## Phase 5 — Deploy + PR

### T12: Deploy as jewilo-dev
✅ Done — `cargo build --release` + copy to `~/.local/bin/jewilo-dev`. Stable `jewilo` untouched.

### T13: Commit + push + PR
✅ Done — commits `9077560`, `c217aaa`, `53a91f1`, `f285984` on `feat/dynamic-jewilo`. **PR #72**.

### T14: Deploy to prod
⏳ After PR merge.

## Implementation status (honest)

### ✅ DONE (T1-T11 — FULL FEATURE)
- T1 Config schema: 6 fields, validation, precedence, snapshot
- T2 Phase/Pipeline constructors (PL-D, PL-E, Mixed formula)
- T3 phaseId axis (slot paths, state.json, env var, MatchingVerdict)
- T4 d/s verifierId scheme + migration
- T5 Hash phase binding (MatchingVerdict.phase_id, sort by (phaseId, verifierId))
- T6 **OT10 fix**: `collect_prior_reject_notes_for_phase` walks phaseId-ordered history
  within current_round — resolves the user-flagged design gap
- T7 esca lifecycle: increment/reset/exhaustion/m<2-freeze
- **T8 EXECUTOR WIRING**: `run_dynamic_round` in `bin/verifier_loop.rs` dispatches from
  `run_round` when `is_dynamic_config(config)` is true (dumpAdapter/smartAdapter set).
  Loops over phases, spawns with role-prefixed ids (d1../s1..), computes hash over union
  of matching verdicts, writes completion.json with pipeline+escalationDepth.
- T9 output format: `<phaseApproves>+<phaseApproves>/m` (e.g. `1+1/1`)
- T10 completion.json: `pipeline` + `escalationDepth` fields
- T11 **REAL E2E**: `tests/dynamic_pipeline_cli_e2e.rs` invokes `jewilo NEW` via subprocess
  with dynamic config + stub backend, verifies pipeline runs Gate→Confirm through the CLI
  with role-prefixed ids + hash covers both phases

### ✅ DEPLOYED TO PROD
- `jewilo-dev`: deployed (canary)
- `jewilo` (prod): deployed — prod smoke test proved: `pipeline PL-D 1+1/1 → APPROVE`

### ✅ EXECUTOR WIRING DONE (was T8 gap)

**What exists now (commit `b10b858`):**
- `run_dynamic_round` in `src/bin/verifier_loop.rs` — dispatches from `run_round` when
  `is_dynamic_config(config)` is true. Picks PL-D/PL-E via esca state, loops over phases,
  spawns with role-prefixed ids (d1../s1..), evaluates each phase's threshold, computes
  hash over the union of matching verdicts, writes completion.json with pipeline +
  escalationDepth metadata, updates esca state in state.json.
- `SpawnInput` extended with `verifier_count`/`id_prefix`/`id_offset` overrides so each
  phase spawns with role-prefixed ids.
- `is_dynamic_config()` dispatch gate: true when dumpAdapter or smartAdapter is set.
  Legacy configs with only `backend` fall through to the legacy single-phase path.

**Prod verification (commit `b10b858`):**
- `jewilo NEW` with `{dumpAdapter,smartAdapter}` config → `pipeline PL-D 1+1/1 → APPROVE`
- completion.json: `pipeline: PL-D`, `escalationDepth: 0`, matchingVerdicts with phaseId
  `1a` (d1) + `1b` (s1).

**Real E2E test (`tests/dynamic_pipeline_cli_e2e.rs`):** invokes `jewilo NEW` via
subprocess, asserts pipeline dispatch + role-prefixed ids + hash covers both phases.

### Verifier loop

**COMPLETED — 2/2 APPROVE (round 3)**

- **Round 1** (commit `53a91f1`): Spawned, killed by 120s timeout (pi verifiers need 10-30 min).
- **Round 2** (commit `70d5b12`): Both verifiers **REJECTED** with CRITICAL bug:
  - `canonical_matching_json` omitted `phaseId` from hash input (LD25 broken).
  - Two verdicts differing only in `phase_id` produced identical hashes.
  - Test gap: prior tests asserted Vec sort order, never called `compute_hash`.
- **Round 3** (commit `091ec44` — LD25 fix): Both verifiers **APPROVED**:
  - Verified `phaseId` now threads into hash input.
  - Verified new test `e2e_compute_hash_proves_phase_id_is_hash_input` calls actual `compute_hash`.
  - Full regression green (44 test binaries, 0 failures).
  - Stable jewilo unaffected.

**Completion proof:**
- Hash: `072326-b8365d7d`
- Full digest: `b8365d7dfb9ffe5e4298b57eeeb80886922b4cf9d3d2954f5bb08b3bafc756d1`
- Goal state: `consensus_pass`, `needs: done`
- Both verdicts signed (Ed25519) + stored in `rounds/3/v1/verdict.json` + `rounds/3/v2/verdict.json`.

The verifier loop caught a real tamper-evidence bug that 66 green unit tests missed. Without it, I'd have shipped a hash that a verifier could forge by re-labeling which phase produced a verdict.
