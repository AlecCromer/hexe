from _common import *
SOP = "sop-groupcreate-pointnormal"
NOTE = "box -> groupcreate POINTS by normal dir(0,1,0) angle 90 (top points 4-7) -> blast points keep selected"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "gc", "type": "groupcreate", "params": {"groupname": "sel", "grouptype": 1, "groupnormal": True, "dir": [0, 1, 0], "angle": 90}}, {"name": "bl", "type": "blast", "params": {"group": "sel", "grouptype": 0, "negate": True}}], "edges": [["box1", 0, "gc", 0], ["gc", 0, "bl", 0]], "display": "bl"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "groupcreate", "gc", groupname="sel", grouptype="point", groupnormal=1, dir=(0, 1, 0), angle=90)
    d = mk(net, "blast", "bl", group="sel", grouptype="points", negate=1)
    g.setInput(0, b); d.setInput(0, g)
    return d
finish(SOP, SCENE, build, NOTE)
