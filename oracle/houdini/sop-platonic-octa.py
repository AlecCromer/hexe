from _common import *
SOP = "sop-platonic-octa"
NOTE = "platonic octahedron"
SCENE = {"nodes": [{"name": "g", "type": "platonic", "params": {"type": 2}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "platonic", "g", type=2)
finish(SOP, SCENE, build, NOTE)
