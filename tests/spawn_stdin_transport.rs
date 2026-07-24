// fix-spawn-argv-overflow §4 — STDIN transport RED tests (prompt-transport spec).
//
// RED phase: written FIRST, against the spec, BEFORE any §5 GREEN implementation.
// You are reading the RED author's output. A DIFFERENT teammate writes GREEN.
//
// Scope of THIS test (§4): the `stdin` transport delivers the rendered prompt to the
// child's stdin pipe (NOT argv), the child argv contains NO prompt-derived bytes, a 1 MiB
// prompt spawns without E2BIG, and EPIPE on the stdin write is handled per design D4
// (non-fatal after ACP output, fatal before any ACP → null verdict).
//
// Strategy (mirrors tests/spawn_orchestrator.rs): write a tiny fake_verifier.sh to a
// tempdir, register it as a custom adapter with `transport: Transport::Stdin`. The fake
// script reads its stdin into a capture file under the vdir (resolved via
// $VERIFIER_LOOP_* env vars), writes its argv into another capture file, and emits a
// minimal ACP stream on stdout. Because the CURRENT orchestrator sets `stdin =
// Stdio::null()` and ignores `adapter.transport` entirely, stdin_capture will be EMPTY
// → every test below FAILS (RED) until §5 GREEN implements real stdin piping.
//
// OUT of scope: goal-file transport (§6), tempfile lifecycle, build_spawn_command
// rewrite details, coverage gate. Those are §6/§5/§10 respectively.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use verifier_loop::{acp, acp::Transport, goal, spawn, store};

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

/// A custom adapter whose spawn template is just the script path (NO `{prompt}`
/// placeholder — the prompt travels via stdin per the transport) and whose transport
/// is explicitly [`Transport::Stdin`].
fn stdin_script_adapter(script_path: &str) -> acp::Adapter {
    acp::Adapter {
        spawn: script_path.to_string(),
        resume: script_path.to_string(),
        transport: Transport::Stdin,
    }
}

/// Returns the verifier dir `goals/<id>/rounds/<round>/<vid>`.
fn verifier_dir(root: &Path, goal_id: &str, round: u32, vid: &str) -> PathBuf {
    root.join("goals")
        .join(goal_id)
        .join("rounds")
        .join(round.to_string())
        .join(vid)
}

/// Common config snippet: m=1, generous timeout.
fn default_config() -> serde_json::Value {
    serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 15
    })
}

// ---------------------------------------------------------------------------
// §4.1 / §4.2 — stdin transport writes the full prompt to the child's stdin,
//               and the child argv contains NO prompt-derived bytes.
//
// A heredoc echo script that:
//   1. Saves its argv to `$VDIR/argv_capture.txt` (one token per line).
//   2. Reads ALL of stdin into `$VDIR/stdin_capture.txt`.
//   3. Emits a minimal ACP stream (session + agent_end) on stdout.
//
// RED today: the orchestrator sets `stdin = Stdio::null()`, so `cat` reads nothing
// and stdin_capture.txt is EMPTY (or absent). The assertion that it equals the prompt
// therefore FAILS until §5 GREEN pipes stdin.
// ---------------------------------------------------------------------------

#[test]
fn stdin_transport_writes_prompt_to_child_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = seed_goal(root, "g", &default_config());

    // The script resolves its vdir from the injected VERIFIER_LOOP_* env vars.
    let script = write_script(
        dir.path(),
        "capture.sh",
        r#"#!/bin/sh
VDIR="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
# 1. Capture argv (one token per line) so we can assert the prompt body is absent.
printf '%s\n' "$@" > "$VDIR/argv_capture.txt"
# 2. Read ALL of stdin into a capture file.
cat > "$VDIR/stdin_capture.txt"
# 3. Emit a minimal ACP stream so gather records a SID + final output.
cat <<'EOF'
{"type":"session","id":"stdin-sid"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"ok"}]}],"willRetry":false}
EOF
# 4. Write a verdict so the verdict-enforcement nudge loop (D5) does NOT re-run.
printf '%s\n' '{"status":"APPROVE","registeredAt":"2026-07-11T00:00:00Z"}' > "$VDIR/verdict.json"
"#,
    );
    let adapter = stdin_script_adapter(&script);

    let prompt = "UNIQUE_PROMPT_BODY_::stdin-transport-marker::";
    let runs = rt()
        .block_on(spawn::spawn_round(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 1,
            config: &store::Config::load_in(root).unwrap(),
            prompt,
            adapter: &adapter,
            verifier_count: None,
            id_prefix: None,
            id_offset: 0,
        }))
        .expect("spawn round should not hard-error");

    assert_eq!(runs.len(), 1);
    let vdir = verifier_dir(root, &goal_id, 1, "v1");

    // (a) The full prompt MUST arrive on stdin (the core stdin-transport contract).
    let stdin_captured = fs::read_to_string(vdir.join("stdin_capture.txt")).unwrap_or_default();
    assert_eq!(
        stdin_captured, prompt,
        "stdin transport must write the FULL rendered prompt to the child's stdin pipe"
    );

    // (b) The argv MUST NOT contain any prompt-derived bytes.
    let argv_captured = fs::read_to_string(vdir.join("argv_capture.txt")).unwrap_or_default();
    assert!(
        !argv_captured.contains("UNIQUE_PROMPT_BODY"),
        "argv must NOT contain prompt-derived bytes; got: {argv_captured}"
    );
}

// ---------------------------------------------------------------------------
// §4.3 — a 1 MiB prompt spawns successfully via stdin (no E2BIG), and the
//         child receives the full 1 MiB on stdin.
//
// RED today: stdin is null → the captured stdin length is 0, not 1 MiB. (The spawn
// itself does not error today because the prompt is simply dropped — neither in argv
// nor piped — so no E2BIG is triggered. The LENGTH assertion is what makes this RED.)
// ---------------------------------------------------------------------------

#[test]
fn large_prompt_spawns_via_stdin_without_ebig() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = seed_goal(root, "g", &default_config());

    let script = write_script(
        dir.path(),
        "big_capture.sh",
        r#"#!/bin/sh
VDIR="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
cat > "$VDIR/stdin_capture.txt"
cat <<'EOF'
{"type":"session","id":"big-sid"}
{"type":"agent_end","messages":[],"willRetry":false}
EOF
printf '%s\n' '{"status":"APPROVE","registeredAt":"2026-07-11T00:00:00Z"}' > "$VDIR/verdict.json"
"#,
    );
    let adapter = stdin_script_adapter(&script);

    // 1 MiB of repeated bytes. This far exceeds MAX_ARG_STRLEN (128 KB) and would
    // trigger E2BIG if it were placed in argv (the bug we are fixing).
    let prompt = "A".repeat(1024 * 1024);

    let result = rt().block_on(async {
        spawn::spawn_round(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 1,
            config: &store::Config::load_in(root).unwrap(),
            prompt: &prompt,
            adapter: &adapter,
            verifier_count: None,
            id_prefix: None,
            id_offset: 0,
        })
        .await
    });

    // The spawn MUST succeed (no E2BIG / "Argument list too long").
    let runs = result.expect("1 MiB prompt via stdin must NOT trigger E2BIG");
    assert_eq!(runs.len(), 1);
    assert!(!runs[0].timed_out, "the verifier should not time out");

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let stdin_bytes = fs::read(vdir.join("stdin_capture.txt")).unwrap_or_default();
    assert_eq!(
        stdin_bytes.len(),
        1024 * 1024,
        "the child must receive the FULL 1 MiB prompt on stdin (got {} bytes)",
        stdin_bytes.len()
    );
    assert!(
        stdin_bytes.iter().all(|&b| b == b'A'),
        "the captured stdin content must match the prompt bytes exactly"
    );
}

// ---------------------------------------------------------------------------
// §4.4 — EPIPE on the stdin write is NON-FATAL when the child has already
//         produced a recognizable ACP stream (design D4).
//
// The fake script reads only the first 64 bytes of stdin, then emits session +
// agent_end and exits 0 WITHOUT draining the rest. With a real stdin pipe (GREEN),
// the orchestrator's background write task will hit EPIPE after the child exits.
// The contract: the run is treated as successful (timed_out=false, SID captured).
//
// RED today: stdin is null → the partial-read capture is EMPTY. The assertion that
// it equals the first 64 prompt bytes FAILS until §5 GREEN pipes stdin (at which
// point the EPIPE-non-fatal path also needs to be implemented for the run to succeed).
// ---------------------------------------------------------------------------

#[test]
fn epipe_after_verdict_is_non_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = seed_goal(root, "g", &default_config());

    let script = write_script(
        dir.path(),
        "early_exit.sh",
        r#"#!/bin/sh
VDIR="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
# Read only the first 64 bytes, then stop reading stdin entirely.
dd bs=1 count=64 of="$VDIR/stdin_partial.txt" 2>/dev/null
# Emit a recognizable ACP stream BEFORE exiting (so EPIPE is non-fatal per D4).
cat <<'EOF'
{"type":"session","id":"epipe-after-sid"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"done-early"}]}],"willRetry":false}
EOF
# Write a verdict so the nudge loop (D5) does NOT re-run and overwrite the capture.
printf '%s\n' '{"status":"APPROVE","registeredAt":"2026-07-11T00:00:00Z"}' > "$VDIR/verdict.json"
# Exit 0 without draining the rest of stdin → orchestrator write hits EPIPE.
exit 0
"#,
    );
    let adapter = stdin_script_adapter(&script);

    // 10 KB prompt: much larger than the 64 bytes the script reads, so a real stdin
    // pipe would still have pending bytes when the child exits → EPIPE on the write.
    let prompt = "B".repeat(10 * 1024);

    let runs = rt()
        .block_on(spawn::spawn_round(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 1,
            config: &store::Config::load_in(root).unwrap(),
            prompt: &prompt,
            adapter: &adapter,
            verifier_count: None,
            id_prefix: None,
            id_offset: 0,
        }))
        .expect("EPIPE after ACP output must NOT hard-error the spawn round");

    assert_eq!(runs.len(), 1);
    // The run gathered cleanly (not a timeout, SID captured).
    assert!(
        !runs[0].timed_out,
        "EPIPE after ACP output is non-fatal: run must not be marked timed out"
    );
    assert_eq!(
        runs[0].sid.as_deref(),
        Some("epipe-after-sid"),
        "SID must be captured despite the early stdin close"
    );

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    // Proves stdin was actually piped (currently RED: file empty because stdin=null).
    let partial = fs::read(vdir.join("stdin_partial.txt")).unwrap_or_default();
    assert_eq!(
        partial.len(),
        64,
        "the child must have received 64 bytes on stdin before closing (got {} bytes)",
        partial.len()
    );
    assert!(
        partial.iter().all(|&b| b == b'B'),
        "the partial stdin capture must match the prompt bytes"
    );
}

// ---------------------------------------------------------------------------
// §4.5 — EPIPE on the stdin write BEFORE any ACP output is FATAL (fail-closed):
//         the verdict stays null (sid None, final_output None). No panic.
//
// The fake script reads exactly 1 byte from stdin (proving the pipe was connected),
// then exits 1 WITHOUT emitting any ACP event. With a real stdin pipe (GREEN), the
// orchestrator's write hits EPIPE almost immediately; per D4 this is fatal because
// no ACP event was ever parsed → null verdict (fail-closed).
//
// RED today: stdin is null → the 1-byte capture is EMPTY. The assertion that it
// matches the first prompt byte FAILS until §5 GREEN pipes stdin.
// ---------------------------------------------------------------------------

#[test]
fn epipe_before_acp_output_is_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = seed_goal(root, "g", &default_config());

    let script = write_script(
        dir.path(),
        "no_acp.sh",
        r#"#!/bin/sh
VDIR="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
# Read exactly 1 byte (proves the stdin pipe was connected), then bail.
dd bs=1 count=1 of="$VDIR/stdin_byte.txt" 2>/dev/null
# Emit NO ACP events. Exit non-zero.
exit 1
"#,
    );
    let adapter = stdin_script_adapter(&script);

    let prompt = "FATAL_BEFORE_ACP_BODY";

    let result = rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt,
        adapter: &adapter,
        verifier_count: None,
        id_prefix: None,
        id_offset: 0,
    }));

    // The gather barrier must complete without panicking (EPIPE is fatal-but-handled).
    let runs = result.expect("gather must not panic on EPIPE-before-ACP");

    assert_eq!(runs.len(), 1);
    // Fail-closed: no SID, no final output → null verdict.
    assert!(
        runs[0].sid.is_none(),
        "EPIPE before any ACP output must yield NO sid (fail-closed)"
    );
    assert!(
        runs[0].final_output.is_none(),
        "EPIPE before any ACP output must yield NO final output (fail-closed)"
    );

    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    // Proves stdin was piped (currently RED: file empty/missing because stdin=null).
    let byte = fs::read(vdir.join("stdin_byte.txt")).unwrap_or_default();
    assert_eq!(
        byte, b"F",
        "the child must have received the first prompt byte on stdin (got {:?})",
        byte
    );

    // The on-disk verdict MUST remain null (pre-created baseline; no verdict registered).
    let verdict: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(vdir.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["status"],
        serde_json::Value::Null,
        "fail-closed: verdict stays null when EPIPE precedes any ACP output"
    );
}
