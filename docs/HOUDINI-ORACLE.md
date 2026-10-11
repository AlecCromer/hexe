# HOUDINI-ORACLE (H30 harness, H32 re-run)

Real Houdini 20.5.487 (hython, Commercial) twin of every `sop-*` ledger row vs `hexe-cli cook` (debug build of this tree). Scripts: `oracle/houdini/sop-*.py` (one per row; build the canonical case, save `fixtures/sop-*/houdini.geo` + `houdini.json`, write the hexe twin `scene.json`). Comparator: `oracle/houdini/compare.py`. Table: `oracle/houdini/report.py`.

Reproduce: `hython oracle/houdini/run_all.py` (re-records every Houdini side in one process, ~20 s) then `cargo xtask oracle --houdini` (= build hexe-cli, `compare.py`, `report.py`).

Rules: counts = points/prims/vertices. Attribs = per-class name sets + point/prim groups. P compared in index order, tol 1e-4. **Permutation is only classified, never passed**: ORDER-DIFF = same P multiset, different order. Hexe `.geo` is loaded back INTO Houdini for the summary (so a loadable file is checked too). Rows with oracle `stat` (houdini.json) get STAT-PASS / STAT-FAIL instead (parity/README.md). Verdict priority: HOUDINI-ERR > HEXE-ERR > COUNT-DIFF > ATTRIB-DIFF > TOPO-DIFF > WINDING-DIFF > ORDER-DIFF > P-DIFF > MATCH (P-DIFF = point multisets differ; TOPO-DIFF = same P order but prim point lists / closedness differ; WINDING-DIFF = same P order and every differing prim is the same cycle with the same first point run the other way (none left since H32)).

Histogram: MATCH 89, STAT-PASS 10, TOPO-DIFF 1  (N=100)

| sop | houdini pts/prims/vtx | hexe pts/prims/vtx | P | verdict | cause guess (one line) |
|---|---|---|---|---|---|
| sop-add | 3/1/3 | 3/1/3 | MATCH | MATCH | - |
| sop-attribcopy | 9/4/16 | 9/4/16 | MATCH | MATCH | - |
| sop-attribdelete | 9/4/16 | 9/4/16 | MATCH | MATCH | - |
| sop-attribpromote | 9/4/16 | 9/4/16 | MATCH | MATCH | - (prim attr v present both) |
| sop-attribtransfer | 9/4/16 | 9/4/16 | MATCH | MATCH | - |
| sop-bend | 5/1/5 | 5/1/5 | MATCH | MATCH | - (Houdini bend toward -X by default, twin sets hexe side=(-1,0,0); parm map is a best guess) |
| sop-blast | 8/5/20 | 8/5/20 | MATCH | MATCH | - |
| sop-boolean | 14/9/42 | 14/9/42 | MATCH | TOPO-DIFF | H35: P ORDER now MATCH (A pieces then B, untouched before split in source prim order, untouched prims keep the source cycle, points by first appearance); remaining TOPO-DIFF = start vertex of 4 split polys (+x/+z/+y of A and B -x), rule not derived (parity/ref/boolean/bx*.topo corpus) |
| sop-box | 8/6/24 | 8/6/24 | MATCH | MATCH | - (prim0 [1,5,4,0] verbatim, clockwise from outside) |
| sop-circle | 6/1/6 | 6/1/6 | MATCH | MATCH | - (default reverse=1 now ON in hexe: clockwise; one prim, identical point cycle) |
| sop-clip | 8/5/20 | 8/5/20 | MATCH | MATCH | cut-point order + prim order: Houdini appends the cut points (+x,-z) first, prims start [1,2,3,0]; hexe cut points (-x,-z) first. Point SET equal, counts 8/5/20 equal; no cap (dofill 0 vs capclip DEFERRED); winding now equal (H32) |
| sop-clip-below | 8/5/20 | 8/5/20 | MATCH | MATCH |  |
| sop-clip-both | 12/10/40 | 12/10/40 | MATCH | MATCH |  |
| sop-clip-cap | 8/6/24 | 8/6/24 | MATCH | MATCH |  |
| sop-clip-grid | 8/3/12 | 8/3/12 | MATCH | MATCH |  |
| sop-clip-sphere | 294/316/1216 | 294/316/1216 | MATCH | MATCH |  |
| sop-clip-tilt | 10/6/24 | 10/6/24 | MATCH | MATCH |  |
| sop-clip-tilt-below | 10/6/24 | 10/6/24 | MATCH | MATCH |  |
| sop-connectivity | 16/12/48 | 16/12/48 | MATCH | MATCH | - (prim attr class present both (values not compared)) |
| sop-copytopoints | 72/54/216 | 72/54/216 | MATCH | MATCH | - (hexe writes copynum (point+prim) when param on, Houdini copytopoints::2.0 writes none) |
| sop-copytopoints-orient | 72/54/216 | 72/54/216 | MATCH | MATCH |  |
| sop-copytopoints-orient-flip | 72/54/216 | 72/54/216 | MATCH | MATCH |  |
| sop-copytopoints-orient-up | 72/54/216 | 72/54/216 | MATCH | MATCH |  |
| sop-copytopoints-pscale | 72/54/216 | 72/54/216 | MATCH | MATCH |  |
| sop-curve | 3/1/3 | 3/1/3 | MATCH | MATCH | - (real curve::2.0 twin: scriptable authoring parm is the Data/Geometry parm savedstashgeo, Houdini passes the stash through, so this checks data model + prim kind + P, not coords parsing) |
| sop-curve-bezier | 4/1/4 | 4/1/4 | MATCH | MATCH |  |
| sop-curve-nurbs | 4/1/4 | 4/1/4 | MATCH | MATCH |  |
| sop-curve-nurbs-closed | 5/1/5 | 5/1/5 | MATCH | MATCH |  |
| sop-curve-poly-open | 5/1/5 | 5/1/5 | MATCH | MATCH |  |
| sop-delete | 9/2/8 | 9/2/8 | MATCH | MATCH | - (9 pts kept, 2 prims left both) |
| sop-divide | 8/12/36 | 8/12/36 | MATCH | MATCH | zero-diff since H33: piece 0 replaces the prim in place, later pieces appended in REVERSE prim order, quad 2nd tri = (c,d,a). n>4 polys: Houdini ear order (observed, regular polys) NOT reproduced - hexe still fans |
| sop-facet | 24/6/24 | 24/6/24 | MATCH | MATCH | zero-diff since H33: unique_points keeps the first vertex of each point on its id, later vertices get new ids appended per source point in DESCENDING order (ascending vertex); 3x3 grid also probed |
| sop-file | 6/8/24 | 6/8/24 | MATCH | MATCH | - |
| sop-foreach | 6/0/0 | 6/0/0 | MATCH | MATCH | -  (piece order a,b,c merge gives P x=0,3,1,4,2,5 in both) |
| sop-fuse | 8/6/24 | 8/6/24 | MATCH | MATCH | - (box -> polyextrude -> fuse: both sides 8/6/24, polyextrude connected default now equal) |
| sop-grid | 12/6/24 | 12/6/24 | MATCH | MATCH | - |
| sop-group | 8/6/24 | 8/6/24 | MATCH | MATCH | - (prim group 'top' present both) |
| sop-groupdelete | 8/6/24 | 8/6/24 | MATCH | MATCH | - |
| sop-grouppromote | 8/6/24 | 8/6/24 | MATCH | MATCH | - (point group toppts both) |
| sop-knife | 12/10/40 | 12/10/40 | MATCH | MATCH | same cut-point/prim order difference as clip (shared core); 12/10/40 equal, groups blanked in both |
| sop-knife-below | 8/5/20 | 8/5/20 | MATCH | MATCH |  |
| sop-knife-box-above | 8/5/20 | 8/5/20 | MATCH | MATCH |  |
| sop-knife-grid | 17/10/40 | 17/10/40 | MATCH | MATCH |  |
| sop-knife-sphere | 294/316/1216 | 294/316/1216 | MATCH | MATCH |  |
| sop-knife-sphere2 | 306/328/1264 | 306/328/1264 | MATCH | MATCH |  |
| sop-knife-tilt | 14/12/48 | 14/12/48 | MATCH | MATCH |  |
| sop-line | 5/1/5 | 5/1/5 | MATCH | MATCH | - |
| sop-matchsize | 8/6/24 | 8/6/24 | MATCH | MATCH | - (explicit target size/center, P equal) |
| sop-measure | 8/6/24 | 8/6/24 | MATCH | MATCH | - (prim attr area present both (values not compared)) |
| sop-merge | 17/10/40 | 17/10/40 | MATCH | MATCH | - |
| sop-mountain | 4900/4761/19044 | 4900/4761/19044 | stat | STAT-PASS | disp_mean 0.03882/0.04032(+3.9%) disp_std 0.03039/0.02925(-3.8%) |
| sop-mountain-freq | 4900/4761/19044 | 4900/4761/19044 | stat | STAT-PASS | disp_mean 0.03881/0.03936(+1.4%) disp_std 0.03024/0.02902(-4.0%) |
| sop-mountain-oct | 4900/4761/19044 | 4900/4761/19044 | stat | STAT-PASS | disp_mean 0.03635/0.03666(+0.9%) disp_std 0.02779/0.02749(-1.1%) |
| sop-noise | 4900/0/0 | 4900/0/0 | stat | STAT-PASS | disp_mean 0.2997/0.3092(+3.2%) disp_std 0.1438/0.1387(-3.5%) |
| sop-noise-freq | 4900/0/0 | 4900/0/0 | stat | STAT-PASS | disp_mean 0.2952/0.3076(+4.2%) disp_std 0.1417/0.136(-4.0%) |
| sop-noise-oct | 4900/0/0 | 4900/0/0 | stat | STAT-PASS | disp_mean 0.2733/0.2783(+1.8%) disp_std 0.1258/0.1219(-3.1%) |
| sop-normal | 8/6/24 | 8/6/24 | MATCH | MATCH | - (vertex N present both; N VALUES not compared) |
| sop-output | 8/6/24 | 8/6/24 | MATCH | MATCH | - |
| sop-peak | 8/6/24 | 8/6/24 | MATCH | MATCH | - (displaced P equal Houdini index-for-index (outward)) |
| sop-polybevel | 56/54/216 | 56/54/216 | MATCH | MATCH | 56/54 match (offset .1 divs 2 round); div 1–4 zero-diff incl. point/prim order |
| sop-polyextrude | 8/6/24 | 8/6/24 | MATCH | MATCH | - (splittype 0 connected components = Houdini default: closed box 8/6/24, corners mitre-displaced +-0.5 -> +-1, point order = first appearance; splittype 1 keeps the per-face path; inset/divisions/connected groups beyond the closed-box case UNVERIFIED) |
| sop-polyfill | 8/6/24 | 8/6/24 | MATCH | MATCH |  |
| sop-polyfill-ears | 8/7/26 | 8/7/26 | MATCH | MATCH |  |
| sop-polyfill-fan | 9/9/32 | 9/9/32 | MATCH | MATCH |  |
| sop-polyfill-quads | 9/7/28 | 9/7/28 | MATCH | MATCH |  |
| sop-ray | 9/4/16 | 9/4/16 | MATCH | MATCH | - (projected P equal Houdini index-for-index) |
| sop-registry | 8/6/24 | 8/6/24 | MATCH | MATCH | - (null passthrough of box) |
| sop-resample | 24/1/24 | 24/1/24 | MATCH | MATCH | - (inherits the circle order; sample points equal) |
| sop-reverse | 8/6/24 | 8/6/24 | MATCH | MATCH | - (Houdini keeps the first vertex of a closed polygon: [1,5,4,0] -> [1,0,4,5]; hexe identical since H32) |
| sop-revolve | 288/288/1152 | 288/288/1152 | MATCH | MATCH |  |
| sop-revolve-arc | 84/72/288 | 84/72/288 | MATCH | MATCH |  |
| sop-revolve-tris | 96/192/576 | 96/192/576 | MATCH | MATCH |  |
| sop-revolve-wall | 24/12/48 | 24/12/48 | MATCH | MATCH |  |
| sop-scatter | 2000/0/0 | 2000/0/0 | stat | STAT-PASS | stdx 2.877/2.912(+1.2%) stdy 0/0(+0.0%) stdz 2.884/2.839(-1.5%) nn 0.1128/0.1138(+0.9%) |
| sop-scatter-density | 2000/0/0 | 2000/0/0 | stat | STAT-PASS | stdx 2.367/2.363(-0.2%) stdy 0/0(+0.0%) stdz 2.901/2.94(+1.4%) nn 0.106/0.1057(-0.2%) |
| sop-scatter-perprim | 2010/0/0 | 2010/0/0 | stat | STAT-PASS | stdx 2.847/2.863(+0.5%) stdy 0/0(+0.0%) stdz 2.865/2.861(-0.1%) nn 0.11/0.1105(+0.5%) |
| sop-scatter-seed | 1500/0/0 | 1500/0/0 | stat | STAT-PASS | stdx 0.3682/0.3751(+1.9%) stdy 0.3725/0.3751(+0.7%) stdz 0.3705/0.3686(-0.5%) nn 0.031/0.03204(+3.4%) |
| sop-skin | 36/24/96 | 36/24/96 | MATCH | MATCH |  |
| sop-skin-2ring | 24/12/48 | 24/12/48 | MATCH | MATCH |  |
| sop-skin-closev | 24/24/96 | 24/24/96 | MATCH | MATCH |  |
| sop-skin-eq | 36/24/96 | 36/24/96 | MATCH | MATCH |  |
| sop-skin-tris | 12/12/36 | 12/12/36 | MATCH | MATCH |  |
| sop-smooth | 8/6/24 | 8/6/24 | MATCH | MATCH | H35 MATCH: hexe now implements the hython-fitted smooth::2.0 model: (I + strength*K) x = P per channel, M = I - D^-1 A (D = distinct-neighbour count), pinned rows of M zeroed, K = diag(M^q)^-1 M^q, q = min(filterquality,5). 200 probe replays (box/grid5/sphere/fan/perturbed grid x none/unsharededges) max/dP/ 5.7e-5 = Houdini's own iterative-solve noise (parity/ref/smooth-probes/). Not probed: groupboundary (Houdini default), group parm, scale/curvature methods. |
| sop-sort | 9/4/16 | 9/4/16 | MATCH | MATCH | - (reverse permutation itself consistent) |
| sop-sphere | 266/288/1104 | 266/288/1104 | MATCH | MATCH | - (poles 0/1, ring +X toward -Z now equal) |
| sop-subdivide | 26/24/96 | 26/24/96 | MATCH | MATCH | zero-diff since H33: face pts then edge pts (first encounter) were already equal; quad child k ring rotated right by k (box), tri/pentagon children unrotated (probed) |
| sop-sweep | 300/288/1152 | 300/288/1152 | MATCH | MATCH |  |
| sop-sweep-caps | 300/290/1176 | 300/290/1176 | MATCH | MATCH |  |
| sop-sweep-closed | 288/288/1152 | 288/288/1152 | MATCH | MATCH |  |
| sop-sweep-roll | 300/288/1152 | 300/288/1152 | MATCH | MATCH |  |
| sop-sweep-scale | 300/288/1152 | 300/288/1152 | MATCH | MATCH |  |
| sop-sweep-tube | 200/194/784 | 200/194/784 | MATCH | MATCH |  |
| sop-switch | 8/6/24 | 8/6/24 | MATCH | MATCH | - (box branch) |
| sop-transform | 8/6/24 | 8/6/24 | MATCH | MATCH | - (SRT math verified (t/r/s)) |
| sop-twist | 8/6/24 | 8/6/24 | MATCH | MATCH | - (hexe twist now turns clockwise seen from +Y like Houdini: +90 moves corner (-x,-z) to (+x,-z); the parm map strength 90 <-> angle 90 is still a best guess) |
| sop-uvlayout | 8/6/24 | 8/6/24 | MATCH | MATCH | - (H35: detail attrs coverage + numnonpacked MATCH; twin = Houdini defaults (res 1024, 1 texel); uv VALUES not compared here (hexe uvunwrap island orientation differs) - proven on Houdini's own unwrap geo by test houdini_unwrap_in_gives_houdini_island_order_pitch_and_cells) |
| sop-uvlayout-stat | 8/6/24 | 8/6/24 | MATCH | MATCH |  |
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

- `sop-curve*`: twin = real `curve::2.0` fed through `savedstashgeo` (no coords parm exists); a pass-through, so coords PARSING / tessellation have no Houdini twin.
- `STAT-PASS|STAT-FAIL` rows (sop-scatter*, sop-noise*, sop-mountain*): statistical oracle, P is NOT compared (streams differ by construction); rules + tolerances in parity/README.md; the `cause` column shows metric houdini/hexe (deviation).
- MATCH/WINDING-DIFF rows compare P (index order) + topology + attribute NAMES; attribute VALUES (N, uv, class, area, ...) are never compared.
- `sop-uvproject` axis mapping (hexe axis y -> Houdini inittype zx), `sop-smooth`/`sop-noise`/`sop-mountain`/`sop-twist`/`sop-bend`/`sop-matchsize` parameter mappings are best guesses (different parm models).
- `sop-registry` = null passthrough only; `rop-geometry` is not a `sop-*` row and is out of scope.

## H31 / H32 findings
- Before (h30 harness + 12 twins, generators in hexe's own order, N=52): MATCH 6, ORDER-DIFF 36, ATTRIB-DIFF 4, COUNT-DIFF 3, P-DIFF 3. After box/grid/sphere/circle fixed to Houdini's point+prim order: see Histogram above.
- HANDEDNESS (closed in H32): Houdini lists polygons CLOCKWISE seen from the front (prim normal = left-hand rule: box prim0 [1,5,4,0] -> -Z on the -Z face). H31 kept hexe counter-clockwise (26 WINDING-DIFF rows); H32 adopts Houdini's winding globally: ONE rule `meshutil::newell_corners`/`prim_normal` (negated textbook Newell), every normal / orientation / signed-volume consumer routes through it, generators emit Houdini's cycles verbatim, `reverse` keeps the first vertex, `twist` turns clockwise from +Y, `polyextrude` gets `splittype` (0 connected components = default).
- Box `dodivs`/`divs`: Houdini's `divsx/y/z` are SEGMENT counts and it keeps interior lattice points (divs 3,3,3 -> 64 pts / 48 prims; unconsolidated 2,2,2 -> 27 pts of 3-point prims); hexe's `divs` = points per axis, surface only. The divisions order extension in box_sop is UNVERIFIED and the semantics differ; only the DEFAULT box is Houdini-verified.
- Other cells of the generator order verified in hython: grid orient xy/yz/zx (row 0 at the LOW end of the second axis for all three; yz columns run along Z, rows along Y), circle orient xy/yz/zx x reverse 0/1 (reverse flips the second in-plane component, also for open arcs), sphere polymesh poles (0 top, 1 bottom) with and without triangularpoles.
