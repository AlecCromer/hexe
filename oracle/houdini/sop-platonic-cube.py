from _common import *
SOP = "sop-platonic-cube"
NOTE = "platonic cube"
SCENE = {"nodes": [{"name": "g", "type": "platonic", "params": {"type": 1}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "platonic", "g", type=1)
finish(SOP, SCENE, build, NOTE)
