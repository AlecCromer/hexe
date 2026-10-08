from _common import *
SOP = "sop-sphere"
NOTE = "sphere type polymesh (hexe menu 2), default rows 13 cols 24 radius 1"
SCENE = {"nodes": [{"name": "sph", "type": "sphere", "params": {"type": 2}}], "edges": [], "display": "sph"}
def build(net):
    return mk(net, "sphere", "sph", type=2)
finish(SOP, SCENE, build, NOTE)
