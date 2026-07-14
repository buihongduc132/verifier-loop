// add-json-output-mode — tasks.md §6 (json-output spec).
// RED phase: written FIRST, against the spec, before any implementation.
//
// Author: teammate red-jewije (RED only). A DIFFERENT teammate will write the GREEN
// implementation in `src/bin/verifier_verdict.rs` + `src/cli/json_output.rs`.
//
// These tests drive the BUILT `verifier-verdict` (`jewije`) binary under the new
// top-level `--json` flag and assert the machine-readable envelope contract from the
// `jewije verdict registration emits a JSON envelope under --json` requirement:
//
//   * approve / reject success  → one JSON object, ok:true, command, goalId,
//     verifierId, round, status:"verdict-registered"; NO legacy `Verdict registered`
//     on stdout; exit 0.
//   * failure under --json      → one JSON object ok:false + error string; exit
//     non-zero; no human-readable error text on stdout.
//   * default (no --json)       → byte-identical to legacy (`Verdict registered`).
//   * receipt-log append        → byte-identical with and without --json (D6: the
//     flag is a pure output layer; on-disk artifacts + hash inputs are unchanged).
//
// Identity resolution mirrors `tests/verdict.rs`: goalId / verifierId / round come
// from VERIFIER_LOOP_GOAL_ID / VERIFIER_LOOP_VERIFIER_ID / VERIFIER_LOOP_ROUND; the
// store root from VERIFIER_LOOP_HOME. The signing secret comes from
// VERIFIER_LOOP_VERIFIER_SECRET and must match the slot's pinned verifier-pubkey.json.

use std::fs;

use assert_cmd::Command;
use serde_json::Value;

use verifier_loop::crypto;
use verifier_loop::goal;
use verifier_loop::receipt;
use verifier_loop::verdict;

// ---------------------------------------------------------------------------
// Hermetic harness — copied from tests/verdict.rs.
// ---------------------------------------------------------------------------

// assert_cmd's `Command` inherits the parent env by default. When `cargo test` runs
// UNDER jewilo (e.g. as a verifier-loop verifier itself), the test process inherits
// jewilo-injected identity/secret/trace env vars that do NOT match the test's temp-dir
// goal. Scrub them so each subprocess only sees what each test sets explicitly.
const INHERITED_JEWILO_ENV: &[&str] = &[
    "VERIFIER_LOOP_VERIFIER_SECRET",
    "VERIFIER_LOOP_GOAL_ID",
    "VERIFIER_LOOP_VERIFIER_ID",
    "VERIFIER_LOOP_ROUND",
    // Trace id is receipt-log metadata; scrubbing it keeps the receipt byte-identity
    // test deterministic across invocations.
    "VERIFIER_LOOP_TRACE_ID",
];

/// Build a hermetic `verifier-verdict` Command: resolves the cargo binary AND scrubs
/// any jewilo-injected identity/secret/trace env vars inherited from the parent
/// process so the subprocess only sees what each test sets explicitly.
fn hermetic_verifier_cmd() -> Command {
    let mut cmd = Command::cargo_bin("verifier-verdict").expect("verifier-verdict cargo bin");
    for var in INHERITED_JEWILO_ENV {
        cmd.env_remove(var);
    }
    cmd
}

/// Create a goal under a fresh temp store root and pre-create the round-1 v1 verifier
/// slot with a null verdict placeholder (mirrors what the spawn layer does at spawn
/// time). No pinned pubkey → legacy unsigned regime. Returns (TempDir, goal_id).
fn fresh_goal_with_null_verdict(round: u32) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), "build it", None).unwrap();

    let vdir = verdict::verdict_path(dir.path(), &goal_id, "v1", round);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(vdir.join(verdict::VERDICT_FILE), r#"{"status":null}"#).unwrap();
    (dir, goal_id)
}

/// Create a goal + null verdict slot AND a pinned pubkey for v1, returning
/// (TempDir, goal_id, signing_key_hex). The signing key hex is what a verifier
/// process receives as VERIFIER_LOOP_VERIFIER_SECRET.
fn fresh_goal_with_pinned_v1(round: u32) -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let goal_id = goal::new(dir.path(), "build it", None).unwrap();

    let vdir = verdict::verdict_path(dir.path(), &goal_id, "v1", round);
    fs::create_dir_all(&vdir).unwrap();
    fs::write(vdir.join(verdict::VERDICT_FILE), r#"{"status":null}"#).unwrap();

    let sk = verdict::mint_and_pin_pubkey(dir.path(), &goal_id, "v1", round)
        .expect("mint_and_pin_pubkey must succeed on a fresh slot");
    let secret_hex = crypto::signing_key_to_hex(&sk);
    (dir, goal_id, secret_hex)
}

/// Parse the (trimmed) stdout of a --json invocation as exactly one JSON root object.
/// Fails loudly if stdout is empty or not a single JSON object.
fn parse_single_envelope(stdout: &str) -> Value {
    let trimmed = stdout.trim();
    assert!(
        !trimmed.is_empty(),
        "expected exactly one JSON envelope on stdout, got empty stdout"
    );
    let v: Value = serde_json::from_str(trimmed)
        .unwrap_or_else(|e| panic!("stdout is not a single JSON object: {e}\n--- stdout ---\n{stdout}"));
    assert!(
        v.is_object(),
        "envelope must be a single JSON root object (not array/NDJSON); got: {v}"
    );
    v
}

// ===========================================================================
// §6.1 — jewije --json approve success envelope
// ===========================================================================

/// `jewije --json approve` with valid identity env + a pinned slot whose secret is
/// supplied must emit exactly one JSON object: ok:true, command:"approve", goalId,
/// verifierId, round, status:"verdict-registered"; stdout must NOT contain the legacy
/// `Verdict registered` line; exit 0.
#[test]
fn jewije_approve_json_success_envelope() {
    let (dir, goal_id, secret) = fresh_goal_with_pinned_v1(1);

    let assert = hermetic_verifier_cmd()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret)
        .args(["--json", "approve"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        !stdout.contains("Verdict registered"),
        "legacy line must NOT appear on stdout under --json; got: {stdout}"
    );

    let env = parse_single_envelope(&stdout);
    assert_eq!(env["ok"], Value::Bool(true), "envelope: {env}");
    assert_eq!(env["command"], Value::String("approve".into()), "envelope: {env}");
    assert_eq!(
        env["goalId"], Value::String(goal_id.clone()),
        "envelope goalId must match env goal id: {env}"
    );
    assert_eq!(env["verifierId"], Value::String("v1".into()), "envelope: {env}");
    assert_eq!(env["round"], Value::Number(1.into()), "envelope: {env}");
    assert_eq!(
        env["status"], Value::String("verdict-registered".into()),
        "envelope status must be verdict-registered: {env}"
    );
}

// ===========================================================================
// §6.2 — jewije reject --notes "..." --json success envelope
// ===========================================================================

/// `jewije reject --notes "broken" --json` must emit one envelope: command:"reject",
/// status:"verdict-registered", ok:true; exit 0.
#[test]
fn jewije_reject_json_success_envelope() {
    let (dir, goal_id, secret) = fresh_goal_with_pinned_v1(1);

    let assert = hermetic_verifier_cmd()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret)
        .args(["reject", "--notes", "broken", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        !stdout.contains("Verdict registered"),
        "legacy line must NOT appear on stdout under --json; got: {stdout}"
    );

    let env = parse_single_envelope(&stdout);
    assert_eq!(env["ok"], Value::Bool(true), "envelope: {env}");
    assert_eq!(
        env["command"], Value::String("reject".into()),
        "envelope command must reflect the invoked subcommand: {env}"
    );
    assert_eq!(
        env["status"], Value::String("verdict-registered".into()),
        "envelope status must be verdict-registered: {env}"
    );
    assert_eq!(env["round"], Value::Number(1.into()), "envelope: {env}");
}

// ===========================================================================
// §6.3 — default (no --json) success is byte-identical to legacy
// ===========================================================================

/// `jewije approve` WITHOUT --json must keep the legacy contract: stdout is exactly
/// `Verdict registered` and nothing else (no JSON object). Regression guard for D6.
#[test]
fn jewije_default_success_is_byte_identical() {
    let (dir, goal_id, secret) = fresh_goal_with_pinned_v1(1);

    let assert = hermetic_verifier_cmd()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret)
        .arg("approve")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert_eq!(
        stdout, "Verdict registered\n",
        "default (no --json) stdout must be exactly the legacy line; got: {stdout:?}"
    );
    // No JSON object may appear in the default path.
    assert!(
        !stdout.trim_start().starts_with('{'),
        "no JSON object may appear on stdout without --json; got: {stdout}"
    );
}

// ===========================================================================
// §6.4 — jewije --json notes-required error envelope
// ===========================================================================

/// `jewije reject --notes "" --json` (empty notes) must emit exactly one envelope
/// ok:false with an error string describing the notes-required failure; exit
/// non-zero; NO human-readable error text on stdout (human diagnostics stay on
/// stderr — the debugging channel).
#[test]
fn jewije_json_notes_required_error_envelope() {
    let (dir, goal_id, secret) = fresh_goal_with_pinned_v1(1);

    let assert = hermetic_verifier_cmd()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret)
        .args(["reject", "--notes", "", "--json"])
        .assert()
        .failure();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let env = parse_single_envelope(&stdout);
    assert_eq!(env["ok"], Value::Bool(false), "error envelope must have ok:false: {env}");
    let err = env["error"]
        .as_str()
        .expect("error envelope must carry an `error` string")
        .to_lowercase();
    assert!(
        err.contains("notes") || err.contains("reject"),
        "error must describe the notes-required failure; got: {err}"
    );

    // No human-readable error text on stdout — only the single JSON envelope.
    assert!(
        !stdout.contains("requires non-empty --notes"),
        "human error text must stay on stderr, not stdout; got: {stdout}"
    );
}

// ===========================================================================
// §6.5 — jewije --json unauthenticated (missing secret) error envelope
// ===========================================================================

/// A pinned-pubkey slot invoked under --json with NO VERIFIER_LOOP_VERIFIER_SECRET
/// must emit exactly one envelope ok:false with an error describing the missing
/// secret; exit non-zero.
#[test]
fn jewije_json_unauthenticated_error_envelope() {
    let (dir, goal_id, _secret) = fresh_goal_with_pinned_v1(1);

    let assert = hermetic_verifier_cmd()
        .env("VERIFIER_LOOP_HOME", dir.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_id)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        // VERIFIER_LOOP_VERIFIER_SECRET deliberately NOT set.
        .args(["--json", "approve"])
        .assert()
        .failure();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let env = parse_single_envelope(&stdout);
    assert_eq!(env["ok"], Value::Bool(false), "error envelope must have ok:false: {env}");
    let err = env["error"]
        .as_str()
        .expect("error envelope must carry an `error` string")
        .to_lowercase();
    assert!(
        err.contains("secret") || err.contains("unauthenticated") || err.contains("auth"),
        "error must describe the missing-secret / unauthenticated reason; got: {err}"
    );
}

// ===========================================================================
// §6.6 — receipt-log append is byte-identical with and without --json
// ===========================================================================

/// Drive `jewije approve` once with --json and once without on two EQUIVALENT slots
/// (same verifier id + round, same pinned keypair so `signed_by` matches, same empty
/// trace id so the optional `traceId` field is omitted from both lines). The two
/// appended receipt-log entries must be byte-identical — the --json flag is a pure
/// output-formatting layer (design D6) and must not perturb the on-disk receipt chain.
#[test]
fn receipt_log_byte_identical_with_and_without_json() {
    // ONE shared keypair pinned to BOTH goals so `signed_by` (the 16-hex pubkey id)
    // is identical across the two slots. Different keypairs would make the receipt
    // lines differ in `signedBy`, defeating the byte-identity check.
    let kp = crypto::generate_keypair();
    let vk_hex = crypto::verifying_key_to_hex(&kp.verifying);
    let secret_hex = crypto::signing_key_to_hex(&kp.signing);
    let pinned_file = verdict::VerifierPubkeyFile {
        pubkey: vk_hex,
        minted_at: "2026-07-15T00:00:00Z".to_string(),
    };
    let pinned_json = serde_json::to_string(&pinned_file).unwrap();

    // Goal A: drive approve WITH --json.
    let dir_a = tempfile::tempdir().unwrap();
    let goal_a = goal::new(dir_a.path(), "build it", None).unwrap();
    let slot_a = verdict::verdict_path(dir_a.path(), &goal_a, "v1", 1);
    fs::create_dir_all(&slot_a).unwrap();
    fs::write(slot_a.join(verdict::VERDICT_FILE), r#"{"status":null}"#).unwrap();
    fs::write(slot_a.join(verdict::PUBKEY_FILE), pinned_json.clone()).unwrap();

    let assert_a = hermetic_verifier_cmd()
        .env("VERIFIER_LOOP_HOME", dir_a.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_a)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret_hex)
        .args(["--json", "approve"])
        .assert()
        .success();
    // The --json path must still write the receipt entry (D6: on-disk artifacts
    // unaffected by the output flag).
    let _ = assert_a;

    // Goal B: drive approve WITHOUT --json, on an equivalent slot.
    let dir_b = tempfile::tempdir().unwrap();
    let goal_b = goal::new(dir_b.path(), "build it", None).unwrap();
    let slot_b = verdict::verdict_path(dir_b.path(), &goal_b, "v1", 1);
    fs::create_dir_all(&slot_b).unwrap();
    fs::write(slot_b.join(verdict::VERDICT_FILE), r#"{"status":null}"#).unwrap();
    fs::write(slot_b.join(verdict::PUBKEY_FILE), pinned_json).unwrap();

    hermetic_verifier_cmd()
        .env("VERIFIER_LOOP_HOME", dir_b.path())
        .env("VERIFIER_LOOP_GOAL_ID", &goal_b)
        .env("VERIFIER_LOOP_VERIFIER_ID", "v1")
        .env("VERIFIER_LOOP_ROUND", "1")
        .env("VERIFIER_LOOP_VERIFIER_SECRET", &secret_hex)
        .arg("approve")
        .assert()
        .success();

    // Read the first (and only) receipt entry from each goal's chain.
    let entries_a = receipt::read_receipt_log(dir_a.path(), &goal_a)
        .expect("goal A receipt log must have one entry after --json approve");
    let entries_b = receipt::read_receipt_log(dir_b.path(), &goal_b)
        .expect("goal B receipt log must have one entry after approve");
    assert_eq!(
        entries_a.len(),
        1,
        "goal A must have exactly one receipt entry"
    );
    assert_eq!(
        entries_b.len(),
        1,
        "goal B must have exactly one receipt entry"
    );

    // Byte-identity: the canonical JSON line written for each must match exactly.
    let line_a = serde_json::to_string(&entries_a[0]).unwrap();
    let line_b = serde_json::to_string(&entries_b[0]).unwrap();
    assert_eq!(
        line_a, line_b,
        "receipt-log entry must be byte-identical with and without --json (D6)\n\
         --json   : {line_a}\n\
         default  : {line_b}"
    );

    // And the chain integrity must hold for both.
    receipt::verify_chain(&entries_a).expect("goal A chain must verify");
    receipt::verify_chain(&entries_b).expect("goal B chain must verify");
}
