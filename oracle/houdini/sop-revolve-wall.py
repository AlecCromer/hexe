from _common import *
SOP = "sop-revolve-wall"
NOTE = "open 2-point wall profile (1,0,0)-(1,1,0) (add sop, open polygon) -> revolve full sweep divs 12 quads"
SCENE = {"nodes": [{"name": "profile", "type": "add", "params": {"points": "1,0,0 1,1,0", "prims": True}}, {"name": "revolve1", "type": "revolve", "params": {"divs": 12}}], "edges": [["profile", 0, "revolve1", 0]], "display": "revolve1"}
def build(net):
    n = net.createNode("add", "profile")
    n.parm("points").set(2)
    for i, p in enumerate([(1, 0, 0), (1, 1, 0)]):
        n.parm("usept%d" % i).set(1)
        n.parmTuple("pt%d" % i).set(p)
    n.parm("prims").set(1)
    n.parm("prim0").set("0 1")
    n.parm("closed0").set(0)
    r = mk(net, "revolve::2.0", "revolve1", divs=12, surftype=5, primtype=1)
    r.setInput(0, n)
    return r
finish(SOP, SCENE, build, NOTE)
