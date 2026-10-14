from _common import *
SOP = "sop-groupexpand-edge"
NOTE = "prim group s=5 -> groupexpand prims primshareedge numsteps 2 (diamond, 11 prims) -> blast keep o"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 5, "cols": 5}}, {"name": "gs", "type": "groupcreate", "params": {"groupname": "s", "grouptype": 0, "basegroup": "5"}}, {"name": "ex", "type": "groupexpand", "params": {"outputgroup": "o", "group": "s", "grouptype": 4, "primshareedge": True, "numsteps": 2}}, {"name": "bl", "type": "blast", "params": {"group": "o", "grouptype": 1, "negate": True}}], "edges": [["g", 0, "gs", 0], ["gs", 0, "ex", 0], ["ex", 0, "bl", 0]], "display": "bl"}
def build(net):
    g = mk(net, "grid", "g", rows=5, cols=5)
    s = mk(net, "groupcreate", "gs", groupname="s", grouptype="primitive", basegroup="5"); s.setInput(0, g)
    e = mk(net, "groupexpand", "ex", outputgroup="o", group="s", grouptype=4, primshareedge=1, numsteps=2); e.setInput(0, s)
    d = mk(net, "blast", "bl", group="o", grouptype="prims", negate=1); d.setInput(0, e)
    return d
finish(SOP, SCENE, build, NOTE)
