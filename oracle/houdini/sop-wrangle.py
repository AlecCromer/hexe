from _common import *
SOP = "sop-wrangle"
NOTE = "grid 11x11 points (type 4) -> wrangle point '@P.y = sin(@P.x) * 0.5;' (examples/wrangle.json)"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"rows": 11, "cols": 11, "type": 4}}, {"name": "w", "type": "wrangle", "params": {"snippet": "@P.y = sin(@P.x) * 0.5;"}}], "edges": [["grid1", 0, "w", 0]], "display": "w"}
def build(net):
    g = mk(net, "grid", "grid1", rows=11, cols=11, type=4); w = mk(net, "attribwrangle", "w", **{"class": "point", "snippet": "@P.y = sin(@P.x) * 0.5;"})
    w.setInput(0, g)
    return w
finish(SOP, SCENE, build, NOTE)
