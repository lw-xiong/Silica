import sys, os, time; sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rec import Rec
r = Rec("editing"); r.reset()
# The clip types into this note and autosave writes it, so start from the repository's copy.
import shutil
NOTE = "Field notes/2026-06-02 Etna lapilli sampling.md"
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
VAULT = os.environ.get("SILICA_REC_VAULT", os.path.join(os.environ.get("SILICA_REC_WORK", os.path.dirname(os.path.abspath(__file__))), "silica-demo"))
shutil.copyfile(os.path.join(REPO, "docs", "silica-demo", NOTE), os.path.join(VAULT, NOTE)); time.sleep(1.0)
r.command("outline:open")
r.open("2026-06-02 Etna lapilli sampling", mode="live"); r.call("focus", set="editor"); r.idle(0.3)
# the cursor goes after the first To do item, so the new line stays clear of the status bar
text = r.state(text=True)["editor"]["text"]; anchor = "fraction for density"
r.call("set_selection", start=len(text[:text.index(anchor) + len(anchor)].encode("utf-8"))); r.idle(0.3)
line = r.state()["editor"]["cursor"]["line"]; r.call("scroll_to", line=max(0, line - 16)); r.idle(0.3); r.frame(1.4)
r.type("\n- [ ] Compare ET-01 with the "); r.frame(0.9)
r.type("[", per_char=True, dur=0.5); r.type("["); r.frame(0.6, name="autocomplete")
r.type("Teph", per_char=True, dur=0.4)
r.frame(1.5)
r.keys("enter"); r.frame(1.0)
r.type(" size classes"); r.frame(1.4)
r.command("editor:open-search"); r.type("lapilli"); r.idle(0.5); r.frame(1.8)
r.keys("escape"); r.idle(0.2)
r.command("workspace:split-vertical"); r.frame(1.2)
r.open("Vesiculation#Measuring it", mode="reading"); r.frame(2.4)
r.done()
