//! Goal lifecycle (tasks.md §3, goal-lifecycle spec).
//!
//! * `NEW "<goal>" [--context]`  → goalId (UUID v4), immutable `goal.json`, `signature.json`
//!   = `SHA256(salt + goalText + createdAt)` (D5), `rounds/` dir, current round = 1.
//! * `RESUME <id> [--fix "…"]`   → increment round, append note to
//!   `rounds/<round>/fix-notes.json` (append-only). `goal.json` and `signature.json` are
//!   byte-for-byte unchanged. The optional `--notes` flag appends goal-scoped notes to
//!   `goal-notes.json` (also append-only, also never a hash input).
//! * Missing store / missing goal → fail closed, no hash.
//!
//! All core functions take the store root explicitly (parallel-safe); env-resolving
//! wrappers are provided for CLI use.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::store;

/// Subdirectory under the store root holding all goals.
pub const GOALS_DIR: &str = "goals";
/// Per-goal immutable goal record.
pub const GOAL_FILE: &str = "goal.json";
/// Per-goal salted signature (input to the completion hash).
pub const SIGNATURE_FILE: &str = "signature.json";
/// Per-goal state file tracking the current round.
pub const STATE_FILE: &str = "state.json";
/// Rounds subdirectory.
pub const ROUNDS_DIR: &str = "rounds";
/// Append-only fix-notes within a round (written by RESUME).
pub const FIX_NOTES_FILE: &str = "fix-notes.json";
/// Goal-scoped append-only notes (written by `RESUME --notes`). Each note is a separate
/// line; the file lives alongside `goal.json` but is NEVER a signature or hash input —
/// `goal.json` and `signature.json` stay byte-for-byte immutable. There is NO command to
/// strip / remove / update notes; only append + load exist.
pub const GOAL_NOTES_FILE: &str = "goal-notes.json";

/// The immutable goal record written once at `NEW`.
///
/// On-disk keys are camelCase (`goalId`, `goalText`, `createdAt`) per the goal-lifecycle
/// spec — this is the audit/verifier-visible contract and the signature-input source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalRecord {
    pub goal_id: String,
    pub goal_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub created_at: String,
    /// Snapshot of the config at creation time (n, m, maxTurn, backend, …).
    pub config: store::Config,
}

/// The signature record: `SHA256(salt + goalText + createdAt)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureRecord {
    pub signature: String,
    pub algorithm: String,
}

const SIG_ALGO: &str = "SHA256(salt+goalText+createdAt)";

/// Per-goal state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateRecord {
    pub current_round: u32,
}

/// Compute the directory for a goal.
pub fn goal_dir(root: &Path, goal_id: &str) -> PathBuf {
    root.join(GOALS_DIR).join(goal_id)
}

/// Create a new immutable, signed goal. Returns the goalId.
///
/// Side effects: ensures the salt exists, creates `goals/<goalId>/{goal.json,
/// signature.json, state.json, rounds/}`. Current round is set to 1.
pub fn new(root: &Path, goal_text: &str, context: Option<&str>) -> Result<String, GoalError> {
    // Fail closed if the root cannot be used as a directory.
    let meta = fs::metadata(root).map_err(GoalError::StoreUnusable)?;
    if meta.is_file() {
        return Err(GoalError::StoreUnusable(io::Error::new(
            io::ErrorKind::InvalidInput,
            "store root is a file, not a directory",
        )));
    }
    store::ensure_home_at(root)?;

    // Salt is an input to the signature; ensure it exists (never printed).
    let salt = store::salt_in(root)?;

    let goal_id = uuid::Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let config = store::Config::load_in(root)?;

    let record = GoalRecord {
        goal_id: goal_id.clone(),
        goal_text: goal_text.to_string(),
        context: context.map(|s| s.to_string()),
        created_at: created_at.clone(),
        config,
    };

    let gdir = goal_dir(root, &goal_id);
    fs::create_dir_all(gdir.join(ROUNDS_DIR))?;

    // Write goal.json (pretty for audit readability).
    let goal_json = serde_json::to_string_pretty(&record)?;
    fs::write(gdir.join(GOAL_FILE), goal_json)?;

    // Write signature.json.
    let signature = compute_signature(&salt, goal_text, &created_at);
    let sig_record = SignatureRecord {
        signature,
        algorithm: SIG_ALGO.to_string(),
    };
    fs::write(
        gdir.join(SIGNATURE_FILE),
        serde_json::to_string_pretty(&sig_record)?,
    )?;

    // Write state.json (current round = 1).
    let state = StateRecord { current_round: 1 };
    fs::write(gdir.join(STATE_FILE), serde_json::to_string_pretty(&state)?)?;

    Ok(goal_id)
}

/// Resume a goal: increment the round, append fix notes (if any). `goal.json` and
/// `signature.json` are never touched. Returns the new round number.
pub fn resume(root: &Path, goal_id: &str, fix_notes: Option<&str>) -> Result<u32, GoalError> {
    let gdir = goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(GoalError::GoalNotFound);
    }

    // Load + increment round.
    let state_path = gdir.join(STATE_FILE);
    let mut state: StateRecord = if state_path.exists() {
        serde_json::from_str(&fs::read_to_string(&state_path)?)?
    } else {
        StateRecord { current_round: 1 }
    };
    state.current_round = state.current_round.saturating_add(1);
    let round = state.current_round;

    // Append fix notes to rounds/<round>/fix-notes.json (append-only array).
    if let Some(notes) = fix_notes {
        let round_dir = gdir.join(ROUNDS_DIR).join(round.to_string());
        fs::create_dir_all(&round_dir)?;
        let notes_path = round_dir.join(FIX_NOTES_FILE);
        let mut arr: serde_json::Value = if notes_path.exists() {
            serde_json::from_str(&fs::read_to_string(&notes_path)?)?
        } else {
            serde_json::json!({ "notes": [] })
        };
        if let Some(n) = arr.get_mut("notes").and_then(|v| v.as_array_mut()) {
            n.push(serde_json::Value::String(notes.to_string()));
        }
        fs::write(&notes_path, serde_json::to_string_pretty(&arr)?)?;
    }

    // Persist the incremented round.
    fs::write(&state_path, serde_json::to_string_pretty(&state)?)?;

    Ok(round)
}

/// Append each note in `notes` to the goal-scoped `goal-notes.json` (append-only across
/// calls, never overwriting). `goal.json` and `signature.json` are NEVER touched — notes
/// are metadata, not a signature or hash input. Fail-closed if the goal dir is missing.
///
/// On-disk shape: `{ "notes": ["line1", "line2", ...] }`. Each call pushes onto the
/// existing array (creating the file if absent). An empty `notes` slice is a no-op that
/// does NOT create the file (backward compatible: `RESUME` without `--notes` leaves no
/// goal-notes.json behind).
pub fn append_notes(root: &Path, goal_id: &str, notes: &[String]) -> Result<(), GoalError> {
    if notes.is_empty() {
        // No-op: do not create an empty goal-notes.json.
        return Ok(());
    }
    let gdir = goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(GoalError::GoalNotFound);
    }
    let notes_path = gdir.join(GOAL_NOTES_FILE);
    let mut arr: serde_json::Value = if notes_path.exists() {
        serde_json::from_str(&fs::read_to_string(&notes_path)?)?
    } else {
        serde_json::json!({ "notes": [] })
    };
    if let Some(arr_notes) = arr.get_mut("notes").and_then(|v| v.as_array_mut()) {
        for note in notes {
            arr_notes.push(serde_json::Value::String(note.clone()));
        }
    }
    fs::write(&notes_path, serde_json::to_string_pretty(&arr)?)?;
    Ok(())
}

/// Load every goal-scoped note ever appended, in insertion order. Returns an empty
/// `Vec` (NOT an error) when no `goal-notes.json` exists. Fail-closed (`GoalNotFound`)
/// when the goal directory itself is missing.
pub fn load_notes(root: &Path, goal_id: &str) -> Result<Vec<String>, GoalError> {
    let gdir = goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(GoalError::GoalNotFound);
    }
    let notes_path = gdir.join(GOAL_NOTES_FILE);
    if !notes_path.exists() {
        return Ok(Vec::new());
    }
    let arr: serde_json::Value = serde_json::from_str(&fs::read_to_string(&notes_path)?)?;
    let mut out = Vec::new();
    if let Some(arr_notes) = arr.get("notes").and_then(|v| v.as_array()) {
        for v in arr_notes {
            if let Some(s) = v.as_str() {
                out.push(s.to_string());
            }
        }
    }
    Ok(out)
}

/// Return the current round for a goal (fail closed if missing).
pub fn current_round(root: &Path, goal_id: &str) -> Result<u32, GoalError> {
    let gdir = goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(GoalError::GoalNotFound);
    }
    let state_path = gdir.join(STATE_FILE);
    if !state_path.exists() {
        return Ok(1);
    }
    let state: StateRecord = serde_json::from_str(&fs::read_to_string(&state_path)?)?;
    Ok(state.current_round)
}

/// Load the goal record (fail closed if missing).
pub fn load(root: &Path, goal_id: &str) -> Result<GoalRecord, GoalError> {
    let gdir = goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(GoalError::GoalNotFound);
    }
    let raw = fs::read_to_string(gdir.join(GOAL_FILE))?;
    Ok(serde_json::from_str(&raw)?)
}

/// Verify that the stored signature matches a recomputation from the current `goal.json`.
///
/// Returns `Ok(())` if the goal is untampered, or `Err` if `goalText`/`createdAt` were
/// edited after creation (signature mismatch) — the core fail-closed immutability guard.
pub fn verify_signature(root: &Path, goal_id: &str) -> Result<(), GoalError> {
    let gdir = goal_dir(root, goal_id);
    if !gdir.exists() {
        return Err(GoalError::GoalNotFound);
    }
    let salt = store::salt_in(root)?;
    let record: GoalRecord = serde_json::from_str(&fs::read_to_string(gdir.join(GOAL_FILE))?)?;
    let sig_record: SignatureRecord =
        serde_json::from_str(&fs::read_to_string(gdir.join(SIGNATURE_FILE))?)?;

    let recomputed = compute_signature(&salt, &record.goal_text, &record.created_at);
    if recomputed == sig_record.signature {
        Ok(())
    } else {
        Err(GoalError::SignatureMismatch)
    }
}

/// The exact signature formula (D5): `SHA256(salt + goalText + createdAt)` as lowercase hex.
pub fn compute_signature(salt: &str, goal_text: &str, created_at: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update(goal_text.as_bytes());
    h.update(created_at.as_bytes());
    hex::encode(h.finalize())
}

/// Errors raised by the goal layer. All fail-closed.
#[derive(Debug, thiserror::Error)]
pub enum GoalError {
    #[error("goal not found (store or goal directory missing)")]
    GoalNotFound,
    #[error("signature mismatch: goal has been tampered after creation")]
    SignatureMismatch,
    #[error("store root is unusable: {0}")]
    StoreUnusable(#[source] io::Error),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("store error: {0}")]
    Store(#[from] store::StoreError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
