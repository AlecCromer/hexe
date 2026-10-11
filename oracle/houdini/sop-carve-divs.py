from _common import *
SOP = "sop-carve-divs"
NOTE = "carve u .2..0.7 by arc length, divsu 4, keep inside + outside on an open 5-point polygon"
PTS = [(0, 0, 0), (1, 2, 0), (3, 2, 0), (4, 0, 0), (5, 1, 1)]
P = {"arclen": True, "firstu": True, "domainu1": 0.2, "secondu": True, "domainu2": 0.7, "divsu": 4, "keepin": True, "keepout": True}
SCENE = {"nodes": [{"name": "src", "type": "curve", "params": {"coords": "0,0,0 1,2,0 3,2,0 4,0,0 5,1,1", "close": False}}, {"name": "op1", "type": "carve", "params": P}], "edges": [["src", 0, "op1", 0]], "display": "op1"}
def build(net):
    c = poly_curve(net, "src", PTS)
    k = mk(net, "carve", "op1", arclen=1, firstu=1, domainu1=0.2, secondu=1, domainu2=0.7, divsu=4, keepin=1, keepout=1)
    k.setInput(0, c)
    return k
finish(SOP, SCENE, build, NOTE)
