//! JSON output envelope + formatter (`add-json-output-mode`).
//!
//! A single stable JSON object is emitted on stdout under `--json` for both binaries
//! (`jewilo` / `jewije`). The envelope is shared, per-command richness is additive via
//! `Option<…>` fields that are skipped when `None` (serde `skip_serializing_if`).
//!
//! Field names are camelCase in the serialized JSON (matching the existing on-disk
//! artifact convention — design D5). Rust fields stay snake_case; `#[serde(rename_all =
//! "camelCase")]` does the mapping.
//!
//! Fail-closed invariants are preserved: this module is purely a presentation layer. It
//! never extends the completion-hash inputs, never alters exit codes, never converts a NULL
//! verdict into APPROVE. The decision about *what* to print stays in the caller; this
//! module decides only *how* (one JSON line on stdout vs the legacy free-text path).

use std::io;

use serde::Serialize;

/// The uniform JSON output envelope for `jewilo` and `jewije` (design D1).
///
/// `ok` and `command` are always present. Every other field is `Option<…>` and is omitted
/// from the serialized JSON entirely when `None` (`#[serde(skip_serializing_if =
/// "Option::is_none")]`). Field names serialize camelCase (`goal_id` → `goalId`,
/// `full_digest` → `fullDigest`, `verifier_id` → `verifierId`, …).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonEnvelope {
    /// `true` for a successful command, `false` for any error/failure path.
    pub ok: bool,
    /// The command name (`"new"`, `"resume"`, `"recover"`, `"status"`, `"verdict"`, …).
    pub command: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verifier_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection: Option<RejectionBreakdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdicts: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Carries the STATS / AUDIT body under `--json` (add-json-output-mode Blocker B).
    /// Omitted entirely when `None`. STATS/AUDIT success envelopes carry `report` and
    /// omit `status`; the bare body is the byte-identical legacy output under Human mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Rejection breakdown lifted into the envelope on a rejected consensus (design D5).
///
/// All three arrays are sorted by verifierId ascending by `from_unsorted` for
/// deterministic consumer equality / golden-file testing. Each tuple is `(verifierId,
/// note)` so the serialized form is an array of `[verifierId, note]` pairs — matching the
/// on-disk artifact convention.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectionBreakdown {
    /// `(verifierId, note)` pairs for every REJECT verdict this round.
    pub reject_notes: Vec<(String, String)>,
    /// VerifierIds that emitted a NULL verdict (no APPROVE/REJECT recorded).
    pub null_verifiers: Vec<String>,
    /// `(verifierId, reason)` pairs for verdicts whose signature failed verification.
    pub signature_failures: Vec<(String, String)>,
}

impl RejectionBreakdown {
    /// Build a `RejectionBreakdown` from unsorted inputs, sorting all three arrays by
    /// verifierId ascending. Sorting is stable so equal verifierIds keep their relative
    /// order (which should not occur in practice — verifierIds are unique per round).
    pub fn from_unsorted(
        mut reject_notes: Vec<(String, String)>,
        mut null_verifiers: Vec<String>,
        mut signature_failures: Vec<(String, String)>,
    ) -> Self {
        reject_notes.sort_by(|(a, _), (b, _)| a.cmp(b));
        null_verifiers.sort();
        signature_failures.sort_by(|(a, _), (b, _)| a.cmp(b));
        Self {
            reject_notes,
            null_verifiers,
            signature_failures,
        }
    }
}

/// Output channel selector (design D6). Chosen once at startup from the parsed `--json`
/// flag; routes every success / error site through a single abstraction so a legacy line
/// can never leak into JSON stdout.
///
/// The writer-parameterised form lets the formatter stay unit-testable in-process without
/// adding a stdout-capture dev-dependency. The bins wire `&mut io::stdout()` /
/// `&mut io::stderr()` at every call site.
pub enum Output {
    /// Legacy free-text mode (default). Human lines go to stdout on success; errors go to
    /// stderr only (matching today's behavior).
    Human,
    /// Machine-readable JSON mode (`--json`). Exactly one envelope object is written to
    /// stdout; the human-readable error text is still mirrored to stderr as a debugging
    /// aid (design D7).
    Json,
}

impl Output {
    /// Print a successful command result.
    ///
    /// * `Human`: writes `human_line` verbatim to `out` (stdout). This IS the legacy
    ///   behavior — the human line is the only stdout output.
    /// * `Json`: writes exactly one `serde_json::to_string(env)` line to `out` (stdout).
    ///   The human line is intentionally NOT printed under Json (stdout is the single
    ///   structured parse point).
    pub fn print_success<W: io::Write>(
        &self,
        env: &JsonEnvelope,
        human_line: &str,
        out: &mut W,
    ) {
        match self {
            Output::Human => {
                let _ = writeln!(out, "{human_line}");
            }
            // D0 / D1: one JSON object total on stdout. Newline-terminated so it composes
            // cleanly with shell pipelines.
            Output::Json => {
                let line = serde_json::to_string(env)
                    .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"envelope-serialize-failed\"}".to_string());
                let _ = writeln!(out, "{line}");
            }
        }
    }

    /// Print a failed command result (error path).
    ///
    /// * `Human`: writes `human_err` to `err` (stderr) only — NOTHING to `out` (stdout),
    ///   matching the legacy "errors go to stderr" behavior.
    /// * `Json`: writes exactly one `serde_json::to_string(env)` object to `out` (stdout)
    ///   AND mirrors `human_err` to `err` (stderr) as a debugging aid (design D7). The
    ///   human-readable text must NEVER appear on stdout under Json.
    pub fn print_error<W: io::Write>(
        &self,
        env: &JsonEnvelope,
        human_err: &str,
        out: &mut W,
        err: &mut W,
    ) {
        match self {
            Output::Human => {
                // Legacy: errors go to stderr only. Stdout stays empty.
                let _ = writeln!(err, "{human_err}");
            }
            Output::Json => {
                // D7: stdout still carries the structured envelope on failure; the human
                // text rides stderr as the debugging channel.
                let line = serde_json::to_string(env)
                    .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"envelope-serialize-failed\"}".to_string());
                let _ = writeln!(out, "{line}");
                let _ = writeln!(err, "{human_err}");
            }
        }
    }
}
