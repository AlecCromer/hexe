from _common import *
SOP = "sop-mountain"
NOTE = "STAT oracle (noise streams differ): 70x70 grid, mountain height 0.5 elementsize 0.4; metric = mean/std of |P-base| +-5 %; Houdini twin perlin basis, noise range zero-centered, offset 3 for seed"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"rows": 70, "cols": 70}}, {"name": "mountain1", "type": "mountain", "params": {"height": 0.5, "elementsize": 0.4, "seed": 3}}], "edges": [["grid1", 0, "mountain1", 0]], "display": "mountain1"}
def build(net):
    g = mk(net, "grid", "grid1", rows=70, cols=70)
    m = mk(net, "mountain::2.0", "mountain1", height=0.5, elementsize=0.4, basis="perlin", fractal="none")
    m.setInput(0, g)
    return m
finish(SOP, SCENE, build, NOTE, oracle="stat", stat="disp")
