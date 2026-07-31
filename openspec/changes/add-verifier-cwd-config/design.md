## Context

Today the verifier working directory is sourced exclusively from
`std::env::current_dir()` inside `run_round` (`src/bin/verifier_loop.rs:312`) and is
explicitly REJECTED from `config.json` (`deny_unknown_fields` on `Config`,
`src/store/config.rs:36`; there is a test asserting `cwd` in config is a hard parse error).
This forces every orchestrator, multi-repo driver, or wrapper that wants to direct
verification at a tree other than the invoker's CWD to `cd` into the target repo before
invoking `jewilo`. There is no way to pin the verifier CWD via configuration.

Concrete code paths today:
- `src/bin/verifier_loop.rs:312` — `let cwd = std::env::current_dir()?` inside `run_round`
- `src/prompt/mod.rs::capture_snapshot_with` — runs git commands in the `cwd` argument it
  already receives; the caller (`run_round`) currently passes `std::env::current_dir()`.
- `src/spawn/orchestrator.rs::build_command_from_template` / `split_into_command` —
  `tokio::process::Command::new(...)`, no `.current_dir(...)` → child inherits parent's CWD.
- `src/spawn/orchestrator.rs::spawn_nudge_child` — also builds a `Command` and inherits CWD.
- `src/goal/mod.rs:42` — `GoalRecord` has no CWD field; config snapshot is just
  `store::Config` verbatim.

Constraints (must NOT break):
- Fail-closed invariants (NULL verdict never → APPROVE; missing store → no hash;
  goalText/verdict edit → hash mismatch).
- The completion hash formula (`src/consensus/mod.rs:336`) is unchanged — cwd is not a
  hash input. We do not alter the hash contract.
- `goal.json` remains readable by older builds; `GoalRecord` does not use
  `deny_unknown_fields`, so adding an optional `resolvedCwd` field is safe.
- Backward compatibility: existing configs and goals must behave identically when `cwd` is
  absent or null.

## Goals / Non-Goals

**Goals:**
- Add an optional `cwd` field to `config.json` that, when set, overrides the runtime CWD
  for both artifact snapshot capture and every spawned verifier child process.
- Default to today's behavior (inherit runtime CWD) when `cwd` is unset/null.
- Add a `--cwd <PATH>` CLI flag on `NEW` only, with precedence `--cwd` flag > `config.json`
  `cwd` > `std::env::current_dir()`.
- Resolve cwd ONCE at NEW, freeze it as `resolvedCwd` in `GoalRecord`, and reuse that
  frozen CWD for every RESUME round. This binds the goal to one repo for its lifetime and
  prevents round-to-round context drift.
- Thread the resolved CWD explicitly through `run_round`, `SpawnInput`, and all spawn sites
  (initial spawn, `spawn_nudge_child`, compaction-recovery resume) so the contract is
  explicit and testable.
- Preserve every fail-closed invariant and the hash-stability contract.
- Update stale docstrings and tests impacted by the new field.

**Non-Goals:**
- Per-verifier distinct CWDs (all `m` verifiers in a round share the same resolved CWD).
- Adding an `AUDIT` subcommand or modifying any AUDIT path (OT1 is deferred / out of scope
  per AGENTS.md). `resolvedCwd` is recorded for round consistency and future auditability,
  not because this change touches an audit implementation.
- Sandboxing / chroot / path-allowlist enforcement.
- Migrating the runtime CWD of the `jewilo` process itself (it keeps its own CWD; only
  children + snapshot use the resolved value).
- Changing `jewije` (verdict-verdict) behavior — it is unaffected.
- Allowing `--cwd` on `RESUME` (that would break round-to-round consistency and is not
  supported by the current immutable-goal model).

## Decisions

### D1 — Precedence: `--cwd` flag > `config.json` `cwd` > `std::env::current_dir()` (NEW only)

The flag wins because it is the most explicit per-invocation override. Config beats
runtime because config is the durable project-level declaration. Runtime is the fallback
so existing behavior is preserved with zero config change. The flag is only on `NEW`;
`RESUME` reuses the frozen `resolvedCwd` from the goal record.

**Alternatives considered:**
- Config-only (no flag): rejected — orchestrators and one-off scripts need a per-call
  override without rewriting `config.json` mid-run.
- Flag-only (no config): rejected — durable cross-repo deployments want a single source
  of truth, not a flag on every wrapper invocation.
- `--cwd` on `RESUME`: rejected — allows a later round to run against a different repo
  than the goal was created for, silently changing the meaning of consensus and breaking
  the immutable-goal contract. If a different repo is needed, create a new goal.

### D2 — Resolution + canonicalization happens ONCE at NEW, not at config-parse time

`run_new` resolves cwd by calling `resolve_cwd(flag, config, runtime)` where `runtime` is
`std::env::current_dir()`, then canonicalizes via `std::fs::canonicalize`. It passes the
resolved `PathBuf` to `goal::new` (which stores it as `GoalRecord.resolvedCwd`) and to
`run_round`. `config.json` parsing does NOT canonicalize `cwd` — it only validates the value
is a string or null; a bad path is caught at resolution time, not at config-load time. This is consistent with the current design
where config.json carries intent and the CLI validates runtime constraints at invocation.

**Why:** A single resolution point makes the value auditable, avoids drift between the
snapshot capture and the child spawn, and gives the frozen goal record one source of truth.

**Alternatives considered:**
- Canonicalize at config-parse time (task 1.4 in the original draft): rejected — a flag or
  runtime CWD may override the config value, so canonicalizing the config value alone is
  wasted work and can produce misleading error messages when the override is what will
  actually be used.
- Each call site reads env/config independently: rejected — drift risk.
- Global static / `once_cell`: rejected — hidden coupling, hard to test.

### D3 — Resolved cwd is recorded in `GoalRecord.resolvedCwd`, NOT inside `Config`

`GoalRecord.config` is a verbatim snapshot of `config.json`. If we mutated `Config.cwd` to
the resolved value, we would be corrupting the config snapshot (it would no longer reflect
the config.json file). Instead, we add a new `resolvedCwd` field to `GoalRecord`. This
field stores the resolved absolute canonicalized path at NEW time.

**Why:** This preserves the config-snapshot-as-verbatim contract and keeps the resolved
CWD at the goal level (where it belongs, since it is immutable for the goal's lifetime).

**Alternatives considered:**
- Mutate `Config.cwd` with the resolved value before serializing: rejected — corrupts the
  config snapshot; also cannot represent a runtime-cwd fallback when no config is set.
- Store only a boolean `cwdOverridden`: rejected — future audit/inspect tooling cannot
  re-derive the actual path.

### D4 — `cwd` becomes a canonical key in `config.json`; `deny_unknown_fields` remains

`src/store/config.rs` admits `cwd` into the recognized key set while keeping the struct
annotated with `deny_unknown_fields`. Every other unknown key remains a hard parse error.
The existing "unknown `cwd` key must be rejected" test is inverted to assert `cwd` is now
accepted and parsed as an absolute path (or null).

**Why:** The closed key-set exists to catch typos and stale config — that property must
survive. Only `cwd` is admitted, deliberately and visibly.

**Alternatives considered:**
- Switch to permissive/open key-set: rejected — loses the typo safety net.
- Namespace as `verifierCwd`: rejected — `cwd` is the natural name, mirrors the prompt
  template variable `{{cwd}}` and the `GoalRecord.resolvedCwd` field.

### D5 — Child spawn sets `current_dir(...)` explicitly on every `Command`

`build_command_from_template` sets `cmd.current_dir(resolved_cwd)` after
`split_into_command` builds the base command. This covers initial spawn, nudge resume, and
compaction-recovery resume. Even when the resolved cwd equals the runtime CWD (the default
case), it is set explicitly — no reliance on inheritance.

**Why:** Eliminates the implicit-inheritance coupling that today makes the spawn CWD
depend on where `jewilo` happens to be invoked. Makes the contract explicit and testable.

**Alternatives considered:**
- Only set `current_dir` when override is active: rejected — implicit vs explicit
  difference is exactly the kind of drift that caused the CA1 confusion about "what is
  the verifier CWD" in the first place.
- Set `current_dir` inside `split_into_command`: rejected — that function only parses the
  template string into argv; the CWD is a spawn property, not an argv property.

### D6 — Fail-closed on bad cwd: not-a-directory, non-existent, non-git

If the resolved cwd (from any source) does not exist, is not a directory, or is not a git
work tree (`git rev-parse --is-inside-work-tree` fails), `capture_snapshot` fails closed
exactly as today. `NEW` exits non-zero with a clear message. No verifier is spawned.

**Why:** A verifier pointed at the wrong tree silently APPROVEs nothing useful and
produces a hash that proves the wrong thing. Failing loud and early is the only safe
default.

**Alternatives considered:**
- Auto-create the directory: rejected — silently fabricating a workspace violates the
  "do not ship blind" principle.
- Warn and continue with runtime CWD: rejected — silent fallback hides misconfiguration.

### D7 — RESUME uses the frozen `resolvedCwd`, not a per-invocation override

`run_resume` loads the goal record, extracts `resolvedCwd`, and passes it to `run_round`.
It does not accept or apply a `--cwd` flag. If `resolvedCwd` is missing (legacy goal), the
legacy behavior is preserved: fall back to `std::env::current_dir()` at RESUME time.

**Why:** This matches the immutable-goal model and prevents a later round from being run
against a different repository than the goal was created for, which would silently change
what the consensus is about.

## Risks / Trade-offs

- **[Legacy goal RESUME cwd drift]** Older goals have no `resolvedCwd`. When resumed, they
  fall back to `std::env::current_dir()`, which is the legacy behavior and therefore
  consistent with how they were created (all prior rounds used the runtime CWD at the time).
  → Mitigation: documented in D7; no behavior change for legacy goals.

- **[Symlink / canonicalization mismatch]** A config `cwd` set to a symlinked path may
  canonicalize differently at NEW time vs audit time if the symlink target changes.
  → Mitigation: D2 canonicalizes via `std::fs::canonicalize` at resolution time, and the
  canonicalized form is what is frozen into `resolvedCwd`. Document that symlink targets
  must be stable for the goal's lifetime.

- **[Test-only breaking change]** The "unknown `cwd` key must be rejected" test flips.
  → Mitigation: called out as test-only in proposal; no external user has ever shipped a
  `cwd` key (it was rejected, so none could).

- **[Multi-verifier CWD coupling]** All `m` verifiers share one CWD; cannot point v1 at
  repo A and v2 at repo B.
  → Trade-off accepted (Non-Goal). If a future need arises, it belongs in a
  `per-verifier-adapter`-style config, not this change.

- **[Config snapshot grows]** `goal.json` gains one optional field. Negligible.

- **[Stale docstrings must be updated]** `src/prompt/mod.rs:275` claims `--show-toplevel`
  is checked; actual code uses `--is-inside-work-tree`. `src/store/config.rs` docstrings
  claim `cwd` is rejected and runtime-only. These must be updated to match the new reality.
  → Mitigation: explicit tasks in tasks.md.

## Migration Plan

- No code migration needed for existing goals or configs. Absent/null `cwd` reproduces
  legacy behavior.
- Older builds reading a `goal.json` with the new `resolvedCwd` field: ignored because
  `GoalRecord` does not use `deny_unknown_fields`. Safe.
- Newer builds reading a legacy `goal.json`: `resolvedCwd` defaults to `None`, so RESUME
  falls back to runtime CWD (legacy behavior). Safe.
- Rollback: revert the change. Any goals created with `resolvedCwd` will still load fine
  (the field is optional) and any future RESUME will use runtime CWD if the code is
  reverted, which is acceptable for a rollback scenario.

## Open Questions

- None. All design decisions are locked.
