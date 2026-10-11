from _common import *
SOP = "sop-torus-alt"
NOTE = "torus alternating triangles rows 4 cols 5"
SCENE = {"nodes": [{"name": "g", "type": "torus", "params": {"surftype": 5, "rows": 4, "cols": 5}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "torus", "g", surftype=5, rows=4, cols=5)
finish(SOP, SCENE, build, NOTE)
