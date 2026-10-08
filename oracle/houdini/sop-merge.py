from _common import *
SOP = "sop-merge"
NOTE = "box + grid(3x3 poly) merged (box first)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "grid1", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "m", "type": "merge"}], "edges": [["box1", 0, "m", 0], ["grid1", 0, "m", 1]], "display": "m"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "grid", "grid1", rows=3, cols=3)
    m = mk(net, "merge", "m")
    m.setInput(0, b); m.setInput(1, g)
    return m
finish(SOP, SCENE, build, NOTE)
