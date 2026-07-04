## ADDED Requirements

### Requirement: Verdict record carries an Ed25519 signature
Every non-null `verdict.json` SHALL include `signature` (128-hex Ed25519) and `pubkeyId` (first 16 hex of the pinned pubkey). The signature SHALL cover the canonical bytes of `{status, notes, registeredAt, goalId, verifierId, round}` (serde_json with sorted keys, no whitespace).

#### Scenario: APPROVE verdict is signed
- **WHEN** V* `v1` registers APPROVE for goal `abc` round `1` with its pinned secret
- **THEN** `verdict.json` contains `signature` (128 hex chars) and `pubkeyId` matching the first 16 hex of `v1`'s pinned pubkey
- **AND** the signature verifies against the pinned pubkey over the canonical record bytes

#### Scenario: Signature binds identity fields
- **WHEN** a verdict signed for `(goalId=abc, verifierId=v1, round=1)` is copied into slot `(goalId=abc, verifierId=v2, round=1)`
- **THEN** signature verification fails because the canonical record bytes include `verifierId`
- **AND** consensus evaluation treats the copied verdict as untrusted

### Requirement: Consensus verifies signature before treating verdict as matching
Consensus evaluation SHALL verify each APPROVE verdict's signature against the slot's pinned pubkey BEFORE counting it toward the n/m threshold. A verdict whose signature fails verification SHALL NOT count as matching and SHALL be surfaced in the rejection summary.

#### Scenario: In-flight verdict edit invalidates signature
- **WHEN** a process edits `status` (REJECT → APPROVE) or `notes` or `registeredAt` in an existing signed `verdict.json` after registration but before consensus eval
- **THEN** the signature no longer verifies against the canonical bytes
- **AND** consensus treats the verdict as untrusted (not matching)
- **AND** the rejection summary names the slot and the signature failure

#### Scenario: Verdict signed by non-pinned key fails closed
- **WHEN** a `verdict.json` carries a valid Ed25519 signature but against a pubkey that does not match the slot's pinned `verifier-pubkey.json`
- **THEN** consensus treats the verdict as untrusted (fail-closed)
- **AND** the rejection summary distinguishes "bad signature" from "wrong pubkey"

### Requirement: Null placeholder remains unsigned
The spawn-time `{status: null}` placeholder SHALL NOT carry a signature. Signature verification SHALL apply only to non-null verdicts.

#### Scenario: Null placeholder has no signature field
- **WHEN** jewilo spawns V* `v1`
- **THEN** the pre-created `verdict.json` contains `{status: null}` with no `signature` or `pubkeyId` field
- **AND** consensus does not attempt signature verification on a null placeholder (it is non-matching by definition)
