//! Playback engine: owns the queue and the active backend and runs on its own
//! thread, so playback (and auto-advance) continue while the UI is hidden.
//!
//! Backends:
//! * Spotify › librespot (Premium), via `audio::spotify`.
//! * Local files › symphonia decoder, via `audio::local`.

use super::output::Output;
use crate::config::{ProviderKind, Quality, RepeatMode};
pub use aqua_core::playback::{PlayRecord, PlayerState, PlaybackControl, SpotifyStatus};
pub use aqua_core::queue::{EntryOrigin, Queue};
use crate::model::Track;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::time::{Duration, Instant};


// ---------------------------------------------------------------------------
// Commands / events / state

#[derive(Clone, Debug)]
pub struct EngineOpts {
    pub quality: Quality,
    pub normalisation: bool,
    pub gapless: bool,
    pub fade_ms: u32,
    /// Which program plays Spotify tracks.
    pub spotify_backend: crate::config::SpotifyBackend,
}

impl Default for EngineOpts {
    fn default() -> Self {
        EngineOpts { quality: Quality::Normal160, normalisation: false, gapless: true, fade_ms: 0, spotify_backend: crate::config::SpotifyBackend::Soloist }
    }
}

pub enum Cmd {
    Play { tracks: Vec<Track>, start: usize },
    /// Restore a saved session (entries in play order, with their origin).
    Restore { entries: Vec<(Track, EntryOrigin)>, start: usize, position_ms: u64 },
    /// Manual "Add to queue" (takes priority over radio recommendations).
    Enqueue(Vec<Track>),
    /// Similarity Radio recommendations (appended after everything else).
    RadioAppend(Vec<Track>),
    /// Drop radio recommendations that haven't played yet.
    RadioClearUpcoming,
    PlayNext(Vec<Track>),
    Toggle,
    Resume,
    Pause,
    Next,
    Prev,
    Seek(u64),
    SeekRel(i64),
    JumpTo(usize),
    Remove(usize),
    Move(usize, usize),
    ClearQueue,
    SetVolume(f32),
    ToggleMute,
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    /// Connect Spotify playback; the source is called for a fresh token on every (re)connect.
    SpotifyConnect(TokenSource),
    SpotifyDisconnect,
    Options(EngineOpts),
    /// Start (or attach to) Spotify Soloist; replaces a running one.
    SoloistStart(super::soloist::SoloistConfig),
    SoloistStop,
    Shutdown,
}

pub enum EngineEvent {
    Cmd(Cmd),
    LocalEnded(u64, Result<(), String>),
    SpotifyEnded(u64),
    /// (play request id, id of the track that failed — may be the *preloaded* next track)
    SpotifyUnavailable(u64, String),
    SpotifyPreload,
    SpotifyPlaying(u64, u32),
    SpotifyRequestId(u64),
    SpotifyDisconnected,
    SpotifyConnected(Result<(), String>),
    SoloistStarted(Result<super::soloist::Soloist, String>),
    Soloist(super::soloist::Event),
}




pub type Listener = aqua_core::playback::StateListener;

/// Shown while Soloist runs but no Spotify account has selected it yet.
pub const NOT_PAIRED: &str = "Spotify Soloist isn't paired yet: open Spotify on your phone or PC, tap the devices icon and choose “Aqua Glass” once. After that it stays paired.";
pub type TokenSource = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

/// Position bookkeeping shared with the UI (lock-free reads).
#[derive(Default)]
pub struct Clock {
    pub track_start_frame: AtomicU64,
    pub base_ms: AtomicU64,
    pub duration_ms: AtomicU64,
    /// Position comes from an external player (Soloist) instead of our output.
    pub external: std::sync::atomic::AtomicBool,
    pub ext_position_ms: AtomicU64,
    pub ext_timestamp_ms: std::sync::atomic::AtomicI64,
    /// speed × 1000 (0 = paused)
    pub ext_speed_milli: AtomicU64,
}

impl Clock {
    fn external_position(&self) -> u64 {
        let a = super::soloist::PosAnchor {
            position_ms: self.ext_position_ms.load(Ordering::Relaxed),
            timestamp_ms: self.ext_timestamp_ms.load(Ordering::Relaxed),
            speed: self.ext_speed_milli.load(Ordering::Relaxed) as f64 / 1000.0,
        };
        a.estimate(aqua_core::storage::now_millis())
    }
    fn set_external(&self, a: super::soloist::PosAnchor) {
        self.ext_position_ms.store(a.position_ms, Ordering::Relaxed);
        self.ext_timestamp_ms.store(a.timestamp_ms, Ordering::Relaxed);
        self.ext_speed_milli.store((a.speed.max(0.0) * 1000.0) as u64, Ordering::Relaxed);
        self.external.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub struct Engine {
    tx: mpsc::Sender<EngineEvent>,
    pub state: Arc<RwLock<PlayerState>>,
    pub out: Output,
    pub clock: Arc<Clock>,
    listeners: Arc<Mutex<Vec<Listener>>>,
}

impl Engine {
    pub fn start(out: Output, opts: EngineOpts, volume: f32, muted: bool, wake: Box<dyn Fn() + Send>) -> Engine {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(RwLock::new(PlayerState { volume, muted, ..Default::default() }));
        out.set_volume(volume);
        out.set_muted(muted);
        out.set_fade_ms(opts.fade_ms);
        let clock = Arc::new(Clock::default());
        let listeners: Arc<Mutex<Vec<Listener>>> = Arc::new(Mutex::new(Vec::new()));
        let e = Engine { tx: tx.clone(), state: state.clone(), out: out.clone(), clock: clock.clone(), listeners: listeners.clone() };
        let mut w = Worker {
            tx,
            out,
            state,
            clock,
            listeners,
            wake,
            opts,
            q: Queue::default(),
            active: Active::None,
            spotify: None,
            pending: Default::default(),
            feeding: None,
            playing: false,
            pending_restore_ms: None,
            local_token: 0,
            version: 0,
            last_spotify_token: None,
            last_current: None,
            leave_hint: None,
            record_seq: 0,
            sp_req: None,
            sp_load_frame: 0,
            sp_loaded_at: Instant::now(),
            sp_failures: 0,
            soloist: None,
            so_logged_in: false,
            so_active: false,
            so_expect: None,
            so_seen: false,
            so_start_ms: 0,
            so_start_paused: false,
            so_status: super::soloist::Status::Idle,
            so_deadline: None,
        };
        std::thread::Builder::new()
            .name("playback-engine".into())
            .spawn(move || w.run(rx))
            .expect("spawn engine");
        e
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(EngineEvent::Cmd(c));
    }

    pub fn snapshot(&self) -> PlayerState {
        self.state.read().unwrap().clone()
    }

    /// Audible position of the current track.
    pub fn position_ms(&self) -> u64 {
        if self.clock.external.load(Ordering::Relaxed) {
            let dur = self.clock.duration_ms.load(Ordering::Relaxed);
            let p = self.clock.external_position();
            return if dur > 0 { p.min(dur) } else { p };
        }
        let rate = self.out.rate().max(1) as u64;
        let consumed = self.out.s.consumed.load(Ordering::Relaxed);
        let start = self.clock.track_start_frame.load(Ordering::Relaxed);
        let base = self.clock.base_ms.load(Ordering::Relaxed);
        let dur = self.clock.duration_ms.load(Ordering::Relaxed);
        let p = base + consumed.saturating_sub(start) * 1000 / rate;
        if dur > 0 { p.min(dur) } else { p }
    }

    pub fn add_listener(&self, l: Listener) {
        self.listeners.lock().unwrap().push(l);
    }
}

/// The desktop engine is this platform's implementation of the portable
/// playback interface (used by Similarity Radio; future sync/handoff).
impl PlaybackControl for Engine {
    fn play(&self, tracks: Vec<Track>, start: usize) {
        self.send(Cmd::Play { tracks, start })
    }
    fn pause(&self) {
        self.send(Cmd::Pause)
    }
    fn resume(&self) {
        self.send(Cmd::Resume)
    }
    fn toggle(&self) {
        self.send(Cmd::Toggle)
    }
    fn next(&self) {
        self.send(Cmd::Next)
    }
    fn previous(&self) {
        self.send(Cmd::Prev)
    }
    fn seek(&self, position_ms: u64) {
        self.send(Cmd::Seek(position_ms))
    }
    fn set_volume(&self, volume: f32) {
        self.send(Cmd::SetVolume(volume))
    }
    fn enqueue(&self, tracks: Vec<Track>) {
        self.send(Cmd::Enqueue(tracks))
    }
    fn play_next(&self, tracks: Vec<Track>) {
        self.send(Cmd::PlayNext(tracks))
    }
    fn append_radio(&self, tracks: Vec<Track>) {
        self.send(Cmd::RadioAppend(tracks))
    }
    fn clear_radio_upcoming(&self) {
        self.send(Cmd::RadioClearUpcoming)
    }
    fn jump_to(&self, order_pos: usize) {
        self.send(Cmd::JumpTo(order_pos))
    }
    fn remove(&self, order_pos: usize) {
        self.send(Cmd::Remove(order_pos))
    }
    fn move_item(&self, from: usize, to: usize) {
        self.send(Cmd::Move(from, to))
    }
    fn clear_queue(&self) {
        self.send(Cmd::ClearQueue)
    }
    fn set_shuffle(&self, on: bool) {
        self.send(Cmd::SetShuffle(on))
    }
    fn set_repeat(&self, mode: RepeatMode) {
        self.send(Cmd::SetRepeat(mode))
    }
    fn snapshot(&self) -> PlayerState {
        Engine::snapshot(self)
    }
    fn position_ms(&self) -> u64 {
        Engine::position_ms(self)
    }
    fn add_listener(&self, listener: aqua_core::playback::StateListener) {
        Engine::add_listener(self, listener)
    }
}

enum Active {
    None,
    Local(super::local::Decoding),
    Spotify,
    /// Spotify Soloist plays the current track (separate process).
    Soloist,
}

/// A track that has started feeding the output but is not yet audible (gapless).
struct Pending {
    boundary_frame: u64,
    /// None = end of queue.
    order_pos: Option<usize>,
    /// The track was not pre-fed (non-gapless): start it when the boundary is reached.
    needs_start: bool,
}

struct Worker {
    tx: mpsc::Sender<EngineEvent>,
    out: Output,
    state: Arc<RwLock<PlayerState>>,
    clock: Arc<Clock>,
    listeners: Arc<Mutex<Vec<Listener>>>,
    wake: Box<dyn Fn() + Send>,
    opts: EngineOpts,
    q: Queue,
    active: Active,
    #[cfg(feature = "spotify-playback")]
    spotify: Option<super::spotify::SpotifyPlayer>,
    #[cfg(not(feature = "spotify-playback"))]
    spotify: Option<()>,
    pending: std::collections::VecDeque<Pending>,
    /// Order-position of the track currently being decoded into the output.
    feeding: Option<usize>,
    playing: bool,
    pending_restore_ms: Option<u64>,
    local_token: u64,
    version: u64,
    last_spotify_token: Option<TokenSource>,
    last_current: Option<Track>,
    /// (listened ms, natural end) for the track being left.
    leave_hint: Option<(u64, bool)>,
    record_seq: u64,
    /// Spotify request id of the current load (from PlayRequestIdChanged).
    sp_req: Option<u64>,
    /// Output frame counter when the current Spotify track was loaded.
    sp_load_frame: u64,
    sp_loaded_at: Instant,
    /// Spotify tracks in a row that failed to produce any audio.
    sp_failures: u32,
    soloist: Option<super::soloist::Soloist>,
    so_logged_in: bool,
    so_active: bool,
    /// URI we asked Soloist to play, and whether Soloist has confirmed it.
    so_expect: Option<String>,
    so_seen: bool,
    so_start_ms: u64,
    so_start_paused: bool,
    so_status: super::soloist::Status,
    so_deadline: Option<Instant>,
}

impl Worker {
    fn run(&mut self, rx: mpsc::Receiver<EngineEvent>) {
        loop {
            let ev = rx.recv_timeout(Duration::from_millis(if !self.pending.is_empty() { 20 } else { 250 }));
            match ev {
                Ok(EngineEvent::Cmd(Cmd::Shutdown)) => {
                    self.stop_all();
                    return;
                }
                Ok(ev) => self.handle(ev),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            self.check_boundary();
        }
    }

    fn publish(&mut self, f: impl FnOnce(&mut PlayerState)) {
        let snap = {
            let mut s = self.state.write().unwrap();
            f(&mut s);
            s.current = self.q.current().cloned();
            s.queue = self.q.in_order();
            s.queue_pos = self.q.pos;
            s.queue_origin = self.q.origins_in_order();
            s.radio_upcoming = self.q.radio_upcoming();
            let key = s.current.as_ref().map(|t| t.key());
            if key != self.last_current.as_ref().map(|t| t.key()) {
                if let Some(prev) = self.last_current.take() {
                    let (listened, natural) = self.leave_hint.take().unwrap_or((0, false));
                    let dur = prev.duration_ms.max(1);
                    let completed = natural || listened * 2 >= dur;
                    self.record_seq += 1;
                    s.last_played = Some(PlayRecord { skipped: !natural && !completed, completed, listened_ms: listened, track: prev, seq: self.record_seq });
                }
                self.last_current = s.current.clone();
            }
            s.shuffle = self.q.shuffle;
            s.repeat = self.q.repeat;
            s.playing = self.playing;
            s.duration_ms = s.current.as_ref().map(|t| t.duration_ms).unwrap_or(0);
            self.version += 1;
            s.version = self.version;
            s.clone()
        };
        self.clock.duration_ms.store(snap.duration_ms, Ordering::Relaxed);
        for l in self.listeners.lock().unwrap().iter_mut() {
            l(&snap);
        }
        (self.wake)();
    }

    fn set_error(&mut self, e: impl Into<String>) {
        let e = crate::security::scrub(&e.into());
        log::warn!("playback: {e}");
        self.publish(|s| s.error = Some(e));
    }

    fn stop_all(&mut self) {
        self.pending.clear();
        self.feeding = None;
        #[cfg(feature = "spotify-playback")]
        if let (Active::Spotify, Some(sp)) = (&self.active, &self.spotify) {
            sp.stop();
        }
        if let (Active::Soloist, Some(so)) = (&self.active, &self.soloist) {
            so.send(super::soloist::Command::Pause);
        }
        self.so_expect = None;
        self.so_deadline = None;
        self.active = Active::None;
        self.out.flush();
    }

    fn reset_clock(&mut self, base_ms: u64) {
        self.clock.external.store(false, Ordering::Relaxed);
        self.clock.track_start_frame.store(self.out.s.pushed.load(Ordering::Relaxed), Ordering::Relaxed);
        self.clock.base_ms.store(base_ms, Ordering::Relaxed);
        // consumed may lag pushed by in-flight callback frames; align
        let consumed = self.out.s.consumed.load(Ordering::Relaxed);
        if consumed > self.clock.track_start_frame.load(Ordering::Relaxed) {
            self.clock.track_start_frame.store(consumed, Ordering::Relaxed);
        }
    }

    /// Start the current queue track from `start_ms`, replacing whatever plays.
    fn start_current(&mut self, start_ms: u64, play: bool) {
        let Some(track) = self.q.current().cloned() else {
            self.stop_all();
            self.playing = false;
            self.publish(|_| {});
            return;
        };
        if self.leave_hint.is_none() {
            self.leave_hint = Some((self.position(), false));
        }
        if self.playing && self.out.fade_ms() > 0 {
            self.out.set_paused(true);
            self.out.wait_faded();
        }
        self.stop_all();
        self.feeding = self.q.pos;
        self.reset_clock(start_ms);
        self.publish(|s| {
            s.error = None;
            s.message = None;
        });
        match track.provider {
            ProviderKind::Local => self.start_local(&track, start_ms),
            ProviderKind::Spotify if self.opts.spotify_backend == crate::config::SpotifyBackend::Soloist => self.start_soloist(&track, start_ms, play),
            ProviderKind::Spotify => self.start_spotify(&track, start_ms as u32),
            ProviderKind::YouTube => {
                // left over from a queue saved before 0.6 (YouTube Music support was removed)
                self.playing = false;
                self.set_error("YouTube Music is no longer supported — skip this song.");
                return;
            }
        }
        self.playing = play;
        self.out.set_paused(!play);
        if !play {
            self.pause_backend();
        }
        self.publish(|_| {});
    }

    fn start_local(&mut self, track: &Track, start_ms: u64) {
        #[cfg(feature = "local-files")]
        {
            self.local_token += 1;
            let token = self.local_token;
            let tx = self.tx.clone();
            let d = super::local::spawn(
                track.id.clone().into(),
                start_ms,
                self.out.clone(),
                self.out.generation(),
                self.opts.normalisation,
                move |r| {
                    let _ = tx.send(EngineEvent::LocalEnded(token, r));
                },
            );
            self.active = Active::Local(d);
        }
        #[cfg(not(feature = "local-files"))]
        {
            let _ = (track, start_ms);
            self.set_error("This build was compiled without local file support.");
        }
    }

    fn start_spotify(&mut self, track: &Track, start_ms: u32) {
        #[cfg(feature = "spotify-playback")]
        {
            if self.spotify.as_ref().is_some_and(|s| !s.is_valid()) {
                self.spotify = None;
                if let Some(t) = self.last_spotify_token.clone() {
                    self.connect_spotify(t);
                }
            } else if self.spotify.is_none() {
                // reconnect lazily after a dropped session
                if let Some(t) = self.last_spotify_token.clone() {
                    self.connect_spotify(t);
                }
            }
            match &self.spotify {
                Some(sp) => {
                    sp.gen.store(self.out.generation(), Ordering::Release);
                    let uri = if track.uri.is_empty() { format!("spotify:track:{}", track.id) } else { track.uri.clone() };
                    self.sp_req = None;
                    self.sp_load_frame = self.out.s.pushed.load(Ordering::Relaxed);
                    self.sp_loaded_at = Instant::now();
                    match sp.load(&uri, start_ms) {
                        Ok(()) => self.active = Active::Spotify,
                        Err(e) => self.set_error(e.to_string()),
                    }
                }
                None => {
                    let st = self.state.read().unwrap().spotify.clone();
                    let msg = match st {
                        SpotifyStatus::Unavailable(e) => format!("Spotify playback unavailable: {e}"),
                        SpotifyStatus::Connecting => "Spotify playback is still connecting — try again in a moment.".into(),
                        _ => "Sign in to Spotify (Premium required) to play Spotify tracks.".into(),
                    };
                    self.set_error(msg);
                }
            }
        }
        #[cfg(not(feature = "spotify-playback"))]
        {
            let _ = (track, start_ms);
            self.set_error("This build was compiled without Spotify playback (feature `spotify-playback`).");
        }
    }

    /// The current Spotify track produced no audio. Skip it, but stop after 3 in a
    /// row instead of racing through the queue (and making the radio refill forever).
    fn spotify_failed(&mut self) {
        self.sp_failures += 1;
        let title = self.q.current().map(|t| t.title.clone()).unwrap_or_default();
        let reason = crate::util::playback_error_since(self.sp_loaded_at);
        if reason.as_deref().is_some_and(|e| e.contains("audio key")) {
            // Spotify refuses librespot's decryption keys for this account: every song
            // will fail the same way, so don't skip through the queue
            self.sp_failures = 0;
            self.stop_all();
            self.playing = false;
            self.out.set_paused(true);
            self.set_error("Spotify refused the audio key for librespot (a known Spotify-side block on many accounts). Switch Spotify playback to Spotify Soloist in Settings › Music Services.");
            return;
        }
        let why = reason.map(|e| format!(" ({e})")).unwrap_or_default();
        if self.sp_failures >= 3 {
            self.sp_failures = 0;
            self.stop_all();
            self.playing = false;
            self.out.set_paused(true);
            self.set_error(format!("Spotify couldn't play 3 songs in a row{why}. Playback stopped. Check that your account is Premium; --debug-log records details."));
            return;
        }
        let msg = format!("Spotify couldn't play “{title}”{why} — skipped.");
        if self.q.advance(false).is_some() {
            self.start_current(0, true);
            self.set_error(msg); // after start_current, which clears errors
        } else {
            self.set_error(msg);
            self.playing = false;
            self.publish(|_| {});
        }
    }

    fn pause_backend(&mut self) {
        #[cfg(feature = "spotify-playback")]
        if let (Active::Spotify, Some(sp)) = (&self.active, &self.spotify) {
            sp.pause();
        }
        if let (Active::Soloist, Some(so)) = (&self.active, &self.soloist) {
            so.send(super::soloist::Command::Pause);
        }
    }

    fn resume_backend(&mut self) {
        #[cfg(feature = "spotify-playback")]
        if let (Active::Spotify, Some(sp)) = (&self.active, &self.spotify) {
            sp.play();
        }
        if let (Active::Soloist, Some(so)) = (&self.active, &self.soloist) {
            so.send(super::soloist::Command::Play(None));
        }
    }

    #[allow(unused_variables)]
    fn connect_spotify(&mut self, source: TokenSource) {
        #[cfg(feature = "spotify-playback")]
        {
            self.last_spotify_token = Some(source.clone());
            self.publish(|s| s.spotify = SpotifyStatus::Connecting);
            let token = match source() {
                Ok(t) => t,
                Err(e) => {
                    self.publish(|s| s.spotify = SpotifyStatus::Unavailable(format!("could not get a Spotify token: {e}")));
                    return;
                }
            };
            let opts = super::spotify::PlayerOpts { quality: self.opts.quality, normalisation: self.opts.normalisation, gapless: self.opts.gapless };
            match super::spotify::SpotifyPlayer::connect(&token, &self.out, &opts, self.tx.clone()) {
                Ok(p) => {
                    self.spotify = Some(p);
                    self.publish(|s| s.spotify = SpotifyStatus::Ready);
                }
                Err(e) => {
                    let msg = crate::security::scrub(&e.to_string());
                    self.publish(|s| s.spotify = SpotifyStatus::Unavailable(msg));
                }
            }
        }
        #[cfg(not(feature = "spotify-playback"))]
        {
            let _ = source;
            self.publish(|s| s.spotify = SpotifyStatus::Unavailable("built without spotify-playback".into()));
        }
    }

    /// The track being fed finished decoding into the output.
    fn source_finished(&mut self) {
        let Some(base) = self.feeding.or(self.q.pos) else { return };
        let boundary = self.out.s.pushed.load(Ordering::Relaxed);
        let Some(np) = self.q.next_pos_from(base, true) else {
            // end of queue: let the buffer drain, then stop
            self.feeding = None;
            self.pending.push_back(Pending { boundary_frame: boundary, order_pos: None, needs_start: false });
            return;
        };
        let cur_provider = self.q.tracks.get(self.q.order[base]).map(|t| t.provider);
        let next = self.q.tracks[self.q.order[np]].clone();
        let gapless_ok = self.opts.gapless && Some(next.provider) == cur_provider;
        if gapless_ok {
            // feed the next track right behind the current one
            self.feeding = Some(np);
            match next.provider {
                ProviderKind::Local => self.start_local(&next, 0),
                ProviderKind::Spotify => self.start_spotify(&next, 0),
                _ => {}
            }
        } else {
            self.feeding = None;
        }
        self.pending.push_back(Pending { boundary_frame: boundary, order_pos: Some(np), needs_start: !gapless_ok });
    }

    fn check_boundary(&mut self) {
        if self.so_deadline.is_some_and(|d| Instant::now() > d) {
            self.so_deadline = None;
            let title = self.q.current().map(|t| t.title.clone()).unwrap_or_default();
            self.playing = false;
            self.set_error(format!("Spotify Soloist didn't start “{title}” within 20 seconds. Check that Soloist is running and paired (Settings › Music Services)."));
        }
        loop {
            let Some(p) = self.pending.front() else { return };
            let consumed = self.out.s.consumed.load(Ordering::Relaxed);
            let drained = consumed >= p.boundary_frame || (self.out.is_paused() && self.out.buffered_frames() == 0 && !self.playing);
            if !drained {
                return;
            }
            let p = self.pending.pop_front().unwrap();
            match p.order_pos {
                None => {
                    self.pending.clear();
                    self.leave_hint = Some((u64::MAX / 4, true));
                    self.playing = false;
                    self.active = Active::None;
                    self.feeding = None;
                    self.out.set_paused(true);
                    self.publish(|s| s.message = Some("End of queue".into()));
                    return;
                }
                Some(np) if p.needs_start => {
                    self.leave_hint = Some((u64::MAX / 4, true));
                    self.q.pos = Some(np);
                    self.start_current(0, true);
                    return;
                }
                Some(np) => {
                    self.leave_hint = Some((u64::MAX / 4, true));
                    self.q.pos = Some(np);
                    self.clock.track_start_frame.store(p.boundary_frame, Ordering::Relaxed);
                    self.clock.base_ms.store(0, Ordering::Relaxed);
                    self.publish(|s| s.error = None);
                }
            }
        }
    }

    fn handle(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::Cmd(c) => self.command(c),
            EngineEvent::LocalEnded(tok, r) => {
                if tok != self.local_token {
                    return;
                }
                if let Err(e) = r {
                    self.set_error(format!("Could not play file: {e}"));
                }
                self.source_finished();
            }
            EngineEvent::SpotifyEnded(id) => {
                if !matches!(self.active, Active::Spotify) || self.sp_req.is_some_and(|r| r != id) {
                    return; // a previous request finishing late
                }
                // librespot also reports "end of track" when it can't fetch/decode audio
                let pushed = self.out.s.pushed.load(Ordering::Relaxed).saturating_sub(self.sp_load_frame);
                let key_refused = crate::util::playback_error_since(self.sp_loaded_at).is_some_and(|e| e.contains("audio key"));
                if key_refused || pushed < self.out.rate().max(1) as u64 {
                    self.spotify_failed();
                } else {
                    self.sp_failures = 0;
                    self.source_finished();
                }
            }
            EngineEvent::SpotifyUnavailable(id, track_id) => {
                let current = self.q.current().filter(|t| t.provider == ProviderKind::Spotify).map(|t| t.id.clone());
                if current.as_deref() != Some(track_id.as_str()) {
                    // the *preloaded next* track failed (librespot reports it with the
                    // current request id) — the current song is fine; don't skip it
                    log::warn!("Spotify: could not preload the next track ({track_id})");
                    return;
                }
                if !matches!(self.active, Active::Spotify) || self.sp_req.is_some_and(|r| r != id) {
                    return;
                }
                self.spotify_failed();
            }
            EngineEvent::SpotifyPreload => {
                #[cfg(feature = "spotify-playback")]
                if self.opts.gapless {
                    if let (Some(n), Some(sp)) = (self.q.peek_next(true), &self.spotify) {
                        if n.provider == ProviderKind::Spotify {
                            sp.preload(&n.uri);
                        }
                    }
                }
            }
            EngineEvent::SpotifyRequestId(id) => {
                if matches!(self.active, Active::Spotify) {
                    self.sp_req = Some(id);
                }
            }
            EngineEvent::SpotifyPlaying(..) => {}
            EngineEvent::SpotifyDisconnected => {
                self.publish(|s| s.spotify = SpotifyStatus::Unavailable("Spotify session disconnected; reconnecting on next play.".into()));
                #[cfg(feature = "spotify-playback")]
                {
                    self.spotify = None;
                }
            }
            EngineEvent::SpotifyConnected(_) => {}
            EngineEvent::SoloistStarted(r) => match r {
                Ok(so) => {
                    let pid = so.pid;
                    let st = self.state.read().unwrap().clone();
                    let v = if st.muted { 0.0 } else { st.volume };
                    so.send(super::soloist::Command::SetVolume((v.clamp(0.0, 1.0) * 100.0).round() as u8));
                    self.soloist = Some(so);
                    self.publish(|s| s.helper_pid = pid);
                }
                Err(e) => {
                    log::warn!("Soloist: {e}");
                    self.publish(|s| s.spotify = SpotifyStatus::Unavailable(format!("Spotify Soloist: {e}")));
                }
            },
            EngineEvent::Soloist(e) => self.soloist_event(e),
        }
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Play { tracks, start } => {
                self.sp_failures = 0;
                self.q.set(tracks, start);
                self.pending_restore_ms = None;
                self.start_current(0, true);
            }
            Cmd::Restore { entries, start, position_ms } => {
                self.q = Queue::from_entries(entries, Some(start), self.q.shuffle, self.q.repeat);
                self.pending_restore_ms = Some(position_ms);
                self.playing = false;
                self.reset_clock(position_ms);
                self.publish(|_| {});
            }
            Cmd::RadioAppend(t) => {
                let was_empty = self.q.current().is_none();
                let at_end = !self.playing && self.pending.iter().any(|p| p.order_pos.is_none());
                self.q.append_radio(t);
                if was_empty {
                    self.start_current(0, true);
                } else if at_end || (self.feeding.is_none() && self.pending.is_empty() && !self.playing && self.state.read().unwrap().message.as_deref() == Some("End of queue")) {
                    // the queue had run out: continue with the recommendations
                    self.pending.clear();
                    if self.q.advance(false).is_some() {
                        self.start_current(0, true);
                    }
                } else {
                    // re-plan the gapless hand-off if the current track was the last one
                    if let Some(i) = self.pending.iter().position(|p| p.order_pos.is_none()) {
                        let b = self.pending[i].boundary_frame;
                        self.pending.remove(i);
                        if let Some(np) = self.feeding.or(self.q.pos).and_then(|f| self.q.next_pos_from(f, true)) {
                            self.pending.push_back(Pending { boundary_frame: b, order_pos: Some(np), needs_start: true });
                        }
                    }
                    self.publish(|_| {});
                }
            }
            Cmd::RadioClearUpcoming => {
                self.q.clear_radio_upcoming();
                self.publish(|_| {});
            }
            Cmd::Enqueue(t) => {
                let was_empty = self.q.current().is_none();
                self.q.insert_manual(t);
                if was_empty {
                    self.start_current(0, true);
                } else {
                    self.publish(|_| {});
                }
            }
            Cmd::PlayNext(t) => {
                let was_empty = self.q.current().is_none();
                self.q.insert_next(t);
                if was_empty {
                    self.start_current(0, true);
                } else {
                    self.publish(|_| {});
                }
            }
            Cmd::Toggle => {
                if self.playing {
                    self.command(Cmd::Pause)
                } else {
                    self.command(Cmd::Resume)
                }
            }
            Cmd::Resume => {
                if let Some(ms) = self.pending_restore_ms.take() {
                    self.start_current(ms, true);
                    return;
                }
                match self.active {
                    Active::None => self.start_current(0, true),
                    _ => {
                        self.playing = true;
                        self.resume_backend();
                        self.out.set_paused(false);
                        self.publish(|_| {});
                    }
                }
            }
            Cmd::Pause => {
                if self.playing {
                    self.playing = false;
                    self.out.set_paused(true);
                    self.pause_backend();
                    self.publish(|_| {});
                }
            }
            Cmd::Next => {
                self.sp_failures = 0;
                self.pending.clear();
                if self.q.advance(false).is_some() {
                    self.start_current(0, true);
                } else {
                    self.set_error("No next track in the queue.");
                }
            }
            Cmd::Prev => {
                let pos = self.position();
                if pos > 3000 || self.q.pos == Some(0) && self.q.repeat != RepeatMode::All {
                    self.seek(0);
                } else {
                    self.pending.clear();
                    self.q.back();
                    self.start_current(0, true);
                }
            }
            Cmd::Seek(ms) => self.seek(ms),
            Cmd::SeekRel(d) => {
                let p = self.position() as i64 + d;
                self.seek(p.max(0) as u64);
            }
            Cmd::JumpTo(opos) => {
                self.sp_failures = 0;
                if opos < self.q.order.len() {
                    self.pending.clear();
                    self.q.pos = Some(opos);
                    self.start_current(0, true);
                }
            }
            Cmd::Remove(opos) => {
                let was_current = self.q.remove(opos);
                if was_current {
                    self.pending.clear();
                    if self.q.current().is_some() {
                        self.start_current(0, self.playing);
                    } else {
                        self.stop_all();
                        self.playing = false;
                    }
                }
                self.publish(|_| {});
            }
            Cmd::Move(a, b) => {
                self.q.move_item(a, b);
                self.publish(|_| {});
            }
            Cmd::ClearQueue => {
                self.q.clear_upcoming();
                self.publish(|_| {});
            }
            Cmd::SetVolume(v) => {
                self.out.set_volume(v);
                if let Some(so) = &self.soloist {
                    so.send(super::soloist::Command::SetVolume((v.clamp(0.0, 1.0) * 100.0).round() as u8));
                }
                self.publish(|s| s.volume = v.clamp(0.0, 1.0));
            }
            Cmd::ToggleMute => {
                let m = !self.state.read().unwrap().muted;
                self.out.set_muted(m);
                if let Some(so) = &self.soloist {
                    let v = if m { 0.0 } else { self.state.read().unwrap().volume };
                    so.send(super::soloist::Command::SetVolume((v * 100.0).round() as u8));
                }
                self.publish(|s| s.muted = m);
            }
            Cmd::SetShuffle(on) => {
                self.q.set_shuffle(on);
                self.publish(|_| {});
            }
            Cmd::SetRepeat(r) => {
                self.q.repeat = r;
                self.publish(|_| {});
            }
            Cmd::SpotifyConnect(tok) => self.connect_spotify(tok),
            Cmd::SpotifyDisconnect => {
                if matches!(self.active, Active::Spotify) {
                    self.stop_all();
                    self.playing = false;
                }
                #[cfg(feature = "spotify-playback")]
                {
                    self.spotify = None;
                }
                self.last_spotify_token = None;
                self.publish(|s| s.spotify = SpotifyStatus::Disconnected);
            }
            Cmd::Options(o) => {
                let recreate = o.quality != self.opts.quality || o.normalisation != self.opts.normalisation || o.gapless != self.opts.gapless;
                self.out.set_fade_ms(o.fade_ms);
                self.opts = o;
                #[cfg(feature = "spotify-playback")]
                if recreate && self.spotify.is_some() {
                    let was = matches!(self.active, Active::Spotify);
                    let pos = self.position();
                    if was {
                        self.stop_all();
                    }
                    self.spotify = None;
                    if let Some(t) = self.last_spotify_token.clone() {
                        self.connect_spotify(t);
                    }
                    if was {
                        self.start_current(pos, self.playing);
                    }
                }
                let _ = recreate;
            }
            Cmd::SoloistStart(cfg) => {
                self.soloist = None; // stops a managed Soloist we started before
                self.so_logged_in = false;
                self.publish(|s| {
                    s.spotify = SpotifyStatus::Connecting;
                    s.helper_pid = None;
                });
                let tx = self.tx.clone();
                // starting can take seconds — don't block playback of local files
                std::thread::Builder::new()
                    .name("soloist-start".into())
                    .spawn(move || {
                        let t2 = tx.clone();
                        let r = super::soloist::Soloist::start(&cfg, move |e| {
                            let _ = t2.send(EngineEvent::Soloist(e));
                        });
                        let _ = tx.send(EngineEvent::SoloistStarted(r));
                    })
                    .ok();
            }
            Cmd::SoloistStop => {
                if matches!(self.active, Active::Soloist) {
                    self.stop_all();
                    self.playing = false;
                }
                self.soloist = None;
                self.so_logged_in = false;
                self.publish(|s| {
                    s.spotify = SpotifyStatus::Disconnected;
                    s.helper_pid = None;
                });
            }
            Cmd::Shutdown => {}
        }
    }

    /// Play a Spotify track through Soloist.
    fn start_soloist(&mut self, track: &Track, start_ms: u64, play: bool) {
        use super::soloist::{Command as SC, PosAnchor};
        let Some(so) = &self.soloist else {
            let msg = match self.state.read().unwrap().spotify.clone() {
                SpotifyStatus::Unavailable(e) => e,
                SpotifyStatus::Connecting => "Spotify Soloist is starting — try again in a moment.".into(),
                _ => "Set up Spotify Soloist in Settings › Music Services to play Spotify songs.".into(),
            };
            self.set_error(msg);
            return;
        };
        if !self.so_logged_in {
            self.set_error(NOT_PAIRED);
            return;
        }
        if !self.so_active {
            so.send(SC::Activate);
        }
        let uri = if track.uri.is_empty() { format!("spotify:track:{}", track.id) } else { track.uri.clone() };
        so.send(SC::Play(Some(uri.clone())));
        self.so_expect = Some(uri);
        self.so_seen = false;
        self.so_start_ms = start_ms;
        self.so_start_paused = !play;
        self.so_deadline = Some(Instant::now() + Duration::from_secs(20));
        self.active = Active::Soloist;
        self.clock.set_external(PosAnchor { position_ms: start_ms, timestamp_ms: aqua_core::storage::now_millis(), speed: 0.0 });
    }

    /// The Soloist track finished (or Soloist moved on by itself): continue with our queue.
    fn soloist_track_ended(&mut self) {
        self.so_expect = None;
        self.so_seen = false;
        self.so_deadline = None;
        let dur = self.q.current().map(|t| t.duration_ms).unwrap_or(0);
        self.leave_hint = Some((dur, true));
        if self.q.advance(true).is_some() {
            self.start_current(0, true);
        } else {
            if let Some(so) = &self.soloist {
                so.send(super::soloist::Command::Pause); // don't let Spotify autoplay take over
            }
            self.active = Active::None;
            self.playing = false;
            self.publish(|_| {});
        }
    }

    fn soloist_event(&mut self, e: super::soloist::Event) {
        use super::soloist::Event as E;
        let ours = matches!(self.active, Active::Soloist);
        match e {
            E::Connected => {}
            E::Auth { logged_in, active, .. } => {
                self.so_logged_in = logged_in;
                self.so_active = active;
                self.publish(|s| s.spotify = if logged_in { SpotifyStatus::Ready } else { SpotifyStatus::Unavailable(NOT_PAIRED.into()) });
            }
            E::Active(a) => self.so_active = a,
            E::Track { uri, duration_ms } => self.soloist_track(ours, uri, duration_ms),
            E::State { status, uri, duration_ms, position, .. } => {
                self.soloist_track(ours, uri, duration_ms);
                if ours && self.so_seen {
                    self.clock.set_external(position);
                }
                self.soloist_status(ours, status);
            }
            E::Playback(st) => self.soloist_status(ours, st),
            E::Position(a) => {
                if ours && self.so_seen {
                    self.clock.set_external(a);
                    self.publish(|_| {});
                }
            }
            E::Error(m) => {
                if m.contains("auth") {
                    self.so_logged_in = false;
                    self.publish(|s| s.spotify = SpotifyStatus::Unavailable(NOT_PAIRED.into()));
                    if ours {
                        self.playing = false;
                        self.set_error(NOT_PAIRED);
                    }
                } else if ours {
                    self.set_error(format!("Spotify Soloist: {m}"));
                }
            }
            E::Down(m) => {
                if m.contains("stopped") || m.contains("expired") {
                    self.soloist = None;
                }
                self.so_logged_in = false;
                if ours {
                    self.active = Active::None;
                    self.playing = false;
                    self.so_expect = None;
                    self.set_error(m.clone());
                }
                self.publish(|s| s.spotify = SpotifyStatus::Unavailable(m));
            }
        }
    }

    fn soloist_track(&mut self, ours: bool, uri: String, duration_ms: u64) {
        if !ours || uri.is_empty() {
            return;
        }
        match &self.so_expect {
            Some(want) if *want == uri => {
                if !self.so_seen {
                    self.so_seen = true;
                    self.so_deadline = None;
                    if let Some(so) = &self.soloist {
                        if self.so_start_ms > 0 {
                            so.send(super::soloist::Command::Seek(self.so_start_ms));
                        }
                        if self.so_start_paused {
                            so.send(super::soloist::Command::Pause);
                        }
                    }
                    if duration_ms > 0 {
                        self.clock.duration_ms.store(duration_ms, Ordering::Relaxed);
                    }
                    self.publish(|s| s.error = None);
                }
            }
            // Soloist moved to a different track after ours played: ours has ended
            // (or someone skipped in the Spotify app) — continue with our queue.
            Some(_) if self.so_seen => self.soloist_track_ended(),
            _ => {}
        }
    }

    fn soloist_status(&mut self, ours: bool, st: super::soloist::Status) {
        use super::soloist::Status;
        self.so_status = st;
        if !ours || !self.so_seen {
            return;
        }
        let dur = self.q.current().map(|t| t.duration_ms).unwrap_or(0).max(self.clock.duration_ms.load(Ordering::Relaxed));
        let near_end = dur > 0 && self.clock.external_position() + 2500 >= dur;
        match st {
            Status::Playing => {
                if !self.playing {
                    self.playing = true;
                    self.publish(|_| {});
                }
            }
            Status::Idle => self.soloist_track_ended(),
            Status::Paused if near_end => self.soloist_track_ended(),
            Status::Paused => {
                // paused from the Spotify app / media keys
                if self.playing {
                    self.playing = false;
                    self.publish(|_| {});
                }
            }
            Status::Buffering => {}
        }
    }

    fn position(&self) -> u64 {
        if self.clock.external.load(Ordering::Relaxed) {
            return self.clock.external_position();
        }
        let rate = self.out.rate().max(1) as u64;
        let consumed = self.out.s.consumed.load(Ordering::Relaxed);
        self.clock.base_ms.load(Ordering::Relaxed) + consumed.saturating_sub(self.clock.track_start_frame.load(Ordering::Relaxed)) * 1000 / rate
    }

    fn seek(&mut self, ms: u64) {
        let dur = self.q.current().map(|t| t.duration_ms).unwrap_or(0);
        let ms = if dur > 0 { ms.min(dur.saturating_sub(500)) } else { ms };
        self.pending.clear();
        match self.active {
            Active::Local(_) => self.start_current(ms, self.playing),
            Active::Soloist => {
                if let Some(so) = &self.soloist {
                    so.send(super::soloist::Command::Seek(ms));
                }
                let speed = if self.playing { 1.0 } else { 0.0 };
                self.clock.set_external(super::soloist::PosAnchor { position_ms: ms, timestamp_ms: aqua_core::storage::now_millis(), speed });
                self.publish(|_| {});
            }
            #[cfg(feature = "spotify-playback")]
            Active::Spotify if self.feeding != self.q.pos => self.start_current(ms, self.playing),
            #[cfg(feature = "spotify-playback")]
            Active::Spotify => {
                self.out.flush();
                self.reset_clock(ms);
                if let Some(sp) = &self.spotify {
                    sp.gen.store(self.out.generation(), Ordering::Release);
                    sp.seek(ms as u32);
                }
                self.publish(|_| {});
            }
            _ => {
                if self.pending_restore_ms.is_some() {
                    self.pending_restore_ms = Some(ms);
                    self.reset_clock(ms);
                    self.publish(|_| {});
                }
            }
        }
    }
}

/// Wait helper for tests.
#[allow(dead_code)]
pub fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

