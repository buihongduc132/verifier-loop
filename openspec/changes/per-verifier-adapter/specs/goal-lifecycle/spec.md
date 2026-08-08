## MODIFIED Requirements

### Requirement: Goal is created immutable and signed
The `NEW` subcommand SHALL write `goal.json` exactly once with goalText, context, createdAt, and a config snapshot. A separate `signature.json` SHALL contain `SHA256(salt + goalText + createdAt)` using the CLI-generated salt at `~/.verifier-loop/.salt` (mode 0600). After creation the goal text MUST NOT be modifiable by any subcommand or by direct agent action short of editing the file (which breaks all downstream hashes). The config snapshot SHALL include the resolved per-verifier adapter configuration (either the `verifiers` array or the `backend` field, whichever was active at creation time).

#### Scenario: NEW creates an immutable signed goal
- **WHEN** `verifier-loop NEW "fix the auth bug" --context "ticket #42"` runs
- **THEN** a `goal.json` is written under `~/.verifier-loop/goals/<goalId>/` containing the exact goalText, context, createdAt, and config snapshot
- **AND** the config snapshot includes the resolved adapter configuration for each verifier slot
- **AND** a `signature.json` is written with `SHA256(salt + goalText + createdAt)`
- **AND** the goalId is printed to stdout

#### Scenario: Goal text cannot be changed after creation
- **WHEN** A attempts any subcommand or file edit that would alter goalText after `goal.json` exists
- **THEN** the change is not applied by the CLI
- **AND** any manual edit to goalText causes `signature.json` recomputation to mismatch every downstream completion hash
