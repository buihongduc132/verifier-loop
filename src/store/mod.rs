//! Salt + config store (tasks.md §2, goal-lifecycle spec).
//!
//! * `~/.verifier-loop/.salt` — 64 hex chars, mode 0600, created once, never printed.
//! * `~/.verifier-loop/config.json` — `n`, `m`, `maxTurn`, `backend`,
//!   `gitDiffMaxChars`, `verifierTimeoutSec`, plus optional prompt/resume templates
//!   and custom-adapter templates (landed in later groups).
//!
//! The store root defaults to `~/.verifier-loop` but is overridable via the
//! `VERIFIER_LOOP_HOME` environment variable. This is necessary for hermetic tests and
//! is spec-compatible (the default behaviour is unchanged). It is documented in USAGE.md.

use std::fs;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Environment variable used to override the store root (default: `~/.verifier-loop`).
pub const HOME_ENV: &str = "VERIFIER_LOOP_HOME";
/// Default store root (under the user's home directory).
pub const DEFAULT_HOME_DIR: &str = ".verifier-loop";
/// Salt file name within the store root.
pub const SALT_FILE: &str = ".salt";
/// Config file name within the store root.
pub const CONFIG_FILE: &str = "config.json";

/// Resolve the store root directory.
///
/// Honours `VERIFIER_LOOP_HOME` if set, otherwise falls back to `~/.verifier-loop`.
/// Returns an error if the home directory cannot be determined and no override is set
/// (fail-closed: no home → no store → no hash).
pub fn home() -> Result<PathBuf, StoreError> {
    if let Some(h) = std::env::var_os(HOME_ENV) {
        return Ok(PathBuf::from(h));
    }
    let home = std::env::var_os("HOME").ok_or(StoreError::NoHomeDir)?;
    Ok(PathBuf::from(home).join(DEFAULT_HOME_DIR))
}

/// Ensure a given store root directory exists, creating it (and parents) if missing.
pub fn ensure_home_at(root: &Path) -> Result<PathBuf, StoreError> {
    fs::create_dir_all(root)?;
    Ok(root.to_path_buf())
}

/// Ensure the default store root directory exists, creating it (and parents) if missing.
pub fn ensure_home() -> Result<PathBuf, StoreError> {
    ensure_home_at(&home()?)
}

/// Read or create the 64-hex-char salt at `<root>/.salt` (mode 0600).
///
/// Core, parallel-safe API: the caller supplies the store root explicitly. The
/// convenience wrapper [`salt`] resolves the root from the environment.
///
/// On first run a fresh salt is generated and persisted. On subsequent runs the existing
/// salt is returned unchanged. The salt value is **never** printed or logged by this
/// function — it is returned only to the caller (the hash computation).
pub fn salt_in(root: &Path) -> Result<String, StoreError> {
    ensure_home_at(root)?;
    let path = root.join(SALT_FILE);
    if path.exists() {
        let s = fs::read_to_string(&path)?;
        let s = s.trim().to_string();
        validate_salt(&s)?;
        return Ok(s);
    }
    let s = generate_salt();
    // Create with mode 0600 via OpenOptions (Unix only; the design targets Unix hosts).
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(StoreError::CreateSalt)?;
    fs::write(&path, &s)?;
    // Belt-and-braces: ensure mode is exactly 0600 even if the file pre-existed with
    // different bits in a future code path.
    let mut perms = fs::metadata(&path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&path, perms)?;
    Ok(s)
}

/// Env-resolving convenience wrapper around [`salt_in`].
pub fn salt() -> Result<String, StoreError> {
    salt_in(&home()?)
}

/// Configuration loaded from `config.json` with fail-safe defaults applied for any
/// missing field. See design D-spec / tasks.md §2.2 for the default set.
///
/// Field names in `config.json` are camelCase (`maxTurn`, `gitDiffMaxChars`, …).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_n")]
    pub n: u32,
    #[serde(default = "default_m")]
    pub m: u32,
    #[serde(default = "default_max_turn")]
    pub max_turn: u32,
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default = "default_git_diff_max_chars")]
    pub git_diff_max_chars: usize,
    #[serde(default = "default_verifier_timeout_sec")]
    pub verifier_timeout_sec: u64,
    // Optional templates — landed in §4/§9; present here so partial configs parse.
    #[serde(default)]
    pub verifier_prompt_template: Option<String>,
    #[serde(default)]
    pub verifier_resume_prompt_template: Option<String>,
    /// Custom adapter spawn/resume templates (used only when backend == "custom").
    #[serde(default)]
    pub custom_spawn_template: Option<String>,
    #[serde(default)]
    pub custom_resume_template: Option<String>,
    /// Whether the custom adapter requires a JSON-mode flag (e.g. `--mode json`).
    #[serde(default)]
    pub custom_json_flag: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            n: default_n(),
            m: default_m(),
            max_turn: default_max_turn(),
            backend: default_backend(),
            git_diff_max_chars: default_git_diff_max_chars(),
            verifier_timeout_sec: default_verifier_timeout_sec(),
            verifier_prompt_template: None,
            verifier_resume_prompt_template: None,
            custom_spawn_template: None,
            custom_resume_template: None,
            custom_json_flag: None,
        }
    }
}

impl Config {
    /// Load `config.json` from the default store root, applying defaults for missing fields.
    /// If the file does not exist, the full default config is returned (fail-safe).
    pub fn load() -> Result<Self, StoreError> {
        Self::load_in(&home()?)
    }

    /// Core, parallel-safe loader: read `config.json` from an explicit root, applying
    /// defaults for any missing field. Missing file → full default config.
    pub fn load_in(root: &Path) -> Result<Self, StoreError> {
        let path = root.join(CONFIG_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path)?;
        let cfg: Self = serde_json::from_str(&raw)?;
        Ok(cfg)
    }
}

fn default_n() -> u32 {
    2
}
fn default_m() -> u32 {
    2
}
fn default_max_turn() -> u32 {
    3
}
fn default_backend() -> String {
    "pi".to_string()
}
fn default_git_diff_max_chars() -> usize {
    10000
}
fn default_verifier_timeout_sec() -> u64 {
    1800
}

/// Backwards-compatible free function wrapper around [`Config::load`].
pub fn load_config() -> Result<Config, StoreError> {
    Config::load()
}

/// Core, parallel-safe loader wrapper around [`Config::load_in`].
pub fn load_config_in(root: &Path) -> Result<Config, StoreError> {
    Config::load_in(root)
}

/// Generate a 64-hex-char salt from the OS CSPRNG (32 bytes → 64 hex chars).
fn generate_salt() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // We do not pull in the `rand` crate to keep the dependency surface minimal; instead
    // we seed from /dev/urandom (Unix) which is the platform CSPRNG. Fall back to a
    // time+pid mix only if /dev/urandom is unavailable (best-effort; on a Unix host this
    // path is effectively unreachable).
    let mut bytes = [0u8; 32];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        use std::io::Read;
        if f.read_exact(&mut bytes).is_ok() {
            return hex::encode(bytes);
        }
    }
    // Fallback (not expected on Unix): mix time + pid into a deterministic-ish seed.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut h_input = Vec::with_capacity(48);
    h_input.extend_from_slice(&nanos.to_le_bytes());
    h_input.extend_from_slice(&pid.to_le_bytes());
    h_input.extend_from_slice(b"verifier-loop-salt-fallback");
    hex::encode(sha2_hash(&h_input))
}

/// Validate an existing salt: 64 lowercase/uppercase hex chars.
fn validate_salt(s: &str) -> Result<(), StoreError> {
    if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(StoreError::CorruptSalt)
    }
}

fn sha2_hash(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

/// Errors raised by the store layer. All are fail-closed: the caller must not produce a
/// hash when any of these occur.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("could not determine home directory (set VERIFIER_LOOP_HOME or $HOME)")]
    NoHomeDir,
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("could not create salt file")]
    CreateSalt(#[source] io::Error),
    #[error("salt file is corrupt (expected 64 hex chars)")]
    CorruptSalt,
    #[error("config.json is invalid: {0}")]
    BadConfig(#[from] serde_json::Error),
}

// `OpenOptionsExt::mode` is used above via `fs::OpenOptions::new()...mode(0o600)`.
