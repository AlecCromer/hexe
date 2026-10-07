# parms-box: harvested Houdini 20.5 parameter templates  [H]

[H] = read from Houdini 20.5.487 on-disk install (no hython). Columns straight from embedded dialog-script (DS). `range` `N!` = hard-clamped bound (min/max locked); no `!` = UI slider hint only. `export all` = parm exported to VEX/ch(). menu `tok=Label`. `cond` = disablewhen/hidewhen.

## box  — SOP_Box.C
- source: `libHoudiniOP2.dylib:42504445` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=6da1540cba0b5971 · 3057 bytes
- harvested parm templates: **14** · help doc entries (`sop/box.txt`): **13**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 |  | `type` | Primitive Type | ordinal | poly |  | poly=Polygon, polymesh=Polygon Mesh, mesh=Mesh, nurbs=NURBS, bezier=Bezier, points=Points, prim=Primitive |  | export all |
| 2 |  | `surftype` | Connectivity | ordinal | quads |  | rows=Rows, cols=Columns, rowcol=Rows and Columns, triangles=Triangles, quads=Quadrilaterals, alttriangles=Alternating Triangles, revtriangles=Reverse Triangles |  | SurfType |
| 3 |  | `consolidatepts` | Consolidate Corner Points | toggle | on |  |  |  | ConsolidatePoints, export all |
| 4 |  | `size` | Size | vector[3] | 1 1 1 | -1 1 |  |  | export all |
| 5 |  | `t` | Center | vector[3] | 0 0 0 | -1 1 |  |  | Center |
| 6 |  | `r` | Rotate | vector[3] | 0 0 0 | 0 360 |  |  | Rotate |
| 7 |  | `scale` | Uniform Scale | float | 1 | 0 10 |  |  |  |
| 8 |  | `divrate` | Axis Divisions | integer[3] | 4 4 4 | 1! 50 |  |  | DivRate, export all |
| 9 |  | `orderrate` | Axis Orders | integer[3] | 4 4 4 | 2! 11! |  |  | Order |
| 10 |  | `dodivs` | Use Divisions | toggle | off |  |  |  | DoDivs, joinnext, nolabel |
| 11 |  | `divs` | Divisions | intvector[3] | 3 3 3 | 1! 50 |  |  |  |
| 12 |  | `rebar` | Enforcement Bars | toggle | off |  |  |  |  |
| 13 |  | `orientedbbox` | Oriented Bounding Box (points only) | toggle | off |  |  |  | OrientedBoundingBox |
| 14 |  | `vertexnormals` | Add Vertex Normals | toggle | off |  |  |  | VertexNormals |

doc entries: Primitive Type; Connectivity; Consolidate Corner Points; Size; Center; Rotate; Uniform Scale; Axis Divisions; Axis Orders; Divisions; Enforcement Bars; Oriented Bounding Box (points only); Add Vertex Normals
