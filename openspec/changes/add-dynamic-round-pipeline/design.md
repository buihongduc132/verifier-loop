# Design — add-dynamic-round-pipeline

> Source of truth for LD traceability: `flow/findings/2026-07-20-dynamic-round-pipeline/2026-07-20-locked-decisions.yaml` (LD1–LD30).
> This file cites `[LDx]` per the immutable-input convention.

## D0 — Scope gate (this implementation)

**In scope:** LD1–LD30 implementation. The only deferred sub-feature is LD19's precedence
rule #1 (`per-slot verifiers[].adapter`) — that requires the `per-verifier-adapter` change
(separate OpenSpec proposal, NOT yet in `origin/main`). This implementation honors
precedence #2 (`dumpAdapter`/`smartAdapter`) → #3 (legacy `backend`). When
`per-verifier-adapter` merges, precedence #1 slots in above #2 with no schema change here.

**Out of scope:** per-slot adapter selection, OTel span-per-phase (existing tracing
already covers per-goal; phase is metadata), chattr hardening.

## D1 — Config schema (LD19, LD23, LD28, LD30)

Six new fields on `Config`. All `Option` or defaulted for backward compatibility.
`deny_unknown_fields` retained.

```rust
pub struct Config {
    // existing
    pub n: u32,
    pub m: u32,
    pub max_turn: u32,
    pub backend: String,            // legacy alias for dumpAdapter (precedence #3)
    pub git_diff_max_chars: u64,
    pub verifier_timeout_sec: u64,
    pub verifier_prompt_file: Option<String>,
    pub min_goal_chars: u64,
    pub file_edit_times_max_chars: u64,
    pub context_max_chars: u64,
    pub prompt_budget_bytes: u64,
    // NEW (dynamic-pipeline)
    pub dump_adapter: Option<String>,    // serde "dumpAdapter"   precedence #2
    pub smart_adapter: Option<String>,    // serde "smartAdapter"  precedence #2
    pub confirm_count: u32,               // serde "confirmCount"  default 1
    pub esca_threshold: u32,              // serde "escaThreshold" default 2 (LD30 full-word)
    pub esca_max_retries: u32,            // serde "escaMaxRetries" default 3 (LD21 oscillation cap)
}
```

**Precedence rule (LD19, narrowed to in-scope):**
1. ~~per-slot `verifiers[].adapter`~~ — out of scope (separate change).
2. `dumpAdapter` / `smartAdapter` — if either is set, it wins for its role.
3. legacy `backend` — alias for `dumpAdapter` when `dumpAdapter` is unset. `smartAdapter`
   defaults to `backend` too if unset (so a config with only `backend` behaves as today:
   all verifiers same adapter, Confirm phase still runs but with the same adapter —
   degenerate but valid).

**Ambiguity rejection:** if BOTH `backend` AND `dumpAdapter` are set → hard parse error
(LD19: "Reject config with both backend AND dumpAdapter present without verifiers[]").

**Validation at parse time (LD28, fail-closed):**
- `1 ≤ n ≤ m` (reject `n=0` vacuous-pass, reject `n>m` impossible)
- `m ≥ 1`
- `confirmCount ≥ 1`
- `escaThreshold ≥ 0` (`0` = disabled per LD21)
- `escaMaxRetries ≥ 0`
- `m < 2 ∧ escaThreshold > 0` → stderr warning, escaThreshold effectively ignored (LD15).
  NOT a hard error (forward-progress over block).

**Snapshot at NEW (LD23):** ALL six new fields frozen into `goal.json`. Live `config.json`
edits do NOT affect in-flight goals. This is the existing pattern (`n`/`m`/`backend`
already snapshot).

## D2 — Phase / Pipeline abstractions (LD11, turn3)

Rot-proof: `Phase` is the atomic unit. Gate/Confirm/Mixed/Final are NOT separate code
paths — they are constructed by `default_pipeline()` / `escalation_pipeline()`.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    pub id: PhaseId,              // "1a", "1b", ... (LD3, LD17)
    pub role: PhaseRole,          // Gate | Confirm | Mixed | Final (label only, not code path)
    pub dump_count: u32,
    pub smart_count: u32,
    pub threshold: u32,           // n for Gate; confirmCount for Confirm/Final; m for Mixed (LD22)
}

pub fn default_pipeline(cfg: &Config) -> Vec<Phase> {
    // PL-D: Gate(m D, n) → Confirm(confirmCount S, confirmCount)
    vec![
        Phase { id: "1a", role: Gate,   dump_count: cfg.m, smart_count: 0, threshold: cfg.n },
        Phase { id: "1b", role: Confirm, dump_count: 0,   smart_count: cfg.confirm_count, threshold: cfg.confirm_count },
    ]
}

pub fn escalation_pipeline(cfg: &Config) -> Vec<Phase> {
    // PL-E: Mixed(⌊m/2⌋ D + ⌈m/2⌉ S, m) → Final(confirmCount S, confirmCount)
    let mixed_dump = cfg.m / 2;
    let mixed_smart = cfg.m - mixed_dump; // == ceil(m/2)
    vec![
        Phase { id: "1a", role: Mixed, dump_count: mixed_dump, smart_count: mixed_smart, threshold: cfg.m },
        Phase { id: "1b", role: Final, dump_count: 0, smart_count: cfg.confirm_count, threshold: cfg.confirm_count },
    ]
}
```

**Mixed threshold = m (unanimity), NOT n (LD22).** All Mixed verifiers must APPROVE.

**The executor is phase-shape-agnostic:**
```rust
fn run_pipeline(phases: &[Phase], ...) -> PipelineResult {
    let mut matching = Vec::new();
    for phase in phases {
        let adapters = resolve_adapters(phase); // Vec<Adapter>, mixed for Mixed phase (LD20)
        let runs = spawn::spawn_round_with_adapters(adapters, phase.id, ...)?;
        let verdicts = gather(runs)?;
        let result = consensus::evaluate_phase(&verdicts, phase.threshold, phase.id)?;
        matching.extend(result.matching_verdicts);
        if !result.passed { return PipelineResult::Reject(result.rejection); }
    }
    PipelineResult::Approve(matching)
}
```

Adding a 5th phase type later = change a pipeline constructor, NOT the executor.

## D3 — phaseId axis (LD3, LD17, LD18, LD25)

`phaseId` is a first-class axis, NOT encoded in the round number. Round stays `u32`
(top-level RESUME counter). phaseId is `"1a"`, `"1b"`, ... WITHIN one invocation.

**Slot path:** `rounds/<round>/<phaseId>/<vid>/verdict.json`
**state.json:** gains `currentPhase: Option<String>` (None for legacy v0 goals).
**Env var:** `VERIFIER_LOOP_PHASE=<phaseId>` propagated to every V* child.
**Hash input:** `MatchingVerdict` gains `phaseId: String`. Sort key becomes
`(phaseId, verifierId)`. Two different phase orderings CANNOT produce the same hash
(LD25).

**collect_prior_reject_notes fix (OT10/LD18):** walks `phaseId`-ordered history within
`current_round`, not just `round < current_round`. Within one invocation, all sub-phases
share `current_round`, so the old filter excluded them. New logic:
- For each phaseId in canonical order ("1a" < "1b" < "1c" < "1d"),
- For each round ≤ current_round (inclusive when phaseId < current phase),
- Collect REJECT notes.

## D4 — d/s verifierIds (LD16, LD26)

**LD26 clarification (interpretation decision):** LD26's literal text ("Gate d1..d{floor(m/2)}")
contradicts LD10 ("Gate is generic m"). The sensible interpretation, consistent with LD16
("role-prefixed") + LD10 ("generic m") + LD26's "monotonic per-invocation" goal:

| Phase | Dump ids | Smart ids |
|-------|----------|-----------|
| Gate (m D) | d1..d_m | — |
| Confirm (k S) | — | s1..s_k |
| Mixed (⌊m/2⌋ D + ⌈m/2⌉ S) | d_{m+1}..d_{m+⌊m/2⌋} | s_{k+1}..s_{k+⌈m/2⌉} |
| Final (k S) | — | s_{k+⌈m/2⌉+1}..s_{k+⌈m/2⌉+k} |

where `k = confirmCount`, `m = config.m`. Monotonic per-invocation: dump counter and
smart counter each only increment within one invocation. No collisions.

**Migration (LD26):** `state.json` gains `verifierIdVersion: u8` (`0` = legacy `v{i+1}`,
`1` = d/s scheme). `verifier_id(idx, version, role, ...)` dispatches on version. Old goals
(v0) continue to use `v{i+1}` for their lifetime; new goals use d/s.

## D5 — esca counter lifecycle (LD4, LD21)

Stored in `state.json` as `escaCount: u32`. Persisted across RESUME.

```
Gate pass + Confirm reject  →  escaCount++
Confirm pass                →  escaCount = 0
Mixed pass + Final reject   →  escaCount = 0   (one PL-E cycle consumed)
Mixed reject                →  escaCount = 0
(escaCount never increments during PL-E — only PL-D invocations can increment)
```

**Activation:** at the START of an invocation, if `escaCount ≥ escaThreshold` AND
`escaThreshold > 0` AND `m ≥ 2` → run `escalation_pipeline()`. Else run
`default_pipeline()`.

**Oscillation cap (LD21):** `escalationDepth: u32` counts completed PL-E cycles. When
`escalationDepth ≥ escaMaxRetries` → hard-fail goal with non-zero exit + "escalation
exhaustion" message. No silent infinite loop.

**m < 2 freeze (LD15, LD21):** if `m < 2`, esca is FROZEN (never increments). Since config
is snapshotted at NEW, m is fixed for a goal's lifetime, so this is a config-validation
warning at NEW time only.

## D6 — Single-cmd executor + lock release (LD5, LD13, LD24)

ONE `jewilo NEW` / `RESUME` runs the entire pipeline. Sub-phase REJECT = hard reject for
that ONE invocation (LD13); outer agent then runs `RESUME N+1`.

**Lock release between phases (LD24):** GoalLock acquired at each phase's spawn-start,
released at that phase's gather-complete. Between phases, RECOVER/STATUS can probe.
Wall-clock ceiling documented: `Σ phase_timeout ≤ 4 × verifierTimeoutSec` (worst case
PL-E: Mixed + Final).

## D7 — Output format (LD6, LD27)

`<phase1approves>+<phase2approves>[+...]/<m>`. Denominator = m (LD27 — backward compat).
The `+` segments encode the pipeline path.

| Path | Output |
|------|--------|
| PL-D Gate reject (1/2 approve) | `1/2` |
| PL-D Gate pass, Confirm reject | `2+0/2` |
| PL-D Gate pass, Confirm pass | `2+1/2` |
| PL-E Mixed reject | `1+1/2` |
| PL-E Mixed pass, Final reject | `1+2+0/2` |
| PL-E Mixed pass, Final pass | `1+2+1/2` |

`--json` mode (extends existing `--json`): `{pipeline, phases:[{role,count,approved}], n, m, verdict}`.

## D8 — completion.json (LD14, LD25)

```json
{
  "completionHash": "240724-deadbeef",
  "fullDigest": "sha256...",
  "n": 2, "m": 2,
  "matchingVerdicts": [{"phaseId":"1a","verifierId":"d1",...}, ...],
  "matchedAt": "2026-07-24T...",
  "pipeline": "PL-D",          // NEW (LD14) — metadata, non-hash
  "escalationDepth": 0          // NEW (LD25) — count of PL-E cycles
}
```

Hash inputs (LD25): `salt + goalId + goalSignature + String(round) +
canonicalJSON(matchingVerdicts sorted by (phaseId, verifierId)) + matchedAtISO`.

## D9 — TDD plan (project hard constraint)

Per AGENTS.md: fresh RED author → different fresh GREEN author → coverage gate ≥80% lines
per new src file. Phases decomposed for parallel-safe RED authoring:

| Group | RED focus | GREEN focus |
|-------|-----------|-------------|
| G1 Config | parse 6 fields + validation + precedence + snapshot | `src/store/config.rs` |
| G2 Phase/Pipeline | constructors + Mixed formula + threshold rules | `src/pipeline/mod.rs` (NEW) |
| G3 phaseId paths | slot path + state.json + env var | `src/spawn/orchestrator.rs` + `src/goal/mod.rs` |
| G4 d/s ids | id scheme + version migration | `src/spawn/orchestrator.rs::verifier_id` |
| G5 hash phaseId | MatchingVerdict schema + sort key | `src/consensus/mod.rs` |
| G6 collect_prior_reject_notes | phaseId walk + within-round inclusion | `src/prompt/mod.rs` |
| G7 esca lifecycle | increment/reset rules + activation | `src/pipeline/esca.rs` (NEW) |
| G8 executor | pipeline runner + lock release + short-circuit | `src/pipeline/executor.rs` (NEW) |
| G9 output format | `<phases>/<m>` + `--json` pipeline shape | `src/bin/verifier_loop.rs` |
| G10 completion.json | pipeline + escalationDepth fields | `src/consensus/mod.rs` |
| E2E | full PL-D pass + PL-E activation | `tests/dynamic_pipeline_e2e.rs` |

## D10 — Risks (carried from explore)

- **R1 (LD26 typo):** the literal LD26 id scheme is internally inconsistent. Mitigation:
  D4 interpretation documented here; flagged in PR description for user review.
- **R2 (hash migration):** old receipts (no phaseId) must re-derive identically.
  Mitigation: `verifierIdVersion=0` path skips phaseId in hash input; v1 goals always
  have phaseId.
- **R3 (lock contention):** per-phase lock release widens the RECOVER window but also
  widens the race window between phases. Mitigation: phase boundary is a natural
  consistent point (no in-flight verdicts); RECOVER between phases sees a complete
  phase's verdicts on disk.
- **R4 (scope creep vs per-verifier-adapter):** LD19 precedence #1 is out of scope.
  Mitigation: precedence rule documented; #1 slots in later with no schema change.
