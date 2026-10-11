from _common import *
SOP = "sop-polyframe"
NOTE = "polyframe defaults (two edges, N + tangentu on points) on a box"
SCENE = {"nodes": [{"name": "b", "type": "box"}, {"name": "op1", "type": "polyframe", "params": {}}], "edges": [["b", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "box", "b")
    k = mk(net, "polyframe", "op1")
    k.setInput(0, b)
    return k
finish(SOP, SCENE, build, NOTE)
