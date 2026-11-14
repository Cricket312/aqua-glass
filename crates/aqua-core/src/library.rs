//! Local library metadata store: imported (cached) playlists, saved albums,
//! followed artists, liked songs and local files, plus synchronization.
//!
//! Distinctions (see README):
//!  1. *Accessing* a playlist directly — fetched live, not stored.
//!  2. *Importing* (caching) — header + items stored on disk under
//!     `<data>/library/<provider>/`, refreshed by sync. Nothing is ever
//!     written back to the service by import or sync.
//!  3. *Exporting* — writing JSON/CSV/M3U files (see `transfer`).
//!  4. *Copying to another provider* — Playlist Transfer.

use crate::model::ProviderKind;
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

pub const LIKED_ID: &str = "__liked__";
pub const LOCAL_ALL_ID: &str = "__local_all__";

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct PlaylistEntry {
    pub meta: Playlist,
    /// Imported = cached locally and kept in sync.
    pub imported: bool,
    /// Disappeared upstream (deleted/unfollowed). Kept until the user decides.
    pub removed_upstream: bool,
    pub last_synced: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ConflictKind {
    RemovedUpstream,
    LargeShrink { before: u32, after: u32 },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Conflict {
    pub playlist_id: String,
    pub name: String,
    pub kind: ConflictKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderLib {
    pub account: String,
    pub user_id: String,
    pub playlists: Vec<PlaylistEntry>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub recent: Vec<Track>,
    pub last_sync: i64,
    pub last_error: String,
    pub conflicts: Vec<Conflict>,
    pub import_liked: bool,
    pub import_albums: bool,
    pub import_artists: bool,
    /// Change signatures (total, newest id) of saved collections from the last sync.
    pub signatures: BTreeMap<String, (u64, String)>,
    #[serde(skip)]
    pub loaded: HashMap<String, Vec<Track>>,
}

impl ProviderLib {
    pub fn playlist(&self, id: &str) -> Option<&PlaylistEntry> {
        self.playlists.iter().find(|p| p.meta.id == id)
    }
    pub fn playlist_mut(&mut self, id: &str) -> Option<&mut PlaylistEntry> {
        self.playlists.iter_mut().find(|p| p.meta.id == id)
    }
}

#[derive(Default)]
pub struct LibraryStore {
    pub root: PathBuf,
    pub libs: BTreeMap<ProviderKind, ProviderLib>,
    /// When false (metadata cache off), only the playlist currently shown keeps its tracks in RAM.
    pub keep_in_memory: bool,
}

fn safe(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn pdir(p: ProviderKind) -> &'static str {
    match p {
        ProviderKind::Spotify => "spotify",
        ProviderKind::YouTube => "youtube",
        ProviderKind::Local => "local",
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub unchanged: usize,
    pub removed_upstream: Vec<String>,
    pub errors: Vec<String>,
}

/// Merge a fresh playlist listing into the stored one. Returns the report and the ids
/// of imported playlists whose items must be re-fetched (snapshot changed).
pub fn merge_listing(lib: &mut ProviderLib, fresh: Vec<Playlist>, import_new: bool) -> (SyncReport, Vec<String>) {
    let mut rep = SyncReport::default();
    let mut refetch = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for p in fresh {
        seen.insert(p.id.clone());
        match lib.playlist_mut(&p.id) {
            Some(e) => {
                e.removed_upstream = false;
                if e.meta.snapshot != p.snapshot || p.snapshot.is_empty() {
                    if e.imported {
                        refetch.push(p.id.clone());
                    }
                    rep.changed.push(p.name.clone());
                } else {
                    rep.unchanged += 1;
                }
                let err = std::mem::take(&mut e.meta.items_error);
                e.meta = p;
                if !refetch.contains(&e.meta.id) {
                    e.meta.items_error = err;
                }
            }
            None => {
                rep.added.push(p.name.clone());
                if import_new {
                    refetch.push(p.id.clone());
                }
                lib.playlists.push(PlaylistEntry { meta: p, imported: import_new, removed_upstream: false, last_synced: 0 });
            }
        }
    }
    lib.conflicts.retain(|c| !matches!(c.kind, ConflictKind::RemovedUpstream));
    for e in lib.playlists.iter_mut() {
        if e.meta.is_liked || seen.contains(&e.meta.id) {
            continue;
        }
        if !e.removed_upstream {
            rep.removed_upstream.push(e.meta.name.clone());
        }
        e.removed_upstream = true;
        if e.imported {
            lib.conflicts.push(Conflict { playlist_id: e.meta.id.clone(), name: e.meta.name.clone(), kind: ConflictKind::RemovedUpstream });
        }
    }
    // never silently drop non-imported, removed playlists from view either: they are just headers
    lib.playlists.retain(|e| e.imported || !e.removed_upstream);
    (rep, refetch)
}

/// Record new items for a playlist, flagging a suspicious shrink as a conflict
/// (the previous cached copy is preserved as a backup file).
pub fn detect_shrink(lib: &mut ProviderLib, id: &str, name: &str, before: usize, after: usize) -> bool {
    if before >= 20 && after * 2 < before {
        lib.conflicts.retain(|c| c.playlist_id != id || !matches!(c.kind, ConflictKind::LargeShrink { .. }));
        lib.conflicts.push(Conflict {
            playlist_id: id.into(),
            name: name.into(),
            kind: ConflictKind::LargeShrink { before: before as u32, after: after as u32 },
        });
        return true;
    }
    false
}

impl LibraryStore {
    pub fn open(root: PathBuf) -> LibraryStore {
        let mut s = LibraryStore { root, libs: BTreeMap::new(), keep_in_memory: true };
        for p in [ProviderKind::Spotify, ProviderKind::YouTube, ProviderKind::Local] {
            let path = s.root.join(pdir(p)).join("index.json");
            let lib = std::fs::read(&path)
                .ok()
                .and_then(|b| match serde_json::from_slice::<ProviderLib>(&b) {
                    Ok(l) => Some(l),
                    Err(e) => {
                        log::warn!("corrupt library index {}: {e}", path.display());
                        let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                        None
                    }
                })
                .unwrap_or_default();
            s.libs.insert(p, lib);
        }
        s
    }

    pub fn lib(&mut self, p: ProviderKind) -> &mut ProviderLib {
        self.libs.entry(p).or_default()
    }

    pub fn save_index(&self, p: ProviderKind) {
        if let Some(l) = self.libs.get(&p) {
            if let Ok(b) = serde_json::to_vec(l) {
                let _ = crate::storage::atomic_write(&self.root.join(pdir(p)).join("index.json"), &b);
            }
        }
    }

    fn tracks_path(&self, p: ProviderKind, id: &str) -> PathBuf {
        self.root.join(pdir(p)).join(format!("pl_{}.json", safe(id)))
    }

    pub fn store_tracks(&mut self, p: ProviderKind, id: &str, tracks: Vec<Track>) {
        if let Ok(b) = serde_json::to_vec(&tracks) {
            let _ = crate::storage::atomic_write(&self.tracks_path(p, id), &b);
        }
        let keep = self.keep_in_memory;
        let l = self.lib(p);
        if keep || l.loaded.contains_key(id) {
            l.loaded.insert(id.to_string(), tracks);
        }
    }

    /// Keep a timestamped copy of the cached items before a destructive refresh.
    pub fn backup_tracks(&self, p: ProviderKind, id: &str) {
        let src = self.tracks_path(p, id);
        if src.exists() {
            let dst = self.root.join("backups").join(format!("{}_{}_{}.json", pdir(p), safe(id), crate::storage::now_unix()));
            let _ = crate::storage::ensure_dir(dst.parent().unwrap());
            let _ = std::fs::copy(src, dst);
        }
    }

    /// Cached tracks (loading from disk if necessary). None if never imported.
    pub fn tracks(&mut self, p: ProviderKind, id: &str) -> Option<&Vec<Track>> {
        if !self.lib(p).loaded.contains_key(id) {
            let path = self.tracks_path(p, id);
            let b = std::fs::read(&path).ok()?;
            match serde_json::from_slice::<Vec<Track>>(&b) {
                Ok(t) => {
                    self.lib(p).loaded.insert(id.to_string(), t);
                }
                Err(_) => {
                    // corrupted cache entry: discard so it will be refetched
                    let _ = std::fs::remove_file(&path);
                    return None;
                }
            }
        }
        self.lib(p).loaded.get(id)
    }

    /// With the metadata cache disabled, drop all in-memory track lists except `keep`.
    pub fn evict_except(&mut self, keep: Option<(ProviderKind, &str)>) {
        for (p, l) in self.libs.iter_mut() {
            l.loaded.retain(|id, _| keep.is_some_and(|(kp, kid)| kp == *p && kid == id));
            l.loaded.shrink_to_fit();
        }
    }

    pub fn loaded_bytes_estimate(&self) -> usize {
        self.libs
            .values()
            .flat_map(|l| l.loaded.values())
            .flat_map(|v| v.iter())
            .map(|t| 160 + t.title.len() + t.album.len() + t.id.len() + t.uri.len() + t.artwork_url.len() + t.artists.iter().map(|a| a.len() + 24).sum::<usize>())
            .sum()
    }

    pub fn remove_playlist(&mut self, p: ProviderKind, id: &str) {
        self.backup_tracks(p, id);
        let _ = std::fs::remove_file(self.tracks_path(p, id));
        let l = self.lib(p);
        l.playlists.retain(|e| e.meta.id != id);
        l.loaded.remove(id);
        l.conflicts.retain(|c| c.playlist_id != id);
        self.save_index(p);
    }

    /// Drop the cached items of a playlist but keep its header (un-import).
    pub fn remove_playlist_cache_only(&mut self, p: ProviderKind, id: &str) {
        let _ = std::fs::remove_file(self.tracks_path(p, id));
        self.lib(p).loaded.remove(id);
        self.save_index(p);
    }

    /// Forget everything for a provider (account disconnect).
    pub fn clear_provider(&mut self, p: ProviderKind) {
        let _ = std::fs::remove_dir_all(self.root.join(pdir(p)));
        self.libs.insert(p, ProviderLib::default());
    }

    pub fn all_tracks_search(&mut self, q: &str, limit: usize) -> Vec<Track> {
        let mut out = Vec::new();
        let q = q.to_lowercase();
        for l in self.libs.values() {
            for v in l.loaded.values() {
                for t in v {
                    if out.len() >= limit {
                        return out;
                    }
                    if t.title.to_lowercase().contains(&q) || t.artist_line().to_lowercase().contains(&q) || t.album.to_lowercase().contains(&q) {
                        if !out.iter().any(|o: &Track| o.key() == t.key()) {
                            out.push(t.clone());
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(id: &str, snap: &str) -> Playlist {
        Playlist { id: id.into(), name: id.to_uppercase(), snapshot: snap.into(), provider: ProviderKind::Spotify, ..Default::default() }
    }

    #[test]
    fn import_and_sync_cycle() {
        let mut lib = ProviderLib::default();
        let (r, refetch) = merge_listing(&mut lib, vec![pl("a", "1"), pl("b", "1")], true);
        assert_eq!(r.added.len(), 2);
        assert_eq!(refetch, vec!["a", "b"]);
        // unchanged second sync: no refetch (no unnecessary API calls)
        let (r, refetch) = merge_listing(&mut lib, vec![pl("a", "1"), pl("b", "1")], true);
        assert!(refetch.is_empty());
        assert_eq!(r.unchanged, 2);
        // a changes, b removed upstream, c new
        let (r, refetch) = merge_listing(&mut lib, vec![pl("a", "2"), pl("c", "1")], true);
        assert_eq!(refetch, vec!["a", "c"]);
        assert_eq!(r.removed_upstream, vec!["B"]);
        let b = lib.playlist("b").unwrap();
        assert!(b.removed_upstream, "kept locally, flagged");
        assert_eq!(lib.conflicts.len(), 1);
        // b comes back
        let (_, _) = merge_listing(&mut lib, vec![pl("a", "2"), pl("b", "1"), pl("c", "1")], true);
        assert!(!lib.playlist("b").unwrap().removed_upstream);
        assert!(lib.conflicts.is_empty());
    }

    #[test]
    fn selective_import_does_not_refetch_unimported() {
        let mut lib = ProviderLib::default();
        merge_listing(&mut lib, vec![pl("a", "1")], false);
        assert!(!lib.playlist("a").unwrap().imported);
        let (_, refetch) = merge_listing(&mut lib, vec![pl("a", "2")], false);
        assert!(refetch.is_empty());
        // non-imported playlists that vanish are simply dropped (nothing cached to protect)
        merge_listing(&mut lib, vec![], false);
        assert!(lib.playlist("a").is_none());
    }

    #[test]
    fn shrink_conflict() {
        let mut lib = ProviderLib::default();
        assert!(!detect_shrink(&mut lib, "x", "X", 10, 1));
        assert!(detect_shrink(&mut lib, "x", "X", 100, 10));
        assert_eq!(lib.conflicts.len(), 1);
    }

    #[test]
    fn store_roundtrip_and_eviction_and_corruption() {
        let dir = std::env::temp_dir().join(format!("aqua-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = LibraryStore::open(dir.clone());
        let tracks: Vec<Track> = (0..5000).map(|i| Track { id: i.to_string(), title: format!("t{i}"), ..Default::default() }).collect();
        s.lib(ProviderKind::Spotify).playlists.push(PlaylistEntry { meta: pl("big", "1"), imported: true, ..Default::default() });
        s.store_tracks(ProviderKind::Spotify, "big", tracks.clone());
        s.save_index(ProviderKind::Spotify);
        s.evict_except(None);
        assert!(s.lib(ProviderKind::Spotify).loaded.is_empty());
        let mut s2 = LibraryStore::open(dir.clone());
        assert_eq!(s2.tracks(ProviderKind::Spotify, "big").unwrap().len(), 5000);
        assert_eq!(s2.all_tracks_search("t4999", 10).len(), 1);
        // corrupt cache file
        std::fs::write(dir.join("spotify").join("pl_big.json"), b"{nope").unwrap();
        let mut s3 = LibraryStore::open(dir.clone());
        assert!(s3.tracks(ProviderKind::Spotify, "big").is_none());
        // corrupt index
        std::fs::write(dir.join("spotify").join("index.json"), b"garbage").unwrap();
        let mut s4 = LibraryStore::open(dir.clone());
        assert!(s4.lib(ProviderKind::Spotify).playlists.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
