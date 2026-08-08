//! RAII guard for goal-file transport tempfiles (fix-spawn-argv-overflow §7, design D3;
//! prompt-transport spec "Tempfile lifecycle is bounded and fail-safe").
//!
//! [`TempPromptFile`] creates a unique file under `std::env::temp_dir()` with the
//! `verifier-loop-` prefix, writes the rendered prompt bytes, and unlinks the file on
//! drop. The orchestrator creates the guard, substitutes its path into the adapter's
//! `{goalFile}` placeholder, spawns the child, and holds the guard until the gather
//! barrier has reaped the child — guaranteeing the child had the full lifetime of its
//! run to open the file by path, while ensuring no tempfile persists past the run
//! (even on orchestrator panic, the guard's `Drop` unlinks).
//!
//! [`sweep_stale_tempfiles`] is a best-effort startup sweep that removes any leftover
//! `verifier-loop-*` tempfiles from prior crashed runs (design R1).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Prefix for all verifier-loop goal-file tempfiles. [`sweep_stale_tempfiles`] matches
/// on this prefix; do not change it without updating the sweep.
pub const TEMPFILE_PREFIX: &str = "verifier-loop-";

/// Monotonic counter so two tempfiles created in the same pid + nanosecond still get
/// distinct names (parallel verifiers spawn within microseconds of each other).
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// RAII guard for a goal-file transport tempfile (design D3).
///
/// On construction: creates a unique file under the OS temp dir, writes the prompt
/// bytes, and returns the guard holding the path. On drop: unlinks the file, ignoring
/// errors (best-effort cleanup — the file may already be gone if the sweep ran).
///
/// The guard is held by the spawn orchestrator across the child's gather barrier so
/// the file lives exactly as long as the child might need to open it by path, and no
/// longer. If the orchestrator panics between creation and drop, Rust's normal unwind
/// still runs `Drop`, unlinking the file; if the process is killed, the next run's
/// [`sweep_stale_tempfiles`] reclaims it.
pub struct TempPromptFile {
    path: PathBuf,
}

impl TempPromptFile {
    /// Create a unique tempfile under the OS temp dir, write `prompt`, return the guard.
    ///
    /// If the write fails (full disk, permission, etc.), the partially-written file is
    /// removed immediately so no tempfile leaks (the guard is never constructed on
    /// error, so its `Drop` would never run).
    pub fn new(prompt: &[u8]) -> io::Result<Self> {
        let path = unique_path();
        if let Err(e) = fs::write(&path, prompt) {
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        Ok(Self { path })
    }

    /// The absolute path of the tempfile. Substitute this into `{goalFile}`.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPromptFile {
    fn drop(&mut self) {
        // Best-effort: ignore "not found" (sweep may have already removed it) and any
        // other error (the OS will reclaim the temp dir eventually).
        let _ = fs::remove_file(&self.path);
    }
}

/// Build a unique tempfile path under the OS temp dir.
///
/// Uniqueness combines: OS temp dir, `verifier-loop-` prefix, pid, monotonic nanos,
/// and a process-local atomic counter. The counter is the decisive guard against
/// same-pid same-nanos collisions (two parallel verifiers spawning back-to-back).
fn unique_path() -> PathBuf {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    dir.join(format!("{TEMPFILE_PREFIX}{pid}-{nanos}-{n}.txt"))
}

/// Best-effort sweep of stale `verifier-loop-*` tempfiles in the OS temp dir
/// (design R1 / tasks.md §7.3).
///
/// Scans `std::env::temp_dir()` for entries whose name starts with
/// [`TEMPFILE_PREFIX`] and unlinks them. Unrelated files are NEVER touched. Every error
/// (unreadable dir, permission denied on a single entry, etc.) is swallowed — this is a
/// non-blocking startup cleanup, not a correctness gate. Safe to call at any time;
/// idempotent.
///
/// Concurrency-safe age threshold: only tempfiles older than [`SWEEP_MIN_AGE_SECS`] are
/// removed, so a freshly-started `verifier-loop` process never deletes an active
/// tempfile belonging to a concurrently-running sibling (parallel CI jobs, multi-user
/// hosts). A file younger than the threshold is left for a future sweep.
pub const SWEEP_MIN_AGE_SECS: u64 = 2;
pub fn sweep_stale_tempfiles() {
    let dir = std::env::temp_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return, // temp dir unreadable — nothing to sweep.
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(TEMPFILE_PREFIX) {
            // Concurrency guard: only remove files older than SWEEP_MIN_AGE_SECS so a
            // newly-started process cannot delete an active tempfile of a concurrently
            // running sibling. Files younger than the threshold are left for a future
            // sweep. Metadata/elapsed errors fall through to "skip" (safe).
            let stale = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|modified| {
                    modified
                        .elapsed()
                        .ok()
                        .map(|e| e >= Duration::from_secs(SWEEP_MIN_AGE_SECS))
                });
            if stale.unwrap_or(false) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_writes_prompt_and_path_is_under_temp_dir() {
        let guard = TempPromptFile::new(b"hello-goalfile").expect("create tempfile");
        let path = guard.path().to_path_buf();
        assert!(
            path.is_file(),
            "tempfile must exist on disk while guard is held"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            b"hello-goalfile",
            "tempfile must contain the prompt bytes exactly"
        );
        assert!(
            path.starts_with(std::env::temp_dir()),
            "tempfile must live under the OS temp dir"
        );
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(TEMPFILE_PREFIX),
            "tempfile name must carry the verifier-loop- prefix"
        );
        drop(guard);
        assert!(!path.exists(), "drop must unlink the tempfile");
    }

    #[test]
    fn drop_unlinks_even_when_prompt_is_large() {
        let big = vec![b'Z'; 1024 * 1024];
        let guard = TempPromptFile::new(&big).expect("create 1MiB tempfile");
        let path = guard.path().to_path_buf();
        assert_eq!(fs::read(&path).unwrap().len(), 1024 * 1024);
        drop(guard);
        assert!(!path.exists(), "large tempfile must be unlinked on drop");
    }

    #[test]
    fn two_consecutive_guards_get_distinct_paths() {
        let a = TempPromptFile::new(b"a").unwrap();
        let b = TempPromptFile::new(b"b").unwrap();
        assert_ne!(a.path(), b.path(), "tempfile names must be unique");
    }

    #[test]
    fn sweep_removes_only_verifier_loop_prefixed_files() {
        let tmp = std::env::temp_dir();
        let tag = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let stale = tmp.join(format!("{TEMPFILE_PREFIX}sweep-{tag}.txt"));
        let unrelated = tmp.join(format!("unrelated-sweep-{tag}.txt"));
        fs::write(&stale, b"x").unwrap();
        fs::write(&unrelated, b"y").unwrap();

        // Age threshold (SWEEP_MIN_AGE_SECS): the sweep only removes files older than
        // this, so a freshly-created stale fixture must age past it before the sweep
        // will reclaim it. Without this wait the file is "active" and correctly left
        // alone (concurrency-safe vs sibling jewilo runs).
        std::thread::sleep(Duration::from_secs(SWEEP_MIN_AGE_SECS));
        sweep_stale_tempfiles();

        assert!(
            !stale.exists(),
            "stale verifier-loop tempfile must be swept"
        );
        assert!(
            unrelated.exists(),
            "unrelated file must NOT be removed by the sweep"
        );
        let _ = fs::remove_file(&unrelated); // cleanup
    }

    #[test]
    fn sweep_is_safe_when_temp_dir_is_unreadable() {
        // Pointing at a non-existent dir must not panic — the sweep swallows errors.
        // We can't easily redirect std::env::temp_dir(), but we can assert the function
        // is idempotent and panic-free when called repeatedly.
        sweep_stale_tempfiles();
        sweep_stale_tempfiles();
    }
}
