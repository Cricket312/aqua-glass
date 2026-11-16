#!/usr/bin/env python3
"""Landscape vs portrait memory benchmark (same playback conditions).

Each run: fresh data folder, identical settings (one preset, Auto layout), the
same 500-track local queue playing, the Now Playing / Playing screen — only the
window size differs (landscape 1240x780, portrait 400x860). After SETTLE seconds
the process is sampled every 0.5 s for SAMPLE seconds.

Linux equivalents of the Windows counters (this is the development platform):
  private working set  ~ RssAnon            (/proc/<pid>/status)
  private bytes        ~ VmData             (/proc/<pid>/status: private writable mappings = commit)
  private resident     = Private_Clean + Private_Dirty (/proc/<pid>/smaps_rollup), cross-check
Playback is verified per run: PulseAudio must show an active stream from the app.

Env: ART_DIR=<folder of FLACs with embedded covers> to measure with artwork,
     PRESETS=Balanced,FullyCustomized

usage: bench/layout_bench.py <out.json> [runs]   (needs DISPLAY, PulseAudio, target/release/aqua-glass)
"""
import json, os, shutil, statistics, subprocess, sys, time, wave, struct, math

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target/release/aqua-glass")
MAKE = os.path.join(ROOT, "target/release/examples/make_config")
OUT = sys.argv[1]
RUNS = int(sys.argv[2]) if len(sys.argv) > 2 else 3
SETTLE = float(os.environ.get("SETTLE", "20"))
SAMPLE = float(os.environ.get("SAMPLE", "10"))
WORK = os.environ.get("BENCH_DIR", "/tmp/aqua-layout-bench")

def make_music():
    d = os.path.join(WORK, "music")
    os.makedirs(d, exist_ok=True)
    src = os.path.join(d, "tone.wav")
    if not os.path.exists(src):
        w = wave.open(src, "wb"); w.setnchannels(2); w.setsampwidth(2); w.setframerate(44100)
        frames = b"".join(struct.pack("<hh", int(5000 * math.sin(2 * math.pi * 330 * i / 44100)), int(5000 * math.sin(2 * math.pi * 440 * i / 44100))) for i in range(44100 * 600))
        w.writeframes(frames); w.close()
    files = []
    for i in range(500):
        p = os.path.join(d, f"track{i:03}.wav")
        if not os.path.exists(p):
            os.symlink(src, p)
        files.append(p)
    return files

def status(pid):
    d = {}
    for line in open(f"/proc/{pid}/status"):
        k, *v = line.split()
        if k in ("RssAnon:", "VmRSS:", "VmData:"):
            d[k[:-1]] = int(v[0]) / 1024
    priv = 0
    for line in open(f"/proc/{pid}/smaps_rollup"):
        if line.startswith(("Private_Clean:", "Private_Dirty:")):
            priv += int(line.split()[1])
    d["Private"] = priv / 1024
    return d

def stream_active():
    out = subprocess.run(["pactl", "list", "short", "sink-inputs"], capture_output=True, text=True).stdout
    return len(out.strip().splitlines()) > 0

def run(layout, preset, files, idx):
    home = os.path.join(WORK, f"home-{layout}-{preset}-{idx}")
    shutil.rmtree(home, ignore_errors=True)
    os.makedirs(home)
    size = "1240x780" if layout == "landscape" else "400x860"
    cfg = subprocess.check_output([MAKE, preset, f"window={size}", "layout=Auto"]).decode()
    open(os.path.join(home, "settings.toml"), "w").write(cfg)
    env = dict(os.environ, AQUA_GLASS_HOME=home)
    p = subprocess.Popen([BIN, "--page", "now-playing", *files], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(SETTLE)
    samples, playing = [], []
    t_end = time.time() + SAMPLE
    while time.time() < t_end:
        samples.append(status(p.pid))
        playing.append(stream_active())
        time.sleep(0.5)
    p.terminate(); p.wait(timeout=10)
    m = lambda k: round(statistics.mean(s[k] for s in samples), 2)
    r = {"layout": layout, "preset": preset, "run": idx, "private_ws_mb": m("RssAnon"), "private_bytes_mb": m("VmData"),
         "private_resident_mb": m("Private"), "rss_mb": m("VmRSS"), "audio_stream_active": all(playing)}
    print(json.dumps(r), flush=True)
    return r

ART_DIR = os.environ.get("ART_DIR")  # folder of tagged FLACs with embedded covers
PRESETS = os.environ.get("PRESETS", "Balanced,FullyCustomized").split(",")
files = sorted(os.path.join(ART_DIR, f) for f in os.listdir(ART_DIR) if f.endswith(".flac")) if ART_DIR else make_music()
results = []
for preset in PRESETS:
    for i in range(RUNS):
        for layout in ["landscape", "portrait"]:  # interleaved so drift affects both equally
            results.append(run(layout, preset, files, i))
summary = {}
for preset in PRESETS:
    for layout in ["landscape", "portrait"]:
        rs = [r for r in results if r["preset"] == preset and r["layout"] == layout]
        summary[f"{preset}/{layout}"] = {k: {"mean": round(statistics.mean(r[k] for r in rs), 2), "min": min(r[k] for r in rs), "max": max(r[k] for r in rs)} for k in ["private_ws_mb", "private_bytes_mb", "private_resident_mb", "rss_mb"]}
        summary[f"{preset}/{layout}"]["all_runs_playing"] = all(r["audio_stream_active"] for r in rs)
json.dump({"music": "tagged FLACs with embedded 600x600 covers" if ART_DIR else "500 untagged WAVs (no artwork)", "settle_s": SETTLE, "sample_s": SAMPLE, "runs": RUNS, "results": results, "summary": summary}, open(OUT, "w"), indent=1)
print(json.dumps(summary, indent=1))
