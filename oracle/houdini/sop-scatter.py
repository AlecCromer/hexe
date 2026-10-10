from _common import *
SOP = "sop-scatter"
NOTE = "STAT oracle (RNG stream not reproducible): grid(10x10 default) -> scatter 2000 pts seed 3, relax OFF. count exact, bbox +-2 %, mean NN +-5 %, P std +-5 %, attr names exact"
SCENE = {"nodes": [{"name": "grid1", "type": "grid"}, {"name": "scatter1", "type": "scatter", "params": {"npts": 2000, "seed": 3}}], "edges": [["grid1", 0, "scatter1", 0]], "display": "scatter1"}
def build(net):
    g = mk(net, "grid", "grid1")
    s = mk(net, "scatter", "scatter1", npts=2000, seed=3, relaxpoints=0)
    s.setInput(0, g)
    return s
finish(SOP, SCENE, build, NOTE, oracle="stat")
