//! Similarity Radio end-to-end: real engine + audio output + library store + radio
//! worker. Uses the artist/album fallback source (no Last.fm key / network), which
//! exercises start-on-play, initial fill, refill below the threshold, duplicate
//! avoidance and manual-queue priority. Skips when no audio device exists.

use aqua_glass::audio::engine::{wait_until, Cmd, Engine, EngineOpts};
use aqua_glass::audio::output::Output;
use aqua_glass::config::ProviderKind;
use aqua_glass::history::{track_ident, History};
use aqua_glass::library::{LibraryStore, PlaylistEntry, LOCAL_ALL_ID};
use aqua_glass::model::{Playlist, Track};
use aqua_glass::radio::{Deps, Msg, Prefs, Radio};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn wav(path: &std::path::Path, secs: f32) {
    let rate = 44100u32;
    let n = (rate as f32 * secs) as u32;
    let mut d = Vec::new();
    d.extend_from_slice(b"RIFF");
    d.extend_from_slice(&(36 + n * 4).to_le_bytes());
    d.extend_from_slice(b"WAVEfmt ");
    for v in [16u32] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&2u16.to_le_bytes());
    d.extend_from_slice(&rate.to_le_bytes());
    d.extend_from_slice(&(rate * 4).to_le_bytes());
    d.extend_from_slice(&4u16.to_le_bytes());
    d.extend_from_slice(&16u16.to_le_bytes());
    d.extend_from_slice(b"data");
    d.extend_from_slice(&(n * 4).to_le_bytes());
    d.resize(d.len() + (n * 4) as usize, 0);
    std::fs::write(path, d).unwrap();
}

#[test]
fn radio_starts_fills_refills_and_respects_manual_queue() {
    let out = Output::start("", 300);
    if out.s.device_error.lock().unwrap().is_some() {
        eprintln!("no audio device — skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("aqua-radio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // 40 one-second tracks by the same artist (the fallback source recommends same-artist songs)
    let mut tracks = Vec::new();
    for i in 0..40 {
        let p = dir.join(format!("t{i}.wav"));
        wav(&p, 1.0);
        tracks.push(Track { provider: ProviderKind::Local, id: p.to_string_lossy().into(), title: format!("Song {i}"), artists: vec!["Artist".into()], album: format!("Album {}", i / 10), duration_ms: 1000, ..Default::default() });
    }
    let other = dir.join("other.wav");
    wav(&other, 1.0);
    let manual = Track { provider: ProviderKind::Local, id: other.to_string_lossy().into(), title: "Manual".into(), artists: vec!["Someone".into()], duration_ms: 1000, ..Default::default() };

    let mut lib = LibraryStore::open(dir.join("lib"));
    lib.lib(ProviderKind::Local).playlists.push(PlaylistEntry { meta: Playlist { id: LOCAL_ALL_ID.into(), ..Default::default() }, imported: true, ..Default::default() });
    lib.store_tracks(ProviderKind::Local, LOCAL_ALL_ID, tracks.clone());
    let library = Arc::new(Mutex::new(lib));
    let history = Arc::new(Mutex::new(History::default()));

    let engine = Engine::start(out, EngineOpts { gapless: true, ..Default::default() }, 0.2, false, Box::new(|| {}));
    let spotify = Arc::new(aqua_glass::services::spotify::SpotifyApi::new("", 8898));
    let youtube = Arc::new(aqua_glass::services::youtube::YouTubeApi::new(""));
    let radio = Radio::start(
        Deps {
            engine: Arc::new(engine.clone()),
            spotify,
            youtube,
            local_tracks: Arc::new(move || library.lock().unwrap().tracks(ProviderKind::Local, LOCAL_ALL_ID).cloned().unwrap_or_default()),
            history,
            wake: Arc::new(|| {}),
        },
        Prefs { target: 15, threshold: 5, ..Prefs::default() },
    );

    // play one song "from search", then start the radio with it as seed
    let seed = tracks[0].clone();
    engine.send(Cmd::Play { tracks: vec![seed.clone()], start: 0 });
    radio.send(Msg::Start(seed.clone()));

    assert!(wait_until(Duration::from_secs(10), || engine.snapshot().radio_upcoming == 15), "initial fill to the target size, got {}", engine.snapshot().radio_upcoming);
    let st = engine.snapshot();
    assert_eq!(st.queue.len(), 16);
    assert!(st.queue_radio[1..].iter().all(|r| *r));
    assert!(st.queue.iter().skip(1).all(|t| t.key() != seed.key()), "seed not recommended again");
    assert!(radio.status.lock().unwrap().source.contains("Last.fm not configured"), "fallback source is labelled");

    // manual queueing goes before the recommendations
    engine.send(Cmd::Enqueue(vec![manual.clone()]));
    assert!(wait_until(Duration::from_secs(3), || engine.snapshot().queue.get(1).map(|t| t.title == "Manual").unwrap_or(false)), "manual track plays next");

    // skip through the queue: the radio must refill before it runs out
    let mut max_len = 0;
    for _ in 0..16 {
        engine.send(Cmd::Next);
        std::thread::sleep(Duration::from_millis(150));
        let s = engine.snapshot();
        max_len = max_len.max(s.queue.len());
        assert!(s.current.is_some(), "playback continues");
    }
    let ok = wait_until(Duration::from_secs(10), || engine.snapshot().radio_upcoming >= 5);
    if !ok {
        eprintln!("radio status: {:?}", radio.status.lock().unwrap());
        eprintln!("queue len {} pos {:?} err {:?}", engine.snapshot().queue.len(), engine.snapshot().queue_pos, engine.snapshot().error);
    }
    assert!(ok, "refilled, upcoming = {}", engine.snapshot().radio_upcoming);
    let s = engine.snapshot();
    assert!(s.queue.len() > 17, "more recommendations were appended (queue {})", s.queue.len());
    let ids: Vec<String> = s.queue.iter().map(track_ident).collect();
    let uniq: HashSet<&String> = ids.iter().collect();
    assert_eq!(uniq.len(), ids.len(), "no duplicate songs in one radio session");
    assert!(s.error.is_none(), "{:?}", s.error);

    // turning radio off removes upcoming recommendations but keeps playing
    radio.send(Msg::Stop);
    assert!(wait_until(Duration::from_secs(3), || engine.snapshot().radio_upcoming == 0));
    assert!(engine.snapshot().current.is_some());
    engine.send(Cmd::Shutdown);
    let _ = std::fs::remove_dir_all(dir);
}
