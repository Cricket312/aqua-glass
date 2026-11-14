//! Registry of every optional, individually toggleable feature.
//!
//! Each entry declares its id, display text, category, RAM impact estimate,
//! dependencies, whether it counts as a "visual customization" and its value in
//! each performance preset. Runtime activation/cleanup is performed by the
//! subsystem owning the feature, driven by `App::apply_feature_change`, which
//! is called for every toggle change (see `ui::App`).
//!
//! Estimates are *engineering estimates*. Actual deltas measured on the user's
//! machine (see `perf::FeatureMeasurer`) are stored separately and always shown
//! in preference to the estimate, labelled "measured".

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Category {
    General,
    Appearance,
    Themes,
    Backgrounds,
    Playback,
    Audio,
    Lyrics,
    Visualizer,
    Discord,
    Shortcuts,
    MiniPlayer,
    Services,
    Library,
    Transfer,
    Performance,
    Privacy,
    Advanced,
}

impl Category {
    pub const ALL: [Category; 17] = [
        Category::General,
        Category::Appearance,
        Category::Themes,
        Category::Backgrounds,
        Category::Playback,
        Category::Audio,
        Category::Lyrics,
        Category::Visualizer,
        Category::Discord,
        Category::Shortcuts,
        Category::MiniPlayer,
        Category::Services,
        Category::Library,
        Category::Transfer,
        Category::Performance,
        Category::Privacy,
        Category::Advanced,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Appearance => "Appearance",
            Category::Themes => "Themes",
            Category::Backgrounds => "Backgrounds",
            Category::Playback => "Playback",
            Category::Audio => "Audio",
            Category::Lyrics => "Lyrics",
            Category::Visualizer => "Visualizer",
            Category::Discord => "Discord",
            Category::Shortcuts => "Keyboard Shortcuts",
            Category::MiniPlayer => "Mini-Player & Overlay",
            Category::Services => "Music Services",
            Category::Library => "Library & Synchronization",
            Category::Transfer => "Playlist Transfer",
            Category::Performance => "Performance",
            Category::Privacy => "Privacy",
            Category::Advanced => "Advanced",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Impact {
    Low,
    Moderate,
    High,
    VeryHigh,
}

impl Impact {
    /// Classify an additional-memory figure (MB) using the documented ranges.
    pub fn classify(mb: f32) -> Impact {
        if mb <= 5.0 {
            Impact::Low
        } else if mb <= 15.0 {
            Impact::Moderate
        } else if mb <= 35.0 {
            Impact::High
        } else {
            Impact::VeryHigh
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Impact::Low => "LOW",
            Impact::Moderate => "MODERATE",
            Impact::High => "HIGH",
            Impact::VeryHigh => "VERY HIGH",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Preset {
    Minimal,
    #[default]
    Balanced,
    Enhanced,
    FullyCustomized,
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::Minimal,
        Preset::Balanced,
        Preset::Enhanced,
        Preset::FullyCustomized,
        Preset::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Preset::Minimal => "Minimal",
            Preset::Balanced => "Balanced",
            Preset::Enhanced => "Enhanced",
            Preset::FullyCustomized => "Fully Customized",
            Preset::Custom => "Custom",
        }
    }
    fn index(self) -> Option<usize> {
        match self {
            Preset::Minimal => Some(0),
            Preset::Balanced => Some(1),
            Preset::Enhanced => Some(2),
            Preset::FullyCustomized => Some(3),
            Preset::Custom => None,
        }
    }
}

pub struct FeatureDef {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub category: Category,
    pub impact: Impact,
    /// Estimated additional RAM in MB (min, max). Not a measurement.
    pub est_mb: (f32, f32),
    /// Features that must be on for this one to have an effect.
    pub deps: &'static [&'static str],
    /// Part of "Disable All Visual Customizations".
    pub visual: bool,
    /// Value in the Minimal / Balanced / Enhanced / Fully Customized presets.
    pub presets: [bool; 4],
    /// Extra search keywords.
    pub keywords: &'static str,
}

impl FeatureDef {
    pub fn default_value(&self) -> bool {
        self.presets[1]
    }
    pub fn preset_value(&self, p: Preset) -> Option<bool> {
        p.index().map(|i| self.presets[i])
    }
    pub fn matches(&self, q: &str) -> bool {
        use crate::util::icontains;
        icontains(self.name, q)
            || icontains(self.desc, q)
            || icontains(self.keywords, q)
            || icontains(self.category.label(), q)
            || icontains(self.impact.label(), q)
            || (icontains("ram memory", q) && self.impact >= Impact::Moderate)
    }
}

macro_rules! f {
    ($id:expr, $name:expr, $cat:ident, $imp:ident, ($a:expr, $b:expr), [$($d:expr),*], $vis:expr, [$m:expr, $bal:expr, $e:expr, $full:expr], $desc:expr, $kw:expr) => {
        FeatureDef { id: $id, name: $name, desc: $desc, category: Category::$cat, impact: Impact::$imp,
            est_mb: ($a, $b), deps: &[$($d),*], visual: $vis, presets: [$m, $bal, $e, $full], keywords: $kw }
    };
}

pub static FEATURES: &[FeatureDef] = &[
    // ---------------- Appearance ----------------
    f!("custom_theme", "Custom Theme", Appearance, Low, (0.5, 2.0), [], true, [false, true, true, true],
        "Apply the selected theme (colors, rounding, spacing). When off, a plain built-in dark style is used.", "colors colour accent"),
    f!("animations", "Interface Animations", Appearance, Low, (0.0, 1.0), [], true, [false, true, true, true],
        "Animate hovering, collapsing and page transitions. Off = instant changes and fewer repaints.", "motion animation speed"),
    f!("hover_effects", "Hover Highlights", Appearance, Low, (0.0, 0.5), ["custom_theme"], true, [false, true, true, true],
        "Highlight rows and buttons under the mouse pointer.", "hover"),
    f!("shadows", "Window & Popup Shadows", Appearance, Low, (0.0, 1.0), ["custom_theme"], true, [false, false, true, true],
        "Soft shadows under popups and menus.", "shadow"),
    f!("translucent_panels", "Translucent Panels", Appearance, Low, (0.0, 1.0), ["custom_theme"], true, [false, false, true, true],
        "Draw the sidebar and player bar semi-transparent so backgrounds show through (glass look).", "glass transparency frosted"),
    f!("window_backdrop", "Windows 11 Acrylic Backdrop", Appearance, Moderate, (2.0, 12.0), ["translucent_panels"], true, [false, false, false, true],
        "Real frosted-glass window background provided by the Windows 11 compositor (DWM). Requires restart. Windows 11 only.", "blur acrylic mica glass dwm"),
    f!("custom_font", "Custom Font", Appearance, Moderate, (3.0, 12.0), [], true, [false, false, true, true],
        "Load a TrueType/OpenType font file (e.g. Segoe UI) instead of the bundled font. Larger fonts use a bigger glyph atlas.", "typeface font family"),
    f!("artwork_in_lists", "Artwork Thumbnails in Lists", Appearance, Moderate, (4.0, 15.0), [], false, [false, true, true, true],
        "Show small album covers next to tracks and playlists. Covers are cached within the artwork cache budget.", "images covers album art thumbnails"),
    // ---------------- Backgrounds ----------------
    f!("bg_album_art", "Album Artwork Background", Backgrounds, Moderate, (3.0, 10.0), [], true, [false, false, true, true],
        "Use the current track's cover as the window background.", "background cover image"),
    f!("bg_image", "Custom Background Image", Backgrounds, High, (8.0, 35.0), [], true, [false, false, false, true],
        "Use an image file as the window background (downscaled to the window size before upload).", "background wallpaper picture"),
    f!("bg_blur", "Blurred Background", Backgrounds, High, (6.0, 25.0), [], true, [false, false, true, true],
        "Blur the background image/artwork (computed once per image on a background thread, then cached).", "blur frosted background"),
    f!("bg_dim", "Background Dimming", Backgrounds, Low, (0.0, 0.2), [], true, [false, true, true, true],
        "Darken background images to keep text readable.", "dim darken opacity"),
    f!("bg_dynamic_colors", "Dynamic Artwork Colors", Backgrounds, Low, (0.0, 1.0), [], true, [false, false, true, true],
        "Tint the accent color with the dominant color of the current album artwork.", "dynamic color palette accent"),
    f!("bg_animated", "Animated Background", Backgrounds, Moderate, (1.0, 8.0), [], true, [false, false, false, true],
        "Slowly moving aqua gradient blobs. Forces continuous repaints at the configured FPS (raises CPU/GPU use).", "animated motion gradient"),
    f!("bg_parallax", "Parallax Effect", Backgrounds, Low, (0.0, 0.5), [], true, [false, false, false, true],
        "Shift the background slightly with the mouse pointer.", "parallax motion"),
    f!("bg_transitions", "Background Transitions", Backgrounds, Moderate, (3.0, 10.0), [], true, [false, false, true, true],
        "Cross-fade between backgrounds when the track changes (keeps the previous texture briefly).", "fade transition"),
    // ---------------- Playback ----------------
    f!("remember_queue", "Restore Queue on Launch", Playback, Low, (0.0, 1.0), [], false, [true, true, true, true],
        "Save the queue and current track on exit and restore it (paused) at next launch.", "resume session persistent"),
    f!("notifications", "Track Change Notifications", Playback, Low, (0.5, 3.0), [], false, [false, false, true, true],
        "Native Windows toast when a new track starts.", "toast notify"),
    // ---------------- Audio ----------------
    f!("eq", "Equalizer", Audio, Low, (0.0, 0.5), [], false, [false, false, false, false],
        "10-band graphic equalizer applied to all providers that play through Aqua Glass.", "eq equalizer bands"),
    f!("bass_boost", "Bass Boost", Audio, Low, (0.0, 0.2), [], false, [false, false, false, false],
        "Low-shelf boost below ~120 Hz.", "bass"),
    f!("normalization", "Volume Normalization / ReplayGain", Audio, Low, (0.0, 0.5), [], false, [false, true, true, true],
        "Spotify: loudness normalisation from Spotify's track gain data. Local files: ReplayGain tags.", "loudness replaygain"),
    f!("fades", "Fade In / Fade Out", Audio, Low, (0.0, 0.1), [], false, [false, true, true, true],
        "Short volume ramps when pausing, resuming and skipping to avoid clicks.", "fade"),
    f!("gapless", "Gapless Playback", Audio, Low, (0.0, 2.0), [], false, [true, true, true, true],
        "Start decoding the next track before the current ends so albums play without gaps.", "gapless preload"),
    // ---------------- Lyrics ----------------
    f!("lyrics", "Lyrics", Lyrics, Moderate, (1.0, 6.0), [], false, [false, true, true, true],
        "Fetch lyrics from LRCLIB (lrclib.net, a free community database). Sends track title, artist, album and duration to LRCLIB.", "lyrics karaoke lrclib text"),
    f!("lyrics_synced", "Synchronized Lyrics", Lyrics, Low, (0.0, 1.0), ["lyrics"], false, [false, true, true, true],
        "Highlight and auto-scroll the current line when time-synced lyrics exist.", "synced timed karaoke"),
    f!("lyrics_word", "Word-by-Word Highlighting", Lyrics, Low, (0.0, 0.5), ["lyrics_synced"], true, [false, false, true, true],
        "Highlight individual words when enhanced (word-timed) LRC data exists.", "karaoke word"),
    f!("lyrics_prefetch", "Prefetch Next Track Lyrics", Lyrics, Low, (0.0, 1.0), ["lyrics"], false, [false, false, true, true],
        "Download lyrics for the next queued track in advance.", "prefetch"),
    f!("lyrics_fullscreen", "Fullscreen Lyrics Button", Lyrics, Low, (0.0, 0.5), ["lyrics"], true, [false, true, true, true],
        "Adds a button on Now Playing to show lyrics fullscreen.", "fullscreen"),
    // ---------------- Visualizer ----------------
    f!("visualizer", "Audio Visualizer", Visualizer, Moderate, (1.0, 6.0), [], true, [false, false, true, true],
        "Real-time visualizer for audio played through Aqua Glass. FFT runs only while it is visible.", "spectrum fft bars waveform"),
    f!("vis_artwork_colors", "Artwork-Reactive Visualizer Colors", Visualizer, Low, (0.0, 0.5), ["visualizer"], true, [false, false, true, true],
        "Color the visualizer from the album artwork instead of the theme accent.", "colors"),
    f!("vis_reduced_motion", "Reduced Motion", Visualizer, Low, (0.0, 0.1), ["visualizer"], false, [false, false, false, false],
        "Heavier smoothing and a 20 FPS cap for a calmer visualizer.", "accessibility motion"),
    f!("vis_in_bar", "Visualizer in Player Bar", Visualizer, Low, (0.0, 1.0), ["visualizer"], true, [false, false, false, true],
        "Show a small spectrum in the bottom playback bar on every page.", "mini spectrum bar"),
    // ---------------- Discord ----------------
    f!("discord", "Discord Rich Presence", Discord, Low, (1.0, 4.0), [], false, [false, false, false, false],
        "Show what you're listening to in Discord. Talks only to the local Discord app over IPC; inactive when off. Discord need not be installed.", "discord rpc presence status"),
    f!("discord_title", "Show Song Title", Discord, Low, (0.0, 0.0), ["discord"], false, [true, true, true, true], "Include the title in your Discord status.", ""),
    f!("discord_artist", "Show Artist", Discord, Low, (0.0, 0.0), ["discord"], false, [true, true, true, true], "Include the artist.", ""),
    f!("discord_album", "Show Album", Discord, Low, (0.0, 0.0), ["discord"], false, [false, true, true, true], "Include the album name.", ""),
    f!("discord_artwork", "Show Album Artwork", Discord, Low, (0.0, 0.0), ["discord"], false, [false, true, true, true],
        "Send the public artwork URL (Spotify/YouTube CDN) so Discord can display it. Local files never send artwork.", "image"),
    f!("discord_elapsed", "Show Elapsed Time", Discord, Low, (0.0, 0.0), ["discord"], false, [true, true, true, true], "Show a running timer.", ""),
    f!("discord_service", "Show Service Name", Discord, Low, (0.0, 0.0), ["discord"], false, [true, true, true, true], "Show 'on Spotify' / 'on YouTube Music'.", ""),
    f!("discord_status", "Show Paused State", Discord, Low, (0.0, 0.0), ["discord"], false, [true, true, true, true], "Keep the status while paused, marked as paused. Off = clear status when paused.", ""),
    f!("discord_privacy", "Privacy Mode", Discord, Low, (0.0, 0.0), ["discord"], false, [false, false, false, false], "Only show 'Listening to music' with no track details.", "private hide"),
    // ---------------- Mini-player / overlay ----------------
    f!("mini_player", "Mini-Player Window", MiniPlayer, Moderate, (4.0, 15.0), [], false, [false, true, true, true],
        "Allow the compact mini-player window (toggle with the button or shortcut). A window is only created while it is open.", "mini compact small window"),
    f!("mini_on_top", "Mini-Player Always on Top", MiniPlayer, Low, (0.0, 0.0), ["mini_player"], false, [true, true, true, true], "Keep the mini-player above other windows.", "topmost pin"),
    f!("mini_lyrics", "Lyrics in Mini-Player", MiniPlayer, Low, (0.0, 0.5), ["mini_player", "lyrics"], false, [false, false, true, true], "Show the current lyric line in the mini-player.", ""),
    f!("overlay", "Desktop Overlay", MiniPlayer, Moderate, (4.0, 15.0), [], true, [false, false, false, true],
        "Transparent always-on-top desktop overlay with the current song. The window is destroyed when off.", "overlay desktop widget"),
    f!("overlay_click_through", "Overlay Click-Through", MiniPlayer, Low, (0.0, 0.0), ["overlay"], false, [true, true, true, true], "Mouse clicks pass through the overlay to windows below.", "passthrough"),
    f!("overlay_artwork", "Overlay Artwork", MiniPlayer, Low, (0.0, 1.0), ["overlay"], true, [false, false, true, true], "Show album artwork in the overlay.", ""),
    f!("overlay_lyrics", "Overlay Lyrics", MiniPlayer, Low, (0.0, 0.5), ["overlay", "lyrics"], false, [false, false, true, true], "Show the current lyric line in the overlay.", ""),
    f!("overlay_progress", "Overlay Progress Bar", MiniPlayer, Low, (0.0, 0.0), ["overlay"], false, [true, true, true, true], "Show a progress bar in the overlay.", ""),
    // ---------------- Shortcuts / Windows ----------------
    f!("global_hotkeys", "Global Hotkeys", Shortcuts, Low, (0.2, 1.0), [], false, [false, true, true, true],
        "Register system-wide shortcuts (work while Aqua Glass is in the background). Only the configured combinations are registered.", "keyboard macro pad global"),
    f!("media_keys", "Windows Media Controls & Media Keys", Shortcuts, Low, (0.5, 3.0), [], false, [true, true, true, true],
        "Integrate with the Windows media overlay (SMTC) and hardware media keys.", "smtc media keys taskbar"),
    f!("tray", "System Tray Icon", General, Low, (0.3, 2.0), [], false, [true, true, true, true],
        "Show a tray icon with playback controls and a Settings entry.", "tray notification area"),
    f!("minimize_to_tray", "Minimize to Tray", General, Low, (0.0, 0.0), ["tray"], false, [false, false, false, false], "Hide the window instead of minimizing to the taskbar.", ""),
    f!("close_to_tray", "Close to Tray", General, Low, (0.0, 0.0), ["tray"], false, [false, false, false, false], "Closing the window keeps Aqua Glass playing in the tray.", ""),
    f!("start_with_windows", "Start with Windows", General, Low, (0.0, 0.0), [], false, [false, false, false, false], "Launch Aqua Glass when you sign in to Windows (per-user, no admin rights).", "autostart startup boot"),
    f!("quick_settings", "Quick-Settings Button", General, Low, (0.0, 0.2), [], false, [false, true, true, true], "A button in the player bar with the most used toggles.", "quick"),
    // ---------------- Services / library ----------------
    f!("auto_sync", "Automatic Library Sync", Library, Low, (0.5, 3.0), [], false, [false, true, true, true],
        "Periodically refresh imported Spotify/YouTube library metadata. Uses snapshot ids to skip unchanged playlists.", "synchronization refresh"),
    f!("metadata_cache", "Persistent Metadata Cache", Library, Moderate, (2.0, 15.0), [], false, [true, true, true, true],
        "Keep imported library metadata in memory and on disk for instant browsing (size scales with library size).", "cache offline"),
    f!("artwork_disk_cache", "Artwork Disk Cache", Library, Low, (0.0, 1.0), [], false, [true, true, true, true],
        "Store downscaled covers on disk so they aren't downloaded again.", "cache images"),
    f!("artwork_prefetch", "Artwork Prefetching", Performance, Moderate, (3.0, 12.0), ["artwork_in_lists"], false, [false, false, true, true],
        "Load covers slightly ahead of scrolling.", "prefetch"),
    // ---------------- Performance ----------------
    f!("auto_optimizer", "Automatic Memory Optimizer", Performance, Low, (0.0, 0.5), [], false, [true, true, true, false],
        "When RAM approaches the target: shrink caches, lower artwork resolution, cap animation FPS. Never interrupts playback.", "memory saving optimize"),
    f!("ram_warnings", "RAM Warnings", Performance, Low, (0.0, 0.0), [], false, [true, true, true, true],
        "Warn before enabling a feature that may push memory over the target.", "warning"),
    f!("fps_counter", "FPS Counter", Performance, Low, (0.0, 0.1), [], false, [false, false, false, false], "Show rendering FPS in the corner.", "fps frame rate"),
    // ---------------- Privacy ----------------
    f!("diagnostic_logging", "Diagnostic Logging", Privacy, Low, (0.2, 2.0), [], false, [false, false, false, false],
        "Write a debug log to the data folder (restart required). Tokens and credentials are never logged.", "log debug"),
];

pub fn get(id: &str) -> Option<&'static FeatureDef> {
    FEATURES.iter().find(|f| f.id == id)
}

/// Features that require `id` (directly).
pub fn dependents(id: &str) -> impl Iterator<Item = &'static FeatureDef> + '_ {
    FEATURES.iter().filter(move |f| f.deps.contains(&id))
}

/// Combined estimate for a set of enabled features, accounting for shared resources:
/// sub-options sharing a parent contribute nothing, and texture-heavy background
/// features share one decoded image so only the largest of them is counted fully.
pub fn combined_estimate(enabled: &dyn Fn(&str) -> bool) -> (f32, f32) {
    let shared_bg = ["bg_album_art", "bg_image", "bg_blur", "bg_transitions"];
    let mut lo = 0.0;
    let mut hi = 0.0;
    let mut bg_max: (f32, f32) = (0.0, 0.0);
    let mut bg_extra = (0.0f32, 0.0f32);
    for f in FEATURES {
        if !enabled(f.id) || !f.deps.iter().all(|d| enabled(d)) {
            continue;
        }
        if shared_bg.contains(&f.id) {
            if f.est_mb.1 > bg_max.1 {
                bg_extra.0 += bg_max.0 * 0.3;
                bg_extra.1 += bg_max.1 * 0.3;
                bg_max = f.est_mb;
            } else {
                bg_extra.0 += f.est_mb.0 * 0.3;
                bg_extra.1 += f.est_mb.1 * 0.3;
            }
            continue;
        }
        lo += f.est_mb.0;
        hi += f.est_mb.1;
    }
    (lo + bg_max.0 + bg_extra.0, hi + bg_max.1 + bg_extra.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_unique_and_deps_exist() {
        let mut ids = std::collections::HashSet::new();
        for f in FEATURES {
            assert!(ids.insert(f.id), "duplicate id {}", f.id);
            assert!(f.est_mb.0 <= f.est_mb.1, "bad estimate {}", f.id);
        }
        for f in FEATURES {
            for d in f.deps {
                assert!(get(d).is_some(), "{} depends on unknown {}", f.id, d);
            }
        }
    }
    #[test]
    fn impact_matches_estimate() {
        for f in FEATURES {
            // the declared class must agree with the upper bound of the estimate
            // (one class of tolerance, as classes are calibrated by measurement later)
            let c = Impact::classify(f.est_mb.1);
            let diff = (c as i32 - f.impact as i32).abs();
            assert!(diff <= 1, "{}: declared {:?} vs estimate {:?}", f.id, f.impact, c);
        }
    }
    #[test]
    fn classify_ranges() {
        assert_eq!(Impact::classify(0.0), Impact::Low);
        assert_eq!(Impact::classify(5.0), Impact::Low);
        assert_eq!(Impact::classify(5.1), Impact::Moderate);
        assert_eq!(Impact::classify(15.1), Impact::High);
        assert_eq!(Impact::classify(36.0), Impact::VeryHigh);
    }
    #[test]
    fn search_finds_ram_lyrics_discord_background() {
        for q in ["RAM", "lyrics", "Discord", "background"] {
            assert!(FEATURES.iter().any(|f| f.matches(q)), "no match for {q}");
        }
    }
    #[test]
    fn combined_is_not_naive_sum() {
        let all = |_: &str| true;
        let naive: f32 = FEATURES.iter().map(|f| f.est_mb.1).sum();
        let (_, hi) = combined_estimate(&all);
        assert!(hi < naive);
    }
}
