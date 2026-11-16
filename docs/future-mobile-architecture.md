# Future mobile client and cross-device sync: architecture

Status (Aqua Glass 0.4.0): **preparation only.**

- There is no Android app, no sync server and no network sync code.
- The Windows app behaves exactly as before. It has:
  - no new background threads or polling;
  - no local server;
  - no device discovery.
- What changed:
  - the code is now organised so portable logic can be reused;
  - a versioned sync data model exists and is tested;
  - one CLI command exports local data in that format.

This document describes the code as it is now. Section 9 says what would be built next.

The eventual goal: listen on the PC, open Aqua Glass on an Android phone, see what is playing, tap **Continue on this device**, and keep going with the same queue, recommendations, playlists and history.

---

## 1. Reusable (portable) Rust code

The repository is a Cargo workspace. The desktop app is still the root package. Portable code lives in `crates/`:

```
aqua-glass/                 desktop app (Windows 11; egui, cpal, librespot, Win32)
├── crates/
│   ├── aqua-core/          portable core — no UI, audio, network or OS APIs
│   ├── aqua-providers/     music services over HTTPS (reqwest + rustls)
│   └── aqua-radio/         Similarity Radio engine
└── docs/
```

The three crates are layered strictly: `aqua-core` ← `aqua-providers` ← `aqua-radio` ← desktop app.

To keep old paths working, the desktop app re-exports the moved modules (`crate::model`, `crate::history`, `crate::library`, `crate::matching`, `crate::radio`, `crate::services`). As a result, the UI code did not change.

I split the code into only three crates. More would have meant more maintenance for no portability gain. The suggested `aqua-library`, `aqua-settings` and `aqua-sync-model` are modules of `aqua-core` instead, because they share the same types and have no extra dependencies.

### `aqua-core` (dependencies: serde, serde_json, anyhow, log, rand)

| Module | What it holds |
|---|---|
| `model` | `Track`, `Playlist`, `Album`, `Artist`, `ProviderKind`, `RepeatMode`. Every track carries its provider and the provider's id. `Track::key()` is `"<Provider>:<id>"`. |
| `matching` | Cross-provider matching: ISRC first, then strict normalised artist/title/duration. Title normalisation. |
| `history` | Bounded local listening history (500 plays) and the `artist\|title` identity used by radio. |
| `library` | On-disk store for imported playlists, saved albums and artists, liked songs and local files. It reconciles changes made on the provider's side (playlists that shrank or were removed). |
| `queue` | The play queue: order, shuffle, repeat. Each entry has an `EntryOrigin`: `Context`, `Manual` or `Radio`. Manual picks always play before radio recommendations. |
| `playback` | The `PlaybackControl` trait (play, pause, resume, next, previous, seek, volume, queue operations, `snapshot`, `position_ms`, listeners), plus `PlayerState` and `PlayRecord`. |
| `session` | The saved "remember queue" session (v2 keeps entry origins; it still reads the v1 files from 0.3). |
| `prefs` | `Strength`, `RadioProvider` and `RadioPrefs`: radio preferences shared by every device. |
| `sync` | Versioned sync and handoff models (see section 4). Data only. |
| `secrets` | The `SecretStore` trait and an `install()` hook. Default: in memory. |
| `storage` | Data directory chosen by the platform, `atomic_write`, clocks. |
| `platform` | The `open_url` hook (OAuth, "open in Spotify") and `Platform::current()`. |

### `aqua-providers` (adds: reqwest blocking + rustls/ring, url, sha2, base64)

This crate holds the Spotify Web API (search, library, playlists, the 2026 response formats), Last.fm, LRCLIB lyrics, OAuth PKCE and the shared rate limiter. It contains no Windows code.

- Secrets go through `aqua_core::secrets`.
- The browser is opened through `aqua_core::platform::open_url`.
- On the desktop, both hooks are installed by `util::init_platform()` (Windows Credential Manager, default browser).

### `aqua-radio` (adds nothing beyond the two crates above)

Similarity Radio, unchanged in behaviour. It covers:

- gathering candidates from Last.fm for each discovery strength;
- ranking: duplicates, recently played, unheard, repeated artists, skips and likes;
- strict resolution to playable tracks;
- the refill worker: 15 queued, refill at 5, endless.

The worker now drives playback through `Arc<dyn PlaybackControl>` and gets local music from a closure. Before, it used the desktop `Engine` and `LibraryStore` directly. A mobile client can therefore reuse the whole radio, including queue refill, by implementing `PlaybackControl`.

Unchanged behaviour:

- Playing a song from search starts radio (`radio_from_search`).
- The target is 15 recommendations, refilled when 5 remain.
- Recommendations keep coming while radio is enabled.
- Manual queue items keep priority.
- Radio never replaces playlist or album playback.

### Checking the boundary

`scripts/check-portable.sh` verifies two things:

1. The three crates don't depend on any UI, audio output or OS crate: eframe/egui/winit, cpal, librespot, symphonia, windows, winreg, keyring, tray-icon, souvlaki, global-hotkey, rfd or open.
2. `aqua-core` compiles for `aarch64-linux-android`. **Verified.**

`aqua-providers` and `aqua-radio` are plain Rust, but their TLS stack (`ring`) compiles C code. Checking them for Android needs the NDK (`cargo ndk -t arm64-v8a check -p aqua-radio`). That wasn't available in the build environment, so it is **not yet verified**.

## 2. Code that stays desktop-only

These modules remain in the root package (`src/`):

| Module | Why it's platform-specific |
|---|---|
| `ui/*` (egui) | Desktop UI. Android should use its own UI (see section 3). |
| `audio/output.rs`, `audio/dsp.rs`, `audio/visualizer.rs` | cpal/WASAPI output pipeline and DSP. The DSP is portable maths but lives behind the cpal ring buffer. |
| `audio/engine.rs` | The Windows `PlaybackControl` implementation: a thread, gapless chaining and backends. |
| `audio/spotify.rs` | librespot playback. See section 5 for why this doesn't carry over to Android. |
| `audio/local.rs` | symphonia decoding. Portable, but tied to the output pipeline. |
| `winint.rs`, `shortcuts.rs` | Tray, Windows media controls (SMTC), global hotkeys, autostart, toasts, DWM. |
| `security.rs` | Windows Credential Manager adapter. |
| `perf.rs`, `features.rs`, `theme.rs`, `artwork.rs`, `config.rs` | Desktop RAM dashboard, feature registry, egui theme engine, egui texture cache, desktop config. Config converts to and from the portable models (`Config::radio_prefs`, `syncprep::settings_state`). |
| `transfer.rs`, `ui/jobs.rs` (library sync) | Portable in substance, but not moved yet (see section 10). |
| `syncprep.rs` | Desktop glue for the sync model (snapshot export, settings conversion). |

## 3. How an Android app would use the Rust core

The recommended shape is a native Kotlin UI with a shared Rust core. The core doesn't force egui.

```
Kotlin (Jetpack Compose UI, MediaSessionService, WorkManager, Keystore)
   │  UniFFI bindings (generated Kotlin)  — or JNI for hot paths
   ▼
aqua-mobile-ffi  (future crate: thin #[uniffi::export] facade)
   ├── aqua-core       models, queue, history, library, sync, handoff planner
   ├── aqua-providers  Spotify Web API, Last.fm, lyrics
   └── aqua-radio      Similarity Radio worker
```

- **UI:** Compose, talking to a small facade. Examples: `search()`, `library()`, `queue_state()`, `radio_start(seed)`, `plan_handoff(offer)`. The facade returns plain records that UniFFI can expose. Using egui on Android would be possible but isn't recommended (accessibility, IME and system integration).
- **Playback:** a Kotlin `MediaSessionService` owns audio focus, background playback and the notification controls. The Rust side sees it through `PlaybackControl`: a Rust struct forwards commands to Kotlin through a UniFFI callback interface, and state changes come back through `add_listener`.
  - The queue logic (`aqua_core::queue::Queue`) can live on either side. Keeping it in Rust keeps the manual/radio ordering rules identical on both platforms.
- **Secrets:** implement `SecretStore` in Kotlin (EncryptedSharedPreferences or a Keystore-wrapped key) and `install()` it at startup.
- **Data directory:** pass `Context.filesDir` to `aqua_core::storage::init_data_dir`.
- **Browser and OAuth:** `install_url_opener` opens a Custom Tab. The desktop OAuth flow uses a loopback redirect (`http://127.0.0.1:<port>/login`). On Android, register an app link or custom scheme with the provider instead and pass the code back into Rust. **This needs a small change in `aqua_providers::oauth`:** split `authorize()` into `begin()` and `finish(code)`.
- **Threads:** the providers use reqwest's *blocking* client. On Android, call the facade from a background dispatcher (`Dispatchers.IO`), never the main thread. The radio worker is an ordinary thread that exists only while radio is on, the same as on desktop.

## 4. Sync model (`aqua_core::sync`)

The sync model is serde and JSON only; there is no transport. Section 6 describes the format, and docs/sync-server.md describes the API.

| Type | Contents |
|---|---|
| `Envelope<T>` | `schema`, `kind`, `device`, `created_ms`, `payload`. Every payload travels in one. |
| `DeviceInfo` / `DeviceId` | A random 128-bit id plus a user-chosen name and the platform. No host name, account name or hardware id. Created lazily in `<data>/device.json`; the app does not create it at startup. |
| `Stamp`, `HybridClock` | Hybrid logical clock with a total order of (ms, counter, device), so every merge is deterministic. |
| `Lww<T>`, `LwwMap<V>` | Last-writer-wins register, and a map whose removals are remembered (tombstones). |
| `TrackRef` | Provider, id, title, artists, album, duration and ISRC. For **local files** the path is *not* sent (it is device-specific and can include the Windows user name); the id becomes the normalised `artist\|title`, which other devices match. |
| `ListeningState` | Device, track, position, duration, playing, `updated_ms`, stamp. `estimated_position_ms(now)` works out the current position. |
| `QueueState` | Entries in play order, each with an `EntryOrigin` (Context/Manual/Radio). Also the current index, shuffle, repeat, and a `RadioSession` (current seed, original seed, `RadioPrefs`). Converts to and from `Queue` and `PlayerState`. The round trip is tested and keeps manual items before radio items. |
| `LibraryState` | Playlists (`SyncPlaylist`), `favorites`, `saved_albums`, `saved_recommendations` (all `LwwMap`) and append-only `history` (unique event ids, capped at 2000). |
| `SyncPlaylist` | A mergeable playlist with per-item ids, an LWW position for moves and an LWW presence flag for removals; name, description and deleted are LWW. `PlaylistOrigin::Local` playlists are editable; `PlaylistOrigin::Provider` playlists are read-only references to Spotify data. |
| `SettingsState` | Theme, `RadioPrefs`, `PlaybackPrefs` (preferred provider, fade, gapless, normalisation, seek step) and `SyncPrefs` (what to sync; theme sync is off by default). |
| `HandoffOffer`, `Capabilities`, `plan_handoff` | See section 5. |
| `Outbox`, `Change` | Pending local changes for offline-first upload. State-type changes coalesce. Acknowledged by sequence number. |
| `Snapshot` | Everything one device can share. |

`SettingsState` has **no fields** for device-specific settings, so they cannot leak into a payload. These include:

- the audio output device, window size and position, UI scale;
- tray, hotkeys, autostart and SMTC;
- RAM target and file paths;
- streaming quality (a phone on mobile data needs its own value).

`syncprep::apply_settings_state` only applies the categories the receiving device accepts. A test checks that the Windows audio device name never appears in the JSON and is left untouched on the receiving side.

Desktop wiring that exists today:

- `aqua-glass.exe --export-sync-snapshot [file.json]` reads this PC's data:
  - the saved session (as the queue and listening state);
  - history;
  - imported provider playlists;
  - saved albums;
  - settings.

  It writes one schema-1 `Snapshot` envelope. It runs once and exits, and opens no network connections. Use it to inspect the format with your own data, or as test input for a future server or mobile client.
- The saved session (`session.json`) now keeps the manual/radio/context origin of every queue entry, so a restart no longer turns radio recommendations into ordinary queue items. Old session files are still read.

## 5. Playback handoff and its limits

`plan_handoff(offer, capabilities, now)` is a pure function. It runs on the *target* device using that device's own sign-in state:

1. **Nothing playing:** `Unavailable`.
2. **The same provider is playable here** (signed in, Premium where needed): `Resume { track, position }`, with the position estimated from `updated_ms`.
   - If the offer is older than 6 hours, playback starts from 0:00.
   - Within 5 seconds of the end, the track restarts.
3. **Another provider can play here** (including legacy YouTube tracks from pre-0.6 data): `ResumeMatched { search: [...] }`. The device matches by ISRC, then strict artist/title/duration. If there is no confident match, nothing plays; it never pretends.
4. **Otherwise:** `Unavailable`, with a reason.

`pause_source` asks the *source Aqua Glass app* to pause its own player. It is never a remote command to Spotify.

Limitations:

- **Spotify on Android.** The desktop plays Spotify through librespot, an unofficial Spotify Connect implementation used with your own Premium account. An Android app should instead use the official **Spotify Android SDK (App Remote)**, which controls the installed Spotify app. That has consequences:
  - playback happens *in Spotify's app*;
  - it needs Premium and the Spotify app installed;
  - seek-to-position works through App Remote, but Aqua Glass's DSP/EQ won't apply.

  Whether shipping librespot on Android is acceptable is a policy decision to make before building, not a technical given.
- **YouTube Music** support was removed in 0.6 (no third-party playback API). `ProviderKind::YouTube` remains only so older data still loads.
- **Pausing the source device** needs both devices online and a delivery path: the sync server relays a `pause_request`. Aqua Glass doesn't use Spotify Connect device switching for this. Spotify's Web API "transfer playback" controls *Spotify's* devices, not Aqua Glass, and needs extra scopes and Premium.
- **Local files** play on the phone only if a matching file exists there (matched by `artist|title`).
- **The position is approximate:** network delay and the time the user takes to tap mean it is accurate to within a few seconds.

## 6. Data formats

- **JSON** (serde_json) for every sync payload, inside `Envelope { schema, kind, device, created_ms, payload }`. Enum values are spelled the way serde writes them by default (`"Spotify"`, `"Radio"`); envelope kinds are snake_case.
- **Versioning:**
  - `SCHEMA_VERSION = 1`. Additive changes (a new optional field with a default) keep the version: readers ignore unknown fields, and fields that are missing take their default.
  - Breaking changes bump the version and add a step to `sync::migrate()`. Readers **refuse newer schemas** (`SyncError::NewerSchema`) rather than guessing.
  - `crates/aqua-core/tests/fixtures/sync-snapshot-v1.json` is a frozen schema-1 sample. A test makes sure it still decodes *and* that today's encoder produces identical JSON, so renaming a field fails CI.
- **Local files are unchanged** (TOML config, JSON library and history). The saved session moved to v2, which can still read v1.
- **Identifiers:**
  - Tracks and albums: `Provider:id`.
  - Playlists created in Aqua Glass: `local:<device>:<ms>:<ctr>` (no collisions when created offline).
  - Imported playlists: `Provider:<provider playlist id>`, so importing the same Spotify playlist on two devices gives **one** synced playlist.
  - Playlist items and history events: `<device>:<ms>:<ctr>`.

## 7. Security

- **No secret fields:** no sync type has a field for tokens, passwords, client secrets or API keys. Tests serialise payloads and check that none of these appear, including a planted Last.fm key.
- **Credentials stay on each device**, in the platform's secret store:
  - Windows Credential Manager today;
  - the Android Keystore later.

  Each device signs in to Spotify itself.
- **Device ids are random,** not derived from hardware or account. Local file paths are not sent.
- **Future server authentication** is per device, can be revoked, and uses HTTPS only (details in docs/sync-server.md).
- **Clock-skew guard:** a device whose clock runs far ahead could win every last-writer-wins merge. `Stamp::plausible()` rejects stamps more than 5 minutes in the future; receivers and the server apply the same check.
- **No new account system** and no sign-in screens were added.

## 8. Expected sync workflow (future)

1. Every local action works offline and is recorded in the local store, plus the `Outbox` if sync is enabled.
2. When the app is in the foreground *and* sync is enabled, it pushes the outbox and pulls changes since its last cursor. This happens on events (app start, track change, play/pause, library edit) and with a modest timer at most. It never polls in the background.
3. The server stores merged state per user and a per-device `ListeningState`/`QueueState`. It merges with the same `aqua-core` code.
4. Opening Aqua Glass on the phone pulls the other devices' `ListeningState` and shows "Playing on Desk PC: Song – 1:42".
5. Tapping **Continue on this device** runs `plan_handoff` with the phone's capabilities, starts playback, restores the queue (origins intact, so radio resumes with the same seed and prefs) and sends `pause_request` to the PC.
6. After days offline, a device pushes its outbox: history merges by union and the latest action wins for likes. Playlists merge item by item; removals are remembered.

Conflict rules (all deterministic, tested in `sync/tests.rs`):

| Situation | Rule |
|---|---|
| Playlist edited on two devices | Item-level merge: both sides' additions are kept; removals win over old copies; latest rename, move or delete wins; ties broken by device id. |
| Liked on one device, unliked on another | `LwwMap`: the later action wins on every device. The tombstone stops old replicas resurrecting it. |
| Two devices change the queue | Queues are per-device sessions and are never merged; each device's latest queue is kept. Handoff copies one queue whole. |
| Reconnect after several days | History union (unique ids, chronological, capped); outbox replay is idempotent; clock observation keeps later edits ordered after what was seen. |
| Conflicting settings | LWW per category (theme, radio, playback, sync prefs); each device decides which categories it accepts. |
| Provider-owned data (Spotify playlists, Liked Songs) | Never edited through sync. The newest provider listing wins. Nothing is written back to Spotify without an explicit user action in that device's UI. |

## 9. Recommended implementation order

1. **Sync server MVP** (docs/sync-server.md): device pairing and tokens; snapshot push and pull reusing `aqua-core` merges.
2. **Desktop sync client:** opt-in in settings. Event-driven push and pull; `Outbox` persisted in `<data>/sync/`; no background polling. Show "Playing on …" for other devices.
3. **`aqua-mobile-ffi` crate:** a UniFFI facade over core, providers and radio. Split OAuth into `begin`/`finish`.
4. **Android app:**
   1. Compose UI;
   2. Keystore `SecretStore`;
   3. browsing, search, library, history (read-only first);
   4. `MediaSessionService` + `PlaybackControl` bridge;
   5. Spotify App Remote;
   6. Similarity Radio.
5. **Handoff UI** on both platforms using `plan_handoff`; `pause_request` relay.
6. Move `transfer.rs` and the library-sync jobs (`ui/jobs.rs`) into the portable crates when the phone needs them.

## 10. Remaining technical challenges

- **Spotify playback on Android:** librespot vs the official SDK (section 5). This is a policy decision as well as a technical one.
- **OAuth on mobile:** the redirect URI type and provider app registration (Spotify Development Mode limits).
- **ring/rustls on Android:** builds with the NDK. CI needs `cargo-ndk`, and binary size should be checked.
- **Blocking HTTP:** fine on worker threads. An async facade may suit Kotlin coroutines better later.
- **Not yet portable:** `transfer.rs` and library sync (`ui/jobs.rs`) still sit in the desktop package; there is no fundamental reason, it's just not done yet.
- **Playlist positions:** `f64` sort keys can run out of precision after very many moves between the same two items. Re-spacing on the server would fix this when needed.
- **Pruning tombstones:** `LwwMap::compact` exists, but the "every device has seen it" watermark is the server's job.
- **Battery and data:** the mobile client must follow the same rule as the desktop, with no background polling. Use FCM or a manual pull for "Playing on PC" updates.

---

## Appendix: requirements for the self-hosted sync server

Full proposal: **docs/sync-server.md**. Summary:

- **Platform:** a single Rust binary (Axum on tokio, rustls) for Linux x86_64 and **ARM64** (e.g. Raspberry Pi 4/5). Target: under 30 MB RAM at idle (an estimate, not measured); SQLite storage; no external database.
- **Transport:** HTTPS only, via rustls with your own certificate or behind a reverse proxy such as Caddy.
- **Data:** metadata only. No audio, no music-service credentials.
- **Devices:** multiple devices per user. Each has a pairing-issued token, can be listed and revoked, and its token is stored hashed.
- **Sync:** incremental (a cursor per device), idempotent pushes, merged with `aqua-core`. Disconnections are tolerated through the outbox. Deterministic ids prevent duplicate playlists.
- **Activity:** no background work beyond request handling and periodic tombstone compaction.
