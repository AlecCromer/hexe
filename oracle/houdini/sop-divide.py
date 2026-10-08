from _common import *
SOP = "sop-divide"
NOTE = "box -> divide convex, max 3 sides (divide.json) = triangulate"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "divide1", "type": "divide", "params": {"convex": True, "numsides": 3}}], "edges": [["box1", 0, "divide1", 0]], "display": "divide1"}
def build(net):
    b = mk(net, "box", "box1")
    d = mk(net, "divide", "divide1", convex=1, usemaxsides=1, numsides=3)
    d.setInput(0, b)
    return d
finish(SOP, SCENE, build, NOTE)
