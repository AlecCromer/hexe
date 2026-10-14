"""hython bgeo-legacy-counts.py -- H39: every legacy BgeoV file shipped in the Houdini 20.5.487 install (< 2 MB) loaded by Houdini itself;
writes fixtures/bgeo-legacy/counts.json (points/prims/vertices/P bbox/attribs). Oracle tooling only (AGENTS.md section 2 exception)."""
import json, os
import hou
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
out = {}
for d, _, fs in os.walk(os.path.realpath("/Applications/Houdini/Houdini20.5.487")):
    for f in fs:
        if not f.endswith(".bgeo"):
            continue
        p = os.path.join(d, f)
        if os.path.islink(p) or os.path.getsize(p) >= 2000000 or open(p, "rb").read(5) != b"BgeoV":
            continue
        g = hou.Geometry()
        g.loadFromFile(p)
        bb = g.boundingBox()
        out[f] = {
            "points": len(g.points()), "prims": len(g.prims()), "vertices": g.intrinsicValue("vertexcount"),
            "bbox": [round(c, 4) for c in (bb.minvec()[0], bb.minvec()[1], bb.minvec()[2], bb.maxvec()[0], bb.maxvec()[1], bb.maxvec()[2])],
            "point_attribs": sorted(a.name() for a in g.pointAttribs()), "prim_attribs": sorted(a.name() for a in g.primAttribs()),
            "prim_types": sorted(set(str(x.type()).split(".")[-1] if x is not None else "None" for x in g.prims())),
        }
with open(os.path.join(ROOT, "fixtures", "bgeo-legacy", "counts.json"), "w") as h:
    json.dump(out, h, indent=1, sort_keys=True)
print(json.dumps(out, sort_keys=True))
