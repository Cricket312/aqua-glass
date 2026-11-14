#!/usr/bin/env bash
# Toggle visual features on/off repeatedly through the real UI (quick-settings menu)
# and record private memory before/after. Needs xdotool, a running X display and an
# audio device. usage: bench/toggle_stress.sh <music-dir> <cycles>
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MUSIC=$1; CYCLES=${2:-30}
HOME_DIR=/tmp/claude-0/toggle-home; rm -rf "$HOME_DIR"; mkdir -p "$HOME_DIR"
"$ROOT/target/release/examples/make_config" Balanced -ram_warnings +bg_image "bg_image_path=$MUSIC/background.png" "folder=$MUSIC" repeat=all > "$HOME_DIR/settings.toml"
AQUA_GLASS_HOME=$HOME_DIR "$ROOT/target/release/aqua-glass" --page now-playing "$MUSIC"/*.wav >/dev/null 2>&1 &
PID=$!
sleep 8
mem() { awk '/RssAnon/{printf "%.1f", $2/1024}' /proc/$PID/status; }
echo "baseline_private_mb=$(mem)"
# quick-settings button, then the toggles inside the menu (coordinates for a 1240x780 window)
QX=1211; QY=739
ITEMS_Y="${TOGGLE_YS:-}"
for c in $(seq 1 "$CYCLES"); do
  xdotool mousemove $QX $QY click 1; sleep 0.4
  for y in $ITEMS_Y; do xdotool mousemove ${TOGGLE_X:-0} "$y" click 1; sleep 0.25; done
  xdotool key Escape; sleep 0.6
  [ $((c % 5)) -eq 0 ] && echo "cycle=$c private_mb=$(mem)"
done
sleep 8
echo "final_private_mb=$(mem)"
kill $PID
