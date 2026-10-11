"""hython oracle/houdini/check-name-values.py <hexe-out-dir>  -- compares the STRING VALUES of the `name` SOP twins
(compare.py checks attribute names only): fixtures/sop-name*/houdini.geo vs <dir>/sop-name*.geo (`hexe-cli cook ... --out`).
Oracle tooling only; exit 1 on any difference."""
import os, sys
import hou
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
WORK = sys.argv[1]
def vals(path, cls, attr):
    g = hou.Geometry()
    g.loadFromFile(path)
    els = g.prims() if cls == "prim" else g.points()
    return [e.attribValue(attr) for e in els]
ok = True
for case, cls, attr in (("sop-name", "prim", "name"), ("sop-name-point", "point", "tag")):
    h = vals(os.path.join(ROOT, "fixtures", case, "houdini.geo"), cls, attr)
    x = vals(os.path.join(WORK, case + ".geo"), cls, attr)
    print(case, h, x, "EQUAL" if h == x else "DIFF")
    ok = ok and h == x
print("ALL EQUAL" if ok else "MISMATCH")
sys.exit(0 if ok else 1)
