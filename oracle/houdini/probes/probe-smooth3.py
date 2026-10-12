"""H34 probe 3: smooth::2.0 on jittered meshes, q 1..5, methods 0..2 -> fixtures/probes/smooth3.json."""
import json, os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import hou
from _common import FIX, scratch_net, mk
net = scratch_net()
J = "@P += set(rand(@ptnum*3+1)-0.5, rand(@ptnum*3+2)-0.5, rand(@ptnum*3+3)-0.5)*0.3;"
def jit(name, src):
    w = mk(net, "attribwrangle", "w_" + name, snippet=J); w.parm("class").set(0); w.setInput(0, src); return w
g = mk(net, "grid", "g", rows=5, cols=6, sizex=4, sizey=3)
sph = mk(net, "sphere", "sph", type=1, rows=6, cols=8)
bx = mk(net, "box", "bx", divrate=(3, 3, 3), divs=(4, 4, 4)) if False else mk(net, "box", "bx")
inputs = {"grid": jit("g", g), "sphere": jit("s", sph), "box": jit("b", bx)}
res = []
for iname, node in inputs.items():
    node.cook(force=True)
    geo = node.geometry()
    res.append({"input": iname, "strength": 0, "quality": 0, "method": 0, "cb": 0, "P": [list(p.position()) for p in geo.points()],
                "prims": [[v.point().number() for v in pr.vertices()] for pr in geo.prims()]})
    for cb in (0, 1):
      for meth in (0, 1, 2):
        for st, q in ((1, 1), (1, 2), (1, 3), (1, 4), (1, 5), (0.5, 2), (2, 2), (0.5, 4)):
            if cb and meth: continue
            sm = net.createNode("smooth::2.0", "s"); sm.setInput(0, node)
            sm.parm("strength").set(st); sm.parm("filterquality").set(q); sm.parm("method").set(meth); sm.parm("contrainedboundary").set(cb)
            sm.cook(force=True)
            res.append({"input": iname, "strength": st, "quality": q, "method": meth, "cb": cb, "P": [list(p.position()) for p in sm.geometry().points()]})
            sm.destroy()
os.makedirs(os.path.join(FIX, "probes"), exist_ok=True)
json.dump(res, open(os.path.join(FIX, "probes", "smooth3.json"), "w"))
print("done", len(res))
