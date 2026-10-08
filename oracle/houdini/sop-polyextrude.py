from _common import *
SOP = "sop-polyextrude"
NOTE = "box -> polyextrude dist 0.5 divs 1 (front+side)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "pe", "type": "polyextrude", "params": {"distance": 0.5, "divisions": 1}}], "edges": [["box1", 0, "pe", 0]], "display": "pe"}
def build(net):
    b = mk(net, "box", "box1"); p = mk(net, "polyextrude::2.0", "pe", dist=0.5, divs=1)
    p.setInput(0, b)
    return p
finish(SOP, SCENE, build, NOTE)
