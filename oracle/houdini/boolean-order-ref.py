"""Reference dump #2 (H35): Houdini boolean::2.0 solid/solid, box A (unit) vs box B at more offsets -> parity/ref/boolean/<case>.topo (ORDER corpus).
Runs under hython (oracle tooling only). Case name: bx<tx>_<ty>_<tz>_<op> with offsets in quarters (e.g. bx2_2_2 = t(.5,.5,.5)), op union|sub|int|subba."""
import os
import hou
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
OUT = os.path.join(ROOT, "parity", "ref", "boolean")
OFFS = [(2, 2, 2), (-2, -2, -2), (2, -2, 2), (1, 1, 1), (2, 0, 0), (-2, 0, 0), (0, 2, 0), (2, 2, 0), (0, 0, -2), (-2, 2, 0)]
OPS = {"union": ("union", None), "sub": ("subtract", "aminusb"), "int": ("intersect", None), "subba": ("subtract", "bminusa")}
for off in OFFS:
    for on, (op, ch) in OPS.items():
        name = "bx%s_%s" % ("_".join(str(c) for c in off), on)
        obj = hou.node("/obj")
        for c in obj.children():
            c.destroy()
        net = obj.createNode("geo", "o")
        a = net.createNode("box", "a"); b = net.createNode("box", "b")
        x = net.createNode("xform", "xf"); x.setInput(0, b); x.parmTuple("t").set(tuple(c / 4.0 for c in off))
        o = net.createNode("boolean::2.0", "bool1")
        o.parm("booleanop").set(op)
        if ch:
            o.parm("subtractchoices").set(ch)
        o.parm("asurface").set("solid"); o.parm("bsurface").set("solid")
        o.setInput(0, a); o.setInput(1, x)
        o.cook(force=True)
        if o.errors():
            print(name, "ERR", o.errors()); continue
        g = o.geometry()
        with open(os.path.join(OUT, name + ".topo"), "w") as f:
            f.write("# %s pts=%d prims=%d\n" % (name, len(g.points()), len(g.prims())))
            for pt in g.points():
                q = pt.position(); f.write("P %d %.5f %.5f %.5f\n" % (pt.number(), q[0], q[1], q[2]))
            for p in g.prims():
                f.write("F %d %s\n" % (p.number(), " ".join(str(v.point().number()) for v in p.vertices())))
