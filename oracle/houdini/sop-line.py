from _common import *
SOP = "sop-line"
NOTE = "line origin(-2,0,0) dir(1,0,0) dist 4 points 5 (curves.json)"
SCENE = {"nodes": [{"name": "line1", "type": "line", "params": {"origin": [-2, 0, 0], "dir": [1, 0, 0], "dist": 4, "points": 5}}], "edges": [], "display": "line1"}
def build(net):
    return mk(net, "line", "line1", origin=(-2, 0, 0), dir=(1, 0, 0), dist=4, points=5)
finish(SOP, SCENE, build, NOTE)
