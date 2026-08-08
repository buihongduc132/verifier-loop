//! Shared ACP JSON stream parser (tasks.md §4.1, verifier-spawn spec).
//!
//! The parser models the ACP event stream as an exhaustive [`AcpEvent`] enum and an
//! exhaustive `match` over every event type (D0 rationale: an unhandled event is a
//! *compile error*, guarding the silent no-SID / no-agent_end failure mode that would
//! otherwise break the fail-closed contract). Non-essential but well-formed event types
//! (`message_update`, `turn_end`, `text_*`) parse to `Ok(None)` so the essential
//! `match` stays focused on the spec's session/agent_start/turn_start/message_start/
//! message_end/agent_end set.
//!
//! Fail-closed: a non-JSON line is a hard error (never silently ignored), because a
//! malformed stream could hide a missing `agent_end` (the fail-closed guard from D3).

use serde_json::Value;

/// Errors raised by the ACP parser. All paths fail closed.
#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    /// A line that is not valid JSON.
    #[error("malformed json line: {0}")]
    MalformedJson(String),
    /// A JSON line whose shape does not match the ACP contract (missing `type`, or a
    /// recognised `type` missing its required fields).
    #[error("bad event shape: {0}")]
    BadEventShape(String),
}

/// A normalised ACP message: just the role + flattened text content.
///
/// The raw ACP `message.content` is an array of `{type:"text", text:"..."}` parts. We
/// flatten every `text` part (in order) into a single `String` so downstream consumers
/// (consensus, prompt rendering) never have to re-parse the array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    pub text: String,
}

/// Exhaustive enumeration of the ACP event types the verifier-loop core cares about.
///
/// This enum is intentionally **not** `#[non_exhaustive]`: adding a variant is a breaking
/// change on purpose. Every consumer must `match` exhaustively so that a newly added
/// essential event surfaces as a compile error everywhere (D0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpEvent {
    /// `{"type":"session","id":"..."}` — carries the resumable session id (SID).
    Session { id: String },
    /// `{"type":"agent_start"}` — the agent process has begun.
    AgentStart,
    /// `{"type":"turn_start"}` — a turn (one user→assistant exchange) has begun.
    TurnStart,
    /// `{"type":"message_start","message":{...}}` — a message is starting to stream.
    MessageStart { message: Message },
    /// `{"type":"message_end","message":{...}}` — a message finished streaming.
    MessageEnd { message: Message },
    /// `{"type":"agent_end","messages":[...],"willRetry":bool}` — agent done, carries the
    /// full message transcript and whether the host will retry (e.g. on compaction).
    AgentEnd {
        messages: Vec<Message>,
        will_retry: bool,
    },
    /// `{"type":"compaction","tokensBefore":N,"tokensAfter":M?}` — the backend compacted
    /// the session context mid-run. Compaction is the confirmed kill mechanism for
    /// verifier verdicts: the session terminates with no `agent_end` and the verdict is
    /// lost. The orchestrator treats this as a first-class recoverable event (D6).
    Compaction {
        tokens_before: Option<u64>,
        tokens_after: Option<u64>,
    },
}

/// Parse a single ACP JSON line into an [`AcpEvent`].
///
/// Returns:
/// * `Ok(Some(event))` for the essential event types in [`AcpEvent`].
/// * `Ok(None)` for well-formed but ignorable event types (`message_update`, `turn_end`,
///   `text_start`, `text_delta`, `text_end`, and any future non-essential type).
/// * `Err(AcpError::MalformedJson)` for non-JSON lines (fail-closed: never silent).
/// * `Err(AcpError::BadEventShape)` for a recognised essential `type` missing required
///   fields (e.g. a `session` line without an `id`).
pub fn parse_event(line: &str) -> Result<Option<AcpEvent>, AcpError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        // Blank lines are ignorable: the conformance loop skips them itself, but stray
        // trailing newlines must not turn into hard errors.
        return Ok(None);
    }

    let value: Value = serde_json::from_str(trimmed)
        .map_err(|e| AcpError::MalformedJson(format!("{e}: {trimmed}")))?;

    let ty = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| AcpError::BadEventShape(format!("missing 'type' field: {trimmed}")))?;

    let event = match ty {
        "session" => {
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| AcpError::BadEventShape("session event missing 'id'".into()))?
                .to_string();
            AcpEvent::Session { id }
        }
        "agent_start" => AcpEvent::AgentStart,
        "turn_start" => AcpEvent::TurnStart,
        "message_start" => AcpEvent::MessageStart {
            message: parse_message(&value)?,
        },
        "message_end" => AcpEvent::MessageEnd {
            message: parse_message(&value)?,
        },
        "agent_end" => {
            let messages = value
                .get("messages")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    AcpError::BadEventShape("agent_end missing 'messages' array".into())
                })?
                .iter()
                .map(parse_message_value)
                .collect::<Result<Vec<_>, _>>()?;
            let will_retry = value
                .get("willRetry")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            AcpEvent::AgentEnd {
                messages,
                will_retry,
            }
        }
        "compaction" => {
            let tokens_before = value.get("tokensBefore").and_then(Value::as_u64);
            let tokens_after = value.get("tokensAfter").and_then(Value::as_u64);
            AcpEvent::Compaction {
                tokens_before,
                tokens_after,
            }
        }
        // Well-formed but non-essential events: ignorable. Returning None keeps the
        // exhaustive `match` over AcpEvent focused on the essential set.
        "message_update" | "turn_end" | "text_start" | "text_delta" | "text_end" => {
            return Ok(None);
        }
        // Unknown future event types: treat as ignorable so the parser stays forward-
        // compatible with additive ACP extensions, but only because they are not in the
        // essential set. An essential type added to the enum must be matched above.
        _ => return Ok(None),
    };

    Ok(Some(event))
}

/// Extract the session id from the **first** `session` event in the stream.
///
/// Fail-closed: returns `None` if there is no `session` event (the orchestrator must not
/// fabricate a SID to resume against — it would point at a non-existent session).
pub fn extract_sid(stream: &str) -> Option<String> {
    for line in stream.lines() {
        if let Ok(Some(AcpEvent::Session { id })) = parse_event(line) {
            return Some(id);
        }
    }
    None
}

/// Extract the **last** assistant message text from the `agent_end` event.
///
/// Fail-closed: returns `None` if there is no `agent_end` event (the host may have been
/// killed mid-run; the orchestrator leaves a null verdict rather than a fabricated output).
pub fn extract_final_output(stream: &str) -> Option<String> {
    let mut last: Option<String> = None;
    for line in stream.lines() {
        if let Ok(Some(AcpEvent::AgentEnd { messages, .. })) = parse_event(line) {
            // The last assistant message in the transcript is the verifier's final output.
            if let Some(msg) = messages.iter().rev().find(|m| m.role == "assistant") {
                last = Some(msg.text.clone());
            }
        }
    }
    last
}

/// Returns true iff the stream contains at least one `compaction` event.
/// Used by the orchestrator to decide whether a no-agent_end exit is a compaction
/// kill (recoverable) vs a plain crash (fail-closed).
pub fn extract_compaction_observed(stream: &str) -> bool {
    for line in stream.lines() {
        if matches!(parse_event(line), Ok(Some(AcpEvent::Compaction { .. }))) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// internal helpers
// ---------------------------------------------------------------------------

/// Parse a `{"message":{...}}` sibling object (the message lives at the top level of
/// `message_start` / `message_end` events).
fn parse_message(event: &Value) -> Result<Message, AcpError> {
    let message = event
        .get("message")
        .ok_or_else(|| AcpError::BadEventShape("message event missing 'message' field".into()))?;
    parse_message_value(message)
}

/// Parse a raw `message` JSON value (`{role, content:[{type,text}]}`) into a [`Message`],
/// flattening every `text` content part into a single string.
fn parse_message_value(message: &Value) -> Result<Message, AcpError> {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| AcpError::BadEventShape("message missing 'role'".into()))?
        .to_string();

    let text = message
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    if part.get("type").and_then(Value::as_str) == Some("text") {
                        part.get("text").and_then(Value::as_str).map(str::to_string)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    Ok(Message { role, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_line_is_ignorable() {
        assert!(parse_event("").unwrap().is_none());
        assert!(parse_event("   \n").unwrap().is_none());
    }

    #[test]
    fn session_missing_id_is_bad_shape() {
        let res = parse_event(r#"{"type":"session"}"#);
        assert!(matches!(res, Err(AcpError::BadEventShape(_))));
    }

    #[test]
    fn unknown_future_type_is_ignorable() {
        let res = parse_event(r#"{"type":"some_new_acp_thing","payload":42}"#);
        assert!(res.unwrap().is_none());
    }

    #[test]
    fn message_flattens_multiple_text_parts() {
        let line = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"hello "},{"type":"tool_use","id":"x"},{"type":"text","text":"world"}]}}"#;
        match parse_event(line).unwrap().unwrap() {
            AcpEvent::MessageEnd { message } => {
                assert_eq!(message.role, "assistant");
                assert_eq!(message.text, "hello world");
            }
            other => panic!("expected MessageEnd, got {other:?}"),
        }
    }

    #[test]
    fn agent_end_defaults_will_retry_false_when_absent() {
        let line = r#"{"type":"agent_end","messages":[]}"#;
        match parse_event(line).unwrap().unwrap() {
            AcpEvent::AgentEnd { will_retry, .. } => assert!(!will_retry),
            other => panic!("expected AgentEnd, got {other:?}"),
        }
    }
}
