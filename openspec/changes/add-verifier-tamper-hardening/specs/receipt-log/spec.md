## ADDED Requirements

### Requirement: Every jewije write appends to a hash-chained receipt log
Every successful `jewije approve` / `jewije reject` invocation SHALL append exactly one line to `<store>/goals/<goalId>/receipt-log.jsonl`. Each line SHALL be a JSON object `{seq, kind, verdictId, status, prevHash, entryHash, signedBy}` where `entryHash = SHA256(prevHash || canonicalEntryFields)` and `signedBy` is the `pubkeyId` of the signing key. The first entry's `prevHash` SHALL be the empty string.

#### Scenario: Approve appends one chained entry
- **WHEN** V* `v1` registers APPROVE for goal `abc` round `1` (receipt log empty)
- **THEN** `receipt-log.jsonl` gains one line with `seq=1`, `prevHash=""`, `entryHash=SHA256("" || canonicalFields)`, `signedBy=<v1 pubkeyId>`
- **AND** the line is appended (the file is not rewritten)

#### Scenario: Subsequent entry chains the previous
- **WHEN** V* `v2` registers APPROVE for the same goal after `v1`
- **THEN** the new line has `seq=2` and `prevHash=<v1 entryHash>`
- **AND** `entryHash=SHA256(<v1 entryHash> || canonicalFields)`

### Requirement: Receipt log head is folded into the completion hash inputs
On consensus, the receipt log's chain head (the `entryHash` of the last appended line) SHALL be appended to the completion-hash input string alongside `matchingVerdicts` and `matchedAtISO`. The hash formula becomes `SHA256(salt + goalId + goalSignature + String(roundNumber) + canonicalJSON(matchingVerdicts) + matchedAtISO + receiptLogHead)`.

#### Scenario: Completion hash includes receipt log head
- **WHEN** consensus is reached for goal `abc` round `1` after two APPROVE writes
- **THEN** the stored `completion.json` `hash` and `fullDigest` reflect the receipt-log head as part of the inputs
- **AND** an auditor recomputing the hash without the receipt-log head produces a different digest

### Requirement: Receipt log tampering is detectable by an auditor
A retroactive edit, deletion, or insertion of a line in `receipt-log.jsonl` SHALL break the hash chain (a `prevHash`/`entryHash` mismatch) OR the stored completion hash (head mismatch). An auditor re-reading the log and recomputing the chain SHALL detect the break.

#### Scenario: Mid-log edit breaks the chain
- **WHEN** a process edits the `status` field of an entry in the middle of `receipt-log.jsonl`
- **THEN** recomputing `entryHash` for that line yields a value different from the stored `entryHash`
- **AND** every subsequent line's `prevHash` no longer matches the recomputed `entryHash`

#### Scenario: Trailing-line deletion breaks the completion hash
- **WHEN** a process deletes the last line of `receipt-log.jsonl` after `completion.json` was written
- **THEN** the recomputed chain head differs from the head folded into the stored completion hash
- **AND** auditor recompute of the completion hash yields a digest different from the stored `fullDigest`

### Requirement: Empty receipt log on a fresh goal
A fresh goal SHALL have no `receipt-log.jsonl` until the first verdict write. The completion-hash input for an empty log SHALL be the empty-string head.

#### Scenario: Consensus with zero receipt entries is impossible
- **WHEN** consensus is attempted on a goal whose `receipt-log.jsonl` does not exist
- **THEN** the round fails to reach consensus (no signed APPROVE verdicts can exist without a receipt entry)
- **AND** no `completion.json` is written
