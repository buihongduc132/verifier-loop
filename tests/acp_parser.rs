// tasks.md §4 — ACP stream parser + adapters (verifier-spawn spec).
// RED phase: written first, against the spec, BEFORE any implementation.
//
// Scope of THIS test (§4): the shared ACP JSON-stream parser (`AcpEvent` enum,
// `parse_event`, SID extraction, final-output capture) and the backend adapter
// spawn/resume template rendering (pi / hermes / acpx / custom). These are pure
// functions that are testable without spawning any process.
//
// OUT of scope here (deliberately): parallel spawn orchestration, per-verifier
// timeout, gather barrier, env-var injection — those are §5 (spawn orchestration)
// and are tested by a SEPARATE group's RED.
//
// The real `pi --mode json` fixture at `flow/fixtures/acp_pi_sample.jsonl` is the
// conformance oracle. Expected facts baked in below:
//   * session id   = 019f27b5-7c4d-7c20-b228-b675c225d71f
//   * willRetry    = false
//   * last assistant message text = "PONG"
//
// (Deviation note: the objective's "separate fresh teammate per phase" safeguard is
//  not applied via a teams tool because none is available in this environment.
//  This RED is committed and verified-failing before any GREEN is written by a
//  DIFFERENT comrade, preserving test-first + separate-author discipline.)

use verifier_loop::acp;

/// Loads the real `pi --mode json` fixture shipped with the repo.
fn fixture_stream() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("flow/fixtures/acp_pi_sample.jsonl");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing ACP fixture at {path:?}: {e}"))
}

// ---------------------------------------------------------------------------
// §4.1 — AcpEvent enum + parse_event (per-line)
// ---------------------------------------------------------------------------

#[test]
fn parse_session_line_yields_session_event_with_id() {
    let line = r#"{"type":"session","version":3,"id":"abc-123","timestamp":"x","cwd":"/tmp"}"#;
    let ev = acp::parse_event(line)
        .expect("session line parses")
        .expect("session line is an essential event, not None");
    match ev {
        acp::AcpEvent::Session { id } => assert_eq!(id, "abc-123"),
        other => panic!("expected Session, got {other:?}"),
    }
}

#[test]
fn parse_agent_start_yields_agent_start() {
    let line = r#"{"type":"agent_start"}"#;
    assert!(matches!(
        acp::parse_event(line).unwrap().unwrap(),
        acp::AcpEvent::AgentStart
    ));
}

#[test]
fn parse_turn_start_yields_turn_start() {
    let line = r#"{"type":"turn_start"}"#;
    assert!(matches!(
        acp::parse_event(line).unwrap().unwrap(),
        acp::AcpEvent::TurnStart
    ));
}

#[test]
fn parse_message_end_yields_message_end_with_role() {
    let line = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#;
    match acp::parse_event(line).unwrap().unwrap() {
        acp::AcpEvent::MessageEnd { message } => {
            assert_eq!(message.role, "assistant");
            assert_eq!(message.text, "hi");
        }
        other => panic!("expected MessageEnd, got {other:?}"),
    }
}

#[test]
fn parse_ignorable_event_types_yield_none() {
    // message_update / turn_end / text_* are NOT in the §4.1 essential set. The parser
    // MUST return None for them so the exhaustive `match` over AcpEvent stays focused on
    // the spec's essential events (session/agent_start/turn_start/message_start/message_end/
    // agent_end). The fixture is full of these; they must not error and must not masquerade
    // as essential events.
    for ty in ["message_update", "turn_end", "text_delta", "text_start", "text_end"] {
        let line = format!(r#"{{"type":"{ty}"}}"#);
        let parsed = acp::parse_event(&line)
            .unwrap_or_else(|e| panic!("ignorable '{ty}' must not error: {e:?}"));
        assert!(parsed.is_none(), "ignorable '{ty}' must parse to None");
    }
}

#[test]
fn parse_malformed_json_is_a_hard_error() {
    // Fail-closed: a non-JSON line is an error, never silently ignored, because a
    // malformed stream could hide a missing agent_end (the fail-closed guard from D3).
    let res = acp::parse_event("this is not json at all");
    assert!(res.is_err(), "malformed line must be an error");
}

#[test]
fn parse_agent_end_yields_messages_and_will_retry_from_fixture() {
    let line = fixture_stream()
        .lines()
        .find(|l| l.contains(r#""type":"agent_end""#))
        .expect("fixture has an agent_end line");
    match acp::parse_event(line).unwrap().unwrap() {
        acp::AcpEvent::AgentEnd {
            messages,
            will_retry,
        } => {
            assert!(!will_retry, "fixture agent_end has willRetry=false");
            assert_eq!(messages.len(), 2, "fixture agent_end carries 2 messages");
            let asst = messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
                .expect("an assistant message is present");
            assert_eq!(asst.text, "PONG", "final assistant text is PONG");
        }
        other => panic!("expected AgentEnd, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// §4.1 — whole-stream helpers (SID + final output)
// ---------------------------------------------------------------------------

#[test]
fn extract_sid_returns_first_session_id_from_fixture() {
    let stream = fixture_stream();
    assert_eq!(
        acp::extract_sid(&stream).expect("fixture has a session id"),
        "019f27b5-7c4d-7c20-b228-b675c225d71f"
    );
}

#[test]
fn extract_sid_returns_none_when_no_session_event() {
    let stream = "{\"type\":\"agent_start\"}\n{\"type\":\"agent_end\",\"messages\":[],\"willRetry\":false}\n";
    assert!(acp::extract_sid(stream).is_none(), "no session -> no SID");
}

#[test]
fn extract_final_output_returns_last_assistant_text_from_fixture() {
    let stream = fixture_stream();
    assert_eq!(
        acp::extract_final_output(&stream).expect("fixture has a final assistant message"),
        "PONG"
    );
}

#[test]
fn extract_final_output_returns_none_without_agent_end() {
    let stream = "{\"type\":\"session\",\"id\":\"x\"}\n{\"type\":\"agent_start\"}\n";
    assert!(
        acp::extract_final_output(stream).is_none(),
        "no agent_end -> no final output (fail-closed)"
    );
}

#[test]
fn parser_is_conformant_over_full_fixture_stream() {
    // Feeding the real fixture line-by-line MUST produce no errors, exactly one
    // Session, and exactly one AgentEnd. This is the per-backend conformance gate
    // the spec demands ("parser conformance per backend").
    let stream = fixture_stream();
    let mut sessions = 0u32;
    let mut agent_ends = 0u32;
    let mut errors = 0u32;
    for line in stream.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match acp::parse_event(line) {
            Ok(Some(acp::AcpEvent::Session { .. })) => sessions += 1,
            Ok(Some(acp::AcpEvent::AgentEnd { .. })) => agent_ends += 1,
            Ok(_) => {}
            Err(_) => errors += 1,
        }
    }
    assert_eq!(errors, 0, "fixture must parse with zero errors");
    assert_eq!(sessions, 1, "exactly one session event");
    assert_eq!(agent_ends, 1, "exactly one agent_end event");
}

// ---------------------------------------------------------------------------
// §4.2 / §4.3 / §4.4 — backend adapter templates + rendering
// ---------------------------------------------------------------------------

#[test]
fn pi_adapter_spawn_uses_pi_flag_p_and_mode_json() {
    // verifier-spawn spec: "spawn uses `pi -p "<prompt>" --mode json`".
    let t = acp::adapter_for("pi").expect("pi is a built-in adapter");
    assert!(
        t.spawn.contains("pi") && t.spawn.contains("-p") && t.spawn.contains("--mode json"),
        "pi spawn template must match spec, got: {}",
        t.spawn
    );
}

#[test]
fn pi_adapter_resume_uses_session_flag_and_mode_json() {
    // verifier-spawn spec: "resume uses `pi --session <sid> -p "<prompt>" --mode json`".
    let t = acp::adapter_for("pi").expect("pi is a built-in adapter");
    assert!(
        t.resume.contains("--session") && t.resume.contains("-p") && t.resume.contains("--mode json"),
        "pi resume template must match spec, got: {}",
        t.resume
    );
}

#[test]
fn hermes_and_acpx_are_builtin_adapters() {
    // §4.3: hermes and acpx each provide spawn/resume templates.
    for backend in ["hermes", "acpx"] {
        let t = acp::adapter_for(backend).unwrap_or_else(|e| panic!("{backend} must be built-in: {e:?}"));
        assert!(!t.spawn.is_empty() && !t.resume.is_empty(), "{backend} templates non-empty");
    }
}

#[test]
fn unknown_builtin_backend_errors_fail_closed() {
    // A typo'd/unsupported backend must error, never silently fall back to pi
    // (fail-closed: a wrong backend would produce an unparseable stream).
    assert!(acp::adapter_for("definitely-not-a-backend").is_err());
}

#[test]
fn render_spawn_substitutes_prompt_into_template() {
    let template = "run --prompt {prompt} --json";
    assert_eq!(acp::render_spawn(template, "hello world"), "run --prompt hello world --json");
}

#[test]
fn render_resume_substitutes_sid_and_prompt_into_template() {
    let template = "resume --sid {sid} --prompt {prompt}";
    assert_eq!(
        acp::render_resume(template, "sid-42", "do the thing"),
        "resume --sid sid-42 --prompt do the thing"
    );
}

#[test]
fn custom_adapter_templates_round_trip_through_render() {
    // §4.4: custom adapters are configurable via config.json with spawn/resume templates
    // and a JSON flag, conforming to the ACP output format (parsed by the shared parser).
    let spawn_tpl = "my-agent --json --prompt {prompt}".to_string();
    let resume_tpl = "my-agent --json --sid {sid} --prompt {prompt}".to_string();

    let rendered_spawn = acp::render_spawn(&spawn_tpl, "verify the thing");
    let rendered_resume = acp::render_resume(&resume_tpl, "sess-1", "verify again");

    assert!(rendered_spawn.contains("--json"));
    assert!(rendered_spawn.contains("verify the thing"));
    assert!(rendered_resume.contains("sess-1"));
    assert!(rendered_resume.contains("verify again"));
}
