from _common import *
SOP = "sop-platonic-icosa"
NOTE = "platonic icosahedron"
SCENE = {"nodes": [{"name": "g", "type": "platonic", "params": {"type": 3}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "platonic", "g", type=3)
finish(SOP, SCENE, build, NOTE)
