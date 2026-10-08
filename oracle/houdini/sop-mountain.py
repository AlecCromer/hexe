from _common import *
SOP = "sop-mountain"
NOTE = "grid 11x11 -> mountain height .5 elementsize 3 (mountain.json). BEST GUESS: Houdini perlin basis, fractal none, 1 octave; seed has no direct twin (offset); noise streams differ by construction"
SCENE = {"nodes": [{"name": "grid1", "type": "grid", "params": {"rows": 11, "cols": 11}}, {"name": "mountain1", "type": "mountain", "params": {"height": 0.5, "elementsize": 3, "seed": 3}}], "edges": [["grid1", 0, "mountain1", 0]], "display": "mountain1"}
def build(net):
    g = mk(net, "grid", "grid1", rows=11, cols=11)
    m = mk(net, "mountain::2.0", "mountain1", height=0.5, elementsize=3, basis="perlin", fractal="none", oct=1)
    m.setInput(0, g)
    return m
finish(SOP, SCENE, build, NOTE)
