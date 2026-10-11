from _common import *
SOP = "sop-dissolve-all"
NOTE = "dissolve with an empty group: every polygon (and point) goes"
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"rows": 4, "cols": 4}}, {"name": "op1", "type": "dissolve", "params": {"group": ""}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    s = mk(net, "grid", "src", rows=4, cols=4)
    k = mk(net, "dissolve::2.0", "op1", group="")
    k.setInput(0, s)
    return k
finish(SOP, SCENE, build, NOTE)
