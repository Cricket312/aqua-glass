//! Small hooks the platform layer provides to portable code.
//!
//! Kept deliberately tiny: anything bigger (audio output, media sessions,
//! notifications, tray, hotkeys) stays in the platform crate/app and talks to
//! portable code through `crate::playback::PlaybackControl`.

use std::sync::OnceLock;

type Opener = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
static OPENER: OnceLock<Opener> = OnceLock::new();

/// Install how URLs are opened in the user's browser (desktop: the default
/// browser; Android: a Custom Tab / `ACTION_VIEW` intent via JNI).
pub fn install_url_opener(f: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static) -> bool {
    OPENER.set(Box::new(f)).is_ok()
}

/// Open `url` in the browser (OAuth sign-in, "Open in YouTube Music", ...).
pub fn open_url(url: &str) -> Result<(), String> {
    match OPENER.get() {
        Some(f) => f(url),
        None => Err("this platform has no browser launcher installed".into()),
    }
}

/// Which kind of device this build runs on (sent with sync/handoff metadata).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum Platform {
    Windows,
    Android,
    Linux,
    MacOs,
    #[default]
    Other,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "android") {
            Platform::Android
        } else if cfg!(target_os = "linux") {
            Platform::Linux
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Other
        }
    }
}
