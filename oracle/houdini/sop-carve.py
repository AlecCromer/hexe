from _common import *
SOP = "sop-carve"
NOTE = "carve defaults (firstu 1 at u=0.25, secondu off) on an open 5-point polygon"
PTS = [(0, 0, 0), (1, 2, 0), (3, 2, 0), (4, 0, 0), (5, 1, 1)]
SCENE = {"nodes": [{"name": "src", "type": "curve", "params": {"coords": "0,0,0 1,2,0 3,2,0 4,0,0 5,1,1", "close": False}}, {"name": "op1", "type": "carve", "params": {}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    c = poly_curve(net, "src", PTS)
    k = mk(net, "carve", "op1")
    k.setInput(0, c)
    return k
finish(SOP, SCENE, build, NOTE)
