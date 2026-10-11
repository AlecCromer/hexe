from _common import *
SOP = "sop-carve-grid"
NOTE = "carve defaults on a 3x3-point polygon grid: every quad is a closed loop re-opened at u=0.25 (original points kept, new end points appended)"
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"size": [6, 4, 0], "rows": 3, "cols": 3}}, {"name": "op1", "type": "carve", "params": {}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    g = mk(net, "grid", "src", size=(6, 4), rows=3, cols=3)
    k = mk(net, "carve", "op1")
    k.setInput(0, g)
    return k
finish(SOP, SCENE, build, NOTE)
