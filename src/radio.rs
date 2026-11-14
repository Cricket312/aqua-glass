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

use crate::audio::engine::{Cmd, Engine, PlayRecord, SpotifyStatus};
use crate::config::ProviderKind;
use crate::history::{ident, track_ident, History};
use crate::library::{LibraryStore, LOCAL_ALL_ID};
use crate::matching::{self, MatchStatus};
use crate::model::Track;
use crate::services::lastfm::{LastFm, Similar};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Strength {
    Familiar,
    #[default]
    Balanced,
    Adventurous,
    DeepDiscovery,
}

impl Strength {
    pub const ALL: [Strength; 4] = [Strength::Familiar, Strength::Balanced, Strength::Adventurous, Strength::DeepDiscovery];
    pub fn label(self) -> &'static str {
        match self {
            Strength::Familiar => "Familiar",
            Strength::Balanced => "Balanced",
            Strength::Adventurous => "Adventurous",
            Strength::DeepDiscovery => "Deep Discovery",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            Strength::Familiar => "Very similar music — the closest matches to the seed track.",
            Strength::Balanced => "Similar music with some new artists from closely related acts.",
            Strength::Adventurous => "More variety from a wider circle of related artists, same general style.",
            Strength::DeepDiscovery => "Favors lesser-known tracks and artists you haven't heard.",
        }
    }
    fn jitter(self) -> f32 {
        match self {
            Strength::Familiar => 0.05,
            Strength::Balanced => 0.15,
            Strength::Adventurous => 0.25,
            Strength::DeepDiscovery => 0.25,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RadioProvider {
    #[default]
    Automatic,
    Spotify,
    Local,
    YouTube,
}

impl RadioProvider {
    pub fn label(self) -> &'static str {
        match self {
            RadioProvider::Automatic => "Automatic",
            RadioProvider::Spotify => "Spotify",
            RadioProvider::Local => "Local files",
            RadioProvider::YouTube => "YouTube Music (opens in browser)",
        }
    }
}

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
    pub spotify: Option<Arc<crate::services::spotify::SpotifyApi>>,
    pub youtube: Option<Arc<crate::services::youtube::YouTubeApi>>,
    pub provider: RadioProvider,
    pub spotify_playable: bool,
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
        if use_spotify {
            if let Some(sp) = &self.spotify {
                let q = format!("track:\"{}\" artist:\"{}\"", c.title.replace('"', ""), c.artist.replace('"', ""));
                if let Ok(r) = sp.search_tracks(&q, 5) {
                    if let Some(t) = accept(&src, r) {
                        return Some(t);
                    }
                }
            }
        }
        if self.provider == RadioProvider::YouTube {
            if let Some(yt) = &self.youtube {
                if let Ok(r) = yt.search_tracks(&format!("{} {}", c.artist, c.title), 3) {
                    return accept(&src, r);
                }
            }
        }
        None
    }
}

/// Fallback without Last.fm: tracks by the same artist / from the same album
/// in the local library and (if available) Spotify. Clearly labelled in the UI.
pub fn fallback_candidates(seed: &Track, local: &[Track], spotify: Option<&crate::services::spotify::SpotifyApi>) -> Vec<Cand> {
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
    pub engine: Engine,
    pub spotify: Arc<crate::services::spotify::SpotifyApi>,
    pub youtube: Arc<crate::services::youtube::YouTubeApi>,
    pub library: Arc<Mutex<LibraryStore>>,
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
                let cur_radio = s.queue_pos.and_then(|p| s.queue_radio.get(p)).copied().unwrap_or(false);
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
}

fn worker(rx: mpsc::Receiver<Msg>, tx: mpsc::Sender<Msg>, status: Arc<Mutex<Status>>, deps: Deps, prefs: Prefs) {
    let mut w = W { deps, prefs, status, tx, session: Session::default(), run: 0, last_fail: None, stalled: false, last_seq: 0, upcoming: 0, current: None };
    while let Ok(m) = rx.recv() {
        match m {
            Msg::Shutdown => break,
            Msg::Prefs(p) => w.prefs = p,
            Msg::Start(seed) => w.start(seed),
            Msg::Stop => {
                w.run += 1;
                *w.status.lock().unwrap() = Status::default();
                w.deps.engine.send(Cmd::RadioClearUpcoming);
            }
            Msg::Refresh => {
                if w.status.lock().unwrap().active {
                    w.deps.engine.send(Cmd::RadioClearUpcoming);
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
                    w.deps.engine.send(Cmd::RadioClearUpcoming);
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
                        w.deps.engine.send(Cmd::RadioAppend(t));
                    }
                    Ok(_) => {
                        s.error = Some("No playable recommendations found for this seed with the current settings.".into());
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
        let youtube = self.deps.youtube.clone();
        let library = self.deps.library.clone();
        let history = self.deps.history.clone();
        let spotify_playable = self.deps.engine.state.read().unwrap().spotify == SpotifyStatus::Ready && spotify.is_signed_in();
        // the fetch thread works on a copy of the session that is merged back afterwards
        std::thread::Builder::new()
            .name("radio-fetch".into())
            .spawn(move || {
                let local: Vec<Track> = library.lock().unwrap().tracks(ProviderKind::Local, LOCAL_ALL_ID).cloned().unwrap_or_default();
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
                    let week = crate::util::now_unix() - 7 * 86400;
                    let mut rng = rand::rng();
                    let ranked = rank(c, &seed, &prefs, &session, &|id| h.heard(id), &|id| h.heard_since(id, week), &mut rng);
                    drop(h);
                    let resolver = StreamResolver { local: index, spotify: Some(spotify.clone()), youtube: Some(youtube.clone()), provider: prefs.provider, spotify_playable };
                    if !spotify_playable && resolver.local.is_empty() && prefs.provider != RadioProvider::YouTube {
                        return Err("No playable source: sign in to Spotify (Premium) or add local music.".to_string());
                    }
                    Ok(build_batch(ranked, want, &resolver, &mut session, want * 4))
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
            (0..60).map(|i| sim(&format!("A{}", i % 20), &format!("S{i}"), 1.0 - i as f32 / 60.0, i, 100_000)).chain(std::iter::once(sim("Seed", "Other", 0.95, 60, 1000))).collect(),
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
