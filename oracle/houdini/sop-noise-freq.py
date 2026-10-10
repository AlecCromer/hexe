from _common import *
SOP = "sop-noise-freq"
NOTE = "STAT oracle (noise streams differ): 70x70 grid, attribnoise amplitude 0.5 freq 5.0; metric = mean/std of |P-base| +-5 %; Houdini twin perlin basis, noise range zero-centered, offset 3 for seed"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"rows": 70, "cols": 70, "type": 4}}, {"name": "noise1", "type": "attribnoise", "params": {"amplitude": 0.5, "frequency": [5.0, 5.0, 5.0], "seed": 3}}], "edges": [["grid1", 0, "noise1", 0]], "display": "noise1"}
def build(net):
    g = mk(net, "grid", "grid1", rows=70, cols=70, type=4)
    n = mk(net, "attribnoise::2.0", "noise1", attribs="P", attribtype="vec", operation="add", noiserange="zcentered", basis="perlin", amplitude=0.5, elementsize=1 / 5.0, offset=3, fractal="none")
    n.setInput(0, g)
    return n
finish(SOP, SCENE, build, NOTE, oracle="stat", stat="disp")
