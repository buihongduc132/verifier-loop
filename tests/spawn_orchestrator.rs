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

use verifier_loop::{acp, consensus, goal, receipt, spawn, store, verdict};

use sha2::{Digest, Sha256};

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

// ===========================================================================
// verifier-spawn MODIFIED (tamper-hardening) — RED phase
//
// New contract (D3 / tasks.md §6 of add-verifier-tamper-hardening):
//   1. Before launching each V*, spawn mints a fresh Ed25519 keypair via
//      `verdict::mint_and_pin_pubkey` and pins the pubkey into the slot as
//      `verifier-pubkey.json`.
//   2. The minted SigningKey's hex is injected into the V* process env as
//      `VERIFIER_LOOP_VERIFIER_SECRET`.
//   3. The stub backend forwards that env to its `jewije approve` call so the
//      signed-verdict path is taken.
//   4. End-to-end `jewilo NEW` (stub_approve backend) produces, per slot,
//      `verifier-pubkey.json` + a SIGNED `verdict.json` (signature + pubkeyId),
//      a populated `receipt-log.jsonl` (one entry per APPROVE), and a
//      `completion.json` whose hash inputs fold in the receipt head.
//
// These tests are written FIRST and are expected to FAIL until the GREEN
// spawn/consensus/stub tasks land. DO NOT implement here.
// ===========================================================================

/// Absolute path to the in-repo `scripts/stub_approve.sh` backend.
fn stub_approve_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/stub_approve.sh")
}

/// Seed a git work tree (the frozen snapshot requires one) + config + return goal dir.
/// Mirrors the cli_e2e.rs pattern: tempdir, `git init`, write `config.json`.
fn seed_worktree(home: &Path, n: u32, m: u32) {
    let git_ok = std::process::Command::new("git")
        .arg("-C")
        .arg(home)
        .args(["init", "-q"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(git_ok, "git init failed in tempdir");

    let cfg = serde_json::json!({
        "n": n, "m": m, "maxTurn": 3, "backend": "stub",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 30
    });
    fs::write(home.join("config.json"), cfg.to_string()).unwrap();
}

/// Drive `verifier-loop NEW "<goal>"` with the stub_approve backend against `home`.
/// Returns the raw process output regardless of exit status (some tests assert failure).
fn run_jewilo_new(home: &Path, goal_text: &str) -> std::process::Output {
    let mut c = std::process::Command::new(assert_cmd::cargo::cargo_bin("verifier-loop"));
    c.arg("NEW")
        .arg(goal_text)
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", stub_approve_path())
        .current_dir(home);
    c.output().expect("verifier-loop subprocess ran")
}

/// Read the per-goal completion.json, return as a JSON value.
fn read_completion(home: &Path, goal_id: &str) -> serde_json::Value {
    let p = goal::goal_dir(home, goal_id).join(consensus::COMPLETION_FILE);
    serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// #1 — spawn mints a per-verifier pubkey BEFORE launching each V*
// ---------------------------------------------------------------------------

#[test]
fn spawn_mints_pubkey_for_each_verifier_before_launch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 2, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    // A backend that does NOT touch the slot: just emits an ACP stream + exits.
    let script = write_script(
        dir.path(),
        "noop.sh",
        r#"#!/bin/sh
cat <<'EOF'
{"type":"session","id":"s-$VERIFIER_LOOP_VERIFIER_ID"}
{"type":"agent_end","messages":[],"willRetry":false}
EOF
"#,
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
    .expect("spawn round succeeds");

    for vid in ["v1", "v2"] {
        let p = verifier_dir(root, &goal_id, 1, vid).join(verdict::PUBKEY_FILE);
        assert!(p.exists(), "{vid} verifier-pubkey.json should be pinned at spawn time");
        let raw = fs::read_to_string(&p).unwrap();
        let file: verdict::VerifierPubkeyFile = serde_json::from_str(&raw).unwrap();
        // 64 hex chars = 32-byte Ed25519 verifying key.
        assert_eq!(file.pubkey.len(), 64, "{vid} pubkey is 64 hex chars");
        assert!(
            file.pubkey.chars().all(|c| c.is_ascii_hexdigit()),
            "{vid} pubkey is lowercase hex"
        );
        assert!(!file.minted_at.is_empty(), "{vid} mintedAt present");
    }
}

// ---------------------------------------------------------------------------
// #2 — distinct pubkeys across verifiers (fresh keypair per slot)
// ---------------------------------------------------------------------------

#[test]
fn spawn_distinct_pubkeys_across_verifiers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let config = serde_json::json!({
        "n": 1, "m": 2, "maxTurn": 3, "backend": "custom",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    let goal_id = seed_goal(root, "g", &config);

    let script = write_script(
        dir.path(),
        "noop.sh",
        r#"#!/bin/sh
cat <<'EOF'
{"type":"session","id":"s"}
{"type":"agent_end","messages":[],"willRetry":false}
EOF
"#,
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

    let v1 = fs::read_to_string(verifier_dir(root, &goal_id, 1, "v1").join(verdict::PUBKEY_FILE))
        .unwrap();
    let v2 = fs::read_to_string(verifier_dir(root, &goal_id, 1, "v2").join(verdict::PUBKEY_FILE))
        .unwrap();
    let pk1: verdict::VerifierPubkeyFile = serde_json::from_str(&v1).unwrap();
    let pk2: verdict::VerifierPubkeyFile = serde_json::from_str(&v2).unwrap();
    assert_ne!(
        pk1.pubkey, pk2.pubkey,
        "each verifier gets its own fresh keypair (no shared key)"
    );
}

// ---------------------------------------------------------------------------
// #3 — closed loop (jewilo NEW + stub_approve) yields SIGNED verdicts
// ---------------------------------------------------------------------------

#[test]
fn closed_loop_produces_signed_verdicts() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_worktree(home, 2, 2);

    let out = run_jewilo_new(home, "ship the feature");
    assert!(
        out.status.success(),
        "jewilo NEW should reach consensus; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Discover the goal id (single goal dir under goals/).
    let goals_dir = home.join("goals");
    let goal_id = fs::read_dir(&goals_dir)
        .unwrap()
        .next()
        .and_then(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .expect("one goal dir exists");

    for vid in ["v1", "v2"] {
        // (a) pinned pubkey present.
        let pinned = verdict::read_pinned_pubkey(home, &goal_id, vid, 1)
            .expect("pinned pubkey reads")
            .expect("pubkey was pinned at spawn");

        // (b) verdict.json is signed.
        let rec = verdict::read_verdict(home, &goal_id, vid, 1).expect("verdict reads");
        assert_eq!(rec.status, verdict::VerdictStatus::Approve, "{vid} APPROVE");
        let sig = rec.signature.as_ref().expect("{vid} signature present");
        assert_eq!(sig.len(), 128, "{vid} signature is 128 hex (64-byte Ed25519)");
        let pkid = rec.pubkey_id.as_ref().expect("{vid} pubkeyId present");
        assert_eq!(pkid.len(), 16, "{vid} pubkeyId is 16 hex");

        // (c) the signature verifies against the pinned key.
        verdict::verify_record(&rec, Some(&pinned), &goal_id, vid, 1)
            .expect("{vid} signature verifies against pinned pubkey");
    }
}

// ---------------------------------------------------------------------------
// #4 — closed loop writes receipt-log entries (one per APPROVE), chain verifies
// ---------------------------------------------------------------------------

#[test]
fn closed_loop_writes_receipt_log_entries() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_worktree(home, 2, 2);

    let out = run_jewilo_new(home, "ship the feature");
    assert!(out.status.success(), "jewilo NEW should pass");

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .and_then(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .unwrap();

    let log_path = receipt::receipt_log_path(home, &goal_id);
    assert!(log_path.exists(), "receipt-log.jsonl exists");

    let entries = receipt::read_receipt_log(home, &goal_id).expect("log parses");
    assert_eq!(entries.len(), 2, "exactly one entry per APPROVE (m=2)");
    for e in &entries {
        assert_eq!(e.status, "APPROVE", "entry status");
        assert_eq!(e.kind, "approve", "entry kind");
    }

    receipt::verify_chain(&entries).expect("chain verifies end-to-end");

    let head = receipt::read_receipt_head(home, &goal_id);
    assert_eq!(
        head,
        entries.last().unwrap().entry_hash,
        "read_receipt_head returns the last entry_hash"
    );
}

// ---------------------------------------------------------------------------
// #5 — completion.json hash inputs fold in the receipt head
//
// RED today: the consensus hash does NOT yet include the receipt head. This test
// recomputes the hash WITH the head and asserts it equals the stored fullDigest.
// GREEN-consensus task makes it pass.
// ---------------------------------------------------------------------------

#[test]
fn closed_loop_completion_hash_inputs_include_receipt_head() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_worktree(home, 2, 2);

    let out = run_jewilo_new(home, "ship the feature");
    assert!(out.status.success(), "jewilo NEW should pass");

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .and_then(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .unwrap();

    let completion = read_completion(home, &goal_id);
    let stored_full = completion["fullDigest"].as_str().expect("fullDigest present").to_string();
    let matched_at = completion["matchedAt"].as_str().expect("matchedAt present").to_string();
    let round = completion["roundNumber"].as_u64().expect("roundNumber present") as u32;

    // Recompute the inputs we can read back.
    let salt = store::salt_in(home).expect("salt reads");
    let sig_record: goal::SignatureRecord = serde_json::from_str(&fs::read_to_string(
        goal::goal_dir(home, &goal_id).join(goal::SIGNATURE_FILE),
    )
    .unwrap())
    .unwrap();

    // Reconstruct matching verdicts from completion.json (already canonical order).
    let matching: Vec<consensus::MatchingVerdict> = completion["matchingVerdicts"]
        .as_array()
        .expect("matchingVerdicts array")
        .iter()
        .map(|v| consensus::MatchingVerdict {
            verifier_id: v["verifierId"].as_str().unwrap().to_string(),
            registered_at: v["registeredAt"].as_str().unwrap().to_string(),
        })
        .collect();

    // Sanity: compute_hash (which now folds in the receipt head) reproduces the stored
    // digest. This proves our reconstructed inputs are correct.
    let head = receipt::read_receipt_head(home, &goal_id);
    let without_head = consensus::compute_hash(
        &salt,
        &goal_id,
        &sig_record.signature,
        round,
        &matching,
        &matched_at,
        &head,
    );
    assert_eq!(
        without_head.full_digest(),
        stored_full,
        "sanity: recompute matches stored digest (inputs reconstructed correctly)"
    );

    // The NEW contract: the digest must fold in the receipt head. Recompute WITH the
    // head appended to the canonical input string and assert it equals the stored
    // digest.
    assert!(!head.is_empty(), "receipt head is non-empty after m=2 approves");

    // Mirror consensus::compute_hash's input assembly, then append the head.
    // (The exact insertion point is the GREEN task's choice; this test pins the
    // contract that the head IS part of the hashed bytes by appending it.)
    let canon_json = serde_json::to_string(
        &matching
            .iter()
            .map(|m| {
                let mut map = std::collections::BTreeMap::new();
                map.insert("registeredAt", serde_json::Value::String(m.registered_at.clone()));
                map.insert("verifierId", serde_json::Value::String(m.verifier_id.clone()));
                serde_json::to_value(&map).unwrap()
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let input_with_head = format!(
        "{salt}{goal_id}{sig}{round}{canon_json}{matched_at}{head}",
        sig = sig_record.signature,
    );
    let recompute_with_head = hex::encode(Sha256::digest(input_with_head.as_bytes()));

    assert_eq!(
        recompute_with_head, stored_full,
        "completion hash MUST fold in the receipt head (verifier-spawn MODIFIED D3)"
    );
}

// ---------------------------------------------------------------------------
// #6 — stub_approve receives VERIFIER_LOOP_VERIFIER_SECRET and forwards it
//
// Proven indirectly: the only way `register_signed_approve` succeeds (and thus a
// signed verdict.json lands) is if the secret arrived at jewije. A signed verdict
// with a verifying pubkeyId therefore proves the env was injected + forwarded.
// ---------------------------------------------------------------------------

#[test]
fn stub_approve_receives_secret_env() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_worktree(home, 1, 1);

    let out = run_jewilo_new(home, "ship the feature");
    assert!(out.status.success(), "jewilo NEW should pass");

    let goal_id = fs::read_dir(home.join("goals"))
        .unwrap()
        .next()
        .and_then(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .unwrap();

    // The verdict must be SIGNED — which only happens if VERIFIER_LOOP_VERIFIER_SECRET
    // reached jewije via the stub. An unsigned APPROVE (or a null verdict) means the
    // secret was never injected/forwarded.
    let rec = verdict::read_verdict(home, &goal_id, "v1", 1).expect("verdict reads");
    assert_eq!(rec.status, verdict::VerdictStatus::Approve);
    assert!(
        rec.signature.is_some(),
        "verdict is signed ⇒ secret reached jewije via the stub backend"
    );
    assert!(
        rec.pubkey_id.is_some(),
        "signed verdict carries a pubkeyId bound to the pinned key"
    );
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
