## MODIFIED Requirements

### Requirement: Verifiers are spawned in parallel via ACP JSON stream
On `NEW` and on each `RESUME` round, the CLI SHALL spawn all `m` verifier sessions concurrently as separate ACP-process invocations (e.g. `pi -p "<prompt>" --mode json`), each with injected identity env vars. The spawns MUST be non-blocking relative to one another; the CLI blocks only at the gather barrier after all are launched.

Every spawned verifier child process — including the initial `m` verifiers, the nudge
resume child (`spawn_nudge_child`), and the compaction-recovery resume child — SHALL have
its working directory set explicitly via `Command::current_dir(&resolved_cwd)` where
`resolved_cwd` is the resolved verifier working directory (precedence at NEW: `--cwd` flag >
`config.json` `cwd` > `std::env::current_dir()`; at RESUME: the `resolvedCwd` frozen in the
goal record). The CLI SHALL NOT rely on implicit CWD inheritance from the parent `jewilo`
process. All `m` verifiers in a round share the single resolved cwd.

#### Scenario: All verifiers start at once
- **WHEN** `verifier-loop NEW "goal"` runs with config `m: 3`
- **THEN** three verifier processes are launched concurrently
- **AND** none blocks the launch of another

#### Scenario: Identity env vars are injected per spawn
- **WHEN** a verifier process is spawned
- **THEN** its environment contains `VERIFIER_LOOP_GOAL_ID`, `VERIFIER_LOOP_VERIFIER_ID` (v1, v2, ...), and `VERIFIER_LOOP_ROUND`

#### Scenario: Child process CWD matches resolved cwd
- **WHEN** a verifier is spawned with resolved cwd `/repos/target`
- **THEN** the child process's working directory is `/repos/target`
- **AND** the child's `pwd`/`std::env::current_dir()` reports `/repos/target`, regardless of the parent `jewilo` process's CWD

#### Scenario: Child CWD follows --cwd flag precedence at NEW
- **WHEN** `--cwd /flag/path` is passed, `config.json` sets `cwd: "/config/path"`, and the parent process CWD is `/invoker/dir`
- **THEN** the spawned verifier child's working directory is `/flag/path`

#### Scenario: Child CWD follows frozen resolvedCwd at RESUME
- **WHEN** `verifier-loop RESUME <goalId>` runs with the goal record's `resolvedCwd: "/repos/target"`, and the parent process CWD is `/invoker/dir`
- **THEN** the spawned verifier child's working directory is `/repos/target`

#### Scenario: Child CWD falls back to runtime when no override at NEW
- **WHEN** no `--cwd` flag is passed and `config.json` omits `cwd` (or sets null)
- **THEN** the spawned verifier child's working directory equals the parent `jewilo` process's `std::env::current_dir()`
- **AND** behavior matches pre-change releases

#### Scenario: Nudge resume child also gets the resolved cwd
- **WHEN** a verdict-enforcement nudge or compaction-recovery resume spawns a new process via `spawn_nudge_child`
- **THEN** that child process also has its working directory set to the same resolved cwd as the initial spawn for that round
