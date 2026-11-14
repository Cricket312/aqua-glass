# Changelog

## 0.1.0 — 2026-11-14 (first build)

### Added
- Native egui/eframe (OpenGL) application shell: Home, Search, Library, Playlists, Artists,
  Albums, Now Playing, Queue, Playlist Transfer, Settings; persistent playback bar;
  service filter; application menu; toasts; virtualized track lists.
- Playback engine on its own thread with one shared audio pipeline (cpal/WASAPI):
  queue, shuffle, repeat (off/all/one), seek, gapless hand-off, fades, volume/mute,
  device selection and automatic recovery when a device disappears.
- Spotify: OAuth PKCE sign-in, refresh-token storage in Credential Manager, profile,
  playlists (2026 `items` and legacy `tracks` formats), Liked Songs, saved albums,
  followed artists, recently played, search, album/artist pages, like, playlist creation;
  Premium playback through librespot 0.8 with a custom sink.
- YouTube Music via YouTube Data API v3: OAuth with the user's Google Desktop client,
  playlists, liked music, search, durations, like, playlist creation/insertion,
  revocation on disconnect; playback opens YouTube Music/YouTube in the browser.
- Local files: MP3/FLAC/Ogg/WAV/AAC playback (symphonia), tag scan, albums, ReplayGain,
  command-line / "Open with" support.
- Library import (all / selected / liked / albums / artists), manual and automatic sync
  using snapshot ids, rate-limit handling, conflict detection (removed upstream, large
  shrink) with backups; strictly read-only towards the services.
- Playlist Transfer: matching with ISRC/title/artist/album/duration and version
  detection (live, remix, remaster, acoustic, cover, clean/explicit…), preview with
  matched/ambiguous/unmatched counts, manual replacement, quota warning for YouTube,
  source backups, logs, removal of created playlists; JSON/CSV/M3U export and restore.
- Settings: 17 categories, search, 64 individually toggleable features each with RAM
  rating, estimate vs. on-device measurement, dependencies and reset; presets (Minimal,
  Balanced, Enhanced, Fully Customized, Custom, user presets with export/import);
  Disable All Visual Customizations + restore; export/import/reset of settings;
  versioned TOML with migration.
- Performance dashboard: private working set, peak, private bytes, CPU, caches, FPS,
  background tasks, RAM budget bar and history; configurable soft RAM target (default
  200 MB); RAM warnings; automatic memory optimizer.
- Theme engine (declarative TOML) with 8 built-in themes and live editor (save, duplicate,
  rename, export, import, reset); custom fonts; translucent panels; Windows 11 acrylic.
- Backgrounds: custom image, album art, blur, dimming, opacity, scaling modes,
  animated gradient, parallax, cross-fade transitions.
- Lyrics from LRCLIB: synced/unsynced, word-by-word, offset, fullscreen, mini-player and
  overlay line, prefetch, styling options.
- Audio: 10-band EQ with presets, bass boost, balance, pre-amp, normalization, buffer size,
  streaming quality.
- Visualizer: bars, waveform, circular, minimal; FPS, sensitivity, smoothing, colors, size,
  artwork colors, reduced motion, player-bar spectrum; FFT only while visible.
- Discord Rich Presence with all privacy sub-options and rate limiting.
- Mini-player (3 layouts, opacity, always on top, remembers geometry) and desktop overlay
  (click-through, opacity, position, font size).
- Shortcuts editor with conflict detection, global hotkeys, media keys/SMTC, tray menu,
  minimize/close to tray, start with Windows, toast notifications.
- Tests (unit + audio integration), memory benchmark and soak scripts, docs.

### Fixed during development
- Gapless hand-off re-queued a track when a track was shorter than the output buffer.
- Overlay/mini-player forced continuous repaints while the main window was visible.
- Startup crash when the player-bar visualizer drew before its first FFT frame.
- Missing glyphs with egui 0.37 default fonts (enabled monochrome emoji fonts).
- Saved shuffle/repeat state was not applied to the engine at startup.
