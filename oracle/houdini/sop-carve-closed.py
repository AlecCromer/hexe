from _common import *
SOP = "sop-carve-closed"
NOTE = "carve u .2..0.7 keep outside only on a CLOSED 5-point polygon (wrapped out piece, seam point twice); plus keepin off"
PTS = [(0, 0, 0), (1, 2, 0), (3, 2, 0), (4, 0, 0), (5, 1, 1)]
P = {"secondu": True, "domainu1": 0.2, "domainu2": 0.7, "keepin": False, "keepout": True}
SCENE = {"nodes": [{"name": "src", "type": "curve", "params": {"coords": "0,0,0 1,2,0 3,2,0 4,0,0 5,1,1", "close": True}}, {"name": "op1", "type": "carve", "params": P}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    c = poly_curve(net, "src", PTS, closed=True)
    k = mk(net, "carve", "op1", secondu=1, domainu1=0.2, domainu2=0.7, keepin=0, keepout=1)
    k.setInput(0, c)
    return k
finish(SOP, SCENE, build, NOTE)
