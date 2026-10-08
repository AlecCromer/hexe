from _common import *
SOP = "sop-polybevel"
NOTE = "box -> polybevel offset 0.1 divisions 2 round (hexe filletshape 0=round -> Houdini 4)"
SCENE = {"nodes": [{"name": "box1", "type": "box"}, {"name": "bevel1", "type": "polybevel", "params": {"offset": 0.1, "divisions": 2, "filletshape": 0}}], "edges": [["box1", 0, "bevel1", 0]], "display": "bevel1"}
def build(net):
    b = mk(net, "box", "box1"); p = mk(net, "polybevel::3.0", "bevel1", offset=0.1, divisions=2, filletshape=4)
    p.setInput(0, b)
    return p
finish(SOP, SCENE, build, NOTE)
