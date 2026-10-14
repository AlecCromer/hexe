from _common import *
SOP = "sop-edgedivide"
NOTE = "3x3 point grid -> wrangle (f@h=ptnum, i@k=ptnum) -> edgedivide group p1-4: interior edge p1-4, default numdivs: one midpoint per adjacent prim"
WR = "f@h = @ptnum; i@k = @ptnum;"
PARAMS = {}
SCENE = {"nodes": [{"name": "g", "type": "grid", "params": {"rows": 3, "cols": 3}}, {"name": "w", "type": "wrangle", "params": {"snippet": WR}}, {"name": "ed", "type": "edgedivide", "params": dict({k: (bool(v) if k == "sharedpoints" else v) for k, v in PARAMS.items()}, group="p1-4")}], "edges": [["g", 0, "w", 0], ["w", 0, "ed", 0]], "display": "ed"}
def build(net):
    g = mk(net, "grid", "g", rows=3, cols=3)
    w = mk(net, "attribwrangle", "w", **{"class": "point", "snippet": WR}); w.setInput(0, g)
    e = mk(net, "edgedivide", "ed", group="p1-4", **PARAMS); e.setInput(0, w)
    return e
finish(SOP, SCENE, build, NOTE)
