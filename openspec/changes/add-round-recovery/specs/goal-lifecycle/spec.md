## MODIFIED Requirements

### Requirement: State-mutating commands take an exclusive goal lock
`NEW`, `RESUME`, and `RECOVER` SHALL each acquire an exclusive advisory lock on `goals/<goalId>/.lock` for their full duration before mutating any goal state. A concurrent invocation of any of these commands on the same goal SHALL exit non-zero with a clear "goal busy; another NEW/RESUME/RECOVER in progress" message. The lock file is advisory (`flock` on Unix); it contains no secret material and is created idempotently. A lock left behind by a crashed process is harmless because advisory locks are released by the OS on process exit.

#### Scenario: Concurrent RESUME is rejected while NEW holds the lock
- **WHEN** a `jewilo NEW "<goal>"` is mid-flight (holding the goal lock) and a second `jewilo RESUME <sameGoalId>` is invoked
- **THEN** the second invocation exits non-zero
- **AND** its stderr states the goal is busy

#### Scenario: A crashed command does not poison the goal
- **WHEN** a `jewilo RECOVER` process is killed while holding the goal lock
- **THEN** a subsequent `jewilo STATUS <goalId>` succeeds (STATUS takes no lock)
- **AND** a subsequent `jewilo RESUME <goalId>` acquires the lock successfully

### Requirement: RESUME warns when the current round has null verdicts
When `RESUME <goalId>` is invoked and the current round has at least one null verdict slot and no `completion.json`, the CLI SHALL emit a warning on stderr suggesting the user consider `RECOVER <goalId>` first to harvest in-flight verdicts, then SHALL proceed with the round increment (RESUME remains the user's explicit escape hatch).

#### Scenario: RESUME warns about a recoverable round
- **WHEN** round 1 has a null slot and no completion, and the user runs `jewilo RESUME <goalId>`
- **THEN** stderr contains a warning referencing `RECOVER`
- **AND** RESUME proceeds to increment the round

### Requirement: RECOVER warns when the round is already complete
When `RECOVER <goalId>` is invoked and the current round already has a `completion.json`, the CLI SHALL emit a warning on stderr directing the user to `RESUME N+1`, and SHALL exit 0 without polling or mutating state.

#### Scenario: RECOVER on a complete round is a no-op with guidance
- **WHEN** `completion.json` exists for the current round and the user runs `jewilo RECOVER <goalId>`
- **THEN** stderr contains a warning referencing `RESUME`
- **AND** the exit code is 0
- **AND** no verdict file is polled or written
