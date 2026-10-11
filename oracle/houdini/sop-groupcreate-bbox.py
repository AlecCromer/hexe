from _common import *
SOP = "sop-groupcreate-bbox"
NOTE = "box -> groupcreate prims by bbox (size 1.2/2/1.2 at x=.5, include not-wholly-contained) -> blast keep selected"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "gc", "type": "groupcreate", "params": {"groupname": "sel", "groupbounding": True, "size": [1.2, 2, 1.2], "t": [0.5, 0, 0], "includenotwhollycontained": True}}, {"name": "bl", "type": "blast", "params": {"group": "sel", "grouptype": 1, "negate": True}}], "edges": [["box1", 0, "gc", 0], ["gc", 0, "bl", 0]], "display": "bl"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "groupcreate", "gc", groupname="sel", grouptype="primitive", groupbounding=1, size=(1.2, 2, 1.2), t=(0.5, 0, 0), includenotwhollycontained=1)
    d = mk(net, "blast", "bl", group="sel", grouptype="prims", negate=1)
    g.setInput(0, b); d.setInput(0, g)
    return d
finish(SOP, SCENE, build, NOTE)
