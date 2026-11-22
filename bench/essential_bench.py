#!/usr/bin/env python3
"""Landscape vs portrait vs Essential Mode, under the same playback conditions.

Each run: fresh data folder, identical settings (Balanced preset, Auto layout),
the same local queue playing (ART_DIR: FLACs with embedded cover art), the Now
Playing / Playing screen. Landscape 1240x780, portrait 400x860. Essential Mode is
entered from the landscape run with its keyboard shortcut (Ctrl+Shift+M) after
the full interface has been shown, so the measurement includes what entering it
actually releases.

Per run, after SETTLE s, sampled every 0.5 s for SAMPLE s:
  private working set ~ RssAnon, total working set ~ VmRSS, private bytes ~ VmData
  (Linux equivalents of the Windows counters), CPU = utime+stime over the sample
  window (% of one core), threads (= running background tasks + audio/GL threads).
Playback is verified in every run (PulseAudio shows an active stream).

Browse scenario (`--browse N`): with the same files as the local library, open
the Albums page (a grid of covers), then enter Essential Mode, then leave it —
memory in each state, N runs. Shows what entering releases after real browsing.

Also (`--switch N`): enter/leave Essential Mode N times in one process and record
memory after every cycle, to catch textures or caches that accumulate.

usage: bench/essential_bench.py <out.json> [runs] [--switch N] [--browse N]
env:   ART_DIR (required), SETTLE (20), SAMPLE (10), BENCH_DIR
needs: DISPLAY, PulseAudio, xdotool, target/release/aqua-glass, examples/make_config
"""
import json, os, shutil, statistics, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target/release/aqua-glass")
MAKE = os.path.join(ROOT, "target/release/examples/make_config")
OUT = sys.argv[1]
RUNS = int(sys.argv[2]) if len(sys.argv) > 2 and not sys.argv[2].startswith("--") else 3
SWITCH = int(sys.argv[sys.argv.index("--switch") + 1]) if "--switch" in sys.argv else 0
BROWSE = int(sys.argv[sys.argv.index("--browse") + 1]) if "--browse" in sys.argv else 0
SETTLE = float(os.environ.get("SETTLE", "20"))
SAMPLE = float(os.environ.get("SAMPLE", "10"))
WORK = os.environ.get("BENCH_DIR", "/tmp/aqua-essential-bench")
ART = os.environ["ART_DIR"]
FILES = sorted((os.path.join(ART, f) for f in os.listdir(ART) if f.endswith(".flac")), key=lambda p: int(os.path.basename(p).split(".")[0]))
TICK = os.sysconf("SC_CLK_TCK")


def status(pid):
    d = {}
    for line in open(f"/proc/{pid}/status"):
        k, *v = line.split()
        if k in ("RssAnon:", "VmRSS:", "VmData:"):
            d[k[:-1]] = int(v[0]) / 1024
        if k == "Threads:":
            d["Threads"] = int(v[0])
    return d


def cpu_s(pid):
    v = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    return (int(v[11]) + int(v[12])) / TICK


def stream_active():
    out = subprocess.run(["pactl", "list", "short", "sink-inputs"], capture_output=True, text=True).stdout
    return len(out.strip().splitlines()) > 0


def window():
    out = subprocess.run(["xdotool", "search", "--name", "Aqua Glass"], capture_output=True, text=True).stdout.split()
    return out[0] if out else None


def key_essential():
    w = window()
    subprocess.run(["xdotool", "windowfocus", "--sync", w], capture_output=True)
    subprocess.run(["xdotool", "key", "--window", w, "ctrl+shift+m"], capture_output=True)


def launch(tag, size, folder=None, files=None):
    home = os.path.join(WORK, f"home-{tag}")
    shutil.rmtree(home, ignore_errors=True)
    os.makedirs(home)
    extra = [f"folder={folder}"] if folder else []
    cfg = subprocess.check_output([MAKE, "Balanced", f"window={size}", "layout=Auto", *extra]).decode()
    open(os.path.join(home, "settings.toml"), "w").write(cfg)
    env = dict(os.environ, AQUA_GLASS_HOME=home)
    p = subprocess.Popen([BIN, "--page", "now-playing", *(files or FILES)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(6)
    w = window()
    if w:
        subprocess.run(["xdotool", "windowmove", w, "0", "0"], capture_output=True)
    return p


def measure(p):
    samples, playing = [], []
    c0, t0 = cpu_s(p.pid), time.time()
    while time.time() - t0 < SAMPLE:
        samples.append(status(p.pid))
        playing.append(stream_active())
        time.sleep(0.5)
    cpu = (cpu_s(p.pid) - c0) / (time.time() - t0) * 100
    m = lambda k: round(statistics.mean(s[k] for s in samples), 2)
    return {"private_ws_mb": m("RssAnon"), "total_ws_mb": m("VmRSS"), "private_bytes_mb": m("VmData"), "threads": round(statistics.mean(s["Threads"] for s in samples)), "cpu_pct_one_core": round(cpu, 2), "audio_stream_active": all(playing)}


def run(mode, idx):
    size = "400x860" if mode == "portrait" else "1240x780"
    p = launch(f"{mode}-{idx}", size)
    time.sleep(SETTLE - 6)
    if mode == "essential":
        key_essential()
        time.sleep(SETTLE)  # settle again in Essential Mode
    r = {"mode": mode, "run": idx, **measure(p)}
    p.terminate()
    p.wait(timeout=10)
    print(json.dumps(r), flush=True)
    return r


def switch_test(n):
    p = launch("switch", "1240x780")
    time.sleep(SETTLE - 6)
    cycles = []
    base = status(p.pid)
    for i in range(n):
        key_essential()
        time.sleep(3)
        ess = status(p.pid)
        key_essential()
        time.sleep(4)  # full UI again: textures reload lazily for what's on screen
        full = status(p.pid)
        cycles.append({"cycle": i + 1, "essential_private_ws_mb": round(ess["RssAnon"], 2), "landscape_private_ws_mb": round(full["RssAnon"], 2), "private_bytes_mb": round(full["VmData"], 2), "audio_stream_active": stream_active()})
        print(json.dumps(cycles[-1]), flush=True)
    p.terminate()
    p.wait(timeout=10)
    return {"start_private_ws_mb": round(base["RssAnon"], 2), "cycles": cycles}


def snap(pid):
    s = status(pid)
    return {"private_ws_mb": round(s["RssAnon"], 2), "total_ws_mb": round(s["VmRSS"], 2), "private_bytes_mb": round(s["VmData"], 2), "audio_stream_active": stream_active()}


def browse_test(n):
    out = []
    for i in range(n):
        p = launch(f"browse-{i}", "1240x780", folder=ART, files=FILES[:1])
        time.sleep(6)
        subprocess.run(["xdotool", "mousemove", "88", "336", "click", "1"], capture_output=True)  # sidebar › Albums
        time.sleep(8)
        albums = snap(p.pid)
        key_essential()
        time.sleep(12)
        ess = snap(p.pid)
        key_essential()
        time.sleep(8)
        back = snap(p.pid)
        p.terminate()
        p.wait(timeout=10)
        out.append({"run": i, "albums_page": albums, "essential": ess, "back_on_albums": back})
        print(json.dumps(out[-1]), flush=True)
    return out


results = []
for i in range(RUNS):
    for mode in ["landscape", "portrait", "essential"]:  # interleaved so drift affects all equally
        results.append(run(mode, i))
summary = {}
for mode in ["landscape", "portrait", "essential"]:
    rs = [r for r in results if r["mode"] == mode]
    summary[mode] = {k: {"mean": round(statistics.mean(r[k] for r in rs), 2), "min": min(r[k] for r in rs), "max": max(r[k] for r in rs)} for k in ["private_ws_mb", "total_ws_mb", "private_bytes_mb", "cpu_pct_one_core", "threads"]}
    summary[mode]["all_runs_playing"] = all(r["audio_stream_active"] for r in rs)
out = {"music": f"{len(FILES)} FLACs with embedded 600x600 covers", "settle_s": SETTLE, "sample_s": SAMPLE, "runs": RUNS, "results": results, "summary": summary}
if SWITCH:
    out["switching"] = switch_test(SWITCH)
if BROWSE:
    out["browse"] = browse_test(BROWSE)
json.dump(out, open(OUT, "w"), indent=1)
print(json.dumps(summary, indent=1))
