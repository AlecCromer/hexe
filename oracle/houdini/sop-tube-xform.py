from _common import *
SOP = "sop-tube-xform"
NOTE = "tube orient z, rad 2,.5, height 2, radscale 1.5, t, r, rows 3 cols 5"
SCENE = {"nodes": [{"name": "g", "type": "tube", "params": {"type": 1, "orient": 2, "rad": [2, 0.5, 0], "height": 2, "radscale": 1.5, "t": [1, 2, 3], "r": [10, 20, 30], "rows": 3, "cols": 5}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "tube", "g", type=1, orient=2, rad=(2, 0.5), height=2, radscale=1.5, t=(1, 2, 3), r=(10, 20, 30), rows=3, cols=5)
finish(SOP, SCENE, build, NOTE)
