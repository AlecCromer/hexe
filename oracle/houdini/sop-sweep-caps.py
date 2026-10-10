from _common import *
SOP = "sop-sweep-caps"
NOTE = "open arc backbone + section -> sweep with Single Polygon end caps (hexe startcap+endcap)"
SCENE = {"nodes": [{"name": "backbone", "type": "circle", "params": {"type": 1, "orient": 2, "arc": 1, "radius": [2.0, 2.0, 0.0], "divisions": 24, "beginangle": 0, "endangle": 270}}, {"name": "section", "type": "circle", "params": {"orient": 0, "radius": [0.3, 0.3, 0.0], "divisions": 12, "reverse": False}}, {"name": "sweep1", "type": "sweep", "params": {"computeuv": False, "startcap": True, "endcap": True}}], "edges": [["backbone", 0, "sweep1", 0], ["section", 0, "sweep1", 1]], "display": "sweep1"}
def build(net):
    b = mk(net, "circle", "backbone", type=1, orient=2, arc=1, rad=(2, 2), divs=24, beginangle=0, endangle=270)
    s = mk(net, "circle", "section", type=1, orient=0, rad=(0.3, 0.3), divs=12, reverse=0)
    w = mk(net, "sweep::2.0", "sweep1", surfacetype=5, primtype=1, endcaptype=1)
    w.setInput(0, b); w.setInput(1, s)
    return w
finish(SOP, SCENE, build, NOTE)
