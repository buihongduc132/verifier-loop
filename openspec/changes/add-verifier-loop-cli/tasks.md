## 1. Project scaffolding (Rust)

- [ ] 1.1 Initialise Rust workspace in `../verifier-loop` (Cargo.toml, src/main.rs, src/lib.rs, tests/, binary targets)
- [ ] 1.2 Add two bin targets: `verifier-loop` (→ aliased `jewilo`), `verifier-verdict` (→ aliased `jewije`)
- [ ] 1.3 Add core crates: `serde` + `serde_json` (JSON), `uuid` (goalId), `sha2` (signature + hash), `clap` (CLI args), `tokio` (parallel spawn + timeout)
- [ ] 1.4 Add test tooling + coverage (`cargo-tarpaulin` or `cargo-llvm-cov`)
- [ ] 1.5 Add `.gitignore`, README stub, AGENTS.md pointing at `flow/explore/` and `flow/findings/` as design source

## 2. Salt and config store

- [ ] 2.1 Implement `~/.verifier-loop/.salt` creation (64 hex chars, mode 0600, first-run only) in `src/store/salt.rs`
- [ ] 2.2 Implement `config.json` loader with defaults (n=2, m=2, maxTurn=3, backend=pi, gitDiffMaxChars=10000, verifierTimeoutSec=1800) in `src/store/config.rs`
- [ ] 2.3 Tests: salt is created once, never printed, mode 0600; config defaults applied when file missing

## 3. Goal lifecycle (goal-lifecycle spec)

- [ ] 3.1 Implement `NEW "<goal>" [--context]` command: generate goalId (UUID), write immutable `goal.json`, write `signature.json = SHA256(salt + goalText + createdAt)`
- [ ] 3.2 Implement goal directory layout creation (`~/.verifier-loop/goals/<goalId>/rounds/`)
- [ ] 3.3 Implement `RESUME <goalId> [--fix]`: load goal, increment round, append to `rounds/<round>/fix-notes.json`, leave goal.json/signature.json untouched
- [ ] 3.4 Implement missing-store / missing-goal fail-closed errors (exit non-zero, no hash)
- [ ] 3.5 Tests: goal immutability (manual edit breaks signature recompute), RESUME preserves goal, fix-notes append-only, deleted store errors

## 4. ACP stream parser and adapters (verifier-spawn spec)

- [ ] 4.1 Implement shared ACP JSON stream parser in `src/acp/parser.rs` — model `AcpEvent` as an exhaustive `enum` (`Session{id}`, `AgentStart`, `TurnStart`, `MessageStart{message}`, `MessageEnd{message}`, `AgentEnd{messages, will_retry}`); exhaustive `match` so unhandled events are a compile error
- [ ] 4.2 Implement built-in pi adapter (spawn = `pi -p "<prompt>" --mode json`, resume = `pi --session <sid> -p "<prompt>" --mode json`)
- [ ] 4.3 Implement hermes and acpx adapters (spawn/resume templates)
- [ ] 4.4 Implement custom-adapter config path (spawn/resume templates + jsonFlag from config.json)
- [ ] 4.5 Tests: parser conformance per backend (fixture streams), SID extraction, final-output capture, custom template rendering

## 5. Verifier spawn orchestration (verifier-spawn spec)

- [ ] 5.1 Implement parallel spawn of m verifiers (concurrent `tokio::process::Command` + `tokio::select!`) in `src/spawn/orchestrator.rs`
- [ ] 5.2 Inject `VERIFIER_LOOP_GOAL_ID`, `VERIFIER_LOOP_VERIFIER_ID`, `VERIFIER_LOOP_ROUND` env per spawn
- [ ] 5.3 Pre-create each `rounds/<round>/<verifierId>/verdict.json` with `status: null` and write `meta.json` (sid, turnsUsed)
- [ ] 5.4 Implement per-verifier timeout (`verifierTimeoutSec`): kill process via `tokio::select!` + abort, leave null verdict
- [ ] 5.5 Implement gather barrier: wait for all agent_end or timeouts, then read verdicts
- [ ] 5.6 Tests: parallel launch order, env injection, timeout→null, gather waits for all

## 6. Session reuse (verifier-spawn spec)

- [ ] 6.1 On RESUME, compute per-verifier `turnsUsed < maxTurn`; reuse SID via adapter resume cmd, else fresh spawn with archived prior SID
- [ ] 6.2 Increment `VERIFIER_LOOP_ROUND` on reuse while keeping verifierId stable
- [ ] 6.3 Tests: reused session uses `--session`, exhausted session spawns fresh, prior SID archived

## 7. Verifier-verdict CLI (verdict-registration spec)

- [ ] 7.1 Implement `verifier-verdict approve` and `verifier-verdict reject --notes "..."` in `src/bin/verifier_verdict.rs` (clap subcommands)
- [ ] 7.2 Resolve goalId/verifierId/round from `VERIFIER_LOOP_*` env via `std::env::var` (env wins over any arg)
- [ ] 7.3 Atomic verdict write with first-write-wins: reject second attempt, exit non-zero
- [ ] 7.4 Reject without notes → exit non-zero, no write
- [ ] 7.5 Print "Verdict registered", exit 0 on success
- [ ] 7.6 Tests: approve/reject writes, first-final semantics, notes-required, env-derived slot

## 8. Consensus and completion (consensus-check + completion-proof specs)

- [ ] 8.1 Implement n/m counter after gather (APPROVE count >= n)
- [ ] 8.2 On pass: compute short hash `completionHash = mmddyy + "-" + first8hex(SHA256(salt + goalId + goalSignature + String(roundNumber) + JSON.stringify(matchingVerdicts sorted by verifierId) + matchedAtISO))` where `mmddyy` = UTC date of matchedAt. Also compute `fullDigest = full SHA256(same inputs)` for exact audit recompute.
- [ ] 8.3 Write `completion.json` (hash, goalId, roundNumber, matchedAt, matchingVerdicts) and print hash
- [ ] 8.4 On fail: print rejection (REJECT notes + null markers) to A, exit non-zero
- [ ] 8.5 Tests: 2/2 pass, 2/3 pass, below-threshold fail, hash determinism, tamper invalidation (goalText edit, verdict edit)

## 9. Verifier prompt rendering (verifier-prompt spec)

- [ ] 9.1 Implement template engine with variables: goalId, verifierId, round, prevRound, goalText, context, fixNotes, prevNotes, cwd, gitStatus, fileEditTimes, gitDiff, gitDiffMaxChars, `{{process.env.*}}`
- [ ] 9.2 Capture frozen artifact snapshot at spawn (cwd, `git status --porcelain`, file edit times, `git diff` truncated to gitDiffMaxChars)
- [ ] 9.3 Bake in default round-1 and resume templates (embed verifier policy text from the pi-plugins verifier-loop skill)
- [ ] 9.4 Render and persist `initial-prompt.txt` per verifier before spawn
- [ ] 9.5 Enforce blindness: round number, other verdicts, n/m, hash never appear unless via opted-in template var
- [ ] 9.6 Tests: truncation, snapshot consistency within round, blindness invariants, env interpolation, null-template default

## 10. CLI wiring and end-to-end

- [ ] 10.1 Wire `verifier-loop NEW`/`RESUME` and `verifier-verdict approve`/`reject` bin targets in `src/bin/`
- [ ] 10.2 End-to-end smoke: `cargo run --bin verifier-loop -- NEW "say hi works"` with n=m=1 against `pi --mode json`; assert hash produced and goal dir populated
- [ ] 10.3 End-to-end reject→resume→pass flow against a stub backend
- [ ] 10.4 Install symlinks/aliases `jewilo` and `jewije` (post `cargo install --path .`)
- [ ] 10.5 Coverage gate: >= 80% lines on all new source files (`cargo tarpaulin` or `cargo llvm-cov`)

## 11. Documentation

- [ ] 11.1 README: install (`cargo install --path .`), config.json reference, usage examples, hash formula, fail-closed guarantees
- [ ] 11.2 USAGE.md: NEW/RESUME/verifier-verdict invocations and env vars
- [ ] 11.3 Cross-link from pi-plugins `flow/findings/verifier-loop-cli/README.md` to this repo once implemented
