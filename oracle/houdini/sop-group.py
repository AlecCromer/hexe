from _common import *
SOP = "sop-group"
NOTE = "box -> group 'top' prims by normal dir(0,1,0) angle 10 (examples/group-blast.json)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "top", "type": "group", "params": {"groupname": "top", "grouptype": 1, "normal": True, "dir": [0, 1, 0], "angle": 10}}], "edges": [["box1", 0, "top", 0]], "display": "top"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "group", "top", crname="top", entity="primitive", groupnormal=1, dir=(0, 1, 0), angle=10)
    g.setInput(0, b)
    return g
finish(SOP, SCENE, build, NOTE)
