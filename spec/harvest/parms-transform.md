# parms-transform: harvested Houdini 20.5 parameter templates  [H]

[H] = read from Houdini 20.5.487 on-disk install (no hython). Columns straight from embedded dialog-script (DS). `range` `N!` = hard-clamped bound (min/max locked); no `!` = UI slider hint only. `export all` = parm exported to VEX/ch(). menu `tok=Label`. `cond` = disablewhen/hidewhen.

## xform  — SOP_Transform.C
- source: `libHoudiniOP2.dylib:43571261` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=2b01ffe7e3f008a2 · 4853 bytes
- harvested parm templates: **27** · help doc entries (`sop/xform.txt`): **24**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 |  | `group` | Group | string |  |  |  |  |  |
| 2 |  | `grouptype` | Group Type | ordinal | guess |  | guess=Guess from Group, breakpoints=Breakpoints, edges=Edges, points=Points, prims=Primitives |  |  |
| 3 |  | `xOrd` | Transform Order | ordinal | srt |  | srt=Scale Rot Trans, str=Scale Trans Rot, rst=Rot Scale Trans, rts=Rot Trans Scale, tsr=Trans Scale Rot, trs=Trans Rot Scale |  | joinnext |
| 4 |  | `rOrd` | Rotate Order | ordinal | xyz |  | xyz=Rx Ry Rz, xzy=Rx Rz Ry, yxz=Ry Rx Rz, yzx=Ry Rz Rx, zxy=Rz Rx Ry, zyx=Rz Ry Rx |  | nolabel |
| 5 |  | `t` | Translate | vector[3] | 0 0 0 | -1 1 |  |  |  |
| 6 |  | `r` | Rotate | vector[3] | 0 0 0 | 0 360 |  |  |  |
| 7 |  | `s` | Scale | vector[3] | 1 1 1 | -1 1 |  |  |  |
| 8 |  | `shear` | Shear | float[3] | 0 0 0 | 0 10 |  |  |  |
| 9 |  | `scale` | Uniform Scale | float | 1 | 0 10 |  |  |  |
| 10 | Pivot Transform grouptag | `p` | Pivot Translate | vector[3] | 0 0 0 | -1 1 |  |  |  |
| 11 | Pivot Transform grouptag | `pr` | Pivot Rotate | vector[3] | 0 0 0 | 0 360 |  |  |  |
| 12 | Pre-Transform grouptag | `prexform_xOrd` | Pre-Transform Order | ordinal | srt |  | srt=Scale Rot Trans, str=Scale Trans Rot, rst=Rot Scale Trans, rts=Rot Trans Scale, tsr=Trans Scale Rot, trs=Trans Rot Scale |  | joinnext |
| 13 | Pre-Transform grouptag | `prexform_rOrd` | Pre-Rotate Order | ordinal | xyz |  | xyz=Rx Ry Rz, xzy=Rx Rz Ry, yxz=Ry Rx Rz, yzx=Ry Rz Rx, zxy=Rz Rx Ry, zyx=Rz Ry Rx |  | nolabel |
| 14 | Pre-Transform grouptag | `prexform_t` | Pre-Translate | vector[3] | 0 0 0 |  |  |  |  |
| 15 | Pre-Transform grouptag | `prexform_r` | Pre-Rotate | vector[3] | 0 0 0 |  |  |  |  |
| 16 | Pre-Transform grouptag | `prexform_s` | Pre-Scale | vector[3] | 1 1 1 |  |  |  |  |
| 17 | Pre-Transform grouptag | `prexform_shear` | Pre-Shear | float[3] | 0 0 0 | 0 10 |  |  |  |
| 18 |  | `movecentroid` | Move Centroid to Origin | button | 0 | 0 1 |  |  | joinnext |
| 19 |  | `movepivot` | Move Pivot to Centroid | button | 0 | 0 1 |  |  |  |
| 20 |  | `attribs` | Attributes | string | * |  |  |  |  |
| 21 |  | `updatenmls` | Recompute Point Normals | toggle | off |  |  |  |  |
| 22 |  | `updateaffectednmls` | Recompute Affected Normals | toggle | on |  |  | disa: { updatenmls == 1 } |  |
| 23 |  | `vlength` | Preserve Normal Length | toggle | on |  |  | disa: { updatenmls == 1 } |  |
| 24 |  | `invertxform` | Invert Transformation | toggle | off |  |  |  |  |
| 25 |  | `addattrib` | Output Attribute | toggle | off |  |  |  | joinnext, nolabel |
| 26 |  | `outputattrib` | Output Attribute | string | xform |  |  | disa: { addattrib == 0 } | joinnext |
| 27 |  | `outputmerge` | Combine | ordinal | post |  | replace=Replace Existing, pre=Pre-Multiply, post=Post-Multiply | disa: { addattrib == 0 } | nolabel |

doc entries: Group; Group Type; Transform Order; Rotate Order; Translate; Rotate; Scale; Shear; Uniform Scale; Pivot Translate; Pivot Rotation; Pre-Transform Order; Ptr-Rotate Order; Pre-Translate; Pre-Rotate; Pre-Scale; Pre-Shear; Attributes; Recompute Point Normals; Recompute Affected Normals; Preserve Normal Length; Invert Transformation; Output Attribute; Combine
