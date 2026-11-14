//! Shortcut actions, binding parsing and conflict detection.
//!
//! Bindings are strings like `Ctrl+Alt+P` or `MediaPlayPause`. The same binding
//! is used for in-app shortcuts (egui) and, when "Global Hotkeys" is on, for
//! system-wide hotkeys (Windows `RegisterHotKey` via the `global-hotkey` crate).
//! Macro pads that send ordinary key combinations (e.g. F13–F24) work as-is.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    PlayPause,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
    Mute,
    SeekForward,
    SeekBackward,
    LikeSong,
    ToggleLyrics,
    ToggleMiniPlayer,
    OpenSearch,
    OpenSettings,
    SwitchProvider,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::PlayPause,
        Action::Next,
        Action::Previous,
        Action::VolumeUp,
        Action::VolumeDown,
        Action::Mute,
        Action::SeekForward,
        Action::SeekBackward,
        Action::LikeSong,
        Action::ToggleLyrics,
        Action::ToggleMiniPlayer,
        Action::OpenSearch,
        Action::OpenSettings,
        Action::SwitchProvider,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Action::PlayPause => "play_pause",
            Action::Next => "next",
            Action::Previous => "previous",
            Action::VolumeUp => "volume_up",
            Action::VolumeDown => "volume_down",
            Action::Mute => "mute",
            Action::SeekForward => "seek_forward",
            Action::SeekBackward => "seek_backward",
            Action::LikeSong => "like_song",
            Action::ToggleLyrics => "toggle_lyrics",
            Action::ToggleMiniPlayer => "toggle_mini_player",
            Action::OpenSearch => "open_search",
            Action::OpenSettings => "open_settings",
            Action::SwitchProvider => "switch_provider",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Action::PlayPause => "Play / Pause",
            Action::Next => "Next track",
            Action::Previous => "Previous track",
            Action::VolumeUp => "Volume up",
            Action::VolumeDown => "Volume down",
            Action::Mute => "Mute",
            Action::SeekForward => "Seek forward",
            Action::SeekBackward => "Seek backward",
            Action::LikeSong => "Like song",
            Action::ToggleLyrics => "Toggle lyrics",
            Action::ToggleMiniPlayer => "Toggle mini-player",
            Action::OpenSearch => "Open search",
            Action::OpenSettings => "Open settings",
            Action::SwitchProvider => "Switch music provider filter",
        }
    }
    pub fn from_id(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.id() == s)
    }
}

pub fn default_bindings() -> BTreeMap<String, String> {
    let d = [
        (Action::PlayPause, "Ctrl+Alt+P"),
        (Action::Next, "Ctrl+Alt+Right"),
        (Action::Previous, "Ctrl+Alt+Left"),
        (Action::VolumeUp, "Ctrl+Alt+Up"),
        (Action::VolumeDown, "Ctrl+Alt+Down"),
        (Action::Mute, "Ctrl+Alt+M"),
        (Action::SeekForward, "Ctrl+Alt+Period"),
        (Action::SeekBackward, "Ctrl+Alt+Comma"),
        (Action::LikeSong, "Ctrl+Alt+L"),
        (Action::ToggleLyrics, "Ctrl+Alt+Y"),
        (Action::ToggleMiniPlayer, "Ctrl+Alt+N"),
        (Action::OpenSearch, "Ctrl+F"),
        (Action::OpenSettings, "Ctrl+Comma"),
        (Action::SwitchProvider, "Ctrl+Alt+S"),
    ];
    d.iter().map(|(a, b)| (a.id().to_string(), b.to_string())).collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: String,
}

/// Recognised key names (canonical spelling).
pub const KEYS: &[&str] = &[
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V",
    "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "F1", "F2", "F3", "F4", "F5", "F6",
    "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22",
    "F23", "F24", "Space", "Enter", "Escape", "Tab", "Backspace", "Insert", "Delete", "Home", "End", "PageUp",
    "PageDown", "Left", "Right", "Up", "Down", "Comma", "Period", "Minus", "Plus", "Slash", "Semicolon",
    "MediaPlayPause", "MediaNext", "MediaPrevious", "MediaStop", "VolumeUp", "VolumeDown", "VolumeMute",
];

impl Binding {
    pub fn parse(s: &str) -> Result<Binding, String> {
        let mut b = Binding { ctrl: false, alt: false, shift: false, win: false, key: String::new() };
        let s = s.trim();
        if s.is_empty() {
            return Err("empty shortcut".into());
        }
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => b.ctrl = true,
                "alt" => b.alt = true,
                "shift" => b.shift = true,
                "win" | "super" | "meta" => b.win = true,
                "" => return Err("malformed shortcut".into()),
                k => {
                    if !b.key.is_empty() {
                        return Err("only one non-modifier key allowed".into());
                    }
                    let canon = KEYS
                        .iter()
                        .find(|c| c.eq_ignore_ascii_case(k))
                        .ok_or_else(|| format!("unknown key '{part}'"))?;
                    b.key = canon.to_string();
                }
            }
        }
        if b.key.is_empty() {
            return Err("missing key".into());
        }
        Ok(b)
    }

    pub fn is_media_key(&self) -> bool {
        self.key.starts_with("Media") || self.key.starts_with("Volume")
    }

    pub fn to_string_canonical(&self) -> String {
        let mut v = Vec::new();
        if self.ctrl {
            v.push("Ctrl");
        }
        if self.alt {
            v.push("Alt");
        }
        if self.shift {
            v.push("Shift");
        }
        if self.win {
            v.push("Win");
        }
        v.push(&self.key);
        v.join("+")
    }

    /// Map to an egui key for in-window shortcuts.
    pub fn egui_key(&self) -> Option<eframe::egui::Key> {
        use eframe::egui::Key;
        let k = self.key.as_str();
        Some(match k {
            "Space" => Key::Space,
            "Enter" => Key::Enter,
            "Escape" => Key::Escape,
            "Tab" => Key::Tab,
            "Backspace" => Key::Backspace,
            "Insert" => Key::Insert,
            "Delete" => Key::Delete,
            "Home" => Key::Home,
            "End" => Key::End,
            "PageUp" => Key::PageUp,
            "PageDown" => Key::PageDown,
            "Left" => Key::ArrowLeft,
            "Right" => Key::ArrowRight,
            "Up" => Key::ArrowUp,
            "Down" => Key::ArrowDown,
            "Comma" => Key::Comma,
            "Period" => Key::Period,
            "Minus" => Key::Minus,
            "Plus" => Key::Plus,
            "Slash" => Key::Slash,
            "Semicolon" => Key::Semicolon,
            _ => return Key::from_name(k),
        })
    }

    pub fn matches_egui(&self, m: &eframe::egui::Modifiers) -> bool {
        m.ctrl == self.ctrl && m.alt == self.alt && m.shift == self.shift
    }
}

/// Returns groups of action ids that share the same binding.
pub fn conflicts(bindings: &BTreeMap<String, String>) -> Vec<(String, Vec<String>)> {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (action, s) in bindings {
        if s.trim().is_empty() {
            continue;
        }
        if let Ok(b) = Binding::parse(s) {
            by.entry(b.to_string_canonical()).or_default().push(action.clone());
        }
    }
    by.into_iter().filter(|(_, v)| v.len() > 1).collect()
}

/// Shortcuts reserved by Aqua Glass / Windows that we warn about.
pub fn reserved_conflict(b: &Binding) -> Option<&'static str> {
    let s = b.to_string_canonical();
    match s.as_str() {
        "Ctrl+C" | "Ctrl+V" | "Ctrl+X" | "Ctrl+A" | "Ctrl+Z" => Some("text editing shortcut"),
        "Alt+F4" => Some("Windows close-window shortcut"),
        "Alt+Tab" | "Win+L" | "Win+D" | "Ctrl+Alt+Delete" => Some("reserved by Windows"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_roundtrip() {
        let b = Binding::parse("ctrl + alt + p").unwrap();
        assert_eq!(b.to_string_canonical(), "Ctrl+Alt+P");
        assert!(Binding::parse("Ctrl+Alt").is_err());
        assert!(Binding::parse("Ctrl+Foo").is_err());
        assert!(Binding::parse("A+B").is_err());
        assert!(Binding::parse("F13").is_ok());
        assert!(Binding::parse("MediaPlayPause").unwrap().is_media_key());
    }
    #[test]
    fn detects_conflicts() {
        let mut m = default_bindings();
        assert!(conflicts(&m).is_empty(), "defaults conflict: {:?}", conflicts(&m));
        m.insert("next".into(), "ctrl+alt+p".into());
        let c = conflicts(&m);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].1.len(), 2);
    }
    #[test]
    fn defaults_parse() {
        for (a, s) in default_bindings() {
            assert!(Action::from_id(&a).is_some());
            let b = Binding::parse(&s).unwrap();
            assert!(b.egui_key().is_some(), "{s}");
        }
    }
}
