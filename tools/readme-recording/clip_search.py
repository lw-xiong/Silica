import sys, os, time; sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rec import Rec
r = Rec("search"); r.reset()
r.command("backlink:open"); r.open("Home", mode="reading"); r.frame(1.2)
r.command("switcher:open"); r.frame(0.8)
r.type("strom", per_char=True, dur=0.35)
r.frame(1.2)
r.keys("enter"); r.frame(2.0)
r.command("global-search:open"); r.idle(0.3)
r.keys("ctrl-a", "backspace"); r.idle(0.3); r.frame(0.7)   # the pane keeps its last query
# Four frames of the result list narrowing, then the rest of the query at once:
# the right-hand pane does not move while it is typed, so holding every letter
# spent a third of the clip on frames that look the same.
r.type("vesi", per_char=True, dur=0.3)
r.type("cularity"); r.idle(0.8); r.frame(2.2, name="search")
st = r.state(rows=10); print("search:", st["docks"]["left"]["search"]["count"], [x.get("path") for x in st["docks"]["left"]["search"]["results"]][:6])
r.click(180, 190); r.frame(1.8)       # pass 1 guess
r.command("tag-pane:open"); r.idle(0.4); r.frame(1.8)
r.open("Silica", mode="reading"); r.command("backlink:open"); r.idle(0.4); r.frame(2.2)
r.done()
