# boolean parity ledger (2026-10-10)
Branch `parity/boolean`. Oracle: hython boolean::2.0, asurface=bsurface=solid; A = unit box. Script `oracle/houdini/boolean-ref.py` → `*.geo` `*.topo` `SUMMARY.txt`.
B: `sph` = sphere polymesh r.65 rows9 cols12 @0 · `off` = box t(.5,.5,.5) · `offx` = box t(.5,0,0). Ops union|sub(aminusb)|int.
## Counts (Houdini = hexe, all 9)
case: pts/prims · valence · area · vol
sph_union 142/118 · 3:40 4:72 18:6 · 6.378 · 1.1934
sph_sub 104/62 · 4:48 6:8 18:6 · 5.108 · .1368
sph_int 96/62 · 4:48 6:8 12:6 · 4.711 · .8633
off_union 20/12 · 4:6 6:6 · 10.5 · 1.875
off_sub 14/9 · 4:6 6:3 · 6.0 · .875
off_int 8/6 · 4:6 · 1.5 · .125
offx_union 16/14 · 4:14 · 8.0 · 1.5
offx_sub 8/6 · 4:6 · 4.0 · .5
offx_int 8/6 · 4:6 · 4.0 · .5
No prim/point groups, no prim attrs in Houdini output (hexe still adds a_only/b_only/seam + seampts).
## Rules found
- Face crossed by a closed curve (sphere pokes a box face) = ONE polygon, hole bridged: cycle `h0, o0, o1..o(k-1), o0, h0, h1..h(m-1)` (18 = 4 outer + 12 hole + 2 repeats). Bridge = closest (outer corner, hole vertex). Done in `bridge_hole`.
- Points numbered by first appearance over prim order (hexe now same).
- Only ONE hole handled; ≥2 holes stay fragments.
## Remaining diff (ORDER only; P multiset equal in 9/9)
1. Prim order: Houdini = untouched A prims, split A prims in A order, B cut faces. Exact now: sph_int, off_int only.
2. Start vertex of split / bridged polys not derived (e.g. off_int prim0 H starts (.5,.5,.5), hexe (.5,0,.5)) → point ids differ.
3. Hole-poly start/bridge pair vs Houdini unverified (sph_sub face0 H: 0 1 [2..13] 2 1 14 15; face1 starts at hole pt).
## Test
`cargo test -p hexe-sop boolean` → `houdini_stat_oracle_box_sphere_and_offset_boxes` (counts, valence, area, P multiset 1e-4). Tag `stat-oracle`.
Compare script (adhoc, not in repo): hexe-cli cook scenes in scratch, hython diff.
