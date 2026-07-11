# References

> Sources consulted during this explore session.

## Source files
- `src/spawn/orchestrator.rs` — child spawn, `kill_on_drop(true)` (L208, L313), `gather()` barrier (L384), per-verifier `start_kill()` on timeout (L432), `pre_create_verifier_dir` / `pre_create_verifier_dir_with_turns` (L517, L521), `read_meta`/`update_meta_after_run` (L540, L549). Central to kill + resumption analysis.
- `src/bin/verifier_loop.rs` — `fn main() -> ExitCode` sync entry (L39), `run()` dispatch, `run_new` / `run_resume`, shared `run_round` driver (snapshot → render → spawn → gather → evaluate → hash/reject), `resolve_adapter`. Confirms no signal handler; runtime built per-call in `run_round`.
- `src/goal/mod.rs` — `new` (L78), `resume` (L134, increments round + appends fix-notes), `current_round` (L177), `StateRecord.current_round`. Defines RESUME's round-advancement semantics that RECOVER must NOT replicate.
- `src/verdict/mod.rs` — `register_signed_approve` / `register_signed_reject` (L294+), `VERDICT_FILE`, first-write-wins (`write_first_verdict`), `AlreadyFinal` on non-null overwrite. Confirms verdict file = durable resumption contract, per round per slot.
- `src/acp/adapters.rs` — `build_spawn_command` / `build_resume_command`, pi/hermes/acpx templates (`pi --offline --session {sid} --mode json`), `Transport::{Stdin,GoalFile}`. Shows resume-by-SID is a native adapter capability.

## Documents
- `openspec list --json` output — confirmed active changes: `fix-prompt-bloat-and-compaction-recovery` (naming-collision risk, OT4), `add-verifier-loop-cli`, `add-verifier-tamper-hardening`, etc.

## Probes (empirical)
- `rg -n "SIGINT|Ctrl|interrupt|signal|kill|SpawnExt|child_kill|kill_on_drop|nix::sys::signal|Child::kill" src/` → zero signal handlers; `kill_on_drop` at orchestrator L208/L313.
- `pi --help` / `pi -p --help` → `--mode json`, `--continue/-c`, `--resume/-r`, `--session <path|id>`, `--session-id <id>`, `--no-session`. Resume-by-SID is supported via `--session`.
- `pi -p --mode json 'say hi'` → first stdout line is `{"type":"session","version":3,"id":"<uuid>","timestamp":...,"cwd":...}`. SID emitted before any agent work (t≈0).
- `pi -p --mode json --session-id test-probe-$$ 'say hi'` → session id echoed as the `--session-id` value; session file not necessarily persisted to default dir in non-interactive mode.
- Concurrent same-SID test: second `pi -p --mode json --session <sid>` started while first still running → second process starts INDEPENDENTLY, does not attach/wait. Confirms same-SID concurrency is a silent race, not coordination.
- Kill + resume test: killed a running `pi -p --mode json` mid-turn, then `pi -p --mode json --session <sid> 'follow up'` → resumed cleanly from the persisted session file. Confirms session file survives process death.

## Code patterns
- `kill_on_drop(true)` on every `Command` — only fires on runtime-driven Drop (errors/panics), NOT on OS signal kill. Core reason scenario B orphans.
- Children inherit jewilo's foreground pgrp (no `setsid`/`process_group`/`pre_exec`) — verified by rg absence. Core reason scenario A's SIGINT reaches children.
- `meta.json {sid, turnsUsed}` written by `update_meta_after_run` AFTER child exits — purely a round-advancement hint, excluded from consensus + completion hash. This is what makes "collect again" recovery cheap.
- `verdict.json` pre-created as `{status:null}` at spawn time (`pre_create_verifier_dir`); child writes final verdict via `verifier-verdict` CLI independently of jewilo. Durable cross-process signal.
