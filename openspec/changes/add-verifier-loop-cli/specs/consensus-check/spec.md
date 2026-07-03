## ADDED Requirements

### Requirement: Consensus is n approvals out of m verifiers
After the gather barrier, the CLI SHALL count APPROVE verdicts among the `m` spawned verifiers. The round passes if and only if the APPROVE count is greater than or equal to `n` (configured in `config.json`). null and REJECT verdicts do not count toward `n`.

#### Scenario: Unanimous 2/2 passes
- **WHEN** config `n: 2, m: 2` and both verifiers register APPROVE
- **THEN** the round passes and a completion hash is generated

#### Scenario: 2/3 majority passes
- **WHEN** config `n: 2, m: 3` and two verifiers APPROVE while one REJECTs
- **THEN** the round passes and a completion hash is generated

#### Scenario: Below threshold fails
- **WHEN** config `n: 3, m: 3` and only two verifiers APPROVE
- **THEN** the round does not pass
- **AND** no completion hash is generated

### Requirement: Rejection surfaces notes to A
When a round does not pass, the CLI SHALL print a rejection to A containing each non-APPROVE verifier's notes (REJECT notes and a marker for null verdicts). A MAY then `RESUME` with `--fix`.

#### Scenario: Rejection includes reject notes
- **WHEN** a round fails with V1 REJECT `notes: "missing test"` and V2 null
- **THEN** A receives a rejection listing V1's notes and an indicator that V2 did not register a verdict

### Requirement: Consensus is static and human-configured
The values of `n` and `m` SHALL be read from `config.json` and MUST NOT be dynamically chosen by the CLI or by A at runtime.

#### Scenario: Config drives the threshold
- **WHEN** `config.json` sets `n: 2, m: 3`
- **THEN** every round for every goal uses a 2-of-3 threshold unless `config.json` is edited
