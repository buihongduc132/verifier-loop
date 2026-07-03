// tasks.md §5 — Verifier spawn orchestration (verifier-spawn spec).
// RED phase: written first, against the spec, BEFORE any implementation.
//
// Scope of THIS test (§5): parallel non-blocking spawn of `m` verifiers
// (`tokio::process::Command` + gather barrier), per-spawn identity env injection
// (`VERIFIER_LOOP_GOAL_ID` / `VERIFIER_LOOP_VERIFIER_ID` / `VERIFIER_LOOP_ROUND`),
// pre-created `rounds/<round>/<verifierId>/verdict.json` `{status:null}` + `meta.json`
// `{sid, turnsUsed}`, per-verifier `verifierTimeoutSec` kill → null verdict, SID capture
// from the ACP `session` event, `final-output.txt` capture from `agent_end`, and the
// gather barrier waiting for ALL verifiers (or timeouts) before returning.
//
// OUT of scope here (deliberately): RESUME-side reuse vs fresh-spawn decisions,
// prior-SID archival, round-increment semantics on resume — those are §6 and are
// covered by `tests/session_reuse.rs`.
//
// Strategy: deterministic, no real `pi` dependency. Each test writes a tiny
// `fake_verifier.sh` to a temp dir and registers it as a CUSTOM adapter (spawn template
// = the script's absolute path, no `{prompt}` placeholder → trivial whitespace split).
// The fake script reads its `VERIFIER_LOOP_*` env, optionally records them to a capture
// file, optionally emits fixed ACP JSON lines on stdout, and optionally sleeps. This
// exercises the real `tokio::process` + `tokio::select!` timeout path end to end.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use verifier_loop::{acp, spawn, store};
use verifier_loop::goal;

/// A blank prompt is fine: the fake adapters carry no `{prompt}` placeholder, so the
/// orchestrator's whitespace split yields just the script path.
const PROMPT: &str = "";

/// Build the test's tokio runtime. Each test gets its own (tests run in parallel).
fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime builds")
}

/// Write `body` to a temp script, chmod 0755, return its absolute path string.
fn write_script(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path.to_string_lossy().into_owned()
}

/// A new goal rooted at `root`, with a config file overriding the given params.
fn seed_goal(root: &Path, goal_text: &str, config: &serde_json::Value) -> String {
    fs::write(root.join("config.json"), config.to_string()).unwrap();
    goal::new(root, goal_text, None).expect("NEW seeds a goal")
}

/// A custom adapter whose spawn template is just the script path.
fn script_adapter(script_path: &str) -> acp::Adapter {
    acp::Adapter::custom(script_path.to_string(), script_path.to_string())
}

/// Returns the verifier dir `goals/<id>/rounds/<round>/<vid>`.
fn verifier_dir(root: &Path, goal_id: &str, round: u32, vid: &str) -> std::path::PathBuf {
    root.join("goals")
        .join(goal_id)
        .join("rounds")
        .join(round.to_string())
        .join(vid)
}

// ---------------------------------------------------------------------------
// §5.3 — pre-create per-verifier verdict.json {status:null} + meta.json
// ---------------------------------------------------------------------------

#[test]
fn spawn_round_creates_per_verifier_dirs_with_null_verdict_and_meta() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 2, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    // Unquoted heredoc (`<<EOF`) so $VERIFIER_LOOP_VERIFIER_ID expands to v1/v2.
    let script = write_script(
        dir.path(),
        "ok.sh",
        r#"#!/bin/sh
cat <<EOF
{"type":"session","id":"sid-$VERIFIER_LOOP_VERIFIER_ID"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"done"}]}],"willRetry":false}
EOF
"#,
    );
    let adapter = script_adapter(&script);

    let runs = rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt: PROMPT,
        adapter: &adapter,
    }))
    .expect("spawn round succeeds");

    assert_eq!(runs.len(), 2, "m=2 spawns exactly two verifier runs");

    for vid in ["v1", "v2"] {
        let vdir = verifier_dir(root, &goal_id, 1, vid);
        let verdict: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(vdir.join("verdict.json")).unwrap()).unwrap();
        assert_eq!(verdict["status"], serde_json::Value::Null, "{vid} verdict null");

        let meta: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
        assert_eq!(meta["sid"], vid.replace('v', "sid-v"), "{vid} sid captured");
        assert!(meta["turnsUsed"].is_number(), "{vid} turnsUsed recorded");
    }
}

// ---------------------------------------------------------------------------
// §5.2 — identity env vars injected per spawn
// ---------------------------------------------------------------------------

#[test]
fn identity_env_vars_are_injected_per_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 2, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    // The script dumps the three identity env vars to a per-verifier capture file.
    let capture_dir = root.join("captures");
    fs::create_dir_all(&capture_dir).unwrap();
    let script = write_script(
        dir.path(),
        "env.sh",
        &format!(
            r#"#!/bin/sh
cat > "{cap}/$VERIFIER_LOOP_VERIFIER_ID.env" <<EOF
$VERIFIER_LOOP_GOAL_ID
$VERIFIER_LOOP_VERIFIER_ID
$VERIFIER_LOOP_ROUND
EOF
cat <<'ACP'
{{"type":"session","id":"s"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
ACP
"#,
            cap = capture_dir.to_string_lossy()
        ),
    );
    let adapter = script_adapter(&script);

    rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt: PROMPT,
        adapter: &adapter,
    }))
    .expect("spawn succeeds");

    for vid in ["v1", "v2"] {
        let cap = fs::read_to_string(capture_dir.join(format!("{vid}.env"))).unwrap();
        let lines: Vec<&str> = cap.trim().lines().collect();
        assert_eq!(lines[0], goal_id, "{vid} goal id injected");
        assert_eq!(lines[1], vid, "{vid} verifier id injected");
        assert_eq!(lines[2], "1", "{vid} round injected");
    }
}

// ---------------------------------------------------------------------------
// §5.1 — parallel non-blocking spawn (no serialization)
// ---------------------------------------------------------------------------

#[test]
fn parallel_spawn_does_not_serialize() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // m=2; each verifier sleeps 300ms. If serialized → ~600ms; parallel → ~300ms.
    let config = serde_json::json!({
        "n": 1, "m": 2, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    let script = write_script(
        dir.path(),
        "slow.sh",
        r#"#!/bin/sh
sleep 0.3
cat <<'EOF'
{"type":"session","id":"s"}
{"type":"agent_end","messages":[],"willRetry":false}
EOF
"#,
    );
    let adapter = script_adapter(&script);

    let start = std::time::Instant::now();
    rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt: PROMPT,
        adapter: &adapter,
    }))
    .expect("spawn succeeds");
    let elapsed = start.elapsed();

    // Parallel upper bound: well below 2x the single sleep. Allow generous slack.
    assert!(
        elapsed < Duration::from_millis(550),
        "spawn was serialized (elapsed={elapsed:?})"
    );
}

// ---------------------------------------------------------------------------
// §5.4 — per-verifier timeout kills process and leaves null verdict
// ---------------------------------------------------------------------------

#[test]
fn timeout_kills_process_and_leaves_null_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // 1s timeout; the verifier sleeps effectively forever.
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 1
    });
    let goal_id = seed_goal(root, "g", &config);

    let script = write_script(
        dir.path(),
        "forever.sh",
        r#"#!/bin/sh
sleep 30
"#,
    );
    let adapter = script_adapter(&script);

    let runs = rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt: PROMPT,
        adapter: &adapter,
    }))
    .expect("spawn round still returns (timeout is not a hard error)");

    assert_eq!(runs.len(), 1);
    assert!(runs[0].timed_out, "the verifier run is marked timed out");
    assert!(runs[0].sid.is_none(), "no SID captured from a timed-out run");

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], serde_json::Value::Null, "null verdict preserved");
}

// ---------------------------------------------------------------------------
// §5 — SID captured from ACP session event; final-output captured from agent_end
// ---------------------------------------------------------------------------

#[test]
fn sid_and_final_output_are_captured() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    let script = write_script(
        dir.path(),
        "acp.sh",
        r#"#!/bin/sh
cat <<'EOF'
{"type":"session","id":"abc-123"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"VERIFY: approve me"}]}],"willRetry":false}
EOF
"#,
    );
    let adapter = script_adapter(&script);

    let runs = rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt: PROMPT,
        adapter: &adapter,
    }))
    .expect("spawn succeeds");

    assert_eq!(runs[0].sid.as_deref(), Some("abc-123"));
    assert_eq!(runs[0].final_output.as_deref(), Some("VERIFY: approve me"));

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let final_txt = fs::read_to_string(vdir.join("final-output.txt")).unwrap();
    assert_eq!(final_txt, "VERIFY: approve me");

    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["sid"], "abc-123", "meta.json carries captured sid");
}

// ---------------------------------------------------------------------------
// §5.5 — gather barrier waits for ALL verifiers
// ---------------------------------------------------------------------------

#[test]
fn gather_barrier_waits_for_all() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 3, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    // Staggered: 100ms, 300ms, 500ms. Gather must return only after the slowest.
    let capture_dir = root.join("done");
    fs::create_dir_all(&capture_dir).unwrap();
    // One script that sleeps a different amount based on verifier id suffix.
    let script = write_script(
        dir.path(),
        "staggered.sh",
        &format!(
            r#"#!/bin/sh
case "$VERIFIER_LOOP_VERIFIER_ID" in
  v1) sleep 0.1 ;;
  v2) sleep 0.3 ;;
  *)  sleep 0.5 ;;
esac
printf '%s' "$VERIFIER_LOOP_VERIFIER_ID" > "{cap}/$VERIFIER_LOOP_VERIFIER_ID"
cat <<'EOF'
{{"type":"session","id":"$VERIFIER_LOOP_VERIFIER_ID"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
"#,
            cap = capture_dir.to_string_lossy()
        ),
    );
    let adapter = script_adapter(&script);

    let runs = rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt: PROMPT,
        adapter: &adapter,
    }))
    .expect("spawn succeeds");

    assert_eq!(runs.len(), 3, "gather returned all three runs");
    // Every verifier reached its done marker → barrier truly waited for all.
    for vid in ["v1", "v2", "v3"] {
        assert!(
            capture_dir.join(vid).exists(),
            "{vid} completed before gather returned"
        );
    }
}
