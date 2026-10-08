from _common import *
SOP = "sop-groupdelete"
NOTE = "box -> group 'top' -> groupdelete top"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "top", "type": "group", "params": {"groupname": "top", "grouptype": 1, "normal": True, "dir": [0, 1, 0], "angle": 10}}, {"name": "gd", "type": "groupdelete", "params": {"group": "top"}}], "edges": [["box1", 0, "top", 0], ["top", 0, "gd", 0]], "display": "gd"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "group", "top", crname="top", entity="primitive", groupnormal=1, dir=(0, 1, 0), angle=10)
    d = mk(net, "groupdelete", "gd")
    d.parm("deletions").set(1); d.parm("enable1").set(1); d.parm("group1").set("top")
    g.setInput(0, b); d.setInput(0, g)
    return d
finish(SOP, SCENE, build, NOTE)
