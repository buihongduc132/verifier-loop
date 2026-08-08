## Context

Current design has a single `backend: String` field in `Config` (src/store/config.rs:46) that applies to ALL `m` verifiers. The spawn orchestrator (src/spawn/orchestrator.rs) resolves this single adapter and uses it for every verifier slot.

Real-world verification scenarios benefit from heterogeneous backends:
- Cross-model consensus (pi + hermes + acpx)
- Different tool capabilities per backend
- A/B testing adapter behavior

## Goals / Non-Goals

**Goals:**
- Per-verifier adapter configuration via `config.json`
- Backward compatible with existing `backend` field
- Spawn layer resolves adapter per verifier index
- Config validation ensures `verifiers.len() == m`

**Non-Goals:**
- Runtime adapter switching (adapter is fixed at goal creation time, captured in config snapshot)
- Per-verifier custom prompt templates (out of scope; prompt is shared across all verifiers)
- Adapter-specific timeout/model overrides (each adapter already has its own config; per-verifier overrides are a separate change)

## Decisions

### D1: Add `verifiers` array alongside `backend` field

**Decision**: Add optional `verifiers: Vec<VerifierConfig>` to `Config` struct. Each element has `adapter: String` and optional custom adapter fields (`spawn`, `resume`, `transport`).

**Rationale**: 
- Preserves backward compatibility (existing configs with `backend` still work)
- Minimal schema change (additive, not breaking)
- Clear precedence: `verifiers` > `backend` > default `pi`

**Alternatives considered**:
- Replace `backend` with `verifiers` only → breaking change, requires migration
- Add `backend_v2` field → confusing, two ways to do the same thing

### D2: Adapter resolution at spawn time, not config load time

**Decision**: `Config` stores the raw `verifiers` array. Adapter resolution happens in `resolve_adapter()` (src/bin/verifier_loop.rs:271) which returns `Vec<Adapter>` (one per verifier slot).

**Rationale**:
- Keeps `Config` simple (data, not logic)
- Adapter resolution is a one-time cost at spawn, not at config load
- Allows future extension (e.g., per-verifier env overrides) without changing `Config`

**Alternatives considered**:
- Resolve adapters at config load time → couples config parsing to adapter registry
- Store `Vec<Adapter>` in `Config` → breaks serde (Adapter has custom deserialization)

### D3: Validation: `verifiers.len() == m` when both present

**Decision**: When `verifiers` is present, its length MUST equal `m`. If both `verifiers` and `backend` are present, `verifiers` takes precedence and a warning is emitted.

**Rationale**:
- Fail-closed: mismatched lengths would cause index-out-of-bounds at spawn
- Warning on dual-specification helps users migrate from `backend` to `verifiers`
- No silent fallback: if `verifiers` is present but wrong length, hard error

**Alternatives considered**:
- Auto-pad `verifiers` to length `m` with default adapter → hides config errors
- Ignore `backend` silently when `verifiers` present → user confusion

### D4: Config snapshot captures resolved adapter per slot

**Decision**: When `goal.json` is created, the config snapshot includes the resolved adapter for each verifier slot (either from `verifiers[i]` or the global `backend`). This ensures the goal's config is self-contained and reproducible.

**Rationale**:
- Goal config snapshot must be immutable and self-describing
- Future resume rounds need to know which adapter was used for each slot
- Avoids ambiguity if `config.json` is modified after goal creation

**Alternatives considered**:
- Store only `verifiers` array in snapshot → loses resolution logic if config changes
- Store only `backend` field → loses per-verifier granularity

## Risks / Trade-offs

**[Risk] Config schema complexity** → Mitigation: `verifiers` is optional; existing configs work unchanged. Documentation emphasizes `backend` for simple cases, `verifiers` for advanced.

**[Risk] Adapter resolution logic duplication** → Mitigation: Centralize in `resolve_adapter()` function. Spawn layer calls once per round, not per verifier.

**[Risk] Config snapshot bloat** → Mitigation: Snapshot stores resolved adapter strings (e.g., `"pi"`, `"hermes"`), not full Adapter structs. Minimal overhead.

**[Trade-off] Backward compat vs. schema cleanliness** → Chose backward compat. Breaking change would force migration for all existing users. Dual-field approach is slightly messy but safe.

## Migration Plan

1. Add `verifiers` field to `Config` (optional, defaults to `None`)
2. Update `resolve_adapter()` to return `Vec<Adapter>` based on `verifiers` or `backend`
3. Update spawn layer to use per-verifier adapter
4. Update config snapshot in `goal.json` to include resolved adapters
5. Add validation: `verifiers.len() == m` when both present
6. Add warning when both `backend` and `verifiers` present
7. Update tests: config parsing, spawn with mixed backends
8. Update docs: `AGENTS.md`, `README.md` with examples

**Rollback**: Revert to single `backend` field. Existing configs with `verifiers` would fail validation (fail-closed). No data migration needed.

## Open Questions

**Q1**: Should per-verifier adapter support per-verifier `maxTurn` / `timeout` overrides?
- Current scope: no. Each adapter already has its own config. Per-verifier overrides are a separate change.

**Q2**: Should the config snapshot store the raw `verifiers` array or the resolved adapters?
- Decision: resolved adapters (strings). Rationale: self-contained, no ambiguity if config changes.
