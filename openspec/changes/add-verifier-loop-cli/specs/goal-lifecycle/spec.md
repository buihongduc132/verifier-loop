## ADDED Requirements

### Requirement: Goal is created immutable and signed
The `NEW` subcommand SHALL write `goal.json` exactly once with goalText, context, createdAt, and a config snapshot. A separate `signature.json` SHALL contain `SHA256(salt + goalText + createdAt)` using the CLI-generated salt at `~/.verifier-loop/.salt` (mode 0600). After creation the goal text MUST NOT be modifiable by any subcommand or by direct agent action short of editing the file (which breaks all downstream hashes).

#### Scenario: NEW creates an immutable signed goal
- **WHEN** `verifier-loop NEW "fix the auth bug" --context "ticket #42"` runs
- **THEN** a `goal.json` is written under `~/.verifier-loop/goals/<goalId>/` containing the exact goalText, context, createdAt, and config snapshot
- **AND** a `signature.json` is written with `SHA256(salt + goalText + createdAt)`
- **AND** the goalId is printed to stdout

#### Scenario: Goal text cannot be changed after creation
- **WHEN** A attempts any subcommand or file edit that would alter goalText after `goal.json` exists
- **THEN** the change is not applied by the CLI
- **AND** any manual edit to goalText causes `signature.json` recomputation to mismatch every downstream completion hash

### Requirement: Salt is generated once and protected
The CLI SHALL generate a single 64-hex-char salt at `~/.verifier-loop/.salt` with mode 0600 on first run. The salt MUST never be printed to stdout, logged, or exposed to the invoking agent (A).

#### Scenario: First run creates the salt
- **WHEN** `verifier-loop NEW` runs and `~/.verifier-loop/.salt` does not exist
- **THEN** a 64-hex-char salt is created with mode 0600
- **AND** the salt value is not present in any stdout, stderr, or log output

### Requirement: RESUME appends fix notes without altering the goal
The `RESUME <goalId> --fix "..."` subcommand SHALL load the existing immutable goal, increment the round counter, and append the fix note to a round-scoped append-only `fix-notes.json`. The goalText and signature MUST remain unchanged.

#### Scenario: RESUME preserves goal and records fix notes
- **WHEN** `verifier-loop RESUME <goalId> --fix "fixed issues 1 and 2"` runs
- **THEN** the round counter increments
- **AND** the fix note is appended to `rounds/<round>/fix-notes.json`
- **AND** `goal.json` and `signature.json` are byte-for-byte unchanged

### Requirement: Missing or deleted goal store yields no proof
If `~/.verifier-loop/` or the goal directory is missing, no completion hash can be produced or validated. The CLI MUST fail closed in this case.

#### Scenario: Deleted store produces no hash
- **WHEN** the goal directory has been deleted and any subcommand is invoked
- **THEN** the CLI reports that no proof exists and exits non-zero
