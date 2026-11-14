#!/usr/bin/env python3
"""Long-running playback soak test: samples private memory over time to detect leaks.
usage: bench/soak.py <music-dir> <minutes> <out.json> [preset] [+feature ...]"""
import json, os, subprocess, sys, time, glob, shutil
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
music, minutes, out = sys.argv[1], float(sys.argv[2]), sys.argv[3]
preset = sys.argv[4] if len(sys.argv) > 4 else "Enhanced"
extra = sys.argv[5:]
home = "/tmp/claude-0/soak-home"
shutil.rmtree(home, ignore_errors=True); os.makedirs(home)
cfg = subprocess.check_output([os.path.join(ROOT, "target/release/examples/make_config"), preset, *extra, "repeat=all",
                               f"bg_image_path={os.path.join(music, 'background.png')}", f"folder={music}"]).decode()
open(os.path.join(home, "settings.toml"), "w").write(cfg)
files = sorted(glob.glob(os.path.join(music, "*.wav")))
p = subprocess.Popen([os.path.join(ROOT, "target/release/aqua-glass"), "--page", "now-playing", *files],
                     env=dict(os.environ, AQUA_GLASS_HOME=home), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
samples = []
t0 = time.time()
while time.time() - t0 < minutes * 60:
    time.sleep(15)
    st = {l.split()[0][:-1]: int(l.split()[1]) / 1024 for l in open(f"/proc/{p.pid}/status") if l.startswith(("RssAnon", "VmRSS"))}
    stat = open(f"/proc/{p.pid}/stat").read().rsplit(")", 1)[1].split()
    samples.append({"t_s": round(time.time() - t0), "private_mb": round(st["RssAnon"], 1), "rss_mb": round(st["VmRSS"], 1), "cpu_s": (int(stat[11]) + int(stat[12])) / 100})
    print(json.dumps(samples[-1]), flush=True)
p.terminate(); p.wait()
json.dump({"preset": preset, "extra": extra, "minutes": minutes, "samples": samples}, open(out, "w"), indent=1)
