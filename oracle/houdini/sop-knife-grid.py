from _common import *
SOP = "sop-knife-grid"
NOTE = "grid 6x4 rows4 cols3, tilted plane dir (1,0,0.4) dist 0.3, keep both, groups off"
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"size": [6, 4, 0], "rows": 4, "cols": 3}}, {"name": "op1", "type": "knife", "params": {"origin": [0, 0, 0], "dir": [1, 0, 0.4], "dist": 0.3, "keep": 2, "seamgroup": "", "abovegroup": "", "belowgroup": ""}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    b = mk(net, "grid", "src", size=(6, 4), rows=4, cols=3)
    c = mk(net, "knife", "op1", origin=(0, 0, 0), dir=(1, 0, 0.4), dist=0.3, knifeop=2, newg=0)
    c.setInput(0, b)
    return c
finish(SOP, SCENE, build, NOTE)
