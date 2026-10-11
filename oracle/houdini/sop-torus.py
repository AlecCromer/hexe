from _common import *
SOP = "sop-torus"
NOTE = "torus type poly, Houdini defaults (rows 12 cols 24 rad 1,.5 quads)"
SCENE = {"nodes": [{"name": "g", "type": "torus", "params": {}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "torus", "g", type=0)
finish(SOP, SCENE, build, NOTE)
