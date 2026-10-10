from _common import *
SOP = "sop-clip-tilt-below"
NOTE = "box clip tilted plane dir (1,1,0.5) dist 0.1 keep below"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "clip1", "type": "clip", "params": {"origin": [0, 0, 0], "dir": [1, 1, 0.5], "dist": 0.1, "keep": 1}}], "edges": [["box1", 0, "clip1", 0]], "display": "clip1"}
def build(net):
    b = mk(net, "box", "box1")
    c = mk(net, "clip::2.0", "clip1", origin=(0, 0, 0), dir=(1, 1, 0.5), dist=0.1, clipop=1, dofill=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
