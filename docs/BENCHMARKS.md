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

## v0.8.0 — landscape vs portrait (same playback conditions)

`bench/layout_bench.py` runs the release build with identical settings and conditions:

* the same preset, Auto layout and Now Playing / Playing screen;
* the same queue, playing through PulseAudio's null sink, with an active audio stream
  verified in every run;
* only the window size differs: 1240×780 for landscape, 400×860 for portrait.

Each layout ran 3 times, interleaved, sampled every 0.5 s for 10 s after 20 s of settling.
The run-to-run spread was ≤ 0.25 MB. Raw data:
`bench/results-layout-linux-dev.json` and `bench/results-layout-artwork-linux-dev.json`.

Linux equivalents of the Windows counters (this is the development environment; the
Windows build was not benchmarked, because Wine doesn't report private memory):

* private working set ≈ `RssAnon`
* private bytes ≈ `VmData` (private writable mappings, i.e. commit)

| Queue | Counter | Landscape | Portrait | Portrait − landscape |
|---|---|---|---|---|
| 500 untagged WAVs (no covers), Balanced | private working set | 44.21 MB | 42.93 MB | **-1.28 MB** |
| 500 untagged WAVs (no covers), Balanced | private bytes | 168.33 MB | 167.01 MB | **-1.32 MB** |
| 500 untagged WAVs (no covers), FullyCustomized | private working set | 44.72 MB | 43.18 MB | **-1.54 MB** |
| 500 untagged WAVs (no covers), FullyCustomized | private bytes | 170.94 MB | 169.46 MB | **-1.48 MB** |
| 40 FLACs with 600×600 covers, Balanced | private working set | 43.22 MB | 43.88 MB | **+0.66 MB** |
| 40 FLACs with 600×600 covers, Balanced | private bytes | 167.59 MB | 168.03 MB | **+0.44 MB** |
| 40 FLACs with 600×600 covers, FullyCustomized | private working set | 43.80 MB | 44.10 MB | **+0.30 MB** |
| 40 FLACs with 600×600 covers, FullyCustomized | private bytes | 170.17 MB | 170.28 MB | **+0.11 MB** |

Reading the numbers:

* **Without covers, portrait is about 1.3–1.5 MB smaller.** No sidebar, no side panel and
  a smaller window, so smaller frame buffers. The renderer here is Mesa llvmpipe (software),
  whose frame buffers live in process memory; with a GPU on Windows part of that moves to
  video memory, so the difference there is likely smaller.
* **With covers, portrait is 0.3–0.7 MB larger.** Its Queue tab shows cover thumbnails of the
  visible rows, which landscape's Now Playing (side panel closed) doesn't show at all. The
  first measurement of this was +3.3–3.6 MB, because list thumbnails were loaded at full
  cover size (300 px). Since 0.8.0 rows use 128 px thumbnails (`Kind::Thumb`), in both
  layouts.
* Both layouts stay around 43–45 MB private, far below the 200 MB target.
* Switching layouts doesn't touch playback: the engine and providers are shared, and audio
  kept playing straight through in the switch tests.

## v0.9.0 — built-in EQ (Equalizer APO integration removed)

### Processing cost (`examples/dsp_bench.rs`)

60 s of 48 kHz stereo noise was processed in 480-frame callbacks (10 ms, a typical
WASAPI shared-mode period). The CPU was an Intel Xeon @ 2.10 GHz (4 vCPU VM). Each case
ran 3 times; the table shows mean time per callback, the range across the 3 runs, and
the share of one core for real-time playback. A counting global allocator checks for
heap allocations in the processing loop.

| Case | 0.8 engine (Mutex `try_lock`, f32) | 0.9 engine (lock-free, f64, smoothing) |
|---|---|---|
| EQ off (bypass) | 0.03–0.04 µs · 0.000 % | 0.03–0.04 µs · 0.000 % |
| Pre-amp only | 0.13–0.16 µs · 0.002 % | 0.87–1.04 µs · 0.010 % |
| 10 bands + bass + pre-amp + balance | 14.0–15.6 µs · 0.14–0.16 % | 13.5–15.5 µs · 0.13–0.16 % |
| …while sliders change every 20 ms | 12.3–14.0 µs · 0.12–0.14 % | 14.1–16.6 µs · 0.14–0.17 % |
| Heap allocations in the audio path | 0 | 0 |
| Processor state / shared settings | 480 B / 72 B | 1 360 B / 80 B |

What this shows:

* With everything on, the new engine costs the same as the old one: about 0.15 % of one
  core, even though it now uses f64 biquads and glides parameter changes.
* Pre-amp only costs a little more, because the gain is applied per 32-frame block, which
  lets it ramp. It is still about 1 µs per 10 ms.
* Single callbacks occasionally took up to ≈0.5 ms in both engines. That is scheduling
  noise on the shared VM; it stays well below the 10 ms budget.

### End to end in the running app (`bench/eq_bench.py`)

Setup:

* The release build plays a local WAV containing tones at 62 Hz, 1 kHz and 8 kHz.
* At the same time **another application** (`paplay`) plays a 3150 Hz tone on the same
  output device.
* The device's mix is recorded from the PulseAudio monitor and each tone is measured by
  FFT.
* Two configurations are compared: "off" (EQ and bass boost off), and "on" (EQ +9 dB at
  62 Hz and −9 dB at 8 kHz, bass boost +6 dB, pre-amp −12 dB).
* The expected change is the processor's own curve at the device rate
  (`dsp_bench response`).
* 3 interleaved runs at 48 kHz. Raw data: `bench/results-eq-linux-dev.json`.

| Tone | Expected change | Measured change (mean of 3) |
|---|---|---|
| 62 Hz (Aqua Glass) | +2.57 dB | **+2.57 dB** |
| 1 kHz (Aqua Glass) | −12.05 dB | **−12.05 dB** |
| 8 kHz (Aqua Glass) | −21.00 dB | **−21.00 dB** |
| 3150 Hz (**other app**, same device) | 0 dB | **0.00 dB** |

The recorded output matches the curve to 0.01 dB, and the other app's audio is untouched.
The same run with the PulseAudio sink set to 44.1 kHz and to 96 kHz (1 run each) gave the
same changes to within 0.01 dB. This is **not** a test of other output rates, though:
`pactl` showed Aqua Glass' stream still at 48 kHz, because PulseAudio's ALSA plug-in offers
48 kHz and resamples. The processor itself is tested at 44.1, 48 and 96 kHz by unit tests
(`measured_response_matches_curve_at_common_rates`). On a real Windows device it runs at
whatever rate WASAPI's shared-mode mix format uses.

Whole app while playing (Balanced preset, Now Playing; CPU = utime+stime over 20 s; memory
sampled every 0.5 s, 3 runs):

| | EQ off | EQ on | Difference |
|---|---|---|---|
| CPU (% of one core) | 8.51 (8.34–8.69) | 8.37 (8.19–8.64) | within run-to-run noise |
| Private working set (`RssAnon`) | 42.96 MB (42.89–43.06) | 42.93 MB (42.92–42.94) | within noise (< 0.1 MB) |
| Private bytes (`VmData`) | 167.43 MB | 167.34 MB | within noise (< 0.1 MB) |

* **The EQ's cost doesn't show up at app level.** The UI's rendering dominates the app's
  CPU use, and the EQ's real cost (≈0.15 % of one core, about 1.4 KB of state) is smaller
  than the run-to-run spread.
* **This was measured on Linux.** The Windows build was not measured: Wine doesn't report
  private memory. The processing code is the same on Windows; only the cpal backend
  (WASAPI shared mode) differs.
