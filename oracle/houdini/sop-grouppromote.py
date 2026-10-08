from _common import *
SOP = "sop-grouppromote"
NOTE = "box -> group 'top' prims -> grouppromote prim->point any, newname toppts, keep source"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "top", "type": "group", "params": {"groupname": "top", "grouptype": 1, "normal": True, "dir": [0, 1, 0], "angle": 10}}, {"name": "pr", "type": "grouppromote", "params": {"group": "top", "fromtype": 1, "totype": 0, "method": 0, "newname": "toppts"}}], "edges": [["box1", 0, "top", 0], ["top", 0, "pr", 0]], "display": "pr"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "group", "top", crname="top", entity="primitive", groupnormal=1, dir=(0, 1, 0), angle=10)
    p = mk(net, "grouppromote", "pr")
    p.parm("promotions").set(1)
    for k, v in dict(enable1=1, fromtype1="prims", totype1="points", group1="top", newname1="toppts", preserve1=1).items():
        setp(p, k, v)
    g.setInput(0, b); p.setInput(0, g)
    return p
finish(SOP, SCENE, build, NOTE)
