"""H34 probe 4: recover the smooth::2.0 linear operator: many random P fields on one mesh, q 1..5 -> fixtures/probes/smooth4.json."""
import json, os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import hou
from _common import FIX, scratch_net, mk
net = scratch_net()
bx = mk(net, "box", "bx", divrate=(3, 3, 3), divs=(3, 3, 3), type=1)
w = mk(net, "attribwrangle", "w", snippet="@P = set(rand(@ptnum*3+1+chi('seed')*1000), rand(@ptnum*3+2+chi('seed')*1000), rand(@ptnum*3+3+chi('seed')*1000));")
w.parm("class").set(0)
w.setInput(0, bx)
t = w.parmTemplateGroup()
t.append(hou.IntParmTemplate("seed", "seed", 1)); w.setParmTemplateGroup(t)
w.cook(force=True)
g0 = w.geometry()
prims = [[v.point().number() for v in pr.vertices()] for pr in g0.prims()]
print("pts", len(g0.points()), "prims", len(prims))
res = {"prims": prims, "runs": []}
for seed in range(24):
    w.parm("seed").set(seed)
    w.cook(force=True)
    P0 = [list(p.position()) for p in w.geometry().points()]
    for q in (1, 2, 3, 4, 5):
        sm = net.createNode("smooth::2.0", "s"); sm.setInput(0, w)
        sm.parm("strength").set(1); sm.parm("filterquality").set(q); sm.parm("contrainedboundary").set(0)
        sm.cook(force=True)
        res["runs"].append({"seed": seed, "quality": q, "P0": P0, "P1": [list(p.position()) for p in sm.geometry().points()]})
        sm.destroy()
os.makedirs(os.path.join(FIX, "probes"), exist_ok=True)
json.dump(res, open(os.path.join(FIX, "probes", "smooth4.json"), "w"))
print("done")
