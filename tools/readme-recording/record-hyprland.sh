#!/usr/bin/env bash
# Records the README GIFs on Hyprland (no sway/nix-shell): record-hyprland.sh [clip ...]
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); REPO=$(cd "$HERE/../.." && pwd)
WALLPAPER=${1:-$HERE/wallpaper.png}; shift || true
CLIPS=${*:-tour media search editing}
WORK=/tmp/silica-rec
SOCK=$WORK/sock
PIDFILE=$WORK/silica.pid
VAULT=$WORK/silica-demo
export WAYLAND_DISPLAY=wayland-1
export SILICA_REC_OUTPUT=silica-rec
export SILICA_REC_WORK=$WORK
echo "work dir $WORK"
# stop previous demo instance recorded in the pidfile
if [ -f "$PIDFILE" ]; then
  kill "$(cat "$PIDFILE")" 2>/dev/null || true
  sleep 1
fi
rm -rf "$WORK"; mkdir -p "$WORK"
cp -r "$REPO/docs/silica-demo/." "$VAULT/"
rm -f "$VAULT/.obsidian/workspace.json" "$SOCK"
hyprctl keyword monitor "eDP-2,preferred,auto,1" >/dev/null 2>&1 || true
hyprctl keyword monitor "silica-rec,3072x2048@60,3072x0,2,$WALLPAPER" >/dev/null 2>&1 || true
sleep 1
"$REPO/target/release/silica" "$VAULT" --control "$SOCK" --note Home > "$WORK/app.log" 2>&1 &
echo $! > "$PIDFILE"
# wait until the control socket answers ping
for _ in $(seq 80); do
  SILICA_SOCK="$SOCK" python3 "$HERE/ctl.py" ping '{}' >/dev/null 2>&1 && break
  sleep 0.25
done
if ! SILICA_SOCK="$SOCK" python3 "$HERE/ctl.py" ping '{}' >/dev/null 2>&1; then
  echo "silica control socket not answering; log:" >&2; tail -20 "$WORK/app.log" >&2; exit 1
fi
sleep 1
hyprctl dispatch 'hl.dsp.window.move({monitor="silica-rec"})' >/dev/null 2>&1 || true
hyprctl dispatch 'hl.dsp.window.float({action=on})' >/dev/null 2>&1 || true
hyprctl dispatch 'hl.dsp.focus' >/dev/null 2>&1 || true
sleep 1
# wait_loaded can race a frame rebuild; retry until the socket answers
for _ in $(seq 20); do
  python3 "$HERE/ctl.py" wait_loaded '{"timeout_ms": 5000}' >/dev/null 2>&1 && break
  sleep 0.25
done
for c in $CLIPS; do
  echo "== clip $c =="
  SILICA_SOCK="$SOCK" python3 "$HERE/clip_$c.py"
  SILICA_REC_WORK="$WORK" python3 "$HERE/compose.py" "$c" "$REPO/docs/img/app/$c.gif" 1000
done
echo "done"
