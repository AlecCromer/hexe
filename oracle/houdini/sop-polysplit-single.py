from _common import *
SOP = "sop-polysplit-single"
NOTE = 'polysplit single edge token: point inserted into both prims sharing the edge, no split'
P = {'splitloc': '0e1'}
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"size": [10, 10, 0], "rows": 3, "cols": 3}}, {"name": "op1", "type": "polysplit", "params": P}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    g = mk(net, "grid", "src", size=(10, 10), rows=3, cols=3)
    k = mk(net, "polysplit", "op1", **P)
    k.setInput(0, g)
    return k
finish(SOP, SCENE, build, NOTE)
