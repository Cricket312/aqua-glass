//! Desktop side of the (future) cross-device sync model in `aqua_core::sync`.
//!
//! Nothing here runs in the background or touches the network. It converts
//! the desktop's own data (config, history, imported playlists, the saved
//! session) to the portable sync model, which is used by
//! `--export-sync-snapshot` and will be used by a future sync client.

use crate::config::{Config, ProviderKind};
use aqua_core::sync::*;

pub fn device_info() -> DeviceInfo {
    DeviceInfo::load_or_create(&crate::util::data_dir().join("device.json"), env!("CARGO_PKG_VERSION"))
}

/// Settings that may be shared. Device-specific settings (audio device,
/// window, UI scale, Windows integrations, RAM target, paths) are left out by
/// construction — `SettingsState` has no fields for them.
pub fn settings_state(cfg: &Config, clock: &mut HybridClock, now_ms: i64) -> SettingsState {
    let s = clock.tick(now_ms);
    SettingsState {
        theme: Lww::new(cfg.appearance.theme.clone(), s.clone()),
        radio: Lww::new(cfg.radio_prefs(), s.clone()),
        playback: Lww::new(
            PlaybackPrefs {
                preferred_provider: cfg.playback.preferred_provider,
                fade_ms: cfg.playback.fade_ms,
                gapless: cfg.on("gapless"),
                normalization: cfg.on("normalization"),
                seek_step_s: cfg.playback.seek_step_s,
            },
            s.clone(),
        ),
        sync: Lww::new(SyncPrefs::default(), s),
    }
}

/// Apply shared settings received from another device, honouring this
/// device's choices about what to accept.
pub fn apply_settings_state(cfg: &mut Config, s: &SettingsState, accept: &SyncPrefs) {
    if accept.theme && !s.theme.value.is_empty() {
        cfg.appearance.theme = s.theme.value.clone();
    }
    if accept.radio_prefs {
        cfg.set_radio_prefs(&s.radio.value);
    }
    if accept.playback_prefs {
        let p = &s.playback.value;
        cfg.playback.preferred_provider = p.preferred_provider;
        cfg.playback.fade_ms = p.fade_ms;
        cfg.playback.seek_step_s = p.seek_step_s;
        cfg.set("gapless", p.gapless);
        cfg.set("normalization", p.normalization);
    }
    cfg.sanitize();
}

/// Everything this PC could share, built from files on disk (works without
/// the UI running). Imported provider playlists are included as provider
/// references; Spotify/YouTube remain the owners.
pub fn snapshot(cfg: &Config) -> Envelope<Snapshot> {
    let device = device_info();
    let mut clock = HybridClock::new(device.id.clone());
    let now = aqua_core::storage::now_millis();
    let mut snap = Snapshot { device: device.clone(), ..Default::default() };

    // listening state + queue from the saved session
    if let Some(s) = std::fs::read(crate::util::data_dir().join("session.json")).ok().and_then(|b| aqua_core::session::SavedSession::decode(&b)) {
        let q = aqua_core::queue::Queue::from_entries(s.entries, Some(s.pos), cfg.playback.shuffle, cfg.playback.repeat);
        let stamp = clock.tick(now);
        if let Some(cur) = q.current() {
            snap.listening = Some(ListeningState { device: device.id.clone(), track: Some(TrackRef::from_track(cur)), position_ms: s.position_ms, duration_ms: cur.duration_ms, playing: false, updated_ms: now, stamp: stamp.clone() });
        }
        snap.queue = Some(QueueState::from_queue(&q, None, stamp));
    }

    // history
    let h = aqua_core::history::History::load();
    for e in &h.events {
        let st = clock.tick(e.at * 1000);
        snap.library.history.push(HistoryEntry::from_event(e, &st));
    }

    // imported provider playlists (references to provider-owned data)
    let mut lib = aqua_core::library::LibraryStore::open(crate::util::data_dir().join("library"));
    for p in [ProviderKind::Spotify, ProviderKind::YouTube] {
        let entries: Vec<_> = lib.lib(p).playlists.iter().filter(|e| e.imported && !e.removed_upstream).map(|e| (e.meta.clone(), e.last_synced)).collect();
        for (meta, synced) in entries {
            let tracks = lib.tracks(p, &meta.id).cloned().unwrap_or_default();
            let st = clock.tick(synced.max(0) * 1000);
            let sp = SyncPlaylist::from_provider(&meta, &tracks, st);
            snap.library.playlists.insert(sp.id.clone(), sp);
        }
        lib.evict_except(None);
        for a in lib.lib(p).albums.clone() {
            snap.library.saved_albums.insert(format!("{p:?}:{}", a.id), AlbumRef::from_album(&a), clock.tick(now));
        }
    }
    snap.settings = settings_state(cfg, &mut clock, now);
    Envelope::new(Kind::Snapshot, device.id, snap)
}

/// `aqua-glass.exe --export-sync-snapshot <file.json>`
pub fn export_snapshot_cli(path: Option<&str>) -> i32 {
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    let (cfg, _) = Config::load();
    let env = snapshot(&cfg);
    let path = path.map(std::path::PathBuf::from).unwrap_or_else(|| crate::util::data_dir().join(format!("sync-snapshot-{}.json", crate::util::now_unix())));
    let s = &env.payload;
    println!(
        "Aqua Glass sync snapshot (schema {}): {} playlists, {} saved albums, {} history entries, queue: {}, settings: theme “{}”",
        env.schema,
        s.library.playlists.len(),
        s.library.saved_albums.len(),
        s.library.history.len(),
        s.queue.as_ref().map(|q| q.entries.len().to_string()).unwrap_or_else(|| "none".into()),
        s.settings.theme.value
    );
    match crate::util::atomic_write(&path, env.to_json_pretty().as_bytes()) {
        Ok(()) => {
            println!("Saved to {} (contains no passwords, tokens or API keys)", path.display());
            0
        }
        Err(e) => {
            eprintln!("Could not save: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_through_sync_model_and_device_settings_stay_local() {
        let mut a = Config::default();
        a.appearance.theme = "Seafoam".into();
        a.audio.device = "Speakers (Realtek(R) Audio)".into();
        a.radio.strength = aqua_core::prefs::Strength::DeepDiscovery;
        a.radio.target = 20;
        a.set("radio_stay_close", true);
        a.playback.fade_ms = 3000;
        let mut c = HybridClock::new(DeviceId("pc".into()));
        let s = settings_state(&a, &mut c, 1000);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("Realtek"), "audio device must not be synced");
        let back: SettingsState = serde_json::from_str(&json).unwrap();

        let mut b = Config::default();
        b.audio.device = "Phone speaker".into();
        apply_settings_state(&mut b, &back, &SyncPrefs::default());
        assert_eq!(b.audio.device, "Phone speaker", "device-specific setting untouched");
        assert_eq!(b.radio.strength, aqua_core::prefs::Strength::DeepDiscovery);
        assert_eq!(b.radio.target, 20);
        assert!(b.on("radio_stay_close"));
        assert_eq!(b.playback.fade_ms, 3000);
        assert_ne!(b.appearance.theme, "Seafoam", "theme sync is opt-in");
        apply_settings_state(&mut b, &back, &SyncPrefs { theme: true, ..Default::default() });
        assert_eq!(b.appearance.theme, "Seafoam");
        assert_eq!(b.radio_prefs(), a.radio_prefs());
    }

    #[test]
    fn snapshot_from_disk_has_no_secrets() {
        crate::security::set(crate::security::keys::LASTFM_API_KEY, "0123456789abcdef0123456789abcdef").unwrap();
        let env = snapshot(&Config::default());
        let j = env.to_json();
        assert!(!j.contains("0123456789abcdef0123456789abcdef"));
        let back: Envelope<Snapshot> = decode(&j, Kind::Snapshot).unwrap();
        assert_eq!(back.payload.device.id, env.payload.device.id);
        crate::security::delete(crate::security::keys::LASTFM_API_KEY);
    }
}
