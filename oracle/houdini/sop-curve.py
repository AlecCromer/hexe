from _common import *
SOP = "sop-curve"
NOTE = "closed poly through (0,1,0) (1,1,0) (1,2,0). Houdini 20.5 curve::2.0 has no coords parm -> twin = curve::1.0 if present else add SOP closed polygon"
SCENE = {"nodes": [{"name": "curve1", "type": "curve", "params": {"coords": "0,1,0 1,1,0 1,2,0", "close": True}}], "edges": [], "display": "curve1"}
def build(net):
    try:
        n = net.createNode("curve::1.0", "curve1")
        n.parm("coords").set("0,1,0 1,1,0 1,2,0")
        n.parm("close").set(1)
        return n
    except Exception as e:
        print("curve::1.0 unavailable:", e)
    n = net.createNode("add", "curve1")
    n.parm("points").set(3)
    for i, p in enumerate([(0, 1, 0), (1, 1, 0), (1, 2, 0)]):
        n.parm("usept%d" % i).set(1)
        n.parmTuple("pt%d" % i).set(p)
    n.parm("prims").set(1)
    n.parm("prim0").set("0 1 2")
    n.parm("closed0").set(1)
    return n
finish(SOP, SCENE, build, NOTE)
