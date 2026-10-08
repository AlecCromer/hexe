"""H34 probe: smooth::2.0 grid. hython probe-smooth.py -> fixtures/probes/smooth.json (oracle input only)."""
import json, os, sys, itertools
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import hou
from _common import FIX, scratch_net, mk
net = scratch_net()
b = mk(net, "box", "box1")
out = []
parms = [p.name() for p in net.createNode("smooth::2.0", "tmp").parms()]
print("PARMS", parms)
for st, q, meth in itertools.product([0.25, 0.5, 1, 2], [1, 2, 4], ["uniform", "scale"]):
    s = net.createNode("smooth::2.0", "s")
    s.setInput(0, b)
    s.parm("strength").set(st); s.parm("filterquality").set(q)
    for m in ("method", "smoothtype", "weighting"):
        if s.parm(m) is not None and s.parm(m).menuItems():
            items = s.parm(m).menuItems()
            if meth == "uniform": s.parm(m).set(0)
            else:
                s.parm(m).set(1 if len(items) > 1 else 0)
    s.cook(force=True)
    g = s.geometry()
    out.append({"strength": st, "quality": q, "method": meth, "P": [list(p.position()) for p in g.points()]})
    s.destroy()
os.makedirs(os.path.join(FIX, "probes"), exist_ok=True)
json.dump(out, open(os.path.join(FIX, "probes", "smooth.json"), "w"), indent=1)
for r in out: print(r["strength"], r["quality"], r["method"], r["P"][0])
