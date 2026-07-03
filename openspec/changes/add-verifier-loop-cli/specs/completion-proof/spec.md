## ADDED Requirements

### Requirement: Completion hash is produced only on consensus
The CLI SHALL compute and print a completion hash to A only when a round reaches n/m consensus. The hash MUST NOT be produced on failure, on null verdicts, or before the gather barrier completes.

#### Scenario: Hash printed on pass
- **WHEN** a round reaches n/m APPROVE consensus
- **THEN** a completion hash of the form `vl:<40 hex chars>` is printed to stdout

#### Scenario: No hash on failure
- **WHEN** a round does not reach consensus
- **THEN** no completion hash is printed
- **AND** the CLI exits non-zero

### Requirement: Hash formula binds goal, round, and matching verdicts
The completion hash SHALL equal `"vl:" + first40hex(SHA256(salt + goalId + goalSignature + String(roundNumber) + JSON.stringify(matchingVerdicts) + matchedAtISO))`, where `goalSignature = SHA256(salt + goalText + createdAt)` and `matchingVerdicts` is the array of the matching round's APPROVE verdicts sorted by verifierId. Each input guards a distinct tamper vector.

#### Scenario: Hash is deterministic for identical inputs
- **WHEN** the same salt, goalId, goalSignature, roundNumber, matchingVerdicts (same order), and matchedAtISO are hashed
- **THEN** the resulting hash is identical

#### Scenario: Tampered goalText invalidates the hash
- **WHEN** goalText in `goal.json` is edited after creation
- **THEN** recomputing the hash yields a different value than any stored completion hash

#### Scenario: Tampered verdict invalidates the hash
- **WHEN** a stored APPROVE verdict's notes or registeredAt is edited
- **THEN** recomputing the hash yields a different value than the stored completion hash

### Requirement: Hash is audit-traceable to its goal directory
The completion hash SHALL be reproducible from the contents of the goal directory plus the salt. An auditor with access to `~/.verifier-loop/goals/<goalId>/` and `.salt` MUST be able to recompute and compare.

#### Scenario: Audit recomputes the hash
- **WHEN** an auditor reads `completion.json` and the goal directory and recomputes the formula
- **THEN** the recomputed hash matches the stored hash for an untampered goal

### Requirement: Completion record is written on success
On consensus the CLI SHALL write `completion.json` containing the hash, goalId, roundNumber, matchedAt, and the matchingVerdicts array.

#### Scenario: completion.json is written
- **WHEN** a round reaches consensus
- **THEN** `~/.verifier-loop/goals/<goalId>/completion.json` is written with hash, goalId, roundNumber, matchedAt, and matchingVerdicts
