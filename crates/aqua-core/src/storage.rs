//! Where portable code keeps its files.
//!
//! The data directory is chosen by the platform layer at startup
//! (`%APPDATA%\AquaGlass` or portable mode on Windows; on a future Android
//! client, `Context.getFilesDir()` passed in over FFI). Portable code only ever
//! asks for `data_dir()` and never guesses OS-specific locations itself.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Set the data directory. Returns false if it was already chosen.
pub fn init_data_dir(dir: PathBuf) -> bool {
    ROOT.set(dir).is_ok()
}

/// The data directory, choosing it with `default` if the platform hasn't set it yet.
pub fn data_dir_or(default: impl FnOnce() -> PathBuf) -> PathBuf {
    ROOT.get_or_init(default).clone()
}

/// The data directory. Falls back to `AQUA_GLASS_HOME`, then to a per-process
/// temporary directory (tests / misconfigured hosts) — never to a guessed OS path.
pub fn data_dir() -> PathBuf {
    data_dir_or(|| {
        if let Ok(p) = std::env::var("AQUA_GLASS_HOME") {
            return PathBuf::from(p);
        }
        std::env::temp_dir().join(format!("aqua-glass-{}", std::process::id()))
    })
}

pub fn ensure_dir(p: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(p)
}

/// Write a file atomically (write temp + rename) so a crash never leaves a half-written file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

pub fn now_unix() -> i64 {
    now_millis() / 1000
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
