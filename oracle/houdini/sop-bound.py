from _common import *
SOP = "sop-bound"
NOTE = "box(t 1,2,3 size 1,2,3) -> bound (defaults: box, no pad)"
SCENE = {"nodes": [{"name": "box1", "type": "box", "params": {"t": [1, 2, 3], "size": [1, 2, 3]}}, {"name": "bd", "type": "bound"}], "edges": [["box1", 0, "bd", 0]], "display": "bd"}
def build(net):
    b = mk(net, "box", "box1", t=(1, 2, 3), size=(1, 2, 3)); n = mk(net, "bound", "bd")
    n.setInput(0, b)
    return n
finish(SOP, SCENE, build, NOTE)
