from _common import *
SOP = "sop-skin"
NOTE = "3 polygon circles (12 / 8 / 12 pts) at y 0,1,2 in XZ merged -> skin quads (hexe examples/skin.json)"
SCENE = {"nodes": [{"name": "low", "type": "circle", "params": {"orient": 2, "radius": [1.0, 1.0, 0.0], "divisions": 12}}, {"name": "mid", "type": "circle", "params": {"orient": 2, "center": [0.0, 1.0, 0.0], "radius": [0.5, 0.5, 0.0], "divisions": 8}}, {"name": "high", "type": "circle", "params": {"orient": 2, "center": [0.0, 2.0, 0.0], "radius": [1.0, 1.0, 0.0], "divisions": 12}}, {"name": "merge1", "type": "merge"}, {"name": "skin1", "type": "skin"}], "edges": [["low", 0, "merge1", 0], ["mid", 0, "merge1", 1], ["high", 0, "merge1", 2], ["merge1", 0, "skin1", 0]], "display": "skin1"}
def build(net):
    a = mk(net, "circle", "low", type=1, orient=2, rad=(1, 1), divs=12)
    b = mk(net, "circle", "mid", type=1, orient=2, t=(0, 1, 0), rad=(0.5, 0.5), divs=8)
    c = mk(net, "circle", "high", type=1, orient=2, t=(0, 2, 0), rad=(1, 1), divs=12)
    m = net.createNode("merge", "merge1")
    for i, x in enumerate((a, b, c)):
        m.setInput(i, x)
    s = mk(net, "skin", "skin1", surftype=4, polys=1)
    s.setInput(0, m)
    return s
finish(SOP, SCENE, build, NOTE)
