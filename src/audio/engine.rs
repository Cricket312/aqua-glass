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
    pub yt_open: YtOpen,
}

impl Default for EngineOpts {
    fn default() -> Self {
        EngineOpts { quality: Quality::Normal160, normalisation: false, gapless: true, fade_ms: 0, yt_open: YtOpen::YouTubeMusic }
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




pub type Listener = aqua_core::playback::StateListener;
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

