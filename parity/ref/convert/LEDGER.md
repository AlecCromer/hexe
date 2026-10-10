# convert parity ledger (2026-10-10)
Branch `parity/convert`. Oracle: hython convert (SOP, Houdini 20.5.487). Script `oracle/houdini/convert-ref.py`; refs `*.topo` (P/F, F has prim type) + `*.geo`. Code: `crates/sop/src/builtins/convert_sop.rs::icosphere`.
## Cases
| case | Houdini pts/prims | hexe |
|---|---|---|
| sphere_lod05 | 12/20 | ZERO diff (<=6e-5) |
| sphere_lod075 | 42/80 | ZERO diff |
| sphere_lod1 | 92/180 | ZERO diff |
| sphere_xf_lod05 (rad 2,3,4 @1,2,3) | 12/20 | ZERO diff |
| sphere_lod2 | 362/720 | order exact, pos <=7e-3 (interior law) |
| tube_lod1 | 140/82 | NOT matched (legacy layout) |
| box_polysoup | 8/1 PolySoup | NOT matched (no PolySoup kind) |
| circle_nurbs_lod1/3 | 156/1, 84/1 | NOT matched |
| nsphere_nurbs_lod1 | 266/1 Mesh | n/a (sphere SOP type 3 = Mesh, not NURBS) |
## Rules
- f = max(1, floor(3*lodu)); lodv irrelevant. pts 10f^2+2, tris 20f^2. Pole axis local Z, xform applied, centre pt dropped.
- 20 base tris (f=1 table in `icosphere`): ring1 z=-1/sqrt5 angles 90-72k, ring2 angles 126+72k, poles pts 0 / 11.
- Per base tri [v0,v1,apex]: lattice (r,c), r from apex, c from v1 side; weights apex (f-r), v1 (r-c), v0 c. Points numbered on FIRST VISIT row-major, base tri by base tri. Ups `[(r+1,c+1),(r+1,c),(r,c)]` row-major, then downs (r>=1) `[(r,c+1),(r+1,c+1),(r,c)]`.
- Edge pts = slerp (exact). Interior (f>=4) true law unknown: coefficients in basis (apex,v1,v0) are symmetric, f=6 (2,1) = .635/.1825/.1825 vs linear .667/.167/.167.
## Other
- polySoup: one prim, points unchanged, vertices = unique points by first appearance over prims (box 1 5 4 0 2 6 3 7).
- nurbs circle (12 CV closed): poly pts = 144*lodu+12, divu irrelevant, uniform param.
