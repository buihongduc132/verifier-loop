# Tasks — add-otel-observability

Implementation roadmap. Follows the repo's standing TDD discipline (`AGENTS.md`):
**RED test by one fresh teammate → GREEN impl by a different fresh teammate → coverage gate `>=80%` lines per new src file before the group is done.**
Every group below is one RED+GREEN pair. Tracing is fail-open (design D5): no group may propagate an `Err` from the observe layer into verdict / consensus / hash.

Reference: proposal.md (why), design.md (how, decisions D0–D7), specs/ (WHAT — each scenario is a test case).

## 1. Setup — dependencies + module skeleton

- [ ] 1.1 Add `tracing = "0.1"` + `tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }` to `[dependencies]` in `Cargo.toml` (always-on).
- [ ] 1.2 Add optional OTLP deps under a new `[features] otel = ["dep:tracing-opentelemetry", "dep:opentelemetry-otlp", "dep:opentelemetry_sdk"]`; pin `tracing-opentelemetry = "0.27"`, `opentelemetry-otlp = { version = "0.16", features = ["grpc-tonic"] }`, `opentelemetry_sdk = { version = "0.23", features = ["rt-tokio"] }`. Verify `cargo build` (default features) and `cargo build --features otel` both succeed.
- [ ] 1.3 Create `src/observe/mod.rs` (and `pub mod observe;` in `src/lib.rs`). Stub the public API surface: `pub fn init() -> Result<(), InitError>`, `pub fn ensure_goal_trace_id(root, goal_id) -> Result<String, io::Error>`, `pub fn current_trace_id() -> Option<String>`, `pub fn with_trace_id<T>(id: &str, f: impl FnOnce() -> T) -> T`. No behavior yet — just type signatures + module doc referencing design D5/D6.
- [ ] 1.4 Write a RED unit test `observe::init_returns_ok_with_no_env` asserting `init()` returns `Ok(())` when no env is set and store is unwritable (the no-op path). RED first; GREEN in 1.5.

## 2. traceId resolution + persistence (trace-export spec)

- [ ] 2.1 RED: `ensure_goal_trace_id_mints_and_persists_on_first_call` — fresh goal dir, no `trace-id` file → call returns a 32-hex id AND writes `<store>/goals/<goalId>/trace-id` with that value. Assert file contents equal returned id.
- [ ] 2.2 RED: `ensure_goal_trace_id_reuses_existing_on_subsequent_call` — pre-write `trace-id` with a known value → call returns that value unchanged and does NOT overwrite the file (assert mtime stable).
- [ ] 2.3 RED: `ensure_goal_trace_id_is_16_bytes_hex` — minted id matches `^[0-9a-f]{32}$` and round-trips through the file.
- [ ] 2.4 GREEN: implement `ensure_goal_trace_id` (random 16 bytes via existing `rand` dep → hex → read-or-write `<goalDir>/trace-id`). Satisfies 2.1–2.3. Different author than the RED tests.
- [ ] 2.5 Coverage gate: `cargo llvm-cov --fail-under-lines 80` for `src/observe/mod.rs`.

## 3. Subscriber init + level/format env (trace-export spec)

- [ ] 3.1 RED: `init_swallows_unwritable_store_and_returns_ok` — point store at a read-only dir; `init()` returns `Ok(())` and a verdict registration afterward still succeeds (design D5 fail-open). Use the stub backend fixture.
- [ ] 3.2 RED: `init_honors_verifier_loop_log_level` — with `VERIFIER_LOOP_LOG=debug`, a `debug!` event is captured (use a `tracing_subscriber::layer` in-memory capture layer in the test); with default unset, `debug!` is NOT captured.
- [ ] 3.3 RED: `init_unrecognized_level_falls_back_to_info_with_warn` — `VERIFIER_LOOP_LOG=verbose` → effective level `info` + exactly one `warn`-level stderr line. (Trap stderr via a test helper.)
- [ ] 3.4 RED: `init_format_json_emits_ndjson_to_stderr` — `VERIFIER_LOOP_LOG_FORMAT=json` → stderr lines are valid JSON objects. `text` (default) → stderr carries only the legacy `eprintln!` lines (byte-identical to pre-change).
- [ ] 3.5 GREEN: implement `init()` building a `tracing_subscriber::Registry` + `EnvFilter` (resolved from `VERIFIER_LOOP_LOG`) + a `fmt` layer (json or text per `VERIFIER_LOOP_LOG_FORMAT`) + the file JSONL layer (group 4) + the `otel`-gated OTLP layer (group 7). All errors swallowed per D5. Different author than RED.
- [ ] 3.6 Coverage gate for `src/observe/mod.rs`.

## 4. File JSONL exporter layer (trace-export spec)

- [ ] 4.1 RED: `first_write_creates_per_goal_trace_jsonl` — `jewilo NEW` against a fresh goal → `<store>/goals/<goalId>/trace.jsonl` exists, first line is a JSON object with keys `timestamp`, `level`, `traceId`, `goalId`, `span_name`.
- [ ] 4.2 RED: `subsequent_invocation_appends_not_truncates` — run NEW then RESUME for the same goal → line count strictly increases; first line's `traceId` == last line's `traceId`.
- [ ] 4.3 RED: `jewije_appends_to_same_file_with_propagated_traceId` — spawn a V* (stub backend) that calls `jewije approve` → a line appears in the goal's `trace.jsonl` whose `traceId` equals the `jewilo`-minted value.
- [ ] 4.4 RED: `camelcase_keys_in_trace_jsonl` — assert NO snake_case key (`goal_id`, `verifier_id`, `trace_id`) appears in any line; only camelCase. (Span fields use serde rename — design lifecycle-tracing spec.)
- [ ] 4.5 RED: `write_failure_disables_file_layer_only` — make the goal dir read-only after first write; subsequent events do not panic and no `Err` propagates; an `error`-level stderr note appears once.
- [ ] 4.6 GREEN: implement the file layer as a `tracing_subscriber::Layer` writing newline-delimited JSON via `tracing_subscriber::fmt::format::Json` + a `MakeWriter` over an append-opened `File` (re-open per write to survive external truncation, or hold a `Mutex<File>`). Different author than RED.
- [ ] 4.7 Coverage gate for the file-layer source file.

## 5. Instrumentation — jewilo + jewije lifecycle spans (lifecycle-tracing spec)

- [ ] 5.1 RED: `jewilo_new_emits_command_span_with_goalId_and_traceId` — run `jewilo NEW` with an in-memory capture layer → a top-level span with `goalId` + `traceId` fields is opened and closed; nested `goal::new` span is a child.
- [ ] 5.2 RED: `jewilo_resume_reuses_existing_traceId` — NEW then RESUME → both invocations' command spans carry the SAME `traceId` (read from `trace-id` file).
- [ ] 5.3 RED: `per_verifier_gather_spans_nested_under_round` — `m=3` round → the round span has 3 child spans named per `verifierId`; the timeout branch records `timed_out=true` on the killed V* (use a stub that sleeps past `verifierTimeoutSec`).
- [ ] 5.4 RED: `rejection_emits_structured_event_under_consensus_span` — failed round → a consensus-span event naming rejecting + null verifiers + signature failures (assert the event is a child of the consensus span, not a root).
- [ ] 5.5 RED: `jewije_approve_span_carries_propagated_traceId` — spawn V* with `VERIFIER_LOOP_TRACE_ID=T` → `jewije`'s registration span has `traceId=T`, `status=approve`, `regime=signed`.
- [ ] 5.6 RED: `jewije_regime_gate_records_regime_field` — refused-unauthenticated path → registration span records `regime=refused`.
- [ ] 5.7 GREEN: add `#[tracing::instrument(skip(_), fields(goal_id, round, trace_id))]` (or explicit `tracing::info_span!`) to the public entry points listed in design D6: `store::Config::load_in`, `store::salt_in`, `goal::new/resume/load`, `spawn::spawn_round/spawn_resume`, `gather` (per-V* span inside the loop), `verdict::register_*` + `read_verdict` + `mint_and_pin_pubkey`, `consensus::evaluate/compute_hash/write_completion`, `receipt::append_entry/read_receipt_head`, and both `bin/main` top-level spans. Different author than the RED tests.
- [ ] 5.8 GREEN: wire `observe::init()` into both `bin/main` before `run()` — `let _ = observe::init(...)` (swallow per D5). Keep all existing `eprintln!` as the legacy `text`-format fallback.
- [ ] 5.9 Coverage gate for any newly-touched src file that is new logic (not just attribute additions).

## 6. traceId propagation + receipt-log/completion metadata (verifier-spawn, receipt-log, completion-proof deltas)

- [ ] 6.1 RED: `spawn_injects_verifier_loop_trace_id_to_every_v` — spawn `m=2` → both child processes' env contains `VERIFIER_LOOP_TRACE_ID` equal to `<store>/goals/<goalId>/trace-id`. (Inspect via a stub backend that prints its env.)
- [ ] 6.2 RED: `receipt_log_entry_has_traceId_field_excluded_from_entryHash` — append two entries identical except `traceId`; assert both `entryHash` are byte-identical AND both lines record their respective `traceId`.
- [ ] 6.3 RED: `completion_json_has_traceId_metadata_not_in_hash_inputs` — reach consensus → `completion.json` has a `"traceId"` field; recompute the hash with a different `traceId` value (but identical hash inputs) → `hash` + `fullDigest` byte-identical; assert `traceId` substring does NOT appear in the hashed input string.
- [ ] 6.4 RED: `manual_jewije_without_trace_id_mints_nonpersisted_fallback` — invoke `jewije approve` with no `VERIFIER_LOOP_TRACE_ID` → receipt entry has a `traceId` field (the fallback) AND `<store>/goals/<goalId>/trace-id` is NOT created or modified.
- [ ] 6.5 GREEN: (a) add `VERIFIER_LOOP_TRACE_ID` to `identity_env_pairs` in `spawn/orchestrator.rs`, sourced from `observe::ensure_goal_trace_id(root, goal_id)` called once at the top of `spawn_round`/`spawn_resume`; (b) extend the receipt-log entry struct with a serde `traceId` field excluded from the canonical hash tuple; (c) add `traceId` to `completion.json` write in `consensus::write_completion`, sourced from the goal's `trace-id`, NOT folded into `compute_hash`; (d) `jewije` reads `VERIFIER_LOOP_TRACE_ID`, else mints a fallback via `observe` and tags spans + passes it into `receipt::append_entry`. Different author than RED.
- [ ] 6.6 RED: `completion_hash_byte_identical_with_and_without_tracing` — same inputs, tracing fully enabled vs fully disabled (no `VERIFIER_LOOP_LOG`, no file layer) → short hash + full digest identical. This is the D4 / lifecycle-tracing fail-closed invariant test.
- [ ] 6.7 Coverage gate for `spawn/orchestrator.rs`, `receipt/mod.rs`, `consensus/mod.rs` new code paths.

## 7. OTLP/gRPC exporter (otel feature, opt-in) (trace-export spec)

- [ ] 7.1 RED (only compiled with `--features otel`): `otel_feature_off_links_no_otlp_dep` — `cargo tree --edges normal --no-default-features` asserts no `opentelemetry-otlp` / `tonic` / `tracing-opentelemetry` node. (Can be a `build-deps`-checking test or a CI grep; document in the test comment.)
- [ ] 7.2 RED (otel feature): `otel_endpoint_set_ships_spans` — with `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT` pointing at a local in-process OTLP receiver (test helper), run `jewilo NEW` → the receiver observes at least one span with resource attribute `service.name=verifier-loop`.
- [ ] 7.3 RED (otel feature): `otel_push_failure_does_not_block_verdict` — endpoint points at a closed port → `jewije approve` still registers the verdict; exit code 0.
- [ ] 7.4 GREEN (otel feature): implement the OTLP layer behind `#[cfg(feature = "otel")]` in `src/observe/otlp.rs`: build `opentelemetry_sdk::trace::Tracer` with `opentelemetry_otlp::new_pipeline().grpc()`, wrap in `tracing_opentelemetry::layer()`, push onto the registry only when `init()` sees the endpoint env. Flush on drop / explicit `shutdown`. Different author than RED.
- [ ] 7.5 Coverage gate for `src/observe/otlp.rs` (best-effort — OTLP paths may need `#[cfg_attr]` test gating).

## 8. Docs + wiring + final gates

- [ ] 8.1 Update `AGENTS.md` module map: add the `observe` row (§this change | lifecycle-tracing + trace-export). Add a short "Observability" subsection pointing at the new env vars and `trace.jsonl` (and stating the fail-open + hash-input-unchanged invariants).
- [ ] 8.2 Update `README.md`: store-layout section gains `trace.jsonl` + `trace-id`; new "Observability / Tracing" section documenting `VERIFIER_LOOP_LOG`, `VERIFIER_LOOP_LOG_FORMAT`, `VERIFIER_LOOP_TRACE_ID`, the `otel` feature + `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT`. State plainly that `traceId` is metadata, not a hash input.
- [ ] 8.3 Update `THREAT-MODEL.md`: add a one-paragraph note that `trace.jsonl` + `traceId` are observability metadata, not tamper-evident evidence; the receipt-log remains the evidence ledger.
- [ ] 8.4 Full-suite gates: `cargo test --all-features`, `cargo llvm-cov --fail-under-lines 80`, `cargo clippy --all-features -- -D warnings`, `cargo fmt --check`.
- [ ] 8.5 Hermetic e2e: extend the existing stub-backend e2e (tests/*.rs) to assert the per-goal `trace.jsonl` is created, carries the propagated `traceId`, and that the completion hash is unchanged by tracing. (Confirms fail-closed + trace-export spec end-to-end.)
- [ ] 8.6 Open question resolution: record Q1 (traceId rotate per round?) / Q2 (default stderr format?) / Q3 (OTLP logs signal?) decisions in `design.md` once tasks §2/§3/§7 land, or leave as documented open questions if deferred.

## Out of scope (do NOT implement — design.md Non-Goals)

- Metrics (counters / histograms / exemplars).
- Cross-host distributed tracing (W3C tracecontext across the subprocess boundary; OTLP from a V* on a separate host).
- Sampling policy beyond `VERIFIER_LOOP_LOG` level.
- Retroactive tracing of pre-change goals.
- Tamper-protection of `trace.jsonl` (it is observation, not evidence).
