## Why

The verifier working directory is sourced exclusively from `std::env::current_dir()` at
runtime (`src/bin/verifier_loop.rs:312` inside `run_round`) and is explicitly REJECTED
from `config.json` (closed key-set with `deny_unknown_fields`, `src/store/config.rs:36`;
there is even a test asserting `cwd` in config is a hard parse error). This forces every
orchestrator, multi-repo driver, or wrapper that wants to direct verification at a tree
other than the invoker's CWD to `cd` into the target repo before invoking `jewilo`. There
is no way to pin the verifier CWD via configuration.

We need a first-class `cwd` config field that, when set, overrides the runtime CWD for
verifier spawn + artifact snapshot, while defaulting to the current behavior (inherit the
invoker's CWD) when unset.

## What Changes

- Add a new optional `cwd` field to the `config.json` schema (string absolute path or null).
  - Default: `null` → use `std::env::current_dir()` (today's behavior, zero behavioral
    change for existing configs).
  - When set to an absolute path: that path becomes the CWD for (a) the artifact snapshot
    capture (`git status`, `git diff`, edit times, `git rev-parse --is-inside-work-tree`)
    and (b) every spawned verifier child process (`cmd.current_dir(...)`).
- Relax the `config.json` closed key-set (`deny_unknown_fields` is KEPT): `cwd` becomes a
  recognized canonical field instead of a hard parse error. All other unknown fields remain
  hard errors.
- **Freeze the resolved cwd into `goal.json` at NEW time** as a NEW field on `GoalRecord`
  (`resolvedCwd`), NOT by mutating the `config` snapshot. The `config` snapshot continues
  to record config.json verbatim. Rationale: round-to-round consistency (RESUME must see
  the same repo NEW saw) and future auditability. **The completion hash is NOT affected** —
  `compute_hash` (`src/consensus/mod.rs:336`) inputs are salt + goalId + signature + round
  + matchingVerdicts + matchedAt + receiptHead; cwd and prompt bytes are NOT hash inputs.
  Recording cwd is therefore NOT required for hash reproducibility; it is required so every
  round's snapshot is taken against the same tree.
- `cli --cwd <PATH>` flag on **`NEW` only** (NOT `RESUME`). RESUME reuses the `resolvedCwd`
  frozen at NEW so the goal stays bound to one repo across rounds. Precedence at NEW:
  `--cwd` flag > `config.json` `cwd` > `std::env::current_dir()`.
- **No dependency on the AUDIT subcommand** (OT1 is deferred / out of scope per AGENTS.md).
  `resolvedCwd` is recorded purely for round consistency and for any future audit tooling;
  this change does NOT modify any AUDIT path (none exists).
- Update stale docstrings in `src/store/config.rs` that claim `cwd` is rejected /
  runtime-only, and the docstring in `src/prompt/mod.rs:275` that says `--show-toplevel`
  (actual code uses `--is-inside-work-tree`).
- Update `Config::Default` impl and the `config_round_trips` test to cover `cwd`.
- **Test-only breaking change**: the existing "unknown `cwd` key must be rejected" test
  flips to assert `cwd` is now accepted. No external user has ever shipped a `cwd` key (it
  was rejected), so no external breakage.

## Capabilities

### New Capabilities

_(none — all changes modify existing capabilities.)_

### Modified Capabilities

- `goal-lifecycle`: `GoalRecord` gains an optional `resolvedCwd` field (separate from the
  `config` snapshot) recording the resolved verifier CWD at NEW. The closed key-set on
  `Config` is widened to admit `cwd` as canonical. RESUME loads `resolvedCwd` from the goal
  record rather than re-resolving, so the goal is bound to one repo for its lifetime.
- `verifier-prompt`: `capture_snapshot` runs its git commands and resolves the `{{cwd}}`
  template variable against the resolved cwd threaded in as a parameter, not
  unconditionally against `std::env::current_dir()`.
- `verifier-spawn`: every spawned verifier child process — including the
  nudge/compaction-recovery resume child (`spawn_nudge_child`) — has `current_dir(...)`
  set to the resolved cwd instead of inheriting the parent's CWD implicitly.

## Impact

- **Code**:
  - `src/store/config.rs` — add `cwd: Option<PathBuf>` to `Config` (with `Default::default`
    = `None`); keep `deny_unknown_fields`; remove `cwd` from the rejected-keys test;
    update `Config::Default` impl, the round-trip test, and the stale docstrings.
  - `src/goal/mod.rs` — add `resolved_cwd: Option<PathBuf>` (`resolvedCwd`, optional,
    `skip_serializing_if = Option::is_none`) to `GoalRecord`; populate at NEW.
  - `src/prompt/mod.rs` — `capture_snapshot_with` already takes `cwd: &Path` as a
    parameter (no signature change needed); update the stale `--show-toplevel` docstring
    to `--is-inside-work-tree`.
  - `src/spawn/orchestrator.rs` — add `cwd: &Path` to `SpawnInput`; set
    `cmd.current_dir(resolved_cwd)` on EVERY built `Command` in `build_command_from_template`
    (after `split_into_command`), covering initial spawn, nudge resume, and
    compaction-recovery resume. Thread through `spawn_round` + `spawn_nudge_child`.
  - `src/bin/verifier_loop.rs` — resolve cwd once at entry inside `run_new` (flag > config >
    runtime), pass it to `goal::new` (to populate `resolvedCwd`), then thread `cwd: &Path`
    through `run_round` (replacing the internal `std::env::current_dir()` call at line 312)
    and into `SpawnInput`. `run_resume` loads `resolvedCwd` from the goal record and
    threads it through; it does NOT accept `--cwd`.
- **CLI surface**: new `--cwd <PATH>` flag on `NEW` only.
- **Backward compatibility**:
  - `cwd: null` (or absent) reproduces today's verification behavior exactly for existing
    configs (the runtime CWD is used). Note: `goal.json` for NEW goals gains the optional
    `resolvedCwd` field, so new goals are NOT byte-identical to pre-change goals, but the
    behavioral contract is equivalent.
  - `GoalRecord` does NOT use `deny_unknown_fields`, so older builds reading a `goal.json`
    with the new optional `resolvedCwd` field will ignore it (safe). Newer builds reading a
    legacy `goal.json` without `resolvedCwd` treat it as `None` and fall back to runtime
    CWD (matches legacy behavior, since legacy goals were created under runtime CWD).
  - No existing hash or receipt changes shape.
- **Relative path semantics**: a relative `cwd` in `config.json` or a relative `--cwd`
  flag value is resolved relative to the invoker's runtime CWD (`std::env::current_dir()`)
  and then canonicalized. This is documented behavior, not an error — it lets wrappers pass
  repo-relative paths. `resolve_cwd` therefore reads the runtime CWD as its third input.
- **Fail-closed invariants**: untouched. If `cwd` is set but is not a directory or is not a
  git work tree, `capture_snapshot` fails closed exactly as a bad runtime CWD would today.
