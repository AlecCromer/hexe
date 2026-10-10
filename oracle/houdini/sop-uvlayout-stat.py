from _common import *
SOP = "sop-uvlayout-stat"
NOTE = "box -> uvunwrap tile 3 -> uvlayout padding 0.002 (Houdini default 1px @512) fit. stat-uv oracle: island count, uv bbox per island, coverage +-5 % (layout/packing is not reproduced exactly; detail attrs coverage/numnonpacked are Houdini-only)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "uvu", "type": "uvunwrap", "params": {"tile": 3}}, {"name": "uvl", "type": "uvlayout", "params": {"padding": 0.002, "fit": True}}], "edges": [["box1", 0, "uvu", 0], ["uvu", 0, "uvl", 0]], "display": "uvl"}
def build(net):
    b = mk(net, "box", "box1"); u = mk(net, "uvunwrap", "uvu"); l = mk(net, "uvlayout::3.0", "uvl")
    u.setInput(0, b); l.setInput(0, u)
    return l
finish(SOP, SCENE, build, NOTE)
