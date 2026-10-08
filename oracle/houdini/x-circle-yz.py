from _common import *
SOP = "x-circle-yz"
NOTE = "extra case: circle YZ plane, radius (2,1), 6 divisions, reverse OFF (hexe reverse=false)"
SCENE = {"nodes": [{"name": "circle1", "type": "circle", "params": {"orient": 1, "divisions": 6, "radius": [2, 1, 0], "reverse": False}}], "edges": [], "display": "circle1"}
def build(net):
    return mk(net, "circle", "circle1", type=1, orient=1, divs=6, rad=(2, 1), reverse=0)
finish(SOP, SCENE, build, NOTE)
