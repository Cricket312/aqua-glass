//! Small shared helpers: data directories, time formatting, logging.

use std::path::PathBuf;

pub use aqua_core::storage::{atomic_write, ensure_dir, now_unix};

/// Root data directory (`%APPDATA%\AquaGlass` on Windows).
/// `AQUA_GLASS_HOME` overrides it (used by tests and portable mode).
/// The choice is handed to `aqua_core::storage` so portable code uses the same place.
pub fn data_dir() -> PathBuf {
    aqua_core::storage::data_dir_or(desktop_data_dir)
}

fn desktop_data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("AQUA_GLASS_HOME") {
        return PathBuf::from(p);
    }
    if cfg!(test) {
        return std::env::temp_dir().join(format!("aqua-glass-test-{}", std::process::id()));
    }
    // Portable mode: a `portable.txt` next to the exe keeps data beside it.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable.txt").exists() {
                return dir.join("data");
            }
        }
    }
    directories::ProjectDirs::from("", "", "AquaGlass")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("aqua-glass-data"))
}

pub fn cache_dir() -> PathBuf {
    data_dir().join("cache")
}

/// Connect the portable crates to this desktop platform: data directory,
/// secret store (Windows Credential Manager) and browser launcher. Idempotent.
pub fn init_platform() {
    let _ = data_dir();
    crate::security::install_platform_store();
    aqua_core::platform::install_url_opener(|u| open::that_detached(u).map_err(|e| e.to_string()));
}

pub fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn fmt_unix(ts: i64) -> String {
    if ts <= 0 {
        return "never".into();
    }
    let fmt = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]");
    let dt = time::OffsetDateTime::from_unix_timestamp(ts).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let dt = match time::UtcOffset::current_local_offset() {
        Ok(off) => dt.to_offset(off),
        Err(_) => dt,
    };
    dt.format(&fmt).unwrap_or_default()
}

pub fn fmt_bytes(b: u64) -> String {
    let mb = b as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.0} KB", b as f64 / 1024.0)
    }
}

/// Initialise logging. Diagnostic file logging only when the user enabled it.
/// Tokens are never passed to the logger (see `security`).
pub fn init_logging(file_logging: bool) {
    use simplelog::*;
    let level = if file_logging { LevelFilter::Debug } else { LevelFilter::Warn };
    let cfg = ConfigBuilder::new()
        .add_filter_ignore_str("librespot_core::http_client")
        .add_filter_ignore_str("reqwest")
        .add_filter_ignore_str("hyper")
        .build();
    let mut loggers: Vec<Box<dyn SharedLogger>> = Vec::new();
    if file_logging {
        let _ = ensure_dir(&data_dir());
        if let Ok(f) = std::fs::File::create(data_dir().join("aqua-glass.log")) {
            loggers.push(WriteLogger::new(level, cfg.clone(), f));
        }
    }
    #[cfg(debug_assertions)]
    loggers.push(SimpleLogger::new(LevelFilter::Info, cfg));
    let _ = CombinedLogger::init(loggers);
}

/// Case-insensitive "contains" used by settings search and library filters.
pub fn icontains(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    hay.to_lowercase().contains(&needle.to_lowercase())
}
