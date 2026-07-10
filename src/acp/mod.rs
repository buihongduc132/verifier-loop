//! Shared ACP JSON stream parser + backend adapters (tasks.md §4, verifier-spawn spec).
//!
//! Exhaustive `AcpEvent` enum + exhaustive `match` (D0 rationale): an unhandled event type
//! is a compile error, guarding the silent no-SID / no-agent_end bug that breaks fail-closed.
//! Built-in adapters (post fix-spawn-argv-overflow §8/D6): pi/hermes/acpx all use the
//! stdin transport — `pi --mode json` etc., with the prompt piped to the child's stdin
//! (no `{prompt}` token in argv). Custom adapter via config.
//!
//! Module layout:
//! * [`parser`]    — `AcpEvent` enum, `parse_event`, `extract_sid`, `extract_final_output`.
//! * [`adapters`]  — built-in `Adapter` templates (pi/hermes/acpx) + `render_spawn`/`render_resume`.

mod adapters;
mod parser;

pub use adapters::{adapter_for, render_resume, render_spawn, Adapter, Transport};
pub use parser::{extract_final_output, extract_sid, parse_event, AcpError, AcpEvent, Message};

// ─── per-verifier-adapter: resolve_adapters ──────────────────────────────────
//
// Resolve one Adapter per verifier slot (m entries). When `config.verifiers` is
// present, each entry defines a slot's adapter (built-in key or custom spawn/resume
// templates). When absent, the legacy `backend` field is replicated m times.
// Custom/stub backends fall back to env-var resolution (same as the bin's
// resolve_adapter).
// ─────────────────────────────────────────────────────────────────────────────

/// Env vars for custom/stub backend command resolution (mirrors bin/verifier_loop.rs).
const ENV_BACKEND_CMD: &str = "VERIFIER_LOOP_BACKEND_CMD";
const ENV_SPAWN_CMD: &str = "VERIFIER_LOOP_SPAWN_CMD";
const ENV_RESUME_CMD: &str = "VERIFIER_LOOP_RESUME_CMD";

/// Resolve adapters for all verifier slots.
///
/// Returns exactly `config.m` adapters. When `config.verifiers` is `Some`, each
/// entry is resolved in slot order (v1, v2, ...). When `None`, the legacy
/// `backend` field is replicated for all m slots.
pub fn resolve_adapters(config: &crate::store::Config) -> Result<Vec<Adapter>, String> {
    let m = config.m as usize;

    if let Some(verifiers) = &config.verifiers {
        if verifiers.len() != m {
            return Err(format!(
                "verifiers array length ({}) must equal m ({}), but does not",
                verifiers.len(),
                m
            ));
        }
        verifiers
            .iter()
            .map(|vc| resolve_one(&vc.adapter, vc.spawn.as_deref(), vc.resume.as_deref(), vc.transport))
            .collect()
    } else {
        // Legacy: replicate the backend adapter m times.
        let adapter = resolve_one(&config.backend, None, None, Transport::Stdin)?;
        Ok(vec![adapter; m])
    }
}

/// Resolve a single adapter from a backend key + optional custom templates.
fn resolve_one(
    backend: &str,
    custom_spawn: Option<&str>,
    custom_resume: Option<&str>,
    transport: Transport,
) -> Result<Adapter, String> {
    // Built-in adapters
    if let Ok(a) = adapter_for(backend) {
        // If custom spawn/resume templates are provided, override them
        if custom_spawn.is_some() || custom_resume.is_some() {
            return Ok(Adapter {
                spawn: custom_spawn.map(String::from).unwrap_or(a.spawn),
                resume: custom_resume.map(String::from).unwrap_or(a.resume),
                transport,
            });
        }
        return Ok(a);
    }

    // Custom/stub: use env vars or provided templates
    let spawn = if let Some(s) = custom_spawn {
        s.to_string()
    } else {
        std::env::var(ENV_BACKEND_CMD)
            .or_else(|_| std::env::var(ENV_SPAWN_CMD))
            .map_err(|_| {
                format!(
                    "unknown backend '{}' and no ${}/{} override set",
                    backend, ENV_BACKEND_CMD, ENV_SPAWN_CMD
                )
            })?
    };
    let resume = if let Some(r) = custom_resume {
        r.to_string()
    } else {
        std::env::var(ENV_RESUME_CMD).unwrap_or_else(|_| spawn.clone())
    };

    Ok(Adapter {
        spawn,
        resume,
        transport,
    })
}
