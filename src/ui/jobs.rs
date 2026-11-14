//! Background jobs (network/disk work) and the messages they send back to the UI.
//! Every job runs on its own short-lived thread; the UI thread never blocks.

use crate::config::ProviderKind;
use crate::library::{self, LibraryStore, PlaylistEntry, SyncReport, LIKED_ID};
use crate::model::*;
use crate::services::{lyrics::Lyrics, spotify, youtube, ApiError};
use crate::transfer::{Plan, TransferLog};
use eframe::egui;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

pub enum Msg {
    SpotifyAuth(Result<Option<spotify::Profile>, String>),
    YouTubeAuth(Result<Option<youtube::Channel>, String>),
    Search { gen: u64, provider: ProviderKind, res: Result<SearchResults, String> },
    PlaylistTracks { provider: ProviderKind, id: String, res: Result<Vec<Track>, String> },
    AlbumTracks { album: Album, res: Result<Vec<Track>, String> },
    ArtistAlbums { artist: Artist, res: Result<Vec<Album>, String> },
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
                let msg = e.to_string();
                if let Some(p) = s.lib(ProviderKind::Spotify).playlist_mut(id) {
                    p.meta.items_error = msg.clone();
                    // force a retry next sync
                    p.meta.snapshot.clear();
                }
                rep.errors.push(format!("{id}: {msg}"));
            }
        }
    }
    if import_liked {
        h.progress(refetch.len(), total);
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
            }
            Err(e) => rep.errors.push(format!("Liked Songs: {e}")),
        }
    }
    if import_albums {
        h.progress(refetch.len() + 1, total);
        match api.saved_albums() {
            Ok(a) => store.lock().unwrap().lib(ProviderKind::Spotify).albums = a,
            Err(e) => rep.errors.push(format!("Saved albums: {e}")),
        }
    }
    if import_artists {
        h.progress(refetch.len() + 2, total);
        match api.followed_artists() {
            Ok(a) => store.lock().unwrap().lib(ProviderKind::Spotify).artists = a,
            Err(e) => rep.errors.push(format!("Followed artists: {e}")),
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

pub fn sync_youtube(api: &youtube::YouTubeApi, store: &Arc<Mutex<LibraryStore>>, import_all: bool, h: &JobHandle) -> Result<SyncReport, String> {
    let ch = api.channel().map_err(e2s)?;
    let lists = api.playlists().map_err(e2s)?;
    let import_liked = {
        let mut s = store.lock().unwrap();
        let l = s.lib(ProviderKind::YouTube);
        l.account = ch.title.clone();
        l.user_id = ch.id.clone();
        l.import_liked
    };
    let (mut rep, refetch) = {
        let mut s = store.lock().unwrap();
        let r = library::merge_listing(s.lib(ProviderKind::YouTube), lists, import_all);
        s.save_index(ProviderKind::YouTube);
        r
    };
    for (i, id) in refetch.iter().enumerate() {
        h.progress(i, refetch.len() + 1);
        let res = api.playlist_items(id);
        let mut s = store.lock().unwrap();
        match res {
            Ok(t) => {
                s.store_tracks(ProviderKind::YouTube, id, t);
                if let Some(e) = s.lib(ProviderKind::YouTube).playlist_mut(id) {
                    e.last_synced = crate::util::now_unix();
                }
            }
            Err(e) => {
                if let Some(p) = s.lib(ProviderKind::YouTube).playlist_mut(id) {
                    p.meta.items_error = e.to_string();
                    p.meta.snapshot.clear();
                }
                rep.errors.push(format!("{id}: {e}"));
            }
        }
    }
    if import_liked {
        match api.liked() {
            Ok(t) => {
                let mut s = store.lock().unwrap();
                let n = t.len() as u32;
                let l = s.lib(ProviderKind::YouTube);
                if l.playlist(LIKED_ID).is_none() {
                    l.playlists.insert(0, PlaylistEntry { meta: Playlist { provider: ProviderKind::YouTube, id: LIKED_ID.into(), name: "Liked Music".into(), is_liked: true, owned_by_user: true, ..Default::default() }, imported: true, ..Default::default() });
                }
                if let Some(e) = l.playlist_mut(LIKED_ID) {
                    e.meta.track_count = n;
                }
                s.store_tracks(ProviderKind::YouTube, LIKED_ID, t);
            }
            Err(e) => rep.errors.push(format!("Liked music: {e}")),
        }
    }
    let mut s = store.lock().unwrap();
    let l = s.lib(ProviderKind::YouTube);
    l.last_sync = crate::util::now_unix();
    l.last_error = rep.errors.first().cloned().unwrap_or_default();
    s.save_index(ProviderKind::YouTube);
    Ok(rep)
}
