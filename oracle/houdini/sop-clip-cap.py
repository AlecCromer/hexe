from _common import *
SOP = "sop-clip-cap"
NOTE = "box clip plane y=0.1 keep above, cap clipped geometry (dofill 1 usefillpolygons 0 / hexe capclip)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "clip1", "type": "clip", "params": {"origin": [0, 0, 0], "dir": [0, 1, 0], "dist": 0.1, "keep": 0, "capclip": True}}], "edges": [["box1", 0, "clip1", 0]], "display": "clip1"}
def build(net):
    b = mk(net, "box", "box1")
    c = mk(net, "clip::2.0", "clip1", origin=(0, 0, 0), dir=(0, 1, 0), dist=0.1, clipop=0, dofill=1, usefillpolygons=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
