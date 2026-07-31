## MODIFIED Requirements

### Requirement: Goal is created immutable and signed
The `NEW` subcommand SHALL write `goal.json` exactly once with goalText, context, createdAt, and a config snapshot. A separate `signature.json` SHALL contain `SHA256(salt + goalText + createdAt)` using the CLI-generated salt at `~/.verifier-loop/.salt` (mode 0600). After creation the goal text MUST NOT be modifiable by any subcommand or by direct agent action short of editing the file (which breaks all downstream hashes).

`GoalRecord` SHALL also record the resolved verifier working directory in an optional `resolvedCwd` field, separate from the `config` snapshot. The `resolvedCwd` value is derived at `NEW` time from the precedence: `--cwd` flag > `config.json` `cwd` field > `std::env::current_dir()`. It SHALL be absolute and canonicalized. The `config` snapshot in `goal.json` SHALL continue to reflect `config.json` verbatim (i.e., `Config.cwd` records the config.json value, which may be null or absent). The `cwd` field is now a recognized canonical key in `config.json` (`deny_unknown_fields` remains; the recognized key set is widened to admit `cwd`).

#### Scenario: NEW creates an immutable signed goal
- **WHEN** `verifier-loop NEW "fix the auth bug" --context "ticket #42"` runs
- **THEN** a `goal.json` is written under `~/.verifier-loop/goals/<goalId>/` containing the exact goalText, context, createdAt, config snapshot, and resolvedCwd
- **AND** a `signature.json` is written with `SHA256(salt + goalText + createdAt)`
- **AND** the goalId is printed to stdout

#### Scenario: Goal text cannot be changed after creation
- **WHEN** A attempts any subcommand or file edit that would alter goalText after `goal.json` exists
- **THEN** the change is not applied by the CLI
- **AND** any manual edit to goalText causes `signature.json` recomputation to mismatch every downstream completion hash

#### Scenario: config.json cwd field is accepted as canonical
- **WHEN** `config.json` contains `"cwd": "/path/to/repo"`
- **THEN** the config parses successfully (no longer a hard parse error)
- **AND** the resolved cwd used for snapshot and spawn is `/path/to/repo` (when no `--cwd` flag overrides it)

#### Scenario: config.json cwd is null or absent
- **WHEN** `config.json` omits `cwd` or sets `"cwd": null` and no `--cwd` flag is passed
- **THEN** the resolved cwd is `std::env::current_dir()` at NEW time
- **AND** `goal.json` records `resolvedCwd` holding that runtime CWD value (resolvedCwd is ALWAYS recorded at NEW, regardless of source)
- **AND** subsequent RESUME rounds use the frozen `resolvedCwd`, NOT a fresh runtime CWD (so changing the invoker's CWD between NEW and RESUME does not change the verifier CWD)

#### Scenario: --cwd flag overrides config and runtime at NEW
- **WHEN** `verifier-loop NEW "goal" --cwd /explicit/path` runs with `config.json` `cwd: "/config/path"`
- **THEN** the resolved cwd is `/explicit/path` (flag precedence)
- **AND** `goal.json` records `resolvedCwd: "/explicit/path"`
- **AND** the `config` snapshot still records `"cwd": "/config/path"`

#### Scenario: resolved cwd is frozen into the goal record
- **WHEN** NEW completes with any cwd source
- **THEN** `goal.json` contains a `resolvedCwd` field holding the resolved absolute canonicalized path
- **AND** every future RESUME uses that same path (unless the field is missing, in which case legacy runtime fallback applies)

#### Scenario: RESUME reuses the frozen resolvedCwd
- **WHEN** `verifier-loop RESUME <goalId>` runs after a NEW that recorded `resolvedCwd: "/repos/target"`
- **THEN** the round's snapshot and spawned verifiers run in `/repos/target`
- **AND** the runtime CWD of the `jewilo` process does not affect the verifier CWD

#### Scenario: legacy goal without resolvedCwd resumes correctly
- **WHEN** a legacy goal (predating this change, no `resolvedCwd` field) is loaded
- **THEN** the missing `resolvedCwd` is treated as `None`
- **AND** RESUME falls back to `std::env::current_dir()` (matching the original behavior for that goal, which was created under runtime CWD every round)
- **AND** note: `goal.json` for new goals gains the optional `resolvedCwd` field, so new goals are NOT byte-identical to pre-change goals; the behavioral contract (cwd used for verification) is equivalent for the runtime-source case
