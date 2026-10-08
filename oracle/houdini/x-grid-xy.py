from _common import *
SOP = "x-grid-xy"
NOTE = "extra case: grid XY plane (orient 0), size 6x4 rows 4 cols 3"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"size": [6, 4, 0], "rows": 4, "cols": 3, "orient": 0}}], "edges": [], "display": "grid1"}
def build(net):
    return mk(net, "grid", "grid1", size=(6, 4), rows=4, cols=3, orient=0)
finish(SOP, SCENE, build, NOTE)
