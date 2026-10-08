# HOUDINI-ORACLE (H30 harness, H32 re-run)

Real Houdini 20.5.487 (hython, Commercial) twin of every `sop-*` ledger row vs `hexe-cli cook` (debug build of this tree). Scripts: `oracle/houdini/sop-*.py` (one per row; build the canonical case, save `fixtures/sop-*/houdini.geo` + `houdini.json`, write the hexe twin `scene.json`). Comparator: `oracle/houdini/compare.py`. Table: `oracle/houdini/report.py`.

Reproduce: `hython oracle/houdini/run_all.py` (re-records every Houdini side in one process, ~20 s) then `cargo xtask oracle --houdini` (= build hexe-cli, `compare.py`, `report.py`).

Rules: counts = points/prims/vertices. Attribs = per-class name sets + point/prim groups. P compared in index order, tol 1e-4. **Permutation is only classified, never passed**: ORDER-DIFF = same P multiset, different order. Hexe `.geo` is loaded back INTO Houdini for the summary (so a loadable file is checked too). Verdict priority: HOUDINI-ERR > HEXE-ERR > COUNT-DIFF > ATTRIB-DIFF > TOPO-DIFF > WINDING-DIFF > ORDER-DIFF > P-DIFF > MATCH (P-DIFF = point multisets differ; TOPO-DIFF = same P order but prim point lists / closedness differ; WINDING-DIFF = same P order and every differing prim is the same cycle with the same first point run the other way (none left since H32)).

Histogram: MATCH 39, ATTRIB-DIFF 4, TOPO-DIFF 4, P-DIFF 3, ORDER-DIFF 1, COUNT-DIFF 1  (N=52)

| sop | houdini pts/prims/vtx | hexe pts/prims/vtx | P | verdict | cause guess (one line) |
|---|---|---|---|---|---|
| sop-add | 3/1/3 | 3/1/3 | MATCH | MATCH | - |
| sop-attribcopy | 9/4/16 | 9/4/16 | MATCH | MATCH | - |
| sop-attribdelete | 9/4/16 | 9/4/16 | MATCH | MATCH | - |
| sop-attribpromote | 9/4/16 | 9/4/16 | MATCH | MATCH | - (prim attr v present both) |
| sop-attribtransfer | 9/4/16 | 9/4/16 | MATCH | MATCH | - |
| sop-bend | 5/1/5 | 5/1/5 | MATCH | MATCH | - (Houdini bend toward -X by default, twin sets hexe side=(-1,0,0); parm map is a best guess) |
| sop-blast | 8/5/20 | 8/5/20 | MATCH | MATCH | - |
| sop-boolean | 14/9/42 | 14/9/42 | ORDER-DIFF | ATTRIB-DIFF (+ORDER-DIFF) | hexe emits prim groups a_only/b_only/seam unconditionally, Houdini only when output toggles on; point/prim ORDER of the result still differs (counts 14/9/42 match) |
| sop-box | 8/6/24 | 8/6/24 | MATCH | MATCH | - (prim0 [1,5,4,0] verbatim, clockwise from outside) |
| sop-circle | 6/1/6 | 6/1/6 | MATCH | MATCH | - (default reverse=1 now ON in hexe: clockwise; one prim, identical point cycle) |
| sop-clip | 8/5/20 | 8/5/20 | MATCH | TOPO-DIFF | cut-point order + prim order: Houdini appends the cut points (+x,-z) first, prims start [1,2,3,0]; hexe cut points (-x,-z) first. Point SET equal, counts 8/5/20 equal; no cap (dofill 0 vs capclip DEFERRED); winding now equal (H32) |
| sop-connectivity | 16/12/48 | 16/12/48 | MATCH | MATCH | - (prim attr class present both (values not compared)) |
| sop-copytopoints | 72/54/216 | 72/54/216 | MATCH | ATTRIB-DIFF | - (hexe writes copynum (point+prim) when param on, Houdini copytopoints::2.0 writes none) |
| sop-curve | 3/1/3 | 3/1/3 | MATCH | MATCH | - (Houdini 20.5 curve::2.0 has no coords parm; twin = Add SOP closed polygon, so this checks the data model not the Curve SOP itself) |
| sop-delete | 9/2/8 | 9/2/8 | MATCH | MATCH | - (9 pts kept, 2 prims left both) |
| sop-divide | 8/12/36 | 8/12/36 | MATCH | TOPO-DIFF | triangulation order/diagonal: Houdini [1,5,4],[2,6,5],[3,7,6],[0,4,7].. (one tri per quad first); hexe [1,0,4],[1,4,5],[2,1,5].. (fan per quad). Counts 8/12/36 equal, P equal |
| sop-facet | 24/6/24 | 24/6/24 | ORDER-DIFF | ORDER-DIFF | unique_points: Houdini keeps the original 8 ids on prim 0 and appends the other corners ([1,5,4,0],[2,6,12,20]..); hexe renumbers 4 fresh points per prim |
| sop-file | 6/8/24 | 6/8/24 | MATCH | MATCH | - |
| sop-foreach | 6/0/0 | 6/0/0 | MATCH | MATCH | -  (piece order a,b,c merge gives P x=0,3,1,4,2,5 in both) |
| sop-fuse | 8/6/24 | 8/6/24 | MATCH | MATCH | - (box -> polyextrude -> fuse: both sides 8/6/24, polyextrude connected default now equal) |
| sop-grid | 12/6/24 | 12/6/24 | MATCH | MATCH | - |
| sop-group | 8/6/24 | 8/6/24 | MATCH | MATCH | - (prim group 'top' present both) |
| sop-groupdelete | 8/6/24 | 8/6/24 | MATCH | MATCH | - |
| sop-grouppromote | 8/6/24 | 8/6/24 | MATCH | MATCH | - (point group toppts both) |
| sop-knife | 12/10/40 | 12/10/40 | MATCH | TOPO-DIFF | same cut-point/prim order difference as clip (shared core); 12/10/40 equal, groups blanked in both |
| sop-line | 5/1/5 | 5/1/5 | MATCH | MATCH | - |
| sop-matchsize | 8/6/24 | 8/6/24 | MATCH | MATCH | - (explicit target size/center, P equal) |
| sop-measure | 8/6/24 | 8/6/24 | MATCH | MATCH | - (prim attr area present both (values not compared)) |
| sop-merge | 17/10/40 | 17/10/40 | MATCH | MATCH | - |
| sop-mountain | 121/100/400 | 121/100/400 | P-DIFF | P-DIFF | noise stream: hexe perlin(seed) vs Houdini perlin basis (no seed twin); Houdini y[-0.15,0.12] vs hexe different. Counts equal |
| sop-noise | 121/0/0 | 121/0/0 | P-DIFF | P-DIFF | noise basis/seed not Houdini's perlin (ledger: UNVERIFIED); Houdini displacement bbox y[0.39,0.77] vs hexe y[-0.28,0.30]; Houdini noiserange default positive |
| sop-normal | 8/6/24 | 8/6/24 | MATCH | MATCH | - (vertex N present both; N VALUES not compared) |
| sop-output | 8/6/24 | 8/6/24 | MATCH | MATCH | - |
| sop-peak | 8/6/24 | 8/6/24 | MATCH | MATCH | - (displaced P equal Houdini index-for-index (outward)) |
| sop-polybevel | 56/54/216 | 48/38/168 | n/a | COUNT-DIFF | hexe 48/38 vs Houdini 56/54 (offset .1 divs 2 round): hexe bevels fewer elements / emits fewer fillet faces than Houdini polybevel::3.0 |
| sop-polyextrude | 8/6/24 | 8/6/24 | MATCH | MATCH | - (splittype 0 connected components = Houdini default: closed box 8/6/24, corners mitre-displaced +-0.5 -> +-1, point order = first appearance; splittype 1 keeps the per-face path; inset/divisions/connected groups beyond the closed-box case UNVERIFIED) |
| sop-ray | 9/4/16 | 9/4/16 | MATCH | MATCH | - (projected P equal Houdini index-for-index) |
| sop-registry | 8/6/24 | 8/6/24 | MATCH | MATCH | - (null passthrough of box) |
| sop-resample | 24/1/24 | 24/1/24 | MATCH | MATCH | - (inherits the circle order; sample points equal) |
| sop-reverse | 8/6/24 | 8/6/24 | MATCH | MATCH | - (Houdini keeps the first vertex of a closed polygon: [1,5,4,0] -> [1,0,4,5]; hexe identical since H32) |
| sop-scatter | 500/0/0 | 500/0/0 | P-DIFF | ATTRIB-DIFF (+P-DIFF) | RNG stream differs (expected, ledger says UNVERIFIED); hexe also writes point attrs N + sourceprim that Houdini scatter does not by default. Count 500 matches |
| sop-smooth | 8/6/24 | 8/6/24 | P-DIFF | P-DIFF | different model: Houdini smooth::2.0 (strength 0.5, quality 2, uniform) shrinks box to +-0.4286; hexe Laplacian x2 shrinks to +-0.2222 |
| sop-sort | 9/4/16 | 9/4/16 | MATCH | MATCH | - (reverse permutation itself consistent) |
| sop-sphere | 266/288/1104 | 266/288/1104 | MATCH | MATCH | - (poles 0/1, ring +X toward -Z now equal) |
| sop-subdivide | 26/24/96 | 26/24/96 | MATCH | TOPO-DIFF | Catmull-Clark counts 26/24/96 match Houdini osdcc; new face/edge/vertex point ORDER differs |
| sop-switch | 8/6/24 | 8/6/24 | MATCH | MATCH | - (box branch) |
| sop-transform | 8/6/24 | 8/6/24 | MATCH | MATCH | - (SRT math verified (t/r/s)) |
| sop-twist | 8/6/24 | 8/6/24 | MATCH | MATCH | - (hexe twist now turns clockwise seen from +Y like Houdini: +90 moves corner (-x,-z) to (+x,-z); the parm map strength 90 <-> angle 90 is still a best guess) |
| sop-uvlayout | 8/6/24 | 8/6/24 | MATCH | ATTRIB-DIFF | - (Houdini adds detail attrs coverage + numnonpacked; uv values not compared) |
| sop-uvproject | 8/6/24 | 8/6/24 | MATCH | MATCH | - (vertex uv present both (uv VALUES not compared)) |
| sop-uvunwrap | 8/6/24 | 8/6/24 | MATCH | MATCH | - (uv present both (values not compared; algorithms differ by design)) |
| sop-wrangle | 121/0/0 | 121/0/0 | MATCH | MATCH | - (grid order now equal; sin() math matches; point-only geometry so no winding) |

## Extra generator cases (not ledger rows; orient / reverse / pole variants)

| case | houdini pts/prims/vtx | hexe pts/prims/vtx | P | verdict |
|---|---|---|---|---|
| x-circle-yz | 6/1/6 | 6/1/6 | MATCH | MATCH |
| x-circle-zx | 6/1/6 | 6/1/6 | MATCH | MATCH |
| x-grid-xy | 12/6/24 | 12/6/24 | MATCH | MATCH |
| x-grid-yz | 12/6/24 | 12/6/24 | MATCH | MATCH |
| x-sphere-tri0 | 14/16/64 | 14/16/64 | MATCH | MATCH |

## Not covered / caveats

- `sop-curve`: Houdini 20.5 has no scriptable coords parm on `curve::2.0`; the twin is an Add SOP closed polygon through the same 3 points (MATCH there does NOT verify the Curve SOP: row stays partial).
- MATCH/WINDING-DIFF rows compare P (index order) + topology + attribute NAMES; attribute VALUES (N, uv, class, area, ...) are never compared.
- `sop-uvproject` axis mapping (hexe axis y -> Houdini inittype zx), `sop-smooth`/`sop-noise`/`sop-mountain`/`sop-twist`/`sop-bend`/`sop-matchsize` parameter mappings are best guesses (different parm models).
- `sop-registry` = null passthrough only; `rop-geometry` is not a `sop-*` row and is out of scope.

## H31 / H32 findings
- Before (h30 harness + 12 twins, generators in hexe's own order, N=52): MATCH 6, ORDER-DIFF 36, ATTRIB-DIFF 4, COUNT-DIFF 3, P-DIFF 3. After box/grid/sphere/circle fixed to Houdini's point+prim order: see Histogram above.
- HANDEDNESS (closed in H32): Houdini lists polygons CLOCKWISE seen from the front (prim normal = left-hand rule: box prim0 [1,5,4,0] -> -Z on the -Z face). H31 kept hexe counter-clockwise (26 WINDING-DIFF rows); H32 adopts Houdini's winding globally: ONE rule `meshutil::newell_corners`/`prim_normal` (negated textbook Newell), every normal / orientation / signed-volume consumer routes through it, generators emit Houdini's cycles verbatim, `reverse` keeps the first vertex, `twist` turns clockwise from +Y, `polyextrude` gets `splittype` (0 connected components = default).
- Box `dodivs`/`divs`: Houdini's `divsx/y/z` are SEGMENT counts and it keeps interior lattice points (divs 3,3,3 -> 64 pts / 48 prims; unconsolidated 2,2,2 -> 27 pts of 3-point prims); hexe's `divs` = points per axis, surface only. The divisions order extension in box_sop is UNVERIFIED and the semantics differ; only the DEFAULT box is Houdini-verified.
- Other cells of the generator order verified in hython: grid orient xy/yz/zx (row 0 at the LOW end of the second axis for all three; yz columns run along Z, rows along Y), circle orient xy/yz/zx x reverse 0/1 (reverse flips the second in-plane component, also for open arcs), sphere polymesh poles (0 top, 1 bottom) with and without triangularpoles.
