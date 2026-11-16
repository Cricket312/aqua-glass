#!/usr/bin/env python3
"""Reproducible memory benchmark for Aqua Glass.

Launches the release binary once per scenario with a fresh data folder,
plays the given audio files, waits for things to settle and samples the
process' private memory (Linux: RssAnon from /proc; the Windows equivalent
is the private working set shown in the app's Performance page).

usage: bench/run_bench.py <music-dir> <out.json>   (needs DISPLAY and an audio device)
"""
import json, os, subprocess, sys, time, glob, shutil, statistics

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target/release/aqua-glass")
MAKE = os.path.join(ROOT, "target/release/examples/make_config")
music = sys.argv[1]
out_path = sys.argv[2]
files = sorted(glob.glob(os.path.join(music, "*.wav")))
bg = os.path.join(music, "background.png")
font = next(iter(glob.glob("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")), "")
SETTLE = float(os.environ.get("SETTLE", "14"))
SAMPLE = float(os.environ.get("SAMPLE", "6"))

def mem(pid):
    d = {}
    for line in open(f"/proc/{pid}/status"):
        k, *v = line.split()
        if k in ("RssAnon:", "VmRSS:", "VmData:"):
            d[k[:-1]] = int(v[0]) / 1024
    return d

def run(name, preset, toggles, page="home", play=True, extra=()):
    home = f"/tmp/claude-0/bench-home/{name}"
    shutil.rmtree(home, ignore_errors=True)
    os.makedirs(home)
    cfg = subprocess.check_output([MAKE, preset, *toggles, f"bg_image_path={bg}", f"font={font}", f"folder={music}", *extra]).decode()
    open(os.path.join(home, "settings.toml"), "w").write(cfg)
    env = dict(os.environ, AQUA_GLASS_HOME=home)
    args = [BIN, "--page", page] + (files if play else [])
    p = subprocess.Popen(args, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(SETTLE)
    samples = []
    t_end = time.time() + SAMPLE
    while time.time() < t_end:
        samples.append(mem(p.pid))
        time.sleep(0.25)
    cpu = open(f"/proc/{p.pid}/stat").read().rsplit(")", 1)[1].split()
    ticks = int(cpu[11]) + int(cpu[12])
    p.terminate()
    p.wait(timeout=10)
    anon = [s["RssAnon"] for s in samples]
    r = {"name": name, "preset": preset, "toggles": toggles, "page": page, "playing": play,
         "private_mb_mean": round(statistics.mean(anon), 1), "private_mb_max": round(max(anon), 1),
         "rss_mb_mean": round(statistics.mean(s["VmRSS"] for s in samples), 1),
         "cpu_seconds_total": ticks / 100.0}
    print(json.dumps(r), flush=True)
    return r

results = []
results.append(run("minimal-idle", "Minimal", [], play=False))
results.append(run("minimal-playback", "Minimal", []))
results.append(run("balanced-idle", "Balanced", [], play=False))
results.append(run("balanced-playback", "Balanced", []))
results.append(run("balanced-nowplaying", "Balanced", [], page="now-playing"))
results.append(run("enhanced-nowplaying", "Enhanced", [], page="now-playing"))
results.append(run("full-nowplaying", "FullyCustomized", ["+bg_image"], page="now-playing"))
base = run("feature-baseline", "Minimal", [], page="now-playing")
results.append(base)
for feats in (["+visualizer"], ["+visualizer", "+vis_in_bar"], ["+bg_image"], ["+bg_image", "+bg_blur"], ["+bg_animated"],
              ["+bg_image", "+bg_transitions"], ["+custom_theme", "+translucent_panels", "+shadows", "+animations", "+hover_effects"],
              ["+custom_font"], ["+overlay"], ["+eq", "+bass_boost"], ["+lyrics"], ["+artwork_in_lists"]):
    r = run("feature:" + ",".join(f[1:] for f in feats), "Minimal", feats, page="now-playing")
    r["delta_vs_baseline_mb"] = round(r["private_mb_mean"] - base["private_mb_mean"], 1)
    results.append(r)
json.dump({"generated": time.strftime("%Y-%m-%d %H:%M"), "settle_s": SETTLE, "sample_s": SAMPLE, "results": results}, open(out_path, "w"), indent=1)
