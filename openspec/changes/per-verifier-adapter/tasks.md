## 1. Config Schema Extension

- [ ] 1.1 Add `VerifierConfig` struct to `src/store/config.rs` with fields: `adapter: String`, optional `spawn: String`, `resume: String`, `transport: Transport`
- [ ] 1.2 Add `verifiers: Option<Vec<VerifierConfig>>` field to `Config` struct with `#[serde(rename = "verifiers")]`
- [ ] 1.3 Add validation in `load_config_in`: when `verifiers` is present, verify `verifiers.len() == m`; hard error on mismatch
- [ ] 1.4 Add warning emission when both `backend` and `verifiers` are present (stderr, non-fatal)
- [ ] 1.5 Add unit tests: `verifiers` array parses correctly, length mismatch errors, `verifiers` takes precedence over `backend`, default when neither present

## 2. Adapter Resolution

- [ ] 2.1 Refactor `resolve_adapter()` in `src/bin/verifier_loop.rs` to return `Vec<Adapter>` instead of single `Adapter`
- [ ] 2.2 Implement per-verifier resolution: iterate `config.verifiers` (if present) or replicate `backend` adapter `m` times
- [ ] 2.3 Handle custom adapter construction per verifier slot (env override + `Adapter::custom`)
- [ ] 2.4 Add unit tests: mixed backend resolution, single backend replicated, custom adapter per slot

## 3. Spawn Layer Integration

- [ ] 3.1 Update `SpawnInput` struct in `src/spawn/orchestrator.rs` to carry `Vec<Adapter>` instead of single `Adapter`
- [ ] 3.2 Update `spawn_round` to use `input.adapters[i]` for verifier index `i` instead of single adapter
- [ ] 3.3 Update `spawn_resume` to use per-verifier adapter for each slot
- [ ] 3.4 Update `build_spawn_command` / `build_resume_command` calls to use per-verifier adapter
- [ ] 3.5 Add integration test: spawn round with mixed pi + hermes backends (mock commands)

## 4. Config Snapshot

- [ ] 4.1 Update goal creation in `src/goal/` to capture resolved per-verifier adapter configuration in config snapshot
- [ ] 4.2 Serialize resolved adapters as array of adapter keys (e.g., `["pi", "hermes"]`) in `goal.json`
- [ ] 4.3 Add test: goal.json config snapshot includes resolved adapters for each slot

## 5. Documentation

- [ ] 5.1 Update `AGENTS.md` with per-verifier adapter config examples
- [ ] 5.2 Update `README.md` with heterogeneous backend usage examples
- [ ] 5.3 Add config.json example showing `verifiers` array with mixed backends

## 6. Coverage Gate

- [ ] 6.1 Run `cargo llvm-cov --fail-under-lines 80` and ensure all new code meets coverage threshold
- [ ] 6.2 Add tests for edge cases: empty verifiers array, single-element verifiers, all-same adapters
