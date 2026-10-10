from _common import *
SOP = "sop-knife-sphere"
NOTE = "polymesh sphere, tilted plane, keep both, groups off"
SCENE = {"nodes": [{"name": "src", "type": "sphere", "params": {"type": 2}}, {"name": "op1", "type": "knife", "params": {"origin": [0, 0, 0], "dir": [0.3, 1, 0.2], "dist": 0.1, "keep": 2, "seamgroup": "", "abovegroup": "", "belowgroup": ""}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "sphere", "src", type=2)
    c = mk(net, "knife", "op1", origin=(0, 0, 0), dir=(0.3, 1, 0.2), dist=0.1, knifeop=2, newg=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
