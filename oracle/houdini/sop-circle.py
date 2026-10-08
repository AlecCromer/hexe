from _common import *
SOP = "sop-circle"
NOTE = "circle polygon divisions 6 radius (2,1) XY plane (curves.json)"
SCENE = {"nodes": [{"name": "circle1", "type": "circle", "params": {"divisions": 6, "radius": [2, 1, 0]}}], "edges": [], "display": "circle1"}
def build(net):
    return mk(net, "circle", "circle1", type=1, divs=6, rad=(2, 1))
finish(SOP, SCENE, build, NOTE)
