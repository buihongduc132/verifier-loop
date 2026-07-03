//! Shared ACP JSON stream parser + backend adapters (tasks.md §4, verifier-spawn spec).
//!
//! Exhaustive `AcpEvent` enum + exhaustive `match` (D0 rationale): an unhandled event type
//! is a compile error, guarding the silent no-SID / no-agent_end bug that breaks fail-closed.
//! Built-in adapters: pi (`pi -p "…" --mode json`), hermes, acpx. Custom adapter via config.
//!
//! Module layout:
//! * [`parser`]    — `AcpEvent` enum, `parse_event`, `extract_sid`, `extract_final_output`.
//! * [`adapters`]  — built-in `Adapter` templates (pi/hermes/acpx) + `render_spawn`/`render_resume`.

mod adapters;
mod parser;

pub use adapters::{adapter_for, render_resume, render_spawn, Adapter};
pub use parser::{extract_final_output, extract_sid, parse_event, AcpError, AcpEvent, Message};
