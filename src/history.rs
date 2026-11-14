//! Bounded local listening history (last 500 plays) used for "Recently Played",
//! Similarity Radio ("Prefer Unheard", "Avoid Recently Played", skip learning) and
//! "Your Top Mixes". Stored only on this PC (`history.json`); never uploaded.

use crate::audio::engine::PlayRecord;
use crate::model::Track;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

pub const MAX_EVENTS: usize = 500;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlayEvent {
    pub track: Track,
    pub at: i64,
    pub listened_ms: u64,
    pub completed: bool,
    pub skipped: bool,
}

#[derive(Default, Serialize, Deserialize)]
pub struct History {
    pub events: VecDeque<PlayEvent>,
    #[serde(skip)]
    last_seq: u64,
}

/// Normalized "artist|title" identity, provider-independent.
pub fn ident(artist: &str, title: &str) -> String {
    let (core, _) = crate::matching::split_qualifiers(title);
    format!("{}|{}", crate::matching::normalize(artist), core)
}

pub fn track_ident(t: &Track) -> String {
    ident(t.artists.first().map(|s| s.as_str()).unwrap_or(""), &t.title)
}

impl History {
    fn path() -> std::path::PathBuf {
        crate::util::data_dir().join("history.json")
    }

    pub fn load() -> History {
        std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(b) = serde_json::to_vec(self) {
            let _ = crate::util::atomic_write(&Self::path(), &b);
        }
    }

    pub fn clear(&mut self) {
        self.events.clear();
        let _ = std::fs::remove_file(Self::path());
    }

    /// Record a finished/left track. Returns true if something was recorded.
    pub fn record(&mut self, r: &PlayRecord, now: i64) -> bool {
        if r.seq == self.last_seq || r.listened_ms < 3000 && !r.completed {
            self.last_seq = r.seq;
            return false; // ignore accidental clicks
        }
        self.last_seq = r.seq;
        let mut track = r.track.clone();
        track.unavailable = false;
        self.events.push_back(PlayEvent { track, at: now, listened_ms: r.listened_ms.min(24 * 3600 * 1000), completed: r.completed, skipped: r.skipped });
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
        true
    }

    /// Most recent distinct tracks, newest first.
    pub fn recent_tracks(&self, n: usize) -> Vec<Track> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for e in self.events.iter().rev() {
            if seen.insert(e.track.key()) {
                out.push(e.track.clone());
                if out.len() >= n {
                    break;
                }
            }
        }
        out
    }

    pub fn heard(&self, ident_: &str) -> bool {
        self.events.iter().any(|e| track_ident(&e.track) == ident_)
    }

    pub fn heard_since(&self, ident_: &str, since: i64) -> bool {
        self.events.iter().rev().take_while(|e| e.at >= since).any(|e| track_ident(&e.track) == ident_)
    }

    /// Artists ranked by completed plays (for "Your Top Mixes"), with a representative track.
    pub fn top_artists(&self, n: usize) -> Vec<(String, Track, u32)> {
        let mut m: HashMap<String, (Track, u32)> = HashMap::new();
        for e in self.events.iter().filter(|e| e.completed) {
            if let Some(a) = e.track.artists.first() {
                let k = a.to_lowercase();
                let ent = m.entry(k).or_insert((e.track.clone(), 0));
                ent.1 += 1;
                ent.0 = e.track.clone();
            }
        }
        let mut v: Vec<(String, Track, u32)> = m.into_values().map(|(t, c)| (t.artists[0].clone(), t, c)).collect();
        v.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        v.truncate(n);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderKind;

    fn rec(title: &str, artist: &str, seq: u64, completed: bool) -> PlayRecord {
        PlayRecord {
            track: Track { provider: ProviderKind::Local, id: title.into(), title: title.into(), artists: vec![artist.into()], duration_ms: 200_000, ..Default::default() },
            listened_ms: if completed { 200_000 } else { 10_000 },
            completed,
            skipped: !completed,
            seq,
        }
    }

    #[test]
    fn bounded_dedup_and_queries() {
        let mut h = History::default();
        for i in 0..600u64 {
            h.record(&rec(&format!("t{}", i % 7), if i % 2 == 0 { "A" } else { "B" }, i + 1, i % 3 != 0), 1000 + i as i64);
        }
        assert_eq!(h.events.len(), MAX_EVENTS);
        let r = h.recent_tracks(10);
        assert_eq!(r.len(), 7, "distinct only");
        assert!(h.heard(&ident("A", "t0")));
        assert!(!h.heard(&ident("C", "t0")));
        assert!(h.heard_since(&ident("B", "t5"), 1500));
        assert_eq!(h.top_artists(5).len(), 2);
        // same seq twice and very short plays are ignored
        let n = h.events.len();
        assert!(!h.record(&rec("x", "A", 600, true), 0));
        let mut short = rec("y", "A", 9999, false);
        short.listened_ms = 500;
        assert!(!h.record(&short, 0));
        assert_eq!(h.events.len(), n);
    }

    #[test]
    fn ident_ignores_qualifiers() {
        assert_eq!(ident("Daft Punk", "Get Lucky (feat. Pharrell)"), ident("daft punk", "Get Lucky"));
    }
}
