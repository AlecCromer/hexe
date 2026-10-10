from _common import *
SOP = "sop-revolve-tris"
NOTE = "same profile, full sweep divs 8, triangles"
SCENE = {"nodes": [{"name": "profile", "type": "circle", "params": {"orient": 0, "center": [1.5, 0.0, 0.0], "radius": [0.5, 0.5, 0.0], "divisions": 12, "reverse": False}}, {"name": "revolve1", "type": "revolve", "params": {"divs": 8, "tris": True}}], "edges": [["profile", 0, "revolve1", 0]], "display": "revolve1"}
def build(net):
    c = mk(net, "circle", "profile", type=1, orient=0, t=(1.5, 0, 0), rad=(0.5, 0.5), divs=12, reverse=0)
    r = mk(net, "revolve::2.0", "revolve1", divs=8, surftype=4, primtype=1)
    r.setInput(0, c)
    return r
finish(SOP, SCENE, build, NOTE)
