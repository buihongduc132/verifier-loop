## MODIFIED Requirements

### Requirement: Hash formula binds goal, round, matching verdicts, and receipt-log head
The completion hash SHALL be `SHA256(salt + goalId + goalSignature + String(roundNumber) + canonicalJSON(matchingVerdicts sorted by verifierId) + matchedAtISO + receiptLogHead)` where `receiptLogHead` is the `entryHash` of the last appended line of `<store>/goals/<goalId>/receipt-log.jsonl` (empty string if the log does not exist). The short form SHALL be `mmddyy-XXXXXXXX` (first 8 hex of the digest); the full 64-hex digest SHALL be stored as `fullDigest`. Identical inputs SHALL yield identical hashes. Consensus evaluation SHALL verify each matching verdict's signature against its slot's pinned pubkey BEFORE computing the hash; unsigned or bad-signature verdicts SHALL NOT be counted as matching.

#### Scenario: Identical inputs yield identical hashes
- **WHEN** the same `(salt, goalId, goalSignature, round, matchingVerdicts, matchedAt, receiptLogHead)` is hashed twice
- **THEN** both short hash and full digest are byte-identical

#### Scenario: Receipt-log head is part of the inputs
- **WHEN** two runs have identical inputs except the receipt-log head
- **THEN** the short hash and full digest differ

#### Scenario: Matching verdicts are signature-verified before counting
- **WHEN** consensus evaluates a round containing an APPROVE verdict whose signature does not verify against the slot's pinned pubkey
- **THEN** that verdict is NOT counted toward n/m
- **AND** the rejection summary names the offending slot and the signature failure

#### Scenario: Tampered goalText invalidates the hash
- **WHEN** `goalText` is edited after the completion hash was stored
- **THEN** recomputing `goalSignature = SHA256(salt + goalText + createdAt)` yields a different value
- **AND** the recomputed completion hash differs from the stored `fullDigest`

#### Scenario: Tampered verdict invalidates the hash
- **WHEN** a verdict's `status` or `notes` or `registeredAt` is edited after the completion hash was stored
- **THEN** signature verification fails (the canonical record bytes changed)
- **AND** recomputing the completion hash yields a digest different from the stored `fullDigest`
