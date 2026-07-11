// tasks.md §6 (D7) — Strengthened prompt template.
// RED phase: written first, against the spec, BEFORE any implementation.
//
// verifier-spawn ADDED requirement: "Default prompt template ends with explicit verdict
// command". The default prompt template and default resume prompt template SHALL end
// with an explicit fenced bash block showing the exact `verifier-verdict approve
// --notes "..."` and `verifier-verdict reject --notes "..."` invocation pattern. The
// final instruction SHALL be a command, not prose.
//
// Today both default_template.txt and default_resume_template.txt end with prose
// ("register your verdict via the verifier-verdict CLI (approve [--notes "..."] /
// reject --notes "...")"), NOT a fenced bash block. These tests FAIL today — RED.

use verifier_loop::prompt;

fn vars_default() -> prompt::PromptVars<'static> {
    prompt::PromptVars {
        goal_id: "goal-1",
        verifier_id: "v1",
        round: 1,
        prev_round: None,
        goal_text: "build the thing",
        context: Some("ctx"),
        fix_notes: None,
        prev_notes: None,
        cwd: "/repo",
        git_status: " M x\n",
        file_edit_times: "x:1\n",
        git_diff: "diff",
        git_diff_max_chars: 10_000,
        truncated: false,
    }
}

// ---------------------------------------------------------------------------
// §6.1 RED: round-1 default template ends with an explicit verdict command
// ---------------------------------------------------------------------------

#[test]
fn default_template_ends_with_explicit_verdict_command() {
    let v = vars_default();
    let out = prompt::render(None, &v).unwrap();

    // The rendered prompt must contain a fenced ```bash block that shows BOTH
    // verdict commands with --notes. Today the template ends with prose only → FAIL.
    assert!(
        out.contains("```bash"),
        "round-1 default must contain a fenced ```bash block with the verdict command; got tail:\n{}",
        &out[out.len().saturating_sub(400)..]
    );
    assert!(
        out.contains("verifier-verdict approve"),
        "round-1 default must show `verifier-verdict approve` in the fenced block: {out}"
    );
    assert!(
        out.contains("verifier-verdict reject"),
        "round-1 default must show `verifier-verdict reject` in the fenced block: {out}"
    );
    // --notes must appear in the command examples (reject requires it; approve shows it optional).
    assert!(
        out.contains("--notes"),
        "round-1 default verdict command examples must include --notes: {out}"
    );
}

// ---------------------------------------------------------------------------
// §6.2 RED: resume default template ends with an explicit verdict command
// ---------------------------------------------------------------------------

#[test]
fn default_resume_template_ends_with_explicit_verdict_command() {
    let v = prompt::PromptVars {
        fix_notes: Some("fixed the bug"),
        prev_notes: Some("my prior notes"),
        ..vars_default()
    };
    let out = prompt::render_resume(None, &v).unwrap();

    assert!(
        out.contains("```bash"),
        "resume default must contain a fenced ```bash block with the verdict command; got tail:\n{}",
        &out[out.len().saturating_sub(400)..]
    );
    assert!(
        out.contains("verifier-verdict approve"),
        "resume default must show `verifier-verdict approve`: {out}"
    );
    assert!(
        out.contains("verifier-verdict reject"),
        "resume default must show `verifier-verdict reject`: {out}"
    );
    assert!(
        out.contains("--notes"),
        "resume default verdict command examples must include --notes: {out}"
    );
}
