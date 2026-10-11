from _common import *
SOP = "sop-groupcreate-normal"
NOTE = "sphere polymesh -> groupcreate prims by normal dir(0,1,0) angle 60 -> blast keep selected"
SCENE = {"nodes": [{"name": "sph", "type": "sphere", "params": {"type": 2}}, {"name": "gc", "type": "groupcreate", "params": {"groupname": "sel", "groupnormal": True, "dir": [0, 1, 0], "angle": 60}}, {"name": "bl", "type": "blast", "params": {"group": "sel", "grouptype": 1, "negate": True}}], "edges": [["sph", 0, "gc", 0], ["gc", 0, "bl", 0]], "display": "bl"}
def build(net):
    s = mk(net, "sphere", "sph", type=2); g = mk(net, "groupcreate", "gc", groupname="sel", grouptype="primitive", groupnormal=1, dir=(0, 1, 0), angle=60)
    d = mk(net, "blast", "bl", group="sel", grouptype="prims", negate=1)
    g.setInput(0, s); d.setInput(0, g)
    return d
finish(SOP, SCENE, build, NOTE)
