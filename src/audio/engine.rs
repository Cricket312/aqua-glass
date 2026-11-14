//! Playback engine: owns the queue and the active backend and runs on its own
//! thread, so playback (and auto-advance) continue while the UI is hidden.
//!
//! Backends:
//! * Spotify › librespot (Premium), via `audio::spotify`.
//! * Local files › symphonia decoder, via `audio::local`.
//! * YouTube › opened in YouTube Music / YouTube in the browser (no
//!   third-party audio API exists; see `services::youtube`).

use super::output::Output;
use crate::config::{ProviderKind, Quality, RepeatMode, YtOpen};
use crate::model::Track;
use rand::seq::SliceRandom;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Queue (pure logic, unit tested)

#[derive(Clone, Debug, Default)]
pub struct Queue {
    pub tracks: Vec<Track>,
    /// Play order: indices into `tracks`.
    pub order: Vec<usize>,
    /// Position in `order` of the current track.
    pub pos: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// Parallel to `tracks`: true for Similarity Radio recommendations.
    pub radio: Vec<bool>,
}

impl Queue {
    pub fn set(&mut self, tracks: Vec<Track>, start: usize) {
        self.radio = vec![false; tracks.len()];
        self.tracks = tracks;
        self.order = (0..self.tracks.len()).collect();
        self.pos = if self.tracks.is_empty() { None } else { Some(start.min(self.tracks.len() - 1)) };
        if self.shuffle {
            self.reshuffle();
        }
    }

    pub fn current(&self) -> Option<&Track> {
        self.pos.and_then(|p| self.order.get(p)).and_then(|i| self.tracks.get(*i))
    }

    /// Order-position of the track that follows the current one.
    /// `auto` = natural end of track (honours repeat-one).
    pub fn next_pos(&self, auto: bool) -> Option<usize> {
        self.next_pos_from(self.pos?, auto)
    }

    /// Order-position following `p`.
    pub fn next_pos_from(&self, p: usize, auto: bool) -> Option<usize> {
        if auto && self.repeat == RepeatMode::One {
            return Some(p);
        }
        if p + 1 < self.order.len() {
            Some(p + 1)
        } else if self.repeat != RepeatMode::Off && !self.order.is_empty() {
            Some(0)
        } else {
            None
        }
    }

    pub fn peek_next(&self, auto: bool) -> Option<&Track> {
        self.next_pos(auto).and_then(|p| self.tracks.get(self.order[p]))
    }

    pub fn advance(&mut self, auto: bool) -> Option<&Track> {
        let n = self.next_pos(auto)?;
        self.pos = Some(n);
        self.current()
    }

    pub fn back(&mut self) -> Option<&Track> {
        let p = self.pos?;
        if p > 0 {
            self.pos = Some(p - 1);
        } else if self.repeat == RepeatMode::All && !self.order.is_empty() {
            self.pos = Some(self.order.len() - 1);
        }
        self.current()
    }

    fn reshuffle(&mut self) {
        let mut rng = rand::rng();
        match self.pos {
            Some(p) => {
                let cur = self.order[p];
                let mut rest: Vec<usize> = (0..self.tracks.len()).filter(|i| *i != cur).collect();
                rest.shuffle(&mut rng);
                self.order = std::iter::once(cur).chain(rest).collect();
                self.pos = Some(0);
            }
            None => {
                self.order = (0..self.tracks.len()).collect();
                self.order.shuffle(&mut rng);
            }
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        if on {
            self.reshuffle();
        } else {
            let cur = self.pos.map(|p| self.order[p]);
            self.order = (0..self.tracks.len()).collect();
            self.pos = cur;
        }
    }

    pub fn append(&mut self, tracks: Vec<Track>) {
        for t in tracks {
            self.tracks.push(t);
            self.radio.push(false);
            self.order.push(self.tracks.len() - 1);
        }
        if self.pos.is_none() && !self.order.is_empty() {
            self.pos = Some(0);
        }
    }

    pub fn insert_next(&mut self, tracks: Vec<Track>) {
        let at = self.pos.map(|p| p + 1).unwrap_or(0);
        for (k, t) in tracks.into_iter().enumerate() {
            self.tracks.push(t);
            self.radio.push(false);
            self.order.insert(at + k, self.tracks.len() - 1);
        }
        if self.pos.is_none() && !self.order.is_empty() {
            self.pos = Some(0);
        }
    }

    /// Remove by order-position. Returns true if the current track was removed.
    pub fn remove(&mut self, opos: usize) -> bool {
        if opos >= self.order.len() {
            return false;
        }
        let idx = self.order.remove(opos);
        self.tracks.remove(idx);
        if idx < self.radio.len() {
            self.radio.remove(idx);
        }
        for o in self.order.iter_mut() {
            if *o > idx {
                *o -= 1;
            }
        }
        match self.pos {
            Some(p) if p == opos => {
                if self.order.is_empty() {
                    self.pos = None;
                } else {
                    self.pos = Some(p.min(self.order.len() - 1));
                }
                true
            }
            Some(p) if p > opos => {
                self.pos = Some(p - 1);
                false
            }
            _ => false,
        }
    }

    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.order.len() || to >= self.order.len() || from == to {
            return;
        }
        let v = self.order.remove(from);
        self.order.insert(to, v);
        if let Some(p) = self.pos {
            self.pos = Some(if p == from {
                to
            } else if from < p && to >= p {
                p - 1
            } else if from > p && to <= p {
                p + 1
            } else {
                p
            });
        }
    }

    pub fn clear_upcoming(&mut self) {
        if let Some(p) = self.pos {
            let keep = self.order[p];
            let t = self.tracks[keep].clone();
            let r = self.radio.get(keep).copied().unwrap_or(false);
            self.tracks = vec![t];
            self.radio = vec![r];
            self.order = vec![0];
            self.pos = Some(0);
        } else {
            self.tracks.clear();
            self.radio.clear();
            self.order.clear();
        }
    }

    fn is_radio_at(&self, opos: usize) -> bool {
        self.order.get(opos).and_then(|i| self.radio.get(*i)).copied().unwrap_or(false)
    }

    /// "Add to queue": manual tracks go after the current track and other manual
    /// items but always *before* generated radio recommendations.
    pub fn insert_manual(&mut self, tracks: Vec<Track>) {
        let start = self.pos.map(|p| p + 1).unwrap_or(0);
        let mut at = (start..self.order.len()).find(|o| self.is_radio_at(*o)).unwrap_or(self.order.len());
        for t in tracks {
            self.tracks.push(t);
            self.radio.push(false);
            self.order.insert(at, self.tracks.len() - 1);
            at += 1;
        }
        if self.pos.is_none() && !self.order.is_empty() {
            self.pos = Some(0);
        }
    }

    /// Append Similarity Radio recommendations at the very end.
    pub fn append_radio(&mut self, tracks: Vec<Track>) {
        for t in tracks {
            self.tracks.push(t);
            self.radio.push(true);
            self.order.push(self.tracks.len() - 1);
        }
        if self.pos.is_none() && !self.order.is_empty() {
            self.pos = Some(0);
        }
    }

    /// Number of radio recommendations still to come.
    pub fn radio_upcoming(&self) -> usize {
        let start = self.pos.map(|p| p + 1).unwrap_or(0);
        (start..self.order.len()).filter(|o| self.is_radio_at(*o)).count()
    }

    /// Remove radio recommendations that haven't played yet.
    pub fn clear_radio_upcoming(&mut self) {
        let start = self.pos.map(|p| p + 1).unwrap_or(0);
        let mut o = self.order.len();
        while o > start {
            o -= 1;
            if self.is_radio_at(o) {
                self.remove(o);
            }
        }
    }

    pub fn radio_flags_in_order(&self) -> Vec<bool> {
        (0..self.order.len()).map(|o| self.is_radio_at(o)).collect()
    }

    pub fn in_order(&self) -> Vec<Track> {
        self.order.iter().map(|i| self.tracks[*i].clone()).collect()
    }
}

// ---------------------------------------------------------------------------
// Commands / events / state

#[derive(Clone, Debug)]
pub struct EngineOpts {
    pub quality: Quality,
    pub normalisation: bool,
    pub gapless: bool,
    pub fade_ms: u32,
    pub yt_open: YtOpen,
}

impl Default for EngineOpts {
    fn default() -> Self {
        EngineOpts { quality: Quality::Normal160, normalisation: false, gapless: true, fade_ms: 0, yt_open: YtOpen::YouTubeMusic }
    }
}

pub enum Cmd {
    Play { tracks: Vec<Track>, start: usize },
    Restore { tracks: Vec<Track>, start: usize, position_ms: u64 },
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
    Shutdown,
}

pub enum EngineEvent {
    Cmd(Cmd),
    LocalEnded(u64, Result<(), String>),
    SpotifyEnded(u64),
    SpotifyUnavailable(u64),
    SpotifyPreload,
    SpotifyPlaying(u64, u32),
    SpotifyRequestId(u64),
    SpotifyDisconnected,
    SpotifyConnected(Result<(), String>),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum SpotifyStatus {
    #[default]
    Disconnected,
    Connecting,
    Ready,
    Unavailable(String),
}

#[derive(Clone, Debug, Default)]
pub struct PlayerState {
    pub current: Option<Track>,
    pub playing: bool,
    pub duration_ms: u64,
    pub queue: Vec<Track>,
    pub queue_pos: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub volume: f32,
    pub muted: bool,
    /// The current track is playing outside Aqua Glass (YouTube in the browser).
    pub external: bool,
    pub message: Option<String>,
    pub error: Option<String>,
    pub spotify: SpotifyStatus,
    /// Parallel to `queue`: which entries are radio recommendations.
    pub queue_radio: Vec<bool>,
    /// Radio recommendations still to play.
    pub radio_upcoming: usize,
    /// The most recently finished/left track (for history, skips and radio learning).
    pub last_played: Option<PlayRecord>,
    /// Incremented on every change (for cheap change detection).
    pub version: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayRecord {
    pub track: Track,
    pub listened_ms: u64,
    /// Played to the end (or at least half of it).
    pub completed: bool,
    /// Left early by the user (Next / jump) before half of the track.
    pub skipped: bool,
    /// Sequence number so listeners can tell records apart.
    pub seq: u64,
}

pub type Listener = Box<dyn FnMut(&PlayerState) + Send>;
pub type TokenSource = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

/// Position bookkeeping shared with the UI (lock-free reads).
#[derive(Default)]
pub struct Clock {
    pub track_start_frame: AtomicU64,
    pub base_ms: AtomicU64,
    pub duration_ms: AtomicU64,
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

enum Active {
    None,
    Local(super::local::Decoding),
    Spotify,
    External,
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
            s.queue_radio = self.q.radio_flags_in_order();
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
            s.external = matches!(self.active, Active::External);
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
        self.active = Active::None;
        self.out.flush();
    }

    fn reset_clock(&mut self, base_ms: u64) {
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
            ProviderKind::Spotify => self.start_spotify(&track, start_ms as u32),
            ProviderKind::YouTube => {
                self.active = Active::External;
                self.playing = false;
                if play {
                    if let Some(u) = track.web_url(self.opts.yt_open) {
                        if let Err(e) = open::that_detached(&u) {
                            self.set_error(format!("Could not open browser: {e}"));
                        }
                    }
                }
                self.publish(|s| {
                    s.message = Some("Playing in YouTube Music (browser). Google offers no third-party audio API, so playback controls there apply.".into())
                });
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
                    match sp.load(&track.uri, start_ms) {
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

    fn pause_backend(&mut self) {
        #[cfg(feature = "spotify-playback")]
        if let (Active::Spotify, Some(sp)) = (&self.active, &self.spotify) {
            sp.pause();
        }
    }

    fn resume_backend(&mut self) {
        #[cfg(feature = "spotify-playback")]
        if let (Active::Spotify, Some(sp)) = (&self.active, &self.spotify) {
            sp.play();
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
        let gapless_ok = self.opts.gapless && Some(next.provider) == cur_provider && next.provider != ProviderKind::YouTube;
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
            EngineEvent::SpotifyEnded(_) => {
                if matches!(self.active, Active::Spotify) {
                    self.source_finished();
                }
            }
            EngineEvent::SpotifyUnavailable(_) => {
                self.set_error("This Spotify track is unavailable in your region or account.");
                if self.q.advance(false).is_some() {
                    self.start_current(0, true);
                }
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
            EngineEvent::SpotifyPlaying(..) | EngineEvent::SpotifyRequestId(_) => {}
            EngineEvent::SpotifyDisconnected => {
                self.publish(|s| s.spotify = SpotifyStatus::Unavailable("Spotify session disconnected; reconnecting on next play.".into()));
                #[cfg(feature = "spotify-playback")]
                {
                    self.spotify = None;
                }
            }
            EngineEvent::SpotifyConnected(_) => {}
        }
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Play { tracks, start } => {
                self.q.set(tracks, start);
                self.pending_restore_ms = None;
                self.start_current(0, true);
            }
            Cmd::Restore { tracks, start, position_ms } => {
                self.q.set(tracks, start);
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
                    Active::External => {
                        self.start_current(0, true);
                    }
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
                self.publish(|s| s.volume = v.clamp(0.0, 1.0));
            }
            Cmd::ToggleMute => {
                let m = !self.state.read().unwrap().muted;
                self.out.set_muted(m);
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
            Cmd::Shutdown => {}
        }
    }

    fn position(&self) -> u64 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: &str) -> Track {
        Track { id: n.into(), title: n.into(), provider: ProviderKind::Local, ..Default::default() }
    }

    fn titles(q: &Queue) -> Vec<String> {
        q.in_order().into_iter().map(|t| t.title).collect()
    }

    #[test]
    fn advance_and_repeat() {
        let mut q = Queue::default();
        q.set(vec![t("a"), t("b"), t("c")], 1);
        assert_eq!(q.current().unwrap().title, "b");
        assert_eq!(q.advance(true).unwrap().title, "c");
        assert!(q.advance(true).is_none());
        q.repeat = RepeatMode::All;
        assert_eq!(q.advance(true).unwrap().title, "a");
        q.repeat = RepeatMode::One;
        assert_eq!(q.advance(true).unwrap().title, "a");
        assert_eq!(q.advance(false).unwrap().title, "b", "manual next ignores repeat-one");
        assert_eq!(q.back().unwrap().title, "a");
    }

    #[test]
    fn shuffle_keeps_current_and_all_tracks() {
        let mut q = Queue::default();
        q.set((0..50).map(|i| t(&i.to_string())).collect(), 10);
        q.set_shuffle(true);
        assert_eq!(q.current().unwrap().title, "10");
        assert_eq!(q.pos, Some(0));
        let mut seen = titles(&q);
        seen.sort();
        assert_eq!(seen.len(), 50);
        seen.dedup();
        assert_eq!(seen.len(), 50);
        q.advance(false);
        let cur = q.current().unwrap().title.clone();
        q.set_shuffle(false);
        assert_eq!(q.current().unwrap().title, cur);
        assert_eq!(titles(&q)[0], "0");
    }

    #[test]
    fn insert_remove_move() {
        let mut q = Queue::default();
        q.set(vec![t("a"), t("b"), t("c")], 0);
        q.insert_next(vec![t("x")]);
        assert_eq!(titles(&q), ["a", "x", "b", "c"]);
        q.append(vec![t("z")]);
        assert_eq!(titles(&q), ["a", "x", "b", "c", "z"]);
        assert!(!q.remove(2));
        assert_eq!(titles(&q), ["a", "x", "c", "z"]);
        q.move_item(3, 1);
        assert_eq!(titles(&q), ["a", "z", "x", "c"]);
        assert_eq!(q.current().unwrap().title, "a");
        q.move_item(0, 2);
        assert_eq!(q.current().unwrap().title, "a");
        assert_eq!(q.pos, Some(2));
        assert!(q.remove(2));
        assert_eq!(q.current().unwrap().title, "c");
        q.clear_upcoming();
        assert_eq!(titles(&q), ["c"]);
    }

    #[test]
    fn manual_queue_has_priority_over_radio() {
        let mut q = Queue::default();
        q.set(vec![t("seed")], 0);
        q.append_radio(vec![t("r1"), t("r2"), t("r3")]);
        assert_eq!(q.radio_upcoming(), 3);
        q.insert_manual(vec![t("m1")]);
        q.insert_manual(vec![t("m2")]);
        assert_eq!(titles(&q), ["seed", "m1", "m2", "r1", "r2", "r3"]);
        // play next still goes right after the current track
        q.insert_next(vec![t("n")]);
        assert_eq!(titles(&q), ["seed", "n", "m1", "m2", "r1", "r2", "r3"]);
        q.advance(false);
        q.advance(false);
        q.advance(false);
        q.advance(false); // now at r1
        assert_eq!(q.current().unwrap().title, "r1");
        assert_eq!(q.radio_upcoming(), 2);
        q.clear_radio_upcoming();
        assert_eq!(titles(&q), ["seed", "n", "m1", "m2", "r1"], "only unplayed radio items are removed");
        assert_eq!(q.current().unwrap().title, "r1");
        q.append_radio(vec![t("r4")]);
        q.insert_manual(vec![t("m3")]);
        assert_eq!(titles(&q), ["seed", "n", "m1", "m2", "r1", "m3", "r4"]);
        assert_eq!(q.radio_flags_in_order(), [false, false, false, false, true, false, true]);
    }

    #[test]
    fn large_queue() {
        let mut q = Queue::default();
        q.set((0..20_000).map(|i| t(&i.to_string())).collect(), 0);
        q.set_shuffle(true);
        for _ in 0..19_999 {
            assert!(q.advance(true).is_some());
        }
        assert!(q.advance(true).is_none());
    }
}
