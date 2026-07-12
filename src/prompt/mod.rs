//! Verifier prompt rendering (tasks.md §9, verifier-prompt spec).
//!
//! Blind + frozen-artifact: V* sees identity, goalText, context, (resume) fix/prev-notes, and a
//! frozen snapshot (cwd, `git status --porcelain`, file edit times, `git diff` truncated to
//! gitDiffMaxChars). V* does NOT see round number, other verdicts, n/m, or the hash (D10).
//!
//! Variables (opt-in via template): `{{goalId}} {{verifierId}} {{round}} {{prevRound}}
//! {{goalText}} {{context}} {{fixNotes}} {{prevNotes}} {{cwd}} {{gitStatus}}
//! {{fileEditTimes}} {{gitDiff}} {{gitDiffMaxChars}} {{process.env.*}}`.
//! Null template -> baked-in verifier-policy default (sourced from the verifier-loop skill,
//! `references/verifier.md`, embedded verbatim below).
//!
//! ## Blinding model
//!
//! [`PromptVars`] deliberately carries NO fields for n/m, peer verdicts, or the completion
//! hash. Anything not in the struct cannot leak into a rendered prompt. The round number is
//! available as an *opt-in* template var (`{{round}}`) — the baked-in defaults do not use it,
//! so V* is blind to the round unless an operator explicitly writes `{{round}}` into the
//! template (LD12). This is the template-as-config model (LD24): there is no boolean flag.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// File name persisted per verifier slot: `rounds/<round>/<verifierId>/initial-prompt.txt`.
pub const INITIAL_PROMPT_FILE: &str = "initial-prompt.txt";

/// Truncation indicator appended to a truncated `git diff` snapshot.
pub const TRUNCATION_INDICATOR: &str = "…[truncated]";

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

// ---------------------------------------------------------------------------
// Baked-in verifier detective policy (verbatim from the verifier-loop skill,
// references/verifier.md). This is the V* operating contract: zero trust, demand proof,
// no sycophancy. Kept as a const so the CLI binary is self-contained (D10).
// ---------------------------------------------------------------------------

/// The verifier detective policy text, embedded verbatim from the verifier-loop skill
/// (`references/verifier.md`). The default prompt template prepends this so every
/// verifier receives its operating rules without depending on the skill file at runtime.
pub const VERIFIER_POLICY: &str = include_str!("verifier_policy.txt");

/// The baked-in default round-1 (NEW) prompt template.
///
/// Embeds the verifier detective policy, then renders the identity + goal + context +
/// frozen snapshot. **No round number** is rendered (LD12): V* must be blind to the round.
///
/// Built at compile time via `concat!`: a short policy preamble + the full canonical policy
/// text (sourced verbatim from the verifier-loop skill) + the template body that references
/// the opt-in vars.
pub const DEFAULT_TEMPLATE: &str = concat!(
    "You are verifier {{verifierId}} for goal {{goalId}}.\n\n",
    "# Verifier Detective Policy (canonical, from verifier-loop skill)\n\n",
    include_str!("verifier_policy.txt"),
    "\n\n---\n\n",
    include_str!("default_template.txt"),
);

/// The baked-in default RESUME prompt template.
///
/// Like the round-1 default but additionally surfaces A's fix notes (`{{fixNotes}}`) and
/// this verifier's OWN prior-round notes (`{{prevNotes}}`) per LD24. Both are optional
/// fields; absence leaves the slot empty rather than leaking "None" prose.
pub const DEFAULT_RESUME_TEMPLATE: &str = concat!(
    "You are verifier {{verifierId}} for goal {{goalId}} (resumed).\n\n",
    "# Verifier Detective Policy (canonical, from verifier-loop skill)\n\n",
    include_str!("verifier_policy.txt"),
    "\n\n---\n\n",
    include_str!("default_resume_template.txt"),
);

/// The baked-in default round-1 prompt template WITH the embedded verifier policy.
pub fn default_template() -> &'static str {
    DEFAULT_TEMPLATE
}

/// The baked-in default resume prompt template WITH the embedded verifier policy.
pub fn default_resume_template() -> &'static str {
    DEFAULT_RESUME_TEMPLATE
}

/// The baked-in default round-1 template WITHOUT the embedded `VERIFIER_POLICY` block
/// (design D2 — override semantics). Used when a custom `verifierPromptFile` is
/// configured: the custom file REPLACES the built-in policy, so the built-in block is
/// omitted to avoid the 2x duplication (62KB wasted) that D2 eliminates.
///
/// Identity preamble + body file only. The bin prepends the custom file via
/// [`prepend_custom`], producing exactly one policy source (the custom file).
pub const DEFAULT_TEMPLATE_NO_POLICY: &str = concat!(
    "You are verifier {{verifierId}} for goal {{goalId}}.\n\n",
    include_str!("default_template.txt"),
);

/// The baked-in default resume template WITHOUT the embedded `VERIFIER_POLICY` block
/// (design D2 — override semantics). See [`DEFAULT_TEMPLATE_NO_POLICY`].
pub const DEFAULT_RESUME_TEMPLATE_NO_POLICY: &str = concat!(
    "You are verifier {{verifierId}} for goal {{goalId}} (resumed).\n\n",
    include_str!("default_resume_template.txt"),
);

/// The baked-in default round-1 template WITHOUT the embedded verifier policy (D2).
pub fn default_template_no_policy() -> &'static str {
    DEFAULT_TEMPLATE_NO_POLICY
}

/// The baked-in default resume template WITHOUT the embedded verifier policy (D2).
pub fn default_resume_template_no_policy() -> &'static str {
    DEFAULT_RESUME_TEMPLATE_NO_POLICY
}

/// Renders the round-1 (NEW) prompt. `template = None` -> baked-in default.
pub fn render(template: Option<&str>, vars: &PromptVars<'_>) -> Result<String, PromptError> {
    let tpl = match template {
        Some(t) => t,
        None => default_template(),
    };
    render_inner(tpl, vars)
}

/// Renders the RESUME prompt. `template = None` -> baked-in resume default.
pub fn render_resume(template: Option<&str>, vars: &PromptVars<'_>) -> Result<String, PromptError> {
    let tpl = match template {
        Some(t) => t,
        None => default_resume_template(),
    };
    render_inner(tpl, vars)
}

/// Prepends a custom verifier-prompt preamble (loaded from `verifierPromptFile`) to an
/// already-rendered prompt. The custom text is inserted RAW (no `{{var}}` expansion),
/// followed by a `---` separator line, then the baked-in rendered prompt:
///
/// ```text
/// <custom file contents>---\n<rendered baked-in prompt>
/// ```
///
/// `custom = None` is a no-op (today's baked-in-only behavior is preserved).
pub fn prepend_custom(rendered: String, custom: Option<&str>) -> String {
    match custom {
        // Ensure exactly one newline separates the custom preamble from the `---` rule,
        // regardless of whether the file ends with a trailing newline.
        Some(c) => {
            let nl = if c.ends_with('\n') { "" } else { "\n" };
            format!("{c}{nl}---\n{rendered}")
        }
        None => rendered,
    }
}

/// Core template engine: linear scan, `{{name}}` substitution.
///
/// - Known vars resolve from [`PromptVars`].
/// - `{{process.env.X}}` resolves via [`std::env::var`]; missing -> empty string.
/// - Unknown vars resolve to empty string (never panic, never leave `{{...}}` in output).
fn render_inner(template: &str, vars: &PromptVars<'_>) -> Result<String, PromptError> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // find closing }}
            if let Some(close) = find_close(&template[i + 2..]) {
                let name = &template[i + 2..i + 2 + close];
                out.push_str(&resolve_var(name.trim(), vars));
                i += 2 + close + 2; // skip {{name}}
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    Ok(out)
}

/// Finds the offset of the closing `}}` after `start`'s base, scanning from `s`.
/// Returns the byte offset of `}}` start relative to `s`, or `None` if unterminated.
fn find_close(s: &str) -> Option<usize> {
    s.find("}}")
}

/// Resolves a single template variable name to its string value.
fn resolve_var(name: &str, vars: &PromptVars<'_>) -> String {
    if let Some(rest) = name.strip_prefix("process.env.") {
        return std::env::var(rest).unwrap_or_default();
    }
    match name {
        "goalId" => vars.goal_id.to_string(),
        "verifierId" => vars.verifier_id.to_string(),
        // Opt-in only (LD12). Defaults do not reference this var.
        "round" => vars.round.to_string(),
        "prevRound" => vars.prev_round.map(|r| r.to_string()).unwrap_or_default(),
        "goalText" => vars.goal_text.to_string(),
        "context" => vars.context.unwrap_or("").to_string(),
        "fixNotes" => vars.fix_notes.unwrap_or("").to_string(),
        "prevNotes" => vars.prev_notes.unwrap_or("").to_string(),
        "cwd" => vars.cwd.to_string(),
        "gitStatus" => vars.git_status.to_string(),
        "fileEditTimes" => vars.file_edit_times.to_string(),
        "gitDiff" => {
            // Defense-in-depth: never leak > gitDiffMaxChars to V*, even if the caller
            // handed an over-cap diff. capture_snapshot already truncates, but render
            // enforces the cap again so a hand-built PromptVars cannot bypass it.
            let (truncated, _) = truncate_diff(vars.git_diff, vars.git_diff_max_chars);
            truncated
        }
        "gitDiffMaxChars" => vars.git_diff_max_chars.to_string(),
        _ => String::new(), // unknown var -> empty
    }
}

/// Truncates `s` to `max_chars` characters, appending [`TRUNCATION_INDICATOR`] when truncated.
pub fn truncate_diff(s: &str, max_chars: u64) -> (String, bool) {
    let max = max_chars as usize;
    if s.len() <= max {
        return (s.to_string(), false);
    }
    // Take a char boundary near the cap to avoid splitting a multi-byte char.
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = String::with_capacity(end + TRUNCATION_INDICATOR.len());
    truncated.push_str(&s[..end]);
    truncated.push_str(TRUNCATION_INDICATOR);
    (truncated, true)
}

/// Captures the frozen artifact snapshot for `cwd` (LD11).
///
/// Runs, in `cwd`:
///   * `git status --porcelain`
///   * `git diff HEAD` (unpaged; staged + unstaged vs last commit)
///   * `git ls-files` then reads each file's mtime (best-effort; a missing/unreadable
///     file is skipped, never fatal).
///   * `git rev-parse --show-toplevel` is checked first; a non-git cwd fails closed.
///
/// The diff is truncated to `max_chars` with [`truncate_diff`]. Any git command that exits
/// non-zero (other than an empty diff) is a hard error — V* must never receive a silently
/// empty snapshot, which would let A hide uncommitted regressions.
pub fn capture_snapshot(cwd: &Path, max_chars: u64) -> Result<Snapshot, PromptError> {
    capture_snapshot_with(cwd, max_chars, 8_000)
}

/// Same as [`capture_snapshot`] but with explicit caps for the diff (`max_chars`) and
/// the fileEditTimes block (`file_edit_times_max_chars`). Prompt-bloat fix D1 threads
/// the latter from `Config.file_edit_times_max_chars`.
/// `capture_snapshot_with` body (shared by the single-cap + dual-cap entrypoints).
pub fn capture_snapshot_with(
    cwd: &Path,
    max_chars: u64,
    file_edit_times_max_chars: u64,
) -> Result<Snapshot, PromptError> {
    // Fail closed if this is not a git work tree.
    git_check(cwd)?;

    let git_status = git_capture(cwd, &["status", "--porcelain"])?;
    // Full working-tree delta vs the last commit (staged AND unstaged). Bare
    // `git diff` would hide staged changes, letting an author `git add` a
    // regression and keep it invisible to every verifier. On a repo with no
    // commits yet (fresh `git init`), `git diff HEAD` errors — fall back to
    // BOTH `git diff --cached` (staged) AND `git diff` (unstaged) so the
    // verifier still sees the full working tree (untracked files are listed by
    // `git status --porcelain` above).
    let raw_diff = match git_capture(cwd, &["diff", "HEAD"]) {
        Ok(d) => d,
        Err(_) if !head_exists(cwd)? => {
            let staged = git_capture(cwd, &["diff", "--cached"])?;
            let unstaged = git_capture(cwd, &["diff"])?;
            if staged.is_empty() {
                unstaged
            } else if unstaged.is_empty() {
                staged
            } else {
                format!("{staged}\n{unstaged}")
            }
        }
        Err(e) => return Err(e),
    };
    let (git_diff, truncated) = truncate_diff(&raw_diff, max_chars);
    let file_edit_times = capture_file_edit_times(cwd, file_edit_times_max_chars)?;

    Ok(Snapshot {
        cwd: cwd.to_string_lossy().into_owned(),
        git_status,
        file_edit_times,
        git_diff,
        git_diff_max_chars: max_chars,
        truncated,
    })
}

/// Returns true iff `cwd` has at least one commit (HEAD resolves). Used to pick
/// between `git diff HEAD` and the fresh-repo fallback without swallowing real
/// git errors (fail-closed still holds for `git diff` failures on a real repo).
fn head_exists(cwd: &Path) -> Result<bool, PromptError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .output()
        .map_err(|e| PromptError::SnapshotCapture(format!("git rev-parse failed: {e}")))?;
    Ok(out.status.success())
}

/// Asserts `cwd` is inside a git work tree; errors otherwise (fail-closed).
fn git_check(cwd: &Path) -> Result<(), PromptError> {
    let out = Command::new("git")
        .args([
            "-C",
            &cwd.to_string_lossy(),
            "rev-parse",
            "--is-inside-work-tree",
        ])
        .output()
        .map_err(|e| PromptError::SnapshotCapture(format!("git not available: {e}")))?;
    if !out.status.success() || String::from_utf8_lossy(&out.stdout).trim() != "true" {
        return Err(PromptError::SnapshotCapture(format!(
            "{} is not a git work tree",
            cwd.display()
        )));
    }
    Ok(())
}

/// Runs a git command in `cwd` and returns its stdout as a string. Non-zero exit is an error.
fn git_capture(cwd: &Path, args: &[&str]) -> Result<String, PromptError> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd);
    cmd.args(args);
    cmd.env("GIT_PAGER", "cat");
    let out = cmd
        .output()
        .map_err(|e| PromptError::SnapshotCapture(format!("git {:?} failed: {e}", args)))?;
    if !out.status.success() {
        return Err(PromptError::SnapshotCapture(format!(
            "git {:?} exited {}: {}",
            args,
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Captures `<path>:<mtime_secs>` for **changed files only** (`git status --porcelain -z`,
/// not `git ls-files`). Best-effort per file: a missing or unreadable file is skipped,
/// never fatal to the whole snapshot. The block is byte-capped to `max_chars` with an
/// indicator when exceeded (prompt-bloat fix D1).
///
/// `git status --porcelain -z` (NUL-delimited, no C-quoting) records look like:
///   - single path: `XY <path>\0`
///   - rename:      `XY <new/dest>\0<old/source>\0`  (destination FIRST, then source;
///     this is the OPPOSITE order of the human-readable `->` form)
///
/// We parse the NUL-delimited stream carefully so pathnames containing spaces or the
/// literal substring ` -> ` are never mis-split.
pub fn capture_file_edit_times(cwd: &Path, max_chars: u64) -> Result<String, PromptError> {
    // Use `-z` (NUL-delimited) so pathnames containing spaces or the literal ` -> `
    // are never C-quoted or mis-split by the `rsplit(" -> ")` heuristic. Each record
    // is `XY <path>\0` for single-path statuses, or `XY <new/dest>\0<old/source>\0`
    // for renames (note: under `-z` the NEW/destination path comes FIRST, then the
    // OLD/source — the opposite of the human-readable `old -> new` form).
    let status = git_capture(cwd, &["status", "--porcelain", "-z"])?;
    let mut lines: Vec<String> = Vec::new();
    let mut records = status.split('\u{0000}').peekable();
    while let Some(record) = records.next() {
        if record.len() < 3 {
            // An empty trailing record (from the final NUL) is fine; skip it.
            continue;
        }
        // Porcelain record: 2-char XY status, 1 space, then the path.
        let status_chars = &record[..2];
        let path_part = &record[3..];
        // Renames (R/C status in either column) emit TWO NUL-delimited entries under
        // `-z`: the format is `XY <new/dest>\0<old/source>\0` (destination path FIRST,
        // then source). So the CURRENT record's path is the NEW/destination path (the
        // post-rename file that exists on disk and that we want to stat); the NEXT
        // record is the OLD/source path and must be consumed and discarded so it is
        // not treated as a standalone changed file.
        let path = if is_rename_status(status_chars) {
            // Consume + discard the trailing old-path record.
            let _old = records.next();
            path_part
        } else {
            path_part
        };
        if path.is_empty() {
            continue;
        }
        let abs = cwd.join(path);
        match fs::metadata(&abs).and_then(|m| m.modified()) {
            Ok(modified) => {
                let secs = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                lines.push(format!("{path}:{secs}"));
            }
            Err(_) => {
                // Skip unreadable/missing file; do not fail the snapshot.
                lines.push(format!("{path}:?"));
            }
        }
    }
    let joined = lines.join("\n");
    let (capped, _) = truncate_diff(&joined, max_chars);
    Ok(capped)
}

/// Returns true if a porcelain `XY` status (two status characters) denotes a rename
/// (R in either the staged-X or working-tree-Y column). A rename record under `-z` is
/// followed by a second NUL-delimited path: `XY <new/dest>\0<old/source>\0` — the
/// NEW/destination path comes FIRST, the OLD/source path comes SECOND.
fn is_rename_status(xy: &str) -> bool {
    xy.starts_with('R')
        || xy.starts_with('C')
        || xy.get(1..2) == Some("R")
        || xy.get(1..2) == Some("C")
}

/// Truncates the `--context` input to `max_chars` characters, appending
/// [`TRUNCATION_INDICATOR`] when truncated. Prompt-bloat fix D3. Mirrors
/// [`truncate_diff`] semantics (char-boundary-safe).
pub fn cap_context(s: &str, max_chars: u64) -> (String, bool) {
    truncate_diff(s, max_chars)
}

/// Returns a per-section budget warning when `rendered` exceeds `budget` bytes, else
/// `None`. The warning does NOT block the spawn — it is informational for the operator.
/// Prompt-bloat fix D4.
///
/// The warning cites the total rendered size and the budget, plus a coarse breakdown by
/// section heading (the function scans for `# `-prefixed headings and reports their byte
/// spans). The breakdown is best-effort: if no headings are present it still reports the
/// total vs budget.
pub fn budget_warning(rendered: &str, budget: usize) -> Option<String> {
    let total = rendered.len();
    if total <= budget {
        return None;
    }
    let mut msg = format!(
        "warning: rendered verifier prompt is {total} bytes, over the {budget}-byte budget (will still spawn).\n",
    );
    msg.push_str("section breakdown:\n");
    // Best-effort: split on `# ` headings and report each section's size.
    let mut sections: Vec<(&str, usize)> = Vec::new();
    let bytes = rendered.as_bytes();
    let mut start = 0;
    let mut heading: Option<&str> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#'
            && (i + 1 < bytes.len() && bytes[i + 1] == b' ')
            && (i == 0 || bytes[i - 1] == b'\n')
        {
            // close previous section
            if let Some(h) = heading {
                sections.push((h, i - start));
            }
            // find end of heading line
            let line_end = bytes[i..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|p| i + p)
                .unwrap_or(bytes.len());
            start = i;
            heading = Some(&rendered[i + 2..line_end]);
            i = line_end;
        } else {
            i += 1;
        }
    }
    if let Some(h) = heading {
        sections.push((h, bytes.len() - start));
    }
    if sections.is_empty() {
        msg.push_str(&format!("  (no headings detected) total = {total} bytes\n"));
    } else {
        for (h, sz) in sections {
            msg.push_str(&format!("  # {h}: ~{sz} bytes\n"));
        }
    }
    Some(msg)
}
/// forming part of the per-verifier trust trail.
pub fn write_initial_prompt(
    goal_root: &Path,
    _goal_id: &str,
    verifier_id: &str,
    round: u32,
    rendered: &str,
) -> Result<PathBuf, PromptError> {
    let slot = goal_root
        .join("rounds")
        .join(round.to_string())
        .join(verifier_id);
    fs::create_dir_all(&slot).map_err(|e| PromptError::Persistence(e.to_string()))?;
    let path = slot.join(INITIAL_PROMPT_FILE);
    fs::write(&path, rendered).map_err(|e| PromptError::Persistence(e.to_string()))?;
    Ok(path)
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

// Allow `?` on io::Error inside helpers without a noisy conversion at every call site.
impl From<io::Error> for PromptError {
    fn from(e: io::Error) -> Self {
        PromptError::Persistence(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    // Behavioural coverage of the public surface lives in the integration test
    // `tests/prompt.rs` (the §9 RED→GREEN contract). These unit tests pin pure helpers
    // that the integration test exercises only indirectly, keeping coverage honest on
    // the truncation + template-engine internals.

    use super::*;

    #[test]
    fn truncate_under_cap_returns_unchanged() {
        let (s, trunc) = truncate_diff("abc", 100);
        assert_eq!(s, "abc");
        assert!(!trunc);
    }

    #[test]
    fn truncate_over_cap_appends_indicator() {
        let (s, trunc) = truncate_diff(&"x".repeat(50), 10);
        assert!(trunc);
        assert!(s.ends_with(TRUNCATION_INDICATOR), "{s}");
        assert!(s.len() < 50);
    }

    #[test]
    fn truncate_respects_char_boundary() {
        // multi-byte chars must not be split
        let input = "é".repeat(20); // 2 bytes each
        let (s, _) = truncate_diff(&input, 5);
        assert!(s.chars().all(|c| c == 'é' || s.ends_with(']') || c == '…'));
    }

    #[test]
    fn render_inner_handles_unterminated_brace() {
        let v = PromptVars {
            goal_id: "g",
            verifier_id: "v",
            round: 1,
            prev_round: None,
            goal_text: "t",
            context: None,
            fix_notes: None,
            prev_notes: None,
            cwd: "/",
            git_status: "",
            file_edit_times: "",
            git_diff: "",
            git_diff_max_chars: 100,
            truncated: false,
        };
        // An unterminated `{{` must pass through literally, not panic.
        let out = render_inner("a{{unterminated", &v).unwrap();
        assert_eq!(out, "a{{unterminated");
    }

    #[test]
    fn resolve_var_known_and_unknown() {
        let v = PromptVars {
            goal_id: "g1",
            verifier_id: "v1",
            round: 3,
            prev_round: Some(2),
            goal_text: "gt",
            context: Some("ctx"),
            fix_notes: Some("fix"),
            prev_notes: Some("prev"),
            cwd: "/r",
            git_status: "M",
            file_edit_times: "f:1",
            git_diff: "d",
            git_diff_max_chars: 10,
            truncated: false,
        };
        assert_eq!(resolve_var("goalId", &v), "g1");
        assert_eq!(resolve_var("verifierId", &v), "v1");
        assert_eq!(resolve_var("round", &v), "3");
        assert_eq!(resolve_var("prevRound", &v), "2");
        assert_eq!(resolve_var("goalText", &v), "gt");
        assert_eq!(resolve_var("context", &v), "ctx");
        assert_eq!(resolve_var("fixNotes", &v), "fix");
        assert_eq!(resolve_var("prevNotes", &v), "prev");
        assert_eq!(resolve_var("cwd", &v), "/r");
        assert_eq!(resolve_var("gitStatus", &v), "M");
        assert_eq!(resolve_var("fileEditTimes", &v), "f:1");
        assert_eq!(resolve_var("gitDiff", &v), "d");
        assert_eq!(resolve_var("gitDiffMaxChars", &v), "10");
        assert_eq!(resolve_var("nope", &v), "");
    }

    #[test]
    fn resolve_var_env_namespace() {
        std::env::set_var("VL_UNIT_ENV", "VALUE");
        let v = PromptVars {
            goal_id: "",
            verifier_id: "",
            round: 0,
            prev_round: None,
            goal_text: "",
            context: None,
            fix_notes: None,
            prev_notes: None,
            cwd: "",
            git_status: "",
            file_edit_times: "",
            git_diff: "",
            git_diff_max_chars: 0,
            truncated: false,
        };
        assert_eq!(resolve_var("process.env.VL_UNIT_ENV", &v), "VALUE");
        assert_eq!(resolve_var("process.env.VL_MISSING", &v), "");
        std::env::remove_var("VL_UNIT_ENV");
    }

    #[test]
    fn default_template_consts_are_nonempty_and_embed_policy() {
        assert!(!DEFAULT_TEMPLATE.is_empty());
        assert!(!DEFAULT_RESUME_TEMPLATE.is_empty());
        assert!(!VERIFIER_POLICY.is_empty());
        assert!(
            VERIFIER_POLICY.contains("Verifier") || VERIFIER_POLICY.contains("ZERO trust"),
            "policy must embed the detective contract"
        );
        // The policy heading must appear EXACTLY ONCE in each composed const
        // (the canonical policy is composed only via the `concat!` preamble; the
        // template body files must NOT carry an inline duplicate). design D3.
        let count_template = DEFAULT_TEMPLATE
            .matches("# Verifier Detective Policy")
            .count();
        let count_resume = DEFAULT_RESUME_TEMPLATE
            .matches("# Verifier Detective Policy")
            .count();
        assert_eq!(
            count_template, 1,
            "DEFAULT_TEMPLATE must contain the policy heading exactly once (got {count_template})"
        );
        assert_eq!(
            count_resume, 1,
            "DEFAULT_RESUME_TEMPLATE must contain the policy heading exactly once (got {count_resume})"
        );
    }

    #[test]
    fn capture_file_edit_times_handles_spaces_and_renames() {
        // Build a temp git repo with a space in the filename and a rename entry, then
        // assert `capture_file_edit_times` parses the NUL-delimited porcelain records
        // correctly: it must NOT mis-split on the ` -> ` literal or on spaces.
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        let _ = git_capture(cwd, &["init", "-q"]).unwrap();
        let _ = git_capture(cwd, &["config", "user.email", "t@t.t"]).unwrap();
        let _ = git_capture(cwd, &["config", "user.name", "t"]).unwrap();

        // Create files with spaces and a ` -> ` literal in the name, then commit.
        fs::write(cwd.join("my path.txt"), "a").unwrap();
        fs::write(cwd.join("original.txt"), "b").unwrap();
        fs::write(cwd.join("tracked.txt"), "c").unwrap();
        let _ = git_capture(cwd, &["add", "."]).unwrap();
        let _ = git_capture(cwd, &["commit", "-m", "seed"]).unwrap();

        // Modify the space-named file.
        fs::write(cwd.join("my path.txt"), "changed").unwrap();
        // Force a REAL rename via `git mv` (staged) so porcelain emits an `R` record
        // whose destination also contains a space.
        let _ = git_capture(cwd, &["mv", "original.txt", "renamed dest.txt"]).unwrap();
        // An untracked file with a space + the ` -> ` literal in its name.
        fs::write(cwd.join("arrow -> new.txt"), "untracked").unwrap();

        let lines = capture_file_edit_times(cwd, 8_000).unwrap();
        let paths: std::collections::HashSet<&str> = lines
            .lines()
            .map(|l| l.rsplit_once(':').map(|(p, _)| p).unwrap_or(l))
            .collect();
        // We expect exactly 3 changed entries: the modified space-name, the rename
        // (new path), and the untracked arrow-named file. No other tracked/unchanged
        // file, and crucially NOT the rename's OLD path.
        assert!(
            paths.contains("my path.txt"),
            "space in filename must be captured; got: {lines}"
        );
        assert!(
            paths.contains("renamed dest.txt"),
            "rename destination must be captured; got: {lines}"
        );
        assert!(
            paths.contains("arrow -> new.txt"),
            "untracked file containing ' -> ' must be captured; got: {lines}"
        );
        assert_eq!(
            paths.len(),
            3,
            "exactly 3 changed entries expected; got: {lines}"
        );
        // The rename's OLD path and the unchanged tracked file must NOT appear.
        assert!(
            !lines.contains("original.txt"),
            "rename source (old path) must be omitted; got: {lines}"
        );
        assert!(
            !lines.contains("tracked.txt"),
            "unchanged tracked file must be omitted; got: {lines}"
        );
    }
}
