//! Playlist export/import (JSON, CSV, M3U) and cross-provider transfer jobs.
//!
//! Transfers never modify the source playlist. Every transfer writes a backup
//! of the source and a log of what was created so it can be reviewed, and the
//! created playlist can be removed again (only on explicit user request).

use crate::config::ProviderKind;
use crate::matching::{self, Candidate, MatchStatus};
use crate::model::{Playlist, Track};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlaylistExport {
    pub format: String,
    pub version: u32,
    pub exported_at: i64,
    pub playlist: Playlist,
    pub tracks: Vec<Track>,
}

pub fn export_json(p: &Playlist, tracks: &[Track], path: &Path) -> anyhow::Result<()> {
    let e = PlaylistExport { format: "aqua-glass-playlist".into(), version: 1, exported_at: crate::util::now_unix(), playlist: p.clone(), tracks: tracks.to_vec() };
    crate::util::atomic_write(path, serde_json::to_string_pretty(&e)?.as_bytes())?;
    Ok(())
}

pub fn export_csv(tracks: &[Track], path: &Path) -> anyhow::Result<()> {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(["position", "title", "artists", "album", "duration_ms", "isrc", "explicit", "provider", "id", "uri"])?;
    for (i, t) in tracks.iter().enumerate() {
        w.write_record([
            (i + 1).to_string(),
            t.title.clone(),
            t.artists.join("; "),
            t.album.clone(),
            t.duration_ms.to_string(),
            t.isrc.clone(),
            t.explicit.to_string(),
            t.provider.label().to_string(),
            t.id.clone(),
            t.uri.clone(),
        ])?;
    }
    crate::util::atomic_write(path, &w.into_inner()?)?;
    Ok(())
}

pub fn export_m3u(p: &Playlist, tracks: &[Track], path: &Path) -> anyhow::Result<()> {
    let mut s = String::from("#EXTM3U\n");
    s.push_str(&format!("#PLAYLIST:{}\n", p.name.replace('\n', " ")));
    for t in tracks {
        let secs = (t.duration_ms / 1000) as i64;
        s.push_str(&format!("#EXTINF:{},{} - {}\n", if secs > 0 { secs } else { -1 }, t.artist_line(), t.title));
        if !t.album.is_empty() {
            s.push_str(&format!("#EXTALB:{}\n", t.album));
        }
        let loc = match t.provider {
            ProviderKind::Local => t.id.clone(),
            _ => t.web_url().unwrap_or_default(),
        };
        s.push_str(&loc);
        s.push('\n');
    }
    crate::util::atomic_write(path, s.as_bytes())?;
    Ok(())
}

/// Restore a playlist from JSON / CSV / M3U.
pub fn import_file(path: &Path) -> anyhow::Result<(Playlist, Vec<Track>)> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let text = std::fs::read_to_string(path)?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Imported".into());
    match ext.as_str() {
        "json" => {
            let e: PlaylistExport = serde_json::from_str(&text)?;
            Ok((e.playlist, e.tracks))
        }
        "csv" => {
            let mut r = csv::Reader::from_reader(text.as_bytes());
            let headers = r.headers()?.clone();
            let col = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
            let (ti, ar, al, du, is, ex, pr, id, ur) =
                (col("title"), col("artists").or(col("artist")), col("album"), col("duration_ms"), col("isrc"), col("explicit"), col("provider"), col("id"), col("uri"));
            let ti = ti.ok_or_else(|| anyhow::anyhow!("CSV needs a 'title' column"))?;
            let mut tracks = Vec::new();
            for rec in r.records() {
                let rec = rec?;
                let g = |i: Option<usize>| i.and_then(|i| rec.get(i)).unwrap_or("").to_string();
                let provider = match g(pr).as_str() {
                    "Spotify" => ProviderKind::Spotify,
                    "YouTube Music" => ProviderKind::YouTube,
                    _ => ProviderKind::Local,
                };
                tracks.push(Track {
                    provider,
                    id: g(id),
                    uri: g(ur),
                    title: g(Some(ti)),
                    artists: g(ar).split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                    album: g(al),
                    duration_ms: g(du).parse().unwrap_or(0),
                    isrc: g(is),
                    explicit: g(ex) == "true",
                    ..Default::default()
                });
            }
            Ok((Playlist { name: stem, track_count: tracks.len() as u32, ..Default::default() }, tracks))
        }
        "m3u" | "m3u8" => {
            let mut name = stem;
            let mut tracks = Vec::new();
            let mut pending: Option<Track> = None;
            for line in text.lines() {
                let line = line.trim();
                if let Some(n) = line.strip_prefix("#PLAYLIST:") {
                    name = n.trim().to_string();
                } else if let Some(info) = line.strip_prefix("#EXTINF:") {
                    let (dur, rest) = info.split_once(',').unwrap_or((info, ""));
                    let (artist, title) = rest.split_once(" - ").unwrap_or(("", rest));
                    let secs: i64 = dur.trim().parse().unwrap_or(-1);
                    pending = Some(Track {
                        title: title.trim().into(),
                        artists: artist.split(", ").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                        duration_ms: if secs > 0 { secs as u64 * 1000 } else { 0 },
                        ..Default::default()
                    });
                } else if let Some(al) = line.strip_prefix("#EXTALB:") {
                    if let Some(p) = pending.as_mut() {
                        p.album = al.trim().into();
                    }
                } else if !line.is_empty() && !line.starts_with('#') {
                    let mut t = pending.take().unwrap_or_else(|| Track {
                        title: Path::new(line).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                        ..Default::default()
                    });
                    if let Some(id) = line.strip_prefix("https://open.spotify.com/track/") {
                        t.provider = ProviderKind::Spotify;
                        t.id = id.split('?').next().unwrap_or("").into();
                        t.uri = format!("spotify:track:{}", t.id);
                    } else if let Some(v) = line.split("watch?v=").nth(1) {
                        t.provider = ProviderKind::YouTube;
                        t.id = v.split('&').next().unwrap_or("").into();
                    } else {
                        t.provider = ProviderKind::Local;
                        t.id = line.into();
                    }
                    tracks.push(t);
                }
            }
            Ok((Playlist { name, track_count: tracks.len() as u32, ..Default::default() }, tracks))
        }
        _ => anyhow::bail!("Unsupported file type '{ext}' (use .json, .csv or .m3u)"),
    }
}

// ---------------------------------------------------------------------------
// Transfer plan

#[derive(Clone, Debug)]
pub struct PlanItem {
    pub src: Track,
    pub candidates: Vec<Candidate>,
    pub status: MatchStatus,
    /// Index into `candidates` the user (or auto-match) picked. None = skip.
    pub choice: Option<usize>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub source: Playlist,
    pub target: ProviderKind,
    pub items: Vec<PlanItem>,
    pub new_name: String,
}

impl Plan {
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let m = self.items.iter().filter(|i| i.status == MatchStatus::Matched).count();
        let a = self.items.iter().filter(|i| i.status == MatchStatus::Ambiguous).count();
        let u = self.items.iter().filter(|i| i.status == MatchStatus::Unmatched).count();
        let sel = self.items.iter().filter(|i| i.choice.is_some()).count();
        (m, a, u, sel)
    }
    pub fn selected_tracks(&self) -> Vec<Track> {
        self.items.iter().filter_map(|i| i.choice.and_then(|c| i.candidates.get(c)).map(|c| c.track.clone())).collect()
    }
}

/// Build a plan by searching the target for each source track.
/// `search` is the target provider's track search; called once per track.
pub fn build_plan(
    source: Playlist,
    tracks: Vec<Track>,
    target: ProviderKind,
    search: &dyn Fn(&str) -> Result<Vec<Track>, String>,
    progress: &dyn Fn(usize, usize),
    cancel: &dyn Fn() -> bool,
) -> Plan {
    let n = tracks.len();
    let mut items = Vec::with_capacity(n);
    let mut rate_limited = false;
    for (i, t) in tracks.into_iter().enumerate() {
        progress(i, n);
        if rate_limited {
            items.push(PlanItem { src: t, candidates: vec![], status: MatchStatus::Unmatched, choice: None, error: Some("not searched: the service asked to slow down — run “Find matches” again later".into()) });
            continue;
        }
        if cancel() {
            items.push(PlanItem { src: t, candidates: vec![], status: MatchStatus::Unmatched, choice: None, error: Some("cancelled".into()) });
            continue;
        }
        if t.provider == target && !t.id.is_empty() {
            // already on target (e.g. restoring a backup)
            items.push(PlanItem { candidates: vec![Candidate { track: t.clone(), score: 1.0, notes: vec!["same provider".into()] }], src: t, status: MatchStatus::Matched, choice: Some(0), error: None });
            continue;
        }
        let q = matching::query_for(&t);
        match search(&q) {
            Ok(c) => {
                let (cands, status) = matching::rank(&t, c);
                let choice = if status == MatchStatus::Matched { Some(0) } else { None };
                items.push(PlanItem { src: t, candidates: cands, status, choice, error: None });
            }
            Err(e) => {
                rate_limited = e.contains("slow down");
                items.push(PlanItem { src: t, candidates: vec![], status: MatchStatus::Unmatched, choice: None, error: Some(e) });
            }
        }
    }
    progress(n, n);
    let new_name = source.name.clone();
    Plan { source, target, items, new_name }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferLog {
    pub at: i64,
    pub source_provider: ProviderKind,
    pub source_id: String,
    pub source_name: String,
    pub target: ProviderKind,
    pub created_playlist_id: String,
    pub added: usize,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    pub backup_file: String,
}

pub fn transfers_dir() -> PathBuf {
    crate::util::data_dir().join("transfers")
}

pub fn save_log(log: &TransferLog) {
    let p = transfers_dir().join(format!("transfer_{}.json", log.at));
    if let Ok(s) = serde_json::to_string_pretty(log) {
        let _ = crate::util::atomic_write(&p, s.as_bytes());
    }
}

pub fn load_logs() -> Vec<TransferLog> {
    let mut v: Vec<TransferLog> = std::fs::read_dir(transfers_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect();
    v.sort_by_key(|l| -l.at);
    v
}

/// Execute a plan. `create` returns the new playlist id; `add` appends items (target ids/uris).
pub fn execute(
    plan: &Plan,
    source_tracks: &[Track],
    create: &dyn Fn(&str, &str) -> Result<String, String>,
    add: &dyn Fn(&str, &[Track]) -> Result<(), String>,
    progress: &dyn Fn(usize, usize),
) -> Result<TransferLog, String> {
    let at = crate::util::now_unix();
    let backup = transfers_dir().join(format!("backup_{}_{}.json", at, plan.source.id.chars().filter(|c| c.is_alphanumeric()).collect::<String>()));
    export_json(&plan.source, source_tracks, &backup).map_err(|e| format!("could not write backup: {e}"))?;
    let desc = format!("Transferred by Aqua Glass from {} \"{}\"", plan.source.provider.label(), plan.source.name);
    let id = create(&plan.new_name, &desc)?;
    let mut log = TransferLog {
        at,
        source_provider: plan.source.provider,
        source_id: plan.source.id.clone(),
        source_name: plan.source.name.clone(),
        target: plan.target,
        created_playlist_id: id.clone(),
        added: 0,
        skipped: vec![],
        failed: vec![],
        backup_file: backup.to_string_lossy().to_string(),
    };
    let chosen: Vec<(&PlanItem, Track)> = plan
        .items
        .iter()
        .filter_map(|i| match i.choice.and_then(|c| i.candidates.get(c)) {
            Some(c) => Some((i, c.track.clone())),
            None => {
                log.skipped.push(format!("{} — {}", i.src.artist_line(), i.src.title));
                None
            }
        })
        .collect();
    let batch = if plan.target == ProviderKind::Spotify { 100 } else { 1 };
    let total = chosen.len();
    for (k, chunk) in chosen.chunks(batch).enumerate() {
        progress(k * batch, total);
        let tracks: Vec<Track> = chunk.iter().map(|(_, t)| t.clone()).collect();
        match add(&id, &tracks) {
            Ok(()) => log.added += tracks.len(),
            Err(e) => {
                for (i, _) in chunk {
                    log.failed.push(format!("{} — {}: {e}", i.src.artist_line(), i.src.title));
                }
                if e.contains("quota") {
                    break;
                }
            }
        }
    }
    progress(total, total);
    save_log(&log);
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn tracks() -> Vec<Track> {
        vec![
            Track { provider: ProviderKind::Spotify, id: "1".into(), uri: "spotify:track:1".into(), title: "One, Two".into(), artists: vec!["A".into(), "B".into()], album: "Al".into(), duration_ms: 61_000, isrc: "X1".into(), ..Default::default() },
            Track { provider: ProviderKind::Spotify, id: "2".into(), uri: "spotify:track:2".into(), title: "Quote \"q\"".into(), artists: vec!["C".into()], duration_ms: 1000, ..Default::default() },
        ]
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aqua-tr-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    #[test]
    fn json_roundtrip() {
        let p = Playlist { name: "My PL".into(), id: "pid".into(), description: "d".into(), ..Default::default() };
        let f = tmp("a.json");
        export_json(&p, &tracks(), &f).unwrap();
        let (p2, t2) = import_file(&f).unwrap();
        assert_eq!(p2, p);
        assert_eq!(t2, tracks());
    }

    #[test]
    fn csv_roundtrip_preserves_order_and_ids() {
        let f = tmp("b.csv");
        export_csv(&tracks(), &f).unwrap();
        let (_, t) = import_file(&f).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].title, "One, Two");
        assert_eq!(t[0].artists, vec!["A", "B"]);
        assert_eq!(t[0].isrc, "X1");
        assert_eq!(t[1].title, "Quote \"q\"");
        assert_eq!(t[1].provider, ProviderKind::Spotify);
    }

    #[test]
    fn m3u_roundtrip() {
        let p = Playlist { name: "Mix".into(), ..Default::default() };
        let f = tmp("c.m3u");
        export_m3u(&p, &tracks(), &f).unwrap();
        let (p2, t) = import_file(&f).unwrap();
        assert_eq!(p2.name, "Mix");
        assert_eq!(t[0].id, "1");
        assert_eq!(t[0].provider, ProviderKind::Spotify);
        assert_eq!(t[0].duration_ms, 61_000);
        assert_eq!(t[0].album, "Al");
    }

    #[test]
    fn plan_and_execute_with_fake_target() {
        std::env::set_var("AQUA_GLASS_HOME", std::env::temp_dir().join(format!("aqua-home-{}", std::process::id())));
        // e.g. a playlist restored from a file: tracks that aren't on Spotify yet
        let src: Vec<Track> = tracks().into_iter().map(|mut t| { t.provider = ProviderKind::Local; t }).collect();
        let search = |q: &str| -> Result<Vec<Track>, String> {
            if q.starts_with("one") {
                Ok(vec![Track { provider: ProviderKind::Spotify, id: "sp1".into(), title: "One, Two".into(), artists: vec!["A".into()], duration_ms: 61_500, ..Default::default() }])
            } else {
                Ok(vec![])
            }
        };
        let plan = build_plan(Playlist { name: "P".into(), id: "p".into(), provider: ProviderKind::Local, ..Default::default() }, src.clone(), ProviderKind::Spotify, &search, &|_, _| {}, &|| false);
        let (m, a, u, sel) = plan.counts();
        assert_eq!((m, a, u, sel), (1, 0, 1, 1));
        let added = RefCell::new(Vec::new());
        let log = execute(
            &plan,
            &src,
            &|name, _| {
                assert_eq!(name, "P");
                Ok("newpl".into())
            },
            &|id, t| {
                assert_eq!(id, "newpl");
                added.borrow_mut().extend(t.iter().map(|t| t.id.clone()));
                Ok(())
            },
            &|_, _| {},
        )
        .unwrap();
        assert_eq!(log.added, 1);
        assert_eq!(log.skipped.len(), 1);
        assert_eq!(*added.borrow(), vec!["sp1"]);
        assert!(Path::new(&log.backup_file).exists(), "source backup written");
        assert!(!load_logs().is_empty());
    }

    #[test]
    fn plan_stops_searching_when_rate_limited() {
        let calls = RefCell::new(0);
        let search = |_: &str| -> Result<Vec<Track>, String> {
            *calls.borrow_mut() += 1;
            Err("The service asked Aqua Glass to slow down — requests resume in about 3 min.".into())
        };
        let src: Vec<Track> = (0..20).map(|i| Track { title: format!("t{i}"), ..Default::default() }).collect();
        let plan = build_plan(Playlist::default(), src, ProviderKind::Spotify, &search, &|_, _| {}, &|| false);
        assert_eq!(*calls.borrow(), 1, "no further searches after a rate limit");
        assert_eq!(plan.items.len(), 20);
        assert!(plan.items[5].error.as_deref().unwrap().contains("run “Find matches” again"));
    }

    #[test]
    fn unsupported_and_bad_files() {
        let f = tmp("x.txt");
        std::fs::write(&f, "hi").unwrap();
        assert!(import_file(&f).is_err());
        let g = tmp("bad.json");
        std::fs::write(&g, "{").unwrap();
        assert!(import_file(&g).is_err());
    }
}
