//! Wire-format compatibility: a checked-in schema-1 snapshot must keep
//! decoding, and today's encoder must produce exactly the same JSON for it.
//! If this fails after an intentional *additive* change, regenerate with
//! `AQUA_REGEN_FIXTURES=1 cargo test -p aqua-core --test sync_compat`; for a
//! breaking change bump `SCHEMA_VERSION`, add a `migrate` step and keep the
//! old fixture as a test input.

use aqua_core::model::{Playlist, ProviderKind, RepeatMode, Track};
use aqua_core::platform::Platform;
use aqua_core::queue::{EntryOrigin, Queue};
use aqua_core::sync::*;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sync-snapshot-v1.json");

fn t(p: ProviderKind, id: &str, title: &str, artist: &str) -> Track {
    Track { provider: p, id: id.into(), title: title.into(), artists: vec![artist.into()], album: "Album".into(), duration_ms: 210_000, isrc: if p == ProviderKind::Spotify { format!("ISRC{id}") } else { String::new() }, ..Default::default() }
}

fn sample() -> Envelope<Snapshot> {
    let pc = DeviceId("9f2c6a1e5b7d4c3a8e0f1b2c3d4e5f60".into());
    let mut c = HybridClock::new(pc.clone());
    let device = DeviceInfo { id: pc.clone(), name: "Windows device".into(), platform: Platform::Windows, app_version: "0.4.0".into() };
    let mut q = Queue::default();
    q.set(vec![t(ProviderKind::Spotify, "4uLU6hMCjMI75M1A2tKUQC", "Song A", "Artist A"), t(ProviderKind::Spotify, "7GhIk7Il098yCjg4BQjzvb", "Song B", "Artist B")], 0);
    q.insert_manual(vec![t(ProviderKind::YouTube, "dQw4w9WgXcQ", "Song C", "Artist C")]);
    q.append_radio(vec![t(ProviderKind::Spotify, "3n3Ppam7vgaVa1iaRUc9Lp", "Song D", "Artist D")]);
    q.repeat = RepeatMode::Off;
    let seed = TrackRef::from_track(&q.tracks[0]);
    let qs = QueueState::from_queue(&q, Some(RadioSession { seed: Some(seed.clone()), original_seed: Some(seed.clone()), prefs: Default::default() }), c.tick(1_763_000_000_000));
    let listening = ListeningState { device: pc.clone(), track: Some(seed.clone()), position_ms: 61_000, duration_ms: 210_000, playing: true, updated_ms: 1_763_000_000_000, stamp: c.tick(1_763_000_000_000) };

    let mut lib = LibraryState::default();
    let mut mine = SyncPlaylist::new_local("Evening", &mut c, 1_763_000_001_000);
    mine.add(&seed, &mut c, 1_763_000_001_001);
    mine.add(&TrackRef::from_track(&t(ProviderKind::Local, r"C:\Music\x.flac", "Local Song", "Local Band")), &mut c, 1_763_000_001_002);
    lib.playlists.insert(mine.id.clone(), mine);
    let pl = Playlist { provider: ProviderKind::Spotify, id: "37i9dQZF1DXcBWIGoYBM5M".into(), name: "Imported".into(), snapshot: "snap1".into(), ..Default::default() };
    let imp = SyncPlaylist::from_provider(&pl, &[q.tracks[1].clone()], c.tick(1_763_000_002_000));
    lib.playlists.insert(imp.id.clone(), imp);
    lib.favorites.insert(seed.key(), seed.clone(), c.tick(1_763_000_003_000));
    let gone = TrackRef::from_track(&q.tracks[1]);
    lib.favorites.insert(gone.key(), gone.clone(), c.tick(1_763_000_003_001));
    lib.favorites.remove(&gone.key(), c.tick(1_763_000_003_002));
    lib.saved_recommendations.insert(seed.key(), seed.clone(), c.tick(1_763_000_003_003));
    let ev = aqua_core::history::PlayEvent { track: q.tracks[0].clone(), at: 1_762_999_000, listened_ms: 205_000, completed: true, skipped: false };
    lib.history.push(HistoryEntry::from_event(&ev, &c.tick(1_762_999_000_000)));

    let mut s = SettingsState::default();
    let st = c.tick(1_763_000_004_000);
    s.theme = Lww::new("Aqua Glass".into(), st.clone());
    s.radio = Lww::new(Default::default(), st.clone());
    s.playback = Lww::new(Default::default(), st.clone());
    s.sync = Lww::new(Default::default(), st);
    let mut env = Envelope::new(Kind::Snapshot, pc, Snapshot { device, listening: Some(listening), queue: Some(qs), library: lib, settings: s });
    env.created_ms = 1_763_000_005_000;
    env
}

#[test]
fn schema_v1_fixture_still_decodes_and_encodes_identically() {
    let now = sample();
    if std::env::var("AQUA_REGEN_FIXTURES").is_ok() {
        std::fs::write(FIXTURE, now.to_json_pretty() + "\n").unwrap();
    }
    let text = std::fs::read_to_string(FIXTURE).expect("fixture missing — run with AQUA_REGEN_FIXTURES=1 once");
    let old: Envelope<Snapshot> = decode(&text, Kind::Snapshot).expect("v1 data must stay readable");
    assert_eq!(old, now, "decoded fixture equals today's model");
    let a: serde_json::Value = serde_json::from_str(&text).unwrap();
    let b: serde_json::Value = serde_json::from_str(&now.to_json()).unwrap();
    assert_eq!(a, b, "field names/shapes unchanged");

    // the data means what it should
    let s = &old.payload;
    let q = s.queue.as_ref().unwrap();
    assert_eq!(q.entries.iter().map(|e| e.origin).collect::<Vec<_>>(), [EntryOrigin::Context, EntryOrigin::Context, EntryOrigin::Manual, EntryOrigin::Radio]);
    assert_eq!(s.library.favorites.len(), 1, "the unliked song stays removed");
    assert_eq!(s.library.playlists.len(), 2);
    assert!(!text.contains(r"C:\\Music"), "no local paths on the wire");
}
