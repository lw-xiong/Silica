#!/usr/bin/env bash
# Records the README GIFs: tools/readme-recording/record.sh [wallpaper.png] [clip ...]
# Needs a release build (target/release/silica), sway, grim, ffmpeg and ImageMagick on PATH
# (on NixOS: nix-shell -p sway grim ffmpeg imagemagick). Runs Silica on a scratch copy of
# docs/silica-demo inside a private headless sway session, so nothing on the desktop is
# touched or captured, and writes the GIFs and stills to docs/img/app.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); REPO=$(cd "$HERE/../.." && pwd)
WALLPAPER=${1:-$HERE/wallpaper.png}; shift || true
CLIPS=${*:-tour media search editing}
export SILICA_REC_WORK=${SILICA_REC_WORK:-$(mktemp -d /tmp/silica-rec.XXXXXX)}
export SILICA_SOCK=/run/user/$(id -u)/silica-rec.sock
SWAYSOCK=$SILICA_REC_WORK/sway.sock
W=$SILICA_REC_WORK
echo "work dir $W"
rm -rf "$W/silica-demo"; mkdir -p "$W/silica-demo"; cp -r "$REPO/docs/silica-demo/." "$W/silica-demo/"
sed "s|WALLPAPER|$WALLPAPER|" "$HERE/sway.cfg" > "$W/sway.cfg"
cleanup() { python3 "$HERE/ctl.py" quit >/dev/null 2>&1 || true; swaymsg -s "$SWAYSOCK" exit >/dev/null 2>&1 || true; }
trap cleanup EXIT
WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_LIBINPUT_NO_DEVICES=1 SWAYSOCK=$SWAYSOCK \
  sway -c "$W/sway.cfg" --unsupported-gpu > "$W/sway.log" 2>&1 &
for _ in $(seq 50); do [ -S "$SWAYSOCK" ] && break; sleep 0.2; done
swaymsg -s "$SWAYSOCK" exec "sh -c 'echo \$WAYLAND_DISPLAY > $W/wl-display.txt'"
for _ in $(seq 50); do [ -s "$W/wl-display.txt" ] && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(cat "$W/wl-display.txt")
rm -f "$SILICA_SOCK"
"$REPO/target/release/silica" "$W/silica-demo" --control "$SILICA_SOCK" --note Home > "$W/app.log" 2>&1 &
for _ in $(seq 100); do [ -S "$SILICA_SOCK" ] && break; sleep 0.2; done
if [ ! -S "$SILICA_SOCK" ]; then
  echo "silica never opened $SILICA_SOCK; last lines of $W/app.log:" >&2
  tail -5 "$W/app.log" >&2
  exit 1
fi
python3 "$HERE/ctl.py" wait_loaded '{"timeout_ms": 60000}' >/dev/null
for c in $CLIPS; do python3 "$HERE/clip_$c.py"; python3 "$HERE/compose.py" "$c" "$REPO/docs/img/app/$c.gif" 1000; done
