from _common import *
SOP = "sop-polyframe-edge1"
NOTE = "polyframe style edge1 + tangentv on a skew closed quad"
Q = [(0, 0, 0), (2, 0, 0), (2.5, 1.5, 0.5), (0, 1, 0)]
SCENE = {"nodes": [{"name": "src", "type": "curve", "params": {"coords": coords(Q), "close": True}}, {"name": "op1", "type": "polyframe", "params": {"style": 0, "tangentvon": True}}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    c = poly_curve(net, "src", Q, closed=True)
    k = mk(net, "polyframe", "op1", style="edge1", tangentvon=1)
    k.setInput(0, c)
    return k
finish(SOP, SCENE, build, NOTE)
