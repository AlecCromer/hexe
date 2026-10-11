from _common import *
SOP = "sop-platonic-xform"
NOTE = "platonic dodecahedron orient x radius 2 t r"
SCENE = {"nodes": [{"name": "g", "type": "platonic", "params": {"type": 4, "orient": 0, "radius": 2, "t": [1, 2, 3], "r": [10, 20, 30]}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "platonic", "g", type=4, orient=0, radius=2, t=(1, 2, 3), r=(10, 20, 30))
finish(SOP, SCENE, build, NOTE)
