from _common import *
SOP = "sop-dissolve-box"
NOTE = "dissolve edges p0-1 p1-2 on a box (three faces merge into an 8-vertex polygon repeating a point)"
SCENE = {"nodes": [{"name": "src", "type": "box"}, {"name": "op1", "type": "dissolve", "params": {"group": "p0-1 p1-2"}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    s = mk(net, "box", "src")
    k = mk(net, "dissolve::2.0", "op1", group="p0-1 p1-2")
    k.setInput(0, s)
    return k
finish(SOP, SCENE, build, NOTE)
