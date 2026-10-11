from _common import *
SOP = "sop-pointjitter"
NOTE = "STAT oracle (Houdini RNG not reproducible): 70x70 point grid, pointjitter defaults (scale 1, uniform cube +-0.5 per axis); metric = mean/std of |P-base| +-5 %"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"rows": 70, "cols": 70, "type": 4}}, {"name": "jit1", "type": "pointjitter", "params": {}}], "edges": [["grid1", 0, "jit1", 0]], "display": "jit1"}
def build(net):
    g = mk(net, "grid", "grid1", rows=70, cols=70, type=4)
    n = mk(net, "pointjitter", "jit1")
    n.setInput(0, g)
    return n
finish(SOP, SCENE, build, NOTE, oracle="stat", stat="disp")
