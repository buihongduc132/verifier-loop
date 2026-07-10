// fix-spawn-argv-overflow §6 — GOAL-FILE transport RED tests (prompt-transport spec).
//
// RED phase: written FIRST, against the spec, BEFORE any §7 GREEN implementation.
// You are reading the RED author's output. A DIFFERENT teammate (agent1) writes GREEN.
//
// Scope of THIS test (§6): the `goal-file` transport writes the rendered prompt to a
// tempfile under the OS temp dir, substitutes the tempfile's absolute path for every
// `{goalFile}` placeholder in the spawn template, unlinks the tempfile after the child
// is spawned (the child holds the open fd), unlinks it on spawn failure too, supports
// a 1 MiB prompt without E2BIG, and stale `verifier-loop-*` tempfiles are swept at
// startup (design D3/D5/R1; spec "Tempfile lifecycle is bounded and fail-safe").
//
// Strategy (mirrors tests/spawn_stdin_transport.rs + tests/spawn_orchestrator.rs):
// write a tiny fake_verifier.sh to a tempdir, register it as a custom adapter with
// `transport: Transport::GoalFile` and spawn template `<script_path> {goalFile}`. The
// fake script reads the file at the path passed as its first argv, copies the contents
// to `$VDIR/goalfile_capture.txt`, dumps its argv to `$VDIR/argv_capture.txt`, and
// emits a minimal ACP stream on stdout.
//
// RED today: the orchestrator's `GoalFile` branch is a STUB (see
// `build_command_from_template` in src/spawn/orchestrator.rs) — it still treats the
// template as legacy `{prompt}` inline and does NOT:
//   - write a tempfile
//   - substitute `{goalFile}`
//   - unlink anything
// So `{goalFile}` reaches the child as a LITERAL argv token, the capture file is empty,
// and no tempfile is created/unlinked → every test below FAILS until §7 GREEN lands.
//
// OUT of scope: stdin transport (§4/§5), build_command_from_template rewrite details,
// coverage gate (§10).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use verifier_loop::{acp, acp::Transport, goal, spawn, store};

/// Serialize all tests in this file. They all touch the SHARED `std::env::temp_dir()`:
/// the spawn tests create `verifier-loop-*` tempfiles via the orchestrator's
/// `TempPromptFile`, and `stale_tempfiles_swept_at_startup` calls the global
/// `sweep_stale_tempfiles()` which deletes every `verifier-loop-*` entry. Without
/// this lock the sweep races with parallel spawns and deletes tempfiles that sibling
/// tests' children are still reading. This is the root-cause fix for the shared
/// global resource, not a workaround.
static TEST_LOCK: Mutex<()> = Mutex::new(());

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

/// A custom adapter whose spawn template is `<script_path> {goalFile}` and whose
/// transport is explicitly [`Transport::GoalFile`]. The `{goalFile}` placeholder is
/// substituted by the orchestrator (§7 GREEN) with the tempfile path.
fn goalfile_script_adapter(script_path: &str) -> acp::Adapter {
    acp::Adapter {
        spawn: format!("{script_path} {{goalFile}}"),
        resume: format!("{script_path} {{goalFile}}"),
        transport: Transport::GoalFile,
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
// §6.1 — goal-file transport substitutes the tempfile path into `{goalFile}`,
//         the child receives the FULL prompt via the file, and the argv contains
//         the tempfile path (NOT the prompt body). The tempfile is unlinked
//         after spawn (spec scenario "Tempfile is unlinked after successful spawn").
//
// The fake script:
//   1. `$1` = the {goalFile} path (first argv after the script name).
//   2. Copies the file at `$1` to `$VDIR/goalfile_capture.txt`.
//   3. Dumps argv to `$VDIR/argv_capture.txt` (one token per line).
//   4. Emits session + agent_end on stdout.
//
// RED today: the GoalFile stub does NOT substitute `{goalFile}`, so `$1` is the
// literal token `{goalFile}` (or the stub inlines the prompt), and the capture
// file is empty / the argv does not contain a real path. Also no tempfile is
// created, so the "tempfile is unlinked" assertion cannot hold.
// ---------------------------------------------------------------------------

#[test]
fn goal_file_transport_substitutes_tempfile_path() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = seed_goal(root, "g", &default_config());

    let script = write_script(
        dir.path(),
        "goalfile_capture.sh",
        r#"#!/bin/sh
# $1 is the {goalFile} path substituted by the orchestrator (§7 GREEN).
GOAL_FILE="$1"
VDIR="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
# Dump argv (one token per line).
printf '%s\n' "$@" > "$VDIR/argv_capture.txt"
# Record the path the orchestrator handed us, so the test can assert it is a real
# path (and later that the file was unlinked).
printf '%s' "$GOAL_FILE" > "$VDIR/goalfile_path.txt"
# Copy the prompt file's contents into the vdir for assertion.
if [ -r "$GOAL_FILE" ]; then
  cp "$GOAL_FILE" "$VDIR/goalfile_capture.txt"
else
  : > "$VDIR/goalfile_capture.txt"
fi
# Emit a minimal ACP stream.
cat <<'EOF'
{"type":"session","id":"goalfile-sid"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"ok"}]}],"willRetry":false}
EOF
"#,
    );
    let adapter = goalfile_script_adapter(&script);

    let prompt = "UNIQUE_GOALFILE_BODY_::goal-file-transport-marker::";
    let runs = rt()
        .block_on(spawn::spawn_round(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 1,
            config: &store::Config::load_in(root).unwrap(),
            prompt,
            adapters: &[adapter.clone()],
        }))
        .expect("spawn round should not hard-error");

    assert_eq!(runs.len(), 1);
    let vdir = verifier_dir(root, &goal_id, 1, "v1");

    // (a) The child received the FULL prompt via the tempfile.
    let captured =
        fs::read_to_string(vdir.join("goalfile_capture.txt")).unwrap_or_default();
    assert_eq!(
        captured, prompt,
        "goal-file transport must write the FULL rendered prompt to the tempfile and \
         the child must read it via the substituted path"
    );

    // (b) The argv MUST contain the tempfile path, NOT the prompt body.
    let argv =
        fs::read_to_string(vdir.join("argv_capture.txt")).unwrap_or_default();
    assert!(
        !argv.contains("UNIQUE_GOALFILE_BODY"),
        "argv must NOT contain prompt-derived bytes; got: {argv}"
    );
    // The path handed to the child must look like a tempfile under the OS temp dir,
    // not the literal placeholder token.
    let recorded_path =
        fs::read_to_string(vdir.join("goalfile_path.txt")).unwrap_or_default();
    assert!(
        !recorded_path.contains("{goalFile}") && !recorded_path.is_empty(),
        "the {{goalFile}} placeholder must be substituted with a real path, not passed \
         through literally; got: {recorded_path}"
    );

    // (c) The tempfile MUST be unlinked after spawn (spec: "Tempfile is unlinked after
    //     successful spawn"). The §7 GREEN author chooses the path; we assert the file
    //     no longer exists on disk once spawn returned + the child read it.
    let p = PathBuf::from(&recorded_path);
    assert!(
        !p.exists(),
        "tempfile {recorded_path} must be unlinked after the child spawned (the child \
         reads via its inherited fd, not the directory entry)"
    );
}

// ---------------------------------------------------------------------------
// §6.2 — the goal-file tempfile is unlinked after spawn, and a verifier that
//         exits NON-ZERO (failure) still triggers the cleanup. Also pins the
//         "on spawn failure the tempfile is unlinked" contract from spec
//         "Tempfile is unlinked on spawn failure".
//
// NOTE on testability: a *pure* `Command::spawn`-failure test (e.g. pointing at a
// non-existent program) cannot be made observably RED against today's stub,
// because the stub never creates a tempfile in the first place — there is nothing
// to observe being unlinked, and the leak-count invariant holds vacuously. To get
// a genuinely failing test that pins the §7 GREEN unlink-on-failure contract, we
// instead exercise the tempfile lifecycle end-to-end: a VALID executable script
// reads the `{goalFile}` path, records it, then EXITS NON-ZERO (child failure
// after spawn). The contract: the child received the prompt via the file, AND the
// tempfile was unlinked once the child had spawned (regardless of the child's
// later exit status). §7 GREEN must keep this green by unlinking post-spawn.
//
// RED today: the GoalFile stub does NOT substitute `{goalFile}`, so `$1` reaches
// the script as the LITERAL token `{goalFile}` (no such file → capture empty), and
// no tempfile is ever created. Both the content assertion and the
// path-was-a-real-tempfile assertion FAIL.
// ---------------------------------------------------------------------------

#[test]
fn goal_file_tempfile_unlinked_on_spawn_failure() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = seed_goal(root, "g", &default_config());

    // A valid, executable script that reads the {goalFile} path, records what it
    // received, then exits NON-ZERO to simulate a failed verifier run.
    let script = write_script(
        dir.path(),
        "fail_after_read.sh",
        r#"#!/bin/sh
GOAL_FILE="$1"
VDIR="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
# Record the path handed to us (so the test can assert it was a real tempfile).
printf '%s' "$GOAL_FILE" > "$VDIR/goalfile_path.txt"
# Copy the prompt file contents if the path is readable.
if [ -r "$GOAL_FILE" ]; then
  cp "$GOAL_FILE" "$VDIR/goalfile_capture.txt"
else
  : > "$VDIR/goalfile_capture.txt"
fi
# Emit an ACP stream so gather records a SID, then exit NON-ZERO (child failure).
cat <<'EOF'
{"type":"session","id":"fail-after-read-sid"}
{"type":"agent_end","messages":[],"willRetry":false}
EOF
exit 3
"#,
    );
    let adapter = goalfile_script_adapter(&script);

    let prompt = "UNIQUE_FAILURE_BODY_::goal-file-failure-marker::";
    let runs = rt()
        .block_on(spawn::spawn_round(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 1,
            config: &store::Config::load_in(root).unwrap(),
            prompt,
            adapters: &[adapter.clone()],
        }))
        .expect("gather must not panic on a non-zero child exit");

    assert_eq!(runs.len(), 1);
    let vdir = verifier_dir(root, &goal_id, 1, "v1");

    // (a) The child received the FULL prompt via the tempfile (proves the goal-file
    //     path ran and wrote the file before spawn).
    let captured =
        fs::read_to_string(vdir.join("goalfile_capture.txt")).unwrap_or_default();
    assert_eq!(
        captured, prompt,
        "the goal-file tempfile must contain the FULL prompt even when the verifier \
         later fails"
    );

    // (b) The path handed to the child must be a real tempfile path (not the literal
    //     `{goalFile}` placeholder).
    let recorded_path =
        fs::read_to_string(vdir.join("goalfile_path.txt")).unwrap_or_default();
    assert!(
        !recorded_path.contains("{goalFile}") && !recorded_path.is_empty(),
        "the {{goalFile}} placeholder must be substituted with a real path even on \
         the failure path; got: {recorded_path}"
    );

    // (c) The tempfile MUST be unlinked after spawn (spec lifecycle invariant),
    //     regardless of the child's exit status.
    let p = PathBuf::from(&recorded_path);
    assert!(
        !p.exists(),
        "tempfile {recorded_path} must be unlinked after spawn even when the \
         verifier exits non-zero"
    );
}

/// Count `verifier-loop-*` entries currently present in `std::env::temp_dir()`.
/// Kept as a helper for future leak diagnostics; not used as a hard assertion here
/// (it is racy with parallel tests that create their own `verifier-loop-*` files).
#[allow(dead_code)]
fn count_verifier_loop_tempfiles() -> usize {
    fs::read_dir(std::env::temp_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("verifier-loop-")
        })
        .count()
}

// ---------------------------------------------------------------------------
// §6.3 — a 1 MiB prompt spawns via goal-file without E2BIG, and the tempfile
//         contains the full 1 MiB (spec "Large prompt via goal-file spawns
//         successfully").
//
// RED today: the GoalFile stub does NOT write a tempfile; it tries to inline the
// 1 MiB prompt into argv (the legacy {prompt} path), which triggers E2BIG. So the
// spawn hard-errors with `Argument list too long`. The §7 GREEN implementation
// eliminates the argv path entirely for goal-file.
// ---------------------------------------------------------------------------

#[test]
fn large_prompt_spawns_via_goal_file() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let goal_id = seed_goal(root, "g", &default_config());

    let script = write_script(
        dir.path(),
        "big_goalfile.sh",
        r#"#!/bin/sh
GOAL_FILE="$1"
VDIR="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
if [ -r "$GOAL_FILE" ]; then
  cp "$GOAL_FILE" "$VDIR/goalfile_capture.txt"
fi
cat <<'EOF'
{"type":"session","id":"big-goalfile-sid"}
{"type":"agent_end","messages":[],"willRetry":false}
EOF
"#,
    );
    let adapter = goalfile_script_adapter(&script);

    // 1 MiB of repeated bytes. This far exceeds MAX_ARG_STRLEN (128 KB) and would
    // trigger E2BIG if placed in argv — which is exactly what the RED stub does.
    let prompt = "A".repeat(1024 * 1024);

    let result = rt().block_on(spawn::spawn_round(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 1,
        config: &store::Config::load_in(root).unwrap(),
        prompt: &prompt,
        adapters: &[adapter.clone()],
    }));

    // (a) The spawn MUST succeed (no E2BIG / "Argument list too long").
    let runs = result.expect("1 MiB prompt via goal-file must NOT trigger E2BIG");
    assert_eq!(runs.len(), 1);
    assert!(!runs[0].timed_out, "the verifier should not time out");

    // (b) The tempfile MUST contain the full 1 MiB prompt.
    let vdir = verifier_dir(root, &goal_id, 1, "v1");
    let captured = fs::read(vdir.join("goalfile_capture.txt")).unwrap_or_default();
    assert_eq!(
        captured.len(),
        1024 * 1024,
        "the tempfile must contain the FULL 1 MiB prompt (got {} bytes)",
        captured.len()
    );
    assert!(
        captured.iter().all(|&b| b == b'A'),
        "the tempfile contents must match the prompt bytes exactly"
    );
}

// ---------------------------------------------------------------------------
// §6.4 — stale `verifier-loop-*` tempfiles in temp_dir() are swept at startup
//         (design R1 / tasks.md §7.3). Unrelated files MUST NOT be touched.
//
// RED today: `spawn::sweep_stale_tempfiles()` is a no-op stub (added so this test
// compiles). It removes nothing → the assertion that both stale files are gone
// FAILS until §7 GREEN implements the real directory scan + unlink.
// ---------------------------------------------------------------------------

#[test]
fn stale_tempfiles_swept_at_startup() {
    let _guard = TEST_LOCK.lock().unwrap();
    let tmp = std::env::temp_dir();

    // Two stale verifier-loop tempfiles (unique names so parallel tests don't collide).
    let tag = uuid_stub();
    let stale_a = tmp.join(format!("verifier-loop-stale-{tag}-A.txt"));
    let stale_b = tmp.join(format!("verifier-loop-stale-{tag}-B.txt"));
    fs::write(&stale_a, b"stale").unwrap();
    fs::write(&stale_b, b"stale").unwrap();

    // An unrelated file that MUST NOT be touched by the sweep.
    let unrelated = tmp.join(format!("sweep-unrelated-{tag}.txt"));
    fs::write(&unrelated, b"keep-me").unwrap();

    // Sanity: all three exist before the sweep.
    assert!(stale_a.exists(), "stale A pre-exists");
    assert!(stale_b.exists(), "stale B pre-exists");
    assert!(unrelated.exists(), "unrelated file pre-exists");

    // Invoke the sweep entry point (§7.3 GREEN implements the real scan).
    // Age threshold (SWEEP_MIN_AGE_SECS): the sweep only removes files older than this
    // so a freshly-started jewilo can't delete a concurrent sibling's active
    // tempfile. The fixtures above were just created, so age them past the threshold.
    std::thread::sleep(std::time::Duration::from_secs(spawn::SWEEP_MIN_AGE_SECS));
    spawn::sweep_stale_tempfiles();

    // The stale verifier-loop-* tempfiles MUST be gone.
    assert!(
        !stale_a.exists(),
        "stale tempfile {:?} must be swept at startup",
        stale_a
    );
    assert!(
        !stale_b.exists(),
        "stale tempfile {:?} must be swept at startup",
        stale_b
    );

    // Unrelated files MUST survive the sweep.
    assert!(
        unrelated.exists(),
        "unrelated file {:?} must NOT be removed by the sweep",
        unrelated
    );

    // Cleanup (in case the §7 GREEN sweep is selective and leaves our unrelated file).
    let _ = fs::remove_file(&unrelated);
}

/// Minimal unique-tag generator so parallel test runs don't collide on filenames.
/// (No `uuid` dep import needed at the test crate level; process pid + instant nonce
/// is unique enough for a tempfile-name collision guard.)
fn uuid_stub() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}
