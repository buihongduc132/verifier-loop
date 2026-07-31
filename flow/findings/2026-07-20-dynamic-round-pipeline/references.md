# References

> Sources consulted during the dynamic-round-pipeline explore session (2026-07-20).

## Source files (codebase)

### OpenSpec specs & proposals

- `openspec/changes/add-verifier-loop-cli/specs/consensus-check/spec.md` — contains the "Consensus is static and human-configured" requirement that DIRECTLY CONFLICTS with the dynamic-pipeline proposal. Load-bearing for the spec-delta argument.
- `openspec/changes/add-verifier-loop-cli/specs/verifier-spawn/spec.md` — spawn parallelism, session reuse, per-verifier identity env vars, timeout→null. Confirms sub-phases can reuse existing spawn machinery.
- `openspec/changes/per-verifier-adapter/specs/*/spec.md` — per-verifier adapter array (`verifiers[]` with `adapter` field). Confirms heterogeneous backends per slot already supported → D and S adapters slot in cleanly.
- `openspec/changes/per-verifier-adapter/proposal.md` — rationale for per-verifier backends; pattern to follow for D/S config.

### Rust source

- `src/consensus/mod.rs` — `ConsensusResult`, `MatchingVerdict`, hash formula (`inputs = salt + goalId + goalSignature + String(roundNumber) + canonicalJSON(matchingVerdicts) + matchedAtISO`), `Rejection`. Hash machinery is phase-agnostic — covers any set of matching verdicts. Load-bearing for LD7 (hash covers chain).
- `src/goal/mod.rs` — `GoalRecord`, `StateRecord { current_round }`, `new`/`resume`/`current_round`/`load`/`verify_signature`/`compute_signature`. `state.json` is where esca counter would live alongside `current_round`.
- `src/spawn/mod.rs` + `src/spawn/orchestrator.rs` — `SpawnInput`, `VerifierRun`, `VerifierMeta`, `spawn_round`, `spawn_resume`, per-slot meta, archive_prior_sid. Confirms sub-phases = just more spawn calls with same API.
- `src/cli/mod.rs` — `VerifierLoopCmd::{New, Resume, Recover, Status}`. LD5 (single cmd) means no new subcommand; pipeline runs inside New/Resume.
- `src/store/config.rs` — `Config { n, m, maxTurn, backend, verifier_timeout_sec, ... }` with defaults. Schema to extend with `dumpAdapter`, `smartAdapter`, `confirmCount`, `escaT`.
- `src/store/mod.rs` — config.json location + load path.
- `src/acp/adapters.rs` — built-in adapters (`pi`, `hermes`, `acpx`) + custom. D/S resolve to adapter keys here.
- `src/bin/verifier_loop.rs` — `run_new`, `run_resume`, `run_round` (the orchestration fn that would host the phase loop). Shows current single-round flow + cooldown check + prompt render + spawn + gather + consensus + completion write.
- `src/lib.rs` — module map; confirms `consensus`, `goal`, `spawn`, `verdict`, `store`, `acp`, `health`, `round_recover`, `observe` modules.

## Documents

- `AGENTS.md` (repo root) — module map, fail-closed invariants, TDD discipline, coverage gate `>=80%`. The "Consensus is static" spec note + completion-proof spec are the relevant anchors.
- `~/.pi/agent/cmd-family/ospx.yml` — ospx family step manifest (referenced by explore prompt; not present in repo — flagged as CA in turn 1).

## Code patterns

- **Per-verifier adapter resolution** (`src/spawn/orchestrator.rs` + per-verifier-adapter spec) — `verifiers[i].adapter` pattern. D/S would extend this: each slot's adapter derived from its role in the current phase.
- **Round lifecycle** (`src/goal/mod.rs` + `src/bin/verifier_loop.rs::run_round`) — spawn → gather → consensus → completion/rejection. Sub-phases reuse this exact lifecycle (LD2).
- **Hash over sorted matching verdicts** (`src/consensus/mod.rs`) — `canonicalJSON(matchingVerdicts sorted by verifierId)`. Extends naturally to multi-phase chain: union of all phases' matching verdicts (LD7).
- **Health cooldown** (`src/health/` + `run_round` cooldown check) — affects pipeline: dumpers crashing trips cooldown, which blocks the smart phase too. Open interaction (mentioned in turn 1 offer (d)).
- **state.json round counter** (`src/goal/mod.rs::StateRecord`) — `current_round: u32`. esca counter would join this struct.
