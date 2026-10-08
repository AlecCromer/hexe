from _common import *
SOP = "sop-delete"
NOTE = "grid 3x3 -> delete prims '0 2' (hexe entity 1=prim; unused points kept; Houdini twin keeppoints=1 for parity of the hexe default)"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "dl", "type": "delete", "params": {"entity": 1, "group": "0 2"}}], "edges": [["g", 0, "dl", 0]], "display": "dl"}
def build(net):
    g = mk(net, "grid", "g", rows=3, cols=3); d = mk(net, "delete", "dl", group="0 2", entity="primitive", keeppoints=1)
    d.setInput(0, g)
    return d
finish(SOP, SCENE, build, NOTE)
