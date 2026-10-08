from _common import *
SOP = "sop-transform"
NOTE = "box -> xform t(1,2,3) r(0,30,0) s(1,2,1)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "xf", "type": "xform", "params": {"t": [1, 2, 3], "r": [0, 30, 0], "s": [1, 2, 1]}}], "edges": [["box1", 0, "xf", 0]], "display": "xf"}
def build(net):
    b = mk(net, "box", "box1")
    x = mk(net, "xform", "xf", t=(1, 2, 3), r=(0, 30, 0), s=(1, 2, 1))
    x.setInput(0, b)
    return x
finish(SOP, SCENE, build, NOTE)
