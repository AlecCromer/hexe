from _common import *
SOP = "sop-polysplit"
NOTE = 'polysplit path between two edge points of ONE quad (grid 3x3): quad cut in two, neighbour of the shared edge gets the point inserted; cut prims appended after uncut ones'
P = {'splitloc': 'p0-1 p3-4'}
SCENE = {"nodes": [{"name": "src", "type": "grid", "params": {"size": [10, 10, 0], "rows": 3, "cols": 3}}, {"name": "op1", "type": "polysplit", "params": P}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    g = mk(net, "grid", "src", size=(10, 10), rows=3, cols=3)
    k = mk(net, "polysplit", "op1", **P)
    k.setInput(0, g)
    return k
finish(SOP, SCENE, build, NOTE)
