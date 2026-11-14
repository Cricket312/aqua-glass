# Aqua Glass

A native, lightweight music client for Windows 11 written in Rust (egui/eframe + OpenGL).
It combines your **Spotify** library (with Premium playback), your **YouTube Music**
library (via the official YouTube Data API) and **local files** in one interface, with a
searchable settings menu where every optional feature can be switched off and shows its
RAM impact.

* No Electron, no Chromium, no embedded browser, no helper processes.
* Measured: **~41–42 MB private memory idle / during playback, ~43–45 MB with the richest
  presets**, Similarity Radio ≈ +0.1 MB (Linux development environment, see
  [Benchmarks](docs/BENCHMARKS.md)). The default soft target is 200 MB.
* **Automatic Similarity Radio**: play one song from search and similar songs keep coming.
* No telemetry, no analytics, no ads. Tokens live in Windows Credential Manager.

> **Status — please read.** Everything listed under "Verified" below was compiled, run and
> tested in this repository's development environment (Linux, virtual display, null audio
> sink). The Windows `.exe` is cross-compiled from Linux and was smoke-tested under
> **Wine 9** (UI, portable mode, tray icon, global hotkeys, local playback via WASAPI,
> auto-advance, command-line files) but has **not yet been run on a real Windows 11
> machine**; Spotify and YouTube sign-in/playback have **not been tested
> against the live services** (no accounts or network access to those services were
> available while building). See [Known limitations](#known-limitations).

---

![Home](docs/screenshot-home.png)

![Similarity Radio](docs/screenshot-radio.png)

## Contents

1. [Installing](#installing)
2. [Connecting your services](#connecting-your-services)
3. [What works with which service](#what-works-with-which-service)
4. [Using Aqua Glass](#using-aqua-glass)
5. [Similarity Radio](#similarity-radio)
6. [Settings, RAM ratings and presets](#settings-ram-ratings-and-presets)
7. [Privacy and security](#privacy-and-security)
8. [Building from source](#building-from-source)
9. [Testing and benchmarking](#testing-and-benchmarking)
10. [Architecture](#architecture)
11. [Known limitations](#known-limitations)

Further reading: [Configuration reference](docs/CONFIGURATION.md) · [Testing Similarity Radio with Last.fm](docs/TESTING-LASTFM.md) ·
[Benchmarks](docs/BENCHMARKS.md) · [Changelog](CHANGELOG.md)

---

## Installing

Aqua Glass is a single portable executable — no installer, no admin rights.

1. Unzip `aqua-glass-windows-x64.zip` anywhere (e.g. `C:\Users\you\Apps\AquaGlass`).
2. Run `aqua-glass.exe`.

Data (settings, library cache, themes, transfer backups) is stored in
`%APPDATA%\AquaGlass`. For a fully portable setup, create an empty file named
`portable.txt` next to the exe; data then goes to a `data` folder beside it.

Command line:

| Argument | Effect |
|---|---|
| `aqua-glass.exe song.mp3 other.flac` | Play these files (works with "Open with…") |
| `--minimized` | Start in the tray (or minimized if the tray is off); used by "Start with Windows" |
| `--page <home\|search\|library\|now-playing\|queue\|transfer\|settings\|performance>` | Start page |

## Connecting your services

### Spotify

Settings → Music Services → **Sign in with Spotify**. Your browser opens Spotify's normal
login page (OAuth 2.0 with PKCE, redirect to `http://127.0.0.1:8898/login`). No passwords or
cookies are ever entered into Aqua Glass.

* **Playback requires Spotify Premium** — this is enforced by Spotify. Library import,
  synchronization, search, export and transfer also work with Free accounts.
* Playback is provided by [librespot](https://github.com/librespot-org/librespot), the
  open-source Spotify Connect library used by spotifyd, ncspot and others. Like those
  projects, Aqua Glass signs in with librespot's public client id by default so that
  one login covers both the Web API (library) and playback. librespot is unofficial;
  Spotify's developer terms restrict third-party streaming clients, so use it at your
  own discretion. There is no DRM circumvention in Aqua Glass itself.
* **Rate limiting.** Spotify limits Web API requests *per app*. By default Aqua Glass uses
  librespot's public app, whose quota is shared with every librespot-based player, so heavy
  use elsewhere can get it throttled. Aqua Glass:
  * paces its requests;
  * pauses all Spotify calls for exactly as long as Spotify asks (`Retry-After`), showing
    "Spotify paused N s" in the sidebar;
  * skips unchanged playlists and collections when syncing, and caches lookups.

  If you still see throttling, **use your own Spotify app** (next point): library, search
  and sync then use your private quota. Playback isn't affected by Web API rate limits.
* Optional, recommended: use your own Spotify Developer app (Settings › Music Services ›
  "Use your own Spotify app"). Aqua Glass then signs in twice once: your app for the Web
  API, librespot's for playback.
  Register `http://127.0.0.1:8898/login` as redirect URI. Since **February 2026** Spotify
  Development-Mode apps need a Premium owner, are limited to 5 users, and have a reduced
  endpoint set (e.g. playlist `tracks` renamed to `items`; some catalog endpoints
  removed). Aqua Glass understands both the old and new response formats. librespot
  playback may reject tokens from custom client ids.
* Disconnecting deletes the stored refresh token. Spotify has no API to revoke a grant;
  remove the app at <https://www.spotify.com/account/apps/> to revoke it fully.

### YouTube Music

Google does not offer a public YouTube *Music* API. Aqua Glass uses the official
**YouTube Data API v3**, which requires a (free) Google Cloud OAuth client that you create
once:

1. <https://console.cloud.google.com/> → create a project → *APIs & Services* → enable
   **YouTube Data API v3**.
2. *OAuth consent screen*: External; add your Google account as a test user.
3. *Credentials* → *Create credentials* → *OAuth client ID* → type **Desktop app**.
4. In Aqua Glass: Settings → Music Services → paste the client ID, store the client
   secret (it goes to Credential Manager), restart, then **Sign in with Google**.

Default API quota is 10,000 units/day: search = 100 units, adding a track to a playlist = 50,
reading = 1. Transferring a 100-track playlist to YouTube therefore needs about 15,000 units
(two days of quota) — Aqua Glass shows this before you start.

### Local files

Settings → Music Services → Local files → add a folder. MP3, FLAC, Ogg Vorbis, WAV and
AAC/M4A are decoded natively. ReplayGain track gain is applied when "Volume
Normalization" is on.

## What works with which service

| Capability | Spotify | YouTube Music | Local |
|---|---|---|---|
| Sign-in | OAuth PKCE (browser) | OAuth (browser, your Google client) | — |
| Playlists (own + followed), order, descriptions, artwork | ✔ | ✔ (own) | folder scan |
| Liked songs / liked music | ✔ | ✔ (liked videos in the Music category) | — |
| Saved albums, followed artists | ✔ | ✖ not in the API | albums from tags |
| Recently played | ✔ | ✖ not in the API | — |
| Search | ✔ | ✔ (100 quota units each) | ✔ |
| Artist / album pages | ✔ | ✖ (API has channels, not discographies) | albums |
| **Playback inside Aqua Glass** | ✔ **Premium**, via librespot | ✖ — opens in YouTube Music in your browser | ✔ |
| Play / pause / seek / next / previous / queue / shuffle / repeat | ✔ | queue order only; controls in the browser | ✔ |
| Like a track | ✔ | ✔ | — |
| Create playlist / add tracks (transfer target) | ✔ | ✔ | — |
| Lyrics (LRCLIB) | ✔ | ✔ | ✔ (needs artist tag) |
| EQ, visualizer, normalization, gapless | ✔ | ✖ | ✔ |

**Why YouTube tracks open in the browser:** Google provides no audio streaming API for
third-party apps, and the YouTube Terms of Service prohibit extracting or downloading
streams outside YouTube's own player. Rather than silently substituting an unofficial
workaround, Aqua Glass opens the track in YouTube Music (or YouTube — configurable) and
says so in the player bar. You can also use **"Find on the other service…"** on any
track to play the same recording from Spotify instead — you always choose; recordings
are never swapped silently.

## Using Aqua Glass

* **Layout** (after the Aqua Glass concept design):
  * a collapsible sidebar with pill navigation (Home, Search, Library, Playlists, Artists,
    Albums, Discover, Queue, Playlist Transfer, Settings) and the RAM Usage card;
  * a top bar with back/forward and a centred search pill (its ⚙ button sets the service
    filter and shows account status);
  * the main content area and the persistent bottom player;
  * an optional right panel with Queue / Similarity Radio / Lyrics tabs, shown only when
    you open it. Click the artwork in the player for the
  Now Playing page. There are no social features.
* **Home** shows a featured banner (from your own playlists or top mix), Recently Played, Favorite Playlists, Made For You (Spotify-made playlists
  you follow), Recently Added, Your Top Mixes, Discover New Music, Recently Imported Spotify
  Playlists and Local Music — each row appears only when there is real data for it.
* **Service filter** (top bar): All / Spotify / YouTube / Local. Disabled services do no background work.
* **Library import & sync** (Library page / Settings → Library & Synchronization):
  * *Accessing* a playlist reads it live from the service.
  * *Importing* caches it locally (`%APPDATA%\AquaGlass\library`) and keeps it in sync;
    "Import all playlists" or tick individual playlists.
  * *Sync Now* or automatic sync (interval configurable). Unchanged playlists are skipped
    using their snapshot ids; 429 rate limits are honoured with back-off.
  * Sync is **read-only** towards your account. Playlists deleted/unfollowed upstream are
    kept locally and flagged for review; a sudden large shrink is flagged and the previous
    copy is backed up.
* **Playlist Transfer:** choose a source playlist (any service, or restore from a
  JSON/CSV/M3U file), choose a destination, preview matches (matched / ambiguous /
  unmatched with scores and reasons such as "version differs: live"), pick replacements
  manually, rename, and create. The source is never modified; a JSON backup and a log are
  written to `transfers\`, and you can remove a created playlist again from the history.
* **Export:** any playlist to JSON (full metadata, re-importable), CSV or M3U.
* **Mini-player** (button in the playback bar, tray menu or `Ctrl+Alt+N`): three
  layouts, opacity, always-on-top, remembers position and size, optional lyric line.
* **Desktop overlay:** transparent always-on-top window with title, artist, artwork,
  current lyric and progress; position/size/opacity/font size; click-through.
* **Keyboard:** `Space` play/pause, `Ctrl+,` settings, `Ctrl+F` search, plus every action
  in Settings → Keyboard Shortcuts (with conflict detection). With "Global Hotkeys" on,
  they work system-wide; macro pads that send key combinations (e.g. F13–F24) work as-is.
  Hardware media keys and the Windows media flyout work via "Windows Media Controls".

## Similarity Radio

Play a single song from search results and Aqua Glass starts **Similarity Radio**
automatically (Settings › Playback › Similarity Radio; on by default). The song plays
immediately; ~15 similar, *playable* songs are added to the "Automatically Recommended"
part of the queue in the background and refilled when fewer than 5 remain.

* **Source of similarity:** [Last.fm](https://www.last.fm/api)'s public similarity data
  (`track.getSimilar`, `artist.getSimilar`, `artist.getTopTracks`). It needs a free API key:
  create one at <https://www.last.fm/api/account/create> and paste it in Settings ›
  Playback › Similarity Radio (stored in Credential Manager). Only the seed's artist/title
  and related artist names are sent; no Last.fm account or scrobbling.
* **Without a key** the radio falls back to songs by the same artist / from the same album
  (local library and Spotify) and labels itself "Artist & album based". It never invents
  recommendations.
* **Playable only:** each candidate is matched to a track you can actually play — your local
  library first, then Spotify (when Premium playback is ready) using the same strict
  matching as playlist transfer (no live/remix/cover substitutions). Unmatched candidates
  are dropped. YouTube is used only if you choose it as provider (tracks open in the browser,
  so the radio can't auto-advance there).
* **Discovery strength:** *Familiar* (the seed's closest similar tracks), *Balanced* (plus the
  best-known songs of the 6 closest related artists), *Adventurous* (artists 4–24 in the
  related-artist list, seed artist down-weighted), *Deep Discovery* (artists 10–60, their
  less popular tracks, popularity-penalised, seed artist and songs you've heard excluded).
* **Rules:** Prefer Unheard, Avoid Recently Played (7 days), Avoid Duplicates (per session),
  Allow Repeated Artists (off = one per artist per batch, not within the last 4), Learn From
  Likes and Skips (3 skips drop an artist for the session — never the seed's artist; likes
  boost an artist and steer the seed), Stay Close to Original (otherwise the seed follows
  the radio), Endless Radio, target size and refill threshold, preferred provider.
* **Queue behaviour:** songs you add with "Add to queue" always play before recommendations;
  playing a playlist or album never gets radio mixed in (with "Activate from Album/Artist
  Pages" on, recommendations follow *after* the album). Controls in the right panel:
  Radio on/off, Refresh, More Like This, Stay close to original, Save queue as playlist
  (Spotify playlist for Spotify songs, M3U file for the rest).
* **Footprint:** one worker thread that only exists while the feature is on, woken by queue
  changes (no polling); bounded caches (48 Last.fm responses); measured ≈ +0.1 MB.
* Listening history (last 500 plays, `history.json`) stays on your PC and can be cleared.

## Settings, RAM ratings and presets

Open settings with the always-visible **⚙ Settings** button, `Ctrl+,`, the tray menu, the
☰ application menu, or the ⚡ quick-settings button. The last category is remembered, and
the search box finds settings by name, description, category or keywords — e.g. "RAM",
"lyrics", "Discord", "background".

Every optional feature (78 of them) has its own toggle, description, dependency info and a
RAM badge:

```
[✔] Blurred Background
    RAM: LOW   Estimated: +0.5–4 MB          ← engineering estimate (not yet measured)
[✔] Custom Background Image
    RAM: HIGH  Measured: +16.4 MB             ← measured on *your* PC
```

* **Estimated** values come from the table in `src/features.rs`, calibrated with the
  benchmark in [docs/BENCHMARKS.md](docs/BENCHMARKS.md).
* **Measured** values are recorded automatically on your machine: when you enable a
  feature, Aqua Glass reads the private working set before and ~6 s after, and keeps a
  running mean per feature (Settings → Performance → "Measured feature costs on this PC").
* Ratings: Low ≤ 5 MB · Moderate ≤ 15 MB · High ≤ 35 MB · Very High > 35 MB.
* The Performance page shows live private working set, peak, private bytes, CPU, cache
  sizes, FPS, background tasks, and a combined estimate of enabled features that counts
  shared resources (one background texture) only once.
* **RAM target** (default 200 MB; 100/150/200/250/300/custom) is a *soft* target. With the
  "Automatic Memory Optimizer" on, Aqua Glass halves the artwork cache at 85 %, and above
  100 % lowers artwork resolution, drops caches and hidden track lists, frees the
  visualizer and asks Windows to trim the working set. It never stops playback, kills
  itself or discards data. "RAM Warnings" asks before enabling a feature that may exceed
  the target (*Enable Anyway* / *Cancel*).
* **Presets:** Minimal, Balanced (default), Enhanced, Fully Customized, Custom. Save, apply,
  export and import your own presets. Presets never touch accounts or library data.
* **Disable All Visual Customizations** turns off every visual feature while keeping
  playback, accounts, libraries, playlists and playback preferences; **Restore previous
  customization profile** undoes it.
* Settings save automatically (`settings.toml`), can be exported/imported (never
  containing credentials), reset per feature, per category or completely.

## Privacy and security

* No telemetry, analytics, crash reporting or advertising SDKs.
* Network access only to: Spotify (accounts/api/CDN/access points), Google (OAuth and
  YouTube Data API), image CDNs for artwork, and lrclib.net for lyrics (when Lyrics is on:
  sends title, artist, album, duration). Discord presence uses only the local Discord IPC pipe.
  The full table is in Settings → Privacy.
* OAuth refresh tokens and the Google client secret are stored in **Windows Credential
  Manager** (`AquaGlass/...`), never in files, logs or exports. Error messages are scrubbed
  of token-like strings.
* Diagnostic logging is off by default.
* "Disconnect" deletes stored tokens (and revokes the grant at Google); optionally deletes
  the cached library.

## Building from source

Requirements: Rust 1.85+.

```bash
# native (Windows, MSVC or GNU toolchain)
cargo build --release
```

Cross-compiling from Linux (how the provided exe is built):

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt-get install mingw-w64
cargo build --release --target x86_64-pc-windows-gnu
```

Note: `vergen` is pinned to 9.0.6 in `Cargo.lock` because librespot-core 0.8's build script
fails with vergen 9.1 (`cargo update -p vergen --precise 9.0.6` if you regenerate the lockfile).

Optional modules are Cargo features, so a build can leave them out entirely (runtime
toggles only stop them):

| Feature | Default | Contents |
|---|---|---|
| `spotify-playback` | ✔ | librespot (Spotify Premium playback) |
| `discord` | ✔ | Discord Rich Presence IPC |
| `local-files` | ✔ | symphonia decoders for local files |
| `accessibility` | ✖ | AccessKit screen-reader support |

```bash
# smallest build: local files only
cargo build --release --no-default-features --features local-files
```

`scripts/package-windows.sh` builds the release exe and the zip.

## Testing and benchmarking

```bash
cargo test --workspace           # unit + integration tests (all crates)
scripts/check-portable.sh        # portable crates have no UI/audio/OS deps; aqua-core builds for Android
```

* 111 unit tests across the workspace (including Similarity Radio and its live self-test harness: the four discovery modes differ as specified,
  duplicate/artist/recently-played/skip/like rules, unplayable candidates dropped, Last.fm
  response parsing, history bounds), plus: config persistence/migration/invalid files, feature registry and
  presets, disable/restore visuals, shortcut parsing and conflicts, theme loading and
  validation, Spotify/YouTube response parsing (both 2026 and older field names), API
  error classification, OAuth PKCE (RFC 7636 vector), library import/sync/conflicts/
  corrupted cache, track matching (live/remix/cover/remaster/feat.), JSON/CSV/M3U
  round-trips, transfer planning/execution with a fake service, queue logic incl. a
  20,000-track shuffle, DSP (EQ gain at 1 kHz, bass shelf, balance, resampler),
  lyrics parsing (LRC, offsets, word timing), visualizer FFT, Discord presence content.
* `tests/radio_e2e.rs` runs the real engine, audio output, library store and radio worker:
  radio starts from a played song, fills to 15, a manually queued song plays before the
  recommendations, the queue refills before running out, no duplicates, Stop clears only
  the recommendations.
* `tests/playback.rs` plays generated audio through the real decoder → DSP → audio output →
  engine: auto-advance (gapless and not, including a track shorter than the buffer), seek,
  pause, end of queue, missing files, Spotify without a session. It needs an audio device
  (on Linux CI: PulseAudio with `module-null-sink`); it skips when none exists.
* Sync model (`crates/aqua-core`): serialization round trips, schema-version checks, a frozen
  schema-1 fixture that must keep decoding and re-encoding identically, queue-state
  preservation (manual items stay ahead of radio items after a round trip), deterministic
  merges for like/unlike and two-device playlist edits, reconnect-after-days history merge,
  handoff planning, no secrets or local paths in payloads.
* `bench/run_bench.py` and `bench/soak.py` reproduce the memory benchmarks; see
  [docs/BENCHMARKS.md](docs/BENCHMARKS.md) for the procedure, environment and results.

## Architecture

The repository is a Cargo workspace. The Windows desktop app is the root package; code
that a future Android client (or a sync server) can reuse lives in platform-independent
crates. See [docs/future-mobile-architecture.md](docs/future-mobile-architecture.md).

```
crates/
  aqua-core/         portable core (no UI/audio/OS deps; compiles for aarch64-linux-android)
    model.rs         provider-neutral Track / Playlist / Album / Artist, ProviderKind
    queue.rs         play queue: order, shuffle, repeat, Context/Manual/Radio entry origins
    playback.rs      PlaybackControl trait (platform audio implements it), PlayerState
    history.rs       local listening history (bounded)
    library.rs       import cache, snapshot-based sync, conflicts, backups
    matching.rs      cross-service track matching
    prefs.rs         portable Similarity Radio preferences
    session.rs       saved "remember queue" session (keeps entry origins)
    sync/            versioned cross-device sync + handoff data model (no networking)
    secrets.rs, storage.rs, platform.rs   hooks the platform fills in (secret store,
                     data directory, browser launcher)
  aqua-providers/    oauth (PKCE loopback), spotify (Web API), youtube (Data API), lastfm,
                     lyrics, shared rate limiter — reqwest + rustls, no OS APIs
  aqua-radio/        Similarity Radio: gather (Last.fm) → rank/filter → resolve → queue
                     refill, driven through PlaybackControl
src/                 Windows desktop app
  main.rs            entry point (windows_subsystem in release)
  lib.rs             run(): CLI, window options
  util.rs            data directory, platform hook installation, logging
  config.rs          settings.toml model, versioned migration, import/export, presets
  features.rs        registry of every optional feature (id, text, RAM estimate, deps, presets)
  perf.rs            Windows process metrics, per-feature measurement, memory pressure
  security.rs        Windows Credential Manager secret store
  syncprep.rs        desktop ↔ sync model conversion, --export-sync-snapshot
  transfer.rs        JSON/CSV/M3U export/import, transfer plan + execution + logs
  theme.rs           declarative TOML themes → egui style
  shortcuts.rs       actions, binding parser, conflict detection
  artwork.rs         downscaling image loader, LRU texture cache with byte budget, blur
  discord.rs         Rich Presence worker (exists only while enabled)
  winint.rs          tray, SMTC media controls, global hotkeys, autostart, toasts, acrylic
  model/history/library/matching/radio/services.rs   re-exports of the portable crates
  audio/
    output.rs        single cpal/WASAPI output: ring buffer, volume, fades, visualizer tap
    dsp.rs           EQ, bass shelf, preamp/ReplayGain, balance, resampler
    engine.rs        playback engine thread (implements PlaybackControl), gapless hand-off
    spotify.rs       librespot session + custom Sink into our output
    local.rs         symphonia decoding, tag probing, folder scan
    visualizer.rs    FFT analyzer (runs only while visible)
  ui/                shell (sidebar/top bar/player/right panel), browse (home/search/discover/
                     now playing/queue), pages (library views), settings, transfer, viewports
```

Key decisions:

* **One audio pipeline.** librespot's decoded PCM goes through a custom `Sink` into the
  same output, DSP and visualizer tap as local files.
* **Playback is independent of the UI.** The engine runs on its own thread, so queue
  advance, media keys, tray actions, Discord and notifications keep working while the
  window is minimized or hidden. `App::logic` processes background messages even when the
  window is hidden.
* **Repaint only on change.** Idle UI does not redraw; playback repaints twice a second for
  the progress bar; only visible animations (visualizer, animated background, transitions)
  request higher frame rates.
* **Everything optional is lazily created and dropped**: Discord thread, tray, hotkey
  registrations, media controls, analyzer, background textures, overlay/mini windows.

## Known limitations

Honest list of what is not done or not verified:

* **Not yet run on real Windows 11.** The exe is cross-compiled (MinGW) and imports only
  standard system DLLs. Under Wine 9 the UI, tray icon, global-hotkey registration, WASAPI
  playback and portable mode worked; SMTC/media flyout, Credential Manager, toasts,
  registry autostart and DWM acrylic were not exercised (Wine doesn't implement them
  faithfully), and Wine doesn't report private memory counters (the dashboard says so). The Windows memory figures in
  the app come from Windows itself; the numbers in docs/BENCHMARKS.md are from Linux.
* **Spotify and YouTube were not tested against the live services** (no credentials or
  network route to them in the build environment). Parsing is tested against fixtures that
  follow the documented response formats, but end-to-end sign-in, sync, playback and
  transfers need verification with real accounts.
* **Similarity Radio has not yet run against real Last.fm responses**: the build sandbox
  blocks `ws.audioscrobbler.com`. Parsing is tested against Last.fm's documented JSON, the
  pipeline against a deterministic similarity graph, and the end-to-end flow (start from
  search, fill, refill, manual priority, no duplicates) with the artist/album fallback and real
  audio. To run the live checks on your PC (in-app button, `--radio-selftest`, or
  `cargo test --test lastfm_live -- --ignored`), see **[docs/TESTING-LASTFM.md](docs/TESTING-LASTFM.md)**.
* The interface follows the supplied Aqua Glass concept image. The window keeps the native
  Windows title bar (with dark mode) instead of the concept's custom-drawn one.
* YouTube Music playback is not in-app (see above). No YouTube albums, artists pages,
  recently played or lyrics from YouTube.
* Not implemented: crossfade, WASAPI exclusive mode, lyric translations, lyric blur/
  position/animation-speed options, taskbar thumbnail toolbar buttons, raw HID
  macro-pad support (key-sending macro pads work), visualizer placement options other
  than Now Playing / player bar.
* The "Windows 11 Acrylic Backdrop" depends on the GPU driver supporting a transparent
  OpenGL surface; it is experimental and needs a restart.
* Discord Rich Presence needs a Discord Application ID you create (free) — Aqua Glass
  doesn't ship one.
* Notifications use the PowerShell AppUserModelID because the portable exe isn't
  registered with Windows; the toast therefore shows "Windows PowerShell" as the source.
