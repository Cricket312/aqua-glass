# Configuration reference

Settings live in `%APPDATA%\AquaGlass\settings.toml` (or `data\settings.toml` next to the
exe in portable mode). The file is written automatically ~1 s after any change (atomically:
temp file + rename). You never need to edit it by hand — everything is in the Settings UI —
but it is plain TOML and safe to edit while Aqua Glass is closed.

* **Versioning:** `version = 2`. Older files are migrated on load (e.g. v1's top-level
  `ram_target` → `perf.ram_target_mb`, `blur_background` → `bg_blur`).
* **Invalid files:** unparsable files are copied to `settings.toml.bad` and defaults are used
  (you get a notification). Out-of-range values are clamped; unknown feature ids are dropped;
  missing keys get defaults.
* **Secrets are never stored here.** Spotify/YouTube refresh tokens and the Google client
  secret are in Windows Credential Manager under `AquaGlass`. Exported settings are
  therefore safe to share.

Other files in the data folder:

| Path | Contents |
|---|---|
| `settings.toml` | this configuration |
| `measurements.toml` | per-feature RAM deltas measured on this PC |
| `themes/*.toml` | user themes (declarative; no scripts) |
| `library/<provider>/index.json` | imported playlist headers, albums, artists, sync status, conflicts |
| `library/<provider>/pl_<id>.json` | cached tracks of an imported playlist |
| `library/backups/` | copies of cached playlists taken before suspicious changes / removals |
| `transfers/` | transfer logs and JSON backups of transfer sources |
| `exports/` | default location for exported playlists/settings/themes |
| `cache/artwork/` | downscaled cover JPEGs ("Artwork Disk Cache") |
| `session.json` | saved queue ("Restore Queue on Launch") |
| `aqua-glass.log` | only when "Diagnostic Logging" is on |

## Feature toggles (`[features]`)

Every optional feature is a boolean under `[features]`. The table is generated from
`src/features.rs` (`cargo run --example feature_table`). *Estimate* is the engineering
estimate shown in the UI until a measurement on your PC exists. *Visual* features are
switched off by "Disable All Visual Customizations". Preset columns show the value each
preset applies.

### General

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `tray` | System Tray Icon | LOW | +0.3–2 MB |  |  | ✔ | ✔ | ✔ | ✔ |
| `minimize_to_tray` | Minimize to Tray | LOW | +0–0 MB | `tray` |  |  |  |  |  |
| `close_to_tray` | Close to Tray | LOW | +0–0 MB | `tray` |  |  |  |  |  |
| `start_with_windows` | Start with Windows | LOW | +0–0 MB |  |  |  |  |  |  |
| `quick_settings` | Quick-Settings Button | LOW | +0–0.2 MB |  |  |  | ✔ | ✔ | ✔ |

### Appearance

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `custom_theme` | Custom Theme | LOW | +0.5–2 MB |  | ✔ |  | ✔ | ✔ | ✔ |
| `animations` | Interface Animations | LOW | +0–1 MB |  | ✔ |  | ✔ | ✔ | ✔ |
| `hover_effects` | Hover Highlights | LOW | +0–0.5 MB | `custom_theme` | ✔ |  | ✔ | ✔ | ✔ |
| `shadows` | Window & Popup Shadows | LOW | +0–1 MB | `custom_theme` | ✔ |  |  | ✔ | ✔ |
| `translucent_panels` | Translucent Panels | LOW | +0–1 MB | `custom_theme` | ✔ |  |  | ✔ | ✔ |
| `window_backdrop` | Windows 11 Acrylic Backdrop | MODERATE | +2–12 MB | `translucent_panels` | ✔ |  |  |  | ✔ |
| `custom_font` | Custom Font | MODERATE | +0.5–15 MB |  | ✔ |  |  | ✔ | ✔ |
| `artwork_in_lists` | Artwork Thumbnails in Lists | MODERATE | +4–15 MB |  |  |  | ✔ | ✔ | ✔ |

### Backgrounds

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `bg_album_art` | Album Artwork Background | MODERATE | +3–10 MB |  | ✔ |  |  | ✔ | ✔ |
| `bg_image` | Custom Background Image | HIGH | +8–35 MB |  | ✔ |  |  |  | ✔ |
| `bg_blur` | Blurred Background | LOW | +0.5–4 MB |  | ✔ |  |  | ✔ | ✔ |
| `bg_dim` | Background Dimming | LOW | +0–0.2 MB |  | ✔ |  | ✔ | ✔ | ✔ |
| `bg_dynamic_colors` | Dynamic Artwork Colors | LOW | +0–1 MB |  | ✔ |  |  | ✔ | ✔ |
| `bg_animated` | Animated Background | LOW | +0–2 MB |  | ✔ |  |  |  | ✔ |
| `bg_parallax` | Parallax Effect | LOW | +0–0.5 MB |  | ✔ |  |  |  | ✔ |
| `bg_transitions` | Background Transitions | MODERATE | +0.5–15 MB |  | ✔ |  |  | ✔ | ✔ |

### Playback

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `remember_queue` | Restore Queue on Launch | LOW | +0–1 MB |  |  | ✔ | ✔ | ✔ | ✔ |
| `notifications` | Track Change Notifications | LOW | +0.5–3 MB |  |  |  |  | ✔ | ✔ |

### Audio

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `eq` | Equalizer | LOW | +0–0.5 MB |  |  |  |  |  |  |
| `bass_boost` | Bass Boost | LOW | +0–0.2 MB |  |  |  |  |  |  |
| `normalization` | Volume Normalization / ReplayGain | LOW | +0–0.5 MB |  |  |  | ✔ | ✔ | ✔ |
| `fades` | Fade In / Fade Out | LOW | +0–0.1 MB |  |  |  | ✔ | ✔ | ✔ |
| `gapless` | Gapless Playback | LOW | +0–2 MB |  |  | ✔ | ✔ | ✔ | ✔ |

### Lyrics

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `lyrics` | Lyrics | MODERATE | +1–6 MB |  |  |  | ✔ | ✔ | ✔ |
| `lyrics_synced` | Synchronized Lyrics | LOW | +0–1 MB | `lyrics` |  |  | ✔ | ✔ | ✔ |
| `lyrics_word` | Word-by-Word Highlighting | LOW | +0–0.5 MB | `lyrics_synced` | ✔ |  |  | ✔ | ✔ |
| `lyrics_prefetch` | Prefetch Next Track Lyrics | LOW | +0–1 MB | `lyrics` |  |  |  | ✔ | ✔ |
| `lyrics_fullscreen` | Fullscreen Lyrics Button | LOW | +0–0.5 MB | `lyrics` | ✔ |  | ✔ | ✔ | ✔ |

### Visualizer

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `visualizer` | Audio Visualizer | LOW | +0.2–3 MB |  | ✔ |  |  | ✔ | ✔ |
| `vis_artwork_colors` | Artwork-Reactive Visualizer Colors | LOW | +0–0.5 MB | `visualizer` | ✔ |  |  | ✔ | ✔ |
| `vis_reduced_motion` | Reduced Motion | LOW | +0–0.1 MB | `visualizer` |  |  |  |  |  |
| `vis_in_bar` | Visualizer in Player Bar | LOW | +0–1 MB | `visualizer` | ✔ |  |  |  | ✔ |

### Discord

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `discord` | Discord Rich Presence | LOW | +1–4 MB |  |  |  |  |  |  |
| `discord_title` | Show Song Title | LOW | +0–0 MB | `discord` |  | ✔ | ✔ | ✔ | ✔ |
| `discord_artist` | Show Artist | LOW | +0–0 MB | `discord` |  | ✔ | ✔ | ✔ | ✔ |
| `discord_album` | Show Album | LOW | +0–0 MB | `discord` |  |  | ✔ | ✔ | ✔ |
| `discord_artwork` | Show Album Artwork | LOW | +0–0 MB | `discord` |  |  | ✔ | ✔ | ✔ |
| `discord_elapsed` | Show Elapsed Time | LOW | +0–0 MB | `discord` |  | ✔ | ✔ | ✔ | ✔ |
| `discord_service` | Show Service Name | LOW | +0–0 MB | `discord` |  | ✔ | ✔ | ✔ | ✔ |
| `discord_status` | Show Paused State | LOW | +0–0 MB | `discord` |  | ✔ | ✔ | ✔ | ✔ |
| `discord_privacy` | Privacy Mode | LOW | +0–0 MB | `discord` |  |  |  |  |  |

### Keyboard Shortcuts

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `global_hotkeys` | Global Hotkeys | LOW | +0.2–1 MB |  |  |  | ✔ | ✔ | ✔ |
| `media_keys` | Windows Media Controls & Media Keys | LOW | +0.5–3 MB |  |  | ✔ | ✔ | ✔ | ✔ |

### Mini-Player & Overlay

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `mini_player` | Mini-Player Window | MODERATE | +4–15 MB |  |  |  | ✔ | ✔ | ✔ |
| `mini_on_top` | Mini-Player Always on Top | LOW | +0–0 MB | `mini_player` |  | ✔ | ✔ | ✔ | ✔ |
| `mini_lyrics` | Lyrics in Mini-Player | LOW | +0–0.5 MB | `mini_player`, `lyrics` |  |  |  | ✔ | ✔ |
| `overlay` | Desktop Overlay | MODERATE | +4–15 MB |  | ✔ |  |  |  | ✔ |
| `overlay_click_through` | Overlay Click-Through | LOW | +0–0 MB | `overlay` |  | ✔ | ✔ | ✔ | ✔ |
| `overlay_artwork` | Overlay Artwork | LOW | +0–1 MB | `overlay` | ✔ |  |  | ✔ | ✔ |
| `overlay_lyrics` | Overlay Lyrics | LOW | +0–0.5 MB | `overlay`, `lyrics` |  |  |  | ✔ | ✔ |
| `overlay_progress` | Overlay Progress Bar | LOW | +0–0 MB | `overlay` |  | ✔ | ✔ | ✔ | ✔ |

### Library & Synchronization

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `auto_sync` | Automatic Library Sync | LOW | +0.5–3 MB |  |  |  | ✔ | ✔ | ✔ |
| `metadata_cache` | Persistent Metadata Cache | MODERATE | +2–15 MB |  |  | ✔ | ✔ | ✔ | ✔ |
| `artwork_disk_cache` | Artwork Disk Cache | LOW | +0–1 MB |  |  | ✔ | ✔ | ✔ | ✔ |

### Performance

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `artwork_prefetch` | Artwork Prefetching | MODERATE | +3–12 MB | `artwork_in_lists` |  |  |  | ✔ | ✔ |
| `auto_optimizer` | Automatic Memory Optimizer | LOW | +0–0.5 MB |  |  | ✔ | ✔ | ✔ |  |
| `ram_warnings` | RAM Warnings | LOW | +0–0 MB |  |  | ✔ | ✔ | ✔ | ✔ |
| `fps_counter` | FPS Counter | LOW | +0–0.1 MB |  |  |  |  |  |  |

### Privacy

| id | Name | RAM impact | Estimate | Requires | Visual | Min | Bal | Enh | Full |
|---|---|---|---|---|---|---|---|---|---|
| `diagnostic_logging` | Diagnostic Logging | LOW | +0.2–2 MB |  |  |  |  |  |  |

## Other sections

| Section | Keys |
|---|---|
| `[general]` | `last_settings_category`, `window_pos`, `window_size`, `ui_scale` (0.6–2.5), `sidebar_width` |
| `[appearance]` | `theme` (name), `font_path`, `player_bar_layout` (`Classic`/`Centered`/`Compact`), `player_bar_height` |
| `[background]` | `image_path`, `blur_radius` (0–80), `dim` (0–1), `opacity` (0–1), `anim_speed`, `anim_fps` (5–60), `transition_ms`, `scale` (`Fill`/`Fit`/`Stretch`/`Center`), `parallax_px` |
| `[playback]` | `preferred_provider`, `ask_when_both`, `volume`, `muted`, `shuffle`, `repeat` (`Off`/`All`/`One`), `quality` (`Low96`/`Normal160`/`High320`), `fade_ms`, `seek_step_s`, `volume_step` |
| `[audio]` | `device` (empty = default), `eq_gains_db` (10 bands, ±12 dB at 31 Hz…16 kHz), `eq_preset`, `bass_db` (0–15), `balance` (−1…1), `buffer_ms` (100–3000), `preamp_db` |
| `[lyrics]` | `font_size`, `bold_current`, `color`, `highlight`, `align`, `line_spacing`, `bg_opacity`, `offset_ms` |
| `[visualizer]` | `mode` (`Bars`/`Waveform`/`Circular`/`Minimal`), `fps`, `sensitivity`, `smoothing`, `bars`, `height`, `color_a`, `color_b` |
| `[mini]` | `pos`, `size`, `opacity`, `layout` (`Horizontal`/`Square`/`Compact`) |
| `[overlay]` | `pos`, `size`, `opacity`, `font_size`, `always_on_top` |
| `[services]` | `spotify_enabled`, `youtube_enabled`, `local_enabled`, `spotify_client_id` (empty = librespot default), `spotify_redirect_port`, `youtube_client_id`, `local_folders`, `youtube_open_in` (`YouTubeMusic`/`YouTube`), `discord_app_id` |
| `[sync]` | `interval_min` (5–1440), `include_followed_playlists` |
| `[perf]` | `ram_target_mb` (default 200), `artwork_max_px`, `artwork_cache_mb`, `idle_fps_cap` |
| `[shortcuts]` | action id → binding, e.g. `play_pause = "Ctrl+Alt+P"` |
| top level | `version`, `preset`, `saved_visual_profile`, `user_presets` |

### Shortcut actions

`play_pause`, `next`, `previous`, `volume_up`, `volume_down`, `mute`, `seek_forward`,
`seek_backward`, `like_song`, `toggle_lyrics`, `toggle_mini_player`, `open_search`,
`open_settings`, `switch_provider`. Key names: `A`–`Z`, `0`–`9`, `F1`–`F24`, `Space`,
`Enter`, `Escape`, `Tab`, `Backspace`, `Insert`, `Delete`, `Home`, `End`, `PageUp`,
`PageDown`, `Left`, `Right`, `Up`, `Down`, `Comma`, `Period`, `Minus`, `Plus`, `Slash`,
`Semicolon`, `MediaPlayPause`, `MediaNext`, `MediaPrevious`, `MediaStop`, `VolumeUp`,
`VolumeDown`, `VolumeMute`; modifiers `Ctrl`, `Alt`, `Shift`, `Win`.

## Themes

A theme is a TOML file; every key is optional (missing keys use the Aqua Glass default) and
numeric values are clamped:

```toml
name = "My Theme"
dark = true
accent = [34, 211, 238]
accent2 = [59, 130, 246]
background = [10, 17, 28]
panel = [16, 27, 42]
surface = [24, 39, 58]
text = [226, 240, 250]
text_dim = [140, 163, 184]
border = [40, 70, 95]
panel_alpha = 0.72        # panel opacity with "Translucent Panels"
rounding = 10.0           # 0–30
spacing = 8.0             # 2–24
font_size = 14.0          # 9–28
heading_size = 22.0       # 12–48
border_width = 1.0
shadow_intensity = 0.5    # 0–1
hover_strength = 0.6      # 0–1
animation_speed = 1.0     # 0.2–5
blur_intensity = 1.0      # multiplies the background blur radius
button_style = "Filled"   # Filled | Outline | Flat
icon_style = "Solid"      # Solid | Outline | Text
```

Built-in: Aqua Glass Default, Midnight Aqua, AMOLED Black, Minimal Dark, Minimal Light,
Rose Pine, Mocha-inspired, Custom (original palettes).

## Default settings file

<details><summary>Complete default <code>settings.toml</code> (Balanced preset)</summary>

```toml
version = 2
preset = "Balanced"

[features]
animations = true
artwork_disk_cache = true
artwork_in_lists = true
artwork_prefetch = false
auto_optimizer = true
auto_sync = true
bass_boost = false
bg_album_art = false
bg_animated = false
bg_blur = false
bg_dim = true
bg_dynamic_colors = false
bg_image = false
bg_parallax = false
bg_transitions = false
close_to_tray = false
custom_font = false
custom_theme = true
diagnostic_logging = false
discord = false
discord_album = true
discord_artist = true
discord_artwork = true
discord_elapsed = true
discord_privacy = false
discord_service = true
discord_status = true
discord_title = true
eq = false
fades = true
fps_counter = false
gapless = true
global_hotkeys = true
hover_effects = true
lyrics = true
lyrics_fullscreen = true
lyrics_prefetch = false
lyrics_synced = true
lyrics_word = false
media_keys = true
metadata_cache = true
mini_lyrics = false
mini_on_top = true
mini_player = true
minimize_to_tray = false
normalization = true
notifications = false
overlay = false
overlay_artwork = false
overlay_click_through = true
overlay_lyrics = false
overlay_progress = true
quick_settings = true
ram_warnings = true
remember_queue = true
shadows = false
start_with_windows = false
translucent_panels = false
tray = true
vis_artwork_colors = false
vis_in_bar = false
vis_reduced_motion = false
visualizer = false
window_backdrop = false

[user_presets]

[general]
last_settings_category = "General"
ui_scale = 1.0
sidebar_width = 210.0

[appearance]
theme = "Aqua Glass Default"
font_path = "C:\\Windows\\Fonts\\segoeui.ttf"   # Windows default (empty on other OSes)
player_bar_layout = "Classic"
player_bar_height = 84.0

[background]
image_path = ""
blur_radius = 24.0
dim = 0.55
opacity = 0.6
anim_speed = 1.0
anim_fps = 30
transition_ms = 600
scale = "Fill"
parallax_px = 12.0

[playback]
preferred_provider = "Spotify"
ask_when_both = true
volume = 0.7
muted = false
shuffle = false
repeat = "Off"
quality = "Normal160"
fade_ms = 150
seek_step_s = 10
volume_step = 0.05

[audio]
device = ""
eq_gains_db = [
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
]
eq_preset = "Flat"
bass_db = 6.0
balance = 0.0
buffer_ms = 500
preamp_db = 0.0

[lyrics]
font_size = 22.0
bold_current = true
color = [
    150,
    170,
    185,
]
highlight = [
    94,
    234,
    212,
]
align = "Center"
line_spacing = 10.0
bg_opacity = 0.0
offset_ms = 0

[visualizer]
mode = "Bars"
fps = 30
sensitivity = 1.0
smoothing = 0.6
bars = 48
height = 140.0
color_a = [
    34,
    211,
    238,
]
color_b = [
    59,
    130,
    246,
]

[mini]
size = [
    360.0,
    120.0,
]
opacity = 0.95
layout = "Horizontal"

[overlay]
pos = [
    40.0,
    40.0,
]
size = [
    460.0,
    110.0,
]
opacity = 0.8
font_size = 18.0
always_on_top = true

[services]
spotify_enabled = true
youtube_enabled = true
local_enabled = true
spotify_client_id = ""
spotify_redirect_port = 8898
youtube_client_id = ""
local_folders = []
youtube_open_in = "YouTubeMusic"
discord_app_id = ""

[sync]
interval_min = 60
include_followed_playlists = true

[perf]
ram_target_mb = 200
artwork_max_px = 300
artwork_cache_mb = 24
idle_fps_cap = 30

[shortcuts]
like_song = "Ctrl+Alt+L"
mute = "Ctrl+Alt+M"
next = "Ctrl+Alt+Right"
open_search = "Ctrl+F"
open_settings = "Ctrl+Comma"
play_pause = "Ctrl+Alt+P"
previous = "Ctrl+Alt+Left"
seek_backward = "Ctrl+Alt+Comma"
seek_forward = "Ctrl+Alt+Period"
switch_provider = "Ctrl+Alt+S"
toggle_lyrics = "Ctrl+Alt+Y"
toggle_mini_player = "Ctrl+Alt+N"
volume_down = "Ctrl+Alt+Down"
volume_up = "Ctrl+Alt+Up"
```
</details>
