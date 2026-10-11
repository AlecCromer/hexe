from _common import *
SOP = "sop-dissolve"
NOTE = "dissolve edges p1-5 p4-5 on a 4x4-point grid (3 quads merge; inline points 1 and 4 vanish; defaults)"
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"rows": 4, "cols": 4}}, {"name": "op1", "type": "dissolve", "params": {"group": "p1-5 p4-5"}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    s = mk(net, "grid", "src", rows=4, cols=4)
    k = mk(net, "dissolve::2.0", "op1", group="p1-5 p4-5")
    k.setInput(0, s)
    return k
finish(SOP, SCENE, build, NOTE)
