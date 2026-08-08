## 1. Config schema (TDD: RED first, then GREEN)

- [ ] 1.1 RED — Write failing test in `src/store/config.rs` tests asserting `cwd` is now a recognized canonical field: `{"cwd": "/abs/path"}` parses to `Config { cwd: Some("/abs/path") }`, and `"cwd": null` parses to `cwd: None`. (Inverts the existing "unknown `cwd` key must be rejected" test.)
- [ ] 1.2 RED — Write failing regression test asserting every other unknown key is still a hard parse error (`deny_unknown_fields` remains intact).
- [ ] 1.3 GREEN — Add `cwd: Option<PathBuf>` to `Config` struct with `#[serde(default)]` behavior; admit `cwd` into the `deny_unknown_fields` key set; remove `cwd` from the rejected-keys test. Keep all other unknown keys as hard errors.
- [ ] 1.4 GREEN — Update `Config::Default` impl (`src/store/config.rs:94-100`) to list the new `cwd: None` field explicitly.
- [ ] 1.5 GREEN — Update `config_round_trips_through_serde_json_camel_case` test (`src/store/config.rs:156`) to include `cwd` fields (both `Some` and `None`).
- [ ] 1.6 GREEN — Update stale docstrings in `src/store/config.rs` that claim `cwd` is rejected and runtime-only (lines 13, 32-36, and 185-186). The "eight canonical fields" count becomes nine.

## 2. Resolve cwd + CLI flag (TDD)

- [ ] 2.1 RED — Write failing test for a new helper `resolve_cwd(flag: Option<PathBuf>, config: &Config, runtime: &Path) -> Result<PathBuf, String>`: flag wins over config, config wins over runtime, all three absent → runtime; bad path (non-existent / not-a-dir) → `Err` with a clear message. Also assert a relative value is resolved relative to `runtime` then canonicalized.
- [ ] 2.2 GREEN — Implement `resolve_cwd` in a new lib module `src/cli/cwd.rs` (or `src/util/cwd.rs`) so it is unit-testable. Canonicalize the winner with `std::fs::canonicalize`; a relative path is resolved relative to the `runtime` parameter. Return a descriptive error if canonicalization fails.
- [ ] 2.3 GREEN — Add `--cwd <PATH>` clap arg to `VerifierLoopCmd::New` in `src/cli/mod.rs:17-28`. Wire `resolve_cwd` inside `run_new` (`src/bin/verifier_loop.rs`). RESUME does NOT get this arg.
- [ ] 2.4 RED — Write failing test for `run_new` error path: when `--cwd` points to a non-git directory, NEW exits before creating the goal directory or writing `signature.json`.
- [ ] 2.5 GREEN — Implement a lightweight pre-goal validation in `run_new`: call a new public helper `validate_cwd_is_git_repo(cwd: &Path) -> Result<(), String>` located in `src/util/cwd.rs` (same module as `resolve_cwd` from task 2.2) — checks existence + is-dir + `git rev-parse --is-inside-work-tree`. Do NOT run a full `capture_snapshot` here; the full snapshot is captured once inside `run_round`. Note: there is an existing private `git_check` in `src/prompt/mod.rs:346`; `validate_cwd_is_git_repo` is a thin public wrapper for the bin entry so `run_new` does not need a prompt-module dependency for pre-goal validation.

## 3. GoalRecord: freeze resolved cwd at NEW

- [ ] 3.1 RED — Write failing test in `src/goal/mod.rs` tests: `goal::new` accepts a resolved cwd and writes it into `GoalRecord.resolvedCwd`.
- [ ] 3.2 GREEN — Add `resolved_cwd: Option<PathBuf>` to `GoalRecord` (camelCase `resolvedCwd`, `skip_serializing_if = Option::is_none`). Update `goal::new` signature to take `resolved_cwd: Option<&Path>` and store it. Then update ALL 26 existing call sites (authoritative, grep-verified): `tests/goal_lifecycle.rs:21,43,64,89,124,162,173,183,191`, `tests/spawn_goal_file_transport.rs:69`, `tests/prompt_reject_notes.rs:28`, `src/bin/verifier_loop.rs:94`, `tests/round_recover.rs:36`, `tests/receipt.rs:45`, `tests/round_recover_cli.rs:261`, `tests/session_reuse.rs:41`, `tests/compaction_recovery.rs:93`, `tests/verdict.rs:64,393,447,815,1131`, `tests/spawn_orchestrator.rs:57`, `tests/spawn_stdin_transport.rs:49`, `tests/wiring.rs:91`, `tests/consensus.rs:123`, and `src/verdict/mod.rs:900`. Pass `None` where a specific cwd is not needed (legacy-equivalent). Re-run `rg -n 'goal::new\(' tests/ src/` before claiming 3.2 done to confirm zero missed sites.
- [ ] 3.3 RED — Write failing test: `GoalRecord` round-trips with `resolvedCwd` both present and absent; verify the `config` snapshot is still verbatim (not mutated to the resolved value).
- [ ] 3.4 GREEN — Ensure `goal::new` stores `config` unchanged and `resolved_cwd` separately.

## 4. Snapshot capture uses the threaded cwd

- [ ] 4.1 RED — Write failing test: `run_round` no longer calls `std::env::current_dir()` internally; it accepts a `cwd: &Path` parameter and passes it to `capture_snapshot_with`.
- [ ] 4.2 GREEN — Change `run_round` signature to `fn run_round(root, config, goal_id, round, fix_notes, kind, prepend, cwd: &Path)`. Remove the `let cwd = std::env::current_dir()?` line. Pass `cwd` to `capture_snapshot_with`.
- [ ] 4.3 GREEN — Update `run_new` to pass the resolved cwd into `run_round` and `goal::new`.
- [ ] 4.4 GREEN — Update `run_resume` to load the goal record, extract `resolvedCwd` (fallback to `std::env::current_dir()` if missing), and pass it to `run_round`.
- [ ] 4.5 RED — Write failing test: `capture_snapshot` with a resolved cwd renders `{{cwd}}` as that path, and git commands run in that directory (use a tempdir git repo, assert the snapshot reflects the tempdir, not the test process CWD).
- [ ] 4.6 GREEN — Verify `capture_snapshot_with` already takes `cwd: &Path`; confirm no internal `std::env::current_dir()` call exists in the snapshot path. Update the stale docstring in `src/prompt/mod.rs:275` that says `--show-toplevel` to match the actual code: `--is-inside-work-tree`.

## 5. Spawn orchestrator sets child current_dir everywhere

- [ ] 5.1 RED — Write failing test: a spawned verifier child's working directory equals the resolved cwd, not the parent's CWD. Use a stub backend that prints its CWD and assert the captured output contains the resolved path.
- [ ] 5.2 GREEN — Add `cwd: &'a Path` to `SpawnInput` (`src/spawn/orchestrator.rs:107`). Then update all existing construction sites that build `SpawnInput` (list: `tests/spawn_orchestrator.rs`, `tests/spawn_stdin_transport.rs`, `tests/spawn_goal_file_transport.rs`, `tests/session_reuse.rs`, `tests/compaction_recovery.rs`, and `src/bin/verifier_loop.rs:412`). Pass `std::env::current_dir()` or the test tempdir CWD as appropriate.
- [ ] 5.3 GREEN — Thread `cwd` through `spawn_round` and into `spawn_nudge_child` (and any other child-spawn functions). Update `build_command_from_template` to take `cwd: &Path` and call `cmd.current_dir(cwd)` AFTER `split_into_command` builds the base `Command`.
- [ ] 5.4 GREEN — Verify `build_spawn_command` and `build_resume_command` pass the cwd through. Update the existing unit tests at `src/spawn/orchestrator.rs:1241` and `:1272` that construct these commands; they will break when `cwd` is added to the signatures. Ensure the final `Command` returned by `build_command_from_template` has `current_dir` set.
- [ ] 5.5 RED — Write failing test for `spawn_nudge_child`: the nudge/resume child also runs in the resolved cwd.
- [ ] 5.6 GREEN — Update `spawn_nudge_child` to set `cmd.current_dir(input.cwd)` before `spawn()`.

## 6. Fail-closed validation

- [ ] 6.1 RED — Write failing test: `NEW` with `--cwd /nonexistent` exits non-zero with a clear error and spawns no verifier; the goal directory is NOT created.
- [ ] 6.2 RED — Write failing test: `NEW` with `--cwd <existing-non-git-dir>` exits non-zero (`git rev-parse --is-inside-work-tree` fails) and spawns no verifier.
- [ ] 6.3 GREEN — Verify `resolve_cwd` + `capture_snapshot` error paths surface to `run_new` and abort before `goal::new` (or before `run_round` if validation is deferred to capture).

## 7. RESUME round-to-round consistency

- [ ] 7.1 RED — Write failing test: after `NEW` with `--cwd /repos/target`, a `RESUME` of the same goal runs snapshot + spawn in `/repos/target` even if the invoker's CWD is `/repos/other`.
- [ ] 7.2 GREEN — Implement `run_resume` to load `GoalRecord.resolvedCwd` and pass it to `run_round`. If `resolvedCwd` is missing (legacy goal), fall back to `std::env::current_dir()`.
- [ ] 7.3 RED — Write failing test: `RESUME` command does NOT accept a `--cwd` argument; passing one yields a clap error.
- [ ] 7.4 GREEN — Remove `--cwd` from the `RESUME` clap definition and confirm clap rejects it.

## 8. Integration + regression

- [ ] 8.1 Run full suite: `cargo test --all-features` — all green, no regressions.
- [ ] 8.2 Coverage gate: `cargo llvm-cov --fail-under-lines 80` — new/modified src files at >=80% lines.
- [ ] 8.3 Manual smoke: invoke `jewilo NEW` from CWD A against a target repo via (a) `--cwd`, (b) `config.json` `cwd`, (c) no override — verify all three produce real signed verdicts and `goal.json` reflects the correct `resolvedCwd` and verbatim config snapshot.
- [ ] 8.4 Manual smoke: `jewilo RESUME` of a goal created with `--cwd` — verify it uses the frozen `resolvedCwd` even when run from a different CWD.
- [ ] 8.5 Manual smoke: 4 parallel lanes (per the multi-lane test pattern) using `--cwd` to point at 4 distinct repos — verify 4 distinct hashes, zero starvation, zero unhealthy events.
- [ ] 8.6 Update `AGENTS.md` module map note for `store/` and `goal/` to mention the new `cwd` config field and `resolvedCwd` goal field.
