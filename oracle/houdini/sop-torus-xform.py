from _common import *
SOP = "sop-torus-xform"
NOTE = "torus orient x, rad 2,.5, scale 1.5, t, r, rows 5 cols 7"
SCENE = {"nodes": [{"name": "g", "type": "torus", "params": {"orient": 0, "rad": [2, 0.5, 0], "scale": 1.5, "t": [1, 2, 3], "r": [10, 20, 30], "rows": 5, "cols": 7}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "torus", "g", orient=0, rad=(2, 0.5), scale=1.5, t=(1, 2, 3), r=(10, 20, 30), rows=5, cols=7)
finish(SOP, SCENE, build, NOTE)
