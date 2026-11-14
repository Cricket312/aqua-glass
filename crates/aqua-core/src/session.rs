//! The locally saved listening session ("Remember queue"): queue entries in
//! play order with their origin (context / manual / radio), the current
//! position and the playback position. Device-local file (`session.json`),
//! never synced as-is (it may contain local file paths); the shareable form is
//! `crate::sync::QueueState`.

use crate::model::Track;
use crate::playback::PlayerState;
use crate::queue::EntryOrigin;
use serde::{Deserialize, Serialize};

pub const SESSION_VERSION: u32 = 2;
/// Upper bound on saved entries (keeps the file small for huge queues).
pub const MAX_ENTRIES: usize = 5000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedSession {
    pub version: u32,
    pub entries: Vec<(Track, EntryOrigin)>,
    pub pos: usize,
    pub position_ms: u64,
}

impl SavedSession {
    pub fn from_state(st: &PlayerState, position_ms: u64) -> Option<SavedSession> {
        let pos = st.queue_pos?;
        let entries = st
            .queue
            .iter()
            .enumerate()
            .take(MAX_ENTRIES)
            .map(|(i, t)| (t.clone(), st.queue_origin.get(i).copied().unwrap_or_default()))
            .collect();
        Some(SavedSession { version: SESSION_VERSION, entries, pos, position_ms })
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Reads the current format and the v1 format (`[tracks, pos, position_ms]`,
    /// Aqua Glass ≤ 0.3), whose entries all become `Context`.
    pub fn decode(bytes: &[u8]) -> Option<SavedSession> {
        if let Ok(s) = serde_json::from_slice::<SavedSession>(bytes) {
            return (s.version <= SESSION_VERSION).then_some(s);
        }
        let (tracks, pos, position_ms) = serde_json::from_slice::<(Vec<Track>, usize, u64)>(bytes).ok()?;
        Some(SavedSession { version: SESSION_VERSION, entries: tracks.into_iter().map(|t| (t, EntryOrigin::Context)).collect(), pos, position_ms })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: &str) -> Track {
        Track { id: n.into(), title: n.into(), ..Default::default() }
    }

    #[test]
    fn roundtrip_keeps_origins() {
        let st = PlayerState {
            queue: vec![t("a"), t("b"), t("c")],
            queue_origin: vec![EntryOrigin::Context, EntryOrigin::Manual, EntryOrigin::Radio],
            queue_pos: Some(1),
            ..Default::default()
        };
        let s = SavedSession::from_state(&st, 4200).unwrap();
        let back = SavedSession::decode(&s.encode()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.entries[2].1, EntryOrigin::Radio);
        assert_eq!(back.position_ms, 4200);
    }

    #[test]
    fn reads_v1_session_files() {
        let v1 = serde_json::to_vec(&(vec![t("a"), t("b")], 1usize, 900u64)).unwrap();
        let s = SavedSession::decode(&v1).unwrap();
        assert_eq!(s.pos, 1);
        assert_eq!(s.entries.len(), 2);
        assert!(s.entries.iter().all(|e| e.1 == EntryOrigin::Context));
        assert!(SavedSession::decode(b"garbage").is_none());
    }
}
