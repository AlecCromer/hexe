from _common import *
SOP = "sop-revolve-arc"
NOTE = "same profile, partial sweep 0..180 deg, divs 6, open arc, quads"
SCENE = {"nodes": [{"name": "profile", "type": "circle", "params": {"orient": 0, "center": [1.5, 0.0, 0.0], "radius": [0.5, 0.5, 0.0], "divisions": 12, "reverse": False}}, {"name": "revolve1", "type": "revolve", "params": {"divs": 6, "startangle": 0.0, "endangle": 180.0}}], "edges": [["profile", 0, "revolve1", 0]], "display": "revolve1"}
def build(net):
    c = mk(net, "circle", "profile", type=1, orient=0, t=(1.5, 0, 0), rad=(0.5, 0.5), divs=12, reverse=0)
    r = mk(net, "revolve::2.0", "revolve1", divs=6, type=1, beginangle=0, endangle=180, surftype=5, primtype=1)
    r.setInput(0, c)
    return r
finish(SOP, SCENE, build, NOTE)
