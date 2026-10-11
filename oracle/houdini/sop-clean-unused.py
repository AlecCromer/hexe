from _common import *
SOP = "sop-clean-unused"
NOTE = "clean deldegengeo off, delunusedpts on: the collinear tri stays, the two unused points vanish"
Q = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
DG = [(3, 0, 0), (4, 0, 0), (5, 0, 0)]
SCENE = {"nodes": [{"name": "q", "type": "curve", "params": {"coords": coords(Q), "close": True}}, {"name": "dg", "type": "curve", "params": {"coords": coords(DG), "close": True}}, {"name": "ad", "type": "add", "params": {"points": "9,9,9 8,8,8"}}, {"name": "m", "type": "merge"}, {"name": "op1", "type": "clean", "params": {"deldegengeo": False, "delunusedpts": True}}], "edges": [["q", 0, "m", 0], ["dg", 0, "m", 1], ["ad", 0, "m", 2], ["m", 0, "op1", 0]], "display": "op1"}
def build(net):
    a = net.createNode("add", "ad")
    a.parm("points").set(2)
    for i, p in enumerate([(9, 9, 9), (8, 8, 8)]):
        a.parm("usept%d" % i).set(1)
        a.parmTuple("pt%d" % i).set(p)
    m = mk(net, "merge", "m")
    m.setInput(0, poly_curve(net, "q", Q, closed=True)); m.setInput(1, poly_curve(net, "dg", DG, closed=True)); m.setInput(2, a)
    k = mk(net, "clean", "op1", deldegengeo=0, delunusedpts=1)
    k.setInput(0, m)
    return k
finish(SOP, SCENE, build, NOTE)
