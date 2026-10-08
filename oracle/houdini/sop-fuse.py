from _common import *
SOP = "sop-fuse"
NOTE = "box -> polyextrude 0.5 -> fuse 0.001 (examples/fuse.json)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "pe1", "type": "polyextrude", "params": {"distance": 0.5}}, {"name": "fuse1", "type": "fuse", "params": {"distance": 0.001}}], "edges": [["box1", 0, "pe1", 0], ["pe1", 0, "fuse1", 0]], "display": "fuse1"}
def build(net):
    b = mk(net, "box", "box1"); p = mk(net, "polyextrude::2.0", "pe1", dist=0.5)
    f = mk(net, "fuse::2.0", "fuse1", tol3d=0.001)
    p.setInput(0, b); f.setInput(0, p)
    return f
finish(SOP, SCENE, build, NOTE)
