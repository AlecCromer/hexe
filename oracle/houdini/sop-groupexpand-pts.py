from _common import *
SOP = "sop-groupexpand-pts"
NOTE = "5x5 point grid, point group s=7 -> groupexpand points (edge neighbours, 1 step) -> group o (point group names + counts compared; membership [2,6,7,8,12] pinned from the hython probe in the unit test, no blast: Houdini blast keeps shortened prims on point deletion)"
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 5, "cols": 5}}, {"name": "gs", "type": "groupcreate", "params": {"groupname": "s", "grouptype": 1, "basegroup": "7"}}, {"name": "ex", "type": "groupexpand", "params": {"outputgroup": "o", "group": "s", "grouptype": 3}}], "edges": [["g", 0, "gs", 0], ["gs", 0, "ex", 0]], "display": "ex"}
def build(net):
    g = mk(net, "grid", "g", rows=5, cols=5)
    s = mk(net, "groupcreate", "gs", groupname="s", grouptype="point", basegroup="7"); s.setInput(0, g)
    e = mk(net, "groupexpand", "ex", outputgroup="o", group="s", grouptype=3); e.setInput(0, s)
    return e
finish(SOP, SCENE, build, NOTE)
