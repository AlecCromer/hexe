from _common import *
SOP = "sop-uvlayout"
NOTE = "box -> uvunwrap -> uvlayout padding 0.05 fit (examples/uv.json). Houdini uvlayout padding is in pixels (default 1 @512)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "uvu", "type": "uvunwrap", "params": {"tile": 3}}, {"name": "uvl", "type": "uvlayout", "params": {"padding": 0.05, "fit": True}}], "edges": [["box1", 0, "uvu", 0], ["uvu", 0, "uvl", 0]], "display": "uvl"}
def build(net):
    b = mk(net, "box", "box1"); u = mk(net, "uvunwrap", "uvu"); l = mk(net, "uvlayout::3.0", "uvl")
    u.setInput(0, b); l.setInput(0, u)
    return l
finish(SOP, SCENE, build, NOTE)
