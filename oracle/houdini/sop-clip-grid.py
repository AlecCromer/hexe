from _common import *
SOP = "sop-clip-grid"
NOTE = "grid 6x4 rows4 cols3, plane dir +X dist 0.5 keep above"
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"size": [6, 4, 0], "rows": 4, "cols": 3}}, {"name": "op1", "type": "clip", "params": {"origin": [0, 0, 0], "dir": [1, 0, 0], "dist": 0.5, "keep": 0}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "grid", "src", size=(6, 4), rows=4, cols=3)
    c = mk(net, "clip::2.0", "op1", origin=(0, 0, 0), dir=(1, 0, 0), dist=0.5, clipop=0, dofill=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
