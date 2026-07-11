// tasks.md §5, §7, §8 (D5, D6) — Verdict enforcement + compaction detection + recovery.
// RED phase: written first, against the spec, BEFORE any implementation.
//
// Covers:
//   * verifier-spawn ADDED: "Verdict is enforced after child exit" (D5) — gather detects
//     no-verdict exits, re-prompts the same sid (resume) with a minimal nudge up to
//     maxTurn - turnsUsed times, records nudgeAttempts in meta.json.
//   * compaction-recovery ADDED: compaction event detected (D6) — parser recognises
//     `{"type":"compaction",...}`, orchestrator records compactionObserved in meta.json,
//     auto-resumes ONCE per slot per round after a compaction+exit-without-verdict to
//     harvest the verdict (recoveryAttempts), fail-closed if recovery also fails.
//   * recovery nudge is minimal (<2KB, no goal/diff/policy re-embedded).
//
// API targets for the GREEN author (documented here so the tests pin the contract):
//   * AcpEvent::Compaction { tokens_before: Option<u64>, tokens_after: Option<u64> }
//       — new parser variant for `{"type":"compaction",...}`.
//   * pub fn acp::extract_compaction_observed(stream: &str) -> bool
//   * VerifierMeta gains:
//       nudge_attempts: u32     (serde "nudgeAttempts", default 0)
//       compaction_observed: bool (serde "compactionObserved", default false)
//       recovery_attempts: u32 (serde "recoveryAttempts", default 0)
//   * spawn::spawn_round already drives gather; the GREEN author adds the verdict-
//     enforcement nudge loop + compaction-recovery branch INSIDE gather, using the
//     adapter's resume command + the same env injection. meta.json is updated with the
//     new counts.
//
// Every test below FAILS today: parser-variant/fn tests are compile errors; orchestrator
// tests are assertion failures (the new meta.json fields are absent today, and no
// nudge/recovery resume occurs).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use verifier_loop::{acp, goal, spawn, store};

// ---------------------------------------------------------------------------
// helpers (mirrors tests/spawn_orchestrator.rs patterns)
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime builds")
}

fn write_script(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path.to_string_lossy().into_owned()
}

fn seed_goal(root: &Path, goal_text: &str, config: &serde_json::Value) -> String {
    fs::write(root.join("config.json"), config.to_string()).unwrap();
    goal::new(root, goal_text, None).expect("NEW seeds a goal")
}

fn script_adapter(script_path: &str) -> acp::Adapter {
    acp::Adapter::custom(script_path.to_string(), script_path.to_string())
}

fn verifier_dir(root: &Path, goal_id: &str, round: u32, vid: &str) -> PathBuf {
    root.join("goals")
        .join(goal_id)
        .join("rounds")
        .join(round.to_string())
        .join(vid)
}

fn spawn_input<'a>(
    root: &'a Path,
    goal_id: &'a str,
    config: &'a store::Config,
    adapter: &'a acp::Adapter,
) -> spawn::SpawnInput<'a> {
    spawn::SpawnInput {
        root,
        goal_id,
        round: 1,
        config,
        prompt: "INITIAL PROMPT BODY",
        adapter,
    }
}

/// Returns a shell snippet that writes a minimal APPROVE verdict directly into the
/// verifier slot (bypasses the verdict CLI, which is not resolvable as a sibling of the
/// test binary). The orchestrator's gather reads verdict.json to decide whether to
/// nudge/recover, so this is sufficient to exercise the control flow. The slot path is
/// built from the env vars the orchestrator injects.
fn write_verdict_snippet() -> String {
    r#"
SLOT="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
mkdir -p "$SLOT"
printf '%s\n' '{"status":"APPROVE","registeredAt":"2026-07-11T00:00:00Z"}' > "$SLOT/verdict.json"
"#
    .to_string()
}

// ===========================================================================
// Group 7 (D6) — parser detects compaction events
// ===========================================================================

// ---------------------------------------------------------------------------
// §7.1 RED: parse_event recognises {"type":"compaction",...}.
// Today AcpEvent has no Compaction variant → compile error.
// ---------------------------------------------------------------------------

#[test]
fn parser_detects_compaction_event() {
    let ev = acp::parse_event(r#"{"type":"compaction","tokensBefore":255106}"#)
        .expect("compaction event parses")
        .expect("compaction event yields Some(event)");
    match ev {
        acp::AcpEvent::Compaction {
            tokens_before,
            tokens_after,
        } => {
            assert_eq!(tokens_before, Some(255106), "tokensBefore captured");
            assert_eq!(tokens_after, None, "tokensAfter absent → None");
        }
        other => panic!("expected AcpEvent::Compaction, got {other:?}"),
    }
}

#[test]
fn parser_detects_compaction_event_with_after() {
    let ev = acp::parse_event(
        r#"{"type":"compaction","tokensBefore":255106,"tokensAfter":32000}"#,
    )
    .unwrap()
    .unwrap();
    match ev {
        acp::AcpEvent::Compaction {
            tokens_before,
            tokens_after,
        } => {
            assert_eq!(tokens_before, Some(255106));
            assert_eq!(tokens_after, Some(32000));
        }
        other => panic!("expected Compaction, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// §7.2/§7.3 RED: extract_compaction_observed returns true/false.
// Today the fn does not exist → compile error.
// ---------------------------------------------------------------------------

#[test]
fn extract_compaction_observed_returns_true_when_present() {
    let stream = "{\"type\":\"session\",\"id\":\"s1\"}\n\
                  {\"type\":\"compaction\",\"tokensBefore\":255106}\n\
                  {\"type\":\"agent_end\",\"messages\":[],\"willRetry\":false}\n";
    assert!(
        acp::extract_compaction_observed(stream),
        "stream with a compaction event must report true"
    );
}

#[test]
fn no_compaction_returns_false() {
    let stream = "{\"type\":\"session\",\"id\":\"s1\"}\n\
                  {\"type\":\"agent_end\",\"messages\":[],\"willRetry\":false}\n";
    assert!(
        !acp::extract_compaction_observed(stream),
        "stream with no compaction event must report false"
    );
}

// ===========================================================================
// Group 5 (D5) — verdict enforcement: gather nudges when no verdict + turns remain
// ===========================================================================

// ---------------------------------------------------------------------------
// §5.1 RED: missing verdict + turns remain → orchestrator nudges (resume) and the
// resumed session writes the verdict. meta.json records nudgeAttempts >= 1.
//
// Today gather does NOT nudge: the stub is invoked exactly once, never writes a verdict,
// and meta.json carries no nudgeAttempts key → both assertions FAIL.
//
// The stub counts invocations; on the 2nd invocation (the nudge resume) it writes an
// APPROVE verdict into the slot.
// ---------------------------------------------------------------------------

#[test]
fn gather_nudges_when_no_verdict_and_turns_remain() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let capture_dir = root.join("cap");
    fs::create_dir_all(&capture_dir).unwrap();

    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    let goal_id = seed_goal(root, "goal text here", &config);

    let script = write_script(
        dir.path(),
        "nudge.sh",
        &format!(
            r#"#!/bin/sh
COUNT_FILE="{cap}/$VERIFIER_LOOP_VERIFIER_ID.count"
COUNT=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNT_FILE"

cat <<EOF
{{"type":"session","id":"sid-$VERIFIER_LOOP_VERIFIER_ID"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF

if [ "$COUNT" -ge 2 ]; then
{verdict_snippet}
fi
"#,
            cap = capture_dir.to_string_lossy(),
            verdict_snippet = write_verdict_snippet(),
        ),
    );
    let adapter = script_adapter(&script);
    let cfg = store::Config::load_in(root).unwrap();

    let runs = rt()
        .block_on(spawn::spawn_round(spawn_input(root, &goal_id, &cfg, &adapter)))
        .expect("spawn succeeds");
    assert_eq!(runs.len(), 1);

    // The stub must have been invoked at least twice (initial + nudge resume).
    let count: u32 = fs::read_to_string(capture_dir.join("v1.count"))
        .unwrap()
        .trim()
        .parse()
        .unwrap_or(0);
    assert!(
        count >= 2,
        "orchestrator must nudge (resume) when no verdict is present and turns remain; invocation count = {count}"
    );

    // meta.json records the nudge attempt.
    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(
        meta["nudgeAttempts"].as_u64(),
        Some(1),
        "meta.json must record nudgeAttempts >= 1 after a nudge round; got: {meta}"
    );

    // The nudge harvested a verdict.
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["status"], "APPROVE",
        "verdict harvested after nudge; got: {verdict}"
    );
}

// ---------------------------------------------------------------------------
// §5.3 RED: a slot with a non-null verdict is NOT nudged. nudgeAttempts == 0.
// Today: no nudge mechanism, nudgeAttempts absent → as_u64() is None ≠ Some(0) → FAIL.
// ---------------------------------------------------------------------------

#[test]
fn gather_does_not_nudge_when_verdict_present() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    let goal_id = seed_goal(root, "goal", &config);

    // The stub writes a verdict on the FIRST invocation.
    let script = write_script(
        dir.path(),
        "approve.sh",
        &format!(
            r#"#!/bin/sh
cat <<EOF
{{"type":"session","id":"sid"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
{verdict_snippet}
"#,
            verdict_snippet = write_verdict_snippet(),
        ),
    );
    let adapter = script_adapter(&script);
    let cfg = store::Config::load_in(root).unwrap();

    rt().block_on(spawn::spawn_round(spawn_input(root, &goal_id, &cfg, &adapter)))
        .expect("spawn succeeds");

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(
        meta["nudgeAttempts"].as_u64(),
        Some(0),
        "a slot with a non-null verdict must NOT be nudged (nudgeAttempts == 0); got: {meta}"
    );
}

// ---------------------------------------------------------------------------
// §5.2 RED: nudge loop respects maxTurn ceiling — when turns are immediately exhausted
// (maxTurn=1, turnsUsed becomes 1 after the initial spawn), no nudge is issued and the
// verdict stays null.
// Today: no nudge mechanism, nudgeAttempts absent → FAIL.
// ---------------------------------------------------------------------------

#[test]
fn gather_nudge_exhausted_leaves_null() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 1, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    let goal_id = seed_goal(root, "goal", &config);

    // The stub never writes a verdict.
    let script = write_script(
        dir.path(),
        "never.sh",
        r#"#!/bin/sh
cat <<EOF
{"type":"session","id":"sid"}
{"type":"agent_end","messages":[],"willRetry":false}
EOF
"#,
    );
    let adapter = script_adapter(&script);
    let cfg = store::Config::load_in(root).unwrap();

    rt().block_on(spawn::spawn_round(spawn_input(root, &goal_id, &cfg, &adapter)))
        .expect("spawn succeeds");

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(
        meta["nudgeAttempts"].as_u64(),
        Some(0),
        "maxTurn=1 → turns immediately exhausted → no nudge (nudgeAttempts == 0); got: {meta}"
    );
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["status"],
        serde_json::Value::Null,
        "verdict stays null when nudge budget exhausted; got: {verdict}"
    );
}

// ---------------------------------------------------------------------------
// §5.5 RED: nudge attempts are recorded in meta.json as nudgeAttempts.
// (Covered structurally by gather_nudges_when_no_verdict_and_turns_remain; this test
// pins the field name + numeric type explicitly.)
// Today: field absent → FAIL.
// ---------------------------------------------------------------------------

#[test]
fn meta_records_nudge_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let capture_dir = root.join("cap");
    fs::create_dir_all(&capture_dir).unwrap();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    let goal_id = seed_goal(root, "goal", &config);

    let script = write_script(
        dir.path(),
        "nudge2.sh",
        &format!(
            r#"#!/bin/sh
COUNT_FILE="{cap}/v1.count"
COUNT=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNT_FILE"
cat <<EOF
{{"type":"session","id":"sid"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
if [ "$COUNT" -ge 2 ]; then
{verdict_snippet}
fi
"#,
            cap = capture_dir.to_string_lossy(),
            verdict_snippet = write_verdict_snippet(),
        ),
    );
    let adapter = script_adapter(&script);
    let cfg = store::Config::load_in(root).unwrap();

    rt().block_on(spawn::spawn_round(spawn_input(root, &goal_id, &cfg, &adapter)))
        .expect("spawn succeeds");

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
    let n = meta["nudgeAttempts"].as_u64();
    assert!(
        n.is_some() && n.unwrap() >= 1,
        "meta.json must contain a numeric nudgeAttempts >= 1; got: {meta}"
    );
}

// ===========================================================================
// Group 8 (D6) — compaction recovery: auto-resume post-compaction
// ===========================================================================

// ---------------------------------------------------------------------------
// §8.1 RED: compaction then exit (no agent_end, no verdict) → exactly ONE recovery
// resume on the same sid. meta.json records compactionObserved + recoveryAttempts == 1.
//
// The stub: 1st invocation emits session + compaction then exits (no agent_end); 2nd
// invocation (the recovery resume) emits agent_end + writes a verdict.
//
// Today: no recovery → invocation count == 1, recoveryAttempts absent → FAIL.
// ---------------------------------------------------------------------------

#[test]
fn compaction_then_exit_triggers_one_recovery_resume() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let capture_dir = root.join("cap");
    fs::create_dir_all(&capture_dir).unwrap();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    let goal_id = seed_goal(root, "goal", &config);

    let script = write_script(
        dir.path(),
        "compact.sh",
        &format!(
            r#"#!/bin/sh
COUNT_FILE="{cap}/v1.count"
COUNT=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNT_FILE"

if [ "$COUNT" -eq 1 ]; then
  cat <<EOF
{{"type":"session","id":"sid"}}
{{"type":"compaction","tokensBefore":255106}}
EOF
  exit 0
fi
cat <<EOF
{{"type":"session","id":"sid"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
{verdict_snippet}
"#,
            cap = capture_dir.to_string_lossy(),
            verdict_snippet = write_verdict_snippet(),
        ),
    );
    let adapter = script_adapter(&script);
    let cfg = store::Config::load_in(root).unwrap();

    rt().block_on(spawn::spawn_round(spawn_input(root, &goal_id, &cfg, &adapter)))
        .expect("spawn succeeds");

    let count: u32 = fs::read_to_string(capture_dir.join("v1.count"))
        .unwrap()
        .trim()
        .parse()
        .unwrap_or(0);
    assert_eq!(
        count, 2,
        "exactly one recovery resume after compaction+exit (initial + recovery = 2 invocations); got {count}"
    );

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(
        meta["compactionObserved"].as_bool(),
        Some(true),
        "meta.json must record compactionObserved == true; got: {meta}"
    );
    assert_eq!(
        meta["recoveryAttempts"].as_u64(),
        Some(1),
        "meta.json must record recoveryAttempts == 1; got: {meta}"
    );

    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["status"], "APPROVE",
        "verdict harvested after recovery resume; got: {verdict}"
    );
}

// ---------------------------------------------------------------------------
// §8.2 RED: the recovery resume nudge prompt is minimal (<2KB) and does NOT re-embed the
// goal/diff/policy. The stub captures the stdin of the recovery resume.
//
// Today: no recovery resume occurs → no stdin capture file → assertion FAILs (missing file).
// ---------------------------------------------------------------------------

#[test]
fn recovery_resume_uses_minimal_nudge_under_2kb() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let capture_dir = root.join("cap");
    fs::create_dir_all(&capture_dir).unwrap();
    let goal_text = "UNIQUE_GOAL_MARKER_FOR_NUDGE_TEST";
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    let goal_id = seed_goal(root, goal_text, &config);

    let script = write_script(
        dir.path(),
        "compact_capture.sh",
        &format!(
            r#"#!/bin/sh
COUNT_FILE="{cap}/v1.count"
COUNT=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNT_FILE"

if [ "$COUNT" -eq 1 ]; then
  cat <<EOF
{{"type":"session","id":"sid"}}
{{"type":"compaction","tokensBefore":255106}}
EOF
  exit 0
fi
# Recovery resume: capture the nudge prompt piped on stdin.
cat > "{cap}/v1.recovery-stdin"
cat <<EOF
{{"type":"session","id":"sid"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
{verdict_snippet}
"#,
            cap = capture_dir.to_string_lossy(),
            verdict_snippet = write_verdict_snippet(),
        ),
    );
    let adapter = script_adapter(&script);
    let cfg = store::Config::load_in(root).unwrap();

    rt().block_on(spawn::spawn_round(spawn_input(root, &goal_id, &cfg, &adapter)))
        .expect("spawn succeeds");

    let stdin_path = capture_dir.join("v1.recovery-stdin");
    assert!(
        stdin_path.exists(),
        "recovery resume must occur and its stdin must be captured"
    );
    let nudge = fs::read(&stdin_path).unwrap();
    assert!(
        nudge.len() < 2048,
        "recovery nudge must be < 2KB (target), got {} bytes",
        nudge.len()
    );
    let nudge_text = String::from_utf8_lossy(&nudge);
    assert!(
        !nudge_text.contains(goal_text),
        "recovery nudge must NOT re-embed the goal text; got: {nudge_text}"
    );
}

// ---------------------------------------------------------------------------
// §8.3 RED: a second compaction+exit after recovery leaves the slot null (no infinite
// loop). recoveryAttempts == 1, verdict null.
//
// The stub compacts+exits on EVERY invocation. Today: no recovery → count == 1,
// recoveryAttempts absent → FAIL.
// ---------------------------------------------------------------------------

#[test]
fn second_compaction_after_recovery_leaves_null() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let capture_dir = root.join("cap");
    fs::create_dir_all(&capture_dir).unwrap();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    let goal_id = seed_goal(root, "goal", &config);

    // ALWAYS compact + exit (no agent_end, no verdict) — on every invocation.
    let script = write_script(
        dir.path(),
        "compact2.sh",
        &format!(
            r#"#!/bin/sh
COUNT_FILE="{cap}/v1.count"
COUNT=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNT_FILE"
cat <<EOF
{{"type":"session","id":"sid"}}
{{"type":"compaction","tokensBefore":255106}}
EOF
exit 0
"#,
            cap = capture_dir.to_string_lossy(),
        ),
    );
    let adapter = script_adapter(&script);
    let cfg = store::Config::load_in(root).unwrap();

    rt().block_on(spawn::spawn_round(spawn_input(root, &goal_id, &cfg, &adapter)))
        .expect("spawn succeeds");

    let count: u32 = fs::read_to_string(capture_dir.join("v1.count"))
        .unwrap()
        .trim()
        .parse()
        .unwrap_or(0);
    assert_eq!(
        count, 2,
        "exactly one recovery resume attempt (initial + 1 recovery = 2); no infinite loop; got {count}"
    );

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(
        meta["recoveryAttempts"].as_u64(),
        Some(1),
        "recoveryAttempts pinned at 1 after a single recovery try; got: {meta}"
    );
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["status"],
        serde_json::Value::Null,
        "slot fails closed to null when recovery also compacts+exits; got: {verdict}"
    );
}

// ---------------------------------------------------------------------------
// §8.4 RED: compaction followed by a successful agent_end does NOT trigger recovery
// (the session self-recovered). recoveryAttempts == 0.
//
// Today: recoveryAttempts absent → FAIL.
// ---------------------------------------------------------------------------

#[test]
fn compaction_then_successful_agent_end_no_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    let goal_id = seed_goal(root, "goal", &config);

    // Emit compaction then CONTINUE to agent_end (self-recovered).
    let script = write_script(
        dir.path(),
        "self_recover.sh",
        r#"#!/bin/sh
cat <<EOF
{"type":"session","id":"sid"}
{"type":"compaction","tokensBefore":255106}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"done"}]}],"willRetry":false}
EOF
"#,
    );
    let adapter = script_adapter(&script);
    let cfg = store::Config::load_in(root).unwrap();

    rt().block_on(spawn::spawn_round(spawn_input(root, &goal_id, &cfg, &adapter)))
        .expect("spawn succeeds");

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(
        meta["compactionObserved"].as_bool(),
        Some(true),
        "compaction was observed even though the session self-recovered; got: {meta}"
    );
    assert_eq!(
        meta["recoveryAttempts"].as_u64(),
        Some(0),
        "no recovery resume when agent_end follows compaction; got: {meta}"
    );
}

// ===========================================================================
// fix-secret: nudge/recovery resume can harvest a SIGNED verdict via the REAL
// verifier-verdict binary. This is the regression for the bug where spawn_nudge_child
// injected an EMPTY secret (mint failed on the already-pinned slot) so every nudge-
// harvested verdict failed consensus signature verification.
//
// The stub calls $VERIFIER_LOOP_VERDICT_BIN (resolved by the test via assert_cmd's
// cargo_bin) on its 2nd invocation (the nudge resume). The orchestrator persists the
// signing secret to verifier-secret.hex at initial-spawn mint time, and spawn_nudge_child
// reads it back to inject into the resume child — so the signed verdict verifies.
// ===========================================================================

/// Resolve the real verifier-verdict binary built by `cargo test`. The orchestrator's
/// `sibling_verifier_verdict()` returns None during tests (the test exe lives in
/// target/debug/deps/, not next to verifier-verdict), but the child INHERITS the
/// parent env, so setting VERIFIER_LOOP_VERDICT_BIN here makes $VERIFIER_LOOP_VERDICT_BIN
/// resolve inside the stub.
fn real_verifier_verdict_bin() -> PathBuf {
    let prog = assert_cmd::cargo::cargo_bin("verifier-verdict");
    // Sanity: it must exist on disk.
    assert!(prog.is_file(), "verifier-verdict binary must exist at {prog:?}");
    prog
}

#[test]
fn nudge_resume_can_register_signed_verdict() {
    let verdict_bin = real_verifier_verdict_bin();
    // Propagate to the spawned children (child inherits parent env when the
    // orchestrator doesn't explicitly override VERIFIER_LOOP_VERDICT_BIN).
    std::env::set_var(spawn::ENV_VERDICT_BIN, &verdict_bin);

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let capture_dir = root.join("cap");
    fs::create_dir_all(&capture_dir).unwrap();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    let goal_id = seed_goal(root, "goal", &config);

    // The stub emits session + agent_end, and registers a verdict via the REAL
    // verifier-verdict binary ONLY on the 2nd invocation (the nudge resume). The
    // verdict CLI reads VERIFIER_LOOP_VERIFIER_SECRET (injected by the orchestrator
    // from verifier-secret.hex) and signs the verdict against the pinned pubkey.
    let script = write_script(
        dir.path(),
        "signed_nudge.sh",
        &format!(
            r#"#!/bin/sh
COUNT_FILE="{cap}/v1.count"
COUNT=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNT_FILE"
cat <<EOF
{{"type":"session","id":"sid"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
if [ "$COUNT" -ge 2 ]; then
  "$VERIFIER_LOOP_VERDICT_BIN" approve --notes "nudge-harvested signed verdict" 2>"{cap}/v1.verdict-stderr.log" || echo "verdict-rc=$?" > "{cap}/v1.verdict-rc"
fi
"#,
            cap = capture_dir.to_string_lossy(),
        ),
    );
    let adapter = script_adapter(&script);
    let cfg = store::Config::load_in(root).unwrap();

    rt().block_on(spawn::spawn_round(spawn_input(root, &goal_id, &cfg, &adapter)))
        .expect("spawn succeeds");

    std::env::remove_var(spawn::ENV_VERDICT_BIN);

    let vdir = verifier_dir(root, &goal_id, 1, "v1");

    // (d) The secret hex file must exist with mode 0600.
    use std::os::unix::fs::PermissionsExt;
    let secret_file = vdir.join(verifier_loop::verdict::SECRET_FILE);
    assert!(
        secret_file.exists(),
        "verifier-secret.hex must be persisted at initial spawn; got dir: {}",
        fs::read_dir(&vdir).unwrap().map(|e| e.unwrap().path().to_string_lossy().to_string()).collect::<Vec<_>>().join(", ")
    );
    let mode = fs::metadata(&secret_file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "secret file mode must be 0600, got {:o}", mode);

    // (c) meta.json must record at least one nudge attempt.
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
    let nudge = meta["nudgeAttempts"].as_u64().unwrap_or(0);
    assert!(
        nudge >= 1,
        "nudge must have been issued to harvest the verdict; nudgeAttempts = {nudge}; meta = {meta}"
    );

    // Surface any verdict-CLI stderr failure for diagnosis.
    let verdict_stderr_path = capture_dir.join("v1.verdict-stderr.log");
    if verdict_stderr_path.exists() {
        let stderr = fs::read_to_string(&verdict_stderr_path).unwrap_or_default();
        if stderr.contains("unauthenticated") || stderr.contains("error") {
            panic!("verifier-verdict CLI failed during nudge resume: {stderr}");
        }
    }

    // (a) + (b) The slot's verdict.json must carry a SIGNED APPROVE.
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["status"].as_str(),
        Some("APPROVE"),
        "nudge must have harvested an APPROVE verdict; got: {verdict}"
    );
    let sig = verdict["signature"].as_str();
    assert!(
        sig.is_some() && !sig.unwrap().is_empty(),
        "harvested verdict must carry a non-empty signature (signed against the pinned pubkey); got: {verdict}"
    );
    assert!(
        verdict["pubkeyId"].as_str().is_some(),
        "signed verdict must carry a pubkeyId; got: {verdict}"
    );
}
