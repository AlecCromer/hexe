# polybevel parity ledger (salvage 2026-10-10)

Branch `parity/polybevel`. Oracle: hython polybevel::3.0, unit box, all 12 edges, round (shape=4) unless noted.
Refs: `*.topo` (P/F lists, header = pts/prims), `*.geo`. Code: `crates/sop/src/builtins/polybevel_sop/layout.rs`.

## Pt/prim counts (Houdini)
div: pts/prims → 1:24/26 2:56/54 3:96/98 4:152/150 5:216/218 6:296/294 7:384/386 8:488/486 9:600/602 10:728/726 11:864/866 12:1016/1014 13:1176/1178 16:1736/1734 17:1944/1946; solid div2: 32/30.
Δpts alternates +32,+40,+56,+64,+80,+88,+104,+112,+128… ; prims odd div = pts+2, even div = pts−2.

## Corner-patch rule found (div 1..=4, ZERO diff)
- Faces keep prim slot; corner = source vertex order.
- Pts: EVEN div → kept source pts first (= patch centres, same ids); then sector pts face-by-face in BFS order (root prim0 corner0; child face starts at corner opening shared edge; discovery walks each face's edges from its start corner).
- Then ONE group per face (BFS order), edges E1..E(m-1),E0 (E0 = parent edge LAST): alloc pts (even: shared mid + own row-1 + shared seam; odd 3: own row-1 + corner D pt); emit that face's half-strip quads + corner-patch quads (start corner, end corner, once/face).
- ODD div: per edge in first-visit order: middle strip quad + 2 edge-corner quads (div 3); then one tri per vertex (first-appearance order in src prims; starts at face arriving along vertex's earliest-visited edge). EVEN: interior sector pts (g11) numbered last, in emission order.
- Profile rows = conic polyline N = max(4, next_pow2(div−1)) verts, linear resample in t (`profile`).
- Interior positions: fitted constants (LAMBDA .2378 → centre pulled 0.4029·o toward corner; SEAM_PAR .6059, SEAM_PERP .0645). Max err ≤2e-5 on 8 cases.

## Test status (`cargo test -p hexe-sop polybevel`)
- 21 pass. `houdini_order_zero_diff_on_box_divisions_1_to_4_and_offsets` = d1_o10 d2_o10 d2_o20 d2_o05 d2_o40 d3_o10 d3_o25 d4_o10 → pts/prims/order exact, boundary+interior ≤2e-5.
- `houdini_order_zero_diff_on_box_divisions_5_to_17` = #[ignore], FAILS (d5: ours 120 pts vs Houdini 216; layout.rs bails → generic path).

## Remaining gap
1. layout.rs limited to div ≤4 → extend to ≥5 (N=8 at div 5..9, N=16 at 10..17); needs the patch grid generalisation (rows beyond ring-1) + point-count formula above.
2. Centre position law unknown: centre fraction of o moves with N (N=4: .4029, 8: .4120, 16: .4131) → constants are fits, not the rule; find closed form.
3. d2_solid (shape=1) ref captured, no test/impl check.
4. Non-closed / non-valence-3 / lone-edge inputs stay on generic path (no Houdini refs).
5. features.toml sop-polybevel + docs/HOUDINI-ORACLE.md row (hexe 48/38 vs 56/54) → re-verify; ci (`cargo xtask ci`) NOT run → no merge.
