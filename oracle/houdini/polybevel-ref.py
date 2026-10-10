"""Reference dump: Houdini polybevel::3.0 on unit box, many (divisions, offset, filletshape) cases.
Runs under hython (oracle tooling only). Writes parity/ref/polybevel/<case>.geo + <case>.topo (plain text P + prim point lists).
"""
import os, sys
import hou
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
OUT = os.path.join(ROOT, "parity", "ref", "polybevel")
os.makedirs(OUT, exist_ok=True)
# name: (divisions, offset, filletshape[houdini enum], extra)
CASES = {
    "d1_o10": (1, 0.1, 4), "d2_o10": (2, 0.1, 4), "d3_o10": (3, 0.1, 4), "d4_o10": (4, 0.1, 4),
    "d2_o20": (2, 0.2, 4), "d2_o05": (2, 0.05, 4), "d3_o25": (3, 0.25, 4),
    "d5_o10": (5, 0.1, 4), "d6_o10": (6, 0.1, 4), "d7_o10": (7, 0.1, 4), "d8_o10": (8, 0.1, 4), "d9_o10": (9, 0.1, 4), "d10_o10": (10, 0.1, 4), "d11_o10": (11, 0.1, 4), "d12_o10": (12, 0.1, 4), "d13_o10": (13, 0.1, 4), "d16_o10": (16, 0.1, 4), "d17_o10": (17, 0.1, 4), "d2_solid": (2, 0.1, 1), "d2_o40": (2, 0.4, 4),
}
only = sys.argv[1:]
for name, (div, off, shape) in CASES.items():
    if only and name not in only:
        continue
    obj = hou.node("/obj")
    for c in obj.children():
        c.destroy()
    net = obj.createNode("geo", "o")
    b = net.createNode("box", "box1")
    p = net.createNode("polybevel::3.0", "bevel1")
    p.setInput(0, b)
    p.parm("offset").set(off); p.parm("divisions").set(div); p.parm("filletshape").set(shape)
    p.cook(force=True)
    if p.errors():
        print(name, "ERR", p.errors()); continue
    g = p.geometry()
    g.saveToFile(os.path.join(OUT, name + ".geo"))
    with open(os.path.join(OUT, name + ".topo"), "w") as f:
        f.write("# %s div=%d off=%g shape=%d pts=%d prims=%d\n" % (name, div, off, shape, len(g.points()), len(g.prims())))
        for pt in g.points():
            q = pt.position(); f.write("P %d %.5f %.5f %.5f\n" % (pt.number(), q[0], q[1], q[2]))
        for pr in g.prims():
            f.write("F %d %s\n" % (pr.number(), " ".join(str(v.point().number()) for v in pr.vertices())))
    print(name, len(g.points()), len(g.prims()), [pr.numVertices() for pr in g.prims()].count(4), "quads")
