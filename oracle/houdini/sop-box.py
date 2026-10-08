from _common import *
SOP = "sop-box"
NOTE = "canonical: default box (size 1, polygon, consolidated)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}], "edges": [], "display": "box1"}
def build(net):
    return mk(net, "box", "box1")
finish(SOP, SCENE, build, NOTE)
