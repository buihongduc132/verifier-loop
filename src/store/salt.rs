//! Salt creation (tasks.md §2.1, goal-lifecycle spec).
//!
//! `~/.verifier-loop/.salt` — 64 hex chars (32 bytes of OS entropy), mode 0600, created on
//! the **first** run only, and thereafter read back verbatim. The salt is never printed,
//! logged, or otherwise exfiltrated by the store API; it is returned only to the caller
//! (the signature/hash computation).
//!
//! Entropy source: `/dev/urandom` (Linux). We avoid pulling a separate `rand`/`getrandom`
//! crate by reading directly from the kernel CSPRNG. A failure to read entropy is a
//! hard, fail-closed error — we never fall back to a weak/predictable salt.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use super::StoreError;

/// Number of raw entropy bytes (32) → 64 hex chars as required by the spec.
const SALT_BYTES: usize = 32;

/// Returns the salt for the given store root, creating it on first run.
///
/// * If `<root>/.salt` does not exist: generate 32 random bytes from `/dev/urandom`,
///   write them as 64 lowercase hex chars to `<root>/.salt` with mode 0600, return the hex.
/// * If `<root>/.salt` exists: read and return it (trimmed of surrounding whitespace),
///   **never** overwriting or regenerating.
///
/// Guarantees (covered by `tests/store.rs`):
///   - 64 hex chars, all `[0-9a-f]`
///   - mode 0600
///   - stable across calls within one store
///   - distinct across independent stores
pub fn salt_in(root: &Path) -> Result<String, StoreError> {
    let path = root.join(".salt");

    if path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        return Ok(raw.trim().to_string());
    }

    // Ensure the parent directory exists (best-effort; the caller usually does this via
    // `ensure_home_at`, but `salt_in` must be self-sufficient for direct store tests).
    std::fs::create_dir_all(root)?;

    let mut bytes = [0u8; SALT_BYTES];
    read_urandom(&mut bytes)?;

    let hex_str = hex::encode(bytes);

    // create_new so a concurrent first-run never clobbers an existing salt (fail-closed).
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    f.write_all(hex_str.as_bytes())?;

    // Re-assert 0600 explicitly: `mode()` is masked by the process umask at creation,
    // so a permissive umask could otherwise leave the salt world/group-readable. The salt
    // MUST be 0600 per spec ("Salt is generated once and protected").
    let mut perms = std::fs::metadata(&path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(&path, perms)?;

    Ok(hex_str)
}

/// Reads exactly `buf.len()` bytes from `/dev/urandom`. Fail-closed on any short read or
/// open failure — a missing CSPRNG means we cannot produce a sound salt.
fn read_urandom(buf: &mut [u8]) -> Result<(), StoreError> {
    let mut f = File::open("/dev/urandom")?;
    f.read_exact(buf)?;
    Ok(())
}
