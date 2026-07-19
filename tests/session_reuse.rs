// tasks.md §6 — Session reuse (verifier-spawn spec).
// RED phase: written first, against the spec, BEFORE any implementation.
//
// Scope of THIS test (§6): on RESUME, a verifier whose `turnsUsed < maxTurn` is resumed
// via the adapter's `--session <sid>` resume command on the SAME SID; a verifier that has
// reached `maxTurn` is freshly spawned (new SID) and the prior SID is archived. The
// `VERIFIER_LOOP_ROUND` env var reflects the new round while `VERIFIER_LOOP_VERIFIER_ID`
// stays stable across rounds.
//
// Reuses the §5 `spawn` API surface (`SpawnInput`, `spawn_resume`) — this file only adds
// the resume/reuse/archival assertions. The fake-script + custom-adapter strategy is the
// same as `spawn_orchestrator.rs`: deterministic, no real `pi`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use verifier_loop::goal;
use verifier_loop::{acp, spawn, store};

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

/// Adapter whose spawn template is the bare script, and whose resume template embeds the
/// SID via `--session {sid}` so the fake script can record whether it was resumed.
fn script_adapter_with_resume(script_path: &str) -> acp::Adapter {
    acp::Adapter::custom(
        script_path.to_string(),
        format!("{script_path} --session {{sid}}"),
    )
}

/// Shell snippet that writes a minimal APPROVE verdict into the orchestrator-injected
/// slot path. Used by the session-reuse stubs so the verdict-enforcement nudge loop
/// (D5, now also active on resume rounds) sees a verdict present and does NOT re-invoke
/// the stub — keeping these tests focused on sid/turnsUsed/archive mechanics, not the
/// nudge loop (which has its own dedicated tests in compaction_recovery.rs).
const VERDICT_WRITE_SNIPPET: &str = r#"
SLOT="$VERIFIER_LOOP_HOME/goals/$VERIFIER_LOOP_GOAL_ID/rounds/$VERIFIER_LOOP_ROUND/$VERIFIER_LOOP_VERIFIER_ID"
mkdir -p "$SLOT"
printf '%s\n' '{"status":"APPROVE","registeredAt":"2026-07-11T00:00:00Z"}' > "$SLOT/verdict.json"
"#;

/// Seed a prior round's per-verifier meta + null verdict, simulating a finished round 1.
fn seed_prior_round(root: &Path, goal_id: &str, round: u32, vid: &str, sid: &str, turns_used: u32) {
    let vdir = root
        .join("goals")
        .join(goal_id)
        .join("rounds")
        .join(round.to_string())
        .join(vid);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(
        vdir.join("verdict.json"),
        serde_json::json!({ "status": null }).to_string(),
    )
    .unwrap();
    fs::write(
        vdir.join("meta.json"),
        serde_json::json!({ "sid": sid, "turnsUsed": turns_used }).to_string(),
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// §6.1 — reused session continues on the same SID via --session
// ---------------------------------------------------------------------------

#[test]
fn resume_reuses_sid_when_turns_used_below_max() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);
    seed_prior_round(root, &goal_id, 1, "v1", "s1-prior", 1); // turnsUsed=1 < maxTurn=3

    // The fake script records its argv + identity env into a capture file.
    let capture_dir = root.join("captures");
    fs::create_dir_all(&capture_dir).unwrap();
    let script = write_script(
        dir.path(),
        "reuse.sh",
        &format!(
            r#"#!/bin/sh
{{
  printf '%s\n' "$VERIFIER_LOOP_GOAL_ID"
  printf '%s\n' "$VERIFIER_LOOP_VERIFIER_ID"
  printf '%s\n' "$VERIFIER_LOOP_ROUND"
  printf 'ARGS:'
  for a in "$@"; do printf ' %s' "$a"; done
  printf '\n'
}} > "{cap}/v1.argv"
cat <<'EOF'
{{"type":"session","id":"s1-resumed"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
{verdict}
"#,
            cap = capture_dir.to_string_lossy(),
            verdict = VERDICT_WRITE_SNIPPET
        ),
    );
    let adapter = script_adapter_with_resume(&script);

    let runs = rt()
        .block_on(spawn::spawn_resume(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 2,
            config: &store::Config::load_in(root).unwrap(),
            prompt: PROMPT,
            adapter: &adapter,
        }))
        .expect("resume spawn succeeds");

    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].sid.as_deref(),
        Some("s1-resumed"),
        "new SID captured"
    );

    let cap = fs::read_to_string(capture_dir.join("v1.argv")).unwrap();
    let lines: Vec<&str> = cap.trim().lines().collect();
    assert_eq!(lines[0], goal_id, "goalId injected");
    assert_eq!(lines[1], "v1", "verifierId STABLE across rounds");
    assert_eq!(lines[2], "2", "ROUND incremented to the new round");
    let argv_line = lines[3];
    assert!(
        argv_line.contains("--session") && argv_line.contains("s1-prior"),
        "resume path must invoke --session <prior sid>: {argv_line}"
    );

    // New round's meta.json reflects the resumed session's captured SID.
    let new_meta: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            root.join("goals")
                .join(&goal_id)
                .join("rounds")
                .join("2")
                .join("v1")
                .join("meta.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(new_meta["sid"], "s1-resumed", "round-2 meta has new sid");
}

// ---------------------------------------------------------------------------
// §6.1 — exhausted session is freshly spawned; prior SID archived
// ---------------------------------------------------------------------------

#[test]
fn exhausted_session_spawns_fresh_and_archives_prior_sid() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);
    seed_prior_round(root, &goal_id, 1, "v1", "s1-prior", 3); // turnsUsed=3 == maxTurn → fresh

    let capture_dir = root.join("captures");
    fs::create_dir_all(&capture_dir).unwrap();
    let script = write_script(
        dir.path(),
        "fresh.sh",
        &format!(
            r#"#!/bin/sh
# Capture argv ONLY on first invocation (skip nudge-loop re-invocations).
if [ ! -f "{cap}/v1.argv" ]; then {{
  printf 'ARGS:'
  for a in "$@"; do printf ' %s' "$a"; done
  printf '\n'
}} > "{cap}/v1.argv"; fi
cat <<'EOF'
{{"type":"session","id":"s1-fresh"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
{verdict}
"#,
            cap = capture_dir.to_string_lossy(),
            verdict = VERDICT_WRITE_SNIPPET
        ),
    );
    let adapter = script_adapter_with_resume(&script);

    let runs = rt()
        .block_on(spawn::spawn_resume(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 2,
            config: &store::Config::load_in(root).unwrap(),
            prompt: PROMPT,
            adapter: &adapter,
        }))
        .expect("resume spawn succeeds");

    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].sid.as_deref(),
        Some("s1-fresh"),
        "fresh SID captured"
    );

    // Fresh spawn must NOT pass --session.
    let argv_line = fs::read_to_string(capture_dir.join("v1.argv")).unwrap();
    assert!(
        !argv_line.contains("--session"),
        "exhausted verifier must spawn fresh (no --session): {argv_line}"
    );

    // Prior SID archived under its originating round directory.
    let archive_path = root
        .join("goals")
        .join(&goal_id)
        .join("rounds")
        .join("1")
        .join("v1")
        .join("archive.json");
    assert!(archive_path.exists(), "prior SID archived");
    let archive: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&archive_path).unwrap()).unwrap();
    assert_eq!(archive["sid"], "s1-prior", "archived prior sid recorded");
}

// ---------------------------------------------------------------------------
// §6.2 — round increments, verifierId stable (explicit combined assertion)
// ---------------------------------------------------------------------------

#[test]
fn round_increments_while_verifierid_stays_stable_across_two_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 5, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);
    seed_prior_round(root, &goal_id, 1, "v1", "sid-1", 1);

    let capture_dir = root.join("captures");
    fs::create_dir_all(&capture_dir).unwrap();
    // Script appends each run's (round,verifierId) to a log so we can see history.
    let script = write_script(
        dir.path(),
        "log.sh",
        &format!(
            r#"#!/bin/sh
printf '%s %s\n' "$VERIFIER_LOOP_ROUND" "$VERIFIER_LOOP_VERIFIER_ID" >> "{cap}/history"
cat <<'EOF'
{{"type":"session","id":"sid-$VERIFIER_LOOP_ROUND"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
{verdict}
"#,
            cap = capture_dir.to_string_lossy(),
            verdict = VERDICT_WRITE_SNIPPET
        ),
    );
    let adapter = script_adapter_with_resume(&script);

    let cfg = store::Config::load_in(root).unwrap();
    // Round 2 (reuse: turnsUsed=1 < maxTurn=5).
    rt().block_on(spawn::spawn_resume(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 2,
        config: &cfg,
        prompt: PROMPT,
        adapter: &adapter,
    }))
    .unwrap();
    // Round 3 (reuse: prior turnsUsed still recorded; SID captured last round).
    seed_prior_round(root, &goal_id, 2, "v1", "sid-2", 2);
    rt().block_on(spawn::spawn_resume(spawn::SpawnInput {
        root,
        goal_id: &goal_id,
        round: 3,
        config: &cfg,
        prompt: PROMPT,
        adapter: &adapter,
    }))
    .unwrap();

    let history = fs::read_to_string(capture_dir.join("history")).unwrap();
    let entries: Vec<&str> = history.trim().lines().collect();
    assert_eq!(
        entries,
        vec!["2 v1", "3 v1"],
        "round increments, verifierId stable"
    );
}

// ---------------------------------------------------------------------------
// §6 — prior SID missing/empty falls back to fresh spawn (gh #45/#48/#56/#59/#62/
// #65/#66/#67/#69: "No session found matching '--mode'")
// ---------------------------------------------------------------------------
//
// Root cause: spawn_resume's reuse arm used `meta.sid.clone().unwrap_or_default()`.
// When a verifier timed out without emitting its `session` event (or the meta.json
// was hand-seeded with `null`/`""`), `unwrap_or_default()` produced an empty string,
// and `build_resume_command` substituted `{sid}` with `""`, yielding argv like
// `pi --session  --mode json`. Whitespace-splitting collapses the doubled space so
// pi parses `--mode` as the session-name argument to `--session` and prints
// `No session found matching '--mode'` — cascading into null verdicts + cooldown.
//
// Fix: a None/empty prior SID MUST fall back to a fresh spawn (no `--session`),
// exactly like the exhausted-turns arm. This pair of tests pins both shapes
// (JSON null and JSON empty string) against regression.

/// Seed a prior round's per-verifier meta + null verdict with a SID that is JSON null.
/// (The shared `seed_prior_round` helper only accepts `&str` sid values, so this
/// bypasses it to write a literal `null`.)
fn seed_prior_round_null_sid(root: &Path, goal_id: &str, round: u32, vid: &str, turns_used: u32) {
    let vdir = root
        .join("goals")
        .join(goal_id)
        .join("rounds")
        .join(round.to_string())
        .join(vid);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(
        vdir.join("verdict.json"),
        serde_json::json!({ "status": null }).to_string(),
    )
    .unwrap();
    // `"sid": null` (JSON null, not a string) — what extract_sid returns nothing for.
    fs::write(
        vdir.join("meta.json"),
        serde_json::json!({ "sid": serde_json::Value::Null, "turnsUsed": turns_used })
            .to_string(),
    )
    .unwrap();
}

#[test]
fn resume_falls_back_to_fresh_when_prior_sid_is_null() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);
    // turnsUsed=1 < maxTurn=3, BUT sid is JSON null (e.g. a prior round that timed out
    // before the `session` event was emitted). The reuse arm MUST reject this and fall
    // through to a fresh spawn.
    seed_prior_round_null_sid(root, &goal_id, 1, "v1", 1);

    let capture_dir = root.join("captures");
    fs::create_dir_all(&capture_dir).unwrap();
    let script = write_script(
        dir.path(),
        "fresh_on_null.sh",
        &format!(
            r#"#!/bin/sh
# Capture argv ONLY on first invocation (skip nudge-loop re-invocations).
if [ ! -f "{cap}/v1.argv" ]; then {{
  printf 'ARGS:'
  for a in "$@"; do printf ' %s' "$a"; done
  printf '\n'
}} > "{cap}/v1.argv"; fi
cat <<'EOF'
{{"type":"session","id":"s1-fresh"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
{verdict}
"#,
            cap = capture_dir.to_string_lossy(),
            verdict = VERDICT_WRITE_SNIPPET
        ),
    );
    let adapter = script_adapter_with_resume(&script);

    let runs = rt()
        .block_on(spawn::spawn_resume(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 2,
            config: &store::Config::load_in(root).unwrap(),
            prompt: PROMPT,
            adapter: &adapter,
        }))
        .expect("resume spawn succeeds even with null prior sid");

    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].sid.as_deref(),
        Some("s1-fresh"),
        "fresh SID captured (not a broken resume)"
    );

    // Fresh spawn must NOT pass --session (the bug signature was `ARGS: --session`).
    let argv_line = fs::read_to_string(capture_dir.join("v1.argv")).unwrap();
    assert!(
        !argv_line.contains("--session"),
        "null prior SID must fall back to fresh spawn (no --session): {argv_line}"
    );

    // No archive.json: nothing to archive when prior SID was null.
    let archive_path = root
        .join("goals")
        .join(&goal_id)
        .join("rounds")
        .join("1")
        .join("v1")
        .join("archive.json");
    assert!(
        !archive_path.exists(),
        "no archive.json when prior SID was null (nothing to archive)"
    );
}

#[test]
fn resume_falls_back_to_fresh_when_prior_sid_is_empty_string() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 1, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);
    // turnsUsed=1 < maxTurn=3, BUT sid is the empty string `""` — the exact value
    // `unwrap_or_default()` produced in the buggy code path. Must fall back to fresh.
    seed_prior_round(root, &goal_id, 1, "v1", "", 1);

    let capture_dir = root.join("captures");
    fs::create_dir_all(&capture_dir).unwrap();
    let script = write_script(
        dir.path(),
        "fresh_on_empty.sh",
        &format!(
            r#"#!/bin/sh
# Capture argv ONLY on first invocation (skip nudge-loop re-invocations).
if [ ! -f "{cap}/v1.argv" ]; then {{
  printf 'ARGS:'
  for a in "$@"; do printf ' %s' "$a"; done
  printf '\n'
}} > "{cap}/v1.argv"; fi
cat <<'EOF'
{{"type":"session","id":"s1-fresh"}}
{{"type":"agent_end","messages":[],"willRetry":false}}
EOF
{verdict}
"#,
            cap = capture_dir.to_string_lossy(),
            verdict = VERDICT_WRITE_SNIPPET
        ),
    );
    let adapter = script_adapter_with_resume(&script);

    let runs = rt()
        .block_on(spawn::spawn_resume(spawn::SpawnInput {
            root,
            goal_id: &goal_id,
            round: 2,
            config: &store::Config::load_in(root).unwrap(),
            prompt: PROMPT,
            adapter: &adapter,
        }))
        .expect("resume spawn succeeds even with empty prior sid");

    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].sid.as_deref(),
        Some("s1-fresh"),
        "fresh SID captured (not a broken resume)"
    );

    let argv_line = fs::read_to_string(capture_dir.join("v1.argv")).unwrap();
    assert!(
        !argv_line.contains("--session"),
        "empty-string prior SID must fall back to fresh spawn (no --session): {argv_line}"
    );

    let archive_path = root
        .join("goals")
        .join(&goal_id)
        .join("rounds")
        .join("1")
        .join("v1")
        .join("archive.json");
    assert!(
        !archive_path.exists(),
        "no archive.json when prior SID was empty (nothing to archive)"
    );
}
