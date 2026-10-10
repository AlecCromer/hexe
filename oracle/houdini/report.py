#!/usr/bin/env python3
"""report.py -> docs/HOUDINI-ORACLE.md from results.json (+ fixtures/*/houdini.json notes) and the CAUSE table below (human, observed from the P dumps)."""
import json, os
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
def W(x=""):
    return "- (" + x + ")" if x else "-"
CAUSE = {
    "sop-add": "-",
    "sop-bend": "- (Houdini bend toward -X by default, twin sets hexe side=(-1,0,0); parm map is a best guess)",
    "sop-circle": "- (default reverse=1 now ON in hexe: clockwise; one prim, identical point cycle)",
    "sop-curve": "- (real curve::2.0 twin: scriptable authoring parm is the Data/Geometry parm savedstashgeo, Houdini passes the stash through, so this checks data model + prim kind + P, not coords parsing)",
    "sop-file": "-",
    "sop-foreach": "-  (piece order a,b,c merge gives P x=0,3,1,4,2,5 in both)",
    "sop-line": "-",
    "sop-resample": "- (inherits the circle order; sample points equal)",
    "sop-wrangle": "- (grid order now equal; sin() math matches; point-only geometry so no winding)",
    "sop-box": "- (prim0 [1,5,4,0] verbatim, clockwise from outside)",
    "sop-grid": W(),
    "sop-sphere": W("poles 0/1, ring +X toward -Z now equal"),
    "sop-transform": W("SRT math verified (t/r/s)"),
    "sop-merge": W(),
    "sop-registry": W("null passthrough of box"),
    "sop-switch": W("box branch"),
    "sop-output": W(),
    "sop-blast": W(),
    "sop-delete": W("9 pts kept, 2 prims left both"),
    "sop-sort": W("reverse permutation itself consistent"),
    "sop-group": W("prim group 'top' present both"),
    "sop-groupdelete": W(),
    "sop-grouppromote": W("point group toppts both"),
    "sop-attribpromote": W("prim attr v present both"),
    "sop-attribtransfer": W(),
    "sop-attribcopy": W(),
    "sop-attribdelete": W(),
    "sop-normal": W("vertex N present both; N VALUES not compared"),
    "sop-peak": W("displaced P equal Houdini index-for-index (outward)"),
    "sop-connectivity": W("prim attr class present both (values not compared)"),
    "sop-measure": W("prim attr area present both (values not compared)"),
    "sop-matchsize": W("explicit target size/center, P equal"),
    "sop-ray": W("projected P equal Houdini index-for-index"),
    "sop-uvproject": W("vertex uv present both (uv VALUES not compared)"),
    "sop-uvunwrap": W("uv present both (values not compared; algorithms differ by design)"),
    "sop-uvlayout": W("Houdini adds detail attrs coverage + numnonpacked; uv values not compared"),
    "sop-copytopoints": W("hexe writes copynum (point+prim) when param on, Houdini copytopoints::2.0 writes none"),
    "sop-boolean": "hexe emits prim groups a_only/b_only/seam unconditionally, Houdini only when output toggles on; point/prim ORDER of the result still differs (counts 14/9/42 match)",
    "sop-reverse": "- (Houdini keeps the first vertex of a closed polygon: [1,5,4,0] -> [1,0,4,5]; hexe identical since H32)",
    "sop-divide": "zero-diff since H33: piece 0 replaces the prim in place, later pieces appended in REVERSE prim order, quad 2nd tri = (c,d,a). n>4 polys: Houdini ear order (observed, regular polys) NOT reproduced - hexe still fans",
    "sop-clip": "cut-point order + prim order: Houdini appends the cut points (+x,-z) first, prims start [1,2,3,0]; hexe cut points (-x,-z) first. Point SET equal, counts 8/5/20 equal; no cap (dofill 0 vs capclip DEFERRED); winding now equal (H32)",
    "sop-knife": "same cut-point/prim order difference as clip (shared core); 12/10/40 equal, groups blanked in both",
    "sop-twist": "- (hexe twist now turns clockwise seen from +Y like Houdini: +90 moves corner (-x,-z) to (+x,-z); the parm map strength 90 <-> angle 90 is still a best guess)",
    "sop-facet": "zero-diff since H33: unique_points keeps the first vertex of each point on its id, later vertices get new ids appended per source point in DESCENDING order (ascending vertex); 3x3 grid also probed",
    "sop-subdivide": "Catmull-Clark counts 26/24/96 match Houdini osdcc; new face/edge/vertex point ORDER differs",
    "sop-smooth": "different model: Houdini smooth::2.0 (strength 0.5, quality 2, uniform) shrinks box to +-0.4286; hexe Laplacian x2 shrinks to +-0.2222",
    "sop-fuse": "- (box -> polyextrude -> fuse: both sides 8/6/24, polyextrude connected default now equal)",
    "sop-polybevel": "56/54 match (offset .1 divs 2 round); div 1–4 zero-diff incl. point/prim order",
    "sop-polyextrude": "- (splittype 0 connected components = Houdini default: closed box 8/6/24, corners mitre-displaced +-0.5 -> +-1, point order = first appearance; splittype 1 keeps the per-face path; inset/divisions/connected groups beyond the closed-box case UNVERIFIED)",
}
RALL = json.load(open(os.path.join(HERE, "results.json")))
R = [r for r in RALL if r["sop"].startswith("sop-")]
XR = [r for r in RALL if not r["sop"].startswith("sop-")]
hist = {}
for r in R:
    hist[r["verdict"]] = hist.get(r["verdict"], 0) + 1
L = []
L.append("# HOUDINI-ORACLE (H30 harness, H32 re-run)\n")
L.append("Real Houdini 20.5.487 (hython, Commercial) twin of every `sop-*` ledger row vs `hexe-cli cook` (debug build of this tree). Scripts: `oracle/houdini/sop-*.py` (one per row; build the canonical case, save `fixtures/sop-*/houdini.geo` + `houdini.json`, write the hexe twin `scene.json`). Comparator: `oracle/houdini/compare.py`. Table: `oracle/houdini/report.py`.\n")
L.append("Reproduce: `hython oracle/houdini/run_all.py` (re-records every Houdini side in one process, ~20 s) then `cargo xtask oracle --houdini` (= build hexe-cli, `compare.py`, `report.py`).\n")
L.append("Rules: counts = points/prims/vertices. Attribs = per-class name sets + point/prim groups. P compared in index order, tol 1e-4. **Permutation is only classified, never passed**: ORDER-DIFF = same P multiset, different order. Hexe `.geo` is loaded back INTO Houdini for the summary (so a loadable file is checked too). Rows with oracle `stat` (houdini.json) get STAT-PASS / STAT-FAIL instead (parity/README.md). Verdict priority: HOUDINI-ERR > HEXE-ERR > COUNT-DIFF > ATTRIB-DIFF > TOPO-DIFF > WINDING-DIFF > ORDER-DIFF > P-DIFF > MATCH (P-DIFF = point multisets differ; TOPO-DIFF = same P order but prim point lists / closedness differ; WINDING-DIFF = same P order and every differing prim is the same cycle with the same first point run the other way (none left since H32)).\n")
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
L.append("- `sop-curve*`: twin = real `curve::2.0` fed through `savedstashgeo` (no coords parm exists); a pass-through, so coords PARSING / tessellation have no Houdini twin.\n- `STAT-PASS|STAT-FAIL` rows (sop-scatter*, sop-noise*, sop-mountain*): statistical oracle, P is NOT compared (streams differ by construction); rules + tolerances in parity/README.md; the `cause` column shows metric houdini/hexe (deviation).\n- MATCH/WINDING-DIFF rows compare P (index order) + topology + attribute NAMES; attribute VALUES (N, uv, class, area, ...) are never compared.\n- `sop-uvproject` axis mapping (hexe axis y -> Houdini inittype zx), `sop-smooth`/`sop-noise`/`sop-mountain`/`sop-twist`/`sop-bend`/`sop-matchsize` parameter mappings are best guesses (different parm models).\n- `sop-registry` = null passthrough only; `rop-geometry` is not a `sop-*` row and is out of scope.\n\n## H31 / H32 findings\n- Before (h30 harness + 12 twins, generators in hexe's own order, N=52): MATCH 6, ORDER-DIFF 36, ATTRIB-DIFF 4, COUNT-DIFF 3, P-DIFF 3. After box/grid/sphere/circle fixed to Houdini's point+prim order: see Histogram above.\n- HANDEDNESS (closed in H32): Houdini lists polygons CLOCKWISE seen from the front (prim normal = left-hand rule: box prim0 [1,5,4,0] -> -Z on the -Z face). H31 kept hexe counter-clockwise (26 WINDING-DIFF rows); H32 adopts Houdini's winding globally: ONE rule `meshutil::newell_corners`/`prim_normal` (negated textbook Newell), every normal / orientation / signed-volume consumer routes through it, generators emit Houdini's cycles verbatim, `reverse` keeps the first vertex, `twist` turns clockwise from +Y, `polyextrude` gets `splittype` (0 connected components = default).\n- Box `dodivs`/`divs`: Houdini's `divsx/y/z` are SEGMENT counts and it keeps interior lattice points (divs 3,3,3 -> 64 pts / 48 prims; unconsolidated 2,2,2 -> 27 pts of 3-point prims); hexe's `divs` = points per axis, surface only. The divisions order extension in box_sop is UNVERIFIED and the semantics differ; only the DEFAULT box is Houdini-verified.\n- Other cells of the generator order verified in hython: grid orient xy/yz/zx (row 0 at the LOW end of the second axis for all three; yz columns run along Z, rows along Y), circle orient xy/yz/zx x reverse 0/1 (reverse flips the second in-plane component, also for open arcs), sphere polymesh poles (0 top, 1 bottom) with and without triangularpoles.\n")
open(os.path.join(ROOT, "docs", "HOUDINI-ORACLE.md"), "w").write("\n".join(L))
print(hist)
