from _common import *
SOP = "sop-grid"
NOTE = "grid size 6x4 rows 4 cols 3, poly, ZX plane (defaults otherwise)"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"size": [6, 4, 0], "rows": 4, "cols": 3}}], "edges": [], "display": "grid1"}
def build(net):
    return mk(net, "grid", "grid1", size=(6, 4), rows=4, cols=3)
finish(SOP, SCENE, build, NOTE)
