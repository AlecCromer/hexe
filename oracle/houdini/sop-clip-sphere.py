from _common import *
SOP = "sop-clip-sphere"
NOTE = "polymesh sphere, tilted plane dir (0.3,1,0.2) dist 0.1, keep both"
SCENE = {"nodes": [{"name": "src", "type": "sphere", "params": {"type": 2}}, {"name": "op1", "type": "clip", "params": {"origin": [0, 0, 0], "dir": [0.3, 1, 0.2], "dist": 0.1, "keep": 2}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "sphere", "src", type=2)
    c = mk(net, "clip::2.0", "op1", origin=(0, 0, 0), dir=(0.3, 1, 0.2), dist=0.1, clipop=2, dofill=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
