from _common import *
SOP = "sop-tube-cap"
NOTE = "tube caps with separate cap points (consolidatepts off), triangles, rows 3 cols 6"
SCENE = {"nodes": [{"name": "g", "type": "tube", "params": {"type": 1, "cap": True, "consolidatepts": False, "surftype": 3, "rows": 3, "cols": 6}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "tube", "g", type=1, cap=1, consolidatepts=0, surftype=3, rows=3, cols=6)
finish(SOP, SCENE, build, NOTE)
