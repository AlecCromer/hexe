from _common import *
SOP = "sop-polyframe-quad"
NOTE = "polyframe two edges, ortho + lefthanded + tangentv, custom names, on a skew closed quad"
Q = [(0, 0, 0), (2, 0, 0), (2.5, 1.5, 0.5), (0, 1, 0)]
P = {"ortho": True, "lefthanded": True, "tangentvon": True, "N": "nn", "tangentu": "tu", "tangentv": "tv"}
SCENE = {"nodes": [{"name": "src", "type": "curve", "params": {"coords": coords(Q), "close": True}}, {"name": "op1", "type": "polyframe", "params": P}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    c = poly_curve(net, "src", Q, closed=True)
    k = mk(net, "polyframe", "op1", ortho=1, lefthanded=1, tangentvon=1, N="nn", tangentu="tu", tangentv="tv")
    k.setInput(0, c)
    return k
finish(SOP, SCENE, build, NOTE)
