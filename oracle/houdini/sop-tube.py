from _common import *
SOP = "sop-tube"
NOTE = "tube type poly (hexe default; Houdini default is prim), 2 rows 12 cols"
SCENE = {"nodes": [{"name": "g", "type": "tube", "params": {"type": 1}}], "edges": [], "display": "g"}
def build(net):
    return mk(net, "tube", "g", type=1)
finish(SOP, SCENE, build, NOTE)
