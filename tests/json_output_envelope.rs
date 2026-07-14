//! RED tests for `add-json-output-mode` (tasks.md group 1 + group 2).
//!
//! These tests pin the JSON output envelope schema, the rejection-array sort order, the
//! `Output` formatter's stdout/stderr separation, and the top-level `--json` flag on
//! `VerifierLoopCli`. They are expected to FAIL (compile error — the
//! `verifier_loop::cli::json_output` module does not exist yet, and `VerifierLoopCli` has
//! no `json` field) until the GREEN implementation lands.
//!
//! TDD discipline (`AGENTS.md`): the RED author and the GREEN author MUST be different
//! teammates. This file was authored by 'red-envelope'; the GREEN author is a different
//! comrade who MUST NOT have authored this file.
//!
//! # Public API these tests assume (GREEN author builds this)
//!
//! `verifier_loop::cli::json_output` exposes:
//!   * `JsonEnvelope` — `#[derive(Debug, Serialize)]` with camelCase serde renames and
//!     `#[serde(skip_serializing_if = "Option::is_none")]` on every `Option` field. Fields
//!     (snake_case in Rust): `ok: bool`, `command: String`, `goal_id`, `round`, `verifier_id`,
//!     `status`, `hash`, `full_digest`, `needs`, `rejection`, `verdicts`, `state`, `error`
//!     (all `Option<…>`). Public so tests can use struct literals.
//!   * `RejectionBreakdown { reject_notes: Vec<(String,String)>, null_verifiers:
//!     Vec<String>, signature_failures: Vec<(String,String)> }` with a
//!     `from_unsorted(reject_notes, null_verifiers, signature_failures)` constructor that
//!     sorts all three by verifierId ascending.
//!   * `Output { Human, Json }` formatter (see the formatter-API note below).
//!
//! ## Formatter API — testability refinement (read me, GREEN author)
//!
//! The task's stated API writes directly to stdout/stderr:
//!   `print_success(&self, env: &JsonEnvelope, human_line: &str)`
//!   `print_error(&self, env: &JsonEnvelope, human_err: &str)`
//!
//! This project has NO stdout-capture dev-dependency (and the existing test style captures
//! stdout only via subprocess `std::process::Command`, which cannot reach an in-process
//! formatter). To keep the formatter unit-testable WITHOUT adding a crate dependency, these
//! RED tests assume a writer-parameterised form:
//!   `print_success<W: io::Write>(&self, env: &JsonEnvelope, human_line: &str, out: &mut W)`
//!   `print_error<W: io::Write>(&self, env: &JsonEnvelope, human_err: &str, out: &mut W, err: &mut W)`
//!
//! This is an additive, behaviour-preserving refinement: the GREEN author wires
//! `&mut io::stdout()` / `&mut io::stderr()` at every bin call site, and the stdout/stderr
//! separation contract is EXACTLY what tests 3 and 4 assert. If the GREEN author prefers the
//! parameterless form, they MUST instead add a capture dev-dependency and rewrite tests 3–4;
//! the writer form is the lower-friction path and is recommended.

use std::io::Write;

use clap::Parser;
use serde_json::Value;

use verifier_loop::cli::json_output::{JsonEnvelope, Output, RejectionBreakdown};
use verifier_loop::cli::VerifierLoopCli;

// ---------------------------------------------------------------------------
// Scenario 1 (tasks.md §1.2) — envelope serializes camelCase + skips None.
// ---------------------------------------------------------------------------

#[test]
fn envelope_serializes_camelcase_and_skips_none() {
    // Minimal envelope: only `ok` + `command` are set; every Option is None.
    let minimal = JsonEnvelope {
        ok: true,
        command: "new".to_string(),
        goal_id: None,
        round: None,
        verifier_id: None,
        status: None,
        hash: None,
        full_digest: None,
        needs: None,
        rejection: None,
        verdicts: None,
        state: None,
        error: None,
    };

    let v: Value = serde_json::to_value(&minimal).expect("minimal envelope serializes");
    let obj = v
        .as_object()
        .expect("envelope root must be a JSON object");
    // `skip_serializing_if = "Option::is_none"` on every Option => exactly 2 keys.
    assert_eq!(
        obj.len(),
        2,
        "minimal envelope must carry ONLY ok + command (got {obj:?})"
    );
    assert_eq!(obj["ok"], true);
    assert_eq!(obj["command"], "new");

    // No optional field may leak when its value is None.
    for absent in [
        "hash", "error", "goalId", "round", "status", "fullDigest", "rejection",
        "verifierId", "needs", "state", "verdicts",
    ] {
        assert!(
            !obj.contains_key(absent),
            "`{absent}` must be ABSENT when its Option is None (got {obj:?})"
        );
    }

    // Now set hash + round and re-assert.
    let with_extras = JsonEnvelope {
        ok: true,
        command: "new".to_string(),
        goal_id: None,
        round: Some(1),
        verifier_id: None,
        status: None,
        hash: Some("260715-deadbeef".to_string()),
        full_digest: None,
        needs: None,
        rejection: None,
        verdicts: None,
        state: None,
        error: None,
    };
    let v2: Value = serde_json::to_value(&with_extras).expect("envelope serializes");
    let obj2 = v2.as_object().unwrap();
    assert_eq!(
        obj2.len(),
        4,
        "now exactly four keys (ok, command, hash, round) — got {obj2:?}"
    );
    assert_eq!(obj2["hash"], "260715-deadbeef");
    assert_eq!(obj2["round"].as_u64(), Some(1));

    // camelCase guarantee: the serialized text NEVER carries a snake_case key.
    let text = serde_json::to_string(&with_extras).unwrap();
    for snake in ["goal_id", "full_digest", "verifier_id", "signature_failures"] {
        assert!(
            !text.contains(snake),
            "snake_case key `{snake}` must NEVER appear in serialized JSON: {text}"
        );
    }
    // And the camelCase form IS present where relevant.
    assert!(text.contains("\"hash\""));
    assert!(text.contains("\"round\""));
}

// ---------------------------------------------------------------------------
// Scenario 2 (tasks.md §1.3) — rejection arrays sorted by verifierId.
// ---------------------------------------------------------------------------

#[test]
fn envelope_rejection_arrays_sorted_by_verifier_id() {
    // Deliberately insert in NON-sorted order (v3 before v1) for both reject_notes and
    // null_verifiers. `from_unsorted` must sort ascending by verifierId.
    let rb = RejectionBreakdown::from_unsorted(
        vec![
            ("v3".to_string(), "v3 says wrong".to_string()),
            ("v1".to_string(), "v1 says broken".to_string()),
        ],
        vec!["v3".to_string(), "v1".to_string()],
        vec![
            ("v2".to_string(), "v2 sig fail".to_string()),
        ],
    );

    let env = JsonEnvelope {
        ok: false,
        command: "new".to_string(),
        goal_id: None,
        round: Some(1),
        verifier_id: None,
        status: Some("rejected".to_string()),
        hash: None,
        full_digest: None,
        needs: None,
        rejection: Some(rb),
        verdicts: None,
        state: None,
        error: None,
    };

    let v: Value = serde_json::to_value(&env).unwrap();
    // camelCase outer field name.
    let rejection = v
        .get("rejection")
        .expect("`rejection` present on a rejected envelope");
    assert_eq!(rejection.as_object().unwrap().len(), 3);

    // rejectNotes is an array of [verifierId, note] tuples, sorted by verifierId ascending.
    let notes = rejection["rejectNotes"]
        .as_array()
        .expect("rejectNotes is an array");
    assert_eq!(notes.len(), 2, "both notes present");
    assert_eq!(notes[0][0], "v1", "rejectNotes[0] must be v1 after sort");
    assert_eq!(notes[0][1], "v1 says broken");
    assert_eq!(notes[1][0], "v3", "rejectNotes[1] must be v3 after sort");
    assert_eq!(notes[1][1], "v3 says wrong");

    // nullVerifiers sorted ascending.
    let nulls = rejection["nullVerifiers"]
        .as_array()
        .expect("nullVerifiers is an array");
    assert_eq!(nulls.len(), 2);
    assert_eq!(nulls[0], "v1");
    assert_eq!(nulls[1], "v3");

    // signatureFailures sorted ascending too.
    let sigfails = rejection["signatureFailures"]
        .as_array()
        .expect("signatureFailures is an array");
    assert_eq!(sigfails.len(), 1);
    assert_eq!(sigfails[0][0], "v2");
}

// ---------------------------------------------------------------------------
// Scenario 3 (tasks.md §1.6) — print_success: one JSON line on stdout (Json),
// verbatim human line on stdout (Human).
// ---------------------------------------------------------------------------

#[test]
fn print_success_json_emits_exactly_one_stdout_line() {
    let env = JsonEnvelope {
        ok: true,
        command: "new".to_string(),
        goal_id: Some("g-123".to_string()),
        round: Some(1),
        verifier_id: None,
        status: Some("consensus-passed".to_string()),
        hash: Some("260715-deadbeef".to_string()),
        full_digest: Some("a".repeat(64)),
        needs: None,
        rejection: None,
        verdicts: None,
        state: None,
        error: None,
    };
    let human_line = "goalId: g-123\n260715-deadbeef";

    // --- Json mode: exactly ONE line on stdout, parseable as JSON matching the envelope.
    let mut out: Vec<u8> = Vec::new();
    Output::Json.print_success(&env, human_line, &mut out);
    let json_out = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = json_out.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "Json print_success must write EXACTLY one line to stdout (got {json_out:?})"
    );
    let parsed: Value = serde_json::from_str(lines[0])
        .expect("the single Json stdout line must parse as a JSON object");
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["command"], "new");
    assert_eq!(parsed["goalId"], "g-123");
    assert_eq!(parsed["round"].as_u64(), Some(1));
    assert_eq!(parsed["status"], "consensus-passed");
    assert_eq!(parsed["hash"], "260715-deadbeef");
    assert_eq!(parsed["fullDigest"], "a".repeat(64));
    assert!(
        !parsed.as_object().unwrap().contains_key("error"),
        "no error field on a success envelope"
    );

    // --- Human mode: writes the human line VERBATIM, and the output is NOT valid JSON.
    let mut out2: Vec<u8> = Vec::new();
    Output::Human.print_success(&env, human_line, &mut out2);
    let human_out = String::from_utf8(out2).unwrap();
    assert_eq!(
        human_out.trim(),
        human_line,
        "Human print_success must write the human line verbatim (got {human_out:?})"
    );
    assert!(
        serde_json::from_str::<Value>(human_out.trim()).is_err(),
        "Human print_success output must NOT be parseable as JSON (got {human_out:?})"
    );
}

// ---------------------------------------------------------------------------
// Scenario 4 (tasks.md §1.5) — print_error: one ok:false envelope on stdout
// (Json); human error text on stderr only (both modes).
// ---------------------------------------------------------------------------

#[test]
fn print_error_json_emits_one_envelope_on_stdout_and_human_on_stderr() {
    let env = JsonEnvelope {
        ok: false,
        command: "new".to_string(),
        goal_id: None,
        round: None,
        verifier_id: None,
        status: None,
        hash: None,
        full_digest: None,
        needs: None,
        rejection: None,
        verdicts: None,
        state: None,
        error: Some("missing store".to_string()),
    };
    let human_err = "error: missing store directory";

    // --- Json mode: exactly one ok:false JSON object on stdout; human text on stderr.
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    Output::Json.print_error(&env, human_err, &mut out, &mut err);
    let json_out = String::from_utf8(out).unwrap();
    let err_out = String::from_utf8(err).unwrap();

    let lines: Vec<&str> = json_out.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "Json print_error must write EXACTLY one JSON object to stdout (got {json_out:?})"
    );
    let parsed: Value =
        serde_json::from_str(lines[0]).expect("Json error stdout must be a single JSON object");
    assert_eq!(parsed["ok"], false);
    assert!(
        parsed["error"]
            .as_str()
            .unwrap()
            .contains("missing store"),
        "error envelope carries the error string: {parsed}"
    );
    assert!(
        !parsed.as_object().unwrap().contains_key("hash"),
        "no hash on an error envelope"
    );
    // Human-readable text stays on stderr (the debugging channel) under Json mode too.
    assert!(
        err_out.contains("missing store"),
        "Json mode must ALSO mirror the human error text to stderr: {err_out:?}"
    );
    assert!(
        !json_out.contains(human_err),
        "human error text must NOT appear on stdout under Json mode: {json_out:?}"
    );

    // --- Human mode: human error text on stderr ONLY; stdout empty.
    let mut out2: Vec<u8> = Vec::new();
    let mut err2: Vec<u8> = Vec::new();
    Output::Human.print_error(&env, human_err, &mut out2, &mut err2);
    let human_stdout = String::from_utf8(out2).unwrap();
    let human_stderr = String::from_utf8(err2).unwrap();
    assert!(
        human_stdout.trim().is_empty(),
        "Human print_error must write NOTHING to stdout (got {human_stdout:?})"
    );
    assert!(
        human_stderr.contains("missing store"),
        "Human print_error must write the human error to stderr: {human_stderr:?}"
    );
    assert!(
        serde_json::from_str::<Value>(human_stderr.trim()).is_err(),
        "Human error stderr must be plain text, not JSON: {human_stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 5 (tasks.md §2.1) — top-level `--json` flag parses both before and
// after the subcommand (global flag, design D2).
// ---------------------------------------------------------------------------

#[test]
fn flag_jewilo_global_json_parses_before_and_after_subcommand() {
    // --json BEFORE the subcommand.
    let cli_before =
        VerifierLoopCli::parse_from(["verifier-loop", "--json", "NEW", "goal"]);
    assert!(
        cli_before.json,
        "`jewilo --json NEW <goal>` must set json=true"
    );

    // No --json at all.
    let cli_none = VerifierLoopCli::parse_from(["verifier-loop", "NEW", "goal"]);
    assert!(
        !cli_none.json,
        "no --json must default json=false"
    );

    // --json AFTER the subcommand. clap with `global = true` (design D2) MUST accept both
    // placements. If the GREEN author discovers this specific struct layout forces a single
    // placement, they MUST document the supported placement in a comment AND update the
    // spec scenario; this assertion forces that clarification by requiring json==true here.
    let cli_after =
        VerifierLoopCli::parse_from(["verifier-loop", "NEW", "goal", "--json"]);
    assert!(
        cli_after.json,
        "`jewilo NEW <goal> --json` must ALSO set json=true (global flag, design D2)"
    );
}
