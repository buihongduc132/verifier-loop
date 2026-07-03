//! Verifier prompt rendering (tasks.md §9, verifier-prompt spec).
//!
//! Blind + frozen-artifact: V* sees identity, goalText, context, (resume) fix/prev-notes, and a
//! frozen snapshot (cwd, `git status --porcelain`, file edit times, `git diff` truncated to
//! gitDiffMaxChars). V* does NOT see round number, other verdicts, n/m, or the hash (D10).
//!
//! Variables (opt-in via template): `{{goalId}} {{verifierId}} {{round}} {{prevRound}}
//! {{goalText}} {{context}} {{fixNotes}} {{prevNotes}} {{cwd}} {{gitStatus}}
//! {{fileEditTimes}} {{gitDiff}} {{gitDiffMaxChars}} {{process.env.*}}`.
//! Null template -> baked-in verifier-policy default (sourced from the verifier-loop skill).
//!
//! **STATUS: RED stub** — types + signatures present so the integration test compiles; the
//! bodies are deliberately non-functional and will fail the §9 test assertions. GREEN lands
//! in a separate commit.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name persisted per verifier slot: `rounds/<round>/<verifierId>/initial-prompt.txt`.
pub const INITIAL_PROMPT_FILE: &str = "initial-prompt.txt";

/// Variables fed into the template engine for a single verifier prompt.
///
/// **Blindness by construction**: this struct deliberately has NO fields for the round
/// threshold (n/m), other verifiers' verdicts, or the completion hash. A verifier can only
/// see data that is present here; anything absent cannot leak.
#[derive(Debug, Clone, Copy)]
pub struct PromptVars<'a> {
    pub goal_id: &'a str,
    pub verifier_id: &'a str,
    pub round: u32,
    pub prev_round: Option<u32>,
    pub goal_text: &'a str,
    pub context: Option<&'a str>,
    /// Resume only: A's `--fix` text.
    pub fix_notes: Option<&'a str>,
    /// Resume only: THIS verifier's own prior-round notes (never a peer's).
    pub prev_notes: Option<&'a str>,
    pub cwd: &'a str,
    pub git_status: &'a str,
    pub file_edit_times: &'a str,
    pub git_diff: &'a str,
    pub git_diff_max_chars: u64,
    pub truncated: bool,
}

/// Frozen artifact snapshot captured at spawn time (LD11): cwd, `git status --porcelain`,
/// file edit times, and `git diff` truncated to `git_diff_max_chars`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub cwd: String,
    pub git_status: String,
    pub file_edit_times: String,
    pub git_diff: String,
    pub git_diff_max_chars: u64,
    pub truncated: bool,
}

/// Renders the round-1 (NEW) prompt. `template = None` -> baked-in default.
///
/// **RED stub**: returns an empty string, which fails every RED assertion.
pub fn render(_template: Option<&str>, _vars: &PromptVars<'_>) -> Result<String, PromptError> {
    Ok(String::new())
}

/// Renders the RESUME prompt. `template = None` -> baked-in resume default.
///
/// **RED stub**: returns an empty string.
pub fn render_resume(
    _template: Option<&str>,
    _vars: &PromptVars<'_>,
) -> Result<String, PromptError> {
    Ok(String::new())
}

/// Captures the frozen artifact snapshot for `cwd`.
///
/// **RED stub**: always errors so the snapshot tests fail closed.
pub fn capture_snapshot(_cwd: &Path, _max_chars: u64) -> Result<Snapshot, PromptError> {
    Err(PromptError::SnapshotCapture("not implemented (RED)".into()))
}

/// The baked-in default round-1 prompt template, embedding the verifier detective policy.
///
/// **RED stub**: empty so the default-template assertions fail.
pub fn default_template() -> &'static str {
    ""
}

/// The baked-in default resume prompt template.
///
/// **RED stub**: empty.
pub fn default_resume_template() -> &'static str {
    ""
}

/// Persists the rendered prompt to `rounds/<round>/<verifierId>/initial-prompt.txt`.
///
/// **RED stub**: returns an error so persistence tests fail.
pub fn write_initial_prompt(
    _goal_root: &Path,
    _goal_id: &str,
    _verifier_id: &str,
    _round: u32,
    _rendered: &str,
) -> Result<PathBuf, PromptError> {
    Err(PromptError::Persistence("not implemented (RED)".into()))
}

/// Errors emitted by prompt rendering / capture.
#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("snapshot capture failed: {0}")]
    SnapshotCapture(String),
    #[error("persistence failed: {0}")]
    Persistence(String),
    #[error("render failed: {0}")]
    Render(String),
}
