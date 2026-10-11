from _common import *
SOP = "sop-polyframe-sphere"
NOTE = "polyframe two edges + tangentv on a coarse polygon sphere (shared points, uneven face areas, poles)"
SCENE = {"nodes": [{"name": "sph", "type": "sphere", "params": {"type": 2, "rows": 5, "cols": 6}}, {"name": "op1", "type": "polyframe", "params": {"tangentvon": True}}], "edges": [["sph", 0, "op1", 0]], "display": "op1"}
def build(net):
    s = mk(net, "sphere", "sph", type=2, rows=5, cols=6)
    k = mk(net, "polyframe", "op1", tangentvon=1)
    k.setInput(0, s)
    return k
finish(SOP, SCENE, build, NOTE)
