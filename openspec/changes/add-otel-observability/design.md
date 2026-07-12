## Context

The `verifier-loop` crate ships two binaries — `jewilo` (`verifier-loop`) and `jewije` (`verifier-verdict`) — that together drive an n/m verifier consensus round. The entire observable surface today is a handful of `eprintln!` calls in the two `src/bin/*.rs` files: round rejection prints `REJECT` notes / null-verifier list / signature failures to stderr, and `jewije` prints a one-line error on failure. There is:

- **No structured logging** — everything is free-text stderr.
- **No timestamps** on any diagnostic line.
- **No correlation id** linking a `jewilo` round to the `jewije` calls its spawned V* processes make, nor linking NEW → RESUME rounds of the same goal.
- **No trace spans** around the async spawn/gather barrier, so when a V* times out or crashes the only post-mortem is `stderr.txt` + the null `verdict.json`.

The lifecycle that needs to be trackable end-to-end:

```
jewilo NEW/RESUME
  ├─ store::Config::load_in          (fail-closed: missing store → no hash)
  ├─ goal::new / goal::resume        (immutable signed goal)
  ├─ prompt::capture_snapshot        (frozen artifact)
  ├─ prompt::render (per V*)         (m verifier prompts)
  ├─ spawn::spawn_round/resume       (m concurrent children)
  │    ├─ per V*: mint_verifier_secret, inject env (incl. traceId)
  │    ├─ per V*: child.spawn()      (non-blocking launch)
  │    └─ gather barrier             (per-V* timeout, stdout/stderr drain)
  ├─ verdict::read_verdict (per V*)
  ├─ consensus::evaluate             (n/m, signature verify)
  └─ completion hash write  OR  rejection print
                                   jewije approve/reject (inside each V* process)
  ├─ resolve identity env
  ├─ regime gate (pinned pubkey ↔ secret)
  ├─ verdict::register_*  (signed/unsigned)
  └─ receipt-log append
```

Constraints that any design MUST preserve (these are the project's fail-closed invariants, see `AGENTS.md`):

- A NULL verdict never becomes APPROVE.
- A missing store yields no hash.
- `goalText` edit → signature mismatch → hash mismatch.
- Verdict edit → hash mismatch.
- The completion-hash **inputs** are exactly `(salt, goalId, goalSignature, roundNumber, canonicalJSON(matchingVerdicts), matchedAtISO, receiptLogHead)`. Observation must not extend this set.

Stakeholders: any agent or human debugging a failed round; the outer driving agent (per `AGENTS.md` the `.jewilo-*` bloat files come from that outer agent, not the binary — tracing must not add to CWD bloat).

## Goals / Non-Goals

**Goals:**
- Every lifecycle phase of both binaries emits a structured span with `goalId` / `verifierId` / `round` / `traceId` fields, so a single goal's full history (NEW → round 1 spawn → V* verdicts → RESUME → round N → consensus) is reconstructable.
- A correlation `traceId` propagates `jewilo → spawn → V* env → jewije → receipt-log → completion.json`, so an auditor holding a completion hash can find the span trail.
- A zero-dependency local trail exists by default: a per-goal `trace.jsonl` newline-delimited JSON file under the store, mirroring the receipt-log's layout.
- An opt-in OTLP/gRPC exporter ships spans + logs to a collector for centralized debugging, behind a Cargo feature so default builds are untouched.
- Legacy stderr behavior is preserved as a `text` format option; when tracing is off, output is byte-identical to today.
- Tracing is fail-safe: any error in the observation layer is swallowed and never propagates to a verdict, consensus, or hash decision.

**Non-Goals:**
- Metrics (counters / histograms / exemplars). Logs + traces only. A later change can add metrics on top of the `observe` module.
- Distributed tracing across a V* on a *separate host* (requires network OTLP infra + W3C tracecontext propagation across the subprocess boundary — deferred; same-box `traceId` env propagation is in scope).
- Sampling policy more sophisticated than `VERIFIER_LOOP_LOG` level. Parent-based sampling comes free from `tracing`, but custom ratio sampling is out of scope.
- Retroactive tracing of goals completed before this change ships.
- Changing the completion-hash input set. `traceId` is recorded *on* `completion.json` as metadata, never folded into the digest.
- Protecting `trace.jsonl` from tampering (it is an observation log, not evidence; the receipt-log already provides tamper-evident evidence).

## Decisions

### D0 — Use `tracing` (not `log` + `slog` + hand-rolled)
**Decision:** adopt the `tracing` crate + `tracing-subscriber` as the sole observation facade.

**Why:** `tracing` gives spans (nested, with structured fields) AND events (logs) in one crate, is the de-facto Rust standard, and its subscriber layer lets us fan out to multiple exporters (file JSONL + OTLP) without touching instrumentation sites. `log` alone has no spans; `slog` is structurally similar but has a smaller ecosystem and no OTLP bridge; hand-rolling a span tree is reinventing `tracing`. The instrumentation is `#[instrument]` attributes + `info!`/`debug!`/`error!` macros — low-touch, compile-time-checked field names.

**Alternatives considered:**
- `log` + `env_logger`: rejected — no spans, no structured field correlation, no async-aware span context. Would give us "structured stderr" but not the cross-process trace.
- `slog`: rejected — equivalent structure but `tracing`'s OTLP bridge (`tracing-opentelemetry`) is what we need for the opt-in collector path, and `slog` has no equivalent first-class bridge.
- Hand-rolled JSONL: rejected for instrumentation (we'd re-derive span enter/exit, field formatting, level filtering), but the *file exporter* is hand-rolled on top of `tracing-subscriber`'s `fmt` layer (see D3).

### D1 — Per-goal `traceId`, stable across NEW + RESUME rounds
**Decision:** `jewilo` resolves a `traceId` per goal: it reads `<store>/goals/<goalId>/trace-id` on entry; if absent it mints a random 16-byte id (hex-encoded) and persists it. The same id is reused for every RESUME of that goal.

**Why:** a stable per-goal id lets an auditor pivot from any span/event/receipt entry/completion record to the *entire* goal history, not just one round. A per-invocation id would fragment NEW from RESUME. Persisting it (rather than deriving from `goalId`) keeps it independent of the hash inputs (D4) and lets the id rotate only on explicit goal recreation.

**Why not W3C `traceparent` across the subprocess boundary:** the spawned V* is a separate process and (for the stub) a separate language; full W3C propagation would require the V* backend to speak `traceparent`. We propagate only our own `VERIFIER_LOOP_TRACE_ID` env (which `jewije` understands natively), and the OTLP exporter (D3) maps our `traceId` onto an OTLP `TraceId` when shipping. True cross-host distributed tracing is a non-goal.

### D2 — Propagate `traceId` via env, record on receipt-log + completion.json
**Decision:**
- `jewilo`'s spawn layer injects `VERIFIER_LOOP_TRACE_ID=<hex>` into every V* child env (alongside the existing `VERIFIER_LOOP_GOAL_ID` / `_VERIFIER_ID` / `_ROUND` / `_HOME` / `_VERIFIER_SECRET`).
- `jewije` reads `VERIFIER_LOOP_TRACE_ID` (if set) and tags every span/event with it; if unset it mints a fallback per-invocation id (so a manual `jewije` call is still self-consistent).
- Each receipt-log entry gains a `traceId` field (the active id). The hash-chained `entryHash` inputs are UNCHANGED — `traceId` is appended to the JSON object but excluded from the canonical hash input string (see D4).
- `completion.json` gains a `traceId` metadata field. This is NOT a hash input.

**Why env over a file:** the spawn layer already passes 5 env vars to each V*; one more is zero new mechanism and survives the subprocess fork/exec. A file would race with concurrent V* reads.

### D3 — Exporter strategy: local JSONL file default, OTLP opt-in behind a feature flag
**Decision:** two exporters, layered via `tracing-subscriber`'s `Layer` model:

1. **File JSONL layer (default-on-when-store-resolvable):** writes newline-delimited JSON to `<store>/goals/<goalId>/trace.jsonl`. One file per goal, mirroring `receipt-log.jsonl`. Implemented as a custom `Layer` over `tracing_subscriber::fmt::format::Json` (or a thin `MakeWriter` pointing at an append-opened `File`). Append-only, best-effort — a write error is logged once to stderr and the layer is disabled for the rest of the process.
2. **OTLP layer (opt-in, behind `otel` Cargo feature):** when compiled with `--features otel` AND `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT` is set, a `tracing-opentelemetry` layer + an `opentelemetry-otlp` exporter ships spans to a collector. Resource attributes (`service.name=verifier-loop`, `service.version`, plus standard `OTEL_*` env) are set per the OTel SDK spec. The exporter is initialized once in `observe::init()` and flushed on process exit.

Level control: `VERIFIER_LOOP_LOG` (default `info`) gates both layers; `VERIFIER_LOOP_LOG_FORMAT` selects `text` (human-readable stderr — legacy) vs `json` (structured stderr, in addition to the file). When `VERIFIER_LOOP_LOG` is unset AND no OTLP endpoint is set AND the store is unresolvable, the subscriber is a no-op (zero overhead — `tracing` macros compile to nothing when no subscriber is active for that level).

**Why a per-goal file (not a single global file):** mirrors `receipt-log.jsonl`'s layout, makes "give me the trail for goal X" a single `cat`, and avoids concurrent-append contention across goals (each goal's `jewilo` is serialized by the user, but two different goals may run concurrently).

**Why OTLP behind a feature flag:** `opentelemetry-otlp` + `tonic` (gRPC) + `tokio` full runtime pull in a sizable dependency tree; users who only want the local trail should not pay that cost. The `otel` feature keeps it opt-in.

**Alternatives considered:**
- Single global `~/.verifier-loop/trace.jsonl`: rejected — concurrent goals interleave, and the file grows unbounded; per-goal files are garbage-colrollable with the goal dir.
- OTLP-only (no local file): rejected — removes the zero-infra local trail, which is the primary debugging artifact for this project.
- `tracing-tree` (human-readable nested spans to stderr): kept as the `text` format option, but not the default for the file (JSONL is greppable + collector-ingestible).

### D4 — Tracing is NOT part of the completion-hash inputs (fail-closed observation)
**Decision:** the completion-hash input set is unchanged: `SHA256(salt + goalId + goalSignature + String(round) + canonicalJSON(matchingVerdicts) + matchedAtISO + receiptLogHead)`. `traceId` is added to `completion.json` and to the receipt-log entry JSON object, but is **excluded** from the canonical bytes hashed into `receiptLogHead` (and hence from the completion hash).

**Why:** the completion hash is a *tamper-evident evidence* artifact; the trace trail is *observability*. Mixing them would (a) make the hash depend on observation state, breaking reproducibility, and (b) force every tracing field into the security model. Keeping them separate preserves the existing threat model (`THREAT-MODEL.md`): the hash binds evidence, the trail aids debugging.

**Mechanism:** in `receipt::append_entry`, the canonical fields hashed into `entryHash` are explicitly the existing set; `traceId` is written to the JSON object after hashing (or in a `#[serde(skip)]`-style separation — the hash function takes the canonical field tuple, not the serialized struct). A unit test pins "traceId does not appear in entryHash inputs".

### D5 — Fail-closed: tracing errors never alter a verdict
**Decision:** every call into the `observe` module returns `()` or is a macro that expands to `if let Err(_)=… { /* swallow */ }`. The subscriber init in `observe::init()` is fallible but its error is only ever logged to stderr (never returned to `run()` in the bins, never propagated to `verdict::register_*` or `consensus::evaluate`).

**Why:** the project's core invariants are fail-closed for *evidence* (NULL never APPROVE, missing store no hash). Observation must be the opposite — fail-*open* for observability, so a broken logger never blocks a verdict or poisons consensus. A tracing panic would be a regression.

**Concrete guardrail:** `observe::init()` is called at the very top of `main()` in both bins, wrapped in `let _ =` (errors go to stderr via `eprintln!`, the legacy channel). No `?` propagation from any observe call into the verdict/consensus/spawn code paths.

### D6 — New `src/observe/` module, instrumentation at module public boundaries
**Decision:** a new `src/observe/` module owns: subscriber init, exporter wiring, `trace_id()` / `set_trace_id()` / `ensure_goal_trace_id(root, goal_id)`, and thin span-helper macros. Instrumentation (the `#[instrument]` attributes and `info!`/`warn!`/`error!` calls) is added at the **public entry points** of each existing module, not scattered through internals:

| Module | Instrumented entry points |
|--------|--------------------------|
| `store::Config::load_in`, `store::salt_in` | span + fields |
| `goal::new`, `goal::resume`, `goal::load` | span (goal_id, round) |
| `spawn::spawn_round`, `spawn::spawn_resume`, `gather` | span per V* (verifier_id, round, timed_out) |
| `verdict::register_approve/reject`, `register_signed_*`, `read_verdict`, `mint_and_pin_pubkey` | span (goal_id, verifier_id, round, status) |
| `consensus::evaluate`, `compute_hash`, `write_completion` | span (round, passed, approve_count) |
| `receipt::append_entry`, `read_receipt_head` | span (seq) |
| both `bin/main` | top-level `jewilo`/`jewije` span (trace_id, goal_id, command) |

**Why public boundaries:** internal helpers (`round_dir`, `build_spawn_command`, `pre_create_verifier_dir`) are too noisy and too likely to be refactored; the public API is the stable surface that maps to the lifecycle phases a debugger thinks in.

### D7 — Async-aware spans in the spawn orchestrator
**Decision:** the `spawn_round`/`spawn_resume`/`gather` functions are `async`; their `#[instrument]` attributes use the async-aware path (`tracing` automatically enters/exits the span across `.await` points when the `tracing` macro is applied to an async fn). Per-V* spans are created inside the gather loop with `tracing::info_span!("verifier", verifier_id, round)` and entered via an `in_scope` around the `tokio::select!` so the timeout vs. exit branch is recorded under the correct V*.

**Why:** the gather barrier is where most "why did this round fail" questions are answered (timeout? crash? EPIPE? bad signature?). Without per-V* spans the m concurrent children collapse into one undifferentiated gather span.

## Risks / Trade-offs

- **[OTLP dependency weight]** `opentelemetry-otlp` + `tonic` + `tokio` full pull in ~30 transitive crates. → *Mitigation:* gated behind the `otel` feature; default `cargo build` adds only `tracing` + `tracing-subscriber`. Documented in `Cargo.toml` + README.
- **[trace.jsonl grows unbounded]** a chatty backend at `trace` level over many rounds can produce large per-goal files. → *Mitigation:* default level is `info`; `trace.jsonl` lives under the goal dir and is removed when the goal dir is removed; document a rotation/cleanup recipe (out of scope to auto-rotate here).
- **[traceId persistence is a new file under the store]** `<store>/goals/<goalId>/trace-id` is a new artifact. → *Mitigation:* it is not evidence (not a hash input), it is orphan-safe (an auditor ignores it without consequence), and it is tiny (32 hex bytes). Documented in README's store-layout section.
- **[Subscriber init failure on a misconfigured OTLP endpoint]** a bad `OTLP_ENDPOINT` could panic at init. → *Mitigation:* D5 — init errors are swallowed + logged to stderr; the process continues with tracing disabled. A unit test asserts "init with a bad endpoint does not panic and returns an error the caller swallows".
- **[Per-V* span flooding on large m]** with `m=5` and many rounds, the file gets verbose. → *Mitigation:* this is the point (debuggability); level filtering (`VERIFIER_LOOP_LOG=warn`) is the escape hatch. No silent sampling in scope.
- **[Tracing attributes on hot paths add overhead]** `#[instrument]` on `read_verdict` (called m times per round) has a measurable cost. → *Mitigation:* `tracing` macros are no-ops when no subscriber is active for the level; the file layer is append-only (no lock contention beyond the file's own). Acceptable for a CLI that spawns m subprocesses (the spawn cost dominates).
- **[traceId must not leak into hash inputs]** a future refactor could accidentally fold it in. → *Mitigation:* D4 unit test pinning "traceId absent from entryHash inputs" + a consensus test pinning "completion hash is byte-identical with and without tracing enabled".
- **[Concurrent goals writing to per-goal files]** two `jewilo` invocations for *different* goals each write their own `trace.jsonl` — no contention. Two for the *same* goal are serialized by the user (the store is single-writer per goal). → *Mitigation:* the file layer opens in append mode; OS-level append atomicity for small writes is sufficient. No cross-process locking added (matches receipt-log's existing advisory-lock stance).
- **[Tracing a crashed backend]** if a V* process segfaults, the spawn gather span records the non-zero exit but the V* side has no `jewije` span (it never called jewije). → *Mitigation:* acceptable — the orchestrator-side span + `stderr.txt` already capture this; cross-process tracing of a segfaulted V* is out of scope.

## Migration Plan

This is an additive, opt-in-by-default-off change. No existing behavior changes when the new env vars are unset.

1. **Add deps** (`tracing`, `tracing-subscriber` always; `tracing-opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk` under `otel` feature) → `cargo build` still green.
2. **Land `src/observe/`** with subscriber init, `trace_id` helpers, file JSONL layer, and the `otel`-gated OTLP layer. Unit tests for init + trace_id resolution + fail-closed swallowing.
3. **Wire `observe::init()`** into both `bin/main` functions (before `run()`). Legacy `eprintln!` paths kept as the `text` format fallback.
4. **Instrument public entry points** module-by-module under TDD (RED test per group, GREEN by a different author — the repo's standing discipline).
5. **Propagate `traceId`** through spawn env + jewije read + receipt-log entry + completion.json metadata. Pin the D4 invariant (traceId not in hash inputs) with a unit test.
6. **No breaking release.** Users who set `VERIFIER_LOOP_LOG=info` (or higher) + `VERIFIER_LOOP_LOG_FORMAT=json` get the new trail; users who set nothing see byte-identical output. OTLP users opt in via `--features otel` + endpoint env.

**Rollback:** remove the `observe` module calls and the env-var reads; the binaries revert to `eprintln!`-only. The `traceId`/`trace.jsonl`/`completion.json.traceId` artifacts are inert metadata and can be ignored or deleted. No on-disk evidence format migration is needed because hash inputs are unchanged.

## Open Questions

- **Q1 — Should `traceId` rotate per round (NEW=trace1, RESUME=trace2, …)?** Current decision (D1): stable per goal, so the full history is one trace. Alternative: per-round ids linked via parent span. Lean stable-per-goal unless a debugging use case demands per-round isolation. *Resolve during tasks §1.*
- **Q2 — Default for `VERIFIER_LOOP_LOG_FORMAT`: `text` or `json`?** Current decision: `text` (legacy-preserving). `json` is opt-in. If the file layer is always JSONL regardless, the format switch only governs *stderr* — confirm this is the desired surface. *Resolve during tasks §3.*
- **Q3 — Should the OTLP exporter also ship logs (via `opentelemetry-logs`), or spans only?** `tracing-opentelemetry` ships spans; logs-as-events ride along as span events. If a collector wants the OTel *logs* signal specifically, that needs `opentelemetry-logs` (separate, less mature). Current decision: spans + span-events only; revisit if a collector integration demands the logs signal. *Resolve when first OTLP user asks.*
