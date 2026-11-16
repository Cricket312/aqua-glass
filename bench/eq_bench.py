#!/usr/bin/env python3
"""End-to-end check of Aqua Glass' built-in EQ on the real output, plus its
CPU / memory cost in the running app.

Each run: fresh data folder, the same local file playing (tones at 62 Hz, 1 kHz
and 8 kHz), Now Playing page. Meanwhile ANOTHER app (paplay) plays a 3150 Hz tone
on the same output device. The device's mix is recorded from the PulseAudio
monitor and the level of every tone is measured by FFT.

Configs: "off" (EQ and bass boost features off) and "on" (EQ +9 dB at 62 Hz,
-9 dB at 8 kHz, bass boost +6 dB, pre-amp -12 dB). Expected changes come from
the processor's own curve (`dsp_bench response`), at the device rate.

Checks: Aqua Glass' tones change by the expected amounts; the other app's tone
does not change at all. Also samples CPU (utime+stime) and private memory
(RssAnon / VmData, as in layout_bench.py) while playing.

Env: SINK=<PulseAudio sink, the default output> RATE=<its sample rate>

usage: bench/eq_bench.py <out.json> [runs]  (needs DISPLAY, PulseAudio null sink,
       target/release/aqua-glass, target/release/examples/{make_config,dsp_bench})
"""
import json, math, os, shutil, statistics, struct, subprocess, sys, time, wave
import numpy as np

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target/release/aqua-glass")
MAKE = os.path.join(ROOT, "target/release/examples/make_config")
DSPB = os.path.join(ROOT, "target/release/examples/dsp_bench")
OUT = sys.argv[1]
RUNS = int(sys.argv[2]) if len(sys.argv) > 2 else 3
WORK = os.environ.get("BENCH_DIR", "/tmp/aqua-eq-bench")
SETTLE, RECORD, CPU_S = 10.0, 4.0, 20.0
RATE = int(os.environ.get("RATE", "48000"))  # must match the sink's rate
SINK = os.environ.get("SINK", "null")
TONES = [62.0, 1000.0, 8000.0]
OTHER = 3150.0
EQ = "0,9,0,0,0,0,0,0,-9,0"
BASS, PREAMP = 6.0, -12.0
CONFIGS = {
    "off": ["-eq", "-bass_boost"],
    "on": ["+eq", "+bass_boost", f"eq={EQ}", f"bass={BASS}", f"preamp={PREAMP}"],
}

def write_wav(path, freqs, amp, seconds, rate=44100):
    if os.path.exists(path):
        return
    n = rate * seconds
    t = np.arange(n) / rate
    x = sum(amp * np.sin(2 * np.pi * f * t) for f in freqs)
    pcm = (np.clip(x, -1, 1) * 32767).astype("<i2")
    st = np.repeat(pcm, 2)
    w = wave.open(path, "wb"); w.setnchannels(2); w.setsampwidth(2); w.setframerate(rate)
    w.writeframes(st.tobytes()); w.close()

def level_db(x, f):
    win = np.hanning(len(x))
    spec = np.abs(np.fft.rfft(x * win))
    b = int(round(f * len(x) / RATE))
    return 20 * math.log10(spec[b - 3:b + 4].max() / (win.sum() / 2) + 1e-12)

def proc_stat(pid):
    v = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    return (int(v[11]) + int(v[12])) / os.sysconf("SC_CLK_TCK")

def mem(pid):
    d = {}
    for line in open(f"/proc/{pid}/status"):
        k, *v = line.split()
        if k in ("RssAnon:", "VmData:"):
            d[k[:-1]] = int(v[0]) / 1024
    return d

def run(name, music, other, idx):
    home = os.path.join(WORK, f"home-{name}-{idx}")
    shutil.rmtree(home, ignore_errors=True); os.makedirs(home)
    cfg = subprocess.check_output([MAKE, "Balanced", "volume=1.0", *CONFIGS[name]]).decode()
    open(os.path.join(home, "settings.toml"), "w").write(cfg)
    pa = subprocess.Popen(["paplay", "-d", SINK, other])  # the "other application"
    p = subprocess.Popen([BIN, "--page", "now-playing", music], env=dict(os.environ, AQUA_GLASS_HOME=home),
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(SETTLE)
    raw = os.path.join(WORK, f"rec-{name}-{idx}.f32")
    subprocess.run(["timeout", str(RECORD + 0.5), "parecord", "-d", f"{SINK}.monitor", "--raw", "--format=float32le",
                    f"--rate={RATE}", "--channels=2", raw])
    c0, t0, mems = proc_stat(p.pid), time.time(), []
    while time.time() - t0 < CPU_S:
        mems.append(mem(p.pid)); time.sleep(0.5)
    cpu = (proc_stat(p.pid) - c0) / (time.time() - t0) * 100
    streams = len(subprocess.run(["pactl", "list", "short", "sink-inputs"], capture_output=True, text=True).stdout.strip().splitlines())
    p.terminate(); p.wait(timeout=10); pa.terminate(); pa.wait()
    x = np.fromfile(raw, dtype="<f4").reshape(-1, 2)
    x = x[RATE // 2: RATE // 2 + int(RECORD * RATE * 0.75)]  # skip the recorder's start-up
    left = x[:, 0].astype(np.float64)
    r = {"config": name, "run": idx, "levels_db": {str(int(f)): round(level_db(left, f), 2) for f in TONES + [OTHER]},
         "cpu_pct_one_core": round(cpu, 2), "private_ws_mb": round(statistics.mean(m["RssAnon"] for m in mems), 2),
         "private_bytes_mb": round(statistics.mean(m["VmData"] for m in mems), 2), "streams_playing": streams}
    print(json.dumps(r), flush=True)
    return r

os.makedirs(WORK, exist_ok=True)
music = os.path.join(WORK, "aqua-tones.wav"); write_wav(music, TONES, 0.08, 300)
other = os.path.join(WORK, "other-app-3150.wav"); write_wav(other, [OTHER], 0.1, 120)
expected = {}
for line in subprocess.check_output([DSPB, "response", EQ, str(BASS), str(RATE), *map(str, TONES)]).decode().split("\n"):
    if line:
        f, d = line.split(); expected[str(int(float(f)))] = round(float(d) + PREAMP, 2)
expected[str(int(OTHER))] = 0.0
results = [run(n, music, other, i) for i in range(RUNS) for n in ["off", "on"]]
mean = lambda n, k: statistics.mean(r[k] for r in results if r["config"] == n)
summary = {"expected_change_db": expected, "measured_change_db": {}, "cpu_pct_one_core": {}, "private_ws_mb": {}, "private_bytes_mb": {}}
for f in expected:
    summary["measured_change_db"][f] = round(statistics.mean(r["levels_db"][f] for r in results if r["config"] == "on")
                                             - statistics.mean(r["levels_db"][f] for r in results if r["config"] == "off"), 2)
for k in ["cpu_pct_one_core", "private_ws_mb", "private_bytes_mb"]:
    for n in CONFIGS:
        vals = [r[k] for r in results if r["config"] == n]
        summary[k][n] = {"mean": round(statistics.mean(vals), 2), "min": min(vals), "max": max(vals)}
json.dump({"device_rate": RATE, "runs": RUNS, "eq": EQ, "bass_db": BASS, "preamp_db": PREAMP, "results": results, "summary": summary}, open(OUT, "w"), indent=1)
print(json.dumps(summary, indent=1))
