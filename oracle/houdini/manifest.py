#!/usr/bin/env python3
"""Append fixtures/MANIFEST.toml rows for fixtures/sop-*/{houdini.geo,houdini.json,scene.json} (idempotent: rewrites only sop-* rows)."""
import glob, hashlib, os, re
ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
mp = os.path.join(ROOT, "fixtures", "MANIFEST.toml")
text = open(mp).read()
blocks = re.split(r"(?m)^(?=\[\[fixture\]\])", text)
head, rows = blocks[0], blocks[1:]
rows = [b for b in rows if not re.search(r'(?m)^case = "sop-', b)]
out = []
for d in sorted(glob.glob(os.path.join(ROOT, "fixtures", "sop-*"))):
    case = os.path.basename(d)
    for f in ("houdini.geo", "houdini.json", "scene.json"):
        p = os.path.join(d, f)
        if not os.path.exists(p):
            continue
        h = hashlib.sha256(open(p, "rb").read()).hexdigest()
        if f == "scene.json":
            by, ver = "authored by oracle/houdini/%s.py (hexe twin of the Houdini canonical case)" % case, "n/a"
        else:
            by, ver = "original headless: hython oracle/houdini/%s.py (hou.Geometry.saveToFile / summarize)" % case, "20.5.487"
        out.append('[[fixture]]\ncase = "%s"\nfile = "%s"\nsha256 = "%s"\nproduced_by = "%s"\noriginal_version = "%s"\ndate = "2026-10-08"\n' % (case, f, h, by, ver))
open(mp, "w").write(head + "".join(rows) + "".join(out))
print(len(out), "rows")
