from _common import *
SOP = "sop-platonic-dodeca"
NOTE = "platonic dodecahedron"
SCENE = {"nodes": [{"name": "g", "type": "platonic", "params": {"type": 4}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "platonic", "g", type=4)
finish(SOP, SCENE, build, NOTE)
