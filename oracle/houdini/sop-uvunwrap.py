from _common import *
SOP = "sop-uvunwrap"
NOTE = "box -> uvunwrap (Houdini defaults; hexe tile 1). Algorithms differ by design (hexe = simple best-fit projection)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "uvu", "type": "uvunwrap", "params": {"tile": 1}}], "edges": [["box1", 0, "uvu", 0]], "display": "uvu"}
def build(net):
    b = mk(net, "box", "box1"); u = mk(net, "uvunwrap", "uvu")
    u.setInput(0, b)
    return u
finish(SOP, SCENE, build, NOTE)
