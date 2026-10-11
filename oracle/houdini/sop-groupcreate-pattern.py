from _common import *
SOP = "sop-groupcreate-pattern"
NOTE = "box -> groupcreate prims basegroup '!0-2 ^4' (unioned complement then subtract) -> blast keep selected"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "gc", "type": "groupcreate", "params": {"groupname": "sel", "basegroup": "!0-2 ^4"}}, {"name": "bl", "type": "blast", "params": {"group": "sel", "grouptype": 1, "negate": True}}], "edges": [["box1", 0, "gc", 0], ["gc", 0, "bl", 0]], "display": "bl"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "groupcreate", "gc", groupname="sel", grouptype="primitive", basegroup="!0-2 ^4")
    d = mk(net, "blast", "bl", group="sel", grouptype="prims", negate=1)
    g.setInput(0, b); d.setInput(0, g)
    return d
finish(SOP, SCENE, build, NOTE)
