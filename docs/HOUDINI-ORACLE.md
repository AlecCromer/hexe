# HOUDINI-ORACLE (H30)

Real Houdini 20.5.487 (hython, Commercial) twin of every `sop-*` ledger row vs `hexe-cli cook` (prebuilt trunk debug binary). Scripts: `oracle/houdini/sop-*.py` (one per row; build the canonical case, save `fixtures/sop-*/houdini.geo` + `houdini.json`, write the hexe twin `scene.json`). Comparator: `oracle/houdini/compare.py`. Table: `oracle/houdini/report.py`.

Reproduce: `hython oracle/houdini/run_all.py` (all 40 in one process, ~10 s) then `python3 oracle/houdini/compare.py` then `python3 oracle/houdini/report.py`.

Rules: counts = points/prims/vertices. Attribs = per-class name sets + point/prim groups. P compared in index order, tol 1e-4. **Permutation is only classified, never passed**: ORDER-DIFF = same P multiset, different order. Hexe `.geo` is loaded back INTO Houdini for the summary (so a loadable file is checked too). Verdict priority: HOUDINI-ERR > HEXE-ERR > COUNT-DIFF > ATTRIB-DIFF > TOPO-DIFF > ORDER-DIFF > P-DIFF > MATCH (P-DIFF = point multisets differ; TOPO-DIFF = same P but prim point lists / closedness differ).

Histogram: ORDER-DIFF 26, MATCH 5, ATTRIB-DIFF 4, COUNT-DIFF 3, P-DIFF 2  (N=40)

| sop | houdini pts/prims/vtx | hexe pts/prims/vtx | P | verdict | cause (one line) |
|---|---|---|---|---|---|
| sop-add | 3/1/3 | 3/1/3 | MATCH | MATCH | - |
| sop-attribcopy | 9/4/16 | 9/4/16 | ORDER-DIFF | ORDER-DIFF | grid Z mirror (point order) |
| sop-attribdelete | 9/4/16 | 9/4/16 | ORDER-DIFF | ORDER-DIFF | grid Z mirror (point order) |
| sop-attribpromote | 9/4/16 | 9/4/16 | ORDER-DIFF | ORDER-DIFF | grid Z mirror (point order); prim attr v present both |
| sop-attribtransfer | 9/4/16 | 9/4/16 | ORDER-DIFF | ORDER-DIFF | grid Z mirror (point order) |
| sop-blast | 8/5/20 | 8/5/20 | ORDER-DIFF | ORDER-DIFF | box order |
| sop-boolean | 14/9/42 | 14/9/42 | ORDER-DIFF | ATTRIB-DIFF (+ORDER-DIFF) | box order; hexe emits prim groups a_only/b_only/seam unconditionally, Houdini only when output toggles on; counts 14/9/42 match |
| sop-box | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box point/prim order: Houdini = bottom ring (y-) CCW then top ring, prim0=[1,5,4,0]; hexe = x-major (0,1,3,2). Geometry identical as a set |
| sop-circle | 6/1/6 | 6/1/6 | ORDER-DIFF | ORDER-DIFF | Houdini default reverse=1: XY circle runs CLOCKWISE (pt1 = (1,-0.87)); hexe runs CCW. Same point set, one prim |
| sop-copytopoints | 72/54/216 | 72/54/216 | ORDER-DIFF | ATTRIB-DIFF (+ORDER-DIFF) | box point order; hexe writes copynum (point+prim) when param on, Houdini copytopoints::2.0 writes none (needs idattrib/ attribute transfer). Counts 72/54/216 match |
| sop-curve | 3/1/3 | 3/1/3 | MATCH | MATCH | - (Houdini 20.5 curve::2.0 has no coords parm; twin = Add SOP closed polygon, so this checks the data model not the Curve SOP itself) |
| sop-delete | 9/2/8 | 9/2/8 | ORDER-DIFF | ORDER-DIFF | grid Z mirror; 9 pts kept, 2 prims left both |
| sop-facet | 24/6/24 | 24/6/24 | ORDER-DIFF | ORDER-DIFF | box order (unique_points -> 24 pts both; post N present both) |
| sop-file | 6/8/24 | 6/8/24 | MATCH | MATCH | - |
| sop-foreach | 6/0/0 | 6/0/0 | MATCH | MATCH | -  (piece order a,b,c merge gives P x=0,3,1,4,2,5 in both) |
| sop-fuse | 8/6/24 | 32/30/120 | n/a | COUNT-DIFF | input polyextrude differs (see sop-polyextrude): hexe feeds 48 pts, Houdini 8 |
| sop-grid | 12/6/24 | 12/6/24 | ORDER-DIFF | ORDER-DIFF | grid row axis: Houdini row0 at z=-size/2 (ascending +Z); hexe row0 at +Z (descending). Same quad index pattern, mirrored in Z |
| sop-group | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box order; prim group 'top' present both |
| sop-groupdelete | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box order |
| sop-grouppromote | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box order; point group toppts both |
| sop-line | 5/1/5 | 5/1/5 | MATCH | MATCH | - |
| sop-merge | 17/10/40 | 17/10/40 | ORDER-DIFF | ORDER-DIFF | box point order only (box + grid: grid part also Z-mirrored); counts 17/10/40 match |
| sop-noise | 121/0/0 | 121/0/0 | P-DIFF | P-DIFF | noise basis/seed not Houdini's perlin (ledger: UNVERIFIED); Houdini displacement bbox y[0.39,0.77] vs hexe y[-0.28,0.30]; hexe also lacks zero-centred vs positive range semantics (Houdini noiserange default positive) |
| sop-normal | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box order only; vertex N attribute present in both |
| sop-peak | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box point order only; displaced P multiset equals Houdini |
| sop-polybevel | 56/54/216 | 48/38/168 | n/a | COUNT-DIFF | hexe 48/38 vs Houdini 56/54 (offset .1 divs 2 round): hexe bevels fewer elements / emits fewer fillet faces than Houdini polybevel::3.0 |
| sop-polyextrude | 8/6/24 | 48/30/120 | n/a | COUNT-DIFF | Houdini default splittype=connected components: closed box = 1 component, no open edges -> NO new faces (8/6/24, box scales to +-1); hexe extrudes each face individually (48/30/120). Spec/param mismatch (hexe lacks splittype), not a bug in per-face math |
| sop-registry | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | null passthrough of box: box point order only |
| sop-resample | 24/1/24 | 24/1/24 | ORDER-DIFF | ORDER-DIFF | inherits Houdini circle CW direction (y-mirrored sample sequence); sample point SET equals hexe (24 pts) |
| sop-reverse | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box point order only; winding of prims not compared in ORDER-DIFF rows |
| sop-scatter | 500/0/0 | 500/0/0 | P-DIFF | ATTRIB-DIFF (+P-DIFF) | RNG stream differs (expected, ledger says UNVERIFIED); hexe also writes point attrs N + sourceprim that Houdini scatter does not by default. Count 500 matches |
| sop-smooth | 8/6/24 | 8/6/24 | P-DIFF | P-DIFF | different model: Houdini smooth::2.0 (strength 0.5, quality 2, uniform) shrinks box to +-0.4286; hexe Laplacian x2 shrinks to +-0.2222 |
| sop-sort | 9/4/16 | 9/4/16 | ORDER-DIFF | ORDER-DIFF | grid Z mirror (point order); reverse permutation itself consistent (prim_pts identical) |
| sop-sphere | 266/288/1104 | 266/288/1104 | ORDER-DIFF | ORDER-DIFF | polymesh order: Houdini poles are points 0 (top),1 (bottom) then rings, ring runs toward -Z from +X; hexe rings first, poles last (264,265), ring runs toward +Z. Same 266/288 counts, same point set |
| sop-subdivide | 26/24/96 | 26/24/96 | ORDER-DIFF | ORDER-DIFF | box order; Catmull-Clark counts 26/24/96 match Houdini osdcc |
| sop-transform | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box point order only; transformed P multiset equals Houdini (SRT math verified, t/r/s) |
| sop-uvlayout | 8/6/24 | 8/6/24 | ORDER-DIFF | ATTRIB-DIFF (+ORDER-DIFF) | box order; Houdini adds detail attrs coverage + numnonpacked; uv values not compared |
| sop-uvproject | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box order; vertex uv present both (uv VALUES not compared) |
| sop-uvunwrap | 8/6/24 | 8/6/24 | ORDER-DIFF | ORDER-DIFF | box order; uv present both (values not compared; algorithms differ by design) |
| sop-wrangle | 121/0/0 | 121/0/0 | ORDER-DIFF | ORDER-DIFF | inherits grid Z mirror (row0 z=-5 vs +5); sin() math matches |

## Not covered / caveats

- `sop-curve`: Houdini 20.5 has no scriptable coords parm on `curve::2.0`; the twin is an Add SOP closed polygon through the same 3 points.
- ORDER-DIFF rows mask value comparison of derived attributes (N, uv, etc.): only names/counts/P-multiset are checked there. Next wave: fix generator order (box, grid, sphere, circle), then re-run to unmask downstream SOPs.
- `sop-uvproject` axis mapping (hexe axis y -> Houdini inittype zx) and `sop-smooth`/`sop-noise` parameter mapping are approximations (different parm models).
- `sop-registry` = null passthrough only; `rop-geometry` is not a `sop-*` row and is out of scope.
- No features.toml edits; no hexe code touched.
