#!/usr/bin/env python3
"""report.py -> docs/HOUDINI-ORACLE.md from results.json (+ fixtures/*/houdini.json notes) and the CAUSE table below (human, observed from the P dumps)."""
import json, os
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
WIND = "winding only: counts, P index order, attribs equal; each prim runs the other way round (hexe CCW / right-handed everywhere, Houdini CW / left-hand normal), same first corner"
CAUSE = {
    "sop-add": "-",
    "sop-bend": "- (Houdini bend toward -X by default, twin sets hexe side=(-1,0,0); parm map is a best guess)",
    "sop-circle": "- (default reverse=1 now ON in hexe: clockwise; one prim, identical point cycle)",
    "sop-curve": "- (Houdini 20.5 curve::2.0 has no coords parm; twin = Add SOP closed polygon, so this checks the data model not the Curve SOP itself)",
    "sop-file": "-",
    "sop-foreach": "-  (piece order a,b,c merge gives P x=0,3,1,4,2,5 in both)",
    "sop-line": "-",
    "sop-resample": "- (inherits the circle order; sample points equal)",
    "sop-wrangle": "- (grid order now equal; sin() math matches; point-only geometry so no winding)",
    "sop-box": WIND + ". Houdini prim0=[1,5,4,0], hexe [1,0,4,5]",
    "sop-grid": WIND,
    "sop-sphere": WIND + " (poles 0/1, ring +X toward -Z now equal)",
    "sop-transform": WIND + "; SRT math verified (t/r/s)",
    "sop-merge": WIND,
    "sop-registry": WIND + " (null passthrough of box)",
    "sop-switch": WIND + " (box branch)",
    "sop-output": WIND,
    "sop-blast": WIND,
    "sop-delete": WIND + "; 9 pts kept, 2 prims left both",
    "sop-sort": WIND + "; reverse permutation itself consistent",
    "sop-group": WIND + "; prim group 'top' present both",
    "sop-groupdelete": WIND,
    "sop-grouppromote": WIND + "; point group toppts both",
    "sop-attribpromote": WIND + "; prim attr v present both",
    "sop-attribtransfer": WIND,
    "sop-attribcopy": WIND,
    "sop-attribdelete": WIND,
    "sop-normal": WIND + "; vertex N present both; N VALUES not compared",
    "sop-peak": WIND + "; displaced P equal Houdini index-for-index (outward)",
    "sop-connectivity": WIND + "; prim attr class present both (values not compared)",
    "sop-measure": WIND + "; prim attr area present both (values not compared)",
    "sop-matchsize": WIND + "; explicit target size/center, P equal",
    "sop-ray": WIND + "; projected P equal Houdini index-for-index",
    "sop-uvproject": WIND + "; vertex uv present both (uv VALUES not compared)",
    "sop-uvunwrap": WIND + "; uv present both (values not compared; algorithms differ by design)",
    "sop-uvlayout": WIND + "; Houdini adds detail attrs coverage + numnonpacked; uv values not compared",
    "sop-copytopoints": WIND + "; hexe writes copynum (point+prim) when param on, Houdini copytopoints::2.0 writes none",
    "sop-boolean": "hexe emits prim groups a_only/b_only/seam unconditionally, Houdini only when output toggles on; point/prim ORDER of the result still differs (counts 14/9/42 match)",
    "sop-reverse": "Houdini reverse keeps the first vertex: [1,5,4,0] -> [1,0,4,5]; hexe reverses the whole list: [1,0,4,5] -> [5,4,0,1]. Same flip, different cycle start",
    "sop-divide": "triangulation order/diagonal: Houdini [1,5,4],[2,6,5],[3,7,6],[0,4,7].. (one tri per quad first); hexe [1,0,4],[1,4,5],[2,1,5].. (fan per quad). Counts 8/12/36 equal, P equal",
    "sop-clip": "cut-point order + prim order: Houdini appends the cut points (+x,-z) first, prims start [1,2,3,0]; hexe cut points (-x,-z) first. Point SET equal, counts 8/5/20 equal; no cap (dofill 0 vs capclip DEFERRED)",
    "sop-knife": "same cut-point/prim order difference as clip (shared core); 12/10/40 equal, groups blanked in both",
    "sop-twist": "direction: Houdini twist strength +90 turns corner (-x,-z) to (+x,-z) (clockwise seen from +Y), hexe angle +90 is right-handed. Sign of the parm map is a best guess (UNVERIFIED which side is 'wrong')",
    "sop-facet": "unique_points: Houdini keeps the original 8 ids on prim 0 and appends the other corners ([1,5,4,0],[2,6,12,20]..); hexe renumbers 4 fresh points per prim",
    "sop-subdivide": "Catmull-Clark counts 26/24/96 match Houdini osdcc; new face/edge/vertex point ORDER differs",
    "sop-scatter": "RNG stream differs (expected, ledger says UNVERIFIED); hexe also writes point attrs N + sourceprim that Houdini scatter does not by default. Count 500 matches",
    "sop-mountain": "noise stream: hexe perlin(seed) vs Houdini perlin basis (no seed twin); Houdini y[-0.15,0.12] vs hexe different. Counts equal",
    "sop-noise": "noise basis/seed not Houdini's perlin (ledger: UNVERIFIED); Houdini displacement bbox y[0.39,0.77] vs hexe y[-0.28,0.30]; Houdini noiserange default positive",
    "sop-smooth": "different model: Houdini smooth::2.0 (strength 0.5, quality 2, uniform) shrinks box to +-0.4286; hexe Laplacian x2 shrinks to +-0.2222",
    "sop-fuse": "input polyextrude differs (see sop-polyextrude): hexe feeds 48 pts, Houdini 8",
    "sop-polybevel": "hexe 48/38 vs Houdini 56/54 (offset .1 divs 2 round): hexe bevels fewer elements / emits fewer fillet faces than Houdini polybevel::3.0",
    "sop-polyextrude": "Houdini default splittype=connected components: closed box = 1 component, no open edges -> NO new faces (8/6/24); hexe extrudes each face individually (48/30/120). Spec/param mismatch (hexe lacks splittype)",
}
RALL = json.load(open(os.path.join(HERE, "results.json")))
R = [r for r in RALL if r["sop"].startswith("sop-")]
XR = [r for r in RALL if not r["sop"].startswith("sop-")]
hist = {}
for r in R:
    hist[r["verdict"]] = hist.get(r["verdict"], 0) + 1
L = []
L.append("# HOUDINI-ORACLE (H30 harness, H31 re-run)\n")
L.append("Real Houdini 20.5.487 (hython, Commercial) twin of every `sop-*` ledger row vs `hexe-cli cook` (debug build of this tree). Scripts: `oracle/houdini/sop-*.py` (one per row; build the canonical case, save `fixtures/sop-*/houdini.geo` + `houdini.json`, write the hexe twin `scene.json`). Comparator: `oracle/houdini/compare.py`. Table: `oracle/houdini/report.py`.\n")
L.append("Reproduce: `hython oracle/houdini/run_all.py` (re-records every Houdini side in one process, ~20 s) then `cargo xtask oracle --houdini` (= build hexe-cli, `compare.py`, `report.py`).\n")
L.append("Rules: counts = points/prims/vertices. Attribs = per-class name sets + point/prim groups. P compared in index order, tol 1e-4. **Permutation is only classified, never passed**: ORDER-DIFF = same P multiset, different order. Hexe `.geo` is loaded back INTO Houdini for the summary (so a loadable file is checked too). Verdict priority: HOUDINI-ERR > HEXE-ERR > COUNT-DIFF > ATTRIB-DIFF > TOPO-DIFF > WINDING-DIFF > ORDER-DIFF > P-DIFF > MATCH (P-DIFF = point multisets differ; TOPO-DIFF = same P order but prim point lists / closedness differ; WINDING-DIFF = same P order and every differing prim is the same cycle with the same first point run the other way).\n")
L.append("Histogram: " + ", ".join("%s %d" % (k, v) for k, v in sorted(hist.items(), key=lambda kv: -kv[1])) + "  (N=%d)\n" % len(R))
L.append("| sop | houdini pts/prims/vtx | hexe pts/prims/vtx | P | verdict | cause guess (one line) |")
L.append("|---|---|---|---|---|---|")
for r in R:
    h = "/".join(map(str, r["houdini"])) if "houdini" in r else "-"
    x = "/".join(map(str, r["hexe"])) if "hexe" in r else "-"
    fl = "+".join(r.get("flags") or [])
    v = r["verdict"] + ((" (+" + fl.replace(r["verdict"], "").strip("+") + ")") if fl and fl != r["verdict"] and fl.replace(r["verdict"], "").strip("+") else "")
    c = CAUSE.get(r["sop"], r.get("detail", ""))
    L.append("| %s | %s | %s | %s | %s | %s |" % (r["sop"], h, x, r.get("p", "-"), v, c.replace("|", "/")))
L.append("\n## Extra generator cases (not ledger rows; orient / reverse / pole variants)\n")
L.append("| case | houdini pts/prims/vtx | hexe pts/prims/vtx | P | verdict |")
L.append("|---|---|---|---|---|")
for r in XR:
    h = "/".join(map(str, r["houdini"])) if "houdini" in r else "-"
    x = "/".join(map(str, r["hexe"])) if "hexe" in r else "-"
    L.append("| %s | %s | %s | %s | %s |" % (r["sop"], h, x, r.get("p", "-"), r["verdict"] + (" " + r.get("detail", "")[:120] if r.get("detail") else "")))
L.append("\n## Not covered / caveats\n")
L.append("- `sop-curve`: Houdini 20.5 has no scriptable coords parm on `curve::2.0`; the twin is an Add SOP closed polygon through the same 3 points (MATCH there does NOT verify the Curve SOP: row stays partial).\n- MATCH/WINDING-DIFF rows compare P (index order) + topology + attribute NAMES; attribute VALUES (N, uv, class, area, ...) are never compared.\n- `sop-uvproject` axis mapping (hexe axis y -> Houdini inittype zx), `sop-smooth`/`sop-noise`/`sop-mountain`/`sop-twist`/`sop-bend`/`sop-matchsize` parameter mappings are best guesses (different parm models).\n- `sop-registry` = null passthrough only; `rop-geometry` is not a `sop-*` row and is out of scope.\n\n## H31 findings\n- Before (h30 harness + 12 twins, generators in hexe's own order, N=52): MATCH 6, ORDER-DIFF 36, ATTRIB-DIFF 4, COUNT-DIFF 3, P-DIFF 3. After box/grid/sphere/circle fixed to Houdini's point+prim order: see Histogram above.\n- HANDEDNESS (open, biggest remaining delta): Houdini lists polygons CLOCKWISE seen from the front (its prim normal = left-hand rule: box prim0 [1,5,4,0] -> -Z on the -Z face; grid xy -Z, yz +X, zx +Y). Hexe is counter-clockwise / right-handed in EVERY SOP (Newell helpers, outward-winding tests, polyextrude, boolean, bevel, sphere ico...). Generators therefore keep Houdini's point order, prim order and first corner but run each cell the other way ([a,d,c,b] for Houdini [a,b,c,d]) -> WINDING-DIFF, not MATCH. Adopting Houdini's convention = one global flip (every Newell consumer + all hand-built CCW test meshes); not done in H31.\n- Box `dodivs`/`divs`: Houdini's `divsx/y/z` are SEGMENT counts and it keeps interior lattice points (divs 3,3,3 -> 64 pts / 48 prims; unconsolidated 2,2,2 -> 27 pts of 3-point prims); hexe's `divs` = points per axis, surface only. The divisions order extension in box_sop is UNVERIFIED and the semantics differ; only the DEFAULT box is Houdini-verified.\n- Other cells of the generator order verified in hython: grid orient xy/yz/zx (row 0 at the LOW end of the second axis for all three; yz columns run along Z, rows along Y), circle orient xy/yz/zx x reverse 0/1 (reverse flips the second in-plane component, also for open arcs), sphere polymesh poles (0 top, 1 bottom) with and without triangularpoles.\n")
open(os.path.join(ROOT, "docs", "HOUDINI-ORACLE.md"), "w").write("\n".join(L))
print(hist)
