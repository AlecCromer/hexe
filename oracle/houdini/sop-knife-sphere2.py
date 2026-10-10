from _common import *
SOP = "sop-knife-sphere2"
NOTE = "polymesh sphere, plane dir (-0.7,0.4,0.55) dist -0.15, keep both: closing-edge-new slot rule + piece cycle start"
SCENE = {"nodes": [{"name": "src", "type": "sphere", "params": {"type": 2}}, {"name": "op1", "type": "knife", "params": {"origin": [0, 0, 0], "dir": [-0.7, 0.4, 0.55], "dist": -0.15, "keep": 2, "seamgroup": "", "abovegroup": "", "belowgroup": ""}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "sphere", "src", type=2)
    c = mk(net, "knife", "op1", origin=(0, 0, 0), dir=(-0.7, 0.4, 0.55), dist=-0.15, knifeop=2, newg=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
