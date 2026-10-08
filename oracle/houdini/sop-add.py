from _common import *
SOP = "sop-add"
NOTE = "3 points (0,3,0)(1,3,0)(2,3,0) joined into one polygon (hexe prims=true)"
SCENE = {"nodes": [{"name": "add1", "type": "add", "params": {"points": "0,3,0 1,3,0 2,3,0", "prims": True}}], "edges": [], "display": "add1"}
def build(net):
    n = net.createNode("add", "add1")
    n.parm("points").set(3)
    for i, p in enumerate([(0, 3, 0), (1, 3, 0), (2, 3, 0)]):
        n.parm("usept%d" % i).set(1)
        n.parmTuple("pt%d" % i).set(p)
    n.parm("prims").set(1)
    n.parm("prim0").set("0 1 2")
    n.parm("closed0").set(0)
    return n
finish(SOP, SCENE, build, NOTE)
