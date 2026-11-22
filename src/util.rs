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
    loggers.push(Box::new(PlaybackErrorCapture));
    let _ = CombinedLogger::init(loggers);
}

/// Most recent warning/error from the Spotify playback library, so the player can
/// say *why* a song didn't play (always on; keeps one short message in memory).
static LAST_PLAYBACK_ERROR: std::sync::Mutex<Option<(std::time::Instant, String)>> = std::sync::Mutex::new(None);

struct PlaybackErrorCapture;

impl log::Log for PlaybackErrorCapture {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn && m.target().starts_with("librespot")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            let mut msg = crate::security::scrub(&r.args().to_string());
            if msg.chars().count() > 160 {
                msg = msg.chars().take(160).collect::<String>() + "…";
            }
            *LAST_PLAYBACK_ERROR.lock().unwrap() = Some((std::time::Instant::now(), msg));
        }
    }
    fn flush(&self) {}
}

impl simplelog::SharedLogger for PlaybackErrorCapture {
    fn level(&self) -> log::LevelFilter {
        log::LevelFilter::Warn
    }
    fn config(&self) -> Option<&simplelog::Config> {
        None
    }
    fn as_log(self: Box<Self>) -> Box<dyn log::Log> {
        self
    }
}

/// The playback library's last error, if it happened after `since`.
pub fn playback_error_since(since: std::time::Instant) -> Option<String> {
    LAST_PLAYBACK_ERROR.lock().unwrap().as_ref().filter(|(t, _)| *t >= since).map(|(_, m)| m.clone())
}

/// Case-insensitive "contains" used by settings search and library filters.
pub fn icontains(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    hay.to_lowercase().contains(&needle.to_lowercase())
}

/// Hand memory freed by the app back to the operating system where the allocator
/// keeps it cached (used when entering Essential Mode). glibc keeps freed heap
/// pages mapped until `malloc_trim`; the Windows heap decommits large free blocks
/// by itself, so nothing is needed there.
pub fn release_free_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: malloc_trim only returns free heap memory to the kernel.
        unsafe {
            malloc_trim(0);
        }
    }
}
