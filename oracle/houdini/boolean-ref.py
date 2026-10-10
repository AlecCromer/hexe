"""Reference dump: Houdini boolean::2.0 (solid/solid), box∩sphere + box∩box offset, union/subtract/intersect.
Runs under hython (oracle tooling only). Writes parity/ref/boolean/<case>.geo + .topo (P + prim point lists) + SUMMARY.txt (counts, bbox, area, volume).
"""
import os, sys
import hou
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
OUT = os.path.join(ROOT, "parity", "ref", "boolean")
os.makedirs(OUT, exist_ok=True)
OPS = {"union": "union", "sub": "subtract", "int": "intersect"}
# B source: (kind, params, translate)
BS = {
    "sph": ("sphere", {"type": 2, "radx": 0.65, "rady": 0.65, "radz": 0.65, "rows": 9, "cols": 12}, (0, 0, 0)),
    "off": ("box", {}, (0.5, 0.5, 0.5)),
    "offx": ("box", {}, (0.5, 0, 0)),
}
only = sys.argv[1:]
summ = []
for bn, (kind, parms, t) in BS.items():
    for on, op in OPS.items():
        name = "%s_%s" % (bn, on)
        if only and name not in only:
            continue
        obj = hou.node("/obj")
        for c in obj.children():
            c.destroy()
        net = obj.createNode("geo", "o")
        a = net.createNode("box", "a")
        b = net.createNode(kind, "b")
        for k, v in parms.items():
            b.parm(k).set(v)
        x = net.createNode("xform", "xf"); x.setInput(0, b)
        x.parmTuple("t").set(t)
        o = net.createNode("boolean::2.0", "bool1")
        o.parm("booleanop").set(op)
        if op == "subtract":
            o.parm("subtractchoices").set("aminusb")
        o.parm("asurface").set("solid"); o.parm("bsurface").set("solid")
        o.setInput(0, a); o.setInput(1, x)
        o.cook(force=True)
        if o.errors():
            print(name, "ERR", o.errors()); continue
        g = o.geometry()
        g.saveToFile(os.path.join(OUT, name + ".geo"))
        pr = g.prims()
        area = sum(p.intrinsicValue("measuredarea") for p in pr)
        vol = 0.0
        for p in pr:  # divergence theorem on fan triangles
            vs = [v.point().position() for v in p.vertices()]
            for i in range(1, len(vs) - 1):
                vol += vs[0].dot(vs[i].cross(vs[i + 1])) / 6.0
        bb = g.boundingBox() if g.points() else None
        hist = {}
        for p in pr:
            hist[p.numVertices()] = hist.get(p.numVertices(), 0) + 1
        with open(os.path.join(OUT, name + ".topo"), "w") as f:
            f.write("# %s pts=%d prims=%d\n" % (name, len(g.points()), len(pr)))
            for pt in g.points():
                q = pt.position(); f.write("P %d %.5f %.5f %.5f\n" % (pt.number(), q[0], q[1], q[2]))
            for p in pr:
                f.write("F %d %s\n" % (p.number(), " ".join(str(v.point().number()) for v in p.vertices())))
        line = "%s pts=%d prims=%d nv=%s area=%.5f vol=%.5f bb=%s..%s pgrp=%s ptgrp=%s pattr=%s" % (
            name, len(g.points()), len(pr), dict(sorted(hist.items())), area, vol,
            [round(c, 4) for c in bb.minvec()] if bb else None, [round(c, 4) for c in bb.maxvec()] if bb else None,
            [q.name() for q in g.primGroups()], [q.name() for q in g.pointGroups()], [q.name() for q in g.primAttribs()])
        print(line); summ.append(line)
if not only:
    with open(os.path.join(OUT, "SUMMARY.txt"), "w") as f:
        f.write("\n".join(summ) + "\n")
