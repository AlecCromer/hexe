"""H35b probe (oracle tooling, hython): does the START VERTEX of a split boolean::2.0 poly depend only on topology? Sweep B = box t(s,s,s) vs A = unit box,
subtract A-B, signature per split poly = kept-corner slots (K<i> = source face cycle slot) / new points (N) in Houdini cycle order, start = first token.
Output parity/ref/boolean/start-sweep.txt: signature changes along s (grid 0.005) + bisected thresholds. Run: hython oracle/houdini/probes/probe-boolean-start-sweep.py"""
import os
import hou
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
S = 10000
BOXP = [(-.5, -.5, -.5), (.5, -.5, -.5), (.5, -.5, .5), (-.5, -.5, .5), (-.5, .5, -.5), (.5, .5, -.5), (.5, .5, .5), (-.5, .5, .5)]
FACES = [[1, 5, 4, 0], [2, 6, 5, 1], [3, 7, 6, 2], [0, 4, 7, 3], [2, 1, 0, 3], [5, 6, 7, 4]]
FN = ["-z", "+x", "+z", "-x", "-y", "+y"]


def q(p, off=(0, 0, 0)):
    return tuple(int(round((c + o) * S)) for c, o in zip(p, off))


def cook(s):
    obj = hou.node("/obj")
    for c in obj.children():
        c.destroy()
    net = obj.createNode("geo", "o")
    a = net.createNode("box", "a"); b = net.createNode("box", "b")
    x = net.createNode("xform", "xf"); x.setInput(0, b); x.parmTuple("t").set((s, s, s))
    o = net.createNode("boolean::2.0", "bool1")
    o.parm("booleanop").set("subtract"); o.parm("subtractchoices").set("aminusb")
    o.parm("asurface").set("solid"); o.parm("bsurface").set("solid")
    o.setInput(0, a); o.setInput(1, x)
    o.cook(force=True)
    g = o.geometry()
    return [q(p.position()) for p in g.points()], [[v.point().number() for v in pr.vertices()] for pr in g.prims()]


def sig(s):
    P, F = cook(s)
    boxes = {"A": [q(p) for p in BOXP], "B": [q(p, (s, s, s)) for p in BOXP]}
    out = {}
    for f in F:
        pos = [P[i] for i in f]
        for nm, pts in boxes.items():
            for k, fc in enumerate(FACES):
                cyc = [pts[i] for i in fc]
                if any(len({c[ax] for c in cyc}) == 1 and all(p[ax] == cyc[0][ax] for p in pos) for ax in range(3)):
                    ks = [cyc.index(p) if p in cyc else None for p in pos]
                    if len(pos) == 4 and None not in ks:
                        continue
                    out[nm + FN[k]] = " ".join("K%d" % i if i is not None else "N" for i in ks)
    return tuple(sorted(out.items()))


lines = ["# A=unit box, B=box t(s,s,s), subtract aminusb, solid/solid. split-poly signature: start = first token; K<i> = kept corner (slot i of the source face cycle), N = new point"]
last = None; prev_s = None
for i in range(2, 199):
    s = round(i * 0.005, 4)
    cur = sig(s)
    if last is not None and cur != last:
        changed = [k for (k, v) in cur if dict(last).get(k) != v]
        lo, hi = prev_s, s
        for _ in range(20):
            m = (lo + hi) / 2
            if dict(sig(m)).get(changed[0]) == dict(last).get(changed[0]):
                lo = m
            else:
                hi = m
        lines.append("threshold s~%.5f  changed: %s" % (hi, ", ".join("%s %s -> %s" % (k, dict(last).get(k), dict(cur).get(k)) for k in changed)))
    if last is None or cur != last:
        lines.append("s=%.3f  %s" % (s, " | ".join("%s: %s" % kv for kv in cur)))
    last = cur; prev_s = s
with open(os.path.join(ROOT, "parity", "ref", "boolean", "start-sweep.txt"), "w") as f:
    f.write("\n".join(lines) + "\n")
print("\n".join(lines))
