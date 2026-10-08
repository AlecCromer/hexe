from _common import *
SOP = "sop-normal"
NOTE = "box -> normal vertex cusp 60"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "n", "type": "normal", "params": {"type": 1, "cusp_angle": 60}}], "edges": [["box1", 0, "n", 0]], "display": "n"}
def build(net):
    b = mk(net, "box", "box1"); n = mk(net, "normal", "n", type=1, cuspangle=60)
    n.setInput(0, b)
    return n
finish(SOP, SCENE, build, NOTE)
