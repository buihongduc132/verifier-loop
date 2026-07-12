## ADDED Requirements

### Requirement: A per-goal newline-delimited JSON trace file is written by default
When the store root is resolvable, `jewilo` and `jewije` SHALL append structured trace records (spans and events) as newline-delimited JSON to `<store>/goals/<goalId>/trace.jsonl`. The file SHALL be created on first write for a goal and appended-to thereafter; it SHALL NOT be rewritten or truncated on subsequent invocations. Each line SHALL be a self-contained JSON object with at minimum `timestamp`, `level`, `traceId`, `goalId`, `span_name` (or `event`), and any structured fields. The file layer SHALL be best-effort: a write error disables the file layer for the rest of the process but does not propagate.

#### Scenario: First jewilo write creates the per-goal trace file
- **WHEN** `jewilo NEW "<goal>"` runs against a fresh goal `abc` in a writable store
- **THEN** `<store>/goals/abc/trace.jsonl` is created
- **AND** its first line is a JSON object containing `"traceId"`, `"goalId":"abc"`, `"timestamp"`, and `"level"`

#### Scenario: RESUME appends to the existing trace file
- **WHEN** `jewilo RESUME abc` runs after a prior NEW for the same goal
- **THEN** the existing `trace.jsonl` is appended to (not truncated)
- **AND** the new lines carry the same `traceId` as the first

#### Scenario: jewije appends to the same per-goal trace file
- **WHEN** a spawned V* process calls `jewije approve` for goal `abc`
- **THEN** a line is appended to `<store>/goals/abc/trace.jsonl`
- **AND** that line's `traceId` equals the `traceId` injected by the spawning `jewilo`

#### Scenario: Unwritable store disables only the file layer
- **WHEN** the store directory is read-only and `trace.jsonl` cannot be created
- **THEN** `jewilo` / `jewije` proceeds normally (verdict registered, hash computed or rejection printed)
- **AND** no error propagates to the caller
- **AND** an `error`-level note is emitted to stderr describing the disabled file layer

### Requirement: OTLP/gRPC export is opt-in behind the otel Cargo feature
An OTLP/gRPC trace exporter SHALL be available only when the crate is compiled with the `otel` Cargo feature AND the `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT` env var is set. When both conditions hold, spans (and their events) SHALL be shipped to the configured collector via OTLP/gRPC using the OpenTelemetry SDK. Resource attributes SHALL include `service.name=verifier-loop`, `service.version=<crate version>`, and SHALL honor standard `OTEL_RESOURCE_ATTRIBUTES` / `OTEL_SERVICE_NAME` env vars. When the `otel` feature is not compiled in, the OTLP code path SHALL be absent from the binary (no OTLP dependency linked).

#### Scenario: otel feature off produces no OTLP dependency
- **WHEN** the crate is built with default features (no `--features otel`)
- **THEN** no `opentelemetry-otlp`, `tonic`, or `tracing-opentelemetry` crate is linked into either binary
- **AND** setting `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT` has no effect

#### Scenario: otel feature on with endpoint ships spans to the collector
- **WHEN** the crate is built with `--features otel` and `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT=http://collector:4317` is set
- **THEN** spans emitted by `jewilo` and `jewije` are shipped to that endpoint via OTLP/gRPC
- **AND** the resource attributes include `service.name=verifier-loop`

#### Scenario: OTLP push failure does not block a verdict
- **WHEN** the OTLP exporter cannot reach the endpoint (e.g. collector down)
- **THEN** the export error is swallowed
- **AND** `jewije` still registers the verdict and `jewilo` still computes the hash or prints the rejection

### Requirement: Log level and stderr format are configurable via env
The `VERIFIER_LOOP_LOG` env var SHALL set the minimum tracing level, accepting `error`, `warn`, `info` (default), `debug`, and `trace` (case-insensitive); an unrecognized value SHALL default to `info` and emit a stderr warning. The `VERIFIER_LOOP_LOG_FORMAT` env var SHALL select the stderr rendering: `text` (human-readable, the legacy behavior — default) or `json` (structured JSON to stderr in addition to the per-goal file). When `VERIFIER_LOOP_LOG` is unset and no OTLP endpoint is configured, stderr tracing output SHALL be byte-identical to the legacy `eprintln!` behavior (i.e. only the existing explicit stderr lines appear).

#### Scenario: VERIFIER_LOOP_LOG=debug raises the verbosity
- **WHEN** `VERIFIER_LOOP_LOG=debug jewilo NEW "<goal>"` is invoked
- **THEN** debug-level spans and events (e.g. per-verifier command build, env injection) are emitted
- **AND** those records appear in `trace.jsonl`

#### Scenario: VERIFIER_LOOP_LOG unset preserves legacy stderr
- **WHEN** `jewilo NEW "<goal>"` is invoked with no `VERIFIER_LOOP_LOG` set
- **THEN** the only stderr output is the legacy lines (`goalId: <id>`, the short hash, or the rejection block)
- **AND** no tracing-generated line appears on stderr

#### Scenario: VERIFIER_LOOP_LOG_FORMAT=json emits structured stderr
- **WHEN** `VERIFIER_LOOP_LOG_FORMAT=json jewilo NEW "<goal>"` is invoked
- **THEN** stderr carries newline-delimited JSON tracing records
- **AND** the per-goal `trace.jsonl` is still written

#### Scenario: Unrecognized level falls back to info with a warning
- **WHEN** `VERIFIER_LOOP_LOG=verbose jewilo ...` is invoked (unrecognized value)
- **THEN** the effective level is `info`
- **AND** a single `warn`-level line is emitted to stderr noting the fallback

### Requirement: A per-goal traceId is persisted and stable across rounds
`jewilo` SHALL resolve exactly one `traceId` per goal: on first entry for a goal, it SHALL mint a random 16-byte id, hex-encode it, and persist it to `<store>/goals/<goalId>/trace-id`. On every subsequent `jewilo` entry for that goal (including RESUME), it SHALL read and reuse the persisted value. The `traceId` SHALL NOT be an input to the completion hash or to the receipt-log `entryHash`. `jewije` SHALL read the `traceId` from `VERIFIER_LOOP_TRACE_ID` env; if absent, it SHALL mint a one-off fallback `traceId` for its own self-correlation (the fallback is NOT persisted to `trace-id`).

#### Scenario: First jewilo entry mints and persists the traceId
- **WHEN** `jewilo NEW "<goal>"` runs for a fresh goal `abc`
- **THEN** `<store>/goals/abc/trace-id` is created containing a 32-hex-character id
- **AND** that id is carried on every span for the invocation

#### Scenario: RESUME reuses the persisted traceId
- **WHEN** `jewilo RESUME abc` runs after a prior NEW
- **THEN** the `traceId` read from `<store>/goals/abc/trace-id` equals the one created at NEW
- **AND** no second `trace-id` file is written

#### Scenario: traceId is not part of the completion hash inputs
- **WHEN** the completion hash is computed for goal `abc` round `1`
- **THEN** the `traceId` does not appear in the hashed input string
- **AND** two runs with identical hash inputs but different `traceId` values produce byte-identical short hash and full digest

#### Scenario: Manual jewije without traceId mints a non-persisted fallback
- **WHEN** `jewije approve` is invoked without `VERIFIER_LOOP_TRACE_ID` in env
- **THEN** a fallback `traceId` is minted for the invocation and carried on its spans
- **AND** no `trace-id` file is written or modified
