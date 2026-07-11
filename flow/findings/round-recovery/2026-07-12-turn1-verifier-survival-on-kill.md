# Explore Turn 1 — verifier survival on jewilo kill

> Date: 2026-07-12
> Phase: investigation (probe + code read)
> Status: concluded → answered

## [user] (verbatim — trust chain L1)

> If the current jewilo is getting CtrlC or interupt mid way , do the underlying verifier still running or killed?

## [assistant]

**Short answer: it depends on how it's interrupted, but in the common case the verifiers ARE killed — by the terminal, not by jewilo.**

Evidence [E1]: `src/spawn/orchestrator.rs:208,313` — every child spawned with `.kill_on_drop(true)`.
Evidence [E2]: `rg` for `signal|SIGINT|ctrl_c` across `src/` → **zero handlers**. jewilo installs no signal handler.
Evidence [E3]: `src/bin/verifier_loop.rs:39` — `fn main() -> ExitCode` (sync, no tokio `#[main]`).

Cause [C1]: `kill_on_drop` only fires when tokio `Child` handles are **dropped by the runtime**. On Ctrl-C / SIGTERM the OS kills jewilo directly — Drop does NOT run — so `kill_on_drop` is a no-op there. It only helps on clean `Err`/panic paths inside jewilo.
Cause [C2]: jewilo does **not** detach children into a new process group/session. Verifiers inherit jewilo's foreground pgrp.

Result by scenario:

| Scenario | jewilo | Verifiers |
|---|---|---|
| Ctrl-C in a TTY (interactive) | dies (no handler) | **killed** — terminal sends SIGINT to whole foreground pgrp [C2] |
| `kill <jewilo-pid>` (SIGTERM, no TTY) | dies | **keep running** as orphans (reparented to init); `kill_on_drop` skipped [C1] |
| `kill -9` | dies | same — orphans |
| jewilo hits internal error/panic | runtime drops `Child` | **killed via `kill_on_drop`** [E1] |
| `verifierTimeoutSec` per-slot | n/a | **killed** (`orchestrator.rs:432 start_kill`) |

Callout [CA1]: The non-TTY orphan case is the real gap. If jewilo is driven by an outer agent (the common jewilo usage) and that agent SIGTERMs jewilo, the spawned `pi`/`hermes`/`acpx` verifiers leak and keep burning tokens until they self-exit or hit their own timeouts. Worth a GH issue + a `SIGTERM`/`ctrl_c` handler in `main` that reaps children before exiting (or `setsid`-detaching them + explicit kill-on-shutdown). This is **not** in the current `fix-prompt-bloat-and-compaction-recovery` openspec change.

Assumption [A]: child verifiers inherit jewilo's pgrp (no `pre_exec`/`process_group` call seen) — verified by `rg` absence, not by reading every `Command` builder.

## Status at end of turn 1

Answered: kill behavior depends on TTY-vs-non-TTY. Gap surfaced: non-TTY SIGTERM orphans verifiers. Next: turn 2 reframes toward "resume the current round" instead of new round.
