from _common import *
SOP = "sop-copytopoints"
NOTE = "box copied onto grid(3x3) points; defaults (promote-1: copynum no longer forced, Houdini 20.5 writes none)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "grid1", "type": "grid", "params": {"rows": 3, "cols": 3, "type": 4}}, {"name": "copy1", "type": "copytopoints", "params": {}}], "edges": [["box1", 0, "copy1", 0], ["grid1", 0, "copy1", 1]], "display": "copy1"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "grid", "grid1", rows=3, cols=3, type=4)
    c = mk(net, "copytopoints::2.0", "copy1")
    c.setInput(0, b); c.setInput(1, g)
    return c
finish(SOP, SCENE, build, NOTE)
