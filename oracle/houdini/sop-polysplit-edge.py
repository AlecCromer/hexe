from _common import *
SOP = "sop-polysplit-edge"
NOTE = 'polysplit prim-edge tokens NeM: 0e0 = edge 0->1 of prim 0, 0e2 = 4->3 (same result as p0-1 p3-4)'
P = {'splitloc': '0e0 0e2'}
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"size": [10, 10, 0], "rows": 3, "cols": 3}}, {"name": "op1", "type": "polysplit", "params": P}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    g = mk(net, "grid", "src", size=(10, 10), rows=3, cols=3)
    k = mk(net, "polysplit", "op1", **P)
    k.setInput(0, g)
    return k
finish(SOP, SCENE, build, NOTE)
