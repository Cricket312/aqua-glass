# Changelog

## 0.4.1 — 2026-11-15 (crash diagnostics)

### Fixed
- With your own Spotify Client ID, the saved Spotify sign-in was not restored at startup:
  the app only looked for the default-app sign-in.

### Added
- **Crash reports.** Release builds used to close silently on an internal error. Now the
  error, where it happened and the thread are written to `crash.log` in the data folder,
  and a message box shows them. Unhandled Windows exceptions (access violation, stack
  overflow) are logged too. On the next start the app mentions the report once and keeps
  it as `crash-last.log`.
- `--safe-mode`: start without restoring the Spotify/YouTube sessions and the saved queue.
- `--debug-log`: diagnostic logging to `aqua-glass.log` for this run, even when the
  "Diagnostic Logging" setting can't be reached.
- `--crash-test`: triggers a deliberate crash to check that reporting works.

## 0.4.0 — 2026-11-14 (architecture preparation for mobile and cross-device sync)

No new user-facing features and no change in behaviour, except that a restored queue now
remembers which songs were added manually and which were Similarity Radio picks. Nothing
new runs in the background; there is no sync server, sync client or device discovery.

### Changed
- **Cargo workspace.** Portable code moved into crates the Windows app re-exports:
  - `aqua-core`: models, queue, history, library, matching, playback interface, sync
    models. It has no UI, audio or OS dependencies and compiles for `aarch64-linux-android`.
  - `aqua-providers`: Spotify, YouTube, Last.fm, lyrics, OAuth and the rate limiter.
  - `aqua-radio`: Similarity Radio.
- Platform hooks: secret store (Windows Credential Manager adapter), data directory and
  browser launcher are installed by the desktop app at startup.
- `PlaybackControl` trait: the desktop engine implements it, and Similarity Radio now
  drives playback only through it.
- Queue entries carry their origin (context / manual / radio). The saved session (v2)
  keeps the origin; v1 session files are still read.
- Radio preferences have a portable form (`RadioPrefs`) shared by the desktop config and
  the runtime radio settings.

### Added
- `aqua_core::sync`: versioned (schema 1), serde-serialisable models for:
  - listening state, queue state and library state (playlists, favourites, saved albums,
    saved recommendations, history);
  - shareable settings and device identity;
  - playback-handoff offers and planning;
  - an offline outbox.

  Merges are deterministic (hybrid logical clocks, last-writer-wins with tombstones,
  item-level playlist merge). Payloads contain no secrets and no local file paths.
- `aqua-glass.exe --export-sync-snapshot [file]`: writes this PC's data in the sync format.
  It runs once and exits, without any network access.
- `scripts/check-portable.sh`, a frozen schema-1 compatibility fixture, and tests for
  serialization, queue-state preservation, conflict handling and handoff.
- Docs: `docs/future-mobile-architecture.md` and `docs/sync-server.md` (a proposed
  self-hosted server; not implemented).

## 0.3.2 — 2026-11-14 (Spotify rate limiting)

### Fixed
- **One shared limiter per service** (`services/ratelimit.rs`): request pacing (Spotify ≈ 3
  req/s, small burst) and a single global cooldown. A 429 now pauses *all* Spotify requests
  for exactly the `Retry-After` time (exponential back-off when absent). Before, each thread
  retried on its own and long waits were capped at 30 s, which tends to prolong Spotify's
  throttling.
- Background work (sync, radio, transfer) yields to interactive actions and waits out
  cooldowns; interactive actions fail fast with "requests resume in N s" instead of freezing.
- Fewer requests:
  - Liked Songs / saved albums / followed artists are re-downloaded only when a
    1-request check shows a change.
  - Similarity Radio is limited to 12 Spotify searches per refill, and remembers lookups
    (hits and misses) across refills.
  - Identical searches are cached for 5 minutes.
  - Automatic sync now runs at most every 15 minutes.
  - A sync that hits a long rate limit pauses and continues next time; transfer matching
    stops instead of marking every remaining song "unmatched".
- **Own Spotify app for the Web API**: with your own Client ID, library/search/sync use your
  app's private quota while Premium playback keeps using librespot (two sign-ins, once). The
  setup steps are now shown prominently in Settings › Music Services, with live rate-limit
  status, and the sidebar shows "Spotify paused N s" during a cooldown.

## 0.3.1 — 2026-11-14

### Fixed
- **Clipboard**: eframe's `clipboard` feature had been disabled together with other default
  features, so "Copy report" (and any copy from Aqua Glass) only reached an internal
  clipboard, and Ctrl+V could not paste text copied from other apps (e.g. a Last.fm API key).
  System clipboard support is re-enabled; verified on Linux/X11 and with the Windows build under Wine.
- "Copy report" now confirms with a notification; added "Open report file".
- Notifications no longer show file paths as `<redacted>` (the token scrubber treated long
  paths as secrets).
- The self-test report area shows the whole report instead of ~4 lines.
- The key-storage message no longer claims Windows Credential Manager on non-Windows dev builds.

## 0.3.0 — 2026-11-14 (reference design + Last.fm live testing)

### Changed — matched to the supplied Aqua Glass concept image
- Sidebar: pill-shaped active item (seafoam tint + border), larger rows, divider before
  Settings, RAM Usage card (chip icon, MB / target, seafoam bar, opens Performance).
  The playlist list in the sidebar is now optional ("Playlists in Sidebar", off by default).
- Top bar: back and **forward** buttons; centred search pill with a filter button (service
  filter + account status) replacing the chips.
- Home: large greeting, featured banner (blurred cover backdrop, sharp cover, Play pill,
  import, pager dots — built only from your own playlists/top mix), art-first cards without
  boxes, landscape cards for Made For You / Top Mixes, "Show all ›".
- Bottom player: seafoam circular play/pause, frameless icons, expand (Now Playing) button.
- Right panel: rounded card with a Queue / Similarity Radio / Lyrics segmented control;
  Now Playing row with like; "Up Next (n)" with shuffle/repeat; artwork rows with durations.
- New optional "Subtle Background Gradient" (static, no textures or repaints; on in
  Balanced and above).

### Added
- `radio::self_test` live check of Similarity Radio against the real Last.fm API, available
  as `aqua-glass.exe --radio-selftest ["Artist" "Title"]` (prints in the terminal, saves a
  report, exit code), as a **Run self-test** button in Settings › Playback › Similarity
  Radio, and as ignored `cargo test --test lastfm_live` tests. Guide: docs/TESTING-LASTFM.md.

## 0.2.0 — 2026-11-14 (redesign + Similarity Radio)

### Changed — interface redesign
- New default palette: dark aqua surfaces (#0B1517 / #101D1F / #172729 / #203436) with
  seafoam (#62D6B2) and aqua (#36B8AC) accents used only for active navigation, progress,
  enabled switches, selected controls, radio indicators and the RAM bar.
- Collapsible icon sidebar with understated branding (small droplet + small name; the
  droplet in collapsed mode is optional), navigation incl. new **Discover** page, a compact
  list of your playlists, and the compact RAM indicator (usage / target / state bar).
- Top bar with a prominent search field, back button and provider filter chips.
- Home: greeting, quick tiles, and card rows (Recently Played, Favorite Playlists, Made For
  You, Recently Added, Your Top Mixes, Discover New Music, Recently Imported Spotify
  Playlists, Local Music) — each shown only when real data exists; 2–8 cards per row
  adapting to the window, sidebar and right panel; hover play buttons.
- Compact bottom player: artwork/title/artist/like · controls + progress · queue,
  lyrics, output device, volume, mini-player, quick settings.
- Optional right panel (Queue & Radio / Lyrics / Now Playing), closed by default.
- Search results with artwork rows; Now Playing centred on large artwork.
- Settings: compact category navigation, grouped cards, switch toggles, RAM badges kept on
  every optional feature; Performance page with summary cards and a collapsible
  "Detailed Diagnostics" section; measured costs listed separately from estimates.
- No social / Friend Activity features exist (none were ever added).

### Added
- **Automatic Similarity Radio**: playing a single song from search starts radio with that
  song as seed (configurable: also from artist/album pages); ~15 recommendations, refilled
  below 5; endless mode; four discovery strengths (Familiar, Balanced, Adventurous, Deep
  Discovery) that use the Last.fm similarity graph differently; rules for unheard songs,
  recently played, duplicates, repeated artists, learning from likes/skips, staying close to
  the original seed; preferred provider. Recommendations are only queued when they resolve
  to a playable track (local library, or Spotify with Premium playback); manual queue items
  always play first; radio never replaces playlist/album playback. Without a Last.fm API key
  it falls back to same-artist/same-album songs and says so. Panel controls: on/off,
  Refresh, More Like This, Stay close, Save queue as playlist.
- Local listening history (500 plays, stays on the PC) powering Recently Played, Top Mixes
  and the radio rules; clearable.
- Embedded cover art for local files.
- Output device menu in the player bar.

### Fixed
- Placeholder artwork glyph was rasterised at up to ~180 px, growing the font atlas; capped
  (Now Playing private memory 50 → 43 MB).
- Similarity rule refinement: three skips drop an artist for the session, but never the
  seed's own artist (found by the end-to-end radio test).

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
