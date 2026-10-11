from _common import *
SOP = "sop-clean-small"
NOTE = "clean make_manifold + delete_small prim_count 1: the 2-quad strip survives (2 > 1), the lone triangle goes"
Q1 = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
Q3 = [(1, 0, 0), (2, 0, 0), (2, 1, 0), (1, 1, 0)]
TR = [(5, 0, 0), (6, 0, 0), (5, 1, 0)]
QS = [("q1", Q1), ("q3", Q3), ("tr", TR)]
SCENE = {"nodes": [{"name": n, "type": "curve", "params": {"coords": coords(q), "close": True}} for n, q in QS] + [{"name": "m", "type": "merge"}, {"name": "f", "type": "fuse", "params": {"distance": 0.001}}, {"name": "op1", "type": "clean", "params": {"make_manifold": True, "delete_small": True, "prim_count": 1}}], "edges": [[n, 0, "m", i] for i, (n, _) in enumerate(QS)] + [["m", 0, "f", 0], ["f", 0, "op1", 0]], "display": "op1"}
def build(net):
    m = mk(net, "merge", "m")
    for i, (n, q) in enumerate(QS):
        m.setInput(i, poly_curve(net, n, q, closed=True))
    f = mk(net, "fuse::2.0", "f", tol3d=0.001)
    f.setInput(0, m)
    k = mk(net, "clean", "op1", make_manifold=1, delete_small=1, prim_count=1)
    k.setInput(0, f)
    return k
finish(SOP, SCENE, build, NOTE)
