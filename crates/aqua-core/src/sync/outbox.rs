//! Pending local changes for offline-first sync.
//!
//! Every change is recorded locally first; the app works fully offline. When
//! a sync server exists, pending changes are uploaded in `seq` order and
//! removed once acknowledged. Because every payload is a mergeable state (not
//! an imperative "do X"), re-sending after a lost acknowledgement is harmless.

use super::state::{HistoryEntry, ListeningState, QueueState, SettingsState, SyncPlaylist, TrackRef};
use super::clock::Lww;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Change {
    Favorite { key: String, value: Lww<Option<TrackRef>> },
    Playlist(SyncPlaylist),
    History(HistoryEntry),
    Settings(SettingsState),
    Listening(ListeningState),
    Queue(QueueState),
}

impl Change {
    /// Changes where only the latest one matters (older pending ones are replaced).
    fn slot(&self) -> Option<String> {
        match self {
            Change::Favorite { key, .. } => Some(format!("fav:{key}")),
            Change::Playlist(p) => Some(format!("pl:{}", p.id)),
            Change::Settings(_) => Some("settings".into()),
            Change::Listening(_) => Some("listening".into()),
            Change::Queue(_) => Some("queue".into()),
            Change::History(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub seq: u64,
    pub change: Change,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Outbox {
    pub next_seq: u64,
    pub pending: Vec<Pending>,
}

/// Bound on queued changes; the oldest history events go first when exceeded
/// (state-type changes are coalesced, so they never pile up).
pub const OUTBOX_CAP: usize = 5000;

impl Outbox {
    pub fn push(&mut self, change: Change) -> u64 {
        if let Some(slot) = change.slot() {
            self.pending.retain(|p| p.change.slot().as_deref() != Some(&slot));
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.pending.push(Pending { seq, change });
        while self.pending.len() > OUTBOX_CAP {
            match self.pending.iter().position(|p| matches!(p.change, Change::History(_))) {
                Some(i) => {
                    self.pending.remove(i);
                }
                None => break,
            }
        }
        seq
    }

    /// The server stored everything up to and including `seq`.
    pub fn ack_through(&mut self, seq: u64) {
        self.pending.retain(|p| p.seq > seq);
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
