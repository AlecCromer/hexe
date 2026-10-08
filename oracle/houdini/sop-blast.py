from _common import *
SOP = "sop-blast"
NOTE = "box -> group 'top' (prims, normal) -> blast group top (delete selected, prim class)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "top", "type": "group", "params": {"groupname": "top", "grouptype": 1, "normal": True, "dir": [0, 1, 0], "angle": 10}}, {"name": "bl", "type": "blast", "params": {"group": "top", "grouptype": 1}}], "edges": [["box1", 0, "top", 0], ["top", 0, "bl", 0]], "display": "bl"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "group", "top", crname="top", entity="primitive", groupnormal=1, dir=(0, 1, 0), angle=10)
    d = mk(net, "blast", "bl", group="top", grouptype="prims")
    g.setInput(0, b); d.setInput(0, g)
    return d
finish(SOP, SCENE, build, NOTE)
