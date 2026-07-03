//! Salt + config store (tasks.md §2, goal-lifecycle spec).
//!
//! Two artefacts live under the store root (default `~/.verifier-loop/`):
//!
//! * `.salt`        — 64 hex chars, mode 0600, created on first run, **never** overwritten,
//!                    never printed/logged/exposed to the invoking agent (A). It salts the
//!                    goal signature (D5) which in turn feeds the completion hash (D6).
//! * `config.json`  — n, m, maxTurn, backend, gitDiffMaxChars, verifierTimeoutSec, plus
//!                    optional prompt/resume + custom-adapter templates. Missing fields fall
//!                    back to documented defaults; a missing file is entirely defaulted.
//!
//! Fail-closed: every error is explicit (`Result<T, StoreError>`); a missing/unreadable
//! store surfaces as an error, never a silent default-of-secrets.

mod config;
mod salt;

use std::path::{Path, PathBuf};

pub use config::{load_config_in, Config};
pub use salt::salt_in;

/// Errors raised by the store layer. All paths fail closed.
///
/// Note: this is `pub` so downstream layers (e.g. `goal`) can propagate it via
/// `#[from] store::StoreError`.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Filesystem I/O failure (read/write/perm).
    #[error("store io error: {0}")]
    Io(#[from] std::io::Error),
    /// `config.json` present but not valid JSON / wrong shape.
    #[error("config.json is invalid: {0}")]
    Json(String),
}

/// Ensures the store root directory exists (creating it and parents as needed).
///
/// Used by the goal layer at `NEW` time before writing `goal.json`/`signature.json`.
/// Idempotent: ignores "already exists". Returns the canonicalised root on success.
pub fn ensure_home_at(root: &Path) -> Result<PathBuf, StoreError> {
    std::fs::create_dir_all(root)?;
    Ok(root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_home_at_creates_missing_dir_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("nested/deep/store");
        assert!(!root.exists());

        let returned = ensure_home_at(&root).expect("creates nested store root");
        assert!(root.exists(), "store root must be created");
        assert_eq!(returned, root);

        // Idempotent: a second call on the now-existing root must not error.
        ensure_home_at(&root).expect("idempotent on existing root");
    }
}
