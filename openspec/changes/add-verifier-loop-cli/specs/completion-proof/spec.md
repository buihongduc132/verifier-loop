## ADDED Requirements

### Requirement: Completion hash is produced only on consensus
The CLI SHALL compute and print a completion hash to A only when a round reaches n/m consensus. The hash MUST NOT be produced on failure, on null verdicts, or before the gather barrier completes.

#### Scenario: Hash printed on pass
- **WHEN** a round reaches n/m APPROVE consensus
- **THEN** a completion hash of the form `mmddyy-XXXXXXXX` (2-digit month + 2-digit day + 2-digit year of `matchedAt`, hyphen, 8 hex chars) is printed to stdout
- **AND** the full 64-hex SHA-256 digest is stored in `completion.json` `fullDigest` (not printed)

#### Scenario: No hash on failure
- **WHEN** a round does not reach consensus
- **THEN** no completion hash is printed
- **AND** the CLI exits non-zero

### Requirement: Hash formula binds goal, round, and matching verdicts
The short completion hash (displayed) SHALL equal `mmddyy + "-" + first8hex(SHA256(salt + goalId + goalSignature + String(roundNumber) + JSON.stringify(matchingVerdicts) + matchedAtISO))`, where `mmddyy` is the UTC date of `matchedAt` (MMDDYY), `goalSignature = SHA256(salt + goalText + createdAt)` (stored full in `signature.json`), and `matchingVerdicts` is the array of the matching round's APPROVE verdicts sorted by verifierId.

The FULL digest `fullDigest = SHA256(same inputs)` (64 hex) SHALL also be computed and stored in `completion.json` for exact (non-probabilistic) audit recompute. Each input guards a distinct tamper vector; tampering any input changes `fullDigest` deterministically (and almost certainly changes the short hash too).

#### Scenario: Hash is deterministic for identical inputs
- **WHEN** the same salt, goalId, goalSignature, roundNumber, matchingVerdicts (same order), and matchedAtISO are hashed
- **THEN** both the short hash and fullDigest are identical

#### Scenario: Tampered goalText invalidates the hash
- **WHEN** goalText in `goal.json` is edited after creation
- **THEN** recomputing the fullDigest yields a different value than the stored `completion.json` `fullDigest`
- **AND** the short hash also differs (with overwhelming probability)

#### Scenario: Tampered verdict invalidates the hash
- **WHEN** a stored APPROVE verdict's notes or registeredAt is edited
- **THEN** recomputing the fullDigest yields a different value than the stored `completion.json` `fullDigest`
- **AND** the short hash also differs (with overwhelming probability)

### Requirement: Hash is audit-traceable to its goal directory
Both the short hash and the full digest SHALL be reproducible from the contents of the goal directory plus the salt. An auditor with access to `~/.verifier-loop/goals/<goalId>/` and `.salt` MUST be able to recompute and compare. Audit compares the `fullDigest` field for exact match (deterministic, no collision risk); the short hash is a scannable ID.

#### Scenario: Audit recomputes the hash
- **WHEN** an auditor reads `completion.json` and the goal directory and recomputes the formula
- **THEN** the recomputed `fullDigest` matches the stored `fullDigest` for an untampered goal
- **AND** the recomputed short hash matches the stored `hash`

### Requirement: Completion record is written on success
On consensus the CLI SHALL write `completion.json` containing the short `hash` (`mmddyy-XXXXXXXX`), `fullDigest` (64-hex SHA-256), goalId, roundNumber, matchedAt, and the matchingVerdicts array.

#### Scenario: completion.json is written
- **WHEN** a round reaches consensus
- **THEN** `~/.verifier-loop/goals/<goalId>/completion.json` is written with `hash`, `fullDigest`, goalId, roundNumber, matchedAt, and matchingVerdicts
