//! Serialisable state shared between devices.

use super::clock::{HybridClock, Lww, LwwMap, Stamp};
use super::DeviceId;
use crate::history::{track_ident, PlayEvent};
use crate::model::{Album, ProviderKind, RepeatMode, Track};
use crate::playback::PlayerState;
use crate::prefs::RadioPrefs;
use crate::queue::{EntryOrigin, Queue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Track references

/// Provider-qualified, stable reference to a track. Always carries the
/// provider and the provider's id; title/artist/album/duration/ISRC travel
/// along as *metadata* for display and for matching on a device where that
/// provider isn't available — they are never the identity on their own.
///
/// Local files: the absolute path is device-specific (and may contain the
/// user's account name), so it is **not** sent. The id becomes the normalised
/// `artist|title` identity, and other devices resolve it by matching.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TrackRef {
    pub provider: ProviderKind,
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isrc: String,
}

impl TrackRef {
    pub fn from_track(t: &Track) -> TrackRef {
        let id = match t.provider {
            ProviderKind::Local => track_ident(t),
            _ => t.id.clone(),
        };
        TrackRef { provider: t.provider, id, title: t.title.clone(), artists: t.artists.clone(), album: t.album.clone(), duration_ms: t.duration_ms, isrc: t.isrc.clone() }
    }

    /// Stable key (`Spotify:<id>`, `Local:<artist|title>`; `YouTube:<id>` in pre-0.6 data) —
    /// the same format as `Track::key()` for streaming tracks.
    pub fn key(&self) -> String {
        format!("{:?}:{}", self.provider, self.id)
    }

    /// Can be played directly by id on any device signed in to the provider.
    pub fn is_portable(&self) -> bool {
        self.provider != ProviderKind::Local
    }

    /// A `Track` for display/playback. Local refs come back without a file
    /// path and must be resolved by matching first.
    pub fn to_track(&self) -> Track {
        Track {
            provider: self.provider,
            id: self.id.clone(),
            uri: if self.provider == ProviderKind::Spotify { format!("spotify:track:{}", self.id) } else { String::new() },
            title: self.title.clone(),
            artists: self.artists.clone(),
            album: self.album.clone(),
            duration_ms: self.duration_ms,
            isrc: self.isrc.clone(),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AlbumRef {
    pub provider: ProviderKind,
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
}

impl AlbumRef {
    pub fn from_album(a: &Album) -> AlbumRef {
        AlbumRef { provider: a.provider, id: a.id.clone(), name: a.name.clone(), artists: a.artists.clone() }
    }
    pub fn key(&self) -> String {
        format!("{:?}:{}", self.provider, self.id)
    }
}

// ---------------------------------------------------------------------------
// Listening state

/// "What is playing where": published by the playing device when the track
/// changes or playback starts/stops/seeks (not continuously). Other devices
/// estimate the live position from `position_ms` + elapsed time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ListeningState {
    pub device: DeviceId,
    pub track: Option<TrackRef>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
    /// Wall-clock ms at which `position_ms` was sampled.
    pub updated_ms: i64,
    pub stamp: Stamp,
}

impl ListeningState {
    pub fn from_player(st: &PlayerState, position_ms: u64, stamp: Stamp) -> ListeningState {
        ListeningState {
            device: stamp.device.clone(),
            track: st.current.as_ref().map(TrackRef::from_track),
            position_ms,
            duration_ms: st.duration_ms,
            playing: st.playing,
            updated_ms: stamp.ms,
            stamp,
        }
    }

    /// Approximate position now (clamped to the duration).
    pub fn estimated_position_ms(&self, now_ms: i64) -> u64 {
        let mut p = self.position_ms;
        if self.playing {
            p += (now_ms - self.updated_ms).max(0) as u64;
        }
        if self.duration_ms > 0 {
            p = p.min(self.duration_ms);
        }
        p
    }

    /// Newer state for the same device wins.
    pub fn merge(&mut self, other: &ListeningState) {
        if other.stamp > self.stamp {
            *self = other.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// Queue state

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub track: TrackRef,
    /// Context / Manual / Radio — preserved so manual picks keep priority over
    /// radio recommendations on every device.
    pub origin: EntryOrigin,
}

/// The radio session that produced the queue's `Radio` entries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RadioSession {
    /// Current seed (drifts with the music unless "Stay close to original").
    pub seed: Option<TrackRef>,
    pub original_seed: Option<TrackRef>,
    pub prefs: RadioPrefs,
}

/// A device's play queue. Queues are per-device sessions: each device
/// publishes its own, and handoff copies one device's queue to another —
/// two devices' queues are never interleaved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct QueueState {
    pub device: DeviceId,
    /// Entries in play order.
    pub entries: Vec<QueueEntry>,
    /// Index into `entries` of the current track.
    pub current: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub radio: Option<RadioSession>,
    pub stamp: Stamp,
}

impl QueueState {
    pub fn from_queue(q: &Queue, radio: Option<RadioSession>, stamp: Stamp) -> QueueState {
        let entries = q.order.iter().map(|i| QueueEntry { track: TrackRef::from_track(&q.tracks[*i]), origin: q.origin.get(*i).copied().unwrap_or_default() }).collect();
        QueueState { device: stamp.device.clone(), entries, current: q.pos, shuffle: q.shuffle, repeat: q.repeat, radio, stamp }
    }

    pub fn from_player(st: &PlayerState, radio: Option<RadioSession>, stamp: Stamp) -> QueueState {
        let entries = st.queue.iter().enumerate().map(|(i, t)| QueueEntry { track: TrackRef::from_track(t), origin: st.queue_origin.get(i).copied().unwrap_or_default() }).collect();
        QueueState { device: stamp.device.clone(), entries, current: st.queue_pos, shuffle: st.shuffle, repeat: st.repeat, radio, stamp }
    }

    /// Rebuild a playable queue (local refs still need resolving).
    pub fn to_queue(&self) -> Queue {
        Queue::from_entries(self.entries.iter().map(|e| (e.track.to_track(), e.origin)).collect(), self.current, self.shuffle, self.repeat)
    }

    pub fn upcoming(&self) -> &[QueueEntry] {
        let start = self.current.map(|c| c + 1).unwrap_or(0).min(self.entries.len());
        &self.entries[start..]
    }

    pub fn merge(&mut self, other: &QueueState) {
        if other.stamp > self.stamp {
            *self = other.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// Library state

/// One play, as an append-only event with a globally unique id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// `<device>:<ms>:<ctr>` — unique, so merging is a set union.
    pub id: String,
    pub track: TrackRef,
    pub at: i64,
    pub listened_ms: u64,
    pub completed: bool,
    pub skipped: bool,
}

impl HistoryEntry {
    pub fn from_event(e: &PlayEvent, stamp: &Stamp) -> HistoryEntry {
        HistoryEntry { id: format!("{}:{}:{}", stamp.device, stamp.ms, stamp.ctr), track: TrackRef::from_track(&e.track), at: e.at, listened_ms: e.listened_ms, completed: e.completed, skipped: e.skipped }
    }
}

/// Where a synced playlist comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlaylistOrigin {
    /// Created in Aqua Glass; owned by the user's devices. Editable everywhere.
    #[default]
    Local,
    /// Imported from a provider. The provider owns it: devices re-read it from
    /// the provider; sync only shares the reference and never writes back.
    Provider { provider: ProviderKind, provider_id: String, snapshot: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub track: TrackRef,
    /// Sort key; moving an item writes a new key between its neighbours.
    pub pos: Lww<f64>,
    pub present: Lww<bool>,
}

/// A playlist as a mergeable document. Items have their own ids, so adding
/// on two devices keeps both additions, removals are tombstones, and
/// rename/move/delete are last-writer-wins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncPlaylist {
    pub id: String,
    pub origin: PlaylistOrigin,
    pub name: Lww<String>,
    pub description: Lww<String>,
    pub deleted: Lww<bool>,
    pub items: BTreeMap<String, PlaylistItem>,
    /// Provider playlists: when this replica's item listing was read from the
    /// provider. The newest listing wins as a whole (the provider is the
    /// source of truth; Aqua Glass never edits it through sync).
    #[serde(default)]
    pub listed: Stamp,
}

impl SyncPlaylist {
    /// A new Aqua Glass playlist. The id embeds the creating device and stamp,
    /// so two devices creating playlists offline never collide.
    pub fn new_local(name: &str, clock: &mut HybridClock, now_ms: i64) -> SyncPlaylist {
        let s = clock.tick(now_ms);
        SyncPlaylist {
            id: format!("local:{}:{}:{}", s.device, s.ms, s.ctr),
            origin: PlaylistOrigin::Local,
            name: Lww::new(name.to_string(), s.clone()),
            description: Lww::new(String::new(), s.clone()),
            deleted: Lww::new(false, s),
            items: BTreeMap::new(),
            listed: Stamp::default(),
        }
    }

    /// Deterministic id for an imported provider playlist: importing the same
    /// Spotify playlist on two devices yields one synced playlist,
    /// not two.
    pub fn import_id(provider: ProviderKind, provider_id: &str) -> String {
        format!("{provider:?}:{provider_id}")
    }

    pub fn from_provider(p: &crate::model::Playlist, tracks: &[Track], stamp: Stamp) -> SyncPlaylist {
        let mut items = BTreeMap::new();
        for (i, t) in tracks.iter().enumerate() {
            // positional ids: provider order is authoritative for imported lists
            items.insert(format!("{i:06}"), PlaylistItem { track: TrackRef::from_track(t), pos: Lww::new(i as f64, stamp.clone()), present: Lww::new(true, stamp.clone()) });
        }
        SyncPlaylist {
            id: Self::import_id(p.provider, &p.id),
            origin: PlaylistOrigin::Provider { provider: p.provider, provider_id: p.id.clone(), snapshot: p.snapshot.clone() },
            name: Lww::new(p.name.clone(), stamp.clone()),
            description: Lww::new(p.description.clone(), stamp.clone()),
            deleted: Lww::new(false, stamp.clone()),
            items,
            listed: stamp,
        }
    }

    /// Only Aqua Glass playlists can be edited through sync.
    pub fn editable(&self) -> bool {
        self.origin == PlaylistOrigin::Local
    }

    /// Append a track. Returns the new item id (None for provider playlists).
    pub fn add(&mut self, t: &TrackRef, clock: &mut HybridClock, now_ms: i64) -> Option<String> {
        if !self.editable() {
            return None;
        }
        let s = clock.tick(now_ms);
        let id = format!("{}:{}:{}", s.device, s.ms, s.ctr);
        let pos = self.items.values().filter(|i| i.present.value).map(|i| i.pos.value).fold(0.0, f64::max) + 1.0;
        self.items.insert(id.clone(), PlaylistItem { track: t.clone(), pos: Lww::new(pos, s.clone()), present: Lww::new(true, s) });
        Some(id)
    }

    pub fn remove(&mut self, item_id: &str, clock: &mut HybridClock, now_ms: i64) -> bool {
        if !self.editable() {
            return false;
        }
        let s = clock.tick(now_ms);
        match self.items.get_mut(item_id) {
            Some(i) => {
                i.present.set(false, s);
                true
            }
            None => false,
        }
    }

    /// Move an item so it ends up at `index` among the present items.
    pub fn move_to(&mut self, item_id: &str, index: usize, clock: &mut HybridClock, now_ms: i64) -> bool {
        if !self.editable() || !self.items.contains_key(item_id) {
            return false;
        }
        let others: Vec<f64> = self.ordered().into_iter().filter(|(id, _)| id != item_id).map(|(_, i)| i.pos.value).collect();
        let new_pos = match (index.checked_sub(1).and_then(|k| others.get(k)), others.get(index)) {
            (None, None) => 1.0,
            (None, Some(b)) => b - 1.0,
            (Some(a), None) => a + 1.0,
            (Some(a), Some(b)) => (a + b) / 2.0,
        };
        let s = clock.tick(now_ms);
        self.items.get_mut(item_id).unwrap().pos.set(new_pos, s);
        true
    }

    pub fn rename(&mut self, name: &str, clock: &mut HybridClock, now_ms: i64) {
        let s = clock.tick(now_ms);
        self.name.set(name.to_string(), s);
    }

    /// Present items in order (position, then item id as a deterministic tie-break).
    pub fn ordered(&self) -> Vec<(String, &PlaylistItem)> {
        let mut v: Vec<(String, &PlaylistItem)> = self.items.iter().filter(|(_, i)| i.present.value).map(|(k, i)| (k.clone(), i)).collect();
        v.sort_by(|a, b| a.1.pos.value.total_cmp(&b.1.pos.value).then_with(|| a.0.cmp(&b.0)));
        v
    }

    pub fn tracks(&self) -> Vec<TrackRef> {
        self.ordered().into_iter().map(|(_, i)| i.track.clone()).collect()
    }

    pub fn merge(&mut self, other: &SyncPlaylist) {
        debug_assert_eq!(self.id, other.id);
        self.name.merge(&other.name);
        self.description.merge(&other.description);
        self.deleted.merge(&other.deleted);
        if !self.editable() || !other.editable() {
            if other.listed > self.listed {
                self.origin = other.origin.clone();
                self.items = other.items.clone();
                self.listed = other.listed.clone();
            }
            return;
        }
        for (k, it) in &other.items {
            match self.items.get_mut(k) {
                Some(mine) => {
                    mine.pos.merge(&it.pos);
                    mine.present.merge(&it.present);
                }
                None => {
                    self.items.insert(k.clone(), it.clone());
                }
            }
        }
    }
}

/// Aqua Glass's own library data, shared between devices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LibraryState {
    /// By playlist id.
    pub playlists: BTreeMap<String, SyncPlaylist>,
    /// Aqua Glass favourites, keyed by `TrackRef::key()`. (Spotify "Liked
    /// Songs" stays provider-owned and is re-read from Spotify on each device.)
    pub favorites: LwwMap<TrackRef>,
    pub saved_albums: LwwMap<AlbumRef>,
    /// Similarity Radio recommendations the user chose to keep.
    pub saved_recommendations: LwwMap<TrackRef>,
    /// Newest last, capped at [`HISTORY_CAP`].
    pub history: Vec<HistoryEntry>,
}

pub const HISTORY_CAP: usize = 2000;

impl LibraryState {
    /// Merge another replica. Commutative, associative and idempotent, so the
    /// order in which devices sync doesn't matter.
    pub fn merge(&mut self, other: &LibraryState) {
        for (id, p) in &other.playlists {
            match self.playlists.get_mut(id) {
                Some(mine) => mine.merge(p),
                None => {
                    self.playlists.insert(id.clone(), p.clone());
                }
            }
        }
        self.favorites.merge(&other.favorites);
        self.saved_albums.merge(&other.saved_albums);
        self.saved_recommendations.merge(&other.saved_recommendations);
        let mut all: BTreeMap<(i64, String), HistoryEntry> = BTreeMap::new();
        for e in self.history.iter().chain(other.history.iter()) {
            all.entry((e.at, e.id.clone())).or_insert_with(|| e.clone());
        }
        let n = all.len();
        self.history = all.into_values().skip(n.saturating_sub(HISTORY_CAP)).collect();
    }

    /// Visible (non-deleted) playlists.
    pub fn live_playlists(&self) -> impl Iterator<Item = &SyncPlaylist> {
        self.playlists.values().filter(|p| !p.deleted.value)
    }
}

// ---------------------------------------------------------------------------
// Settings state

/// Playback preferences that make sense on any device. Streaming quality is
/// deliberately *not* here (a phone on mobile data wants a different value).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackPrefs {
    pub preferred_provider: ProviderKind,
    pub fade_ms: u32,
    pub gapless: bool,
    pub normalization: bool,
    pub seek_step_s: u32,
}

impl Default for PlaybackPrefs {
    fn default() -> Self {
        PlaybackPrefs { preferred_provider: ProviderKind::Spotify, fade_ms: 0, gapless: true, normalization: false, seek_step_s: 10 }
    }
}

/// What this user wants synchronised. Each category can be switched off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncPrefs {
    pub history: bool,
    pub playlists: bool,
    pub favorites: bool,
    pub queue: bool,
    pub listening: bool,
    pub radio_prefs: bool,
    pub playback_prefs: bool,
    /// Off by default: a theme tuned for a desktop may not suit a phone.
    pub theme: bool,
}

impl Default for SyncPrefs {
    fn default() -> Self {
        SyncPrefs { history: true, playlists: true, favorites: true, queue: true, listening: true, radio_prefs: true, playback_prefs: true, theme: false }
    }
}

/// Settings that may be shared between devices. Device-specific settings —
/// audio output device, window size/position, UI scale, Windows integrations
/// (tray, hotkeys, autostart, SMTC), file paths, RAM limits — have no field
/// here, so they cannot leak into a sync payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SettingsState {
    /// Theme name (built-in or user theme).
    pub theme: Lww<String>,
    pub radio: Lww<RadioPrefs>,
    pub playback: Lww<PlaybackPrefs>,
    pub sync: Lww<SyncPrefs>,
}

impl SettingsState {
    pub fn merge(&mut self, other: &SettingsState) {
        self.theme.merge(&other.theme);
        self.radio.merge(&other.radio);
        self.playback.merge(&other.playback);
        self.sync.merge(&other.sync);
    }
}

/// Everything one device can share, for export/inspection and as the unit a
/// future server stores per device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Snapshot {
    pub device: super::DeviceInfo,
    pub listening: Option<ListeningState>,
    pub queue: Option<QueueState>,
    pub library: LibraryState,
    pub settings: SettingsState,
}
