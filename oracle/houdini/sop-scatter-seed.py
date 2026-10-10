from _common import *
SOP = "sop-scatter-seed"
NOTE = "STAT oracle: box (default, 3D surface) -> scatter 1500 pts seed 11, relax OFF, source-prim attribute `sourceprim` on (useprimnumattrib)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "scatter1", "type": "scatter", "params": {"npts": 1500, "seed": 11, "primnumattrib": "sourceprim"}}], "edges": [["box1", 0, "scatter1", 0]], "display": "scatter1"}
def build(net):
    b = mk(net, "box", "box1")
    s = mk(net, "scatter", "scatter1", npts=1500, seed=11, relaxpoints=0, useprimnumattrib=1, primnumattrib="sourceprim")
    s.setInput(0, b)
    return s
finish(SOP, SCENE, build, NOTE, oracle="stat")
