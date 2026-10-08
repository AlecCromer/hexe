from _common import *
SOP = "sop-noise"
NOTE = "grid 11x11 points -> attribnoise P add amplitude 0.5 freq 0.7 seed 3 (examples/attribnoise.json); Houdini twin: attribs P, elementsize 1/0.7, offset 3 -> P values not expected to match (noise streams)"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"rows": 11, "cols": 11, "type": 4}}, {"name": "noise1", "type": "attribnoise", "params": {"amplitude": 0.5, "frequency": [0.7, 0.7, 0.7], "seed": 3}}], "edges": [["grid1", 0, "noise1", 0]], "display": "noise1"}
def build(net):
    g = mk(net, "grid", "grid1", rows=11, cols=11, type=4)
    n = mk(net, "attribnoise::2.0", "noise1", attribs="P", attribtype="vec", operation="add", amplitude=0.5, elementsize=1 / 0.7, offset=3, basis="perlin")
    n.setInput(0, g)
    return n
finish(SOP, SCENE, build, NOTE)
