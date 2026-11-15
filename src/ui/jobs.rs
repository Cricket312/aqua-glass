//! Background jobs (network/disk work) and the messages they send back to the UI.
//! Every job runs on its own short-lived thread; the UI thread never blocks.

use crate::config::ProviderKind;
use crate::library::{self, LibraryStore, PlaylistEntry, SyncReport, LIKED_ID};
use crate::model::*;
use crate::services::{lyrics::Lyrics, spotify, ApiError};
use crate::transfer::{Plan, TransferLog};
use eframe::egui;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

pub enum Msg {
    SpotifyAuth(Result<Option<spotify::Profile>, String>),
    Search { gen: u64, provider: ProviderKind, res: Result<SearchResults, String> },
    PlaylistTracks { provider: ProviderKind, id: String, res: Result<Vec<Track>, String> },
    AlbumTracks { album: Album, res: Result<Vec<Track>, String> },
    ArtistAlbums { artist: Artist, res: Result<Vec<Album>, String> },
    /// Artist looked up by name (artist link on a track without artist ids).
    ArtistFound { name: String, res: Result<Option<Artist>, String> },
    SyncDone { provider: ProviderKind, res: Result<SyncReport, String> },
    Lyrics { key: String, res: Result<Lyrics, String> },
    LocalScanned(Vec<Track>),
    PlanReady(Plan, Vec<Track>),
    TransferDone(Result<TransferLog, String>),
    Alternatives { src: Track, res: Result<Vec<crate::matching::Candidate>, String> },
    Toast(String, bool),
}

#[derive(Clone)]
pub struct JobInfo {
    pub label: String,
    pub started: Instant,
    pub progress: Option<(usize, usize)>,
}

#[derive(Clone)]
pub struct Jobs {
    pub tx: mpsc::Sender<Msg>,
    pub active: Arc<Mutex<BTreeMap<u64, JobInfo>>>,
    next: Arc<std::sync::atomic::AtomicU64>,
    ctx: egui::Context,
    pub cancel: Arc<AtomicBool>,
}

pub struct JobHandle {
    id: u64,
    jobs: Jobs,
}

impl JobHandle {
    pub fn progress(&self, done: usize, total: usize) {
        if let Some(j) = self.jobs.active.lock().unwrap().get_mut(&self.id) {
            j.progress = Some((done, total));
        }
        self.jobs.ctx.request_repaint();
    }
    pub fn send(&self, m: Msg) {
        let _ = self.jobs.tx.send(m);
        self.jobs.ctx.request_repaint();
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        self.jobs.active.lock().unwrap().remove(&self.id);
        self.jobs.ctx.request_repaint();
    }
}

impl Jobs {
    pub fn new(ctx: egui::Context) -> (Jobs, mpsc::Receiver<Msg>) {
        let (tx, rx) = mpsc::channel();
        (Jobs { tx, active: Default::default(), next: Default::default(), ctx, cancel: Arc::new(AtomicBool::new(false)) }, rx)
    }

    pub fn spawn(&self, label: impl Into<String>, f: impl FnOnce(JobHandle) + Send + 'static) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.active.lock().unwrap().insert(id, JobInfo { label: label.into(), started: Instant::now(), progress: None });
        let h = JobHandle { id, jobs: self.clone() };
        std::thread::Builder::new().name("job".into()).spawn(move || f(h)).ok();
    }

    pub fn is_running(&self, label_prefix: &str) -> bool {
        self.active.lock().unwrap().values().any(|j| j.label.starts_with(label_prefix))
    }
}

fn e2s(e: ApiError) -> String {
    e.to_string()
}

/// Full Spotify synchronization (read-only towards Spotify).
pub fn sync_spotify(api: &spotify::SpotifyApi, store: &Arc<Mutex<LibraryStore>>, include_followed: bool, import_all: bool, h: &JobHandle) -> Result<SyncReport, String> {
    let _bg = crate::services::ratelimit::background();
    let me = api.me().map_err(e2s)?;
    let mut lists = api.playlists(&me.id).map_err(e2s)?;
    if !include_followed {
        lists.retain(|p| p.owned_by_user || p.collaborative);
    }
    let (import_liked, import_albums, import_artists) = {
        let mut s = store.lock().unwrap();
        let l = s.lib(ProviderKind::Spotify);
        l.account = me.display_name.clone();
        l.user_id = me.id.clone();
        (l.import_liked, l.import_albums, l.import_artists)
    };
    let (mut rep, refetch) = {
        let mut s = store.lock().unwrap();
        let l = s.lib(ProviderKind::Spotify);
        let r = library::merge_listing(l, lists, import_all);
        s.save_index(ProviderKind::Spotify);
        r
    };
    let total = refetch.len() + 3;
    for (i, id) in refetch.iter().enumerate() {
        h.progress(i, total);
        let res = api.playlist_items(id);
        let mut s = store.lock().unwrap();
        match res {
            Ok(tracks) => {
                let before = s.tracks(ProviderKind::Spotify, id).map(|t| t.len()).unwrap_or(0);
                let name = s.lib(ProviderKind::Spotify).playlist(id).map(|p| p.meta.name.clone()).unwrap_or_default();
                if library::detect_shrink(s.lib(ProviderKind::Spotify), id, &name, before, tracks.len()) {
                    s.backup_tracks(ProviderKind::Spotify, id);
                }
                s.store_tracks(ProviderKind::Spotify, id, tracks);
                if let Some(e) = s.lib(ProviderKind::Spotify).playlist_mut(id) {
                    e.last_synced = crate::util::now_unix();
                    e.meta.items_error.clear();
                }
            }
            Err(e) => {
                let limited = matches!(e, ApiError::RateLimited(_));
                let msg = e.to_string();
                if let Some(p) = s.lib(ProviderKind::Spotify).playlist_mut(id) {
                    p.meta.items_error = msg.clone();
                    // force a retry next sync
                    p.meta.snapshot.clear();
                }
                rep.errors.push(format!("{id}: {msg}"));
                if limited {
                    // make sure the playlists not reached yet are refetched by the next sync
                    for rid in &refetch[i + 1..] {
                        if let Some(p) = s.lib(ProviderKind::Spotify).playlist_mut(rid) {
                            p.meta.snapshot.clear();
                        }
                    }
                    rep.errors.push("Sync paused: Spotify asked Aqua Glass to slow down. Remaining playlists will update on the next sync.".into());
                    break;
                }
            }
        }
    }
    // saved collections: one cheap request each; full download only when changed
    let unchanged = |which: &str, sig: &Result<(u64, String), ApiError>, has_cache: bool| -> bool {
        let prev = store.lock().unwrap().lib(ProviderKind::Spotify).signatures.get(which).cloned();
        has_cache && matches!((sig, prev), (Ok(a), Some(b)) if *a == b)
    };
    let remember = |which: &str, sig: Result<(u64, String), ApiError>| {
        if let Ok(sg) = sig {
            store.lock().unwrap().lib(ProviderKind::Spotify).signatures.insert(which.to_string(), sg);
        }
    };
    if import_liked {
        h.progress(refetch.len(), total);
        let sig = api.collection_signature("tracks");
        let has = store.lock().unwrap().tracks(ProviderKind::Spotify, LIKED_ID).is_some();
        if unchanged("tracks", &sig, has) {
            rep.unchanged += 1;
        } else {
            match api.liked_tracks() {
                Ok(t) => {
                    let mut s = store.lock().unwrap();
                    let n = t.len() as u32;
                    let l = s.lib(ProviderKind::Spotify);
                    if l.playlist(LIKED_ID).is_none() {
                        l.playlists.insert(0, PlaylistEntry { meta: Playlist { provider: ProviderKind::Spotify, id: LIKED_ID.into(), name: "Liked Songs".into(), is_liked: true, owned_by_user: true, ..Default::default() }, imported: true, ..Default::default() });
                    }
                    if let Some(e) = l.playlist_mut(LIKED_ID) {
                        e.meta.track_count = n;
                        e.last_synced = crate::util::now_unix();
                    }
                    s.store_tracks(ProviderKind::Spotify, LIKED_ID, t);
                    drop(s);
                    remember("tracks", sig);
                }
                Err(e) => rep.errors.push(format!("Liked Songs: {e}")),
            }
        }
    }
    if import_albums {
        h.progress(refetch.len() + 1, total);
        let sig = api.collection_signature("albums");
        let has = !store.lock().unwrap().lib(ProviderKind::Spotify).albums.is_empty();
        if !unchanged("albums", &sig, has) {
            match api.saved_albums() {
                Ok(a) => {
                    store.lock().unwrap().lib(ProviderKind::Spotify).albums = a;
                    remember("albums", sig);
                }
                Err(e) => rep.errors.push(format!("Saved albums: {e}")),
            }
        }
    }
    if import_artists {
        h.progress(refetch.len() + 2, total);
        let sig = api.collection_signature("artists");
        let has = !store.lock().unwrap().lib(ProviderKind::Spotify).artists.is_empty();
        if !unchanged("artists", &sig, has) {
            match api.followed_artists() {
                Ok(a) => {
                    store.lock().unwrap().lib(ProviderKind::Spotify).artists = a;
                    remember("artists", sig);
                }
                Err(e) => rep.errors.push(format!("Followed artists: {e}")),
            }
        }
    }
    if let Ok(r) = api.recently_played() {
        store.lock().unwrap().lib(ProviderKind::Spotify).recent = r;
    }
    let mut s = store.lock().unwrap();
    let l = s.lib(ProviderKind::Spotify);
    l.last_sync = crate::util::now_unix();
    l.last_error = rep.errors.first().cloned().unwrap_or_default();
    s.save_index(ProviderKind::Spotify);
    Ok(rep)
}

