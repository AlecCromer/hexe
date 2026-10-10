from _common import *
SOP = "sop-noise-oct"
NOTE = "STAT oracle (noise streams differ): 70x70 grid, attribnoise amplitude 0.5 freq 2.0, 4 octaves fBm rough 0.5 lac 2; metric = mean/std of |P-base| +-5 %; Houdini twin perlin basis, offset 3 for seed; Houdini fBm carries a DC offset (+0.1846 per component at amp 0.5, 4 octaves) which hexe does not model, so the twin recentres it with noiserange midplusminusrange midvaluev -0.1846 (parms: noiserange, midvaluev)"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"rows": 70, "cols": 70, "type": 4}}, {"name": "noise1", "type": "attribnoise", "params": {"amplitude": 0.5, "frequency": [2.0, 2.0, 2.0], "seed": 3, "fractal": 1, "octaves": 4, "rough": 0.5}}], "edges": [["grid1", 0, "noise1", 0]], "display": "noise1"}
def build(net):
    g = mk(net, "grid", "grid1", rows=70, cols=70, type=4)
    n = mk(net, "attribnoise::2.0", "noise1", attribs="P", attribtype="vec", operation="add", noiserange="midplusminusrange", midvaluev=-0.1846, basis="perlin", amplitude=0.5, elementsize=1 / 2.0, offset=3, fractal="fBm", oct=4, rough=0.5, lac=2.0)
    n.setInput(0, g)
    return n
finish(SOP, SCENE, build, NOTE, oracle="stat", stat="disp")
