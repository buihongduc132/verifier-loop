## Why

Current design has a single `backend` field in `config.json` that applies to ALL verifiers. This forces homogeneous backend usage (e.g., all pi or all hermes). Real-world verification scenarios often benefit from heterogeneous backends — e.g., 1 pi verifier + 1 hermes verifier for cross-model consensus, or mixing different tool capabilities across backends.

## What Changes

- Add `verifiers` array to `config.json` with per-verifier adapter configuration
- Each verifier slot specifies its own `adapter` (e.g., `"pi"`, `"hermes"`, `"acpx"`, or custom)
- Backward compatible: existing `backend` field still works as shorthand for "all verifiers use same backend"
- Spawn orchestrator resolves per-verifier adapter instead of single global adapter
- Config validation: `verifiers.len()` must equal `m` (verifier count)

## Capabilities

### New Capabilities
- `per-verifier-adapter`: Per-verifier backend adapter configuration with backward-compatible `backend` field

### Modified Capabilities
- `goal-lifecycle`: Config schema extended with `verifiers` array; validation rules updated
- `verifier-spawn`: Spawn orchestrator uses per-verifier adapter instead of global backend

## Impact

- **Config schema**: `config.json` gains optional `verifiers` array; `backend` becomes optional (either `backend` OR `verifiers` required)
- **Spawn layer**: `spawn_round` and `spawn_resume` must resolve adapter per verifier index
- **Validation**: Config loader must validate `verifiers.len() == m` when both present
- **Tests**: New config parsing tests, spawn tests with mixed backends
- **Docs**: `AGENTS.md`, `README.md` updated with new config examples
