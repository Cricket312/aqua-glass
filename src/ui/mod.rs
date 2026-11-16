//! The egui/eframe user interface.

mod background;
mod browse;
mod jobs;
mod pages;
mod shell;
mod settings;
mod transfer_ui;
mod viewports;
pub mod widgets;

use crate::artwork::{Artwork, Kind};
use crate::audio::engine::{Cmd, Engine, EngineOpts, PlayerState};
use crate::audio::output::Output;
use crate::audio::visualizer::Analyzer;
use crate::config::{Config, ProviderKind};
use crate::features;
use crate::library::LibraryStore;
use crate::model::*;
use crate::perf;
use crate::services::lyrics::Lyrics;
use crate::services::spotify::SpotifyApi;
use crate::shortcuts::{Action, Binding};
use crate::theme::{self, Theme};
use crate::winint::{self, UiCmd};
use eframe::egui::{self, Color32, RichText};
use jobs::{Jobs, Msg};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Home,
    Search,
    Library,
    Playlists,
    Artists,
    Albums,
    NowPlaying,
    Discover,
    Queue,
    Transfer,
    Settings,
    Playlist(ProviderKind, String),
    Album(Album),
    Artist(Artist),
}

/// Where playback was started from (decides whether Similarity Radio starts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayFrom {
    Search,
    Artist,
    Album,
    Radio,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Only(ProviderKind),
}

impl Filter {
    pub fn allows(self, p: ProviderKind) -> bool {
        match self {
            Filter::All => true,
            Filter::Only(x) => x == p,
        }
    }
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub gen: u64,
    pub results: HashMap<ProviderKind, Result<SearchResults, String>>,
    pub pending: usize,
    pub library_hits: Vec<Track>,
    pub focus: bool,
}

#[derive(Default)]
pub struct ViewState {
    /// Live tracks for a non-imported playlist / album (not cached on disk).
    pub live: HashMap<String, Result<Vec<Track>, String>>,
    pub loading: std::collections::HashSet<String>,
    pub filter: String,
    pub artist_albums: HashMap<String, Result<Vec<Album>, String>>,
}

#[derive(Default)]
pub struct LyricsState {
    pub key: String,
    pub current: Option<Result<Lyrics, String>>,
    pub cache: HashMap<String, Result<Lyrics, String>>,
    pub loading: std::collections::HashSet<String>,
    pub fullscreen: bool,
    pub shared: Arc<Mutex<Option<Lyrics>>>,
}

pub struct Integrations {
    pub tray: Option<winint::Tray>,
    pub media: Option<winint::Media>,
    pub hotkeys: Option<winint::Hotkeys>,
    pub hotkey_errors: Vec<String>,
    pub notify: Arc<AtomicBool>,
    pub errors: Vec<String>,
}

pub struct AquaApp {
    pub cfg: Config,
    /// Config as last applied to subsystems (for change detection).
    applied: Option<Config>,
    cfg_dirty_at: Option<Instant>,
    saved_cfg: Config,
    pub themes: Vec<Theme>,
    pub theme_errors: Vec<String>,
    pub edit_theme: Theme,
    pub page: Page,
    pub back: Vec<Page>,
    pub forward: Vec<Page>,
    pub featured_idx: usize,
    /// Settings signature Soloist was last started with (see `ensure_soloist`).
    pub soloist_sig: Option<u64>,
    /// Equalizer APO: config folder (None = not installed), whether config.txt
    /// includes our file, text to write after a short pause, last text written.
    pub apo_dir: Option<std::path::PathBuf>,
    pub apo_linked: bool,
    pub apo_pending: Option<(String, Instant)>,
    pub apo_last: Option<String>,
    pub apo_error: Option<String>,
    /// Playing state the Equalizer APO file was last computed for.
    pub apo_playing_seen: bool,
    pub filter: Filter,
    pub engine: Engine,
    pub out: Output,
    pub library: Arc<Mutex<LibraryStore>>,
    pub spotify: Arc<SpotifyApi>,
    pub sp_profile: Option<crate::services::spotify::Profile>,
    pub sp_status: String,
    pub jobs: Jobs,
    msg_rx: mpsc::Receiver<Msg>,
    ui_rx: mpsc::Receiver<UiCmd>,
    pub ui_tx: mpsc::Sender<UiCmd>,
    pub artwork: Arc<Mutex<Artwork>>,
    pub monitor: perf::Monitor,
    pub measurements: perf::Measurements,
    pending_measure: Option<perf::PendingMeasure>,
    pub pressure: perf::Pressure,
    pub optimizer_log: VecDeque<String>,
    pub toasts: Vec<Toast>,
    pub search: SearchState,
    pub view: ViewState,
    pub lyrics: LyricsState,
    pub analyzer: Option<Analyzer>,
    vis_buf: Vec<f32>,
    vis_wanted: bool,
    vis_last: Instant,
    pub ints: Integrations,
    pub hwnd: Option<isize>,
    pub mini_open: Arc<AtomicBool>,
    pub settings: settings::SettingsUi,
    pub transfer: transfer_ui::TransferUi,
    frame_times: VecDeque<f32>,
    last_frame: Instant,
    pub fps: f32,
    sync_next_at: Instant,
    pub confirm: Option<Confirm>,
    pub alternatives: Option<(Track, Option<Result<Vec<crate::matching::Candidate>, String>>)>,
    pub bg: background::BgState,
    last_state_version: u64,
    pub state: PlayerState,
    pub start_hidden: bool,
    first_frame: bool,
    font_applied: Option<String>,
    quitting: bool,
    pub local_scanning: bool,
    pub accent_override: Option<[u8; 3]>,
    pub vis_shared: viewports::Shared,
    /// Main window hidden to the tray.
    pub main_hidden: bool,
    pub radio: Option<crate::radio::Radio>,
    radio_prefs_last: Option<crate::radio::Prefs>,
    pub history: Arc<Mutex<crate::history::History>>,
    /// Track keys liked in this session or present in a cached Liked collection.
    pub liked_keys: std::collections::HashSet<String>,
    pub lastfm_key: String,
    pub lastfm_input: String,
    pub top_search: String,
}

pub enum Confirm {
    /// RAM warning before enabling a feature.
    EnableFeature(String),
    RemoveImported(ProviderKind, String, String),
    DisconnectSpotify,
    ResetAll,
    RemoveTransferred(crate::transfer::TransferLog),
}

impl AquaApp {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, cfg_warning: Option<String>, start_hidden: bool, safe_mode: bool) -> AquaApp {
        let ctx = cc.egui_ctx.clone();
        crate::services::install_crypto();
        let hwnd = window_handle(cc);
        let out = Output::start(&cfg.audio.device, cfg.audio.buffer_ms);
        let c2 = ctx.clone();
        let engine = Engine::start(out.clone(), engine_opts(&cfg), cfg.playback.volume, cfg.playback.muted, Box::new(move || c2.request_repaint()));
        let (jobs, msg_rx) = Jobs::new(ctx.clone());
        let (ui_tx, ui_rx) = mpsc::channel();
        let mut lib = LibraryStore::open(crate::util::data_dir().join("library"));
        lib.keep_in_memory = cfg.active("metadata_cache");
        let library = Arc::new(Mutex::new(lib));
        let spotify = Arc::new(SpotifyApi::new(&cfg.services.spotify_client_id, cfg.services.spotify_redirect_port));
        let mut art = Artwork::new(ctx.clone(), cfg.perf.artwork_cache_mb, cfg.perf.artwork_max_px);
        art.disk_cache = cfg.active("artwork_disk_cache");
        let (themes, theme_errors) = all_themes();
        let edit_theme = themes.iter().find(|t| t.name == cfg.appearance.theme).cloned().unwrap_or_default();

        // integrations driven by engine listeners (work while the window is hidden)
        let notify = Arc::new(AtomicBool::new(false));
        {
            let n = notify.clone();
            let mut last_key = String::new();
            engine.add_listener(Box::new(move |st: &PlayerState| {
                let key = st.current.as_ref().map(|t| t.key()).unwrap_or_default();
                if key != last_key {
                    last_key = key;
                    if n.load(Ordering::Relaxed) && st.playing {
                        if let Some(t) = &st.current {
                            winint::notify(&t.title, &format!("{} · {}", t.artist_line(), t.provider.label()));
                        }
                    }
                }
            }));
        }

        // local listening history (recorded from the engine thread, so it works while hidden)
        let history = Arc::new(Mutex::new(crate::history::History::load()));
        {
            let h = history.clone();
            engine.add_listener(Box::new(move |st: &PlayerState| {
                if let Some(r) = &st.last_played {
                    let mut g = h.lock().unwrap();
                    if g.record(r, crate::util::now_unix()) {
                        g.save();
                    }
                }
            }));
        }

        let mut app = AquaApp {
            saved_cfg: cfg.clone(),
            cfg,
            applied: None,
            cfg_dirty_at: None,
            themes,
            theme_errors,
            edit_theme,
            page: Page::Home,
            back: Vec::new(),
            forward: Vec::new(),
            featured_idx: 0,
            soloist_sig: None,
            apo_dir: None,
            apo_linked: false,
            apo_pending: None,
            apo_last: None,
            apo_error: None,
            apo_playing_seen: false,
            filter: Filter::All,
            engine,
            out,
            library,
            spotify,
            sp_profile: None,
            sp_status: "Not signed in".into(),
            jobs,
            msg_rx,
            ui_rx,
            ui_tx,
            artwork: Arc::new(Mutex::new(art)),
            monitor: perf::Monitor::new(),
            measurements: perf::Measurements::load(),
            pending_measure: None,
            pressure: perf::Pressure::Normal,
            optimizer_log: VecDeque::new(),
            toasts: Vec::new(),
            search: SearchState::default(),
            view: ViewState::default(),
            lyrics: LyricsState::default(),
            analyzer: None,
            vis_buf: Vec::new(),
            vis_wanted: false,
            vis_last: Instant::now(),
            ints: Integrations { tray: None, media: None, hotkeys: None, hotkey_errors: vec![], notify, errors: vec![] },
            hwnd,
            mini_open: Arc::new(AtomicBool::new(false)),
            settings: settings::SettingsUi::default(),
            transfer: transfer_ui::TransferUi::default(),
            frame_times: VecDeque::new(),
            last_frame: Instant::now(),
            fps: 0.0,
            sync_next_at: Instant::now() + Duration::from_secs(20),
            confirm: None,
            alternatives: None,
            bg: background::BgState::default(),
            last_state_version: u64::MAX,
            state: PlayerState::default(),
            start_hidden,
            first_frame: true,
            font_applied: None,
            quitting: false,
            local_scanning: false,
            accent_override: None,
            vis_shared: viewports::Shared::default(),
            main_hidden: false,
            radio: None,
            radio_prefs_last: None,
            history,
            liked_keys: Default::default(),
            lastfm_key: crate::security::get(crate::security::keys::LASTFM_API_KEY).unwrap_or_default(),
            lastfm_input: String::new(),
            top_search: String::new(),
        };
        app.settings.category = features::Category::ALL
            .into_iter()
            .find(|c| c.label() == app.cfg.general.last_settings_category)
            .unwrap_or(features::Category::General);
        if let Some(w) = cfg_warning {
            app.toast(w, true);
        }
        for e in app.theme_errors.clone() {
            app.toast(format!("Invalid theme skipped: {e}"), true);
        }
        app.engine.send(Cmd::SetRepeat(app.cfg.playback.repeat));
        app.engine.send(Cmd::SetShuffle(app.cfg.playback.shuffle));
        app.remove_legacy_youtube_data();
        app.detect_apo();
        if safe_mode {
            app.toast("Safe mode: the Spotify session and the saved queue were not restored. Sign in again from Settings › Music Services when ready.", false);
        } else {
            app.restore_sessions();
            app.restore_queue();
        }
        if crate::crash::log_path().exists() {
            // shown once; the report is kept as crash-last.log
            let seen = crate::util::data_dir().join("crash-last.log");
            let _ = std::fs::rename(crate::crash::log_path(), &seen);
            app.toast(format!("Aqua Glass crashed last time. Details: {}", seen.display()), true);
        }
        // first start with local folders configured: build the local library
        let needs_scan = app.cfg.services.local_enabled && !app.cfg.services.local_folders.is_empty() && app.library.lock().unwrap().lib(ProviderKind::Local).last_sync == 0;
        if needs_scan {
            app.scan_local();
        }
        app
    }

    /// Handle command-line files / start page.
    pub fn open_cli(&mut self, files: Vec<std::path::PathBuf>, page: Option<&str>) {
        #[cfg(feature = "local-files")]
        {
            let tracks: Vec<Track> = files.iter().filter_map(|f| crate::audio::local::probe_track(f)).collect();
            if !tracks.is_empty() {
                self.engine.send(Cmd::Play { tracks, start: 0 });
            }
        }
        #[cfg(not(feature = "local-files"))]
        let _ = files;
        if let Some(p) = page {
            match p {
                "home" => self.page = Page::Home,
                "search" => self.page = Page::Search,
                "library" => self.page = Page::Library,
                "now-playing" => self.page = Page::NowPlaying,
                "queue" => self.page = Page::Queue,
                "transfer" => self.page = Page::Transfer,
                "settings" => self.page = Page::Settings,
                "performance" => {
                    self.page = Page::Settings;
                    self.settings.category = features::Category::Performance;
                }
                _ => {}
            }
        }
    }

    pub fn radio_prefs(&self) -> crate::radio::Prefs {
        crate::radio::Prefs::from_user(&self.cfg.radio_prefs(), &self.lastfm_key)
    }

    pub fn radio_status(&self) -> Option<crate::radio::Status> {
        self.radio.as_ref().map(|r| r.status.lock().unwrap().clone())
    }

    pub fn radio_send(&self, m: crate::radio::Msg) {
        if let Some(r) = &self.radio {
            r.send(m);
        }
    }

    /// Play a single song from search results — starts Similarity Radio when enabled.
    pub fn play_single(&mut self, t: Track, from: PlayFrom) {
        if !self.provider_enabled(t.provider) {
            self.toast(format!("{} is disabled in Settings › Music Services.", t.provider.label()), true);
            return;
        }
        self.engine.send(Cmd::Play { tracks: vec![t.clone()], start: 0 });
        let start_radio = self.radio.is_some()
            && match from {
                PlayFrom::Search => self.cfg.active("radio_from_search"),
                PlayFrom::Artist => self.cfg.active("radio_from_artist"),
                PlayFrom::Album => self.cfg.active("radio_from_album"),
                PlayFrom::Radio => true,
                PlayFrom::Other => false,
            };
        if start_radio {
            self.radio_send(crate::radio::Msg::Start(t));
        } else {
            self.radio_send(crate::radio::Msg::Stop);
        }
    }

    /// Play a list (playlist/album/...). Radio never overwrites list playback; with
    /// "Activate from Album/Artist Pages" recommendations follow *after* the list.
    pub fn play_list(&mut self, tracks: Vec<Track>, start: usize, from: PlayFrom) {
        let seed = tracks.get(start).cloned();
        self.play(tracks, start);
        let follow = self.radio.is_some()
            && match from {
                PlayFrom::Album => self.cfg.active("radio_from_album"),
                PlayFrom::Artist => self.cfg.active("radio_from_artist"),
                _ => false,
            };
        match (follow, seed) {
            (true, Some(s)) => self.radio_send(crate::radio::Msg::Start(s)),
            _ => self.radio_send(crate::radio::Msg::Stop),
        }
    }

    pub fn is_liked(&self, t: &Track) -> bool {
        self.liked_keys.contains(&t.key())
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        let text = crate::security::scrub(&text.into());
        if error {
            log::warn!("{text}");
        }
        self.toasts.push(Toast { text, error, at: Instant::now() });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    pub fn mark_dirty(&mut self) {
        self.cfg_dirty_at = Some(Instant::now());
    }

    pub fn navigate(&mut self, p: Page) {
        if self.page != p {
            self.forward.clear();
            let prev = std::mem::replace(&mut self.page, p);
            self.back.push(prev);
            if self.back.len() > 30 {
                self.back.remove(0);
            }
        }
        if !self.cfg.active("metadata_cache") {
            // free track lists not shown anymore
            let keep = match &self.page {
                Page::Playlist(p, id) => Some((*p, id.clone())),
                _ => None,
            };
            self.library.lock().unwrap().evict_except(keep.as_ref().map(|(p, i)| (*p, i.as_str())));
            self.view.live.clear();
        }
    }

    pub fn go_back(&mut self) {
        if let Some(p) = self.back.pop() {
            let cur = std::mem::replace(&mut self.page, p);
            self.forward.push(cur);
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(p) = self.forward.pop() {
            let cur = std::mem::replace(&mut self.page, p);
            self.back.push(cur);
        }
    }

    pub fn accent(&self) -> Color32 {
        theme::c(self.accent_override.unwrap_or_else(|| self.current_theme().accent))
    }

    pub fn current_theme(&self) -> &Theme {
        self.themes.iter().find(|t| t.name == self.cfg.appearance.theme).unwrap_or(&self.themes[0])
    }

    // ------------------------------------------------------------------
    // Accounts

    fn restore_sessions(&mut self) {
        // with your own Spotify app the Web API sign-in is stored under a separate key
        let sp_key = if self.spotify.uses_default_client() { crate::security::keys::SPOTIFY_REFRESH } else { crate::security::keys::SPOTIFY_WEBAPI_REFRESH };
        if self.cfg.services.spotify_enabled && crate::security::get(sp_key).is_some() {
            log::info!("restoring Spotify session (custom app: {})", !self.spotify.uses_default_client());
            self.sp_status = "Restoring session…".into();
            let api = self.spotify.clone();
            self.jobs.spawn("Spotify: restore session", move |h| {
                let r = api.restore().map_err(|e| e.to_string()).and_then(|ok| if ok { api.me().map(Some).map_err(|e| e.to_string()) } else { Ok(None) });
                h.send(Msg::SpotifyAuth(r));
            });
        }
    }

    /// YouTube Music support was removed in 0.6. Once, on the first start of 0.6:
    /// delete stored Google credentials, the cached YouTube library and YouTube
    /// plays from the listening history. Everything else is untouched.
    fn remove_legacy_youtube_data(&mut self) {
        let marker = crate::util::data_dir().join("youtube-removed");
        if marker.exists() {
            return;
        }
        crate::security::delete(crate::security::keys::YOUTUBE_REFRESH);
        crate::security::delete(crate::security::keys::YOUTUBE_CLIENT_SECRET);
        self.library.lock().unwrap().clear_provider(ProviderKind::YouTube);
        {
            let mut h = self.history.lock().unwrap();
            let before = h.events.len();
            h.events.retain(|e| e.track.provider != ProviderKind::YouTube);
            if h.events.len() != before {
                h.save();
            }
        }
        let _ = crate::util::atomic_write(&marker, b"YouTube Music data removed by Aqua Glass 0.6\n");
        log::info!("removed legacy YouTube Music data");
    }

    /// Soloist's data folder: its own device identity and stored Spotify Connect session.
    pub fn soloist_data_dir(&self) -> std::path::PathBuf {
        let d = self.cfg.services.soloist_data_dir.trim();
        if d.is_empty() { crate::util::data_dir().join("soloist") } else { std::path::PathBuf::from(d) }
    }

    /// Start, restart or stop Spotify Soloist to match the settings. Only acts when
    /// something relevant changed (or `force`), so it's cheap to call every frame.
    pub fn ensure_soloist(&mut self, force: bool) {
        use std::hash::{Hash, Hasher};
        let s = &self.cfg.services;
        let want = s.spotify_enabled && s.spotify_backend == crate::config::SpotifyBackend::Soloist && (!s.soloist_manage || !s.soloist_path.trim().is_empty());
        let key = crate::security::get(crate::security::keys::SOLOIST_API_KEY).unwrap_or_default();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (want, s.soloist_path.trim(), s.soloist_device_name.trim(), s.soloist_manage, self.soloist_data_dir(), &key).hash(&mut h);
        let sig = h.finish();
        if !force && self.soloist_sig == Some(sig) {
            return;
        }
        self.soloist_sig = Some(sig);
        if !want {
            self.engine.send(Cmd::SoloistStop);
            return;
        }
        let name = if s.soloist_device_name.trim().is_empty() { "Aqua Glass".to_string() } else { s.soloist_device_name.trim().to_string() };
        self.engine.send(Cmd::SoloistStart(crate::audio::soloist::SoloistConfig {
            exe: std::path::PathBuf::from(s.soloist_path.trim()),
            api_key: key,
            device_name: name,
            data_dir: self.soloist_data_dir(),
            manage: s.soloist_manage,
        }));
    }

    /// Open the album of a track (falls back to Now Playing when the album is unknown).
    pub fn open_album_of(&mut self, t: &Track) {
        if t.album_id.is_empty() || t.album.is_empty() || t.provider == ProviderKind::YouTube {
            self.navigate(Page::NowPlaying);
            return;
        }
        self.navigate(Page::Album(crate::model::Album {
            provider: t.provider,
            id: t.album_id.clone(),
            name: t.album.clone(),
            artists: t.artists.clone(),
            artwork_url: t.artwork_url.clone(),
            ..Default::default()
        }));
    }

    /// Open the page of the `i`-th artist of a track. Spotify: the artist page (looked
    /// up by name when the track has no artist ids); local files: a library search.
    pub fn open_artist_of(&mut self, t: &Track, i: usize) {
        let Some(name) = t.artists.get(i).cloned() else { return };
        match t.provider {
            ProviderKind::Spotify => {
                let id = t.artist_ids.get(i).cloned().unwrap_or_default();
                self.navigate(Page::Artist(crate::model::Artist { provider: ProviderKind::Spotify, id, name, ..Default::default() }));
            }
            _ => {
                self.search.query = name;
                self.navigate(Page::Search);
                self.run_search();
            }
        }
    }

    /// Look for Equalizer APO (startup and the settings "Check again" button).
    pub fn detect_apo(&mut self) {
        self.apo_dir = crate::eqapo::config_dir();
        self.apo_linked = self.apo_dir.as_deref().is_some_and(crate::eqapo::is_linked);
        self.apo_last = None; // rewrite our file on the next change
        self.applied = None; // re-run apply_config so the current settings are written
    }

    /// Compute what Equalizer APO should apply now and queue it if it changed.
    fn apo_queue(&mut self) {
        self.apo_playing_seen = self.state.playing;
        if !self.apo_linked {
            return;
        }
        let c = &self.cfg;
        let dsp = crate::audio::dsp::DspParams {
            eq_on: c.active("eq"),
            eq_gains_db: c.audio.eq_gains_db,
            bass_on: c.active("bass_boost"),
            bass_db: c.audio.bass_db,
            preamp_db: c.audio.preamp_db,
            balance: c.audio.balance,
        };
        let engine = c.audio.eq_engine == crate::config::EqEngine::EqualizerApo;
        let active = engine && (self.state.playing || !c.audio.apo_only_while_playing);
        let text = crate::eqapo::render(&dsp, active);
        if self.apo_last.as_ref() != Some(&text) {
            self.apo_pending = Some((text, Instant::now()));
        } else {
            self.apo_pending = None;
        }
    }

    /// Write pending Equalizer APO settings once the user pauses (150 ms), so
    /// dragging a slider doesn't make Equalizer APO reload 60 times a second.
    fn apo_tick(&mut self, ctx: &egui::Context) {
        let Some((text, at)) = &self.apo_pending else { return };
        let wait = Duration::from_millis(150).saturating_sub(at.elapsed());
        if !wait.is_zero() {
            ctx.request_repaint_after(wait);
            return;
        }
        let text = text.clone();
        self.apo_pending = None;
        let Some(dir) = self.apo_dir.clone() else { return };
        match crate::eqapo::write(&dir, &text) {
            Ok(()) => {
                self.apo_last = Some(text);
                self.apo_error = None;
            }
            Err(e) => self.apo_error = Some(crate::eqapo::explain(&e, &dir)),
        }
    }

    pub fn spotify_login(&mut self) {
        self.sp_status = "Waiting for browser sign-in…".into();
        let api = self.spotify.clone();
        self.jobs.spawn("Spotify: sign in", move |h| {
            let r = api.login().map_err(|e| e.to_string()).and_then(|_| api.me().map(Some).map_err(|e| e.to_string()));
            h.send(Msg::SpotifyAuth(r));
        });
    }

    pub fn spotify_logout(&mut self, forget_library: bool) {
        self.spotify.logout();
        self.engine.send(Cmd::SpotifyDisconnect);
        self.sp_profile = None;
        self.sp_status = "Signed out".into();
        if forget_library {
            self.library.lock().unwrap().clear_provider(ProviderKind::Spotify);
        }
    }

    pub fn sync_now(&mut self, p: ProviderKind, import_all: bool) {
        let label = format!("Sync {}", p.label());
        if self.jobs.is_running(&label) {
            return;
        }
        let store = self.library.clone();
        match p {
            ProviderKind::Spotify if self.spotify.is_signed_in() => {
                let api = self.spotify.clone();
                let followed = self.cfg.sync.include_followed_playlists;
                self.jobs.spawn(label, move |h| {
                    let r = jobs::sync_spotify(&api, &store, followed, import_all, &h);
                    h.send(Msg::SyncDone { provider: ProviderKind::Spotify, res: r });
                });
            }
            ProviderKind::Local => self.scan_local(),
            _ => self.toast(format!("Sign in to {} first (Settings › Music Services).", p.label()), true),
        }
    }

    pub fn scan_local(&mut self) {
        #[cfg(feature = "local-files")]
        {
            if self.local_scanning {
                return;
            }
            self.local_scanning = true;
            let folders = self.cfg.services.local_folders.clone();
            self.jobs.spawn("Scan local files", move |h| {
                let t = crate::audio::local::scan(&folders, &|n| h.progress(n, 0));
                h.send(Msg::LocalScanned(t));
            });
        }
    }

    // ------------------------------------------------------------------
    // Playback helpers

    pub fn play(&mut self, tracks: Vec<Track>, start: usize) {
        if tracks.is_empty() {
            return;
        }
        if let Some(t) = tracks.get(start) {
            if !self.provider_enabled(t.provider) {
                self.toast(format!("{} is disabled in Settings › Music Services.", t.provider.label()), true);
                return;
            }
        }
        self.engine.send(Cmd::Play { tracks, start });
    }

    pub fn provider_enabled(&self, p: ProviderKind) -> bool {
        match p {
            ProviderKind::Spotify => self.cfg.services.spotify_enabled,
            // YouTube Music support was removed in 0.6; old entries stay hidden
            ProviderKind::YouTube => false,
            ProviderKind::Local => self.cfg.services.local_enabled,
        }
    }

    pub fn like(&mut self, t: &Track) {
        self.liked_keys.insert(t.key());
        self.radio_send(crate::radio::Msg::Liked(t.clone()));
        let t = t.clone();
        match t.provider {
            ProviderKind::Spotify => {
                let api = self.spotify.clone();
                self.jobs.spawn("Like on Spotify", move |h| {
                    let m = match api.like(&t.id) {
                        Ok(()) => Msg::Toast(format!("Saved “{}” to Liked Songs", t.title), false),
                        Err(e) => Msg::Toast(format!("Could not like: {e}"), true),
                    };
                    h.send(m);
                });
            }
            ProviderKind::YouTube => self.toast("YouTube Music is no longer supported.", true),
            ProviderKind::Local => self.toast("Local files can't be liked.", true),
        }
    }

    /// Look up the same recording on Spotify (never auto-substituted).
    pub fn find_alternatives(&mut self, t: &Track) {
        let target = ProviderKind::Spotify;
        if t.provider == ProviderKind::Spotify {
            self.alternatives = Some((t.clone(), Some(Err("This song is already on Spotify.".into()))));
            return;
        }
        self.alternatives = Some((t.clone(), None));
        let src = t.clone();
        let q = crate::matching::query_for(t);
        match target {
            ProviderKind::Spotify if self.spotify.is_signed_in() => {
                let api = self.spotify.clone();
                self.jobs.spawn("Find on Spotify", move |h| {
                    let r = api.search_tracks(&q, 8).map(|c| crate::matching::rank(&src, c).0).map_err(|e| e.to_string());
                    h.send(Msg::Alternatives { src, res: r });
                });
            }
            _ => self.alternatives = Some((t.clone(), Some(Err(format!("Sign in to {} to look for this track there.", target.label()))))),
        }
    }

    pub fn run_search(&mut self) {
        let q = self.search.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.search.gen += 1;
        self.search.results.clear();
        self.search.pending = 0;
        let gen = self.search.gen;
        self.search.library_hits = self.library.lock().unwrap().all_tracks_search(&q, 50);
        if self.filter.allows(ProviderKind::Spotify) && self.cfg.services.spotify_enabled && self.spotify.is_signed_in() {
            self.search.pending += 1;
            let api = self.spotify.clone();
            let q2 = q.clone();
            self.jobs.spawn("Search Spotify", move |h| {
                h.send(Msg::Search { gen, provider: ProviderKind::Spotify, res: api.search(&q2).map_err(|e| e.to_string()) });
            });
        }
        if self.filter.allows(ProviderKind::Local) && self.cfg.services.local_enabled {
            let mut s = self.library.lock().unwrap();
            let local = s.tracks(ProviderKind::Local, crate::library::LOCAL_ALL_ID).cloned().unwrap_or_default();
            let ql = q.to_lowercase();
            let hits: Vec<Track> = local
                .into_iter()
                .filter(|t| t.title.to_lowercase().contains(&ql) || t.artist_line().to_lowercase().contains(&ql) || t.album.to_lowercase().contains(&ql))
                .take(50)
                .collect();
            drop(s);
            self.search.results.insert(ProviderKind::Local, Ok(SearchResults { tracks: hits, ..Default::default() }));
        }
    }

    /// Tracks of a playlist: imported › disk cache; otherwise fetched live (not stored).
    pub fn playlist_tracks(&mut self, p: ProviderKind, id: &str) -> Option<Result<Vec<Track>, String>> {
        if let Some(t) = self.library.lock().unwrap().tracks(p, id) {
            return Some(Ok(t.clone()));
        }
        let key = format!("{p:?}:{id}");
        if let Some(r) = self.view.live.get(&key) {
            return Some(r.clone());
        }
        if !self.view.loading.contains(&key) {
            self.view.loading.insert(key);
            let id2 = id.to_string();
            match p {
                ProviderKind::Spotify => {
                    let api = self.spotify.clone();
                    self.jobs.spawn("Load playlist", move |h| {
                        let r = if id2 == crate::library::LIKED_ID { api.liked_tracks() } else { api.playlist_items(&id2) };
                        h.send(Msg::PlaylistTracks { provider: p, id: id2, res: r.map_err(|e| e.to_string()) });
                    });
                }
                ProviderKind::YouTube | ProviderKind::Local => {}
            }
        }
        None
    }

    // ------------------------------------------------------------------
    // Feature toggles

    /// Request a toggle change; may open the RAM warning dialog.
    pub fn request_toggle(&mut self, id: &str, value: bool) {
        if value && self.cfg.active("ram_warnings") {
            if let Some(f) = features::get(id) {
                let est = self.measurements.by_feature.get(id).map(|m| m.delta_mb).unwrap_or(f.est_mb.1);
                if self.monitor.current_mb() + est > self.cfg.perf.ram_target_mb as f32 && est > 0.5 {
                    self.confirm = Some(Confirm::EnableFeature(id.to_string()));
                    return;
                }
            }
        }
        self.set_feature(id, value);
    }

    pub fn set_feature(&mut self, id: &str, value: bool) {
        self.cfg.set(id, value);
        self.mark_dirty();
        if value {
            // measure the actual effect once it settles
            self.pending_measure = Some(perf::PendingMeasure { id: id.to_string(), baseline: perf::sample().private_ws, at: Instant::now() });
        } else {
            self.pending_measure = None;
        }
    }

    // ------------------------------------------------------------------
    // Runtime application of configuration (activation + cleanup handlers)

    fn apply_config(&mut self, ctx: &egui::Context) {
        let prev = self.applied.take();
        if prev.as_ref() == Some(&self.cfg) && !self.bg.theme_dirty {
            self.applied = prev;
            return; // nothing changed since last frame
        }
        let owned = self.cfg.clone();
        let c = &owned;
        let changed = |f: &dyn Fn(&Config) -> String| prev.as_ref().is_none_or(|p| f(p) != f(c));
        let feat_changed = |ids: &[&str]| prev.as_ref().is_none_or(|p| ids.iter().any(|i| p.active(i) != c.active(i)));

        // theme / appearance
        if prev.as_ref().is_none_or(|p| p.appearance != c.appearance || p.general.ui_scale != c.general.ui_scale)
            || feat_changed(&["custom_theme", "animations", "hover_effects", "shadows", "translucent_panels"])
            || self.bg.theme_dirty
        {
            self.bg.theme_dirty = false;
            let t = self.current_theme().clone();
            theme::apply(
                ctx,
                &t,
                &theme::ApplyOpts {
                    custom_theme: c.active("custom_theme"),
                    animations: c.active("animations"),
                    hover: c.active("hover_effects"),
                    shadows: c.active("shadows"),
                    translucent: c.active("translucent_panels"),
                    ui_scale: c.general.ui_scale,
                    accent_override: self.accent_override,
                },
            );
        }
        // fonts
        let want_font = if c.active("custom_font") && !c.appearance.font_path.trim().is_empty() { Some(c.appearance.font_path.clone()) } else { None };
        if want_font != self.font_applied {
            let mut fonts = egui::FontDefinitions::default();
            if let Some(path) = &want_font {
                match std::fs::read(path) {
                    Ok(bytes) => {
                        fonts.font_data.insert("custom".into(), Arc::new(egui::FontData::from_owned(bytes)));
                        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "custom".into());
                    }
                    Err(e) => self.toast(format!("Could not load font {path}: {e}"), true),
                }
            }
            ctx.set_fonts(fonts);
            self.font_applied = want_font;
        }
        // audio engine options
        if changed(&|c: &Config| format!("{:?}{:?}{:?}", c.playback.quality, c.playback.fade_ms, c.services.spotify_backend)) || feat_changed(&["normalization", "gapless", "fades"]) {
            self.engine.send(Cmd::Options(engine_opts(c)));
        }
        self.ensure_soloist(false);
        // DSP
        let dsp = crate::audio::dsp::DspParams {
            eq_on: c.active("eq"),
            eq_gains_db: c.audio.eq_gains_db,
            bass_on: c.active("bass_boost"),
            bass_db: c.audio.bass_db,
            preamp_db: c.audio.preamp_db,
            balance: c.audio.balance,
        };
        // only once connected: until then Aqua Glass keeps processing local files itself
        let apo = c.audio.eq_engine == crate::config::EqEngine::EqualizerApo && self.apo_linked;
        // with Equalizer APO the whole device is processed there; don't do it twice
        self.out.s.dsp.set(if apo { crate::audio::dsp::DspParams::default() } else { dsp.clone() });
        self.apo_queue();
        if changed(&|c: &Config| c.audio.buffer_ms.to_string()) {
            self.out.set_buffer_ms(c.audio.buffer_ms);
        }
        if prev.as_ref().is_some_and(|p| p.audio.device != c.audio.device) {
            self.out.open_device(&c.audio.device);
        }
        // artwork cache
        {
            let mut a = self.artwork.lock().unwrap();
            if self.pressure == perf::Pressure::Normal {
                a.budget_bytes = c.perf.artwork_cache_mb as usize * 1024 * 1024;
                a.max_px = c.perf.artwork_max_px;
            }
            a.disk_cache = c.active("artwork_disk_cache");
            if feat_changed(&["artwork_in_lists"]) && !c.active("artwork_in_lists") {
                a.clear();
            }
            if feat_changed(&["bg_album_art", "bg_image", "bg_blur"]) {
                a.clear_kind_backgrounds();
                self.bg.prev = None;
            }
        }
        // library metadata cache
        if feat_changed(&["metadata_cache"]) {
            let keep = c.active("metadata_cache");
            let mut l = self.library.lock().unwrap();
            l.keep_in_memory = keep;
            if !keep {
                l.evict_except(None);
            }
        }
        self.ints.notify.store(c.active("notifications"), Ordering::Relaxed);
        // tray
        if c.active("tray") && self.ints.tray.is_none() && feat_changed(&["tray"]) {
            let ctx2 = ctx.clone();
            match winint::create_tray(self.engine.clone(), self.ui_tx.clone(), Arc::new(move || ctx2.request_repaint())) {
                Ok(t) => self.ints.tray = Some(t),
                Err(e) => self.ints.errors.push(format!("Tray: {e}")),
            }
        } else if !c.active("tray") {
            self.ints.tray = None;
        }
        // media controls
        if c.active("media_keys") && self.ints.media.is_none() && feat_changed(&["media_keys"]) {
            if let Some(h) = self.hwnd {
                match winint::create_media(h, self.engine.clone()) {
                    Ok(m) => self.ints.media = Some(m),
                    Err(e) => self.ints.errors.push(format!("Media controls: {e}")),
                }
            }
        } else if !c.active("media_keys") {
            self.ints.media = None;
        }
        // global hotkeys
        if feat_changed(&["global_hotkeys"]) || changed(&|c: &Config| format!("{:?}{}{}", c.shortcuts, c.playback.volume_step, c.playback.seek_step_s)) {
            self.ints.hotkeys = None;
            self.ints.hotkey_errors.clear();
            if c.active("global_hotkeys") {
                let binds: Vec<(Action, Binding)> = c
                    .shortcuts
                    .iter()
                    .filter_map(|(a, s)| Some((Action::from_id(a)?, Binding::parse(s).ok()?)))
                    .collect();
                let ctx2 = ctx.clone();
                match winint::create_hotkeys(&binds, self.engine.clone(), self.ui_tx.clone(), Arc::new(move || ctx2.request_repaint()), c.playback.volume_step, c.playback.seek_step_s as i64 * 1000) {
                    Ok(h) => {
                        self.ints.hotkey_errors = h.errors.clone();
                        self.ints.hotkeys = Some(h);
                    }
                    Err(e) => self.ints.hotkey_errors.push(e.to_string()),
                }
            }
        }
        // autostart (only touch the registry when the toggle changes)
        if prev.as_ref().is_some_and(|p| p.active("start_with_windows") != c.active("start_with_windows")) {
            if let Err(e) = winint::set_autostart(c.active("start_with_windows")) {
                self.toast(format!("Start with Windows: {e}"), true);
            }
        }
        // acrylic backdrop (applies to transparent windows; takes full effect after restart)
        if feat_changed(&["window_backdrop"]) {
            if let Some(h) = self.hwnd {
                winint::set_backdrop(h, c.active("window_backdrop"), self.current_theme().dark);
            }
        }
        // Similarity Radio: worker exists only while enabled
        if c.active("similarity_radio") {
            let prefs = self.radio_prefs();
            if self.radio.is_none() {
                let ctx2 = ctx.clone();
                self.radio = Some(crate::radio::Radio::start(
                    crate::radio::Deps {
                        engine: Arc::new(self.engine.clone()),
                        spotify: self.spotify.clone(),
                        local_tracks: {
                            let lib = self.library.clone();
                            Arc::new(move || lib.lock().unwrap().tracks(crate::config::ProviderKind::Local, crate::library::LOCAL_ALL_ID).cloned().unwrap_or_default())
                        },
                        history: self.history.clone(),
                        wake: Arc::new(move || ctx2.request_repaint()),
                    },
                    prefs.clone(),
                ));
            } else if self.radio_prefs_last.as_ref() != Some(&prefs) {
                if let Some(r) = &self.radio {
                    r.send(crate::radio::Msg::Prefs(prefs.clone()));
                }
            }
            self.radio_prefs_last = Some(prefs);
        } else if self.radio.is_some() {
            self.radio = None; // worker exits; pending recommendations are removed
            self.radio_prefs_last = None;
            self.engine.send(Cmd::RadioClearUpcoming);
        }
        // visualizer cleanup
        if !c.active("visualizer") {
            self.analyzer = None;
            self.vis_buf = Vec::new();
        }
        // lyrics cleanup
        if !c.active("lyrics") {
            self.lyrics.cache.clear();
            self.lyrics.current = None;
            self.lyrics.key.clear();
            *self.lyrics.shared.lock().unwrap() = None;
        }
        if !c.active("mini_player") {
            self.mini_open.store(false, Ordering::Relaxed);
        }
        if !c.active("bg_dynamic_colors") && self.accent_override.is_some() {
            self.accent_override = None;
            self.bg.theme_dirty = true;
        }
        self.applied = Some(self.cfg.clone());
    }

    // ------------------------------------------------------------------

    fn handle_msgs(&mut self) {
        while let Ok(m) = self.msg_rx.try_recv() {
            match m {
                Msg::SpotifyAuth(r) => match r {
                    Ok(Some(p)) => {
                        self.sp_status = format!("Signed in as {}", p.display_name);
                        let premium_note = if !p.product.is_empty() && p.product != "premium" { " — playback needs Premium" } else { "" };
                        self.toast(format!("Spotify connected: {}{premium_note}", p.display_name), false);
                        self.sp_profile = Some(p);
                        let api = self.spotify.clone();
                        log::info!("Spotify signed in (product: {}); playback session: {}", p_product_dbg(&self.sp_profile), self.spotify.playback_signed_in());
                        if self.cfg.services.spotify_backend == crate::config::SpotifyBackend::Soloist {
                            // Soloist plays the audio; it pairs through Spotify Connect, not with this token
                        } else if self.spotify.playback_signed_in() {
                            self.engine.send(Cmd::SpotifyConnect(Arc::new(move || api.playback_token().map_err(|e| e.to_string()))));
                        } else {
                            self.toast("Library connected with your own Spotify app. Sign in once more for Premium playback (Settings › Music Services).", false);
                        }
                        let first = self.library.lock().unwrap().lib(ProviderKind::Spotify).last_sync == 0;
                        if first {
                            self.sync_now(ProviderKind::Spotify, false);
                        }
                    }
                    Ok(None) => self.sp_status = "Not signed in".into(),
                    Err(e) => {
                        self.sp_status = format!("Sign-in failed: {e}");
                        self.toast(format!("Spotify: {e}"), true);
                    }
                },
                Msg::Search { gen, provider, res } => {
                    if gen == self.search.gen {
                        self.search.pending = self.search.pending.saturating_sub(1);
                        self.search.results.insert(provider, res);
                    }
                }
                Msg::PlaylistTracks { provider, id, res } => {
                    let key = format!("{provider:?}:{id}");
                    self.view.loading.remove(&key);
                    self.view.live.insert(key, res);
                }
                Msg::AlbumTracks { album, res } => {
                    let key = format!("album:{}", album.id);
                    self.view.loading.remove(&key);
                    self.view.live.insert(key, res);
                }
                Msg::ArtistFound { name, res } => {
                    // the lookup key stays in `loading` on failure so the page doesn't retry every frame
                    if matches!(res, Ok(Some(_))) {
                        self.view.loading.remove(&format!("artist-lookup:{name}"));
                    }
                    let showing = matches!(&self.page, Page::Artist(a) if a.id.is_empty() && a.name == name);
                    match res {
                        // replace the placeholder page in place (no extra history entry)
                        Ok(Some(a)) if showing => self.page = Page::Artist(a),
                        Ok(None) if showing => self.toast(format!("Spotify has no artist named “{name}”."), true),
                        Err(e) if showing => self.toast(format!("Could not open the artist: {e}"), true),
                        _ => {}
                    }
                }
                Msg::ArtistAlbums { artist, res } => {
                    self.view.loading.remove(&format!("artist:{}", artist.id));
                    self.view.artist_albums.insert(artist.id, res);
                }
                Msg::SyncDone { provider, res } => match res {
                    Ok(r) => {
                        let mut s = format!("{} synced: {} new, {} changed, {} unchanged", provider.label(), r.added.len(), r.changed.len(), r.unchanged);
                        if !r.removed_upstream.is_empty() {
                            s.push_str(&format!(", {} removed upstream (kept locally — review in Settings › Library)", r.removed_upstream.len()));
                        }
                        let err = !r.errors.is_empty();
                        if err {
                            s.push_str(&format!(". {} error(s): {}", r.errors.len(), r.errors[0]));
                        }
                        self.toast(s, err);
                        self.view.live.clear();
                    }
                    Err(e) => {
                        self.library.lock().unwrap().lib(provider).last_error = e.clone();
                        self.toast(format!("{} sync failed: {e}", provider.label()), true);
                    }
                },
                Msg::Lyrics { key, res } => {
                    self.lyrics.loading.remove(&key);
                    if self.lyrics.cache.len() > 6 {
                        let cur = self.lyrics.key.clone();
                        self.lyrics.cache.retain(|k, _| *k == cur);
                    }
                    if key == self.lyrics.key {
                        *self.lyrics.shared.lock().unwrap() = res.as_ref().ok().cloned();
                        self.lyrics.current = Some(res.clone());
                    }
                    self.lyrics.cache.insert(key, res);
                }
                Msg::LocalScanned(tracks) => {
                    self.local_scanning = false;
                    let n = tracks.len();
                    let mut s = self.library.lock().unwrap();
                    let l = s.lib(ProviderKind::Local);
                    l.playlists.retain(|p| p.meta.id != crate::library::LOCAL_ALL_ID);
                    l.playlists.insert(
                        0,
                        crate::library::PlaylistEntry {
                            meta: Playlist { provider: ProviderKind::Local, id: crate::library::LOCAL_ALL_ID.into(), name: "All Local Files".into(), track_count: n as u32, owned_by_user: true, artwork_url: tracks.iter().find(|t| !t.artwork_url.is_empty()).map(|t| t.artwork_url.clone()).unwrap_or_default(), ..Default::default() },
                            imported: true,
                            removed_upstream: false,
                            last_synced: crate::util::now_unix(),
                        },
                    );
                    let mut albums: std::collections::BTreeMap<String, Album> = Default::default();
                    for t in &tracks {
                        if !t.album.is_empty() {
                            albums.entry(t.album.clone()).or_insert_with(|| Album {
                                provider: ProviderKind::Local,
                                id: t.album_id.clone(),
                                name: t.album.clone(),
                                artists: t.artists.clone(),
                                artwork_url: t.artwork_url.clone(),
                                total_tracks: 0,
                                ..Default::default()
                            });
                        }
                    }
                    l.albums = albums.into_values().collect();
                    l.last_sync = crate::util::now_unix();
                    s.store_tracks(ProviderKind::Local, crate::library::LOCAL_ALL_ID, tracks);
                    s.save_index(ProviderKind::Local);
                    drop(s);
                    self.toast(format!("Local library: {n} tracks"), false);
                }
                Msg::PlanReady(plan, src) => self.transfer.on_plan(plan, src),
                Msg::TransferDone(r) => self.transfer.on_done(r),
                Msg::Alternatives { src, res } => {
                    if self.alternatives.as_ref().is_some_and(|(s, _)| s.key() == src.key()) {
                        self.alternatives = Some((src, Some(res)));
                    }
                }
                Msg::Toast(t, e) => self.toast(t, e),
            }
        }
    }

    fn handle_ui_cmds(&mut self, ctx: &egui::Context) {
        while let Ok(c) = self.ui_rx.try_recv() {
            match c {
                UiCmd::Show => {
                    self.main_hidden = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                UiCmd::OpenSettings => {
                    self.main_hidden = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.navigate(Page::Settings);
                }
                UiCmd::ToggleMini => self.do_action(Action::ToggleMiniPlayer, ctx),
                UiCmd::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                UiCmd::Action(a) => self.do_action(a, ctx),
            }
        }
    }

    pub fn do_action(&mut self, a: Action, ctx: &egui::Context) {
        if winint::engine_action(&self.engine, a, self.cfg.playback.volume_step, self.cfg.playback.seek_step_s as i64 * 1000) {
            return;
        }
        match a {
            Action::LikeSong => {
                if let Some(t) = self.engine.snapshot().current {
                    self.like(&t);
                }
            }
            Action::ToggleLyrics => {
                if self.cfg.active("lyrics") {
                    if self.page == Page::NowPlaying {
                        self.go_back();
                    } else {
                        self.navigate(Page::NowPlaying);
                    }
                } else {
                    self.toast("Lyrics are disabled (Settings › Lyrics).", true);
                }
            }
            Action::ToggleMiniPlayer => {
                if self.cfg.active("mini_player") {
                    let v = !self.mini_open.load(Ordering::Relaxed);
                    self.mini_open.store(v, Ordering::Relaxed);
                    ctx.request_repaint();
                } else {
                    self.toast("Mini-player is disabled (Settings › Mini-Player).", true);
                }
            }
            Action::OpenSearch => {
                self.navigate(Page::Search);
                self.search.focus = true;
            }
            Action::OpenSettings => self.navigate(Page::Settings),
            Action::SwitchProvider => {
                self.filter = match self.filter {
                    Filter::All => Filter::Only(ProviderKind::Spotify),
                    Filter::Only(ProviderKind::Spotify) => Filter::Only(ProviderKind::Local),
                    Filter::Only(_) => Filter::All,
                };
                let label = match self.filter {
                    Filter::All => "All services".to_string(),
                    Filter::Only(p) => p.label().to_string(),
                };
                self.toast(format!("Showing: {label}"), false);
            }
            _ => {}
        }
    }

    fn in_app_shortcuts(&mut self, ctx: &egui::Context) {
        // Ctrl+, always opens settings (documented shortcut)
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.navigate(Page::Settings);
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let mut fired = Vec::new();
        for (aid, s) in &self.cfg.shortcuts {
            let (Some(a), Ok(b)) = (Action::from_id(aid), Binding::parse(s)) else { continue };
            // when registered globally, Windows delivers them through the hotkey handler instead
            if self.ints.hotkeys.is_some() && (b.ctrl || b.alt || b.win) {
                continue;
            }
            let Some(k) = b.egui_key() else { continue };
            let mods = egui::Modifiers { alt: b.alt, ctrl: b.ctrl, shift: b.shift, mac_cmd: false, command: b.ctrl };
            if ctx.input_mut(|i| i.consume_key(mods, k)) {
                fired.push(a);
            }
        }
        for a in fired {
            self.do_action(a, ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
            self.engine.send(Cmd::Toggle);
        }
    }

    fn perf_tick(&mut self) {
        let interval = if self.page == Page::Settings && self.settings.category == features::Category::Performance { Duration::from_secs(1) } else { Duration::from_secs(5) };
        if !self.monitor.tick(interval) {
            return;
        }
        // finish per-feature measurement
        if let Some(p) = &self.pending_measure {
            if p.at.elapsed() >= perf::MEASURE_SETTLE {
                if let Some(d) = perf::finish_measure(p, &self.monitor.last) {
                    let id = p.id.clone();
                    self.measurements.record(&id, d);
                    self.measurements.save();
                }
                self.pending_measure = None;
            }
        }
        // automatic memory optimizer
        if !self.cfg.active("auto_optimizer") {
            if self.pressure != perf::Pressure::Normal {
                self.pressure = perf::Pressure::Normal;
                self.applied = None; // restore normal limits
            }
            return;
        }
        let target = self.cfg.perf.ram_target_mb as f32;
        let p = perf::pressure(self.monitor.current_mb(), target, self.pressure);
        if p == self.pressure {
            return;
        }
        let mut actions = Vec::new();
        match p {
            perf::Pressure::Normal => {
                self.applied = None; // restores configured cache sizes
                actions.push("usage back to normal — restored configured limits".to_string());
            }
            perf::Pressure::Elevated => {
                let mut a = self.artwork.lock().unwrap();
                a.budget_bytes = (self.cfg.perf.artwork_cache_mb as usize * 1024 * 1024) / 2;
                let b = a.budget_bytes;
                a.evict_to(b);
                actions.push("halved artwork cache".into());
                let cur = self.lyrics.key.clone();
                self.lyrics.cache.retain(|k, _| *k == cur);
                actions.push("dropped cached lyrics".into());
                self.view.live.clear();
                actions.push("released hidden playlist views".into());
            }
            perf::Pressure::Over => {
                let mut a = self.artwork.lock().unwrap();
                a.max_px = a.max_px.min(160);
                a.budget_bytes = 4 * 1024 * 1024;
                a.clear();
                actions.push("artwork resolution › 160 px, cache cleared".into());
                drop(a);
                self.bg.prev = None;
                if !self.cfg.active("metadata_cache") || self.library.lock().unwrap().loaded_bytes_estimate() > 8 << 20 {
                    let keep = match &self.page {
                        Page::Playlist(p, id) => Some((*p, id.clone())),
                        _ => None,
                    };
                    self.library.lock().unwrap().evict_except(keep.as_ref().map(|(p, i)| (*p, i.as_str())));
                    actions.push("unloaded library track lists not on screen".into());
                }
                self.analyzer = None;
                perf::trim_working_set();
                actions.push("asked Windows to trim the working set".into());
            }
        }
        self.pressure = p;
        let line = format!("{} — {:?} at {:.0} MB: {}", crate::util::fmt_unix(crate::util::now_unix()), p, self.monitor.current_mb(), actions.join("; "));
        self.optimizer_log.push_front(line);
        self.optimizer_log.truncate(20);
    }

    fn lyrics_tick(&mut self) {
        let needs = self.cfg.active("lyrics") && (self.page == Page::NowPlaying || self.lyrics.fullscreen || (self.mini_open.load(Ordering::Relaxed) && self.cfg.active("mini_lyrics")) || self.cfg.active("overlay_lyrics"));
        if !needs {
            return;
        }
        let Some(t) = self.state.current.clone() else { return };
        let key = t.key();
        if key != self.lyrics.key {
            self.lyrics.key = key.clone();
            self.lyrics.current = self.lyrics.cache.get(&key).cloned();
            *self.lyrics.shared.lock().unwrap() = self.lyrics.current.as_ref().and_then(|r| r.as_ref().ok().cloned());
            if self.lyrics.current.is_none() {
                self.fetch_lyrics(t.clone());
            }
            if self.cfg.active("lyrics_prefetch") {
                let q = &self.state.queue;
                if let Some(next) = self.state.queue_pos.and_then(|p| q.get(p + 1)).cloned() {
                    if !self.lyrics.cache.contains_key(&next.key()) {
                        self.fetch_lyrics(next);
                    }
                }
            }
        }
    }

    pub fn fetch_lyrics(&mut self, t: Track) {
        let key = t.key();
        if self.lyrics.loading.contains(&key) {
            return;
        }
        self.lyrics.loading.insert(key.clone());
        self.jobs.spawn("Fetch lyrics", move |h| {
            let r = crate::services::lyrics::fetch(&t).map_err(|e| e.to_string());
            h.send(Msg::Lyrics { key, res: r });
        });
    }

    fn auto_sync_tick(&mut self) {
        if !self.cfg.active("auto_sync") || Instant::now() < self.sync_next_at {
            return;
        }
        self.sync_next_at = Instant::now() + Duration::from_secs(self.cfg.sync.interval_min as u64 * 60);
        if self.cfg.services.spotify_enabled && self.spotify.is_signed_in() {
            self.sync_now(ProviderKind::Spotify, false);
        }
    }

    /// Visualizer: tap the output only while a visualizer is on screen.
    pub fn vis_data(&mut self) -> Option<(&[f32], &[f32])> {
        if !self.cfg.active("visualizer") {
            return None;
        }
        self.vis_wanted = true;
        let fps = if self.cfg.active("vis_reduced_motion") { self.cfg.visualizer.fps.min(20) } else { self.cfg.visualizer.fps };
        let a = self.analyzer.get_or_insert_with(Analyzer::new);
        if a.bars.len() != self.cfg.visualizer.bars {
            a.bars.resize(self.cfg.visualizer.bars, 0.0);
        }
        if self.vis_last.elapsed() >= Duration::from_millis(1000 / fps.max(1) as u64) {
            self.vis_last = Instant::now();
            self.out.vis_snapshot(&mut self.vis_buf);
            let smoothing = if self.cfg.active("vis_reduced_motion") { self.cfg.visualizer.smoothing.max(0.85) } else { self.cfg.visualizer.smoothing };
            let playing = self.state.playing;
            if !playing {
                self.vis_buf.clear();
            }
            a.update(&self.vis_buf, self.out.rate(), self.cfg.visualizer.bars, self.cfg.visualizer.sensitivity, smoothing);
        }
        Some((&a.bars, &a.wave))
    }

    fn save_if_dirty(&mut self, force: bool) {
        if let Some(t) = self.cfg_dirty_at {
            if force || t.elapsed() > Duration::from_millis(800) {
                self.cfg.sanitize();
                if self.cfg != self.saved_cfg {
                    if let Err(e) = self.cfg.save() {
                        self.toast(format!("Could not save settings: {e}"), true);
                    } else {
                        self.saved_cfg = self.cfg.clone();
                    }
                }
                self.cfg_dirty_at = None;
            }
        }
    }

    fn restore_queue(&mut self) {
        if !self.cfg.active("remember_queue") {
            return;
        }
        let p = crate::util::data_dir().join("session.json");
        if let Some(s) = std::fs::read(&p).ok().and_then(|b| aqua_core::session::SavedSession::decode(&b)) {
            if !s.entries.is_empty() {
                self.engine.send(Cmd::Restore { entries: s.entries, start: s.pos, position_ms: s.position_ms });
            }
        }
    }

    fn save_queue(&self) {
        let p = crate::util::data_dir().join("session.json");
        if !self.cfg.active("remember_queue") {
            let _ = std::fs::remove_file(p);
            return;
        }
        let st = self.engine.snapshot();
        if let Some(s) = aqua_core::session::SavedSession::from_state(&st, self.engine.position_ms()) {
            let _ = crate::util::atomic_write(&p, &s.encode());
        }
    }

    pub fn reload_themes(&mut self) {
        let (t, e) = all_themes();
        self.themes = t;
        self.theme_errors = e;
        self.bg.theme_dirty = true;
    }
}

fn all_themes() -> (Vec<Theme>, Vec<String>) {
    let mut v = theme::builtin_themes();
    let (user, errs) = theme::load_user_themes();
    for t in user {
        if let Some(i) = v.iter().position(|b| b.name == t.name) {
            v[i] = t;
        } else {
            v.push(t);
        }
    }
    (v, errs)
}

pub fn engine_opts(c: &Config) -> EngineOpts {
    EngineOpts {
        quality: c.playback.quality,
        normalisation: c.active("normalization"),
        gapless: c.active("gapless"),
        fade_ms: if c.active("fades") { c.playback.fade_ms } else { 0 },
        spotify_backend: c.services.spotify_backend,
    }
}

#[allow(unused_variables)]
fn window_handle(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = cc.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                return Some(w.hwnd.get());
            }
        }
    }
    None
}

impl eframe::App for AquaApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_msgs();
        self.handle_ui_cmds(ctx);
        let st = self.engine.state.read().unwrap();
        if st.version != self.last_state_version {
            self.last_state_version = st.version;
            let track_changed = st.current.as_ref().map(|t| t.key()) != self.state.current.as_ref().map(|t| t.key());
            self.state = st.clone();
            drop(st);
            if track_changed {
                if let Some(t) = self.state.current.clone() {
                    // mark as liked when it's in a cached Liked collection
                    let k = t.key();
                    if !self.liked_keys.contains(&k) && t.provider == ProviderKind::Spotify {
                        let mut lib = self.library.lock().unwrap();
                        if lib.tracks(t.provider, crate::library::LIKED_ID).is_some_and(|l| l.iter().any(|x| x.id == t.id)) {
                            self.liked_keys.insert(k);
                        }
                    }
                }
            }
        } else {
            drop(st);
        }
        self.apply_config(ctx);
        if self.state.playing != self.apo_playing_seen {
            self.apo_queue(); // "only while playing": EQ on at play, flat at pause/stop
        }
        self.apo_tick(ctx);
        self.perf_tick();
        self.lyrics_tick();
        self.auto_sync_tick();
        self.save_if_dirty(false);
        // window close / minimize to tray
        let (close, minimized) = ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized.unwrap_or(false)));
        if close && !self.quitting && self.cfg.active("close_to_tray") && self.ints.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.main_hidden = true;
        } else if close {
            self.save_queue();
            self.save_if_dirty(true);
            self.engine.send(Cmd::Shutdown);
        }
        if minimized && self.cfg.active("minimize_to_tray") && self.ints.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.main_hidden = true;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.frame_times.push_back(dt);
        if self.frame_times.len() > 60 {
            self.frame_times.pop_front();
        }
        let avg = self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32;
        self.fps = if avg > 0.0 { 1.0 / avg } else { 0.0 };

        if self.first_frame {
            self.first_frame = false;
            if self.start_hidden {
                if self.cfg.active("tray") {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    self.main_hidden = true;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
        }
        // remember window geometry
        if let Some(r) = ctx.input(|i| i.viewport().outer_rect) {
            let pos = [r.min.x, r.min.y];
            let size = ctx.input(|i| i.viewport().inner_rect.map(|r| [r.width(), r.height()])).unwrap_or([r.width(), r.height()]);
            if self.cfg.general.window_pos != Some(pos) || self.cfg.general.window_size != Some(size) {
                self.cfg.general.window_pos = Some(pos);
                self.cfg.general.window_size = Some(size);
                self.mark_dirty();
            }
        }

        self.artwork.lock().unwrap().pump(&ctx);
        self.in_app_shortcuts(&ctx);
        self.vis_wanted = false;

        // dynamic accent from artwork
        if self.cfg.active("bg_dynamic_colors") {
            let url = self.state.current.as_ref().map(|t| t.artwork_url.clone()).unwrap_or_default();
            let mut a = self.artwork.lock().unwrap();
            let _ = a.get(&url, Kind::Cover);
            let d = a.dominant(&url);
            drop(a);
            if d.is_some() && d != self.accent_override {
                self.accent_override = d;
                self.bg.theme_dirty = true;
            }
        }

        background::paint(self, ui);
        shell::shell(self, ui);
        viewports::show(self, &ctx);
        self.dialogs(&ctx);
        self.paint_toasts(&ctx);
        if self.cfg.active("fps_counter") {
            egui::Area::new(egui::Id::new("fps")).anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0]).show(&ctx, |ui| {
                ui.label(RichText::new(format!("{:.0} FPS", self.fps)).small().monospace());
            });
        }

        // visualizer tap on/off depending on visibility this frame
        let tap = self.vis_wanted && self.cfg.active("visualizer");
        if tap != self.out.s.vis_enabled.load(Ordering::Relaxed) {
            self.out.set_vis(tap);
        }
        if !tap && self.analyzer.is_some() && !self.cfg.active("visualizer") {
            self.analyzer = None;
        }

        // repaint scheduling: only as often as something visibly changes
        if tap && self.state.playing {
            let fps = if self.cfg.active("vis_reduced_motion") { self.cfg.visualizer.fps.min(20) } else { self.cfg.visualizer.fps };
            ctx.request_repaint_after(Duration::from_millis(1000 / fps.max(1) as u64));
        } else if self.state.playing {
            let synced = self.page == Page::NowPlaying && self.cfg.active("lyrics_word");
            ctx.request_repaint_after(Duration::from_millis(if synced { 100 } else { 500 }));
        }
        if !self.jobs.active.lock().unwrap().is_empty() || !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.pending_measure.is_some() || self.cfg_dirty_at.is_some() {
            ctx.request_repaint_after(Duration::from_millis(1000));
        }
    }

    fn clear_color(&self, v: &egui::Visuals) -> [f32; 4] {
        if self.cfg.active("window_backdrop") {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            v.extreme_bg_color.to_normalized_gamma_f32()
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.apo_linked {
            if let Some(dir) = &self.apo_dir {
                let keep = self.cfg.audio.eq_engine == crate::config::EqEngine::EqualizerApo && !self.cfg.audio.apo_flat_on_exit;
                let text = match (&self.apo_pending, keep) {
                    (Some((t, _)), true) => t.clone(),
                    (_, true) => self.apo_last.clone().unwrap_or_default(),
                    _ => crate::eqapo::render(&crate::audio::dsp::DspParams::default(), false),
                };
                if !text.is_empty() {
                    let _ = crate::eqapo::write(dir, &text);
                }
            }
        }
        self.save_queue();
        self.save_if_dirty(true);
        self.engine.send(Cmd::Shutdown);
    }
}

impl AquaApp {
    fn paint_toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(if t.error { 9 } else { 4 }));
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts")).anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -110.0]).show(ctx, |ui| {
            ui.set_max_width(420.0);
            for t in &self.toasts {
                let c = if t.error { Color32::from_rgb(248, 113, 113) } else { self.accent() };
                egui::Frame::popup(ui.style()).stroke(egui::Stroke::new(1.0, c)).show(ui, |ui| {
                    ui.label(RichText::new(&t.text).color(ui.visuals().strong_text_color()));
                });
                ui.add_space(4.0);
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(c) = self.confirm.take() else {
            self.alternatives_dialog(ctx);
            return;
        };
        let mut keep = true;
        let mut action: Option<Box<dyn FnOnce(&mut AquaApp)>> = None;
        egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            match &c {
                Confirm::EnableFeature(id) => {
                    let f = features::get(id);
                    ui.label(RichText::new("Memory target warning").strong().size(17.0));
                    ui.label("This feature may cause Aqua Glass to exceed your configured memory target.");
                    if let Some(f) = f {
                        let est = self.measurements.by_feature.get(id).map(|m| format!("measured +{:.1} MB", m.delta_mb)).unwrap_or(format!("estimated +{}–{} MB", f.est_mb.0, f.est_mb.1));
                        ui.label(RichText::new(format!("{}: {est}. Current usage {:.0} MB, target {} MB.", f.name, self.monitor.current_mb(), self.cfg.perf.ram_target_mb)).weak());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Enable Anyway").clicked() {
                            let id = id.clone();
                            action = Some(Box::new(move |a: &mut AquaApp| a.set_feature(&id, true)));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Confirm::RemoveImported(p, id, name) => {
                    ui.label(RichText::new(format!("Remove “{name}” from Aqua Glass?")).strong());
                    ui.label("Only the local copy is removed (a backup is kept). Your playlist on the service is not touched.");
                    ui.horizontal(|ui| {
                        if ui.button("Remove local copy").clicked() {
                            let (p, id) = (*p, id.clone());
                            action = Some(Box::new(move |a: &mut AquaApp| a.library.lock().unwrap().remove_playlist(p, &id)));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Confirm::DisconnectSpotify => {
                    ui.label(RichText::new("Disconnect Spotify?").strong());
                    ui.label("Stored tokens are deleted from Windows Credential Manager.");
                    ui.label(RichText::new("To fully revoke access, also remove the app at spotify.com/account/apps.").weak());
                    ui.horizontal(|ui| {
                        if ui.button("Disconnect, keep library cache").clicked() {
                            action = Some(Box::new(move |a: &mut AquaApp| a.spotify_logout(false)));
                            keep = false;
                        }
                        if ui.button("Disconnect and delete cache").clicked() {
                            action = Some(Box::new(move |a: &mut AquaApp| a.spotify_logout(true)));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Confirm::ResetAll => {
                    ui.label(RichText::new("Restore all settings to defaults?").strong());
                    ui.label("Accounts, library and playlists are kept.");
                    ui.horizontal(|ui| {
                        if ui.button("Restore defaults").clicked() {
                            action = Some(Box::new(|a: &mut AquaApp| {
                                let services = a.cfg.services.clone();
                                a.cfg = Config::default();
                                a.cfg.services = services;
                                a.mark_dirty();
                            }));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Confirm::RemoveTransferred(log) => {
                    ui.label(RichText::new(format!("Delete the playlist Aqua Glass created on {}?", log.target.label())).strong());
                    ui.label(format!("“{}” ({} tracks). The source playlist is not affected.", log.source_name, log.added));
                    ui.horizontal(|ui| {
                        if ui.button("Delete created playlist").clicked() {
                            let l = log.clone();
                            action = Some(Box::new(move |a: &mut AquaApp| a.transfer.remove_created(a.spotify.clone(), &a.jobs, l)));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
            }
        });
        if let Some(f) = action {
            f(self);
        }
        if keep {
            self.confirm = Some(c);
        }
    }

    fn alternatives_dialog(&mut self, ctx: &egui::Context) {
        let Some((src, res)) = self.alternatives.clone() else { return };
        let mut open = true;
        let mut play: Option<Track> = None;
        egui::Window::new("Choose a version").open(&mut open).collapsible(false).default_size([520.0, 360.0]).show(ctx, |ui| {
            ui.label(format!("Original: {} — {} ({}, {})", src.artist_line(), src.title, src.provider.label(), crate::util::fmt_ms(src.duration_ms)));
            widgets::note(ui, "Aqua Glass never swaps recordings silently. Pick the version you want; scores consider title, artists, duration and version (live/remix/…).");
            ui.separator();
            match &res {
                None => {
                    ui.spinner();
                }
                Some(Err(e)) => widgets::error(ui, e),
                Some(Ok(c)) if c.is_empty() => {
                    ui.label("No candidates found.");
                }
                Some(Ok(c)) => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for cand in c {
                            ui.horizontal(|ui| {
                                widgets::provider_badge(ui, cand.track.provider);
                                ui.label(format!("{:.0}%", cand.score * 100.0));
                                if ui.button(icon_play()).clicked() {
                                    play = Some(cand.track.clone());
                                }
                                ui.label(format!("{} — {} ({})", cand.track.artist_line(), cand.track.title, crate::util::fmt_ms(cand.track.duration_ms)));
                                if !cand.notes.is_empty() {
                                    ui.label(RichText::new(cand.notes.join(", ")).small().weak());
                                }
                            });
                        }
                    });
                }
            }
        });
        if let Some(t) = play {
            self.engine.send(Cmd::PlayNext(vec![t]));
            self.engine.send(Cmd::Next);
            open = false;
        }
        if !open {
            self.alternatives = None;
        }
    }
}

fn icon_play() -> &'static str {
    widgets::icon::PLAY()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_filter() {
        assert!(Filter::All.allows(ProviderKind::Local));
        assert!(Filter::Only(ProviderKind::Spotify).allows(ProviderKind::Spotify));
        assert!(!Filter::Only(ProviderKind::Spotify).allows(ProviderKind::Local));
    }
    #[test]
    fn engine_opts_follow_toggles() {
        let mut c = Config::default();
        c.set("fades", false);
        c.set("gapless", false);
        let o = engine_opts(&c);
        assert_eq!(o.fade_ms, 0);
        assert!(!o.gapless);
        c.set("fades", true);
        assert_eq!(engine_opts(&c).fade_ms, c.playback.fade_ms);
    }
}

fn p_product_dbg(p: &Option<crate::services::spotify::Profile>) -> String {
    p.as_ref().map(|p| p.product.clone()).unwrap_or_default()
}
