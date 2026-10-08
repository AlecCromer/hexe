#!/usr/bin/env python3
"""report.py -> docs/HOUDINI-ORACLE.md from results.json (+ fixtures/*/houdini.json notes) and the CAUSE table below (human, observed from the P dumps)."""
import json, os
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
BOX = "box point/prim order: Houdini = bottom ring (y-) CCW then top ring, prim0=[1,5,4,0]; hexe = x-major (0,1,3,2). Geometry identical as a set"
GRIDZ = "grid row axis: Houdini row0 at z=-size/2 (ascending +Z); hexe row0 at +Z (descending). Same quad index pattern, mirrored in Z"
CAUSE = {
    "sop-box": BOX,
    "sop-grid": GRIDZ,
    "sop-sphere": "polymesh order: Houdini poles are points 0 (top),1 (bottom) then rings, ring runs toward -Z from +X; hexe rings first, poles last (264,265), ring runs toward +Z. Same 266/288 counts, same point set",
    "sop-line": "-",
    "sop-circle": "Houdini default reverse=1: XY circle runs CLOCKWISE (pt1 = (1,-0.87)); hexe runs CCW. Same point set, one prim",
    "sop-curve": "- (Houdini 20.5 curve::2.0 has no coords parm; twin = Add SOP closed polygon, so this checks the data model not the Curve SOP itself)",
    "sop-add": "-",
    "sop-transform": "box point order only; transformed P multiset equals Houdini (SRT math verified, t/r/s)",
    "sop-merge": "box point order only (box + grid: grid part also Z-mirrored); counts 17/10/40 match",
    "sop-scatter": "RNG stream differs (expected, ledger says UNVERIFIED); hexe also writes point attrs N + sourceprim that Houdini scatter does not by default. Count 500 matches",
    "sop-copytopoints": "box point order; hexe writes copynum (point+prim) when param on, Houdini copytopoints::2.0 writes none (needs idattrib/ attribute transfer). Counts 72/54/216 match",
    "sop-polybevel": "hexe 48/38 vs Houdini 56/54 (offset .1 divs 2 round): hexe bevels fewer elements / emits fewer fillet faces than Houdini polybevel::3.0",
    "sop-polyextrude": "Houdini default splittype=connected components: closed box = 1 component, no open edges -> NO new faces (8/6/24, box scales to +-1); hexe extrudes each face individually (48/30/120). Spec/param mismatch (hexe lacks splittype), not a bug in per-face math",
    "sop-subdivide": "box order; Catmull-Clark counts 26/24/96 match Houdini osdcc",
    "sop-smooth": "different model: Houdini smooth::2.0 (strength 0.5, quality 2, uniform) shrinks box to +-0.4286; hexe Laplacian x2 shrinks to +-0.2222",
    "sop-peak": "box point order only; displaced P multiset equals Houdini",
    "sop-facet": "box order (unique_points -> 24 pts both; post N present both)",
    "sop-resample": "inherits Houdini circle CW direction (y-mirrored sample sequence); sample point SET equals hexe (24 pts)",
    "sop-wrangle": "inherits grid Z mirror (row0 z=-5 vs +5); sin() math matches",
    "sop-noise": "noise basis/seed not Houdini's perlin (ledger: UNVERIFIED); Houdini displacement bbox y[0.39,0.77] vs hexe y[-0.28,0.30]; hexe also lacks zero-centred vs positive range semantics (Houdini noiserange default positive)",
    "sop-normal": "box order only; vertex N attribute present in both",
    "sop-fuse": "input polyextrude differs (see sop-polyextrude): hexe feeds 48 pts, Houdini 8",
    "sop-boolean": "box order; hexe emits prim groups a_only/b_only/seam unconditionally, Houdini only when output toggles on; counts 14/9/42 match",
    "sop-reverse": "box point order only; winding of prims not compared in ORDER-DIFF rows",
    "sop-registry": "null passthrough of box: box point order only",
    "sop-group": "box order; prim group 'top' present both",
    "sop-groupdelete": "box order",
    "sop-grouppromote": "box order; point group toppts both",
    "sop-attribpromote": "grid Z mirror (point order); prim attr v present both",
    "sop-attribtransfer": "grid Z mirror (point order)",
    "sop-attribcopy": "grid Z mirror (point order)",
    "sop-attribdelete": "grid Z mirror (point order)",
    "sop-blast": "box order",
    "sop-file": "-",
    "sop-sort": "grid Z mirror (point order); reverse permutation itself consistent (prim_pts identical)",
    "sop-delete": "grid Z mirror; 9 pts kept, 2 prims left both",
    "sop-foreach": "-  (piece order a,b,c merge gives P x=0,3,1,4,2,5 in both)",
    "sop-uvproject": "box order; vertex uv present both (uv VALUES not compared)",
    "sop-uvunwrap": "box order; uv present both (values not compared; algorithms differ by design)",
    "sop-uvlayout": "box order; Houdini adds detail attrs coverage + numnonpacked; uv values not compared",
}
R = json.load(open(os.path.join(HERE, "results.json")))
hist = {}
for r in R:
    hist[r["verdict"]] = hist.get(r["verdict"], 0) + 1
L = []
L.append("# HOUDINI-ORACLE (H30)\n")
L.append("Real Houdini 20.5.487 (hython, Commercial) twin of every `sop-*` ledger row vs `hexe-cli cook` (prebuilt trunk debug binary). Scripts: `oracle/houdini/sop-*.py` (one per row; build the canonical case, save `fixtures/sop-*/houdini.geo` + `houdini.json`, write the hexe twin `scene.json`). Comparator: `oracle/houdini/compare.py`. Table: `oracle/houdini/report.py`.\n")
L.append("Reproduce: `hython oracle/houdini/run_all.py` (all 40 in one process, ~10 s) then `python3 oracle/houdini/compare.py` then `python3 oracle/houdini/report.py`.\n")
L.append("Rules: counts = points/prims/vertices. Attribs = per-class name sets + point/prim groups. P compared in index order, tol 1e-4. **Permutation is only classified, never passed**: ORDER-DIFF = same P multiset, different order. Hexe `.geo` is loaded back INTO Houdini for the summary (so a loadable file is checked too). Verdict priority: HOUDINI-ERR > HEXE-ERR > COUNT-DIFF > ATTRIB-DIFF > TOPO-DIFF > ORDER-DIFF > P-DIFF > MATCH (P-DIFF = point multisets differ; TOPO-DIFF = same P but prim point lists / closedness differ).\n")
L.append("Histogram: " + ", ".join("%s %d" % (k, v) for k, v in sorted(hist.items(), key=lambda kv: -kv[1])) + "  (N=%d)\n" % len(R))
L.append("| sop | houdini pts/prims/vtx | hexe pts/prims/vtx | P | verdict | cause (one line) |")
L.append("|---|---|---|---|---|---|")
for r in R:
    h = "/".join(map(str, r["houdini"])) if "houdini" in r else "-"
    x = "/".join(map(str, r["hexe"])) if "hexe" in r else "-"
    fl = "+".join(r.get("flags") or [])
    v = r["verdict"] + ((" (+" + fl.replace(r["verdict"], "").strip("+") + ")") if fl and fl != r["verdict"] and fl.replace(r["verdict"], "").strip("+") else "")
    c = CAUSE.get(r["sop"], r.get("detail", ""))
    L.append("| %s | %s | %s | %s | %s | %s |" % (r["sop"], h, x, r.get("p", "-"), v, c.replace("|", "/")))
L.append("\n## Not covered / caveats\n")
L.append("- `sop-curve`: Houdini 20.5 has no scriptable coords parm on `curve::2.0`; the twin is an Add SOP closed polygon through the same 3 points.\n- ORDER-DIFF rows mask value comparison of derived attributes (N, uv, etc.): only names/counts/P-multiset are checked there. Next wave: fix generator order (box, grid, sphere, circle), then re-run to unmask downstream SOPs.\n- `sop-uvproject` axis mapping (hexe axis y -> Houdini inittype zx) and `sop-smooth`/`sop-noise` parameter mapping are approximations (different parm models).\n- `sop-registry` = null passthrough only; `rop-geometry` is not a `sop-*` row and is out of scope.\n- No features.toml edits; no hexe code touched.\n")
open(os.path.join(ROOT, "docs", "HOUDINI-ORACLE.md"), "w").write("\n".join(L))
print(hist)
