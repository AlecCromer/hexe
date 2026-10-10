from _common import *
SOP = "sop-scatter-density"
SNIP = "f@density = clamp((@P.x + 5.0) / 10.0, 0.0, 1.0);"
NOTE = "STAT oracle: grid points get density = clamp((x+5)/10,0,1) (wrangle) -> scatter 2000 pts seed 5 weighted by point attr `density`, attribs transferred (*), relax OFF"
SCENE = {"nodes": [{"name": "grid1", "type": "grid"}, {"name": "w", "type": "wrangle", "params": {"snippet": SNIP}}, {"name": "scatter1", "type": "scatter", "params": {"npts": 2000, "seed": 5, "density_attrib": "density", "attribs": "*"}}], "edges": [["grid1", 0, "w", 0], ["w", 0, "scatter1", 0]], "display": "scatter1"}
def build(net):
    g = mk(net, "grid", "grid1")
    w = mk(net, "attribwrangle", "w", **{"class": "point", "snippet": SNIP})
    w.setInput(0, g)
    s = mk(net, "scatter", "scatter1", npts=2000, seed=5, relaxpoints=0, usedensityattrib=1, densityattrib="density")
    s.setInput(0, w)
    return s
finish(SOP, SCENE, build, NOTE, oracle="stat")
