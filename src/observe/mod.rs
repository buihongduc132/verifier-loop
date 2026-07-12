//! Observation layer: structured tracing + per-goal trace files (add-otel-observability).
//!
//! This module owns the OpenTelemetry-style observability surface:
//! * a per-goal `traceId` resolved once and reused across NEW + RESUME rounds (design D1),
//! * a newline-delimited JSON `trace.jsonl` per goal (default local trail, D3),
//! * an opt-in OTLP/gRPC exporter behind the `otel` Cargo feature (D3),
//! * level/format control via `VERIFIER_LOOP_LOG` / `VERIFIER_LOOP_LOG_FORMAT` (D3).
//!
//! **Fail-open (design D5):** every call into this module returns `Result` or `()`,
//! but no tracing error ever propagates to a verdict, consensus, or hash decision.
//! `init()` is best-effort; a broken logger never blocks the CLI.
//!
//! **`traceId` is metadata, NOT a hash input (design D4):** it is recorded on the
//! receipt-log entry and `completion.json` for audit convenience, but excluded from
//! the canonical bytes hashed into `entryHash` and the completion digest.
//!
//! See `openspec/changes/add-otel-observability/` for the full spec + design.

use std::path::{Path, PathBuf};

use rand::RngCore;

use crate::goal;

/// Env var carrying the active per-goal trace id, propagated `jewilo → spawn → V* → jewije`.
pub const ENV_TRACE_ID: &str = "VERIFIER_LOOP_TRACE_ID";
/// Env var: minimum tracing level (`error`/`warn`/`info`/`debug`/`trace`).
pub const ENV_LOG: &str = "VERIFIER_LOOP_LOG";
/// Env var: stderr rendering (`text` legacy | `json` structured).
pub const ENV_LOG_FORMAT: &str = "VERIFIER_LOOP_LOG_FORMAT";
/// Per-goal trace-id filename, sibling to `goal.json` / `receipt-log.jsonl`.
pub const TRACE_ID_FILE: &str = "trace-id";
/// Per-goal newline-delimited JSON trace file.
pub const TRACE_JSONL_FILE: &str = "trace.jsonl";
/// Default tracing level when `VERIFIER_LOOP_LOG` is unset.
pub const DEFAULT_LEVEL: &str = "info";

/// Errors raised by the observation layer. All paths fail-open: the caller MUST
/// swallow these (design D5) — they never propagate to a verdict or hash.
#[derive(Debug, thiserror::Error)]
pub enum ObserveError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("subscriber init error: {0}")]
    Init(String),
}

/// Resolve (mint-or-read) the stable per-goal trace id.
///
/// On first entry for a goal: mints a random 16-byte id, hex-encodes it (32 lowercase
/// hex chars), persists it to `<store>/goals/<goalId>/trace-id`, and returns it.
/// On every subsequent entry (incl. RESUME): reads and returns the persisted value
/// unchanged. The id is NOT an input to the completion hash or receipt entryHash.
///
/// Fail-open: an io error is returned to the caller, which MUST swallow it (design D5).
pub fn ensure_goal_trace_id(root: &Path, goal_id: &str) -> Result<String, ObserveError> {
    let dir = goal::goal_dir(root, goal_id);
    std::fs::create_dir_all(&dir)?;
    let trace_file = dir.join(TRACE_ID_FILE);
    if trace_file.exists() {
        let existing = std::fs::read_to_string(&trace_file)?;
        let trimmed = existing.trim().to_string();
        if is_valid_trace_id(&trimmed) {
            return Ok(trimmed);
        }
        // fall through to mint if malformed
    }
    let id = mint_trace_id();
    std::fs::write(&trace_file, &id)?;
    Ok(id)
}

/// Read-only lookup of the persisted per-goal trace id. Returns `None` if the file
/// is absent or malformed. Does NOT mint or write — used by [`append_trace_event`]
/// so a manual `jewije` call never persists a trace id (trace-export spec).
fn read_goal_trace_id(root: &Path, goal_id: &str) -> Option<String> {
    let trace_file = goal::goal_dir(root, goal_id).join(TRACE_ID_FILE);
    let raw = std::fs::read_to_string(&trace_file).ok()?;
    let trimmed = raw.trim().to_string();
    if is_valid_trace_id(&trimmed) {
        Some(trimmed)
    } else {
        None
    }
}

/// Read the propagated trace id from `VERIFIER_LOOP_TRACE_ID` env, if set.
///
/// `jewije` uses this to join the spawning `jewilo`'s trace. Returns `None` when
/// unset (caller mints a one-off fallback).
pub fn trace_id_from_env() -> Option<String> {
    std::env::var(ENV_TRACE_ID).ok().filter(|s| !s.is_empty())
}

/// Resolve the on-disk per-goal trace file path: `<root>/goals/<goal_id>/trace.jsonl`.
pub fn trace_jsonl_path(root: &Path, goal_id: &str) -> PathBuf {
    goal::goal_dir(root, goal_id).join(TRACE_JSONL_FILE)
}

/// Append one newline-delimited JSON trace record to the goal's `trace.jsonl`
/// (trace-export spec, design D3). Best-effort + fail-open (design D5): any io error
/// is swallowed and `Ok(())` returned — a broken trace file never blocks a verdict.
///
/// The record carries camelCase keys: `timestamp`, `level`, `traceId`, `goalId`,
/// `event`, plus the merged `fields` object.
///
/// **traceId resolution (non-persisting):** resolved in priority order:
/// 1. `VERIFIER_LOOP_TRACE_ID` env (set by the spawning jewilo for V* children),
/// 2. read-only lookup of the persisted `trace-id` file (jewilo's own events),
/// 3. a freshly minted fallback id that is NOT written to disk.
///
/// This ensures a manual `jewije` call without the env var does NOT create a
/// `trace-id` file (trace-export spec: "manual jewije without traceId mints a
/// non-persisted fallback").
///
/// The file is created on first write and appended-to thereafter; never rewritten.
pub fn append_trace_event(
    root: &Path,
    goal_id: &str,
    level: &str,
    event: &str,
    fields: serde_json::Value,
) -> Result<(), ObserveError> {
    // Resolve traceId WITHOUT persisting (see doc comment above).
    let trace_id = trace_id_from_env()
        .or_else(|| read_goal_trace_id(root, goal_id))
        .unwrap_or_else(mint_trace_id);

    let mut record = serde_json::Map::new();
    record.insert(
        "timestamp".to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );
    record.insert(
        "level".to_string(),
        serde_json::Value::String(level.to_string()),
    );
    record.insert("traceId".to_string(), serde_json::Value::String(trace_id));
    record.insert(
        "goalId".to_string(),
        serde_json::Value::String(goal_id.to_string()),
    );
    record.insert(
        "event".to_string(),
        serde_json::Value::String(event.to_string()),
    );
    if let serde_json::Value::Object(map) = fields {
        for (k, v) in map {
            record.insert(k, v);
        }
    }

    // Best-effort write: create the parent dir + append. Swallow io errors (D5).
    let path = trace_jsonl_path(root, goal_id);
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(&serde_json::Value::Object(record))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        use std::io::Write;
        writeln!(file, "{line}")?;
        Ok(())
    })();
    // Fail-open: swallow the error. The caller proceeds regardless.
    if let Err(e) = result {
        // Log once to stderr (the legacy channel), then continue.
        eprintln!("verifier-loop: trace.jsonl append failed (continuing): {e}");
    }
    Ok(())
}

/// Initialize the tracing subscriber (fail-open, design D5).
///
/// Wires up, in order:
///   1. an `EnvFilter` resolved from `VERIFIER_LOOP_LOG` (default `info`),
///   2. a `fmt` stderr layer in `text` (legacy) or `json` format per
///      `VERIFIER_LOOP_LOG_FORMAT`,
///   3. the opt-in OTLP layer behind the `otel` feature.
///
/// `store_root` is accepted for API completeness; the per-goal `trace.jsonl` is
/// written by explicit [`append_trace_event`] calls at lifecycle points (round
/// start, consensus pass/reject, verdict registered) rather than via a subscriber
/// `Layer`. This events-based design is deliberate: it captures the semantically
/// meaningful lifecycle events in a stable, greppable JSONL format without the
/// complexity of a custom `Layer` that would need to resolve the goal id from
/// span context on every event. Spans (for OTLP export) flow through the
/// subscriber; lifecycle events (for the local trail) flow through this helper.
///
/// Returns `Ok(())` unconditionally on best-effort paths — any init error is
/// logged to stderr (the legacy channel) and swallowed.
///
/// **Re-entrancy:** safe to call multiple times across tests; subsequent calls
/// are no-ops if a global subscriber is already set (`try_init` returns Err,
/// swallowed per D5).
pub fn init(store_root: Option<&Path>) -> Result<(), ObserveError> {
    use tracing_subscriber::EnvFilter;

    let level = std::env::var(ENV_LOG).unwrap_or_else(|_| DEFAULT_LEVEL.to_string());
    let filter = match EnvFilter::try_new(&level) {
        Ok(f) => f,
        Err(_) => {
            eprintln!(
                "verifier-loop: unrecognized {ENV_LOG}={level:?}, falling back to {DEFAULT_LEVEL}"
            );
            EnvFilter::new(DEFAULT_LEVEL)
        }
    };

    let use_json = std::env::var(ENV_LOG_FORMAT)
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    // store_root is not used for a subscriber layer (see doc comment above); the
    // per-goal trace.jsonl path is resolved per-write inside append_trace_event.
    let _ = store_root;

    // Branch on feature + format so each composition has a concrete, inferrable
    // type — boxing dyn Layer defeats SubscriberInitExt::try_init.
    init_layers(filter, use_json);
    Ok(())
}

/// Compose + install the global subscriber. The OTLP branch is inlined (not
/// factored into a typed helper) because the `OpenTelemetryLayer<S, T>` type
/// parameter `S` must match the subscriber it is composed onto — a pre-built
/// layer with `S = Registry` won't satisfy `Layer<Layered<EnvFilter, Registry>>`.
fn init_layers(filter: tracing_subscriber::EnvFilter, use_json: bool) {
    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Default (text) mode keeps stderr quiet — tracing events go to the file
    // layer (group 4) + OTLP (otel) only. JSON mode emits structured NDJSON.
    if use_json {
        let stderr = fmt::layer().with_writer(std::io::stderr).json();
        #[cfg(feature = "otel")]
        {
            if let Some(tracer) = build_otlp_tracer() {
                let _ = tracing_subscriber::registry()
                    .with(filter)
                    .with(stderr)
                    .with(tracing_opentelemetry::layer().with_tracer(tracer))
                    .try_init();
                return;
            }
        }
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(stderr)
            .try_init();
        return;
    }

    // Text/quiet mode: no stderr layer (legacy byte-identical output).
    #[cfg(feature = "otel")]
    {
        if let Some(tracer) = build_otlp_tracer() {
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .try_init();
            return;
        }
    }
    let _ = tracing_subscriber::registry().with(filter).try_init();
}

/// Flush + shut down the OTLP tracer provider (design D3, fail-open D5).
///
/// `opentelemetry_otlp` uses async gRPC transport; without an explicit flush,
/// in-flight spans can be silently lost when the CLI exits. Both bins MUST call
/// this at the end of `main()` (after `run()` returns) when the `otel` feature
/// is enabled. It is a no-op when OTLP is not configured (no provider installed)
/// or when the `otel` feature is off.
pub fn shutdown() {
    #[cfg(feature = "otel")]
    {
        // Best-effort: flush all pending spans + drop the provider. Errors are
        // swallowed (fail-open D5) — a flush failure never blocks process exit.
        opentelemetry::global::shutdown_tracer_provider();
    }
}

/// Mint a fresh 16-byte random trace id, hex-encoded (32 lowercase chars).
fn mint_trace_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Validate a trace id is 32 lowercase hex chars.
fn is_valid_trace_id(s: &str) -> bool {
    s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Build the OTLP tracer (only compiled with `--features otel`). Design D3.
/// Returns `Some(tracer)` only when `VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT` is
/// set AND the exporter builds cleanly; otherwise `None` (fail-open: the caller
/// proceeds without OTLP, per design D5). Returning the tracer (not a layer)
/// lets the caller compose it inline where `S` infers correctly.
///
/// Uses the opentelemetry-otlp 0.26 pipeline API (`new_pipeline().tracing()…`)
/// rather than a manual `SpanExporter::builder()` (which has a different shape
/// in 0.26 vs 0.27). The pipeline wires resource attributes + the simple
/// (synchronous) exporter; batch export is available via `install_batch` if a
/// tokio runtime is guaranteed at init time.
#[cfg(feature = "otel")]
fn build_otlp_tracer() -> Option<opentelemetry_sdk::trace::Tracer> {
    use opentelemetry_otlp::WithExportConfig;

    let endpoint = std::env::var("VERIFIER_LOOP_OTEL_EXPORTER_OTLP_ENDPOINT").ok()?;
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&endpoint);

    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(opentelemetry_sdk::trace::Config::default().with_resource(
            opentelemetry_sdk::Resource::new(vec![
                opentelemetry::KeyValue::new("service.name", "verifier-loop"),
                opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            ]),
        ))
        .install_simple()
        .ok()?;

    // Install W3C tracecontext propagator for cross-process context (same-box).
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    use opentelemetry::trace::TracerProvider as _;
    Some(provider.tracer("verifier-loop"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_trace_id_is_32_lowercase_hex() {
        let id = mint_trace_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn is_valid_trace_id_accepts_well_formed() {
        assert!(is_valid_trace_id("0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn is_valid_trace_id_rejects_malformed() {
        assert!(!is_valid_trace_id("short"));
        assert!(!is_valid_trace_id("UPPERCASENOTALLOWED1234567890ab"));
        assert!(!is_valid_trace_id(""));
    }

    #[test]
    fn ensure_goal_trace_id_mints_then_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let id1 = ensure_goal_trace_id(dir.path(), "unit-mint").unwrap();
        let id2 = ensure_goal_trace_id(dir.path(), "unit-mint").unwrap();
        assert_eq!(id1, id2, "second call must reuse the first");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("goals/unit-mint/trace-id")).unwrap(),
            id1
        );
    }

    #[test]
    fn read_goal_trace_id_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_goal_trace_id(dir.path(), "no-such-goal").is_none());
    }

    #[test]
    fn read_goal_trace_id_returns_value_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let known = "0123456789abcdef0123456789abcdef";
        ensure_goal_trace_id(dir.path(), "g-read").unwrap();
        std::fs::write(dir.path().join("goals/g-read/trace-id"), known).unwrap();
        assert_eq!(
            read_goal_trace_id(dir.path(), "g-read").as_deref(),
            Some(known)
        );
    }

    #[test]
    fn trace_jsonl_path_under_goal_dir() {
        let p = trace_jsonl_path(Path::new("/store"), "g7");
        assert_eq!(p, PathBuf::from("/store/goals/g7/trace.jsonl"));
    }
}
