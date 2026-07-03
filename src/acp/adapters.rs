//! Built-in backend adapters (tasks.md §4.2 / §4.3 / §4.4, verifier-spawn spec).
//!
//! Every built-in adapter (pi, hermes, acpx) provides two command *templates*:
//!
//! * `spawn` — start a fresh verifier session.
//! * `resume` — continue an existing session on its captured SID.
//!
//! Templates carry `{prompt}` and `{sid}` placeholders rendered by [`render_spawn`] /
//! [`render_resume`]. Every backend's output is parsed by the **shared** ACP parser
//! (`crate::acp::parser`); only the invocation differs per backend.
//!
//! Custom adapters (§4.4) are constructed from `config.json` spawn/resume templates via
//! [`Adapter::custom`]; the orchestrator wires them at spawn time. Custom adapters MUST
//! conform to the ACP output format (the parser does not special-case any backend).
//!
//! Fail-closed: [`adapter_for`] errors on an unknown backend rather than silently falling
//! back to pi (a wrong backend would emit an unparseable stream and a null verdict).

use super::parser::AcpError;

/// A backend adapter: a pair of spawn/resume command templates sharing `{prompt}` and
/// `{sid}` placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// Fresh-spawn template, e.g. `pi -p "{prompt}" --mode json`.
    pub spawn: String,
    /// Resume template, e.g. `pi --session {sid} -p "{prompt}" --mode json`.
    pub resume: String,
}

impl Adapter {
    /// Construct a custom adapter from explicit templates (§4.4). Used by the orchestrator
    /// when `config.json` declares `backend: "custom"`. The shared ACP parser consumes
    /// whatever the rendered command emits.
    #[allow(dead_code)] // wired by §5 spawn orchestration; kept minimal here.
    pub fn custom(spawn: String, resume: String) -> Self {
        Self { spawn, resume }
    }
}

/// Resolve a built-in adapter by backend name.
///
/// Built-ins: `pi`, `hermes`, `acpx`. Returns `Err` for any unknown name (fail-closed:
/// never silently fall back to pi, which would produce an unparseable stream and a null
/// verdict indistinguishable from a real crash).
pub fn adapter_for(backend: &str) -> Result<Adapter, AcpError> {
    match backend {
        "pi" => Ok(Adapter {
            spawn: r#"pi -p "{prompt}" --mode json"#.to_string(),
            resume: r#"pi --session {sid} -p "{prompt}" --mode json"#.to_string(),
        }),
        "hermes" => Ok(Adapter {
            spawn: r#"hermes -p "{prompt}" --mode json"#.to_string(),
            resume: r#"hermes --session {sid} -p "{prompt}" --mode json"#.to_string(),
        }),
        "acpx" => Ok(Adapter {
            spawn: r#"acpx -p "{prompt}" --mode json"#.to_string(),
            resume: r#"acpx --session {sid} -p "{prompt}" --mode json"#.to_string(),
        }),
        other => Err(AcpError::BadEventShape(format!(
            "unknown backend '{other}' (expected one of: pi, hermes, acpx, or custom)"
        ))),
    }
}

/// Render a spawn template by substituting `{prompt}` with the given prompt.
///
/// Pure substitution — no shell escaping. The orchestrator splits the rendered command on
/// whitespace (§5); prompts needing quoting are the caller's responsibility at the spawn
/// layer, not the template layer.
pub fn render_spawn(template: &str, prompt: &str) -> String {
    template.replace("{prompt}", prompt)
}

/// Render a resume template by substituting `{sid}` and `{prompt}`.
pub fn render_resume(template: &str, sid: &str, prompt: &str) -> String {
    template.replace("{sid}", sid).replace("{prompt}", prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_templates_match_spec() {
        let a = adapter_for("pi").unwrap();
        assert_eq!(a.spawn, r#"pi -p "{prompt}" --mode json"#);
        assert_eq!(a.resume, r#"pi --session {sid} -p "{prompt}" --mode json"#);
    }

    #[test]
    fn hermes_templates_match_spec() {
        let a = adapter_for("hermes").unwrap();
        assert!(a.spawn.contains("hermes") && a.spawn.contains("--mode json"));
        assert!(a.resume.contains("--session") && a.resume.contains("{sid}"));
    }

    #[test]
    fn acpx_templates_match_spec() {
        let a = adapter_for("acpx").unwrap();
        assert!(a.spawn.contains("acpx") && a.spawn.contains("--mode json"));
    }

    #[test]
    fn unknown_backend_errors() {
        assert!(adapter_for("claude").is_err());
    }

    #[test]
    fn render_spawn_replaces_all_prompt_placeholders() {
        let t = "x {prompt} {prompt} y";
        assert_eq!(render_spawn(t, "P"), "x P P y");
    }

    #[test]
    fn render_resume_replaces_sid_then_prompt() {
        let t = "{sid}:{prompt}";
        assert_eq!(render_resume(t, "S", "P"), "S:P");
    }

    #[test]
    fn custom_adapter_round_trips() {
        let a = Adapter::custom("a {prompt}".into(), "b {sid} {prompt}".into());
        assert_eq!(render_spawn(&a.spawn, "P"), "a P");
        assert_eq!(render_resume(&a.resume, "S", "P"), "b S P");
    }
}
