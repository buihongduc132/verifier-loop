// tasks.md §2 (D2) — Policy dedup (override semantics).
// RED phase: written first, against the spec, BEFORE any implementation.
//
// verifier-prompt ADDED "Policy is not duplicated when custom verifierPromptFile is set":
//   * When verifierPromptFile is set, the rendered prompt SHALL include the custom policy
//     file content exactly once and SHALL NOT also embed the built-in VERIFIER_POLICY
//     constant. The two policy sources are mutually exclusive (override semantics).
//   * When verifierPromptFile is null, the built-in policy is used exactly once.
//
// API targets for the GREEN author (documented here so the tests pin the contract):
//   * pub const DEFAULT_TEMPLATE_NO_POLICY: &str
//       (identity preamble + body file, NO VERIFIER_POLICY concat block)
//   * pub const DEFAULT_RESUME_TEMPLATE_NO_POLICY: &str
//   * pub fn default_template_no_policy() -> &'static str
//   * pub fn default_resume_template_no_policy() -> &'static str
//
// The bin picks the no-policy template variant when a custom verifierPromptFile is
// present, then prepends the custom file. These tests exercise that path via the public
// prompt API directly (the same primitives the bin composes).
//
// Every test below FAILS today: the no-policy consts/getters do not exist (compile
// error), and even if they did, prepend_custom still leaves the built-in policy in place
// when the default template is used.

use verifier_loop::prompt;

fn vars_default() -> prompt::PromptVars<'static> {
    prompt::PromptVars {
        goal_id: "goal-d2",
        verifier_id: "v1",
        round: 1,
        prev_round: None,
        goal_text: "build the thing",
        context: Some("optional context"),
        fix_notes: None,
        prev_notes: None,
        cwd: "/repo",
        git_status: " M src/lib.rs\n",
        file_edit_times: "src/lib.rs:1234567\n",
        git_diff: "diff --git a/src/lib.rs b/src/lib.rs\n+fn new() {}\n",
        git_diff_max_chars: 10_000,
        truncated: false,
    }
}

/// The custom verifierPromptFile body used in the override tests. Deliberately does NOT
/// contain the canonical policy marker `<_unfold.md>` nor the built-in heading, so any
/// occurrence of those in the rendered output MUST come from the built-in VERIFIER_POLICY
/// (which D2 says must be absent when the custom file is set).
const CUSTOM_POLICY_BODY: &str =
    "CUSTOM OPERATING RULES (overrides built-in policy)\nBe rigorous.\n";

// ===========================================================================
// §2.1 RED: custom verifierPromptFile set -> built-in VERIFIER_POLICY ABSENT.
// Today: default_template_no_policy() does not exist -> compile error.
// ===========================================================================

#[test]
fn custom_file_overrides_built_in_policy() {
    let vars = vars_default();
    // GREEN target: a no-policy template variant + prepend_custom composes the override
    // path the bin will use. The built-in policy block MUST be absent.
    let body = prompt::render(Some(prompt::default_template_no_policy()), &vars).unwrap();
    let rendered = prompt::prepend_custom(body, Some(CUSTOM_POLICY_BODY));

    // The built-in canonical policy marker must NOT appear (it lives ONLY inside
    // VERIFIER_POLICY, which D2 says to skip when a custom file is set).
    assert!(
        !rendered.contains("<_unfold.md>"),
        "custom-file override path must NOT embed the built-in VERIFIER_POLICY marker `<_unfold.md>`: found in rendered output"
    );
    // The built-in policy heading must NOT appear (the custom file replaces it).
    assert!(
        !rendered.contains("# Verifier Detective Policy (canonical, from verifier-loop skill)"),
        "custom-file override path must NOT embed the built-in policy heading: found in rendered output"
    );
    // And the custom file's own heading IS present (override, not omission).
    assert!(
        rendered.contains("CUSTOM OPERATING RULES (overrides built-in policy)"),
        "custom verifierPromptFile content must be present in the override path: {rendered}"
    );
}

// ===========================================================================
// §2.2 RED: null verifierPromptFile -> built-in policy EXACTLY ONCE.
// Today: passes structurally (render(None) embeds policy once), but kept as a guard so
// the override refactor cannot accidentally strip the policy from the null path.
// ===========================================================================

#[test]
fn null_file_uses_built_in_policy_exactly_once() {
    let vars = vars_default();
    let rendered = prompt::render(None, &vars).unwrap();

    let heading_count = rendered
        .matches("# Verifier Detective Policy (canonical, from verifier-loop skill)")
        .count();
    assert_eq!(
        heading_count, 1,
        "null verifierPromptFile must embed the built-in policy heading EXACTLY once (got {heading_count})"
    );

    let marker_count = rendered.matches("<_unfold.md>").count();
    assert_eq!(
        marker_count, 1,
        "null verifierPromptFile must embed the built-in policy marker `<_unfold.md>` exactly once (got {marker_count})"
    );
}

// ===========================================================================
// §2.3 RED: custom file content IS present (override, not omission).
// ===========================================================================

#[test]
fn custom_file_content_present() {
    let vars = vars_default();
    let body = prompt::render(Some(prompt::default_template_no_policy()), &vars).unwrap();
    let rendered = prompt::prepend_custom(body, Some(CUSTOM_POLICY_BODY));

    // The custom file body appears at the top (before the separator).
    assert!(
        rendered.starts_with(CUSTOM_POLICY_BODY),
        "custom verifierPromptFile content must lead the rendered prompt (override semantics): {rendered}"
    );
    // The goal body still renders (the no-policy template keeps the body file).
    assert!(
        rendered.contains("build the thing"),
        "no-policy template variant must still render {{goalText}}: {rendered}"
    );
    // The identity preamble is retained (the no-policy variant keeps "You are verifier...").
    assert!(
        rendered.contains("You are verifier"),
        "no-policy template variant must keep the identity preamble: {rendered}"
    );
}

// ===========================================================================
// §2.4 RED: resume override path — custom file overrides built-in policy on RESUME.
// ===========================================================================

#[test]
fn custom_file_overrides_built_in_policy_on_resume() {
    let vars = prompt::PromptVars {
        fix_notes: Some("fixed the off-by-one"),
        prev_notes: Some("my prior notes"),
        ..vars_default()
    };
    let body =
        prompt::render_resume(Some(prompt::default_resume_template_no_policy()), &vars).unwrap();
    let rendered = prompt::prepend_custom(body, Some(CUSTOM_POLICY_BODY));

    assert!(
        !rendered.contains("<_unfold.md>"),
        "resume override path must NOT embed the built-in VERIFIER_POLICY marker: found in rendered output"
    );
    assert!(
        !rendered.contains("# Verifier Detective Policy (canonical, from verifier-loop skill)"),
        "resume override path must NOT embed the built-in policy heading: found in rendered output"
    );
    assert!(
        rendered.contains("CUSTOM OPERATING RULES (overrides built-in policy)"),
        "resume override path must include the custom file content: {rendered}"
    );
    // Resume-specific vars still render.
    assert!(
        rendered.contains("fixed the off-by-one"),
        "resume no-policy variant must still render {{fixNotes}}: {rendered}"
    );
}
