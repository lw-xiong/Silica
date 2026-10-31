#!/usr/bin/env bash
# Run one CDP probe script against a fresh scratch vault, in phases:
#   fixtures  (Obsidian not running: the script writes its notes)
#   run       (Obsidian running; may end by quitting it)
#   after-restart (only if "run" quit Obsidian: relaunch and finish)
# Usage: run-block.sh <script.mjs> <results.json> [vault-dir]
# Never points Obsidian at a real vault: it creates its own folder under $TMPDIR
# unless a vault-dir is given, swaps ~/.config/obsidian/obsidian.json in for the
# duration, and restores it byte-identical on exit.
set -euo pipefail
SCRIPT=$(readlink -f "$1"); RESULTS=$(readlink -f -m "$2")
VAULT=${3:-$(mktemp -d "${TMPDIR:-/tmp}/silica-probe-vault.XXXXXX")}
CFG=~/.config/obsidian/obsidian.json
BACKUP=$(mktemp); command cp -f "$CFG" "$BACKUP"
LOG=$(mktemp)
# Anchored at ^/nix/store so it can never match this shell. The group admits the
# --remote-debugging-port flag between the electron binary and the asar: without
# it nothing launched with CDP matches, the kill is a no-op, and the next launch
# is swallowed by the single-instance lock of the survivor.
PGREP='^/nix/store/[^ ]*electron( --[^ ]*)* /nix/store/[^ ]*obsidian'

# Only an Obsidian this run launched is killed on the way out: refusing to
# start because one was already running must not close that one.
LAUNCHED=0
restore() {
  if [ "$LAUNCHED" = 1 ] && pgrep -f "$PGREP" >/dev/null; then pkill -f "$PGREP" || true; sleep 2; fi
  command cp -f "$BACKUP" "$CFG"; rm -f "$BACKUP"
  echo "[run-block] obsidian.json restored; vault left at $VAULT; obsidian log $LOG" >&2
}
trap restore EXIT

# A previous instance can linger for seconds after its window closes, and a new
# launch during that window is handed to the dying instance (which then opens
# ITS vault, not ours). Wait for both the process and the CDP endpoint to go.
settled() { ! pgrep -f "$PGREP" >/dev/null && ! curl -s --max-time 1 http://127.0.0.1:9222/json >/dev/null 2>&1; }
for i in $(seq 1 40); do settled && break; sleep 0.5; done
if ! settled; then echo "[run-block] Obsidian is already running; stop it first" >&2; exit 2; fi
sleep 2
mkdir -p "$VAULT/.obsidian"; [ -f "$VAULT/.obsidian/app.json" ] || printf '{}' > "$VAULT/.obsidian/app.json"
export VAULT RESULTS

# Register the scratch vault under a fresh 16-hex id. Reusing an id across
# folders inherits that id's IndexedDB cache; a malformed id is ignored.
ID=$(python3 -c 'import secrets;print(secrets.token_hex(8))')
python3 - "$BACKUP" "$CFG" "$ID" "$VAULT" <<'PY'
import json,sys
src,dst,id_,path=sys.argv[1:]
d=json.load(open(src))
for v in d['vaults'].values(): v.pop('open',None)
d['vaults'][id_]={'path':path,'ts':1792373000000,'open':True}
json.dump(d,open(dst,'w'))
PY

# Launch, wait for a booted vault window, and refuse to proceed unless it is OUR vault.
launch_once() {
  LAUNCHED=1
  nohup obsidian --remote-debugging-port=9222 >>"$LOG" 2>&1 &
  node -e "
    import('$(dirname "$SCRIPT")/cdp.mjs').then(async (m) => {
      await m.waitForObsidian(90000);
      const c = await m.connect();
      const p = await c.vaultPath(); c.close();
      const want = process.env.VAULT.replace(/\/+$/, '');
      if (p.replace(/\/+$/, '') !== want) { console.error('[run-block] wrong vault open: ' + p + ' (wanted ' + want + ')'); process.exit(3); }
      process.exit(0);
    }).catch((e) => { console.error(e.message); process.exit(1); })"
}
launch() {
  if launch_once; then return 0; fi
  echo "[run-block] relaunching once after a bad start" >&2
  pkill -f "$PGREP" || true
  for i in $(seq 1 40); do settled && break; sleep 0.5; done; sleep 2
  launch_once
}
gone() { ! curl -s --max-time 1 http://127.0.0.1:9222/json >/dev/null 2>&1; }

echo "[run-block] phase fixtures" >&2;   node "$SCRIPT" fixtures
echo "[run-block] launch" >&2;           launch
echo "[run-block] phase run" >&2;        node "$SCRIPT" run || echo "[run-block] run phase exited non-zero" >&2
sleep 2
if gone; then
  echo "[run-block] Obsidian quit during run; relaunching for after-restart" >&2
  launch
  echo "[run-block] phase after-restart" >&2; node "$SCRIPT" after-restart || echo "[run-block] after-restart exited non-zero" >&2
fi
echo "[run-block] done: $RESULTS" >&2
