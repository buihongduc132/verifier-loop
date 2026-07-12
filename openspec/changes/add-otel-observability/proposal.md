## Why

Every diagnostic the `jewilo`/`jewije` CLIs emit today is an `eprintln!` to stderr — unstructured, timestamp-less, and uncorrelated across the process tree. A failed round surfaces only a few `REJECT`/`null`/`signature failure` lines on the console; reconstructing *what happened when* (which V* was spawned, when it timed out, which `jewije` call registered which verdict, where the spawn gathered) means manually cross-reading `verdict.json`, `meta.json`, `stderr.txt`, and `receipt-log.jsonl` with no shared timeline or causal link. There is no way to follow one goal's lifecycle end-to-end, and no way to ship the trail to a collector for later debugging. This change adds OpenTelemetry-style structured logging + tracing so the full `jewilo`/`jewije`/spawn lifecycle is observable and debuggable after the fact.

## What Changes

- **Structured tracing instrumentation.** Add the `tracing` crate and emit spans + events around every lifecycle phase of both binaries:
  - `jewilo`: goal NEW, goal RESUME, snapshot capture, per-verifier prompt render, `spawn_round`/`spawn_resume`, per-verifier spawn + gather, consensus evaluate, completion-hash write, rejection.
  - `jewije`: verdict registration (approve/reject, signed/unsigned), pinned-pubkey read, regime gate decision.
  - shared library (`store`, `goal`, `spawn`, `verdict`, `consensus`, `receipt`): span enter at each public entry point with the `goalId`/`verifierId`/`round` as structured fields.
- **Correlation id across the process tree.** `jewilo` mints a per-goal `traceId` (stable across NEW + RESUME rounds) and propagates it to spawned V* processes via a new `VERIFIER_LOOP_TRACE_ID` env var; `jewije` picks it up so a verdict registration is joins the same trace as the spawn that launched it. The `traceId` is also recorded on the receipt-log entry and on `completion.json` so an auditor can find the full trail from the completion hash.
- **Pluggable exporters (defaults to off; file when on).**
  - **File exporter (default-on-when-configured):** writes a newline-delimited JSON span/event stream to `<store>/goals/<goalId>/trace.jsonl`, one file per goal — a zero-dependency, always-available local trail that mirrors the receipt-log's per-goal layout.
  - **OTLP exporter (opt-in):** when `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT` is set, spans + logs are shipped to an OpenTelemetry collector via OTLP/gRPC. Compiled behind a feature flag (`otel`) so the default build pulls no OTLP deps.
- **Log level control.** `VERIFIER_LOOP_LOG` (env) sets the level (`error`/`warn`/`info`/`debug`/`trace`, default `info`); `VERIFIER_LOOP_LOG_FORMAT` selects `text` (human-readable stderr, the legacy behavior) vs `json` (structured). When tracing is fully off, behavior is byte-identical to today (the `eprintln!` paths are preserved as the `text` fallback).
- **No behavior change to verdicts, consensus, or the completion hash.** Tracing is a pure observation layer: it never alters a verdict, never short-circuits fail-closed paths, and the completion-hash inputs are unchanged (the `traceId` is *recorded on* `completion.json` but is NOT an input to the hash — it is metadata, not evidence).

## Capabilities

### New Capabilities
- `lifecycle-tracing`: structured spans + events covering the full `jewilo` (NEW/RESUME/spawn/gather/consensus/hash) and `jewije` (approve/reject/register) lifecycle, with `goalId`/`verifierId`/`round`/`traceId` as correlated fields; the contract for which lifecycle phases emit which spans.
- `trace-export`: the pluggable exporter contract — a local newline-delimited JSON `trace.jsonl` per goal (default), an opt-in OTLP/gRPC exporter behind the `otel` feature flag, and the `VERIFIER_LOOP_LOG` / `VERIFIER_LOOP_LOG_FORMAT` / `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT` env contract; fail-closed guarantee that a tracing failure never alters a verdict.

### Modified Capabilities
- `verifier-spawn`: the spawn layer SHALL accept and propagate a `traceId` into each V* process env (`VERIFIER_LOOP_TRACE_ID`), and SHALL open a span around each per-verifier spawn + gather so the orchestrator's view of a run is correlated with the V* side.
- `receipt-log`: each receipt-log entry SHALL record the active `traceId`, so an auditor can pivot from a receipt entry to the full span trail.
- `completion-proof`: `completion.json` SHALL record the goal's `traceId` as metadata (NOT as a hash input — hash inputs are unchanged); this is a new convenience field for audit, not new evidence.

## Impact

- **Code**: new `src/observe/` module (subscriber init, exporter wiring, `traceId` mint/propagate, span helpers); instrumentation added to `src/bin/verifier_loop.rs`, `src/bin/verifier_verdict.rs`, `src/spawn/orchestrator.rs`, and the public entry points of `src/store/`, `src/goal/`, `src/verdict/`, `src/consensus/`, `src/receipt/`; `src/lib.rs` gains a `pub mod observe;`.
- **Dependencies**: add `tracing` + `tracing-subscriber` (always); `tracing-opentelemetry` + `opentelemetry-otlp` + `opentelemetry_sdk` behind a new `otel` Cargo feature (off by default). All optional deps are gated so a default build adds only the `tracing` surface.
- **APIs/CLI**: no breaking CLI changes. New env vars: `VERIFIER_LOOP_LOG`, `VERIFIER_LOOP_LOG_FORMAT`, `VERIFIER_LOOP_TRACE_ID` (read by jewije, written by jewilo/spawn), `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT` (+ standard `OTEL_*` resource attrs). New file: `<store>/goals/<goalId>/trace.jsonl`. New metadata field on `completion.json`: `traceId` (no hash-input change).
- **Specs**: two new specs (`lifecycle-tracing`, `trace-export`); three modified delta specs (`verifier-spawn`, `receipt-log`, `completion-proof`).
- **Fail-closed invariants preserved**: tracing errors are swallowed and never propagate to a verdict or consensus decision; the hash inputs are byte-identical; a NULL verdict still never becomes APPROVE; a missing store still yields no hash.
- **Out of scope (explicit non-goals)**: metrics (counters/histograms) — logs + traces only for now; distributed tracing across an out-of-process V* on a separate host (requires network OTLP infra, deferred); sampling policy beyond `VERIFIER_LOOP_LOG` level; retroactive tracing of already-completed goals.
