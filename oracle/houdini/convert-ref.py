"""Reference dump: Houdini convert SOP (sphere/tube prim -> poly, poly -> polysoup, nurbs -> poly w/ lod).
Runs under hython (oracle tooling only). Writes parity/ref/convert/<case>.geo + <case>.topo (P + prim point lists).
"""
import os, sys
import hou
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
OUT = os.path.join(ROOT, "parity", "ref", "convert")
os.makedirs(OUT, exist_ok=True)

def src_sphere(net, rad=(1, 1, 1), t=(0, 0, 0)):
    s = net.createNode("sphere", "src"); s.parm("type").set(0)  # primitive
    s.parmTuple("rad").set(rad); s.parmTuple("t").set(t); return s
def src_tube(net, cap=1):
    s = net.createNode("tube", "src"); s.parm("type").set(0); s.parm("cap").set(cap); return s
def src_box(net):
    return net.createNode("box", "src")
def src_circle(net):
    c = net.createNode("circle", "src"); c.parm("type").set(2); return c  # nurbs
def src_nsphere(net):
    s = net.createNode("sphere", "src"); s.parm("type").set(3); return s  # nurbs
# name: (source builder, convert parms)
CASES = {
    "sphere_lod05": (src_sphere, {"fromtype": "sphere", "totype": "poly", "lodu": 0.5, "lodv": 0.5}),
    "sphere_lod075": (src_sphere, {"fromtype": "sphere", "totype": "poly", "lodu": 0.75, "lodv": 0.75}),
    "sphere_lod1": (src_sphere, {"fromtype": "sphere", "totype": "poly", "lodu": 1, "lodv": 1}),
    "sphere_lod2": (src_sphere, {"fromtype": "sphere", "totype": "poly", "lodu": 2, "lodv": 2}),
    "sphere_xf_lod05": (lambda n: src_sphere(n, (2, 3, 4), (1, 2, 3)), {"fromtype": "sphere", "totype": "poly", "lodu": 0.5, "lodv": 0.5}),
    "tube_lod1": (src_tube, {"fromtype": "tube", "totype": "poly", "lodu": 1, "lodv": 1}),
    "box_polysoup": (src_box, {"fromtype": "poly", "totype": "polySoup"}),
    "circle_nurbs_lod1": (src_circle, {"fromtype": "nurbCurve", "totype": "poly", "lodu": 1, "lodv": 1}),
    "circle_nurbs_lod3": (src_circle, {"fromtype": "nurbCurve", "totype": "poly", "divu": 3, "divv": 3}),
    "nsphere_nurbs_lod1": (src_nsphere, {"fromtype": "nurbSurf", "totype": "poly", "lodu": 1, "lodv": 1}),
}
only = sys.argv[1:]
for name, (mk, parms) in CASES.items():
    if only and name not in only:
        continue
    obj = hou.node("/obj")
    for c in obj.children():
        c.destroy()
    net = obj.createNode("geo", "o")
    s = mk(net)
    cv = net.createNode("convert", "conv1"); cv.setInput(0, s)
    for k, v in parms.items():
        p = cv.parm(k)
        if p is None:
            print(name, "NOPARM", k); continue
        if isinstance(v, str):
            try: p.set(v)
            except Exception as e: print(name, "BADMENU", k, v, [x for x in p.menuItems()])
        else: p.set(v)
    cv.cook(force=True)
    if cv.errors():
        print(name, "ERR", cv.errors()); continue
    g = cv.geometry()
    g.saveToFile(os.path.join(OUT, name + ".geo"))
    with open(os.path.join(OUT, name + ".topo"), "w") as f:
        f.write("# %s %s pts=%d prims=%d\n" % (name, parms, len(g.points()), len(g.prims())))
        for pt in g.points():
            q = pt.position(); f.write("P %d %.5f %.5f %.5f\n" % (pt.number(), q[0], q[1], q[2]))
        for pr in g.prims():
            f.write("F %d %s %s\n" % (pr.number(), str(pr.type()).split(".")[-1], " ".join(str(v.point().number()) for v in pr.vertices())))
    print(name, len(g.points()), len(g.prims()), sorted(set(str(pr.type()).split(".")[-1] for pr in g.prims())))
