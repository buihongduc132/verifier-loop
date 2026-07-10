//! Built-in backend adapters (tasks.md §4.2 / §4.3 / §4.4, verifier-spawn spec;
//! fix-spawn-argv-overflow §8 — D6 built-in template migration).
//!
//! Every built-in adapter (pi, hermes, acpx) provides two command *templates*:
//!
//! * `spawn` — start a fresh verifier session (e.g. `pi --mode json`).
//! * `resume` — continue an existing session on its captured SID
//!   (e.g. `pi --session {sid} --mode json`).
//!
//! Built-in templates carry NO `{prompt}` placeholder — the prompt travels on the
//! child's stdin pipe (`transport = Stdin`, design D1/D6), so argv contains zero
//! prompt-derived bytes (no E2BIG regardless of prompt size). `{sid}` is the only
//! placeholder in a built-in template (resume only).
//!
//! [`render_spawn`] / [`render_resume`] still substitute `{prompt}` for legacy /
//! programmatic callers, but the built-in adapters no longer use it. Custom adapters
//! that try to load `{prompt}` through serde are rejected at config load. Every
//! backend's output is parsed by the **shared** ACP parser (`crate::acp::parser`);
//!
//! Custom adapters (§4.4) are constructed from `config.json` spawn/resume templates via
//! [`Adapter::custom`]; the orchestrator wires them at spawn time. Custom adapters MUST
//! conform to the ACP output format (the parser does not special-case any backend).
//!
//! Fail-closed: [`adapter_for`] errors on an unknown backend rather than silently falling
//! back to pi (a wrong backend would emit an unparseable stream and a null verdict).

use serde::{Deserialize, Serialize};

use super::parser::AcpError;

/// How the orchestrator delivers the rendered verifier prompt to the spawned
/// process (fix-spawn-argv-overflow design D1; prompt-transport spec).
///
/// * `Stdin` — write the prompt to the child's stdin pipe. Default for all
///   built-in adapters (`pi`, `hermes`, `acpx`). The argv contains NO
///   prompt-derived bytes.
/// * `GoalFile` — write the prompt to a tempfile and substitute its path via
///   the `{goalFile}` placeholder.
///
/// The JSON representation is kebab-case: `"stdin"` <-> `Stdin`,
/// `"goal-file"` <-> `GoalFile`. Unknown values are rejected by serde
/// (fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    Stdin,
    GoalFile,
}

impl Default for Transport {
    fn default() -> Self {
        // Built-in adapters default to the stdin transport (design D1/D6).
        Self::Stdin
    }
}

/// A backend adapter: a pair of spawn/resume command templates plus the prompt
/// transport (fix-spawn-argv-overflow design D1/D2; prompt-transport spec).
///
/// Built-in adapters (`pi`, `hermes`, `acpx`) are produced by [`adapter_for`] with
/// `transport = Stdin`. Custom adapters come from `config.json` via serde, where
/// `transport` is REQUIRED and the `{prompt}` placeholder is REJECTED — see the
/// custom [`Deserialize`] impl below.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Adapter {
    /// Fresh-spawn template. Built-in pi: `pi --mode json` (design D6; prompt on
    /// stdin, no `{prompt}` token). Custom adapters MUST NOT contain `{prompt}`;
    /// use `{goalFile}` with `transport: "goal-file"`, or `transport: "stdin"`
    /// with no placeholder.
    pub spawn: String,
    /// Resume template, e.g. built-in pi: `pi --session {sid} --mode json`.
    /// Same `{prompt}` restriction as `spawn`.
    pub resume: String,
    /// How the orchestrator delivers the rendered prompt to the child. Built-ins
    /// default to [`Transport::Stdin`]; custom adapters MUST set this explicitly.
    pub transport: Transport,
}

/// Custom deserialization (fix-spawn-argv-overflow §1.3 / prompt-transport spec):
/// fail-closed at config load if a custom adapter omits `transport` OR uses the
/// legacy inline `{prompt}` placeholder. The error message names both
/// `{goalFile}` and `transport: "goal-file"` so users know the migration path.
impl<'de> Deserialize<'de> for Adapter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// Field helper used by the visitor.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct AdapterDef {
            spawn: String,
            resume: String,
            // No `#[serde(default)]`: `transport` is REQUIRED for custom adapters.
            transport: Transport,
        }

        let def = AdapterDef::deserialize(deserializer)?;

        // Reject the legacy inline `{prompt}` placeholder (the root cause of
        // gh #1 / E2BIG). Name both `{goalFile}` and `goal-file` in the message
        // so the migration path is unambiguous.
        if def.spawn.contains("{prompt}") {
            return Err(serde::de::Error::custom(format!(
                "`spawn` template contains the forbidden `{{prompt}}` placeholder \
                 (this inlines the prompt into argv and triggers E2BIG). \
                 Migrate to `{{goalFile}}` with `transport: \"goal-file\"`, \
                 or use `transport: \"stdin\"` and drop the placeholder entirely. \
                 Spawn template was: {}",
                def.spawn
            )));
        }
        if def.resume.contains("{prompt}") {
            return Err(serde::de::Error::custom(format!(
                "`resume` template contains the forbidden `{{prompt}}` placeholder \
                 (this inlines the prompt into argv and triggers E2BIG). \
                 Migrate to `{{goalFile}}` with `transport: \"goal-file\"`, \
                 or use `transport: \"stdin\"` and drop the placeholder entirely. \
                 Resume template was: {}",
                def.resume
            )));
        }

        Ok(Adapter {
            spawn: def.spawn,
            resume: def.resume,
            transport: def.transport,
        })
    }
}

impl Adapter {
    /// Construct a custom adapter from explicit templates (§4.4). Used by the orchestrator
    /// when `config.json` declares `backend: "custom"`. The shared ACP parser consumes
    /// whatever the rendered command emits. NOTE: this constructor is for in-process use;
    /// config-file custom adapters go through serde (which enforces the `{prompt}` ban).
    #[allow(dead_code)] // wired by §5 spawn orchestration; kept minimal here.
    pub fn custom(spawn: String, resume: String) -> Self {
        Self {
            spawn,
            resume,
            // Built-in programmatic adapters default to stdin (design D1/D6).
            // `{prompt}` here is tolerated because this path is not user-facing
            // config; serde validation only applies to `config.json` loads.
            transport: Transport::Stdin,
        }
    }
}

/// Resolve a built-in adapter by backend name.
///
/// Built-ins: `pi`, `hermes`, `acpx`. Returns `Err` for any unknown name (fail-closed:
/// never silently fall back to pi, which would produce an unparseable stream and a null
/// verdict indistinguishable from a real crash).
pub fn adapter_for(backend: &str, profile: Option<&str>) -> Result<Adapter, AcpError> {
    match backend {
        // Built-in adapters (design D6 / fix-spawn-argv-overflow §8): the prompt is
        // delivered on the child's stdin pipe (transport = Stdin). argv contains NO
        // `{prompt}` token — the spawn template is just `<bin> --mode json` and the
        // resume template is `<bin> --session {sid} --mode json`. This eliminates
        // E2BIG for any prompt size (the root cause of gh #1).
        //
        // NOTE: only `pi` was scout-verified to actually read stdin
        // (flow/findings/2026-07-05-pi-stdin-prompt.md). `hermes` and `acpx` are
        // kept on the same stdin template for spec parity (D6 table); a follow-up
        // probe must confirm each reads stdin before relying on them in production.
        "pi" => Ok(Adapter {
            spawn: "pi --mode json".to_string(),
            resume: "pi --session {sid} --mode json".to_string(),
            ..Default::default()
        }),
        "hermes" => {
            // When a profile is specified, inject `-p <profile>` into both templates.
            // Spawn: `hermes -p <profile> --mode json`
            // Resume: `hermes -p <profile> --session {sid} --mode json`
            let (spawn, resume) = match profile {
                Some(p) => (
                    format!("hermes -p {p} --mode json"),
                    format!("hermes -p {p} --session {{sid}} --mode json"),
                ),
                None => (
                    "hermes --mode json".to_string(),
                    "hermes --session {sid} --mode json".to_string(),
                ),
            };
            Ok(Adapter {
                spawn,
                resume,
                ..Default::default()
            })
        }
        "acpx" => Ok(Adapter {
            spawn: "acpx --mode json".to_string(),
            resume: "acpx --session {sid} --mode json".to_string(),
            ..Default::default()
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

    /// §8 (D6) — built-in pi templates use stdin transport: NO `{prompt}` token.
    #[test]
    fn pi_templates_match_spec() {
        let a = adapter_for("pi", None).unwrap();
        assert_eq!(a.spawn, "pi --mode json");
        assert_eq!(a.resume, "pi --session {sid} --mode json");
        // Hard fail-closed: the spawn argv must carry ZERO prompt-derived bytes.
        assert!(!a.spawn.contains("{prompt}"), "spawn must not inline prompt (E2BIG risk)");
        assert!(!a.resume.contains("{prompt}"), "resume must not inline prompt (E2BIG risk)");
    }

    /// §8 (D6) — hermes templates mirror pi for spec parity (stdin transport).
    /// Scout note: hermes stdin behaviour is NOT yet verified on this host; this
    /// asserts the template contract only, not that hermes reads stdin.
    #[test]
    fn hermes_templates_match_spec() {
        let a = adapter_for("hermes", None).unwrap();
        assert_eq!(a.spawn, "hermes --mode json");
        assert_eq!(a.resume, "hermes --session {sid} --mode json");
        assert!(!a.spawn.contains("{prompt}") && !a.resume.contains("{prompt}"));
    }

    /// §8 (D6) — acpx templates mirror pi for spec parity (stdin transport).
    #[test]
    fn acpx_templates_match_spec() {
        let a = adapter_for("acpx", None).unwrap();
        assert_eq!(a.spawn, "acpx --mode json");
        assert_eq!(a.resume, "acpx --session {sid} --mode json");
        assert!(!a.spawn.contains("{prompt}") && !a.resume.contains("{prompt}"));
    }

    #[test]
    fn unknown_backend_errors() {
        assert!(adapter_for("claude", None).is_err());
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

    // ── RED tests for fix-spawn-argv-overflow §1 (prompt-transport spec). ───────
    // These tests encode the NOT-YET-IMPLEMENTED behaviour from design.md D1/D2.
    // They are expected to FAIL until the GREEN author (§2) implements:
    //   - real `Transport` serde (rename_all = "lowercase", kebab-case for GoalFile)
    //   - built-in defaults of Transport::Stdin
    //   - required `transport` field for custom adapters
    //   - `{prompt}` placeholder rejection at load
    // ────────────────────────────────────────────────────────────────────────────

    /// §1.4 — built-in `pi` adapter defaults to `transport == Transport::Stdin`.
    #[test]
    fn transport_field_defaults_to_stdin_for_builtin_pi() {
        let a = adapter_for("pi", None).expect("pi is a built-in adapter");
        assert_eq!(
            a.transport,
            Transport::Stdin,
            "built-in pi adapter must default to the stdin transport (design D1/D6)"
        );
    }

    /// §1.4 — built-in `hermes` and `acpx` adapters also default to Stdin.
    #[test]
    fn transport_field_defaults_to_stdin_for_builtin_hermes_and_acpx() {
        let hermes = adapter_for("hermes", None).expect("hermes is a built-in adapter");
        assert_eq!(
            hermes.transport,
            Transport::Stdin,
            "built-in hermes adapter must default to the stdin transport"
        );
        let acpx = adapter_for("acpx", None).expect("acpx is a built-in adapter");
        assert_eq!(
            acpx.transport,
            Transport::Stdin,
            "built-in acpx adapter must default to the stdin transport"
        );
    }

    /// §1.5 — a custom adapter deserialized from config MUST declare a `transport`
    /// field; omitting it is a load-time error (fail-closed).
    #[test]
    fn custom_adapter_requires_transport_field() {
        let json = r#"{"spawn":"pi --mode json","resume":"pi --mode json"}"#;
        let result = serde_json::from_str::<Adapter>(json);
        assert!(
            result.is_err(),
            "custom adapter without a `transport` field must be rejected at config load"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.to_lowercase().contains("transport"),
            "rejection error must name the missing `transport` field; got: {err}"
        );
    }

    /// §1.3 / spec: Inline `{prompt}` placeholder in `spawn` template MUST be rejected
    /// at config load with a migration message naming `{goalFile}` + `transport: goal-file`
    /// (or `transport: stdin` with no placeholder).
    #[test]
    fn custom_adapter_with_inline_prompt_placeholder_is_rejected() {
        // `{prompt}` anywhere in the spawn template must be rejected at load.
        // No need for surrounding quotes — the placeholder substring is the trigger.
        let json = r#"{"spawn":"pi -p {prompt} --mode json","resume":"pi --mode json","transport":"stdin"}"#;
        let result = serde_json::from_str::<Adapter>(json);
        assert!(
            result.is_err(),
            "custom adapter whose `spawn` template contains {{prompt}} must be rejected"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("{goalFile}"),
            "error must point users at the {{goalFile}} migration placeholder; got: {err}"
        );
        assert!(
            err.contains("goal-file"),
            "error must mention `transport: goal-file` migration; got: {err}"
        );
    }

    /// §1.3 — same rejection for the `resume` template.
    #[test]
    fn custom_adapter_with_resume_prompt_placeholder_is_rejected() {
        let json = r#"{"spawn":"pi --mode json","resume":"pi --session {sid} -p {prompt} --mode json","transport":"stdin"}"#;
        let result = serde_json::from_str::<Adapter>(json);
        assert!(
            result.is_err(),
            "custom adapter whose `resume` template contains {{prompt}} must be rejected"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("{goalFile}"),
            "error must point users at the {{goalFile}} migration placeholder; got: {err}"
        );
        assert!(
            err.contains("goal-file"),
            "error must mention `transport: goal-file` migration; got: {err}"
        );
    }

    /// spec scenario: `transport: "goal-file"` together with a `{goalFile}` spawn
    /// template loads successfully.
    #[test]
    fn goal_file_transport_accepted() {
        let json = r#"{"spawn":"pi --goal-file {goalFile} --mode json","resume":"pi --mode json","transport":"goal-file"}"#;
        let result = serde_json::from_str::<Adapter>(json);
        let a = result.expect("goal-file transport with {goalFile} template must load");
        assert_eq!(
            a.transport,
            Transport::GoalFile,
            "transport must round-trip to Transport::GoalFile"
        );
    }

    // ── RED tests for hermes-profile-adapter: adapter_for with profile ────────
    // These tests encode the NOT-YET-IMPLEMENTED behaviour: adapter_for must
    // accept an optional profile parameter. They are expected to FAIL (RED)
    // until the GREEN author implements the profile-aware signature.
    // ────────────────────────────────────────────────────────────────────────────

    /// hermes-profile: adapter_for("hermes", Some("verifier")) must produce
    /// spawn template "hermes -p verifier --mode json".
    #[test]
    fn hermes_with_profile_renders_spawn_with_profile_flag() {
        let a = adapter_for("hermes", Some("verifier")).unwrap();
        assert_eq!(
            a.spawn, "hermes -p verifier --mode json",
            "hermes spawn with profile must include `-p <profile>` flag"
        );
    }

    /// hermes-profile: adapter_for("hermes", Some("verifier")) must produce
    /// resume template "hermes -p verifier --session {sid} --mode json".
    #[test]
    fn hermes_with_profile_renders_resume_with_profile_flag() {
        let a = adapter_for("hermes", Some("verifier")).unwrap();
        assert_eq!(
            a.resume, "hermes -p verifier --session {sid} --mode json",
            "hermes resume with profile must include `-p <profile>` before --session"
        );
    }

    /// hermes-profile: adapter_for("hermes", None) must produce the same
    /// templates as before (no profile flag).
    #[test]
    fn hermes_without_profile_unchanged() {
        let a = adapter_for("hermes", None).unwrap();
        assert_eq!(a.spawn, "hermes --mode json");
        assert_eq!(a.resume, "hermes --session {sid} --mode json");
    }

    /// hermes-profile: pi adapter must ignore the profile parameter entirely.
    #[test]
    fn pi_ignores_profile_param() {
        let with_profile = adapter_for("pi", Some("verifier")).unwrap();
        let without_profile = adapter_for("pi", None).unwrap();
        assert_eq!(with_profile.spawn, without_profile.spawn,
            "pi spawn must be identical regardless of profile param");
        assert_eq!(with_profile.resume, without_profile.resume,
            "pi resume must be identical regardless of profile param");
        assert_eq!(with_profile.spawn, "pi --mode json");
        assert_eq!(with_profile.resume, "pi --session {sid} --mode json");
    }

    /// hermes-profile: acpx adapter must ignore the profile parameter entirely.
    #[test]
    fn acpx_ignores_profile_param() {
        let with_profile = adapter_for("acpx", Some("verifier")).unwrap();
        let without_profile = adapter_for("acpx", None).unwrap();
        assert_eq!(with_profile.spawn, without_profile.spawn,
            "acpx spawn must be identical regardless of profile param");
        assert_eq!(with_profile.resume, without_profile.resume,
            "acpx resume must be identical regardless of profile param");
        assert_eq!(with_profile.spawn, "acpx --mode json");
        assert_eq!(with_profile.resume, "acpx --session {sid} --mode json");
    }

    /// hermes-profile edge case: empty profile string is treated as a profile name
    /// (produces `hermes -p  --mode json`). This is technically valid but unusual.
    #[test]
    fn hermes_with_empty_profile_string() {
        let a = adapter_for("hermes", Some("")).unwrap();
        assert_eq!(a.spawn, "hermes -p  --mode json");
        assert_eq!(a.resume, "hermes -p  --session {sid} --mode json");
    }

    /// hermes-profile edge case: profile with spaces is rendered as-is (single token).
    #[test]
    fn hermes_with_profile_containing_spaces() {
        let a = adapter_for("hermes", Some("my profile")).unwrap();
        assert_eq!(a.spawn, "hermes -p my profile --mode json");
    }

    /// hermes-profile edge case: profile with special chars is rendered as-is.
    #[test]
    fn hermes_with_profile_containing_special_chars() {
        let a = adapter_for("hermes", Some("verifier-2")).unwrap();
        assert_eq!(a.spawn, "hermes -p verifier-2 --mode json");
        assert_eq!(a.resume, "hermes -p verifier-2 --session {sid} --mode json");
    }

    /// §1.1 — `Transport` serde round-trip: `"stdin"` <-> `Transport::Stdin`,
    /// `"goal-file"` <-> `Transport::GoalFile`. Unknown value -> Err.
    #[test]
    fn transport_field_serializes_correctly() {
        // stdin <-> Stdin
        let s = serde_json::to_string(&Transport::Stdin).expect("serialize Stdin");
        assert_eq!(s, "\"stdin\"", "Stdin must serialize as the JSON string \"stdin\"");
        let t: Transport =
            serde_json::from_str("\"stdin\"").expect("deserialize \"stdin\"");
        assert_eq!(t, Transport::Stdin, "\"stdin\" must deserialize to Transport::Stdin");

        // goal-file <-> GoalFile
        let s = serde_json::to_string(&Transport::GoalFile).expect("serialize GoalFile");
        assert_eq!(s, "\"goal-file\"", "GoalFile must serialize as the JSON string \"goal-file\"");
        let t: Transport =
            serde_json::from_str("\"goal-file\"").expect("deserialize \"goal-file\"");
        assert_eq!(t, Transport::GoalFile, "\"goal-file\" must deserialize to Transport::GoalFile");

        // unknown -> Err
        let unknown = serde_json::from_str::<Transport>("\"weird\"");
        assert!(
            unknown.is_err(),
            "unknown transport value must be rejected, not silently coerced"
        );
    }
}
