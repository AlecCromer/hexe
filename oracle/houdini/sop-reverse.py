from _common import *
SOP = "sop-reverse"
NOTE = "box -> reverse (flip winding)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "rv", "type": "reverse"}], "edges": [["box1", 0, "rv", 0]], "display": "rv"}
def build(net):
    b = mk(net, "box", "box1"); r = mk(net, "reverse", "rv", vtxsort="reverse")
    r.setInput(0, b)
    return r
finish(SOP, SCENE, build, NOTE)
