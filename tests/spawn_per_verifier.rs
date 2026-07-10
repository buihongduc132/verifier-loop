// RED tests for per-verifier-adapter task group 3: Spawn Layer Integration.
//
// These tests exercise the INTENDED API where SpawnInput carries
// `adapters: &[acp::Adapter]` (one per verifier slot) instead of a single
// `adapter: &acp::Adapter`. They MUST fail to compile until tasks 3.1–3.4
// are implemented:
//   3.1  SpawnInput struct updated to `adapters: &[acp::Adapter]`
//   3.2  spawn_round uses `input.adapters[i]`
//   3.3  spawn_resume uses per-verifier adapter
//   3.4  build_spawn_command / build_resume_command use per-verifier adapter
//
// After implementation these tests should pass, proving that each verifier
// slot uses its own adapter.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use verifier_loop::{acp, goal, spawn, store};

const PROMPT: &str = "";

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

/// Script that emits ACP session + agent_end so the gather barrier completes.
/// The session ID embeds `$VERIFIER_LOOP_VERIFIER_ID` so we can distinguish
/// which adapter each verifier used.
fn acp_ok_script_body() -> &'static str {
    r#"#!/bin/sh
cat <<EOF
{"type":"session","id":"sid-$VERIFIER_LOOP_VERIFIER_ID"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"done"}]}],"willRetry":false}
EOF
"#
}

// ===========================================================================
// Test: spawn_round uses per-verifier adapters (adapters field)
// ===========================================================================
// This test fails at compile time because SpawnInput has `adapter` not `adapters`.

#[test]
fn spawn_round_uses_per_verifier_adapters_field() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 2, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    let script = write_script(dir.path(), "ok.sh", acp_ok_script_body());

    // Two DIFFERENT adapters — the key assertion is that SpawnInput accepts
    // a slice of adapters rather than a single one.
    let adapter1 = script_adapter(&script);
    let adapter2 = script_adapter(&script);
    let adapters = vec![adapter1.clone(), adapter2.clone()];

    let runs = rt()
        .block_on(spawn::spawn_round(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 1,
            config: &store::Config::load_in(root).unwrap(),
            prompt: PROMPT,
            adapters: &adapters, // <-- NEW field name, will fail to compile
        }))
        .expect("spawn round succeeds");

    assert_eq!(runs.len(), 2, "m=2 spawns exactly two verifier runs");
    assert_eq!(runs[0].verifier_id, "v1");
    assert_eq!(runs[1].verifier_id, "v2");
}

// ===========================================================================
// Test: spawn_round with m=3 uses the adapter at each index
// ===========================================================================

#[test]
fn spawn_round_with_three_verifiers_uses_each_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 3, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    let script = write_script(dir.path(), "ok.sh", acp_ok_script_body());

    // Three adapters, one per verifier slot
    let adapters: Vec<acp::Adapter> = (0..3).map(|_| script_adapter(&script)).collect();

    let runs = rt()
        .block_on(spawn::spawn_round(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 1,
            config: &store::Config::load_in(root).unwrap(),
            prompt: PROMPT,
            adapters: &adapters,
        }))
        .expect("spawn round succeeds");

    assert_eq!(runs.len(), 3, "m=3 spawns exactly three verifier runs");
}

// ===========================================================================
// Test: spawn_round uses DIFFERENT adapters per verifier (mixed backends)
// ===========================================================================
// Each verifier script writes a marker so we can prove which adapter ran.
// This validates task 3.5: integration test with mixed adapters.

#[test]
fn spawn_round_mixed_adapters_each_verifier_uses_own() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let capture_dir = root.join("capture");
    fs::create_dir_all(&capture_dir).unwrap();

    let config = serde_json::json!({
        "n": 1, "m": 2, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    // Script for v1 — marks itself as "alpha"
    let script_alpha = write_script(
        dir.path(),
        "alpha.sh",
        &format!(
            r#"#!/bin/sh
echo "alpha" > "{cap}/$VERIFIER_LOOP_VERIFIER_ID.marker"
cat <<'EOF'
{{"type":"session","id":"alpha-$VERIFIER_LOOP_VERIFIER_ID"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
"#,
            cap = capture_dir.to_string_lossy()
        ),
    );

    // Script for v2 — marks itself as "beta"
    let script_beta = write_script(
        dir.path(),
        "beta.sh",
        &format!(
            r#"#!/bin/sh
echo "beta" > "{cap}/$VERIFIER_LOOP_VERIFIER_ID.marker"
cat <<'EOF'
{{"type":"session","id":"beta-$VERIFIER_LOOP_VERIFIER_ID"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
"#,
            cap = capture_dir.to_string_lossy()
        ),
    );

    // Two DIFFERENT adapters — v1 uses alpha, v2 uses beta
    let adapters = vec![script_adapter(&script_alpha), script_adapter(&script_beta)];

    let runs = rt()
        .block_on(spawn::spawn_round(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 1,
            config: &store::Config::load_in(root).unwrap(),
            prompt: PROMPT,
            adapters: &adapters,
        }))
        .expect("spawn round succeeds");

    assert_eq!(runs.len(), 2);

    // Each verifier ran its OWN adapter (not the same one)
    let v1_marker = fs::read_to_string(capture_dir.join("v1.marker")).unwrap();
    let v2_marker = fs::read_to_string(capture_dir.join("v2.marker")).unwrap();
    assert_eq!(v1_marker.trim(), "alpha", "v1 used the alpha adapter");
    assert_eq!(v2_marker.trim(), "beta", "v2 used the beta adapter");

    // SIDs should reflect per-verifier adapter
    assert!(
        runs[0].sid.as_deref().unwrap_or("").starts_with("alpha-"),
        "v1 SID from alpha adapter"
    );
    assert!(
        runs[1].sid.as_deref().unwrap_or("").starts_with("beta-"),
        "v2 SID from beta adapter"
    );
}

// ===========================================================================
// Test: spawn_resume uses per-verifier adapters field
// ===========================================================================
// Mirrors session_reuse.rs but with the adapters slice API.

#[test]
fn spawn_resume_uses_per_verifier_adapters_field() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    // Seed a prior round with a SID and turnsUsed=1 (< maxTurn=3 → reuse path)
    let prev_round_dir = root
        .join("goals")
        .join(&goal_id)
        .join("rounds")
        .join("1")
        .join("v1");
    fs::create_dir_all(&prev_round_dir).unwrap();
    fs::write(
        prev_round_dir.join("meta.json"),
        serde_json::json!({"sid": "s1-prior", "turnsUsed": 1}).to_string(),
    )
    .unwrap();
    fs::write(
        prev_round_dir.join("verdict.json"),
        serde_json::json!({"status": null}).to_string(),
    )
    .unwrap();

    let script = write_script(dir.path(), "resume_ok.sh", acp_ok_script_body());
    let adapters = vec![script_adapter(&script)];

    let runs = rt()
        .block_on(spawn::spawn_resume(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 2,
            config: &store::Config::load_in(root).unwrap(),
            prompt: PROMPT,
            adapters: &adapters, // <-- NEW field name, will fail to compile
        }))
        .expect("resume spawn succeeds");

    assert_eq!(runs.len(), 1, "m=1 resumes exactly one verifier run");
}

// ===========================================================================
// Test: spawn_resume with mixed adapters per verifier slot
// ===========================================================================

#[test]
fn spawn_resume_mixed_adapters_each_verifier_uses_own() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let capture_dir = root.join("capture");
    fs::create_dir_all(&capture_dir).unwrap();

    let config = serde_json::json!({
        "n": 1, "m": 2, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    // Seed prior round for both verifiers (turnsUsed=1 < maxTurn → reuse)
    for vid in ["v1", "v2"] {
        let prev_vdir = root
            .join("goals")
            .join(&goal_id)
            .join("rounds")
            .join("1")
            .join(vid);
        fs::create_dir_all(&prev_vdir).unwrap();
        fs::write(
            prev_vdir.join("meta.json"),
            serde_json::json!({"sid": format!("{}-prior", vid), "turnsUsed": 1}).to_string(),
        )
        .unwrap();
        fs::write(
            prev_vdir.join("verdict.json"),
            serde_json::json!({"status": null}).to_string(),
        )
        .unwrap();
    }

    let script_alpha = write_script(
        dir.path(),
        "resume_alpha.sh",
        &format!(
            r#"#!/bin/sh
echo "alpha" > "{cap}/$VERIFIER_LOOP_VERIFIER_ID.resume_marker"
cat <<'EOF'
{{"type":"session","id":"alpha-resumed"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
"#,
            cap = capture_dir.to_string_lossy()
        ),
    );

    let script_beta = write_script(
        dir.path(),
        "resume_beta.sh",
        &format!(
            r#"#!/bin/sh
echo "beta" > "{cap}/$VERIFIER_LOOP_VERIFIER_ID.resume_marker"
cat <<'EOF'
{{"type":"session","id":"beta-resumed"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
"#,
            cap = capture_dir.to_string_lossy()
        ),
    );

    let adapters = vec![script_adapter(&script_alpha), script_adapter(&script_beta)];

    let runs = rt()
        .block_on(spawn::spawn_resume(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 2,
            config: &store::Config::load_in(root).unwrap(),
            prompt: PROMPT,
            adapters: &adapters,
        }))
        .expect("resume spawn succeeds");

    assert_eq!(runs.len(), 2);

    // Each verifier ran its OWN adapter
    let v1_marker = fs::read_to_string(capture_dir.join("v1.resume_marker")).unwrap();
    let v2_marker = fs::read_to_string(capture_dir.join("v2.resume_marker")).unwrap();
    assert_eq!(v1_marker.trim(), "alpha", "v1 used alpha adapter on resume");
    assert_eq!(v2_marker.trim(), "beta", "v2 used beta adapter on resume");
}
