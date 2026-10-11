from _common import *
SOP = "sop-dissolve-boundary"
NOTE = "dissolve boundary edges p0-1 p0-4 (deletes corner quad + its point) and p5-6 (merge) on a 4x4-point grid"
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"rows": 4, "cols": 4}}, {"name": "op1", "type": "dissolve", "params": {"group": "p0-1 p0-4 p5-6"}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    s = mk(net, "grid", "src", rows=4, cols=4)
    k = mk(net, "dissolve::2.0", "op1", group="p0-1 p0-4 p5-6")
    k.setInput(0, s)
    return k
finish(SOP, SCENE, build, NOTE)
