from _common import *
SOP = "sop-clip"
NOTE = "box clip plane origin 0 dir +Y dist 0, keep above (hexe keep 0 = Houdini clipop above), no cap/fill (dofill 0; hexe capclip DEFERRED)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "clip1", "type": "clip", "params": {"origin": [0, 0, 0], "dir": [0, 1, 0], "keep": 0}}], "edges": [["box1", 0, "clip1", 0]], "display": "clip1"}
def build(net):
    b = mk(net, "box", "box1")
    c = mk(net, "clip::2.0", "clip1", origin=(0, 0, 0), dir=(0, 1, 0), dist=0, clipop=0, dofill=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
