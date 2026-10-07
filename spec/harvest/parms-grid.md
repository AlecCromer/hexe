# parms-grid: harvested Houdini 20.5 parameter templates  [H]

[H] = read from Houdini 20.5.487 on-disk install (no hython). Columns straight from embedded dialog-script (DS). `range` `N!` = hard-clamped bound (min/max locked); no `!` = UI slider hint only. `export all` = parm exported to VEX/ch(). menu `tok=Label`. `cond` = disablewhen/hidewhen.

## grid  — SOP_Grid.C
- source: `libHoudiniOP2.dylib:42859284` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=5aabdacb34c351bc · 1968 bytes
- harvested parm templates: **12** · help doc entries (`sop/grid.txt`): **12**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 |  | `type` | Primitive Type | ordinal | poly | 0 1 | poly=Polygon, mesh=Mesh, nurbs=NURBS, bezier=Bezier, points=Points, polysoup=Polygon Soup |  | export all |
| 2 |  | `surftype` | Connectivity | ordinal | quads | 0 1 | rows=Rows, cols=Columns, rowcol=Rows and Columns, triangles=Triangles, quads=Quadrilaterals, alttriangles=Alternating Triangles, revtriangles=Reverse Triangles | disa: { type == points } |  |
| 3 |  | `orient` | Orientation | ordinal | zx | 0 1 | xy=XY Plane, yz=YZ Plane, zx=ZX Plane |  | export all |
| 4 |  | `size` | Size | vector2[2] | 10 10 | -1 1 |  |  | export all |
| 5 |  | `t` | Center | vector[3] | 0 0 0 | -1 1 |  |  |  |
| 6 |  | `r` | Rotate | vector[3] | 0 0 0 | 0 360 |  |  |  |
| 7 |  | `rows` | Rows | integer | 10 | 1! 50 |  |  | export all |
| 8 |  | `cols` | Columns | integer | 10 | 1! 50 |  |  | export all |
| 9 |  | `orderu` | U Order | integer | 4 | 2! 11! |  | disa: { type != nurbs type != bezier } |  |
| 10 |  | `orderv` | V Order | integer | 4 | 2! 11! |  | disa: { type != nurbs type != bezier } |  |
| 11 |  | `interpu` | End-point interpolate in U | toggle | on | 0 1 |  | disa: { type != nurbs } |  |
| 12 |  | `interpv` | End-point interpolate in V | toggle | on | 0 1 |  | disa: { type != nurbs } |  |

doc entries: Primitive Type; Connectivity; Orientation; Size; Center; Rotate; Rows; Columns; U Order; V Order; End-point interpolate in U; End-point interpolate in V
