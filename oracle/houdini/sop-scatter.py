from _common import *
SOP = "sop-scatter"
NOTE = "grid(10x10 default) -> scatter npts 500 seed 7, relax OFF (hexe has no relax). RNG stream is not expected to match"
SCENE = {"nodes": [{"name": "grid1", "type": "grid"}, {"name": "scatter1", "type": "scatter", "params": {"npts": 500, "seed": 7}}], "edges": [["grid1", 0, "scatter1", 0]], "display": "scatter1"}
def build(net):
    g = mk(net, "grid", "grid1")
    s = mk(net, "scatter", "scatter1", npts=500, seed=7, relaxpoints=0)
    s.setInput(0, g)
    return s
finish(SOP, SCENE, build, NOTE)
