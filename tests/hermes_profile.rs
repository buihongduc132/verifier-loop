// hermes-profile-adapter — Group 3: Spawn Layer Integration
//
// These tests verify that when the hermes adapter has a profile configured,
// the spawned command includes `-p <profile>` in its argv.
//
// Spec reference: hermes-profile-adapter/specs/verifier-spawn/spec.md

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use verifier_loop::{acp, goal, spawn, store};

/// Build the test's tokio runtime.
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

// ---------------------------------------------------------------------------
// Test 1: adapter_for("hermes", Some("verifier")) produces correct spawn template
// ---------------------------------------------------------------------------

/// When adapter_for is called for hermes with a profile, the resulting
/// adapter's spawn template should include `-p <profile>`.
#[test]
fn hermes_adapter_with_profile_has_correct_spawn_template() {
    // Get hermes adapter with profile "verifier"
    let adapter = acp::adapter_for("hermes", Some("verifier")).unwrap();
    
    // The spawn template should include -p verifier
    assert_eq!(
        adapter.spawn, "hermes -p verifier --mode json",
        "hermes adapter spawn template must include '-p verifier' when profile is set"
    );
    
    // The resume template should also include -p verifier
    assert_eq!(
        adapter.resume, "hermes -p verifier --session {sid} --mode json",
        "hermes adapter resume template must include '-p verifier' when profile is set"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Spawn command includes -p <profile> when profile is set
// ---------------------------------------------------------------------------

/// When the hermes adapter has a profile configured, the spawned command
/// must include `-p <profile>` as argv arguments. We verify this by using
/// a script that records its full argv to a file.
#[test]
fn spawn_includes_profile_flag_in_argv() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "test profile spawn", &config);

    // Capture dir for argv recording
    let capture_dir = root.join("captures");
    fs::create_dir_all(&capture_dir).unwrap();

    // Script that records its full argv ($0, $1, $2, ...) to a file
    let script = write_script(
        dir.path(),
        "argv_capture.sh",
        &format!(
            r#"#!/bin/sh
# Record all arguments to a capture file
echo "$@" > "{cap}/argv.txt"
# Emit minimal ACP so the orchestrator considers this a successful run
cat <<'ACP'
{{"type":"session","id":"test-sid"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
ACP
"#,
            cap = capture_dir.to_string_lossy()
        ),
    );

    // Create adapter with profile embedded in template - this simulates what
    // adapter_for("hermes", Some("verifier")) produces
    let adapter = acp::Adapter {
        spawn: format!("{} -p verifier --mode json", script),
        resume: format!("{} -p verifier --session {{sid}} --mode json", script),
        transport: acp::Transport::Stdin,
    };

    rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt: "",
        adapter: &adapter,
    }))
    .expect("spawn round succeeds");

    // Read the captured argv
    let argv_content = fs::read_to_string(capture_dir.join("argv.txt"))
        .expect("argv capture file should exist");

    // The command should include -p and verifier as separate args
    assert!(
        argv_content.contains("-p"),
        "spawn command must include -p flag, got argv: {}",
        argv_content
    );
    assert!(
        argv_content.contains("verifier"),
        "spawn command must include profile name 'verifier', got argv: {}",
        argv_content
    );
}

// ---------------------------------------------------------------------------
// Test 3: Profile flag is a single argv element (not shell-split)
// ---------------------------------------------------------------------------

/// The profile value must be passed as a single argv element.
/// A profile with spaces or special chars must NOT be shell-split.
#[test]
fn profile_value_is_single_argv_element() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "test profile argv isolation", &config);

    let capture_dir = root.join("captures");
    fs::create_dir_all(&capture_dir).unwrap();

    // Script that records argument count and each argument on its own line
    let script = write_script(
        dir.path(),
        "argv_count.sh",
        &format!(
            r#"#!/bin/sh
# Record argument count and each arg on its own line
echo "$#" > "{cap}/argc.txt"
i=1
for arg in "$@"; do
    echo "$arg" >> "{cap}/arg_$i.txt"
    i=$((i + 1))
done
# Emit minimal ACP
cat <<'ACP'
{{"type":"session","id":"test-sid"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
ACP
"#,
            cap = capture_dir.to_string_lossy()
        ),
    );

    // Create adapter with profile embedded in template
    let adapter = acp::Adapter {
        spawn: format!("{} -p my-profile --mode json", script),
        resume: format!("{} -p my-profile --session {{sid}} --mode json", script),
        transport: acp::Transport::Stdin,
    };

    rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt: "",
        adapter: &adapter,
    }))
    .expect("spawn round succeeds");

    // Verify -p and the profile value are separate argv elements
    let argc = fs::read_to_string(capture_dir.join("argc.txt"))
        .expect("argc capture file should exist");
    let argc: usize = argc.trim().parse().expect("argc should be a number");

    // We expect at least 4 args: -p, my-profile, --mode, json
    // (the script path itself is $0, not counted in $#)
    assert!(
        argc >= 4,
        "expected at least 4 args (-p, profile, --mode, json), got {}: check captures",
        argc
    );

    // Verify -p is its own argument
    let arg_1 = fs::read_to_string(capture_dir.join("arg_1.txt"))
        .expect("arg_1 should exist");
    assert_eq!(
        arg_1.trim(),
        "-p",
        "first arg must be -p flag"
    );

    // Verify profile value is its own argument (not combined with -p)
    let arg_2 = fs::read_to_string(capture_dir.join("arg_2.txt"))
        .expect("arg_2 should exist");
    assert_eq!(
        arg_2.trim(),
        "my-profile",
        "second arg must be the profile value as a separate element"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Resume command also includes profile flag
// ---------------------------------------------------------------------------

/// When resuming a session with a profiled hermes adapter, the resume command
/// must also include `-p <profile>`.
#[test]
fn resume_includes_profile_flag() {
    // Get hermes adapter with profile "verifier"
    let adapter = acp::adapter_for("hermes", Some("verifier")).unwrap();

    // Render the resume template with a session ID
    let rendered_resume = acp::render_resume(&adapter.resume, "abc-123", "");

    // The rendered resume command should include -p verifier
    assert!(
        rendered_resume.contains("-p verifier"),
        "resume command must include '-p verifier' when profile is set, got: {}",
        rendered_resume
    );
    
    // And the session ID should be substituted
    assert!(
        rendered_resume.contains("abc-123"),
        "resume command must include the session ID, got: {}",
        rendered_resume
    );
}
