from _common import *
SOP = "sop-polyframe-primc"
NOTE = "polyframe primitive-centroid style on a closed quad merged with an open 3-point polyline"
Q = [(0, 0, 0), (2, 0, 0), (2.5, 1.5, 0.5), (0, 1, 0)]
L = [(0, 0, 0), (1, 0, 0), (1, 1, 0)]
SCENE = {"nodes": [{"name": "q", "type": "curve", "params": {"coords": coords(Q), "close": True}}, {"name": "l", "type": "curve", "params": {"coords": coords(L), "close": False}}, {"name": "m", "type": "merge"}, {"name": "op1", "type": "polyframe", "params": {"style": 2, "tangentvon": True}}], "edges": [["q", 0, "m", 0], ["l", 0, "m", 1], ["m", 0, "op1", 0]], "display": "op1"}
def build(net):
    a = poly_curve(net, "q", Q, closed=True); b = poly_curve(net, "l", L)
    m = mk(net, "merge", "m"); m.setInput(0, a); m.setInput(1, b)
    k = mk(net, "polyframe", "op1", style="primC", tangentvon=1)
    k.setInput(0, m)
    return k
finish(SOP, SCENE, build, NOTE)
