from _common import *
SOP = "sop-clean"
NOTE = "clean defaults on: quad, collinear tri, plain tri, thin tri (area .00025 kept), tiny tri (area < tol^2 deleted), open 0.0005 line... plus an unused point"
Q = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
DG = [(3, 0, 0), (4, 0, 0), (5, 0, 0)]
TRI = [(3, 2, 0), (4, 2, 0), (3, 3, 0)]
T = [(6, 0, 0), (7, 0, 0), (6.5, 0.0005, 0)]
S = [(6, 3, 0), (6.0005, 3, 0), (6, 3.001, 0)]
LN = [(8, 0, 0), (9, 0, 0)]
LS = [(8, 2, 0), (8.0005, 2, 0)]
SCENE = {"nodes": [{"name": "q", "type": "curve", "params": {"coords": coords(Q), "close": True}}, {"name": "dg", "type": "curve", "params": {"coords": coords(DG), "close": True}}, {"name": "tri", "type": "curve", "params": {"coords": coords(TRI), "close": True}}, {"name": "t", "type": "curve", "params": {"coords": coords(T), "close": True}}, {"name": "s", "type": "curve", "params": {"coords": coords(S), "close": True}}, {"name": "ln", "type": "curve", "params": {"coords": coords(LN), "close": False}}, {"name": "ls", "type": "curve", "params": {"coords": coords(LS), "close": False}}, {"name": "ad", "type": "add", "params": {"points": "9,9,9"}}, {"name": "m", "type": "merge"}, {"name": "m2", "type": "merge"}, {"name": "m3", "type": "merge"}, {"name": "op1", "type": "clean", "params": {}}], "edges": [["q", 0, "m", 0], ["dg", 0, "m", 1], ["tri", 0, "m", 2], ["t", 0, "m", 3], ["m", 0, "m2", 0], ["s", 0, "m2", 1], ["ln", 0, "m2", 2], ["ls", 0, "m2", 3], ["m2", 0, "m3", 0], ["ad", 0, "m3", 1], ["m3", 0, "op1", 0]], "display": "op1"}
def add_pts(net, name, pts):
    a = net.createNode("add", name)
    a.parm("points").set(len(pts))
    for i, p in enumerate(pts):
        a.parm("usept%d" % i).set(1)
        a.parmTuple("pt%d" % i).set(p)
    return a
def build(net):
    ins = [poly_curve(net, "q", Q, closed=True), poly_curve(net, "dg", DG, closed=True), poly_curve(net, "tri", TRI, closed=True), poly_curve(net, "t", T, closed=True), poly_curve(net, "s", S, closed=True), poly_curve(net, "ln", LN), poly_curve(net, "ls", LS), add_pts(net, "ad", [(9, 9, 9)])]
    m = mk(net, "merge", "m")
    for i, n in enumerate(ins[:4]):
        m.setInput(i, n)
    m2 = mk(net, "merge", "m2")
    m2.setInput(0, m)
    for i, n in enumerate(ins[4:7]):
        m2.setInput(i + 1, n)
    m3 = mk(net, "merge", "m3")
    m3.setInput(0, m2); m3.setInput(1, ins[7])
    m = m3
    k = mk(net, "clean", "op1")
    k.setInput(0, m)
    return k
finish(SOP, SCENE, build, NOTE)
