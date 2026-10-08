from _common import *
SOP = "sop-output"
NOTE = "box -> output (outputidx 0)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "out1", "type": "output"}], "edges": [["box1", 0, "out1", 0]], "display": "out1"}
def build(net):
    b = mk(net, "box", "box1")
    o = mk(net, "output", "out1", outputidx=0); o.setInput(0, b)
    return o
finish(SOP, SCENE, build, NOTE)
