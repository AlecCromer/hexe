# parms-sphere: harvested Houdini 20.5 parameter templates  [H]

[H] = read from Houdini 20.5.487 on-disk install (no hython). Columns straight from embedded dialog-script (DS). `range` `N!` = hard-clamped bound (min/max locked); no `!` = UI slider hint only. `export all` = parm exported to VEX/ch(). menu `tok=Label`. `cond` = disablewhen/hidewhen.

## sphere  — SOP_Sphere.C
- source: `libHoudiniOP2.dylib:43449449` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=f7dfe916c34b65d0 · 3023 bytes
- harvested parm templates: **16** · help doc entries (`sop/sphere.txt`): **17**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 |  | `type` | Primitive Type | ordinal | prim |  | prim=Primitive, poly=Polygon, polymesh=Polygon Mesh, mesh=Mesh, nurbs=NURBS, bezier=Bezier, polysoup=Polygon Soup |  | export all |
| 2 |  | `surftype` | Connectivity | ordinal | quads |  | rows=Rows, cols=Columns, rowcol=Rows and Columns, triangles=Triangles, quads=Quadrilaterals, alttriangles=Alternating Triangles, revtriangles=Reverse Triangles | disa: { type == prim } { type == poly } { type == polysoup } |  |
| 3 |  | `rad` | Radius | vector[3] | 1 1 1 | -1 1 |  | disa: { hasinput(0) == 1 } | export all |
| 4 |  | `t` | Center | vector[3] | 0 0 0 | -1 1 |  | disa: { hasinput(0) == 1 } | export all |
| 5 |  | `r` | Rotate | vector[3] | 0 0 0 | 0 360 |  |  |  |
| 6 |  | `scale` | Uniform Scale | float | 1 | 0 10 |  | disa: { hasinput(0) == 1 } |  |
| 7 |  | `orient` | Orientation | ordinal | y | 0 1 | x=X Axis, y=Y Axis, z=Z Axis |  |  |
| 8 |  | `freq` | Frequency | integer | 2 | 1! 10 |  | disa: { type != poly type != polysoup } | export all |
| 9 |  | `rows` | Rows | integer | 13 | 3! 50 |  | disa: { type == prim } { type == poly } { type == polysoup } { type == nurbs imperfect == 0 } { type == bezier imperfect == 0 } | export all |
| 10 |  | `cols` | Columns | integer | 24 | 3! 50 |  | disa: { type == prim } { type == poly } { type == polysoup } { type == nurbs imperfect == 0 } { type == bezier imperfect == 0 } | export all |
| 11 |  | `orderu` | U Order | integer | 4 | 2! 11! |  | disa: { type != bezier type != nurbs } |  |
| 12 |  | `orderv` | V Order | integer | 4 | 2! 11! |  | disa: { type != bezier type != nurbs } |  |
| 13 |  | `imperfect` | Imperfect | toggle | on | 0 1 |  | disa: { type != bezier type != nurbs } |  |
| 14 |  | `upole` | Unique Points per Pole | toggle | off |  |  | disa: { type != polymesh type != mesh } |  |
| 15 |  | `accurate` | Accurate Bounds | toggle | on |  |  | disa: { hasinput(0) == 0 } |  |
| 16 |  | `triangularpoles` | Triangular Poles | toggle | on |  |  | disa: { type != polymesh } { type == polymesh surftype != quads surftype != rows surftype != cols surftype != rowcol } |  |

doc entries: Primitive Type; Connectivity; Radius; Center; Rotation; Uniform Scale; Orientation; Frequency; Rows; Columns; U Order; V Order; U Wrap; Imperfect; Unique Points per Pole; Accurate Bounds; Triangular Poles
