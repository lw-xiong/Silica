"""Records a clip: drives Silica over the control socket, grabs the headless output with
grim after each step, and writes $SILICA_REC_WORK/frames/<clip>/NNN.png plus durations.json."""
import json, os, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ctl import Ctl

ROOT = os.environ.get("SILICA_REC_WORK", os.path.dirname(os.path.abspath(__file__)))

class Rec:
    def __init__(self, clip):
        self.c = Ctl()
        self.dir = os.path.join(ROOT, "frames", clip)
        import shutil; shutil.rmtree(self.dir, ignore_errors=True); os.makedirs(self.dir)
        self.frames = []
        # Pointer coordinates are window-local; under sway gpui's origin sits DX, DY from the
        # frame's (calibrated by clicking explorer rows), so every pointer op adds the offset.
        self.dx, self.dy = -24, -24

    # --- driving -----------------------------------------------------
    def call(self, method, **params):
        return self.c.call(method, **params)
    def idle(self, settle=0.25):
        self.c.call("wait_idle", frames=2, timeout_ms=3000)
        time.sleep(settle)
    def open(self, name, mode=None, settle=0.4):
        p = {"name": name}
        if mode: p["mode"] = mode
        r = self.c.call("open", **p); self.idle(settle); return r
    def command(self, cid, settle=0.3):
        r = self.c.call("command", id=cid); self.idle(settle); return r
    def keys(self, *keys, settle=0.25):
        r = self.c.call("keys", keys=list(keys)); self.idle(settle); return r
    def click(self, x, y, settle=0.3, **kw):
        x, y = x + self.dx, y + self.dy
        self.c.call("mouse", action="move", x=x, y=y)
        self.c.call("wait_idle", frames=1, timeout_ms=1000)
        r = self.c.call("click", x=x, y=y, **kw); self.idle(settle); return r
    def hover(self, x, y, settle=0.5):
        self.c.call("mouse", action="move", x=x + self.dx, y=y + self.dy); self.idle(settle)
    def scroll(self, dy, dx=0, x=720, y=500, settle=0.3):
        """dy > 0 scrolls the content up (reads further down); on a canvas, dx > 0 pans right."""
        self.c.call("scroll", x=x + self.dx, y=y + self.dy, dx=-dx, dy=-dy, unit="pixels"); self.idle(settle)
    def type(self, text, per_char=None, dur=0.12):
        """Types text; with per_char, grabs a frame after every character."""
        for ch in text:
            self.c.call("type", text=ch)
            if per_char:
                self.idle(0.1); self.frame(dur)
        if not per_char: self.idle(0.2)
    def state(self, **kw):
        return self.c.call("state", **kw)

    # --- capture -----------------------------------------------------
    def frame(self, dur=1.0, name=None):
        n = len(self.frames)
        path = os.path.join(self.dir, f"{n:03d}.png")
        # grim captures the headless sway output: the wallpaper, the window and its own shadow.
        env = dict(os.environ)
        try:
            env["WAYLAND_DISPLAY"] = open(os.path.join(ROOT, "wl-display.txt")).read().strip()
        except FileNotFoundError:
            pass  # WAYLAND_DISPLAY already set in the environment (Hyprland)
        out = os.environ.get("SILICA_REC_OUTPUT", "silica-rec")
        subprocess.run(["grim", "-o", out, path], check=True, env=env)
        size = subprocess.run(["magick", "identify", "-format", "%wx%h", path], capture_output=True, text=True).stdout
        assert size == "3072x2048", f"unexpected capture {size}: not the headless output"
        self.frames.append({"file": os.path.basename(path), "dur": dur, "name": name})
        return path
    def done(self):
        json.dump(self.frames, open(os.path.join(self.dir, "durations.json"), "w"), indent=1)
        print(f"{len(self.frames)} frames in {self.dir}")

    # --- helpers ------------------------------------------------------
    def row_y(self, name):
        """Window-local y of an explorer row by its name (rows are 25 px from y 116)."""
        rows = self.state(rows=80)["docks"]["left"]["explorer"]["rows"]
        for i, row in enumerate(rows):
            if row["name"] == name: return 116 + 25 * i
        raise KeyError(name)
    def click_row(self, name, settle=0.4):
        self.click(170, self.row_y(name), settle=settle)
    def collapse_all(self):
        """Collapses every expanded top-level folder in the explorer."""
        while True:
            rows = self.state(rows=80)["docks"]["left"]["explorer"]["rows"]
            open_folder = None
            for i, row in enumerate(rows):
                if row["folder"] and row["depth"] == 0 and i + 1 < len(rows) and rows[i + 1]["depth"] > 0:
                    open_folder = row["name"]; break
            if not open_folder: return
            self.click_row(open_folder, settle=0.2)
    def goto_end(self):
        """Puts the editor cursor after the last byte of the note."""
        text = self.state(text=True)["editor"].get("text") or self.state(text=True).get("text") or ""
        n = len(text.encode("utf-8"))
        self.call("set_selection", start=n); self.idle(0.2)
    def reset(self):
        """One tab group, one tab, Files and Backlinks showing, every folder collapsed."""
        for _ in range(8):
            if len(self.state(rows=1)["groups"]["items"]) <= 1: break
            self.command("workspace:close-tab-group")
        self.open("Home", mode="live"); self.command("workspace:close-others")
        self.command("file-explorer:open"); self.command("backlink:open"); self.collapse_all()
        self.idle(0.4)
