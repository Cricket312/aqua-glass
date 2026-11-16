# Benchmarks

All numbers on this page are **real measurements**, produced by the scripts in `bench/`
with the raw output committed next to them (`bench/results-linux-dev.json`,
`bench/soak-linux-dev.json`). They were taken in the **Linux development environment**
described below — not on Windows 11. Treat them as indicative of Aqua Glass' own
footprint; Windows numbers will differ (see "How this maps to Windows"). The app's
Performance page reports the real Windows values on your machine, and the per-feature
RAM badges switch from "Estimated" to "Measured" as you use features.

## Environment

| | |
|---|---|
| Date | 2026-11-14 |
| Build | `cargo build --release` (opt-level 3, fat LTO, codegen-units 1, panic=abort), commit in this repo |
| OS | Ubuntu 24.04.5 LTS, kernel 6.18 (container) |
| CPU | Intel Xeon @ 2.10 GHz, 4 vCPUs |
| RAM | 15 GiB |
| Display | Xvfb 1400×900×24, no window manager, no compositor |
| OpenGL | Mesa llvmpipe (LLVM 20, **software rendering**) |
| Audio | PulseAudio 16.1 `module-null-sink` (real-time paced) via ALSA pulse plugin |
| Window | 1240×780 logical px, scale 1.0 |
| Media | 4 generated 44.1 kHz stereo WAV files (8–12 s), playing in a loop; 2560×1440 PNG background |
| Network | no access to Spotify/Google/LRCLIB (YouTube Music was still supported then) (services signed out) |

Metric: **private memory = `RssAnon`** (anonymous resident memory) from `/proc/<pid>/status`,
the closest Linux equivalent of the Windows *private working set* that Aqua Glass shows.
`VmRSS` (all resident pages incl. shared libraries and the Mesa driver) is listed for
completeness.

## Procedure

```bash
cargo build --release && cargo build --release --examples
# virtual display + paced null audio sink
Xvfb :99 -screen 0 1400x900x24 &
pulseaudio --exit-idle-time=-1 --daemonize=yes -n --load="module-null-sink sink_name=nullsink" --load=module-native-protocol-unix
printf 'pcm.!default { type pulse }\nctl.!default { type pulse }\n' > ~/.asoundrc
# scenarios: fresh data folder each, 12 s settle, then 5 s of samples every 250 ms
DISPLAY=:99 SETTLE=12 SAMPLE=5 python3 bench/run_bench.py <music-dir> bench/results-linux-dev.json
# soak: 10 minutes, sample every 15 s
DISPLAY=:99 python3 bench/soak.py <music-dir> 10 bench/soak-linux-dev.json Enhanced +bg_image +visualizer
```

`bench/run_bench.py` writes a `settings.toml` for each scenario with
`examples/make_config.rs` (preset + extra toggles), launches the release binary with
`--page <page> <audio files…>`, waits, samples, and terminates it.

## Version 0.3.0 (reference design)

Same procedure, raw data in `bench/results-linux-dev-v0.3.json`. Private memory is unchanged by
the concept-matched interface: Minimal idle **41.4 MB**, Balanced playing **41.9 MB**, Now
Playing **42.9 MB**, Enhanced **44.2 MB**, Fully Customized + background image **45.6 MB**.
The new static background gradient costs no memory; with software rendering (llvmpipe) its
large translucent areas add fill-rate cost on each repaint (on a GPU this is negligible).
CPU figures in this run were noisy (e.g. Minimal idle read higher than Balanced idle), so no
CPU conclusions are drawn from it.

## Version 0.2.0 (redesign + Similarity Radio)

Same procedure and environment, raw data in `bench/results-linux-dev-v0.2.json`:

| Scenario | 0.1.0 | **0.2.0** |
|---|---|---|
| Minimal, idle | 41.3 MB | **41.3 MB** |
| Minimal, playing | 41.4 MB | **41.9 MB** |
| Balanced (default), idle | 41.3 MB | **41.6 MB** |
| Balanced, playing (Home) | 41.5 MB | **41.8 MB** |
| Balanced, playing (Now Playing) | 49.8 MB | **42.8 MB** |
| Enhanced, Now Playing | 51.2 MB | **43.9 MB** |
| Fully Customized + background image + Discord | 53.5 MB | **45.4 MB** |
| Background image (Δ vs baseline) | +16.1 MB | **+10.8 MB** |

The first 0.2.0 run showed Now Playing at 66.8 MB: the placeholder artwork glyph was being
rasterised at ~180 px, which grows egui's font-atlas texture. Capping it at 48 px fixed the
regression and also removed most of the old 0.1.0 Now Playing overhead.

**Similarity Radio:** search → play a song with the 24-track tagged local library, radio on
vs. off, 12 s after starting, mean of 5 samples: **50.3 MB vs 50.2 MB** (+0.1 MB; both runs
include the open right panel and search results). The radio is rated Low (+0.5–3 MB
estimate) and its worker only exists while enabled.

## Results: operating conditions (0.1.0)

| Scenario | Page | Private memory | VmRSS |
|---|---|---|---|
| Minimal preset, idle (nothing playing) | Home | **41.3 MB** | 123.6 MB |
| Minimal preset, playing | Home | **41.4 MB** | 124.0 MB |
| Balanced (default), idle | Home | **41.3 MB** | 124.0 MB |
| Balanced (default), playing | Home | **41.5 MB** | 124.1 MB |
| Balanced, playing | Now Playing | **49.8 MB** | 133.3 MB |
| Enhanced, playing (visualizer, blur, lyrics, …) | Now Playing | **51.2 MB** | 135.2 MB |
| Fully Customized + background image + Discord, playing | Now Playing | **53.5 MB** | 140.0 MB |

Against the targets (minimal idle < 75 MB, normal idle < 100 MB, playback 80–150 MB,
moderate customization < 200 MB) every scenario is well inside the budget in this
environment. Most of the ~8 MB step from Home to Now Playing is the software renderer
(llvmpipe) rasterising the large artwork area into CPU memory.

## Results: individual features

Baseline: Minimal preset, playing, Now Playing page (49.4 MB). Each row enables the listed
toggles on top of the baseline (mean of 20 samples).

| Feature(s) | Δ private memory | Notes |
|---|---|---|
| Audio Visualizer | +0.3 MB | FFT + analyzer buffers |
| Visualizer + player-bar spectrum | +0.7 MB | |
| Custom Background Image (2560×1440 PNG) | **+16.1 MB** | downscaled to window size, one RGBA texture (in CPU memory with llvmpipe) |
| Background Image + Blur | +1.2 MB | blurred image is kept at ≤ 192 px — blur *saves* memory |
| Animated Background | +0.0 MB | no RAM, but continuous repaints (CPU/GPU) |
| Background Image + Transitions | +16.2 MB | previous texture only held during a fade |
| Custom theme + translucent panels + shadows + animations + hover | +0.2 MB | |
| Custom Font (DejaVu Sans, 0.7 MB file) | +0.8 MB | grows with the font file and glyph atlas |
| Desktop Overlay | −0.1 MB | *not meaningful here*: Xvfb has no compositor; measure on Windows |
| Equalizer + Bass Boost | +0.1 MB | |
| Discord Rich Presence (Discord not running) | +0.1 MB | worker thread only |
| Lyrics (offline) | +0.2 MB | no lyrics could be downloaded here |
| Artwork Thumbnails in Lists | +0.0 MB | *not meaningful here*: local test files have no artwork |

These measurements were used to calibrate the estimates in `src/features.rs`
(e.g. Blurred Background was re-rated from High to Low, Animated Background and Visualizer
from Moderate to Low). Features whose cost could not be observed in this environment keep
their engineering estimate and are labelled "Estimated" in the app until measured on the
user's PC.

## CPU

CPU seconds consumed over the 17-second run (includes ~0.6 s of start-up):

| Scenario | CPU time / 17 s |
|---|---|
| Minimal idle | 0.71 s |
| Balanced playing (Home) | 1.43 s |
| Balanced playing (Now Playing) | 1.55 s |
| Visualizer on (30 FPS) | 11.4 s |
| Animated background (30 FPS) | 17.2 s |

After start-up, idle and plain playback cost well under 1 % of one core: the UI only
repaints when something changes (twice a second for the progress bar while playing).
Visualizer and animated background repaint the whole window at their FPS; with
**software** rendering (llvmpipe) that costs most of a core here. With a GPU the same
frames are far cheaper, but these remain the most CPU/GPU-hungry options, which is why
they are off in the Minimal and Balanced presets and their FPS is configurable.

An earlier build had a bug where the desktop overlay forced continuous repaints (≈ 270 %
CPU); it was found with this benchmark and fixed (now ≈ 6 % of one core with the overlay
open in this software-rendered environment, 2 repaints/s).

## Soak test (leak check)

**10-minute playback soak** (Enhanced preset + background image + visualizer, Now Playing
page, repeat-all over 4 tracks ≈ 65 track changes, sample every 15 s):
private memory **51.6 MB at 15 s → 51.8 MB at 600 s** (min 51.6, max 51.8). No growth trend.
Raw data: `bench/soak-linux-dev.json`.

**Toggle stress** (`bench/toggle_stress.sh`, driven through the real UI with xdotool):
40 cycles of switching Visualizer, Blurred Background, Animated Background and the Desktop
Overlay window on/off (160 toggles, overlay window created and destroyed 20 times) with a
background image and playback running: private memory **65.7 MB before → 66.7 MB after**,
unchanged from cycle 5 to cycle 40 (allocator high-water mark, no accumulation). Raw
output: `bench/toggle-stress-linux-dev.txt`.

## How this maps to Windows

* The private-memory figures exclude shared libraries, as does the Windows private
  working set. Aqua Glass' own heap (UI state, audio buffers, caches, librespot when
  signed in) is platform-independent.
* With a hardware GPU driver on Windows, texture memory is mostly accounted to the GPU /
  driver rather than private CPU memory, so texture-heavy features (background images)
  will typically look *cheaper* than here; the OpenGL driver itself adds its own private
  allocations (typically tens of MB, depending on the vendor).
* librespot (Spotify playback) adds its session, decryption and decoder buffers once
  signed in; this could not be measured here (no Spotify access) — the in-app dashboard
  shows it on your PC.

## v0.4.0 (architecture preparation) — A/B check

The workspace split and sync models add no threads or timers. To check for a regression,
both Windows release builds were run the same way under Wine 9 / Xvfb:

- portable mode;
- playing two local WAV files;
- Home page;
- sampled 30–40 s after start.

| Build | RssAnon (private, incl. Wine) | VmRSS |
|---|---|---|
| 0.3.2 | 76.3 MB | 164.2 MB |
| 0.4.0 | 76.6 MB | 164.3 MB |

The +0.3 MB difference is within run-to-run variation. Under Wine these numbers include
Wine's own allocations, so compare only the two builds with each other. Neither is the
native Windows figure; see the Linux numbers above for the app's own footprint. The
200 MB target is unchanged.

> Discord Rich Presence was removed in 0.7.4. The rows above that mention it are kept as
> historical measurements.
