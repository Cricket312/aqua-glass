//! Persistent configuration (TOML), versioning/migration, import/export, profiles.
//!
//! Secrets are never stored here: tokens live in the Windows Credential Manager
//! (see `security`). Exported settings therefore never contain credentials.

use crate::features::{self, Preset};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CONFIG_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default, Hash, PartialOrd, Ord)]
pub enum ProviderKind {
    #[default]
    Spotify,
    YouTube,
    Local,
}

impl ProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Spotify => "Spotify",
            ProviderKind::YouTube => "YouTube Music",
            ProviderKind::Local => "Local Files",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            ProviderKind::Spotify => "SP",
            ProviderKind::YouTube => "YT",
            ProviderKind::Local => "LO",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct General {
    pub last_settings_category: String,
    pub window_pos: Option<[f32; 2]>,
    pub window_size: Option<[f32; 2]>,
    pub ui_scale: f32,
    pub sidebar_width: f32,
}
impl Default for General {
    fn default() -> Self {
        Self {
            last_settings_category: "General".into(),
            window_pos: None,
            window_size: None,
            ui_scale: 1.0,
            sidebar_width: 210.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Appearance {
    pub theme: String,
    pub font_path: String,
    pub player_bar_layout: PlayerBarLayout,
    pub player_bar_height: f32,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "Aqua Glass Default".into(),
            font_path: default_font_path(),
            player_bar_layout: PlayerBarLayout::Classic,
            player_bar_height: 84.0,
        }
    }
}

fn default_font_path() -> String {
    if cfg!(windows) {
        r"C:\Windows\Fonts\segoeui.ttf".into()
    } else {
        String::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlayerBarLayout {
    #[default]
    Classic,
    Centered,
    Compact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BgScale {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Background {
    pub image_path: String,
    pub blur_radius: f32,
    pub dim: f32,
    pub opacity: f32,
    pub anim_speed: f32,
    pub anim_fps: u32,
    pub transition_ms: u32,
    pub scale: BgScale,
    pub parallax_px: f32,
}
impl Default for Background {
    fn default() -> Self {
        Self {
            image_path: String::new(),
            blur_radius: 24.0,
            dim: 0.55,
            opacity: 0.6,
            anim_speed: 1.0,
            anim_fps: 30,
            transition_ms: 600,
            scale: BgScale::Fill,
            parallax_px: 12.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Quality {
    Low96,
    #[default]
    Normal160,
    High320,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Playback {
    pub preferred_provider: ProviderKind,
    pub ask_when_both: bool,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub quality: Quality,
    pub fade_ms: u32,
    pub seek_step_s: u32,
    pub volume_step: f32,
}
impl Default for Playback {
    fn default() -> Self {
        Self {
            preferred_provider: ProviderKind::Spotify,
            ask_when_both: true,
            volume: 0.7,
            muted: false,
            shuffle: false,
            repeat: RepeatMode::Off,
            quality: Quality::Normal160,
            fade_ms: 150,
            seek_step_s: 10,
            volume_step: 0.05,
        }
    }
}

pub const EQ_FREQS: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Audio {
    /// Output device name; empty = system default.
    pub device: String,
    pub eq_gains_db: [f32; 10],
    pub eq_preset: String,
    pub bass_db: f32,
    /// -1 (left) .. 1 (right)
    pub balance: f32,
    pub buffer_ms: u32,
    pub preamp_db: f32,
}
impl Default for Audio {
    fn default() -> Self {
        Self {
            device: String::new(),
            eq_gains_db: [0.0; 10],
            eq_preset: "Flat".into(),
            bass_db: 6.0,
            balance: 0.0,
            buffer_ms: 500,
            preamp_db: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LyricsCfg {
    pub font_size: f32,
    pub bold_current: bool,
    pub color: [u8; 3],
    pub highlight: [u8; 3],
    pub align: Align,
    pub line_spacing: f32,
    pub bg_opacity: f32,
    pub offset_ms: i32,
    pub scroll_speed: f32,
}
impl Default for LyricsCfg {
    fn default() -> Self {
        Self {
            font_size: 22.0,
            bold_current: true,
            color: [150, 170, 185],
            highlight: [94, 234, 212],
            align: Align::Center,
            line_spacing: 10.0,
            bg_opacity: 0.0,
            offset_ms: 0,
            scroll_speed: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VisMode {
    #[default]
    Bars,
    Waveform,
    Circular,
    Minimal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VisCfg {
    pub mode: VisMode,
    pub fps: u32,
    pub sensitivity: f32,
    pub smoothing: f32,
    pub bars: usize,
    pub height: f32,
    pub color_a: [u8; 3],
    pub color_b: [u8; 3],
}
impl Default for VisCfg {
    fn default() -> Self {
        Self {
            mode: VisMode::Bars,
            fps: 30,
            sensitivity: 1.0,
            smoothing: 0.6,
            bars: 48,
            height: 140.0,
            color_a: [34, 211, 238],
            color_b: [59, 130, 246],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MiniCfg {
    pub pos: Option<[f32; 2]>,
    pub size: [f32; 2],
    pub opacity: f32,
    pub layout: MiniLayout,
}
impl Default for MiniCfg {
    fn default() -> Self {
        Self { pos: None, size: [360.0, 120.0], opacity: 0.95, layout: MiniLayout::Horizontal }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MiniLayout {
    #[default]
    Horizontal,
    Square,
    Compact,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OverlayCfg {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub opacity: f32,
    pub font_size: f32,
    pub always_on_top: bool,
}
impl Default for OverlayCfg {
    fn default() -> Self {
        Self { pos: [40.0, 40.0], size: [460.0, 110.0], opacity: 0.8, font_size: 18.0, always_on_top: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServicesCfg {
    pub spotify_enabled: bool,
    pub youtube_enabled: bool,
    pub local_enabled: bool,
    /// Optional own Spotify Developer client id for Web API calls.
    pub spotify_client_id: String,
    pub spotify_redirect_port: u16,
    /// Google Cloud OAuth client (Desktop app) id. The client secret is kept in Credential Manager.
    pub youtube_client_id: String,
    pub local_folders: Vec<String>,
    pub youtube_open_in: YtOpen,
    /// Discord application id used for Rich Presence (not a secret).
    pub discord_app_id: String,
}
impl Default for ServicesCfg {
    fn default() -> Self {
        Self {
            spotify_enabled: true,
            youtube_enabled: true,
            local_enabled: true,
            spotify_client_id: String::new(),
            spotify_redirect_port: 8898,
            youtube_client_id: String::new(),
            local_folders: Vec::new(),
            youtube_open_in: YtOpen::YouTubeMusic,
            discord_app_id: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum YtOpen {
    #[default]
    YouTubeMusic,
    YouTube,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SyncCfg {
    pub interval_min: u32,
    pub include_followed_playlists: bool,
}
impl Default for SyncCfg {
    fn default() -> Self {
        Self { interval_min: 60, include_followed_playlists: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PerfCfg {
    pub ram_target_mb: u32,
    pub artwork_max_px: u32,
    pub artwork_cache_mb: u32,
    pub idle_fps_cap: u32,
}
impl Default for PerfCfg {
    fn default() -> Self {
        Self { ram_target_mb: 200, artwork_max_px: 300, artwork_cache_mb: 24, idle_fps_cap: 30 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub preset: Preset,
    pub features: BTreeMap<String, bool>,
    /// Saved by "Disable All Visual Customizations" so it can be restored.
    pub saved_visual_profile: Option<BTreeMap<String, bool>>,
    pub user_presets: BTreeMap<String, BTreeMap<String, bool>>,
    pub general: General,
    pub appearance: Appearance,
    pub background: Background,
    pub playback: Playback,
    pub audio: Audio,
    pub lyrics: LyricsCfg,
    pub visualizer: VisCfg,
    pub mini: MiniCfg,
    pub overlay: OverlayCfg,
    pub services: ServicesCfg,
    pub sync: SyncCfg,
    pub perf: PerfCfg,
    pub shortcuts: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut c = Config {
            version: CONFIG_VERSION,
            preset: Preset::Balanced,
            features: BTreeMap::new(),
            saved_visual_profile: None,
            user_presets: BTreeMap::new(),
            general: General::default(),
            appearance: Appearance::default(),
            background: Background::default(),
            playback: Playback::default(),
            audio: Audio::default(),
            lyrics: LyricsCfg::default(),
            visualizer: VisCfg::default(),
            mini: MiniCfg::default(),
            overlay: OverlayCfg::default(),
            services: ServicesCfg::default(),
            sync: SyncCfg::default(),
            perf: PerfCfg::default(),
            shortcuts: crate::shortcuts::default_bindings(),
        };
        c.fill_missing_features();
        c
    }
}

impl Config {
    pub fn path() -> PathBuf {
        crate::util::data_dir().join("settings.toml")
    }

    /// Load from disk; invalid or outdated files are migrated or replaced by defaults
    /// (the broken file is kept as `settings.toml.bad` for inspection).
    pub fn load() -> (Config, Option<String>) {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(s) => match Self::from_toml_str(&s) {
                Ok(c) => (c, None),
                Err(e) => {
                    let _ = std::fs::copy(&path, path.with_extension("toml.bad"));
                    (Config::default(), Some(format!("Settings file was invalid and has been reset ({e}). A copy was kept as settings.toml.bad.")))
                }
            },
            Err(_) => (Config::default(), None),
        }
    }

    pub fn from_toml_str(s: &str) -> anyhow::Result<Config> {
        let mut raw: toml::Value = toml::from_str(s)?;
        migrate(&mut raw);
        let mut c: Config = raw.try_into()?;
        c.version = CONFIG_VERSION;
        c.fill_missing_features();
        c.sanitize();
        Ok(c)
    }

    pub fn to_toml_string(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        crate::util::atomic_write(&Self::path(), self.to_toml_string().as_bytes())
    }

    pub fn export_to(&self, path: &Path) -> std::io::Result<()> {
        // Config holds no secrets by construction; export is the same document.
        crate::util::atomic_write(path, self.to_toml_string().as_bytes())
    }

    pub fn import_from(path: &Path) -> anyhow::Result<Config> {
        let s = std::fs::read_to_string(path)?;
        Self::from_toml_str(&s)
    }

    pub fn fill_missing_features(&mut self) {
        for f in features::FEATURES {
            self.features.entry(f.id.to_string()).or_insert(f.default_value());
        }
        // Drop ids that no longer exist.
        self.features.retain(|k, _| features::get(k).is_some());
        for (k, v) in crate::shortcuts::default_bindings() {
            self.shortcuts.entry(k).or_insert(v);
        }
    }

    /// Clamp values so hand-edited files can't break the app.
    pub fn sanitize(&mut self) {
        let p = &mut self.playback;
        p.volume = p.volume.clamp(0.0, 1.0);
        p.volume_step = p.volume_step.clamp(0.01, 0.25);
        p.seek_step_s = p.seek_step_s.clamp(1, 120);
        p.fade_ms = p.fade_ms.min(3000);
        let a = &mut self.audio;
        for g in a.eq_gains_db.iter_mut() {
            *g = if g.is_finite() { g.clamp(-12.0, 12.0) } else { 0.0 };
        }
        a.bass_db = a.bass_db.clamp(0.0, 15.0);
        a.balance = a.balance.clamp(-1.0, 1.0);
        a.buffer_ms = a.buffer_ms.clamp(100, 3000);
        a.preamp_db = a.preamp_db.clamp(-12.0, 6.0);
        let v = &mut self.visualizer;
        v.fps = v.fps.clamp(5, 144);
        v.bars = v.bars.clamp(8, 128);
        v.smoothing = v.smoothing.clamp(0.0, 0.95);
        let pf = &mut self.perf;
        pf.ram_target_mb = pf.ram_target_mb.clamp(50, 4096);
        pf.artwork_max_px = pf.artwork_max_px.clamp(64, 1024);
        pf.artwork_cache_mb = pf.artwork_cache_mb.clamp(2, 512);
        pf.idle_fps_cap = pf.idle_fps_cap.clamp(5, 240);
        self.background.dim = self.background.dim.clamp(0.0, 1.0);
        self.background.opacity = self.background.opacity.clamp(0.0, 1.0);
        self.background.blur_radius = self.background.blur_radius.clamp(0.0, 80.0);
        self.background.anim_fps = self.background.anim_fps.clamp(5, 60);
        self.general.ui_scale = self.general.ui_scale.clamp(0.6, 2.5);
        self.general.sidebar_width = self.general.sidebar_width.clamp(150.0, 400.0);
        self.mini.opacity = self.mini.opacity.clamp(0.2, 1.0);
        self.overlay.opacity = self.overlay.opacity.clamp(0.1, 1.0);
        self.sync.interval_min = self.sync.interval_min.clamp(5, 24 * 60);
    }

    /// Raw toggle value.
    pub fn on(&self, id: &str) -> bool {
        self.features.get(id).copied().unwrap_or(false)
    }

    /// Toggle value with dependencies taken into account (what subsystems should use).
    pub fn active(&self, id: &str) -> bool {
        match features::get(id) {
            Some(f) => self.on(id) && f.deps.iter().all(|d| self.active(d)),
            None => false,
        }
    }

    pub fn set(&mut self, id: &str, v: bool) {
        if let Some(e) = self.features.get_mut(id) {
            if *e != v {
                *e = v;
                self.preset = Preset::Custom;
            }
        }
    }

    pub fn apply_preset(&mut self, p: Preset) {
        for f in features::FEATURES {
            if let Some(v) = f.preset_value(p) {
                self.features.insert(f.id.to_string(), v);
            }
        }
        self.preset = p;
    }

    pub fn apply_user_preset(&mut self, name: &str) -> bool {
        let Some(map) = self.user_presets.get(name).cloned() else { return false };
        for (k, v) in map {
            if features::get(&k).is_some() {
                self.features.insert(k, v);
            }
        }
        self.preset = Preset::Custom;
        true
    }

    /// Disable all visual customizations, remembering the previous state.
    /// Playback, accounts, library and playback preferences are untouched.
    pub fn disable_all_visual(&mut self) {
        let snapshot: BTreeMap<String, bool> = features::FEATURES
            .iter()
            .filter(|f| f.visual)
            .map(|f| (f.id.to_string(), self.on(f.id)))
            .collect();
        if snapshot.values().any(|v| *v) {
            self.saved_visual_profile = Some(snapshot);
        }
        for f in features::FEATURES.iter().filter(|f| f.visual) {
            self.features.insert(f.id.to_string(), false);
        }
        self.preset = Preset::Custom;
    }

    pub fn restore_visual(&mut self) -> bool {
        match self.saved_visual_profile.take() {
            Some(map) => {
                for (k, v) in map {
                    self.features.insert(k, v);
                }
                self.preset = Preset::Custom;
                true
            }
            None => false,
        }
    }

    pub fn reset_feature(&mut self, id: &str) {
        if let Some(f) = features::get(id) {
            self.features.insert(id.into(), f.default_value());
        }
    }

    pub fn reset_category(&mut self, cat: features::Category) {
        for f in features::FEATURES.iter().filter(|f| f.category == cat) {
            self.features.insert(f.id.into(), f.default_value());
        }
        use features::Category as C;
        let d = Config::default();
        match cat {
            C::General => self.general = d.general,
            C::Appearance | C::Themes => self.appearance = d.appearance,
            C::Backgrounds => self.background = d.background,
            C::Playback => self.playback = Playback { volume: self.playback.volume, ..d.playback },
            C::Audio => self.audio = d.audio,
            C::Lyrics => self.lyrics = d.lyrics,
            C::Visualizer => self.visualizer = d.visualizer,
            C::MiniPlayer => {
                self.mini = d.mini;
                self.overlay = d.overlay;
            }
            C::Shortcuts => self.shortcuts = d.shortcuts,
            C::Library => self.sync = d.sync,
            C::Performance => self.perf = d.perf,
            // Account / service configuration is never reset implicitly.
            _ => {}
        }
    }
}

/// Step-wise migration of older config documents.
fn migrate(raw: &mut toml::Value) {
    let Some(tbl) = raw.as_table_mut() else { return };
    let v = tbl.get("version").and_then(|v| v.as_integer()).unwrap_or(1);
    if v < 2 {
        // v1 stored the RAM target at the top level as `ram_target`.
        if let Some(t) = tbl.remove("ram_target") {
            let perf = tbl
                .entry("perf")
                .or_insert_with(|| toml::Value::Table(Default::default()));
            if let Some(p) = perf.as_table_mut() {
                p.insert("ram_target_mb".into(), t);
            }
        }
        // v1 named the blurred background toggle `blur_background`.
        if let Some(f) = tbl.get_mut("features").and_then(|f| f.as_table_mut()) {
            if let Some(b) = f.remove("blur_background") {
                f.insert("bg_blur".into(), b);
            }
        }
    }
    tbl.insert("version".into(), toml::Value::Integer(CONFIG_VERSION as i64));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        c.set("visualizer", true);
        c.perf.ram_target_mb = 150;
        let s = c.to_toml_string();
        let back = Config::from_toml_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn migrates_v1() {
        let s = "version = 1\nram_target = 123\n[features]\nblur_background = true\nlyrics = false\n";
        let c = Config::from_toml_str(s).unwrap();
        assert_eq!(c.perf.ram_target_mb, 123);
        assert!(c.on("bg_blur"));
        assert!(!c.on("lyrics"));
        assert_eq!(c.version, CONFIG_VERSION);
    }

    #[test]
    fn invalid_values_are_clamped_and_unknown_ignored() {
        let s = "[playback]\nvolume = 7.0\n[features]\nnot_a_feature = true\n[audio]\neq_gains_db=[99,0,0,0,0,0,0,0,0,-99]\n";
        let c = Config::from_toml_str(s).unwrap();
        assert_eq!(c.playback.volume, 1.0);
        assert!(!c.features.contains_key("not_a_feature"));
        assert_eq!(c.audio.eq_gains_db[0], 12.0);
        assert_eq!(c.audio.eq_gains_db[9], -12.0);
    }

    #[test]
    fn garbage_is_error() {
        assert!(Config::from_toml_str("this is = = not toml").is_err());
    }

    #[test]
    fn disable_and_restore_visual() {
        let mut c = Config::default();
        c.apply_preset(Preset::FullyCustomized);
        let vol = c.playback.volume;
        let before = c.features.clone();
        c.disable_all_visual();
        for f in features::FEATURES.iter().filter(|f| f.visual) {
            assert!(!c.on(f.id), "{} still on", f.id);
        }
        // non-visual untouched
        assert_eq!(c.on("media_keys"), before["media_keys"]);
        assert_eq!(c.playback.volume, vol);
        assert!(c.restore_visual());
        assert_eq!(c.features, before);
    }

    #[test]
    fn presets_and_active_deps() {
        let mut c = Config::default();
        c.apply_preset(Preset::Minimal);
        assert!(!c.on("visualizer"));
        c.apply_preset(Preset::FullyCustomized);
        assert!(c.on("visualizer"));
        c.set("lyrics", false);
        assert_eq!(c.preset, Preset::Custom);
        assert!(c.on("lyrics_synced"));
        assert!(!c.active("lyrics_synced"));
    }

    #[test]
    fn user_presets() {
        let mut c = Config::default();
        let mut m = BTreeMap::new();
        m.insert("visualizer".to_string(), true);
        c.user_presets.insert("Mine".into(), m);
        assert!(c.apply_user_preset("Mine"));
        assert!(c.on("visualizer"));
        assert!(!c.apply_user_preset("Nope"));
    }

    #[test]
    fn export_contains_no_secrets() {
        let c = Config::default();
        let s = c.to_toml_string().to_lowercase();
        assert!(!s.contains("token"));
        assert!(!s.contains("secret"));
    }
}
