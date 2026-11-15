use super::*;
use crate::model::{Playlist, ProviderKind, RepeatMode, Track};
use crate::queue::{EntryOrigin, Queue};

fn dev(n: &str) -> DeviceId {
    DeviceId(n.into())
}
fn tr(p: ProviderKind, id: &str, title: &str) -> Track {
    Track { provider: p, id: id.into(), title: title.into(), artists: vec!["Artist".into()], duration_ms: 200_000, ..Default::default() }
}
fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap()
}

#[test]
fn envelope_roundtrip_and_version_checks() {
    let mut c = HybridClock::new(dev("pc"));
    let st = crate::playback::PlayerState { current: Some(tr(ProviderKind::Spotify, "abc", "Song")), playing: true, duration_ms: 200_000, ..Default::default() };
    let ls = ListeningState::from_player(&st, 61_000, c.tick(1_000_000));
    let env = Envelope::new(Kind::Listening, dev("pc"), ls.clone());
    let back: Envelope<ListeningState> = decode(&env.to_json(), Kind::Listening).unwrap();
    assert_eq!(back.payload, ls);
    assert_eq!(back.schema, SCHEMA_VERSION);

    // newer schema: refuse instead of guessing
    let mut v: serde_json::Value = serde_json::from_str(&env.to_json()).unwrap();
    v["schema"] = serde_json::json!(SCHEMA_VERSION + 1);
    assert!(matches!(decode::<ListeningState>(&v.to_string(), Kind::Listening), Err(SyncError::NewerSchema { .. })));
    // wrong kind / malformed
    assert!(matches!(decode::<ListeningState>(&env.to_json(), Kind::Queue), Err(SyncError::WrongKind { .. })));
    assert!(matches!(decode::<ListeningState>("{", Kind::Listening), Err(SyncError::Malformed(_))));
    // unknown (future, additive) fields are ignored
    v["schema"] = serde_json::json!(SCHEMA_VERSION);
    v["payload"]["some_future_field"] = serde_json::json!(42);
    assert!(decode::<ListeningState>(&v.to_string(), Kind::Listening).is_ok());
}

#[test]
fn queue_state_preserves_order_origins_and_radio_seed() {
    let mut q = Queue::default();
    q.set(vec![tr(ProviderKind::Spotify, "c1", "Ctx 1"), tr(ProviderKind::Spotify, "c2", "Ctx 2")], 0);
    q.append_radio(vec![tr(ProviderKind::Spotify, "r1", "Radio 1"), tr(ProviderKind::YouTube, "r2", "Radio 2")]);
    q.insert_manual(vec![tr(ProviderKind::Spotify, "m1", "Manual 1")]);
    q.repeat = RepeatMode::All;
    let before: Vec<(String, EntryOrigin)> = q.in_order().into_iter().map(|t| t.id).zip(q.origins_in_order()).collect();
    assert_eq!(before.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["c1", "c2", "m1", "r1", "r2"]);

    let seed = TrackRef::from_track(&tr(ProviderKind::Spotify, "c1", "Ctx 1"));
    let radio = RadioSession { seed: Some(seed.clone()), original_seed: Some(seed), prefs: Default::default() };
    let mut c = HybridClock::new(dev("pc"));
    let qs = QueueState::from_queue(&q, Some(radio), c.tick(5));
    let env = Envelope::new(Kind::Queue, dev("pc"), qs.clone());
    let back: QueueState = decode(&env.to_json(), Kind::Queue).unwrap().payload;
    assert_eq!(back, qs);
    assert_eq!(back.radio.as_ref().unwrap().prefs.target, 15);
    assert_eq!(back.radio.as_ref().unwrap().prefs.threshold, 5);

    let mut restored = back.to_queue();
    let after: Vec<(String, EntryOrigin)> = restored.in_order().into_iter().map(|t| t.id).zip(restored.origins_in_order()).collect();
    assert_eq!(after, before);
    assert_eq!(restored.pos, Some(0));
    assert_eq!(restored.repeat, RepeatMode::All);
    assert_eq!(restored.radio_upcoming(), 2);
    // manual items keep priority over radio items after the round trip
    restored.insert_manual(vec![tr(ProviderKind::Spotify, "m2", "Manual 2")]);
    let ids: Vec<String> = restored.in_order().into_iter().map(|t| t.id).collect();
    assert_eq!(ids, ["c1", "c2", "m1", "m2", "r1", "r2"]);
    assert_eq!(back.upcoming().len(), 4);
}

#[test]
fn queue_state_from_player_matches_engine_view() {
    let st = crate::playback::PlayerState {
        queue: vec![tr(ProviderKind::Spotify, "a", "A"), tr(ProviderKind::Spotify, "b", "B")],
        queue_origin: vec![EntryOrigin::Manual, EntryOrigin::Radio],
        queue_pos: Some(0),
        shuffle: true,
        ..Default::default()
    };
    let qs = QueueState::from_player(&st, None, HybridClock::new(dev("x")).tick(1));
    assert_eq!(qs.entries[1].origin, EntryOrigin::Radio);
    assert!(qs.shuffle);
}

#[test]
fn local_tracks_never_leak_file_paths() {
    let t = Track { provider: ProviderKind::Local, id: r"C:\Users\someone\Music\a.flac".into(), title: "Song (Remastered)".into(), artists: vec!["Band".into()], ..Default::default() };
    let r = TrackRef::from_track(&t);
    assert!(!json(&r).contains("someone"));
    assert!(!r.is_portable());
    assert_eq!(r.id, crate::history::track_ident(&t));
    assert_eq!(TrackRef::from_track(&tr(ProviderKind::Spotify, "id1", "x")).key(), tr(ProviderKind::Spotify, "id1", "x").key());
}

#[test]
fn like_unlike_conflict_is_deterministic() {
    let song = TrackRef::from_track(&tr(ProviderKind::Spotify, "s", "S"));
    let mut pc = HybridClock::new(dev("pc"));
    let mut phone = HybridClock::new(dev("phone"));
    let mut a = LibraryState::default();
    let mut b = LibraryState::default();
    a.favorites.insert(song.key(), song.clone(), pc.tick(1000)); // liked on PC
    b.favorites.remove(&song.key(), phone.tick(2000)); // unliked later on the phone
    let mut ab = a.clone();
    ab.merge(&b);
    let mut ba = b.clone();
    ba.merge(&a);
    assert_eq!(json(&ab), json(&ba), "same result in either order");
    assert!(!ab.favorites.contains(&song.key()), "the later action (unlike) wins");
    // an old replica doesn't resurrect it
    ab.merge(&a);
    assert!(!ab.favorites.contains(&song.key()));
    // same millisecond: the device id breaks the tie, still deterministically
    let mut x = LibraryState::default();
    let mut y = LibraryState::default();
    x.favorites.insert(song.key(), song.clone(), HybridClock::new(dev("a")).tick(5));
    y.favorites.remove(&song.key(), HybridClock::new(dev("b")).tick(5));
    let mut xy = x.clone();
    xy.merge(&y);
    let mut yx = y.clone();
    yx.merge(&x);
    assert_eq!(json(&xy), json(&yx));
}

#[test]
fn playlist_edited_on_two_devices_merges_without_loss() {
    let mut pc = HybridClock::new(dev("pc"));
    let mut phone = HybridClock::new(dev("phone"));
    let mut base = SyncPlaylist::new_local("Road trip", &mut pc, 1000);
    let t = |i: &str| TrackRef::from_track(&tr(ProviderKind::Spotify, i, i));
    let first = base.add(&t("one"), &mut pc, 1001).unwrap();
    base.add(&t("two"), &mut pc, 1002);

    // offline edits on both devices
    let mut a = base.clone();
    let mut b = base.clone();
    a.add(&t("pc-add"), &mut pc, 2000);
    a.rename("Road trip 2026", &mut pc, 2001);
    b.add(&t("phone-add"), &mut phone, 2100);
    b.remove(&first, &mut phone, 2101);
    b.rename("Summer road trip", &mut phone, 2102);

    let mut ab = a.clone();
    ab.merge(&b);
    let mut ba = b.clone();
    ba.merge(&a);
    assert_eq!(json(&ab), json(&ba), "merge is commutative");
    let ids: Vec<String> = ab.tracks().into_iter().map(|t| t.id).collect();
    assert_eq!(ids, ["two", "pc-add", "phone-add"], "both additions kept, removal applied");
    assert_eq!(ab.name.value, "Summer road trip", "later rename wins");
    // idempotent, and an old copy doesn't bring back the removed item
    let snapshot = json(&ab);
    ab.merge(&a);
    ab.merge(&base);
    assert_eq!(json(&ab), snapshot);

    // moves
    let pc_item = ab.ordered().into_iter().find(|(_, i)| i.track.id == "pc-add").unwrap().0;
    assert!(ab.move_to(&pc_item, 0, &mut pc, 3000));
    let ids: Vec<String> = ab.tracks().into_iter().map(|t| t.id).collect();
    assert_eq!(ids, ["pc-add", "two", "phone-add"]);
}

#[test]
fn importing_the_same_provider_playlist_twice_does_not_duplicate() {
    let pl = Playlist { provider: ProviderKind::Spotify, id: "37i9".into(), name: "Mix".into(), snapshot: "s1".into(), ..Default::default() };
    let tracks = vec![tr(ProviderKind::Spotify, "a", "A")];
    let pc = SyncPlaylist::from_provider(&pl, &tracks, HybridClock::new(dev("pc")).tick(10));
    let mut pl2 = pl.clone();
    pl2.snapshot = "s2".into();
    let phone = SyncPlaylist::from_provider(&pl2, &[tracks[0].clone(), tr(ProviderKind::Spotify, "b", "B")], HybridClock::new(dev("phone")).tick(20));
    let mut a = LibraryState::default();
    a.playlists.insert(pc.id.clone(), pc.clone());
    let mut b = LibraryState::default();
    b.playlists.insert(phone.id.clone(), phone.clone());
    a.merge(&b);
    assert_eq!(a.playlists.len(), 1);
    let merged = a.playlists.values().next().unwrap();
    assert_eq!(merged.tracks().len(), 2, "newest provider listing wins");
    // provider-owned playlists can't be edited through sync
    let mut c = HybridClock::new(dev("pc"));
    let mut m = merged.clone();
    assert!(!m.editable());
    assert!(m.add(&TrackRef::default(), &mut c, 30).is_none());
}

#[test]
fn reconnecting_after_days_offline_merges_history() {
    let mut pc = HybridClock::new(dev("pc"));
    let mut phone = HybridClock::new(dev("phone"));
    let ev = |t: &str, at: i64| crate::history::PlayEvent { track: tr(ProviderKind::Spotify, t, t), at, listened_ms: 100_000, completed: true, skipped: false };
    let mut server = LibraryState::default();
    server.history.push(HistoryEntry::from_event(&ev("x", 100), &pc.tick(100_000)));
    // phone offline for 5 days, listens to things
    let mut offline = server.clone();
    for d in 1..=5 {
        offline.history.push(HistoryEntry::from_event(&ev(&format!("p{d}"), 100 + d * 86400), &phone.tick((100 + d * 86400) * 1000)));
    }
    // meanwhile the PC kept listening
    server.history.push(HistoryEntry::from_event(&ev("y", 200), &pc.tick(200_000)));
    let mut merged = server.clone();
    merged.merge(&offline);
    assert_eq!(merged.history.len(), 7);
    assert!(merged.history.windows(2).all(|w| w[0].at <= w[1].at), "chronological");
    let mut again = merged.clone();
    again.merge(&offline);
    again.merge(&server);
    assert_eq!(again, merged, "re-sending is harmless");
}

#[test]
fn clock_orders_after_remote_and_ignores_absurd_skew() {
    let mut c = HybridClock::new(dev("slow"));
    let remote = Stamp { ms: 10_000, ctr: 3, device: dev("fast") };
    c.observe(&remote, 9_000);
    assert!(c.tick(9_000) > remote, "local change after seeing remote orders after it");
    let bogus = Stamp { ms: 9_000 + 24 * 3600 * 1000, ctr: 0, device: dev("broken") };
    assert!(!bogus.plausible(9_000));
    let before = c.tick(9_001);
    c.observe(&bogus, 9_001);
    assert!(c.tick(9_002).ms < bogus.ms);
    assert!(before.ms < bogus.ms);
}

#[test]
fn settings_payload_has_no_device_specific_fields_or_secrets() {
    let mut c = HybridClock::new(dev("pc"));
    let s = SettingsState { theme: Lww::new("Aqua Night".into(), c.tick(1)), ..Default::default() };
    let j = json(&s);
    for banned in ["output_device", "audio_device", "token", "api_key", "password", "window", "ui_scale", "client_secret"] {
        assert!(!j.contains(banned), "{banned} in {j}");
    }
    let mut other = SettingsState::default();
    other.radio.set(crate::prefs::RadioPrefs { strength: crate::prefs::Strength::Adventurous, ..Default::default() }, c.tick(2));
    let mut m = s.clone();
    m.merge(&other);
    assert_eq!(m.theme.value, "Aqua Night");
    assert_eq!(m.radio.value.strength, crate::prefs::Strength::Adventurous);
}

#[test]
fn handoff_plans() {
    let pc = DeviceInfo { id: dev("pc"), name: "Desk PC".into(), platform: crate::platform::Platform::Windows, app_version: "0.4.0".into() };
    let mut c = HybridClock::new(dev("pc"));
    let st = crate::playback::PlayerState { current: Some(tr(ProviderKind::Spotify, "sp1", "Song")), playing: true, duration_ms: 200_000, ..Default::default() };
    let listening = ListeningState::from_player(&st, 60_000, c.tick(1_000_000));
    let offer = HandoffOffer { from: pc.clone(), listening: listening.clone(), queue: None };

    // phone signed in to Spotify: resume ~10 s later in the song
    let caps = Capabilities { playable: vec![ProviderKind::Spotify], ..Default::default() };
    let p = plan_handoff(&offer, &caps, 1_010_000);
    assert_eq!(p.action, HandoffAction::Resume { track: listening.track.clone().unwrap(), position_ms: 70_000 });
    assert!(p.pause_source);

    // phone only has local files: match instead of pretending
    let caps = Capabilities { local_library: true, ..Default::default() };
    assert!(matches!(plan_handoff(&offer, &caps, 1_010_000).action, HandoffAction::ResumeMatched { ref search, .. } if search == &vec![ProviderKind::Local]));

    // nothing usable
    assert!(matches!(plan_handoff(&offer, &Capabilities::default(), 1_010_000).action, HandoffAction::Unavailable { .. }));

    // stale offers resume from the start; paused state doesn't advance
    let p = plan_handoff(&offer, &Capabilities { playable: vec![ProviderKind::Spotify], ..Default::default() }, 1_000_000 + 7 * 3600 * 1000);
    assert!(matches!(p.action, HandoffAction::Resume { position_ms: 0, .. }));
    let mut paused = offer.clone();
    paused.listening.playing = false;
    assert!(matches!(plan_handoff(&paused, &Capabilities { playable: vec![ProviderKind::Spotify], ..Default::default() }, 1_100_000).action, HandoffAction::Resume { position_ms: 60_000, .. }));

    // a legacy YouTube track (pre-0.6 data) is matched on a provider this device has
    let yt = crate::playback::PlayerState { current: Some(tr(ProviderKind::YouTube, "vid", "Clip")), playing: true, duration_ms: 300_000, ..Default::default() };
    let offer = HandoffOffer { from: pc, listening: ListeningState::from_player(&yt, 95_000, c.tick(2_000_000)), queue: None };
    assert!(matches!(plan_handoff(&offer, &Capabilities { playable: vec![ProviderKind::Spotify], ..Default::default() }, 2_000_000).action, HandoffAction::ResumeMatched { position_ms: 95_000, .. }));
    // nothing playing
    let mut empty = offer.clone();
    empty.listening.track = None;
    assert!(matches!(plan_handoff(&empty, &Capabilities::default(), 0).action, HandoffAction::Unavailable { .. }));
}

#[test]
fn outbox_coalesces_and_acks() {
    let mut c = HybridClock::new(dev("pc"));
    let mut o = Outbox::default();
    let st = crate::playback::PlayerState::default();
    o.push(Change::Listening(ListeningState::from_player(&st, 0, c.tick(1))));
    o.push(Change::Listening(ListeningState::from_player(&st, 5, c.tick(2))));
    let ev = crate::history::PlayEvent { track: tr(ProviderKind::Spotify, "a", "A"), at: 1, listened_ms: 1, completed: true, skipped: false };
    let s3 = o.push(Change::History(HistoryEntry::from_event(&ev, &c.tick(3))));
    o.push(Change::History(HistoryEntry::from_event(&ev, &c.tick(4))));
    assert_eq!(o.pending.len(), 3, "only the latest listening state is kept");
    o.ack_through(s3);
    assert_eq!(o.pending.len(), 1);
    let back: Outbox = serde_json::from_str(&json(&o)).unwrap();
    assert_eq!(back, o);
}

#[test]
fn device_identity_is_stable_and_anonymous() {
    let dir = std::env::temp_dir().join(format!("aqua-dev-{}", std::process::id()));
    let p = dir.join("device.json");
    let a = DeviceInfo::load_or_create(&p, "0.4.0");
    let b = DeviceInfo::load_or_create(&p, "0.4.1");
    assert_eq!(a.id, b.id);
    assert_eq!(a.id.0.len(), 32);
    assert_eq!(b.app_version, "0.4.1");
    let _ = std::fs::remove_dir_all(dir);
}
