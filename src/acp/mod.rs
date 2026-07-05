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
