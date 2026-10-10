from _common import *
SOP = "sop-knife-box-above"
NOTE = "box, plane dir (0.2,-1,0.6) dist 0.05, keep above only (knifeop 0): slot/append layout under single-side keep"
SCENE = {"nodes": [{"name": "src", "type": "box"}, {"name": "op1", "type": "knife", "params": {"origin": [0, 0, 0], "dir": [0.2, -1, 0.6], "dist": 0.05, "keep": 0, "seamgroup": "", "abovegroup": "", "belowgroup": ""}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "box", "src")
    c = mk(net, "knife", "op1", origin=(0, 0, 0), dir=(0.2, -1, 0.6), dist=0.05, knifeop=0, newg=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
