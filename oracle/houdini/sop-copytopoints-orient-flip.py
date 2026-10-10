from _common import *
SOP = "sop-copytopoints-orient-flip"
NOTE = "box onto grid(3x3) points with N = -Z on every target (antiparallel); Houdini orients by N by default "
SN = "@N = set(0.0, 0.0, -1.0);"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "grid1", "type": "grid", "params": {"rows": 3, "cols": 3, "type": 4}}, {"name": "w", "type": "wrangle", "params": {"snippet": SN}}, {"name": "copy1", "type": "copytopoints", "params": {}}], "edges": [["grid1", 0, "w", 0], ["box1", 0, "copy1", 0], ["w", 0, "copy1", 1]], "display": "copy1"}
def build(net):
    b = mk(net, "box", "box1"); g = mk(net, "grid", "grid1", rows=3, cols=3, type=4)
    w = mk(net, "attribwrangle", "w", **{"class": "point", "snippet": SN}); w.setInput(0, g)
    c = mk(net, "copytopoints::2.0", "copy1")
    c.setInput(0, b); c.setInput(1, w)
    return c
finish(SOP, SCENE, build, NOTE)
