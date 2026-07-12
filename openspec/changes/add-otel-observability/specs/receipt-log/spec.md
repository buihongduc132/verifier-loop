## MODIFIED Requirements

### Requirement: Every jewije write appends to a hash-chained receipt log
Every successful `jewije approve` / `jewije reject` invocation SHALL append exactly one line to `<store>/goals/<goalId>/receipt-log.jsonl`. Each line SHALL be a JSON object `{seq, kind, verdictId, status, prevHash, entryHash, signedBy, traceId}` where `entryHash = SHA256(prevHash || canonicalEntryFields)` and `signedBy` is the `pubkeyId` of the signing key. The first entry's `prevHash` SHALL be the empty string. The `traceId` field SHALL carry the active trace id (from `VERIFIER_LOOP_TRACE_ID` env, or the fallback minted by `jewije` when unset). The `traceId` SHALL be EXCLUDED from the `canonicalEntryFields` hashed into `entryHash` — it is observability metadata, not evidence; two entries identical except for `traceId` SHALL produce identical `entryHash` values.

#### Scenario: Approve appends one chained entry
- **WHEN** V* `v1` registers APPROVE for goal `abc` round `1` (receipt log empty)
- **THEN** `receipt-log.jsonl` gains one line with `seq=1`, `prevHash=""`, `entryHash=SHA256("" || canonicalFields)`, `signedBy=<v1 pubkeyId>`, `traceId=<active traceId>`
- **AND** the line is appended (the file is not rewritten)

#### Scenario: Subsequent entry chains the previous
- **WHEN** V* `v2` registers APPROVE for the same goal after `v1`
- **THEN** the new line has `seq=2` and `prevHash=<v1 entryHash>`
- **AND** `entryHash=SHA256(<v1 entryHash> || canonicalFields)`

#### Scenario: traceId is recorded but not hashed
- **WHEN** two `jewije approve` invocations for the same slot produce identical `(seq, kind, verdictId, status, prevHash, signedBy)` but carry different `traceId` values
- **THEN** both entries' `entryHash` values are byte-identical
- **AND** the `traceId` field of each line differs as recorded

#### Scenario: Manual jewije records its fallback traceId
- **WHEN** `jewije approve` is invoked without `VERIFIER_LOOP_TRACE_ID` in env
- **THEN** the appended line's `traceId` field carries the fallback id minted by `jewije`
- **AND** that fallback id is not persisted to `<store>/goals/<goalId>/trace-id`
