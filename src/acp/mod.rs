//! Shared ACP JSON stream parser + backend adapters (tasks.md §4, verifier-spawn spec).
//!
//! Exhaustive `AcpEvent` enum + exhaustive `match` (D0 rationale): an unhandled event type
//! is a compile error, guarding the silent no-SID / no-agent_end bug that breaks fail-closed.
//! Built-in adapters: pi (`pi -p "…" --mode json`), hermes, acpx. Custom adapter via config.

// TODO §4: AcpEvent enum + parser + adapter templates (RED then GREEN, separate fresh teammates).
