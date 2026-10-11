from _common import *
SOP = "sop-attribrandomize"
NOTE = "STAT oracle (Houdini RNG not reproducible): 70x70 point grid, attribrandomize P add uniform -0.5..0.5 (dims 3, scale 1); metric = mean/std of |P-base| +-5 %"
PARAMS = {"name": "P", "class": 2, "operation": 1, "distribution": 1, "dimensions": 3, "minx": -0.5, "miny": -0.5, "minz": -0.5, "maxx": 0.5, "maxy": 0.5, "maxz": 0.5}
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"rows": 70, "cols": 70, "type": 4}}, {"name": "rnd1", "type": "attribrandomize", "params": PARAMS}], "edges": [["grid1", 0, "rnd1", 0]], "display": "rnd1"}
def build(net):
    g = mk(net, "grid", "grid1", rows=70, cols=70, type=4)
    n = mk(net, "attribrandomize", "rnd1")
    for k, v in ("name", "P"), ("operation", "add"), ("distribution", "uniform"), ("minx", -0.5), ("miny", -0.5), ("minz", -0.5), ("maxx", 0.5), ("maxy", 0.5), ("maxz", 0.5):
        n.parm(k).set(v)
    n.setInput(0, g)
    return n
finish(SOP, SCENE, build, NOTE, oracle="stat", stat="disp")
