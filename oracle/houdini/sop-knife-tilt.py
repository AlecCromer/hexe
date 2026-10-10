from _common import *
SOP = "sop-knife-tilt"
NOTE = "box, tilted plane dir (1,1,0.5) dist 0.1, keep both, groups off"
SCENE = {"nodes": [{"name": "src", "type": "box"}, {"name": "op1", "type": "knife", "params": {"origin": [0, 0, 0], "dir": [1, 1, 0.5], "dist": 0.1, "keep": 2, "seamgroup": "", "abovegroup": "", "belowgroup": ""}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "box", "src")
    c = mk(net, "knife", "op1", origin=(0, 0, 0), dir=(1, 1, 0.5), dist=0.1, knifeop=2, newg=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
