## ADDED Requirements

### Requirement: Every jewilo lifecycle phase emits a structured span
`jewilo` (the `verifier-loop` binary) SHALL emit a structured tracing span at each public lifecycle phase: goal NEW, goal RESUME, frozen-artifact snapshot capture, per-verifier prompt render, `spawn_round` / `spawn_resume`, per-verifier gather (including the timeout-vs-exit branch), verdict read, consensus evaluate, completion-hash write, and rejection. Each span SHALL carry the structured fields `goalId`, `round` (when known), and `traceId`. Spans SHALL be nested so that per-verifier gather spans are children of the round span, which is a child of the command span.

#### Scenario: NEW round emits a command span with goalId and traceId
- **WHEN** `jewilo NEW "<goal>"` is invoked against a resolvable store
- **THEN** a top-level command span is opened with fields `goalId=<id>` and `traceId=<hex>`
- **AND** a nested `goal::new` span is opened beneath it
- **AND** both spans are recorded with enter and exit timestamps

#### Scenario: RESUME round reuses the goal's existing traceId
- **WHEN** `jewilo RESUME <goalId>` is invoked for a goal that already has a persisted `traceId`
- **THEN** the command span's `traceId` field equals the previously persisted value
- **AND** no new `traceId` is minted

#### Scenario: Per-verifier gather spans are nested under the round span
- **WHEN** `jewilo` spawns `m=3` verifiers for a round
- **THEN** the round span has 3 child spans, one per `verifierId` (`v1`, `v2`, `v3`)
- **AND** each child span records whether the run timed out (`timed_out=true`) or exited normally

#### Scenario: Rejection emits a span event with the rejection summary
- **WHEN** a round fails to reach n/m consensus
- **THEN** the consensus span records a structured event naming the rejecting verifiers, null verifiers, and signature failures
- **AND** the event is a child of the consensus span (not a root event)

### Requirement: Every jewije lifecycle phase emits a structured span
`jewije` (the `verifier-verdict` binary) SHALL emit a structured tracing span covering: identity env resolution, pinned-pubkey read, the regime-gate decision (pinned-vs-secret pairing), and the verdict registration (approve/reject, signed/unsigned). Each span SHALL carry `goalId`, `verifierId`, `round`, `traceId`, and `status` (when decided).

#### Scenario: jewije approve opens a registration span
- **WHEN** `jewije approve` is invoked inside a V* process whose env carries `VERIFIER_LOOP_TRACE_ID`
- **THEN** a registration span is opened with `goalId`, `verifierId`, `round`, `traceId`, and `status=approve`
- **AND** the `traceId` equals the value propagated by the spawning `jewilo`

#### Scenario: Regime-gate decision is recorded as a span field
- **WHEN** `jewije` evaluates whether the slot's pinned pubkey matches the supplied signing secret
- **THEN** the registration span records `regime=<signed|unsigned>` reflecting the gate's decision
- **AND** a `reject` whose gate refused an unauthenticated caller records `regime=refused`

#### Scenario: Manual jewije call without traceId still self-correlates
- **WHEN** `jewije reject --notes "..."` is invoked manually without `VERIFIER_LOOP_TRACE_ID` in env
- **THEN** the registration span carries a freshly minted fallback `traceId`
- **AND** that fallback `traceId` is recorded on the appended receipt-log entry

### Requirement: Span fields use stable lowercase names with camelCase exception for on-disk JSON
Structured span field names in Rust code SHALL be snake_case (e.g. `goal_id`, `verifier_id`, `trace_id`). When a field is persisted to an on-disk JSON artifact (`trace.jsonl`, `receipt-log.jsonl`, `completion.json`), it SHALL be serialized as camelCase (`goalId`, `verifierId`, `traceId`) to match the existing artifact convention. The mapping SHALL be applied via serde rename, not by hand at each call site.

#### Scenario: trace.jsonl uses camelCase field names
- **WHEN** any span or event is written to `<store>/goals/<goalId>/trace.jsonl`
- **THEN** the JSON object keys are camelCase (`goalId`, `verifierId`, `round`, `traceId`)
- **AND** no snake_case key appears in the file

### Requirement: Tracing is fail-open and never alters a verdict or hash
Any error raised by the tracing layer (subscriber init failure, exporter write failure, OTLP push failure, malformed field) SHALL be swallowed and at most logged once to stderr. No tracing error SHALL propagate as an `Err` to `verdict::register_*`, `consensus::evaluate`, `spawn::spawn_round`/`spawn_resume`, or the completion-hash computation. A disabled or broken tracing layer SHALL leave verdict, consensus, and hash outputs byte-identical to a build without tracing.

#### Scenario: Subscriber init failure does not block a verdict
- **WHEN** `observe::init()` returns an error (e.g. unwritable store)
- **THEN** the error is written to stderr and swallowed
- **AND** the subsequent `jewije approve` proceeds and registers the verdict normally
- **AND** the process exit code is unchanged

#### Scenario: File exporter write failure disables only the file layer
- **WHEN** the per-goal `trace.jsonl` cannot be appended (e.g. disk full)
- **THEN** the file layer is disabled for the remainder of the process
- **AND** no `Err` propagates to the caller
- **AND** the verdict / consensus / hash outputs are unchanged

#### Scenario: Completion hash is identical with and without tracing
- **WHEN** the same `(salt, goalId, goalSignature, round, matchingVerdicts, matchedAt, receiptLogHead)` inputs are hashed in a build with tracing fully enabled vs fully disabled
- **THEN** both the short hash and the full digest are byte-identical
