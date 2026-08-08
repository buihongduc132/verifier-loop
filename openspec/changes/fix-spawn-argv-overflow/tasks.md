## 1. Adapter schema + config validation (RED)

- [ ] 1.1 Add `Transport` enum (`Stdin`, `GoalFile`) to `src/acp/adapters.rs` with serde (deny-unknown-fields, case-insensitive).
- [ ] 1.2 Add `transport: Transport` field to `Adapter`; default `Stdin` for built-ins; required for `Adapter::custom`.
- [ ] 1.3 Write failing test: `config.json` adapter with `spawn` containing `{prompt}` is rejected at load with a non-zero exit and a migration message. (RED)
- [ ] 1.4 Write failing test: built-in `pi` adapter defaults to `transport=Stdin`, `spawn="pi --mode json"`, `resume="pi --session {sid} --mode json"`. (RED)
- [ ] 1.5 Write failing test: custom adapter missing `transport` is rejected. (RED)

## 2. Adapter schema + config validation (GREEN)

- [ ] 2.1 Implement `transport` field, defaults, and `{prompt}` rejection in `Adapter::custom` / config loader.
- [ ] 2.2 Make §1 tests pass with minimal code.
- [ ] 2.3 `cargo llvm-cov --fail-under-lines 80` for `src/acp/adapters.rs`.

## 3. Backend stdin-readiness smoke (resolves OQ1, R3)

- [ ] 3.1 Manual probe: `printf '<prompt>' | pi --mode json` — confirm `pi` emits a `session` ACP event and processes the prompt. Record result in `flow/findings/<date>-pi-stdin-prompt.md`.
- [ ] 3.2 If `pi` does NOT read stdin: probe `pi --goal-file <file> --mode json` and `pi -p -`. Pick the working path; update design D6 + this tasks file before proceeding to §4.
- [ ] 3.3 Document the chosen pi invocation in `design.md` D6 and close OQ1.

## 4. Orchestrator: stdin transport (RED)

- [ ] 4.1 Write failing test: spawning with `transport=Stdin` sets child `stdin = Stdio::piped()` (assert via a stub `Command` builder or `assert_cmd` inspecting the spawned process). (RED)
- [ ] 4.2 Write failing test: the rendered prompt is written to the child's stdin and the child argv contains NO prompt-derived bytes. Use a tiny echo-binary child that reflects stdin + argv. (RED)
- [ ] 4.3 Write failing test: a 1 MiB prompt spawns successfully (no `E2BIG`). (RED)
- [ ] 4.4 Write failing test: `EPIPE` on stdin write after a verdict is registered is treated as non-fatal. (RED)
- [ ] 4.5 Write failing test: `EPIPE` on stdin write before any ACP event is fatal → verdict stays `null`. (RED)

## 5. Orchestrator: stdin transport (GREEN)

- [ ] 5.1 Refactor `build_spawn_command` to take `(template, transport)`; for `Stdin`, drop `cmd.arg(prompt)`, drop the literal `"` token handling, set `stdin=Stdio::piped()`.
- [ ] 5.2 Add a `tokio::spawn` background task that writes the full prompt to child stdin then closes it; surface `EPIPE` to gather logic.
- [ ] 5.3 Wire `EPIPE`-after-verdict = non-fatal, `EPIPE`-before-ACP = fatal (null verdict) in the gather path. Preserve fail-closed invariant.
- [ ] 5.4 Make §4 tests pass.
- [ ] 5.5 `cargo llvm-cov --fail-under-lines 80` for `src/spawn/orchestrator.rs`.

## 6. Orchestrator: goal-file transport (RED)

- [ ] 6.1 Write failing test: `transport=GoalFile` writes prompt to a tempfile under `temp_dir()`, substitutes `{goalFile}` with the absolute path, and unlinks after spawn. (RED)
- [ ] 6.2 Write failing test: on spawn failure, the tempfile is unlinked and the error propagates. (RED)
- [ ] 6.3 Write failing test: a 1 MiB prompt via goal-file spawns successfully. (RED)
- [ ] 6.4 Write failing test: stale `verifier-loop-*` tempfiles in `temp_dir()` are swept at startup (best-effort). (RED)

## 7. Orchestrator: goal-file transport (GREEN)

- [ ] 7.1 Implement `TempPromptFile` RAII guard (write + auto-unlink on drop) in `src/spawn/orchestrator.rs` (or new `src/spawn/tempfile.rs`).
- [ ] 7.2 Implement `{goalFile}` substitution in `build_spawn_command` for `GoalFile` transport; keep `stdin=Stdio::null()`.
- [ ] 7.3 Implement startup sweep of stale tempfiles (non-blocking, ignores errors).
- [ ] 7.4 Make §6 tests pass.
- [ ] 7.5 `cargo llvm-cov --fail-under-lines 80` for the new tempfile module.

## 8. Built-in adapter migration

- [ ] 8.1 Update built-in `pi`/`hermes`/`acpx` spawn/resume templates per design D6 (or the §3-resolved path).
- [ ] 8.2 Update existing adapter tests in `src/acp/adapters.rs` to assert new templates + transport.
- [ ] 8.3 Update the stale doc comment on `build_spawn_command` (currently falsely claims `sh -c` delegation).

## 9. Existing spawn-test updates

- [ ] 9.1 Audit `src/spawn/orchestrator.rs` tests (lines ~457+) that assert `{prompt}` is a single arg — update or remove per new contract.
- [ ] 9.2 Audit any e2e/integration test that inlines a prompt; switch to stdin/goal-file harness.

## 10. Coverage + fail-closed invariant gate

- [ ] 10.1 `cargo llvm-cov --fail-under-lines 80` across `src/spawn/`, `src/acp/`.
- [ ] 10.2 Add invariant test: NULL verdict never → APPROVE still holds after the transport change.
- [ ] 10.3 Add invariant test: `goalText` edit → signature mismatch → hash mismatch still holds.

## 11. End-to-end smoke + issue closure

- [ ] 11.1 e2e: `jewilo NEW "chunk1-rename"` (the exact gh #1 repro) spawns verifiers and reaches consensus.
- [ ] 11.2 e2e: a >128KB rendered prompt (large `verifierPromptFile`) spawns without `E2BIG`.
- [ ] 11.3 Comment on gh #1 and gh #4 with the fix summary and close them.
- [ ] 11.4 Update `flow/lesson_learn/` with the `E2BIG` root-cause + transport fix; reference from `AGENTS.md`.
