## MODIFIED Requirements

### Requirement: Frozen artifact snapshot is captured at spawn
At spawn time the CLI SHALL capture `cwd`, `git status --porcelain`, file edit times, and `git diff` truncated to `gitDiffMaxChars` (default 10000). This snapshot is frozen for the round and rendered into every verifier's prompt.

The `cwd` captured and rendered into the `{{cwd}}` template variable SHALL be the resolved verifier working directory threaded in by the caller. At NEW the resolution precedence is: `--cwd` flag > `config.json` `cwd` field > `std::env::current_dir()`; at RESUME the resolved cwd is the `resolvedCwd` frozen in the goal record (falling back to runtime for legacy goals). The CLI SHALL NOT call `std::env::current_dir()` inside `run_round`; it SHALL resolve cwd once at NEW (or load it at RESUME) and thread the resolved `PathBuf` into `capture_snapshot_with`. All git commands run by `capture_snapshot` (`git status`, `git diff`, `git rev-parse --is-inside-work-tree`, file edit times) SHALL execute in the resolved cwd via `git -C <cwd>`.

#### Scenario: Diff is truncated
- **WHEN** the git diff exceeds `gitDiffMaxChars`
- **THEN** the rendered `{{gitDiff}}` is truncated to that many characters with an indicator that truncation occurred

#### Scenario: Snapshot is consistent within a round
- **WHEN** two verifiers are spawned in the same round
- **THEN** both receive byte-identical artifact snapshots

#### Scenario: Snapshot uses config.json cwd at NEW
- **WHEN** `config.json` sets `cwd: "/repos/target"` and no `--cwd` flag is passed
- **THEN** `{{cwd}}` renders as `/repos/target`
- **AND** `git status` / `git diff` run in `/repos/target`
- **AND** the snapshot reflects `/repos/target`'s working tree, not the invoker's CWD

#### Scenario: Snapshot uses --cwd flag when both flag and config are set
- **WHEN** `--cwd /flag/path` is passed and `config.json` sets `cwd: "/config/path"`
- **THEN** `{{cwd}}` renders as `/flag/path` (flag precedence)
- **AND** git commands run in `/flag/path`

#### Scenario: Snapshot uses frozen resolvedCwd at RESUME
- **WHEN** `verifier-loop RESUME <goalId>` runs and the goal record has `resolvedCwd: "/repos/target"`
- **THEN** `{{cwd}}` renders as `/repos/target`
- **AND** git commands run in `/repos/target`, regardless of the `jewilo` process runtime CWD

#### Scenario: Snapshot fails closed on non-existent cwd
- **WHEN** the resolved cwd (from any source) does not exist or is not a directory
- **THEN** `capture_snapshot` returns an error
- **AND** NEW/RESUME exits non-zero without spawning any verifier

#### Scenario: Snapshot fails closed on non-git cwd
- **WHEN** the resolved cwd exists but `git rev-parse --is-inside-work-tree` fails
- **THEN** `capture_snapshot` returns an error (fail-closed, unchanged from pre-change behavior)
- **AND** NEW/RESUME exits non-zero without spawning any verifier
