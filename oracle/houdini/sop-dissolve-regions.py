from _common import *
SOP = "sop-dissolve-regions"
NOTE = "dissolve two regions {0,1} and {4,7,8} on a 4x4-point grid, inline removal off"
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"rows": 4, "cols": 4}}, {"name": "op1", "type": "dissolve", "params": {"group": "p1-5 p9-10 p10-14", "reminlinepts": False}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    s = mk(net, "grid", "src", rows=4, cols=4)
    k = mk(net, "dissolve::2.0", "op1", group="p1-5 p9-10 p10-14", reminlinepts=0)
    k.setInput(0, s)
    return k
finish(SOP, SCENE, build, NOTE)
