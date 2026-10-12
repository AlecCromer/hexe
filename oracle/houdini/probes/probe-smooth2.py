"""H34 probe 2: smooth::2.0 menus, quality sweep, non-symmetric meshes."""
import json, os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import hou
from _common import FIX, scratch_net, mk
net = scratch_net()
s = net.createNode("smooth::2.0", "tmp")
for n in ("method", "filterquality", "strength"):
    p = s.parm(n); print(n, p.parmTemplate().type(), p.menuItems() if p.parmTemplate().type() == hou.parmTemplateType.Menu else (p.parmTemplate().minValue(), p.parmTemplate().maxValue()))
print("default", s.parm("method").eval(), s.parm("strength").eval(), s.parm("filterquality").eval())
print(s.parm("method").parmTemplate().menuLabels())
s.destroy()
out = {}
# meshes: perturbed grid, perturbed box-with-more-points (polyextrude not needed: use grid 4x4 + wrangle displacement)
def mesh(name):
    g = mk(net, "grid", "g_" + name, rows=5, cols=6, sizex=4, sizey=3)
    w = mk(net, "attribwrangle", "w_" + name, snippet="@P.y = sin(@P.x*2.1)*0.7 + @P.z*0.3; @P.x += 0.2*cos(@P.z*3.0); @P.z += @ptnum*0.01;")
    w.parm("class").set(0)
    w.setInput(0, g)
    return w
m = mesh("a")
boxsub = mk(net, "sphere", "sph", type=1, rows=6, cols=8)  # polymesh sphere
inputs = {"grid": m, "sphere": boxsub}
res = []
for iname, node in inputs.items():
    node.cook(force=True)
    res.append({"input": iname, "strength": 0, "quality": 0, "method": 0, "P": [list(p.position()) for p in node.geometry().points()],
                "prims": [[v.point().number() for v in pr.vertices()] for pr in node.geometry().prims()]})
    for meth in (0, 1, 2):
        for st, q in ((1, 1), (0.5, 2), (1, 2), (1, 3), (1, 4), (2, 3), (1, 6), (1, 8), (1, 10)):
            sm = net.createNode("smooth::2.0", "s"); sm.setInput(0, node)
            sm.parm("strength").set(st); sm.parm("filterquality").set(q)
            try: sm.parm("method").set(meth)
            except Exception as e: print("method err", e); sm.destroy(); continue
            sm.cook(force=True)
            res.append({"input": iname, "strength": st, "quality": q, "method": meth, "P": [list(p.position()) for p in sm.geometry().points()]})
            sm.destroy()
# box quality sweep
b = mk(net, "box", "b")
for meth in (0, 1, 2):
    for q in range(1, 13):
        sm = net.createNode("smooth::2.0", "s"); sm.setInput(0, b)
        sm.parm("strength").set(1); sm.parm("filterquality").set(q)
        try: sm.parm("method").set(meth)
        except Exception: sm.destroy(); continue
        sm.cook(force=True)
        print("box meth", meth, "q", q, sm.geometry().points()[0].position()[0])
        sm.destroy()
os.makedirs(os.path.join(FIX, "probes"), exist_ok=True)
json.dump(res, open(os.path.join(FIX, "probes", "smooth2.json"), "w"))
