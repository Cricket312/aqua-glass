//! Platform-independent playback interface.
//!
//! The *logical* player — queue, current song, position, play/pause, the
//! manual-vs-radio distinction — is described here. Each platform implements
//! [`PlaybackControl`] on top of its own audio stack:
//!
//! * Windows desktop: `audio::engine::Engine` (cpal output, librespot,
//!   symphonia), with SMTC/tray/hotkeys as separate adapters.
//! * Android (future): a controller wired to a `MediaSessionService`,
//!   audio focus and notification controls, using Android audio output.
//!
//! Similarity Radio and (future) sync/handoff only talk to this trait, so they
//! never depend on a particular audio backend or UI toolkit.

use crate::model::{RepeatMode, Track};
use crate::queue::EntryOrigin;

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
    pub message: Option<String>,
    pub error: Option<String>,
    pub spotify: SpotifyStatus,
    /// Parallel to `queue`: why each entry is queued (context / manual / radio).
    pub queue_origin: Vec<EntryOrigin>,
    /// Process id of a separate playback helper (Spotify Soloist), for memory accounting.
    pub helper_pid: Option<u32>,
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

impl PlayerState {
    /// Queue entry `i` (play order) is a Similarity Radio recommendation.
    pub fn is_radio(&self, i: usize) -> bool {
        self.queue_origin.get(i) == Some(&EntryOrigin::Radio)
    }
}

pub type StateListener = Box<dyn FnMut(&PlayerState) + Send>;

/// Commands every playback controller understands. Methods are fire-and-forget
/// (they queue a command for the controller's own thread); read results back
/// with [`PlaybackControl::snapshot`] or a listener.
pub trait PlaybackControl: Send + Sync {
    /// Replace the queue with `tracks` and start playing at `start`.
    fn play(&self, tracks: Vec<Track>, start: usize);
    fn pause(&self);
    fn resume(&self);
    fn toggle(&self);
    fn next(&self);
    fn previous(&self);
    fn seek(&self, position_ms: u64);
    /// 0.0 ..= 1.0
    fn set_volume(&self, volume: f32);
    /// Manual "Add to queue" (plays before radio recommendations).
    fn enqueue(&self, tracks: Vec<Track>);
    /// Manual "Play next".
    fn play_next(&self, tracks: Vec<Track>);
    /// Similarity Radio recommendations (after everything else).
    fn append_radio(&self, tracks: Vec<Track>);
    /// Drop radio recommendations that haven't played yet.
    fn clear_radio_upcoming(&self);
    fn jump_to(&self, order_pos: usize);
    fn remove(&self, order_pos: usize);
    fn move_item(&self, from: usize, to: usize);
    fn clear_queue(&self);
    fn set_shuffle(&self, on: bool);
    fn set_repeat(&self, mode: RepeatMode);
    /// Current state (cheap clone).
    fn snapshot(&self) -> PlayerState;
    /// Audible position of the current track.
    fn position_ms(&self) -> u64;
    /// Called on the controller's thread after every state change.
    fn add_listener(&self, listener: StateListener);
}
