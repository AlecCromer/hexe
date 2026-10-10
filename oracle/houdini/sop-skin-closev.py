from _common import *
SOP = "sop-skin-closev"
NOTE = "3 polygon circles (8 pts) at y 0,1,2 -> skin quads closev"
SCENE = {"nodes": [{"name": "c0", "type": "circle", "params": {"orient": 2, "center": [0.0, 0, 0.0], "radius": [1.0, 1.0, 0.0], "divisions": 8}}, {"name": "c1", "type": "circle", "params": {"orient": 2, "center": [0.0, 1, 0.0], "radius": [0.5, 0.5, 0.0], "divisions": 8}}, {"name": "c2", "type": "circle", "params": {"orient": 2, "center": [0.0, 2, 0.0], "radius": [1.0, 1.0, 0.0], "divisions": 8}}, {"name": "merge1", "type": "merge"}, {"name": "skin1", "type": "skin", "params": {"computeuv": False, "closev": True}}], "edges": [["c0", 0, "merge1", 0], ["c1", 0, "merge1", 1], ["c2", 0, "merge1", 2], ["merge1", 0, "skin1", 0]], "display": "skin1"}
def build(net):
    cs = [mk(net, "circle", "c0", type=1, orient=2, t=(0, 0, 0), rad=(1.0, 1.0), divs=8), mk(net, "circle", "c1", type=1, orient=2, t=(0, 1, 0), rad=(0.5, 0.5), divs=8), mk(net, "circle", "c2", type=1, orient=2, t=(0, 2, 0), rad=(1.0, 1.0), divs=8)]
    m = net.createNode("merge", "merge1")
    for i, x in enumerate(cs):
        m.setInput(i, x)
    s = mk(net, "skin", "skin1", surftype=4, polys=1, closev=1)
    s.setInput(0, m)
    return s
finish(SOP, SCENE, build, NOTE)
