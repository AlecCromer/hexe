from _common import *
SOP = "sop-knife-below"
NOTE = "box, plane y=0 keep below, groups off"
SCENE = {"nodes": [{"name": "src", "type": "box"}, {"name": "op1", "type": "knife", "params": {"origin": [0, 0, 0], "dir": [0, 1, 0], "keep": 1, "seamgroup": "", "abovegroup": "", "belowgroup": ""}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "box", "src")
    c = mk(net, "knife", "op1", origin=(0, 0, 0), dir=(0, 1, 0), knifeop=1, newg=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
