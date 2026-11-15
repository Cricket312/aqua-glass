//! Automatic Similarity Radio.
//!
//! Pipeline: **gather** similar (artist, title) candidates for a seed track from
//! Last.fm (mode-dependent), **rank/filter** them with the user's rules
//! (duplicates, recently played, unheard, repeated artists, skips/likes),
//! **resolve** each to a *playable* track (local library, Spotify search with
//! strict matching; unresolvable candidates are dropped — never shown as
//! playable), and append them to the engine's radio queue section.
//!
//! The worker thread exists only while the feature is enabled. It wakes on
//! engine state changes (no polling) and refills when fewer than `threshold`
//! recommendations remain. Without a Last.fm key it falls back to an
//! artist/album-based source and says so.
//!
//! Portability: this crate only depends on `aqua-core` and `aqua-providers`.
//! It drives playback through `aqua_core::playback::PlaybackControl` and gets
//! local music through a closure, so a future Android client reuses the same
//! gathering, ranking, duplicate prevention, resolution and 15/5 refill logic.

use aqua_core::history::{ident, track_ident, History};
use aqua_core::matching::{self, MatchStatus};
use aqua_core::model::Track;
use aqua_core::playback::{PlayRecord, PlaybackControl, SpotifyStatus};
use aqua_providers::lastfm::{LastFm, Similar};
use rand::Rng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub use aqua_core::prefs::{RadioPrefs, RadioProvider, Strength};

/// User preferences (from settings).
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub strength: Strength,
    pub target: usize,
    pub threshold: usize,
    pub provider: RadioProvider,
    pub endless: bool,
    pub prefer_unheard: bool,
    pub avoid_recent: bool,
    pub avoid_duplicates: bool,
    pub allow_repeat_artists: bool,
    pub learn: bool,
    pub stay_close: bool,
    pub lastfm_key: String,
    /// Internal: candidates come from the same-artist fallback (no per-artist cap).
    pub same_artist_source: bool,
}

impl Prefs {
    /// Runtime preferences from the (portable, sync-able) user preferences plus
    /// this device's Last.fm key.
    pub fn from_user(u: &RadioPrefs, lastfm_key: &str) -> Prefs {
        Prefs {
            strength: u.strength,
            target: u.target,
            threshold: u.threshold,
            provider: u.provider,
            endless: u.endless,
            prefer_unheard: u.prefer_unheard,
            avoid_recent: u.avoid_recent,
            avoid_duplicates: u.avoid_duplicates,
            allow_repeat_artists: u.allow_repeat_artists,
            learn: u.learn,
            stay_close: u.stay_close,
            lastfm_key: lastfm_key.to_string(),
            same_artist_source: false,
        }
    }
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            strength: Strength::Balanced,
            target: 15,
            threshold: 5,
            provider: RadioProvider::Automatic,
            endless: true,
            prefer_unheard: true,
            avoid_recent: true,
            avoid_duplicates: true,
            allow_repeat_artists: false,
            learn: true,
            stay_close: false,
            lastfm_key: String::new(),
            same_artist_source: false,
        }
    }
}

/// A recommendation candidate (optionally already resolved to a playable track).
#[derive(Clone, Debug)]
pub struct Cand {
    pub artist: String,
    pub title: String,
    pub score: f32,
    pub listeners: Option<u64>,
    pub track: Option<Track>,
}

impl From<Similar> for Cand {
    fn from(s: Similar) -> Cand {
        Cand { artist: s.artist, title: s.title, score: s.score, listeners: s.listeners, track: None }
    }
}

/// Similarity source abstraction (Last.fm in production; fakes in tests).
pub trait Source {
    fn similar_tracks(&self, artist: &str, title: &str, limit: u32) -> anyhow::Result<Vec<Similar>>;
    fn similar_artists(&self, artist: &str, limit: u32) -> anyhow::Result<Vec<(String, f32)>>;
    fn top_tracks(&self, artist: &str, limit: u32) -> anyhow::Result<Vec<Similar>>;
}

impl Source for LastFm {
    fn similar_tracks(&self, a: &str, t: &str, l: u32) -> anyhow::Result<Vec<Similar>> {
        LastFm::similar_tracks(self, a, t, l)
    }
    fn similar_artists(&self, a: &str, l: u32) -> anyhow::Result<Vec<(String, f32)>> {
        LastFm::similar_artists(self, a, l)
    }
    fn top_tracks(&self, a: &str, l: u32) -> anyhow::Result<Vec<Similar>> {
        LastFm::top_tracks(self, a, l)
    }
}

fn primary_artist(t: &Track) -> String {
    t.artists.first().cloned().unwrap_or_default()
}

/// Gather candidates for a seed. Each mode uses the similarity graph differently:
/// * Familiar — the seed's own similar-track list, high matches only.
/// * Balanced — similar tracks + top tracks of the 6 closest similar artists.
/// * Adventurous — similar tracks (down-weighted) + top tracks of artists 4–24 in the similar-artist list.
/// * Deep Discovery — deeper similar artists (10–60) and their *less popular* tracks, popularity-penalised; seed artist excluded.
pub fn gather(seed: &Track, strength: Strength, src: &dyn Source) -> anyhow::Result<Vec<Cand>> {
    let artist = primary_artist(seed);
    let (core, _) = matching::split_qualifiers(&seed.title);
    let title = if core.is_empty() { seed.title.clone() } else { seed.title.clone() };
    let mut out: Vec<Cand> = Vec::new();
    let seed_artist_lc = artist.to_lowercase();
    match strength {
        Strength::Familiar => {
            out.extend(src.similar_tracks(&artist, &title, 60)?.into_iter().filter(|s| s.score >= 0.2).map(Cand::from));
            if out.len() < 10 {
                // thin data: fall back to the closest artists' best-known songs
                for (a, m) in src.similar_artists(&artist, 4)? {
                    out.extend(src.top_tracks(&a, 4)?.into_iter().map(|s| Cand { score: m * 0.6, ..Cand::from(s) }));
                }
            }
        }
        Strength::Balanced => {
            out.extend(src.similar_tracks(&artist, &title, 100)?.into_iter().map(Cand::from));
            for (a, m) in src.similar_artists(&artist, 12)?.into_iter().take(6) {
                out.extend(src.top_tracks(&a, 6)?.into_iter().map(|s| {
                    let sc = m * 0.7 * (1.0 - s.rank as f32 * 0.05);
                    Cand { score: sc, ..Cand::from(s) }
                }));
            }
        }
        Strength::Adventurous => {
            out.extend(src.similar_tracks(&artist, &title, 100)?.into_iter().map(|s| Cand { score: s.score * 0.6, ..Cand::from(s) }));
            for (a, m) in src.similar_artists(&artist, 40)?.into_iter().skip(4).take(20) {
                out.extend(src.top_tracks(&a, 10)?.into_iter().map(|s| {
                    let sc = (0.35 + m * 0.6) * (1.0 - s.rank as f32 * 0.03);
                    Cand { score: sc, ..Cand::from(s) }
                }));
            }
            for c in out.iter_mut() {
                if c.artist.to_lowercase() == seed_artist_lc {
                    c.score *= 0.3;
                }
            }
        }
        Strength::DeepDiscovery => {
            for (a, m) in src.similar_artists(&artist, 60)?.into_iter().skip(10).take(12) {
                out.extend(src.top_tracks(&a, 30)?.into_iter().filter(|s| s.rank >= 5).map(|s| {
                    let pop = s.listeners.map(|l| ((l as f32 + 10.0).log10() / 7.0).clamp(0.0, 1.0)).unwrap_or(0.5);
                    Cand { score: (0.4 + m * 0.4) * (1.1 - pop), ..Cand::from(s) }
                }));
            }
            out.extend(src.similar_tracks(&artist, &title, 150)?.into_iter().filter(|s| s.rank >= 40).map(|s| Cand { score: s.score * 0.5, ..Cand::from(s) }));
            out.retain(|c| c.artist.to_lowercase() != seed_artist_lc);
        }
    }
    Ok(out)
}

/// Session memory for one radio run.
#[derive(Default, Clone, Debug)]
pub struct Session {
    /// Idents already recommended or queued (never repeat in a session).
    pub seen: HashSet<String>,
    /// Artists of the most recent recommendations (repeat-artist spacing).
    pub recent_artists: VecDeque<String>,
    /// Skips per artist (lower-cased) — "learn from skips".
    pub skips: HashMap<String, u32>,
    /// Artists of liked tracks — "learn from likes".
    pub liked: HashSet<String>,
}

/// Apply the user's rules and order candidates (best first). Pure — unit tested.
pub fn rank(cands: Vec<Cand>, seed: &Track, p: &Prefs, s: &Session, heard: &dyn Fn(&str) -> bool, heard_recently: &dyn Fn(&str) -> bool, rng: &mut impl Rng) -> Vec<Cand> {
    let seed_id = track_ident(seed);
    let seed_artist = primary_artist(seed).to_lowercase();
    let mut best: HashMap<String, Cand> = HashMap::new();
    for mut c in cands {
        if c.title.trim().is_empty() || c.artist.trim().is_empty() {
            continue;
        }
        let id = ident(&c.artist, &c.title);
        if id == seed_id || (p.avoid_duplicates && s.seen.contains(&id)) {
            continue;
        }
        if p.avoid_recent && heard_recently(&id) {
            continue;
        }
        let a = c.artist.to_lowercase();
        if p.learn {
            let skips = s.skips.get(&a).copied().unwrap_or(0);
            // three skips drop an artist for the session — except the seed's own artist,
            // which is only down-weighted (otherwise the radio could lose its anchor)
            if skips >= 3 && a != seed_artist {
                continue;
            }
            if skips > 0 {
                c.score *= 0.6f32.powi(skips.min(4) as i32);
            }
            if s.liked.contains(&a) {
                c.score *= 1.25;
            }
        }
        if heard(&id) {
            if p.strength == Strength::DeepDiscovery {
                continue;
            }
            if p.prefer_unheard {
                c.score *= 0.35;
            }
        }
        c.score += rng.random::<f32>() * p.strength.jitter();
        match best.get(&id) {
            Some(b) if b.score >= c.score => {}
            _ => {
                best.insert(id, c);
            }
        }
    }
    let mut v: Vec<Cand> = best.into_values().collect();
    v.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    // artist spacing
    let per_batch = if p.same_artist_source { usize::MAX } else if p.allow_repeat_artists { 3 } else { 1 };
    let mut counts: HashMap<String, usize> = HashMap::new();
    v.retain(|c| {
        let a = c.artist.to_lowercase();
        if !p.allow_repeat_artists && s.recent_artists.iter().any(|r| *r == a) {
            return false;
        }
        let n = counts.entry(a).or_default();
        *n += 1;
        *n <= per_batch
    });
    v
}

/// Turns candidates into playable tracks.
pub trait Resolver {
    fn resolve(&self, c: &Cand) -> Option<Track>;
}

/// Production resolver: local library index first, then Spotify search (strict matching).
pub struct StreamResolver {
    pub local: HashMap<String, Track>,
    pub spotify: Option<Arc<aqua_providers::spotify::SpotifyApi>>,
    pub provider: RadioProvider,
    pub spotify_playable: bool,
    /// Remembered lookups (hits *and* misses) shared across refills.
    pub cache: Arc<Mutex<ResolveCache>>,
    /// Remaining network searches allowed for this refill.
    pub budget: std::cell::Cell<usize>,
}

/// Bounded cache of candidate › playable track lookups (None = known miss).
#[derive(Default)]
pub struct ResolveCache {
    map: HashMap<String, Option<Track>>,
    order: VecDeque<String>,
}

pub const RESOLVE_CACHE_MAX: usize = 600;
/// Network searches per refill (keeps radio far below service rate limits).
pub const SEARCH_BUDGET: usize = 12;

impl ResolveCache {
    pub fn get(&self, k: &str) -> Option<&Option<Track>> {
        self.map.get(k)
    }
    pub fn put(&mut self, k: String, v: Option<Track>) {
        if self.map.insert(k.clone(), v).is_none() {
            self.order.push_back(k);
            while self.order.len() > RESOLVE_CACHE_MAX {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl StreamResolver {
    fn take_budget(&self) -> bool {
        let b = self.budget.get();
        if b == 0 {
            return false;
        }
        self.budget.set(b - 1);
        true
    }
}

fn accept(src: &Track, cands: Vec<Track>) -> Option<Track> {
    let (r, st) = matching::rank(src, cands);
    let b = r.into_iter().next()?;
    let ok = st == MatchStatus::Matched || (b.score >= 0.75 && b.notes.iter().all(|n| !n.starts_with("version")));
    ok.then_some(b.track)
}

impl Resolver for StreamResolver {
    fn resolve(&self, c: &Cand) -> Option<Track> {
        if let Some(t) = &c.track {
            return Some(t.clone());
        }
        let src = Track { title: c.title.clone(), artists: vec![c.artist.clone()], ..Default::default() };
        let id = ident(&c.artist, &c.title);
        let use_local = matches!(self.provider, RadioProvider::Automatic | RadioProvider::Local);
        let use_spotify = matches!(self.provider, RadioProvider::Automatic | RadioProvider::Spotify) && self.spotify_playable;
        if use_local {
            if let Some(t) = self.local.get(&id) {
                return Some(t.clone());
            }
        }
        let key = format!("{:?}|{id}", self.provider);
        if let Some(hit) = self.cache.lock().unwrap().get(&key) {
            return hit.clone();
        }
        let mut found = None;
        let mut searched = false;
        if use_spotify {
            if let Some(sp) = &self.spotify {
                if !self.take_budget() {
                    return None; // budget exhausted: try again on the next refill
                }
                searched = true;
                let q = format!("track:\"{}\" artist:\"{}\"", c.title.replace('"', ""), c.artist.replace('"', ""));
                match sp.search_tracks(&q, 5) {
                    Ok(r) => found = accept(&src, r),
                    Err(aqua_providers::ApiError::RateLimited(_)) => {
                        self.budget.set(0); // stop searching for this refill
                        return None;
                    }
                    Err(_) => return None, // transient: don't cache
                }
            }
        }
        if searched {
            self.cache.lock().unwrap().put(key, found.clone());
        }
        found
    }
}

/// Fallback without Last.fm: tracks by the same artist / from the same album
/// in the local library and (if available) Spotify. Clearly labelled in the UI.
pub fn fallback_candidates(seed: &Track, local: &[Track], spotify: Option<&aqua_providers::spotify::SpotifyApi>) -> Vec<Cand> {
    let a = primary_artist(seed).to_lowercase();
    let mut out: Vec<Cand> = local
        .iter()
        .filter(|t| t.artists.iter().any(|x| x.to_lowercase() == a) || (!seed.album.is_empty() && t.album == seed.album))
        .map(|t| Cand { artist: primary_artist(t), title: t.title.clone(), score: if t.album == seed.album { 0.85 } else { 0.9 }, listeners: None, track: Some(t.clone()) })
        .collect();
    if let Some(sp) = spotify {
        if let Ok(r) = sp.search_tracks(&format!("artist:\"{}\"", primary_artist(seed).replace('"', "")), 10) {
            out.extend(r.into_iter().map(|t| Cand { artist: primary_artist(&t), title: t.title.clone(), score: 0.8, listeners: None, track: Some(t) }));
        }
    }
    out
}

/// Build one batch of up to `n` playable recommendations.
pub fn build_batch(ranked: Vec<Cand>, n: usize, resolver: &dyn Resolver, session: &mut Session, max_attempts: usize) -> Vec<Track> {
    let mut out = Vec::new();
    for (attempts, c) in ranked.into_iter().enumerate() {
        if out.len() >= n || attempts >= max_attempts {
            break;
        }
        if let Some(t) = resolver.resolve(&c) {
            let id = track_ident(&t);
            if session.seen.contains(&id) {
                continue;
            }
            session.seen.insert(id);
            session.seen.insert(ident(&c.artist, &c.title));
            let a = c.artist.to_lowercase();
            session.recent_artists.push_back(a);
            while session.recent_artists.len() > 4 {
                session.recent_artists.pop_front();
            }
            out.push(t);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Worker

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub active: bool,
    pub seed: Option<Track>,
    pub original_seed: Option<Track>,
    pub source: String,
    pub fetching: bool,
    pub message: Option<String>,
    pub error: Option<String>,
    pub generated: usize,
    pub batches: usize,
}

pub enum Msg {
    Start(Track),
    Stop,
    Refresh,
    MoreLike(Track),
    Liked(Track),
    Prefs(Prefs),
    Tick { current: Option<Track>, upcoming: usize, queue: Vec<Track>, last: Option<PlayRecord>, current_is_radio: bool },
    Fetched { run: u64, res: Result<Vec<Track>, String>, source: String, session: Box<Session> },
    Shutdown,
}

pub struct Deps {
    /// The platform's playback controller (desktop: `audio::engine::Engine`).
    pub engine: Arc<dyn PlaybackControl>,
    pub spotify: Arc<aqua_providers::spotify::SpotifyApi>,
    /// Local music the radio may play (desktop: the scanned local-files library).
    pub local_tracks: Arc<dyn Fn() -> Vec<Track> + Send + Sync>,
    pub history: Arc<Mutex<History>>,
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

pub struct Radio {
    tx: mpsc::Sender<Msg>,
    pub status: Arc<Mutex<Status>>,
}

impl Radio {
    pub fn start(deps: Deps, prefs: Prefs) -> Radio {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let st = status.clone();
        let tx2 = tx.clone();
        // feed engine changes to the worker (listener lives as long as the sender works)
        let fwd = Arc::new(Mutex::new(Some(tx.clone())));
        let f2 = fwd.clone();
        deps.engine.add_listener(Box::new(move |s| {
            let mut g = f2.lock().unwrap();
            if let Some(t) = g.as_ref() {
                let cur_radio = s.queue_pos.is_some_and(|p| s.is_radio(p));
                let m = Msg::Tick { current: s.current.clone(), upcoming: s.radio_upcoming, queue: s.queue.clone(), last: s.last_played.clone(), current_is_radio: cur_radio };
                if t.send(m).is_err() {
                    *g = None;
                }
            }
        }));
        std::thread::Builder::new().name("similarity-radio".into()).spawn(move || worker(rx, tx2, st, deps, prefs)).expect("radio thread");
        Radio { tx, status }
    }
    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }
}

impl Drop for Radio {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

struct W {
    deps: Deps,
    prefs: Prefs,
    status: Arc<Mutex<Status>>,
    tx: mpsc::Sender<Msg>,
    session: Session,
    run: u64,
    last_fail: Option<Instant>,
    stalled: bool,
    last_seq: u64,
    upcoming: usize,
    current: Option<Track>,
    resolve_cache: Arc<Mutex<ResolveCache>>,
}

fn worker(rx: mpsc::Receiver<Msg>, tx: mpsc::Sender<Msg>, status: Arc<Mutex<Status>>, deps: Deps, prefs: Prefs) {
    let mut w = W { deps, prefs, status, tx, session: Session::default(), run: 0, last_fail: None, stalled: false, last_seq: 0, upcoming: 0, current: None, resolve_cache: Arc::new(Mutex::new(ResolveCache::default())) };
    while let Ok(m) = rx.recv() {
        match m {
            Msg::Shutdown => break,
            Msg::Prefs(p) => w.prefs = p,
            Msg::Start(seed) => w.start(seed),
            Msg::Stop => {
                w.run += 1;
                *w.status.lock().unwrap() = Status::default();
                w.deps.engine.clear_radio_upcoming();
            }
            Msg::Refresh => {
                if w.status.lock().unwrap().active {
                    w.deps.engine.clear_radio_upcoming();
                    w.upcoming = 0;
                    w.run += 1;
                    w.status.lock().unwrap().fetching = false;
                    w.stalled = false;
                    w.fetch(true);
                }
            }
            Msg::MoreLike(t) => {
                if !w.status.lock().unwrap().active {
                    w.start(t);
                } else {
                    w.deps.engine.clear_radio_upcoming();
                    w.upcoming = 0;
                    w.run += 1;
                    {
                        let mut s = w.status.lock().unwrap();
                        s.seed = Some(t.clone());
                        s.original_seed = Some(t);
                        s.fetching = false;
                    }
                    w.stalled = false;
                    w.fetch(true);
                }
            }
            Msg::Liked(t) => {
                if let Some(a) = t.artists.first() {
                    w.session.liked.insert(a.to_lowercase());
                }
                if w.prefs.learn && !w.prefs.stay_close && w.status.lock().unwrap().active {
                    w.status.lock().unwrap().seed = Some(t);
                }
            }
            Msg::Tick { current, upcoming, queue, last, current_is_radio } => {
                w.upcoming = upcoming;
                let changed = current.as_ref().map(|t| t.key()) != w.current.as_ref().map(|t| t.key());
                if changed {
                    w.stalled = false;
                }
                w.current = current.clone();
                for t in &queue {
                    w.session.seen.insert(track_ident(t));
                }
                if let Some(r) = last {
                    if r.seq != w.last_seq {
                        w.last_seq = r.seq;
                        if r.skipped && w.status.lock().unwrap().active {
                            if let Some(a) = r.track.artists.first() {
                                *w.session.skips.entry(a.to_lowercase()).or_default() += 1;
                            }
                        }
                    }
                }
                // drift: follow the radio as it plays (unless "Stay Close to Original")
                if changed && current_is_radio && !w.prefs.stay_close {
                    if let Some(c) = current {
                        w.status.lock().unwrap().seed = Some(c);
                    }
                }
                w.maybe_refill();
            }
            Msg::Fetched { run, res, source, session } => {
                if run != w.run {
                    continue; // stale (stopped/refreshed meanwhile)
                }
                w.session.seen.extend(session.seen);
                w.session.recent_artists = session.recent_artists;
                let mut s = w.status.lock().unwrap();
                s.fetching = false;
                s.source = source;
                match res {
                    Ok(t) if !t.is_empty() => {
                        s.generated += t.len();
                        s.batches += 1;
                        s.error = None;
                        s.message = None;
                        w.upcoming += t.len();
                        w.deps.engine.append_radio(t);
                    }
                    Ok(_) => {
                        if s.generated > 0 {
                            s.message = Some("No more similar songs available right now — trying again after the next song.".into());
                        } else {
                            s.error = Some("No playable recommendations found for this song with the current settings and sources.".into());
                        }
                        w.stalled = true;
                    }
                    Err(e) => {
                        s.error = Some(e);
                        w.last_fail = Some(Instant::now());
                    }
                }
                drop(s);
                (w.deps.wake)();
            }
        }
    }
}

impl W {
    fn start(&mut self, seed: Track) {
        self.run += 1;
        self.session = Session::default();
        self.session.seen.insert(track_ident(&seed));
        self.stalled = false;
        self.last_fail = None;
        *self.status.lock().unwrap() = Status { active: true, seed: Some(seed.clone()), original_seed: Some(seed), ..Default::default() };
        self.upcoming = 0;
        self.fetch(true);
    }

    fn maybe_refill(&mut self) {
        let (active, fetching, batches) = {
            let s = self.status.lock().unwrap();
            (s.active, s.fetching, s.batches)
        };
        if !active || fetching || self.stalled {
            return;
        }
        if batches > 0 && !self.prefs.endless {
            return;
        }
        if self.last_fail.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        if self.upcoming < self.prefs.threshold.max(1) {
            self.fetch(false);
        }
    }

    fn fetch(&mut self, first: bool) {
        let seed = {
            let mut s = self.status.lock().unwrap();
            s.fetching = true;
            if self.prefs.stay_close { s.original_seed.clone() } else { s.seed.clone() }
        };
        let Some(seed) = seed else { return };
        let want = self.prefs.target.saturating_sub(if first { 0 } else { self.upcoming }).max(1);
        let run = self.run;
        let prefs = self.prefs.clone();
        let mut session = self.session.clone();
        let tx = self.tx.clone();
        let spotify = self.deps.spotify.clone();
        let local_tracks = self.deps.local_tracks.clone();
        let history = self.deps.history.clone();
        let spotify_playable = self.deps.engine.snapshot().spotify == SpotifyStatus::Ready && spotify.is_signed_in();
        let cache = self.resolve_cache.clone();
        // the fetch thread works on a copy of the session that is merged back afterwards
        std::thread::Builder::new()
            .name("radio-fetch".into())
            .spawn(move || {
                let _bg = aqua_providers::ratelimit::background();
                let local: Vec<Track> = local_tracks();
                let index: HashMap<String, Track> = local.iter().map(|t| (track_ident(t), t.clone())).collect();
                let mut prefs = prefs;
                let (cands, source) = if prefs.lastfm_key.trim().is_empty() {
                    // the fallback is same-artist/same-album music by definition
                    prefs.allow_repeat_artists = true;
                    prefs.same_artist_source = true;
                    let sp = if spotify_playable { Some(&*spotify) } else { None };
                    (Ok(fallback_candidates(&seed, &local, sp)), "Artist & album based (Last.fm not configured)".to_string())
                } else {
                    let lf = LastFm::new(&prefs.lastfm_key);
                    (gather(&seed, prefs.strength, &lf).map_err(|e| e.to_string()), format!("Last.fm · {}", prefs.strength.label()))
                };
                drop(local);
                let res = cands.map(|c| {
                    let h = history.lock().unwrap();
                    let week = aqua_core::storage::now_unix() - 7 * 86400;
                    let mut rng = rand::rng();
                    let ranked = rank(c, &seed, &prefs, &session, &|id| h.heard(id), &|id| h.heard_since(id, week), &mut rng);
                    drop(h);
                    let resolver = StreamResolver { local: index, spotify: Some(spotify.clone()), provider: prefs.provider, spotify_playable, cache, budget: std::cell::Cell::new(SEARCH_BUDGET) };
                    if !spotify_playable && resolver.local.is_empty() {
                        return Err("No playable source: sign in to Spotify (Premium) or add local music.".to_string());
                    }
                    Ok(build_batch(ranked, want, &resolver, &mut session, want * 6))
                });
                let res = match res {
                    Ok(r) => r,
                    Err(e) => Err(e),
                };
                let _ = tx.send(Msg::Fetched { run, res, source, session: Box::new(session) });
            })
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqua_core::model::ProviderKind;
    use std::collections::HashMap as Map;

    /// Deterministic in-memory similarity graph for tests.
    struct Fake {
        tracks: Map<String, Vec<Similar>>,
        artists: Map<String, Vec<(String, f32)>>,
        top: Map<String, Vec<Similar>>,
    }
    fn sim(a: &str, t: &str, score: f32, rank: usize, listeners: u64) -> Similar {
        Similar { artist: a.into(), title: t.into(), score, listeners: Some(listeners), rank }
    }
    impl Source for Fake {
        fn similar_tracks(&self, a: &str, t: &str, _l: u32) -> anyhow::Result<Vec<Similar>> {
            Ok(self.tracks.get(&format!("{a}|{t}")).cloned().unwrap_or_default())
        }
        fn similar_artists(&self, a: &str, _l: u32) -> anyhow::Result<Vec<(String, f32)>> {
            Ok(self.artists.get(a).cloned().unwrap_or_default())
        }
        fn top_tracks(&self, a: &str, l: u32) -> anyhow::Result<Vec<Similar>> {
            Ok(self.top.get(a).cloned().unwrap_or_default().into_iter().take(l as usize).collect())
        }
    }
    fn fake() -> Fake {
        let mut tracks = Map::new();
        tracks.insert(
            "Seed|Song".to_string(),
            {
                // ordered by similarity, like real Last.fm responses
                let mut v: Vec<Similar> = (0..60).map(|i| sim(&format!("A{}", i % 20), &format!("S{i}"), 1.0 - i as f32 / 60.0, i, 100_000)).chain(std::iter::once(sim("Seed", "Other", 0.95, 60, 1000))).collect();
                v.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
                for (i, x) in v.iter_mut().enumerate() {
                    x.rank = i;
                }
                v
            },
        );
        let artists: Vec<(String, f32)> = (0..60).map(|i| (format!("R{i}"), 1.0 - i as f32 / 60.0)).collect();
        let mut am = Map::new();
        am.insert("Seed".to_string(), artists);
        let mut top = Map::new();
        for i in 0..60 {
            top.insert(format!("R{i}"), (0..30).map(|k| sim(&format!("R{i}"), &format!("T{k}"), 1.0, k, 10u64.pow(7 - (k as u32 / 6)))).collect());
        }
        Fake { tracks, artists: am, top }
    }
    fn seed() -> Track {
        Track { title: "Song".into(), artists: vec!["Seed".into()], ..Default::default() }
    }
    struct All;
    impl Resolver for All {
        fn resolve(&self, c: &Cand) -> Option<Track> {
            Some(Track { provider: ProviderKind::Local, id: format!("{}-{}", c.artist, c.title), title: c.title.clone(), artists: vec![c.artist.clone()], ..Default::default() })
        }
    }
    struct OnlyEven;
    impl Resolver for OnlyEven {
        fn resolve(&self, c: &Cand) -> Option<Track> {
            let n: usize = c.title.trim_start_matches(|ch: char| !ch.is_ascii_digit()).parse().ok()?;
            (n % 2 == 0).then(|| All.resolve(c).unwrap())
        }
    }
    fn batch(strength: Strength, p: Prefs, s: &mut Session, res: &dyn Resolver) -> Vec<Track> {
        let c = gather(&seed(), strength, &fake()).unwrap();
        let mut rng = rand::rng();
        let r = rank(c, &seed(), &Prefs { strength, ..p }, s, &|_| false, &|_| false, &mut rng);
        build_batch(r, 15, res, s, 60)
    }
    fn artists(v: &[Track]) -> HashSet<String> {
        v.iter().map(|t| t.artists[0].clone()).collect()
    }

    #[test]
    fn modes_are_meaningfully_different() {
        let p = Prefs::default();
        let fam = batch(Strength::Familiar, p.clone(), &mut Session::default(), &All);
        let bal = batch(Strength::Balanced, p.clone(), &mut Session::default(), &All);
        let adv = batch(Strength::Adventurous, p.clone(), &mut Session::default(), &All);
        let deep = batch(Strength::DeepDiscovery, p.clone(), &mut Session::default(), &All);
        for b in [&fam, &bal, &adv, &deep] {
            assert_eq!(b.len(), 15);
        }
        // Familiar: only the seed's direct similar tracks (A* artists / seed artist)
        assert!(fam.iter().all(|t| t.artists[0].starts_with('A') || t.artists[0] == "Seed"), "{fam:?}");
        // Balanced mixes in close related artists (R0..R5)
        assert!(bal.iter().all(|t| t.artists[0].starts_with('A') || t.artists[0] == "Seed" || ["R0", "R1", "R2", "R3", "R4", "R5"].contains(&t.artists[0].as_str())));
        // Adventurous reaches further out (R4..R23)
        assert!(adv.iter().any(|t| t.artists[0].starts_with('R') && t.artists[0][1..].parse::<u32>().unwrap() >= 6), "{adv:?}");
        // Deep: never the seed artist, never the top-5 hits, deeper artists only
        assert!(deep.iter().all(|t| t.artists[0] != "Seed"));
        assert!(deep.iter().filter(|t| t.artists[0].starts_with('R')).all(|t| t.artists[0][1..].parse::<u32>().unwrap() >= 10 && t.title[1..].parse::<u32>().unwrap() >= 5));
        assert_ne!(artists(&fam), artists(&deep));
    }

    #[test]
    fn no_repeated_artists_and_no_duplicates_across_batches() {
        let mut s = Session::default();
        let b1 = batch(Strength::Balanced, Prefs::default(), &mut s, &All);
        assert_eq!(artists(&b1).len(), b1.len(), "one track per artist when repeats are off");
        let b2 = batch(Strength::Balanced, Prefs::default(), &mut s, &All);
        let ids1: HashSet<String> = b1.iter().map(track_ident).collect();
        assert!(b2.iter().all(|t| !ids1.contains(&track_ident(t))), "no duplicates within a session");
        // allowing repeats permits several tracks per artist
        let c = gather(&seed(), Strength::Familiar, &fake()).unwrap();
        let mut rng = rand::rng();
        let count = |v: &[Cand]| v.iter().filter(|x| x.artist == "A0").count();
        let on = rank(c.clone(), &seed(), &Prefs { allow_repeat_artists: true, ..Prefs::default() }, &Session::default(), &|_| false, &|_| false, &mut rng);
        let off = rank(c, &seed(), &Prefs::default(), &Session::default(), &|_| false, &|_| false, &mut rng);
        assert_eq!(count(&on), 3, "up to 3 per artist when repeats are allowed");
        assert_eq!(count(&off), 1);
    }

    #[test]
    fn unplayable_candidates_are_dropped_not_faked() {
        let b = batch(Strength::Familiar, Prefs::default(), &mut Session::default(), &OnlyEven);
        assert!(!b.is_empty());
        assert!(b.iter().all(|t| t.title[1..].parse::<u32>().unwrap() % 2 == 0));
        struct Never;
        impl Resolver for Never {
            fn resolve(&self, _: &Cand) -> Option<Track> {
                None
            }
        }
        assert!(batch(Strength::Balanced, Prefs::default(), &mut Session::default(), &Never).is_empty());
    }

    #[test]
    fn history_skips_and_likes_shape_results() {
        let c = gather(&seed(), Strength::Familiar, &fake()).unwrap();
        let mut rng = rand::rng();
        // recently played tracks are excluded
        let recent = ident("A0", "S0");
        let r = rank(c.clone(), &seed(), &Prefs::default(), &Session::default(), &|_| false, &|id| id == recent, &mut rng);
        assert!(r.iter().all(|x| ident(&x.artist, &x.title) != recent));
        // an artist skipped three times disappears (one skip only down-weights)
        let mut s = Session::default();
        s.skips.insert("a0".into(), 3);
        let r = rank(c.clone(), &seed(), &Prefs::default(), &s, &|_| false, &|_| false, &mut rng);
        assert!(r.iter().all(|x| x.artist != "A0"));
        // learning off ignores skips
        let r = rank(c.clone(), &seed(), &Prefs { learn: false, ..Prefs::default() }, &s, &|_| false, &|_| false, &mut rng);
        assert!(r.iter().any(|x| x.artist == "A0"));
        // ...but the seed's own artist is never dropped, only down-weighted
        let mut s2 = Session::default();
        s2.skips.insert("seed".into(), 5);
        let r = rank(vec![Cand { artist: "Seed".into(), title: "Other".into(), score: 1.0, listeners: None, track: None }], &seed(), &Prefs::default(), &s2, &|_| false, &|_| false, &mut rng);
        assert_eq!(r.len(), 1);
        assert!(r[0].score < 0.5);
        // the seed itself is never recommended
        let r = rank(vec![Cand { artist: "Seed".into(), title: "Song".into(), score: 9.0, listeners: None, track: None }], &seed(), &Prefs::default(), &Session::default(), &|_| false, &|_| false, &mut rng);
        assert!(r.is_empty());
    }

    #[test]
    fn self_test_harness_passes_on_a_well_formed_graph() {
        let t = self_test_with(&fake(), "Seed", "Song", &[]);
        assert!(t.ok(), "{}", t.report);
        assert!(t.report.contains("Deep Discovery"));
        // and fails clearly when the source returns nothing
        struct Empty;
        impl Source for Empty {
            fn similar_tracks(&self, _: &str, _: &str, _: u32) -> anyhow::Result<Vec<Similar>> {
                Ok(vec![])
            }
            fn similar_artists(&self, _: &str, _: u32) -> anyhow::Result<Vec<(String, f32)>> {
                Ok(vec![])
            }
            fn top_tracks(&self, _: &str, _: u32) -> anyhow::Result<Vec<Similar>> {
                Ok(vec![])
            }
        }
        let e = self_test_with(&Empty, "x", "y", &[]);
        assert!(!e.ok());
        assert!(self_test("", "a", "b", &[]).report.contains("No Last.fm API key"));
    }

    #[test]
    fn resolver_uses_budget_and_caches_misses() {
        // no Spotify client: nothing is searched, nothing cached, budget untouched
        let r = StreamResolver { local: HashMap::new(), spotify: None, provider: RadioProvider::Automatic, spotify_playable: false, cache: Default::default(), budget: std::cell::Cell::new(2) };
        assert!(r.resolve(&Cand { artist: "a".into(), title: "b".into(), score: 1.0, listeners: None, track: None }).is_none());
        assert_eq!(r.budget.get(), 2);
        assert!(r.cache.lock().unwrap().is_empty());
        assert!(r.take_budget() && r.take_budget() && !r.take_budget());
        let mut c = ResolveCache::default();
        for i in 0..RESOLVE_CACHE_MAX + 50 {
            c.put(format!("k{i}"), None);
        }
        assert_eq!(c.len(), RESOLVE_CACHE_MAX, "bounded");
        assert!(c.get("k0").is_none() && c.get(&format!("k{}", RESOLVE_CACHE_MAX + 49)).is_some());
    }

    #[test]
    fn fallback_uses_same_artist_and_album_from_library() {
        let lib = vec![
            Track { title: "x1".into(), artists: vec!["Seed".into()], album: "AL".into(), ..Default::default() },
            Track { title: "x2".into(), artists: vec!["Other".into()], album: "AL".into(), ..Default::default() },
            Track { title: "x3".into(), artists: vec!["Nope".into()], album: "ZZ".into(), ..Default::default() },
        ];
        let mut s = seed();
        s.album = "AL".into();
        let c = fallback_candidates(&s, &lib, None);
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|c| c.track.is_some()));
    }
}

// ---------------------------------------------------------------------------
// Live self-test against the real Last.fm API (run on a PC with network access)

#[derive(Debug, Clone)]
pub struct SelfTest {
    pub report: String,
    pub passed: usize,
    pub failed: usize,
}

impl SelfTest {
    pub fn ok(&self) -> bool {
        self.failed == 0 && self.passed > 0
    }
    pub fn ok_or_empty(&self) -> bool {
        self.failed == 0
    }
}

/// Query Last.fm for real, run every discovery mode on the result and check the
/// response shape and mode behaviour. `local` (optional) is used to report how many
/// recommendations would be playable from the local library.
pub fn self_test(key: &str, artist: &str, title: &str, local: &[Track]) -> SelfTest {
    if key.trim().is_empty() {
        return SelfTest {
            report: format!("Aqua Glass {} — Similarity Radio live self-test\nNo Last.fm API key. Set AQUA_LASTFM_KEY or save a key in Settings › Playback › Similarity Radio.\n", env!("CARGO_PKG_VERSION")),
            passed: 0,
            failed: 1,
        };
    }
    self_test_with(&LastFm::new(key), artist, title, local)
}

/// The self-test against any similarity source (Last.fm in production).
pub fn self_test_with(lf: &dyn Source, artist: &str, title: &str, local: &[Track]) -> SelfTest {
    use std::fmt::Write;
    let mut r = String::new();
    let (mut passed, mut failed) = (0usize, 0usize);
    let mut check = |r: &mut String, ok: bool, what: &str| {
        let _ = writeln!(r, "  [{}] {what}", if ok { "PASS" } else { "FAIL" });
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }
    };
    let _ = writeln!(r, "Aqua Glass {} — Similarity Radio live self-test", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(r, "Date (unix): {}", aqua_core::storage::now_unix());
    let _ = writeln!(r, "Seed: {artist} — {title}\n");
    let seed = Track { title: title.into(), artists: vec![artist.into()], ..Default::default() };

    let _ = writeln!(r, "1. Raw API responses");
    let t0 = Instant::now();
    let st = lf.similar_tracks(artist, title, 100);
    let _ = writeln!(r, "  track.getSimilar: {:?} in {} ms", st.as_ref().map(|v| v.len()).map_err(|e| e.to_string()), t0.elapsed().as_millis());
    let t1 = Instant::now();
    let sa = lf.similar_artists(artist, 60);
    let _ = writeln!(r, "  artist.getSimilar: {:?} in {} ms", sa.as_ref().map(|v| v.len()).map_err(|e| e.to_string()), t1.elapsed().as_millis());
    let st = match st {
        Ok(v) => v,
        Err(e) => {
            let _ = writeln!(r, "\nLast.fm request failed: {e}");
            check(&mut r, false, "Last.fm reachable and key accepted");
            return SelfTest { report: r, passed, failed };
        }
    };
    let sa = sa.unwrap_or_default();
    check(&mut r, true, "Last.fm reachable and key accepted");
    check(&mut r, !st.is_empty(), "track.getSimilar returned similar tracks for the seed");
    check(&mut r, st.iter().all(|s| (0.0..=1.0).contains(&s.score)), "similarity scores are within 0..1");
    check(&mut r, st.windows(2).all(|w| w[0].score + 1e-6 >= w[1].score), "similar tracks are ordered by similarity");
    check(&mut r, !sa.is_empty(), "artist.getSimilar returned related artists");
    if let Some((a, _)) = sa.first() {
        let tt = lf.top_tracks(a, 10);
        let _ = writeln!(r, "  artist.getTopTracks({a}): {:?}", tt.as_ref().map(|v| v.len()).map_err(|e| e.to_string()));
        check(&mut r, tt.as_ref().map(|v| !v.is_empty()).unwrap_or(false), "artist.getTopTracks returned tracks");
        if let Ok(v) = &tt {
            check(&mut r, v.iter().any(|x| x.listeners.is_some()), "top tracks include listener counts (used by Deep Discovery)");
        }
    }
    let _ = writeln!(r, "  first similar tracks: {}", st.iter().take(5).map(|s| format!("{} — {} ({:.2})", s.artist, s.title, s.score)).collect::<Vec<_>>().join("; "));

    let _ = writeln!(r, "\n2. Discovery modes (ranking with default rules, empty history)");
    let index: HashMap<String, Track> = local.iter().map(|t| (track_ident(t), t.clone())).collect();
    let mut artist_sets: Vec<(Strength, HashSet<String>)> = Vec::new();
    let mut rng = rand::rng();
    for strength in Strength::ALL {
        let t = Instant::now();
        match gather(&seed, strength, lf) {
            Ok(c) => {
                let n = c.len();
                let ranked = rank(c, &seed, &Prefs { strength, ..Prefs::default() }, &Session::default(), &|_| false, &|_| false, &mut rng);
                let top: Vec<&Cand> = ranked.iter().take(15).collect();
                let artists: HashSet<String> = top.iter().map(|c| c.artist.to_lowercase()).collect();
                let playable = top.iter().filter(|c| index.contains_key(&ident(&c.artist, &c.title))).count();
                let _ = writeln!(r, "  {:<15} {n:>4} candidates, top 15 from {} artists, {playable} playable from local library, {} ms", strength.label(), artists.len(), t.elapsed().as_millis());
                for c in top.iter().take(15) {
                    let _ = writeln!(r, "      {:.2}  {} — {}{}", c.score, c.artist, c.title, c.listeners.map(|l| format!("  ({l} listeners)")).unwrap_or_default());
                }
                check(&mut r, top.len() >= 10, &format!("{}: at least 10 recommendations", strength.label()));
                check(&mut r, top.iter().all(|c| ident(&c.artist, &c.title) != track_ident(&seed)), &format!("{}: seed not recommended", strength.label()));
                if strength == Strength::DeepDiscovery {
                    check(&mut r, top.iter().all(|c| !c.artist.eq_ignore_ascii_case(artist)), "Deep Discovery: excludes the seed artist");
                }
                artist_sets.push((strength, artists));
            }
            Err(e) => {
                let _ = writeln!(r, "  {:<15} error: {e}", strength.label());
                check(&mut r, false, &format!("{}: gather succeeded", strength.label()));
            }
        }
    }
    let _ = writeln!(r, "\n3. Mode differences");
    let get = |s: Strength| artist_sets.iter().find(|x| x.0 == s).map(|x| x.1.clone()).unwrap_or_default();
    let (fam, deep, adv) = (get(Strength::Familiar), get(Strength::DeepDiscovery), get(Strength::Adventurous));
    let jac = |a: &HashSet<String>, b: &HashSet<String>| if a.is_empty() && b.is_empty() { 1.0 } else { a.intersection(b).count() as f32 / a.union(b).count().max(1) as f32 };
    let _ = writeln!(r, "  artist overlap Familiar↔Adventurous {:.2}, Familiar↔Deep {:.2}", jac(&fam, &adv), jac(&fam, &deep));
    check(&mut r, jac(&fam, &deep) < 0.5, "Familiar and Deep Discovery recommend mostly different artists");
    check(&mut r, adv.len() >= fam.len().min(10) / 2, "Adventurous draws from a varied set of artists");
    let _ = writeln!(r, "\nResult: {passed} passed, {failed} failed");
    SelfTest { report: r, passed, failed }
}
