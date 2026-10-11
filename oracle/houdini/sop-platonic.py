from _common import *
SOP = "sop-platonic"
NOTE = "platonic tetrahedron (type 0), Houdini defaults"
SCENE = {"nodes": [{"name": "g", "type": "platonic", "params": {}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "platonic", "g", type=0)
finish(SOP, SCENE, build, NOTE)
