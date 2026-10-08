from _common import *
SOP = "sop-peak"
NOTE = "box -> peak distance 0.25 (no N update)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "pk", "type": "peak", "params": {"distance": 0.25}}], "edges": [["box1", 0, "pk", 0]], "display": "pk"}
def build(net):
    b = mk(net, "box", "box1"); p = mk(net, "peak", "pk", dist=0.25, updatenmls=0)
    p.setInput(0, b)
    return p
finish(SOP, SCENE, build, NOTE)
