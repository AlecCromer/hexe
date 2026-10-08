from _common import *
SOP = "x-grid-yz"
NOTE = "extra case: grid YZ plane (orient 1), size 6x4 rows 4 cols 3"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"size": [6, 4, 0], "rows": 4, "cols": 3, "orient": 1}}], "edges": [], "display": "grid1"}
def build(net):
    return mk(net, "grid", "grid1", size=(6, 4), rows=4, cols=3, orient=1)
finish(SOP, SCENE, build, NOTE)
