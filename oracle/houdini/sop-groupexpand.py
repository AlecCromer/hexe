from _common import *
SOP = "sop-groupexpand"
NOTE = "5x5 point grid, prim group s=prim 5 -> groupexpand prims (share POINT, 1 step) -> o = 3x3 block -> blast keep o"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 5, "cols": 5}}, {"name": "gs", "type": "groupcreate", "params": {"groupname": "s", "grouptype": 0, "basegroup": "5"}}, {"name": "ex", "type": "groupexpand", "params": {"outputgroup": "o", "group": "s", "grouptype": 4}}, {"name": "bl", "type": "blast", "params": {"group": "o", "grouptype": 1, "negate": True}}], "edges": [["g", 0, "gs", 0], ["gs", 0, "ex", 0], ["ex", 0, "bl", 0]], "display": "bl"}
def build(net):
    g = mk(net, "grid", "g", rows=5, cols=5)
    s = mk(net, "groupcreate", "gs", groupname="s", grouptype="primitive", basegroup="5"); s.setInput(0, g)
    e = mk(net, "groupexpand", "ex", outputgroup="o", group="s", grouptype=4); e.setInput(0, s)
    d = mk(net, "blast", "bl", group="o", grouptype="prims", negate=1); d.setInput(0, e)
    return d
finish(SOP, SCENE, build, NOTE)
