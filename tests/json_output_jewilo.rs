// add-json-output-mode — RED tests (groups 3, 4, 5, 7) for `jewilo --json` machine-readable
// output. These are END-TO-END tests that invoke the compiled `verifier-loop` binary with the
// `--json` global flag.
//
// RED phase: written first, against the spec, BEFORE any `--json` implementation exists. Every
// assertion here is expected to FAIL until a DIFFERENT comrade lands the GREEN implementation
// (tasks.md groups 3/4/5/7). Source of truth:
//   * openspec/changes/add-json-output-mode/specs/json-output/spec.md
//   * openspec/changes/add-json-output-mode/tasks.md groups 3, 4, 5, 7
//
// Determinism strategy is COPIED EXACTLY from tests/cli_e2e.rs and tests/health_cooldown_e2e.rs:
//   * a STUB backend (`backend: "stub"` in config.json + `VERIFIER_LOOP_BACKEND_CMD=<abs script>`)
//   * the stub emits a fixed ACP stream (session + agent_end) then runs the built
//     `verifier-verdict` to register APPROVE / REJECT
//   * a tempdir HOME doubles as the git work tree (the frozen snapshot requires a work tree)
//   * No real `pi`, no network.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::Value;

// ---------------------------------------------------------------------------
// Harness — copied verbatim from tests/cli_e2e.rs (same proven pattern).
// ---------------------------------------------------------------------------

fn bin(name: &str) -> PathBuf {
    assert_cmd::cargo::cargo_bin(name)
}

fn verdict_bin_path() -> PathBuf {
    bin("verifier-verdict")
}

/// Run `verifier-loop` as a raw subprocess, returning full output regardless of exit status.
fn run_vl_raw(
    cwd: &Path,
    home: &Path,
    stub: &Path,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> std::process::Output {
    let mut c = std::process::Command::new(bin("verifier-loop"));
    c.args(args)
        .env("VERIFIER_LOOP_HOME", home)
        .env("VERIFIER_LOOP_BACKEND_CMD", stub)
        .current_dir(cwd);
    for (k, v) in extra_env {
        c.env(k, v);
    }
    c.output().expect("verifier-loop subprocess ran")
}

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

/// Default stub: emits the ACP stream then registers approve (or reject when
/// `$VERIFIER_LOOP_STUB_VERDICT=reject`). Mirrors tests/cli_e2e.rs `stub_script`.
fn stub_script(dir: &Path) -> PathBuf {
    let verdict = verdict_bin_path();
    let v = verdict.to_string_lossy();
    write_script(
        dir,
        "stub_backend.sh",
        &format!(
            r#"#!/bin/sh
cat <<'ACP'
{{"type":"session","id":"stub-session-id"}}
{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"stub final output"}}]}}],"willRetry":false}}
ACP
case "${{VERIFIER_LOOP_STUB_VERDICT:-approve}}" in
  reject) "{v}" reject --notes "stub rejection: no proof produced" ;;
  *)      "{v}" approve ;;
esac
"#,
        ),
    )
}

/// A stub that picks its verdict per verifier id via `$VERIFIER_LOOP_STUB_VERDICT_MAP`
/// (a `vid=verdict` list joined by commas, e.g. `v1=reject,v3=reject`). Verifiers not listed
/// approve. Used to drive a sorted-rejection scenario with notes from v3 + v1.
fn stub_map_script(dir: &Path) -> PathBuf {
    let verdict = verdict_bin_path();
    let v = verdict.to_string_lossy();
    write_script(
        dir,
        "stub_map.sh",
        &format!(
            r#"#!/bin/sh
cat <<'ACP'
{{"type":"session","id":"stub-session-id"}}
{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"stub final output"}}]}}],"willRetry":false}}
ACP
# Default approve; per-id override from the map env.
verdict_for() {{
  key="$1="
  map="${{VERIFIER_LOOP_STUB_VERDICT_MAP:-}}"
  # walk comma-separated vid=verdict pairs
  ifs_save="$IFS"
  IFS=','
  for pair in $map; do
    case "$pair" in
      "$key"reject) echo reject; IFS="$ifs_save"; return ;;
      "$key"approve) echo approve; IFS="$ifs_save"; return ;;
    esac
  done
  IFS="$ifs_save"
  echo approve
}}
vid="${{VERIFIER_LOOP_VERIFIER_ID:-v1}}"
case "$(verdict_for "$vid")" in
  reject) "{v}" reject --notes "$vid rejection: missing proof" ;;
  *)      "{v}" approve ;;
esac
"#,
        ),
    )
}

/// Seed a git work tree at `dir` with `config.json`. Returns nothing — the caller passes the
/// chosen stub explicitly. Mirrors tests/health_cooldown_e2e.rs `seed_workdir`.
fn seed_workdir(dir: &Path, n: u32, m: u32) {
    let git_ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["init", "-q"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(git_ok, "git init failed in tempdir");
    let cfg = serde_json::json!({
        "n": n, "m": m, "maxTurn": 3, "backend": "stub",
        "gitDiffMaxChars": 1000, "verifierTimeoutSec": 10
    });
    fs::write(dir.join("config.json"), cfg.to_string()).unwrap();
    fs::write(dir.join(".gitkeep"), "").unwrap();
    for (k, val) in [("user.email", "t@e.com"), ("user.name", "T")] {
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", k, val])
            .status();
    }
    let _ = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["add", "."])
        .status();
    let _ = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", "seed"])
        .status();
}

/// Extract the goalId printed by `NEW` from the legacy `goalId: <id>` stdout line.
fn goal_id_from_legacy_stdout(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("goalId: ").map(|s| s.trim().to_string())
    })
}

/// Count the number of top-level (depth-0) JSON root objects in `stdout`. The `--json`
/// contract requires exactly ONE root object per process invocation regardless of how many
/// internal phases ran. We scan brace depth: a `{` that moves depth 0 -> 1 opens a root.
fn count_top_level_json_roots(stdout: &str) -> usize {
    let mut depth: i64 = 0;
    let mut roots = 0usize;
    let mut in_string = false;
    let mut prev = '\0';
    for ch in stdout.chars() {
        if in_string {
            if ch == '"' && prev != '\\' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => in_string = true,
                '{' => {
                    if depth == 0 {
                        roots += 1;
                    }
                    depth += 1;
                }
                '}' => {
                    if depth > 0 {
                        depth -= 1;
                    }
                }
                _ => {}
            }
        }
        prev = ch;
    }
    roots
}

/// Parse the single JSON object that `--json` must emit on stdout. Asserts there is exactly
/// one top-level root object (the single-object invariant, design D0) before returning it.
fn parse_json_envelope(stdout: &str) -> Value {
    let roots = count_top_level_json_roots(stdout);
    assert_eq!(
        roots, 1,
        "expected exactly ONE JSON root object on stdout, found {roots}.\nstdout:\n{stdout}"
    );
    // Find the root object substring and parse it.
    let start = stdout.find('{').expect("at least one '{' present");
    let mut depth: i64 = 0;
    let mut in_string = false;
    let mut prev = '\0';
    let mut end = start;
    for (i, ch) in stdout[start..].char_indices() {
        if in_string {
            if ch == '"' && prev != '\\' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        prev = ch;
    }
    let slice = &stdout[start..end];
    serde_json::from_str(slice).unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\n---\n{slice}"))
}

// ===========================================================================
// Group 3 — `jewilo NEW` / `RESUME` consensus-passed + cooldown paths under `--json`
// (tasks.md 3.1, 3.2, 3.4, 3.5)
// ===========================================================================

/// 3.1 RED — `jewilo --json NEW "<goal>"` against an approving stub (n=m=2) reaches consensus;
/// stdout is exactly one JSON envelope with ok:true, command:"new", goalId, round:1,
/// status:"consensus-passed", hash, fullDigest; stdout does NOT contain the legacy `goalId:`
/// line or a bare non-JSON hash; exit 0.
#[test]
fn jewilo_new_json_consensus_passed_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 2);
    let stub = stub_script(home);

    let out = run_vl_raw(home, home, &stub, &["--json", "NEW", "ship the json feature"], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "NEW --json must exit 0; stderr:\n{stderr}");

    let env = parse_json_envelope(&stdout);
    assert_eq!(env["ok"], true, "envelope.ok must be true: {env}");
    assert_eq!(
        env["command"].as_str(),
        Some("new"),
        "envelope.command must be \"new\": {env}"
    );
    assert!(
        env["goalId"].as_str().is_some_and(|s| !s.is_empty()),
        "envelope.goalId must be set: {env}"
    );
    assert_eq!(
        env["round"].as_u64(),
        Some(1),
        "envelope.round must be 1: {env}"
    );
    assert_eq!(
        env["status"].as_str(),
        Some("consensus-passed"),
        "envelope.status must be \"consensus-passed\": {env}"
    );
    let hash = env["hash"].as_str().expect("hash present");
    assert!(
        hash.len() == 15
            && hash[6..7] == *"-"
            && hash[..6].chars().all(|c: char| c.is_ascii_digit())
            && hash[7..].chars().all(|c: char| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "hash must be mmddyy-XXXXXXXX: {hash}"
    );
    let full = env["fullDigest"]
        .as_str()
        .expect("fullDigest present");
    assert_eq!(
        full.len(),
        64,
        "fullDigest must be 64 hex chars: {full}"
    );
    assert!(
        full.chars().all(|c: char| c.is_ascii_hexdigit()),
        "fullDigest must be hex: {full}"
    );
    assert!(
        env.get("error").is_none() || env["error"].is_null(),
        "no error field on a passing round: {env}"
    );

    // Legacy lines must NOT appear on stdout under --json.
    assert!(
        !stdout.contains("goalId:"),
        "legacy `goalId:` line must NOT appear under --json: {stdout}"
    );
    // No bare 15-char hash line (the hash must live inside the JSON object only).
    let bare_hash_line = stdout.lines().any(|l| {
        let l = l.trim();
        l.len() == 15
            && l[6..7] == *"-"
            && l[..6].chars().all(|c: char| c.is_ascii_digit())
            && l[7..]
                .chars()
                .all(|c: char| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    });
    assert!(
        !bare_hash_line,
        "no bare non-JSON hash line on stdout under --json: {stdout}"
    );
}

/// 3.2 RED — the SAME approving round WITHOUT `--json` must be byte-identical to the legacy
/// behavior: stdout first line is `goalId: <id>` and last line is the bare short hash; no JSON
/// object appears.
#[test]
fn jewilo_new_default_is_byte_identical_to_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 2);
    let stub = stub_script(home);

    let out = run_vl_raw(home, home, &stub, &["NEW", "ship the json feature"], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "NEW must exit 0; stderr:\n{stderr}");

    let first = stdout.lines().next().unwrap_or("").trim();
    assert!(
        first.starts_with("goalId: "),
        "default first line must be `goalId: <id>`: {first:?}\n{stdout}"
    );
    let last = stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .last()
        .unwrap_or("");
    assert!(
        last.len() == 15
            && last[6..7] == *"-"
            && last[..6].chars().all(|c: char| c.is_ascii_digit())
            && last[7..]
                .chars()
                .all(|c: char| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "default last non-empty line must be the bare short hash: {last:?}\n{stdout}"
    );
    // No JSON object on stdout in legacy mode.
    assert_eq!(
        count_top_level_json_roots(&stdout),
        0,
        "default mode must NOT emit a JSON object: {stdout}"
    );
}

/// 3.4 RED — `jewilo --json NEW "<goal>"` while the store is in cooldown (>3 unhealthy events
/// in the last hour) → envelope ok:true, status:"cooldown-fallback", hash == `<mmddyy>-ffffff`;
/// exit 0. The human cooldown notice stays on stderr only.
#[test]
fn jewilo_new_json_cooldown_fallback_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 2);
    let stub = stub_script(home);

    // Seed >3 unhealthy events at "now" (the cooldown threshold is "more than 3" in 1h).
    let now = chrono::Utc::now().to_rfc3339();
    let mut log = String::new();
    for _ in 0..4 {
        log.push_str(&format!("{{\"event\":\"unhealthy\",\"at\":\"{now}\"}}\n"));
    }
    fs::write(home.join("health.jsonl"), log).unwrap();

    let out = run_vl_raw(home, home, &stub, &["--json", "NEW", "ship during cooldown"], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "cooldown-fallback must exit 0; stderr:\n{stderr}"
    );

    let env = parse_json_envelope(&stdout);
    assert_eq!(env["ok"], true, "cooldown envelope.ok must be true: {env}");
    assert_eq!(
        env["status"].as_str(),
        Some("cooldown-fallback"),
        "envelope.status must be \"cooldown-fallback\": {env}"
    );
    let hash = env["hash"].as_str().expect("fallback hash present");
    // `<mmddyy>-ffffff` (6 digits, hyphen, six f's).
    assert!(
        hash.len() == 13
            && hash[6..7] == *"-"
            && hash[..6].chars().all(|c: char| c.is_ascii_digit())
            && hash[7..] == *"ffffff",
        "cooldown hash must be `<mmddyy>-ffffff`: {hash}"
    );

    // Human-readable cooldown notice must be on STDERR only. Note: the JSON envelope's
    // status value "cooldown-fallback" legitimately contains the substring "cooldown",
    // and the envelope itself is on stdout by design. So we only assert that the
    // distinctive human-readable phrases (which never appear inside the envelope) are
    // absent from stdout and present on stderr.
    assert_eq!(
        count_top_level_json_roots(&stdout),
        1,
        "stdout must be exactly one JSON envelope: {stdout}"
    );
    assert!(
        !stdout.contains("no verifiers spawned"),
        "human cooldown phrase must NOT appear on stdout (stderr-only): {stdout}"
    );
    assert!(
        !stdout.contains("unhealthy verifier runs"),
        "human cooldown phrase must NOT appear on stdout (stderr-only): {stdout}"
    );
    assert!(
        stderr.to_lowercase().contains("cooldown") || stderr.to_lowercase().contains("unhealthy"),
        "human cooldown notice must be on stderr: {stderr}"
    );
}

/// 3.5 RED — the on-disk `completion.json` written by a `--json` run carries the SAME
/// `hash` and `fullDigest` that the stdout JSON envelope reports. This is the correct,
/// testable expression of the spec invariant "the `--json` flag is a pure output-formatting
/// concern; it does NOT alter the on-disk `completion.json`, the hash inputs, or the
/// computed hash" (openspec change `add-json-output-mode`, spec scenario "completion.json
/// byte-identical with and without --json").
///
/// # Why cross-run byte-identity is NOT testable in this harness
///
/// The original form of this test asserted two INDEPENDENT `NEW` runs (two tempdir homes)
/// produce byte-identical `completion.json` files. That is mathematically impossible in the
/// current harness: the completion hash inputs (see `src/consensus/mod.rs`) are
/// `salt + goalId + goalSignature + round + canonicalJSON(matchingVerdicts) + matchedAtISO
/// + receiptHead`, where `salt` is 32 random bytes read from `/dev/urandom`
/// (`src/store/salt.rs`, no env override), `goalId` is `Uuid::new_v4()` (`src/goal/mod.rs`,
/// no override), and `matchedAt` is wall-clock time. Two independent runs therefore ALWAYS
/// differ on at least `salt`, `goalId`, and `matchedAt`, so the byte-identical assertion can
/// never hold. The spec intent survives a SINGLE run, however: a single `--json` run writes
/// a `completion.json` whose `hash`/`fullDigest` must EQUAL the values the envelope reports
/// — proving the `--json` path computes and reports the REAL hash (not a json-mode-altered
/// hash) and writes the REAL `completion.json`.
///
/// # Restoring the stronger test (future work)
///
/// If a future change adds deterministic salt + clock injection (e.g. an env var that pins
/// `/dev/urandom` reads and the matched-at clock), the original cross-run byte-identity
/// assertion can be restored as a stronger test by running twice in two tempdirs that share
/// those injected values. That gap is intentionally left open here.
///
/// # Companion verification
///
/// The structural companion `hash_path_code_is_json_agnostic` in
/// `tests/json_output_envelope.rs` asserts the SOURCE of the hash/salt/receipt/store code
/// contains NO reference to the json output path — proving the --json flag structurally
/// cannot reach the hash inputs.
#[test]
fn completion_json_byte_identical_with_and_without_json() {
    // ONE tempdir home + an approving stub. We drive a single --json run and then prove the
    // on-disk completion.json agrees with the stdout envelope on hash + fullDigest.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 2);
    let stub = stub_script(home);

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["--json", "NEW", "stable completion artifact goal"],
        &[],
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "--json NEW must reach consensus; stderr:\n{stderr}"
    );

    // Parse the single stdout JSON envelope.
    let env = parse_json_envelope(&stdout);
    let env_hash = env["hash"]
        .as_str()
        .expect("envelope.hash present")
        .to_string();
    let env_full = env["fullDigest"]
        .as_str()
        .expect("envelope.fullDigest present")
        .to_string();

    // The hash must be the REAL consensus form `mmddyy-XXXXXXXX` (6 digits, hyphen, 8 hex),
    // NOT the cooldown fallback `<mmddyy>-ffffff` (which would be len 13 with suffix ffffff).
    // This proves the --json path did not silently degrade to a fallback hash.
    assert_eq!(
        env_hash.len(),
        15,
        "real hash must be `mmddyy-XXXXXXXX` (15 chars), got len {}: {env_hash}",
        env_hash.len()
    );
    assert_eq!(
        &env_hash[6..7],
        "-",
        "hash must have a hyphen at index 6: {env_hash}"
    );
    assert!(
        env_hash[..6].chars().all(|c: char| c.is_ascii_digit()),
        "hash prefix must be 6 ascii digits: {env_hash}"
    );
    assert!(
        env_hash[7..]
            .chars()
            .all(|c: char| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "hash suffix must be 8 lowercase hex chars: {env_hash}"
    );
    assert_ne!(
        &env_hash[7..], "ffffff",
        "hash must NOT be the cooldown fallback (`ffffff` suffix): {env_hash}"
    );

    // Locate <home>/goals/<goalId>/completion.json using the goalId from the envelope
    // (same lookup strategy the other group-3 tests rely on for the per-goal dir).
    let goal_id = env["goalId"]
        .as_str()
        .expect("envelope.goalId present")
        .to_string();
    let completion_path = home
        .join("goals")
        .join(&goal_id)
        .join("completion.json");
    assert!(
        completion_path.is_file(),
        "completion.json must be written at {completion_path:?}"
    );

    let completion_bytes = fs::read(&completion_path)
        .unwrap_or_else(|e| panic!("completion.json readable: {e}"));
    let completion: Value = serde_json::from_slice(&completion_bytes)
        .unwrap_or_else(|e| panic!("completion.json is valid JSON: {e}"));
    let comp_hash = completion["hash"]
        .as_str()
        .expect("completion.json.hash present")
        .to_string();
    let comp_full = completion["fullDigest"]
        .as_str()
        .expect("completion.json.fullDigest present")
        .to_string();

    // The core invariant: the --json envelope reports the SAME hash + fullDigest that the
    // on-disk completion.json carries. If these diverge, the --json path altered the hash
    // inputs or computed a different hash, violating the "--json is a no-op for
    // completion.json" spec invariant.
    assert_eq!(
        env_hash, comp_hash,
        "--json envelope.hash must equal completion.json.hash: \
         env={env_hash} completion={comp_hash}"
    );
    assert_eq!(
        env_full, comp_full,
        "--json envelope.fullDigest must equal completion.json.fullDigest: \
         env={env_full} completion={comp_full}"
    );
}

// ===========================================================================
// Group 4 — `jewilo` rejection path under `--json` (tasks.md 4.1)
// ===========================================================================

/// 4.1 RED — stub backend REJECTs from v1 and v3 (n=2, m=3 so v1,v2,v3 are spawned; v2
/// does not reject but the round fails n/m consensus); the `jewilo --json NEW` envelope has
/// ok:false, status:"rejected", and `rejection.rejectNotes` sorted by verifierId ascending
/// (v1 before v3). Exit non-zero.
#[test]
fn jewilo_new_json_rejection_envelope_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 3);
    let stub = stub_map_script(home);

    let out = run_vl_raw(
        home,
        home,
        &stub,
        &["--json", "NEW", "goal that will be rejected"],
        &[(
            "VERIFIER_LOOP_STUB_VERDICT_MAP",
            "v1=reject,v3=reject",
        )],
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !out.status.success(),
        "rejected round must exit non-zero; stderr:\n{stderr}"
    );

    let env = parse_json_envelope(&stdout);
    assert_eq!(env["ok"], false, "rejection envelope.ok must be false: {env}");
    assert_eq!(
        env["status"].as_str(),
        Some("rejected"),
        "envelope.status must be \"rejected\": {env}"
    );

    let rejection = env
        .get("rejection")
        .and_then(|r| r.as_object())
        .expect("rejection object present");

    // rejectNotes sorted by verifierId ascending.
    let notes = rejection
        .get("rejectNotes")
        .and_then(|n| n.as_array())
        .expect("rejection.rejectNotes array present");
    let note_ids: Vec<String> = notes
        .iter()
        .map(|n| {
            // Each note entry is a tuple-array `[verifierId, note]` (locked by the
            // envelope unit test). Fall back to string/object shapes for robustness.
            if let Some(arr) = n.as_array() {
                if arr.len() >= 2 {
                    return arr[0].as_str().unwrap_or_default().to_string();
                }
            }
            if let Some(s) = n.as_str() {
                s.to_string()
            } else if let Some(o) = n.as_object() {
                o.get("verifierId")
                    .or_else(|| o.get("verifier_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            } else {
                String::new()
            }
        })
        .collect();
    assert!(
        note_ids.len() >= 2,
        "rejectNotes must contain at least two entries (v1,v3): {note_ids:?}"
    );
    let mut sorted = note_ids.clone();
    sorted.sort();
    assert_eq!(
        note_ids, sorted,
        "rejectNotes must be sorted by verifierId ascending: {note_ids:?}"
    );
    assert!(
        note_ids.first().is_some_and(|v| v == "v1"),
        "first rejectNote must be v1: {note_ids:?}"
    );

    // nullVerifiers + signatureFailures arrays present (may be empty).
    assert!(
        rejection.get("nullVerifiers").and_then(|v| v.as_array()).is_some(),
        "rejection.nullVerifiers array must be present: {env}"
    );
    assert!(
        rejection
            .get("signatureFailures")
            .and_then(|v| v.as_array())
            .is_some(),
        "rejection.signatureFailures array must be present: {env}"
    );
}

// ===========================================================================
// Group 5 — `jewilo RECOVER` + `STATUS` under `--json` (tasks.md 5.2, 5.3, 5.4)
// ===========================================================================

/// 5.2 RED — `jewilo --json RECOVER <id>` on a round that times out with null slots still
/// present → envelope ok:false, status:"recover-null-after-timeout"; exit non-zero; the human
/// guidance remains on stderr only.
#[test]
fn jewilo_recover_json_null_after_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 2);

    // A stub that only registers v1's verdict; v2 stays null forever (orphan died).
    let only_v1 = {
        let verdict = verdict_bin_path();
        let v = verdict.to_string_lossy();
        write_script(
            home,
            "stub_only_v1.sh",
            &format!(
                r#"#!/bin/sh
cat <<'ACP'
{{"type":"session","id":"stub-session-id"}}
{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"stub final output"}}]}}],"willRetry":false}}
ACP
if [ "$VERIFIER_LOOP_VERIFIER_ID" = "v1" ]; then
  "{v}" approve
fi
"#,
            ),
        )
    };

    // NEW itself fails (no 2/2 consensus) — expected; goalId is still printed.
    let out = run_vl_raw(home, home, &only_v1, &["NEW", "dead-null round goal"], &[]);
    let goal_id = goal_id_from_legacy_stdout(&String::from_utf8_lossy(&out.stdout))
        .expect("goalId must be printed even on round failure");

    let out = run_vl_raw(home, home, &only_v1, &["--json", "RECOVER", &goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !out.status.success(),
        "recover-null-after-timeout must exit non-zero; stderr:\n{stderr}"
    );

    let env = parse_json_envelope(&stdout);
    assert_eq!(env["ok"], false, "recover-null envelope.ok must be false: {env}");
    assert_eq!(
        env["status"].as_str(),
        Some("recover-null-after-timeout"),
        "envelope.status must be \"recover-null-after-timeout\": {env}"
    );

    // Human-readable RESUME guidance stays on stderr only.
    assert!(
        stderr.to_lowercase().contains("resume"),
        "recover-null guidance must point to RESUME on stderr: {stderr}"
    );
    let stdout_has_resume = stdout.to_lowercase().contains("resume");
    assert!(
        !stdout_has_resume,
        "recover guidance must NOT appear inside the stdout JSON object: {stdout}"
    );
}

/// 5.3 RED — `jewilo --json STATUS <id>` wraps the legacy STATUS body in the standard envelope:
/// ok:true, command:"status", goalId, round, state, needs, and verdicts preserved.
#[test]
fn jewilo_status_json_wraps_body_in_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 2);
    let stub = stub_script(home);

    let out = run_vl_raw(home, home, &stub, &["NEW", "status probe goal"], &[]);
    let goal_id = goal_id_from_legacy_stdout(&String::from_utf8_lossy(&out.stdout))
        .expect("goalId printed");

    let out = run_vl_raw(home, home, &stub, &["--json", "STATUS", &goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "STATUS --json must exit 0; stderr:\n{stderr}");

    let env = parse_json_envelope(&stdout);
    assert_eq!(env["ok"], true, "status envelope.ok must be true: {env}");
    assert_eq!(
        env["command"].as_str(),
        Some("status"),
        "envelope.command must be \"status\": {env}"
    );
    assert_eq!(env["goalId"].as_str(), Some(goal_id.as_str()));
    assert!(env["round"].as_u64().is_some(), "round present: {env}");
    assert!(env["state"].is_string(), "state present: {env}");
    assert!(env["needs"].is_string(), "needs present: {env}");
    // The slot/verdict breakdown must be preserved somewhere under the envelope (the spec
    // names the field `verdicts`).
    let has_verdicts = env.get("verdicts").and_then(|v| v.as_array()).is_some()
        || env.get("verdicts").and_then(|v| v.as_object()).is_some()
        || env.get("verdicts").is_some();
    assert!(
        has_verdicts,
        "envelope must preserve `verdicts` (slots) from the STATUS body: {env}"
    );
}

/// 5.4 RED — `jewilo STATUS <id>` WITHOUT `--json` is byte-identical to today: a bare JSON
/// object (round, state, needs, verdicts/slots) with NO `ok`/`command` envelope wrapper.
#[test]
fn jewilo_status_default_byte_identical_to_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 2);
    let stub = stub_script(home);

    let out = run_vl_raw(home, home, &stub, &["NEW", "status legacy goal"], &[]);
    let goal_id = goal_id_from_legacy_stdout(&String::from_utf8_lossy(&out.stdout))
        .expect("goalId printed");

    let out = run_vl_raw(home, home, &stub, &["STATUS", &goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "STATUS must exit 0; stderr:\n{stderr}");

    // The whole stdout must be a single bare JSON object (legacy body).
    let body: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("STATUS stdout must be a bare JSON object: {e}\n{stdout}"));
    assert!(body["round"].as_u64().is_some(), "round present: {body}");
    assert!(body["state"].is_string(), "state present: {body}");
    assert!(body["needs"].is_string(), "needs present: {body}");
    // No envelope wrapper.
    assert!(
        body.get("ok").is_none(),
        "legacy STATUS must NOT have an `ok` field: {body}"
    );
    assert!(
        body.get("command").is_none(),
        "legacy STATUS must NOT have a `command` field: {body}"
    );
}

// ===========================================================================
// Blocker A regression — RECOVER on an already-consensus goal MUST NOT emit any
// stdout under Human mode (legacy empty-stdout contract from origin/main, before
// add-json-output-mode). Only the stderr notice is allowed.
// ===========================================================================

/// RECOVER Done arm: reach consensus, then `jewilo RECOVER <id>` WITHOUT --json. stdout
/// must be EMPTY (byte-identical to legacy origin/main behavior); stderr carries the
/// "already reached consensus; use RESUME" notice; exit 0.
#[test]
fn jewilo_recover_default_mode_empty_stdout_on_done() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 2);
    let stub = stub_script(home);

    // Reach consensus on round 1.
    let out = run_vl_raw(home, home, &stub, &["NEW", "reach consensus then recover"], &[]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "seed NEW must reach consensus; stderr:\n{stderr}"
    );
    let goal_id = goal_id_from_legacy_stdout(&String::from_utf8_lossy(&out.stdout))
        .expect("goalId printed on NEW");

    // RECOVER on the already-consensus goal WITHOUT --json.
    let out = run_vl_raw(home, home, &stub, &["RECOVER", &goal_id], &[]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "RECOVER on a done goal must exit 0; stderr:\n{stderr}"
    );
    assert_eq!(
        stdout,
        "",
        "default-mode RECOVER on an already-consensus goal must emit NOTHING to stdout \
         (byte-identical to legacy origin/main); got: {stdout:?}"
    );
    // The legacy stderr notice must still appear.
    assert!(
        stderr.contains("already reached consensus"),
        "stderr must carry the already-consensus notice: {stderr}"
    );
    assert!(
        stderr.contains("RESUME"),
        "stderr must point the user at RESUME: {stderr}"
    );
}

// ===========================================================================
// Group 7 — Determinism + single-object invariants (tasks.md 7.1)
// ===========================================================================

/// 7.1 RED — an m=5 RESUME round reaching consensus under `--json` emits exactly ONE top-level
/// JSON root object, not one per verifier and not one per internal phase.
#[test]
fn jewilo_resume_m5_json_emits_exactly_one_object() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    seed_workdir(home, 2, 5);

    // Round 1: reject so we can RESUME into round 2 with m=5.
    let reject = stub_script(home);
    let out = run_vl_raw(
        home,
        home,
        &reject,
        &["NEW", "m5 consensus single-object goal"],
        &[("VERIFIER_LOOP_STUB_VERDICT", "reject")],
    );
    let goal_id = goal_id_from_legacy_stdout(&String::from_utf8_lossy(&out.stdout))
        .expect("goalId printed on round-1 reject");

    // Round 2: RESUME, approving, under --json.
    let approve = stub_script(home);
    let out = run_vl_raw(
        home,
        home,
        &approve,
        &["--json", "RESUME", &goal_id, "--fix", "added the missing proof"],
        &[],
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "m=5 RESUME must reach consensus; stderr:\n{stderr}"
    );

    let roots = count_top_level_json_roots(&stdout);
    assert_eq!(
        roots, 1,
        "m=5 RESUME under --json must emit exactly ONE JSON root object, found {roots}.\nstdout:\n{stdout}"
    );

    // And that one object is the standard envelope.
    let env = parse_json_envelope(&stdout);
    assert_eq!(env["ok"], true, "resume envelope.ok must be true: {env}");
}
