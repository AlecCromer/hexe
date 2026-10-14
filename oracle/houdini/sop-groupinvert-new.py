from _common import *
SOP = "sop-groupinvert-new"
NOTE = "box -> groupcreate a=prims 0-2 -> groupinvert a newname b (a kept) -> blast keep b (= prims 3-5); a+b both present"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "ga", "type": "groupcreate", "params": {"groupname": "a", "basegroup": "0-2"}}, {"name": "inv", "type": "groupinvert", "params": {"group": "a", "grouptype": 1, "newname": "b"}}, {"name": "bl", "type": "blast", "params": {"group": "b", "grouptype": 1, "negate": True}}], "edges": [["box1", 0, "ga", 0], ["ga", 0, "inv", 0], ["inv", 0, "bl", 0]], "display": "bl"}
def build(net):
    b = mk(net, "box", "box1")
    g = mk(net, "groupcreate", "ga", groupname="a", grouptype="primitive", basegroup="0-2"); g.setInput(0, b)
    i = mk(net, "groupinvert", "inv", group="a", grouptype="prims", newname="b"); i.setInput(0, g)
    d = mk(net, "blast", "bl", group="b", grouptype="prims", negate=1); d.setInput(0, i)
    return d
finish(SOP, SCENE, build, NOTE)
