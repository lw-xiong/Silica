import sys, os, time; sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rec import Rec
r = Rec("tour"); r.reset()
r.command("backlink:open"); r.open("Home", mode="live"); r.idle(0.5)
r.frame(2.0, name="workspace")
r.command("markdown:toggle-preview"); r.frame(1.4)
for dy, d in ((380, 1.0), (380, 1.0), (380, 1.0), (380, 1.3)): r.scroll(dy); r.frame(d)
r.click_row("Rocks"); r.frame(0.9)
r.click_row("Silica"); r.frame(2.0)
for dy, d in ((420, 1.0), (420, 1.0), (420, 1.3)): r.scroll(dy); r.frame(d)
r.command("outline:open"); r.frame(1.3)
r.click(1190, 182); r.frame(1.8)          # outline row: pass 1 guess, corrected from the frame
r.done()
