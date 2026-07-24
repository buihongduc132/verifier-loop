# GREEN Subagent Task — Dynamic Round Pipeline Foundation (T1-T4)

You are the GREEN author for the dynamic-round-pipeline foundation phase (T1-T4).

**Working directory:** `.worktrees/wt-dynamic-jewilo/` (already created, clean off origin/main).

**Your job:** Implement the MINIMAL code in `src/` to make the RED tests (T1-T4) pass. Do NOT modify the test files. Do NOT add extra features. Do NOT refactor existing code unless necessary to make tests pass.

**Design source:** Read `openspec/changes/add-dynamic-round-pipeline/design.md` (D1-D4) + `flow/findings/2026-07-20-dynamic-round-pipeline/2026-07-20-locked-decisions.yaml` (LD19, LD23, LD28, LD30 for config; LD11, turn3 for phase; LD3, LD17, LD18, LD25 for phaseId; LD16, LD26 for verifierIds).

**T1 — Config (src/store/config.rs):**
- Add 6 new fields to `Config`: `dump_adapter: Option<String>`, `smart_adapter: Option<String>`, `confirm_count: u32`, `esca_threshold: u32`, `esca_max_retries: u32`.
- Serde rename: `dumpAdapter`, `smartAdapter`, `confirmCount`, `escaThreshold`, `escaMaxRetries`.
- Defaults: `confirm_count = 1`, `esca_threshold = 2`, `esca_max_retries = 3`, others `None`.
- Validation (LD28): `1 ≤ n ≤ m`, `m ≥ 1`, `confirm_count ≥ 1`, `esca_threshold ≥ 0`, `esca_max_retries ≥ 0`.
- Precedence (LD19): if BOTH `backend` AND `dump_adapter` are set → hard parse error.
- Snapshot at NEW (LD23): all 6 fields frozen into `goal.json` (extend existing snapshot logic).
- m<2 ∧ esca_threshold>0 → stderr warning (LD15).

**T2 — Phase/Pipeline (src/pipeline/mod.rs — NEW):**
- `Phase` struct: `id: String`, `role: PhaseRole`, `dump_count: u32`, `smart_count: u32`, `threshold: u32`.
- `PhaseRole` enum: `Gate`, `Confirm`, `Mixed`, `Final`.
- `default_pipeline(cfg: &Config) -> Vec<Phase>`: `[Gate(m D, n), Confirm(confirmCount S, confirmCount)]`.
- `escalation_pipeline(cfg: &Config) -> Vec<Phase>`: `[Mixed(⌊m/2⌋ D + ⌈m/2⌉ S, m), Final(confirmCount S, confirmCount)]`.
- Mixed threshold = m (unanimity, LD22).
- m=1 edge case: PL-E degenerates (⌊1/2⌋=0 D, ⌈1/2⌉=1 S → Mixed ≡ Confirm).

**T3 — phaseId axis (src/spawn/orchestrator.rs + src/goal/mod.rs):**
- Slot path: `rounds/<round>/<phaseId>/<vid>/verdict.json`.
- state.json gains `currentPhase: Option<String>`.
- Env var `VERIFIER_LOOP_PHASE=<phaseId>` propagated to children.
- Hash input includes phaseId (LD25).

**T4 — d/s verifierIds (src/spawn/orchestrator.rs::verifier_id):**
- Gate (m D): d1..d_m.
- Confirm (k S): s1..s_k.
- Mixed (⌊m/2⌋ D + ⌈m/2⌉ S): d_{m+1}..d_{m+⌊m/2⌋}, s_{k+1}..s_{k+⌈m/2⌉}.
- Final (k S): s_{k+⌈m/2⌉+1}..s_{k+⌈m/2⌉+k}.
- Version migration: v0 = legacy v{i+1}, v1 = d/s scheme.

**Constraints:**
- Implement ONLY what the tests require. No extra features.
- Do NOT modify the test files (tests/dynamic_pipeline_*.rs).
- Run `cargo test --test dynamic_pipeline_config --test dynamic_pipeline_phase --test dynamic_pipeline_phaseid --test dynamic_pipeline_verifier_ids` and confirm ALL tests pass.
- Report: which tests pass, any failures, any compilation errors.

**Output:** Modify only `src/` files. Do NOT modify `tests/` files.
