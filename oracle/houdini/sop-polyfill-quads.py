from _common import *
SOP = "sop-polyfill-quads"
NOTE = "box minus top face (group+blast) -> polyfill quadfan (hexe fillmode 2 -> Houdini fillmode 3)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "top", "type": "group", "params": {"groupname": "top", "grouptype": 1, "normal": True, "dir": [0, 1, 0], "angle": 10}}, {"name": "hole", "type": "blast", "params": {"group": "top", "grouptype": 1}}, {"name": "polyfill1", "type": "polyfill", "params": {"fillmode": 2, "filledgroup": ""}}], "edges": [["box1", 0, "top", 0], ["top", 0, "hole", 0], ["hole", 0, "polyfill1", 0]], "display": "polyfill1"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "group", "top", crname="top", entity="primitive", groupnormal=1, dir=(0, 1, 0), angle=10)
    d = mk(net, "blast", "hole", group="top", grouptype="prims"); f = mk(net, "polyfill", "polyfill1", fillmode=3)
    g.setInput(0, b); d.setInput(0, g); f.setInput(0, d)
    return f
finish(SOP, SCENE, build, NOTE)
