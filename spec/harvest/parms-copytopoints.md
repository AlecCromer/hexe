# parms-copytopoints: harvested Houdini 20.5 parameter templates  [H]

[H] = read from Houdini 20.5.487 on-disk install (no hython). Columns straight from embedded dialog-script (DS). `range` `N!` = hard-clamped bound (min/max locked); no `!` = UI slider hint only. `export all` = parm exported to VEX/ch(). menu `tok=Label`. `cond` = disablewhen/hidewhen.

## copytopoints  — SOP_CopyToPoints-2.0.C (type copytopoints, ver 2.0)
- source: `libHoudiniOP2.dylib:42587532` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=a0b1e7282ec60f0b · 4468 bytes
- harvested parm templates: **16** · help doc entries (`sop/copytopoints.txt`): **13**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 |  | `sourcegroup` | Source Group | string |  |  |  |  | SourceGroup, tags:sop_input=0 |
| 2 |  | `sourcegrouptype` | Source Group Type | ordinal | 0 |  | guess=Guess from Group, prims=Primitives, points=Points |  | SourceGroupType |
| 3 |  | `targetgroup` | Target Points | string |  |  |  |  | TargetGroup, tags:sop_input=1 |
| 4 |  | `useidattrib` | Piece Attribute | toggle | 0 |  |  |  | UseIDAttrib, joinnext, nolabel |
| 5 |  | `idattrib` | Piece Attribute | string | name |  |  | disa: { useidattrib == 0 } | IDAttrib, tags:sop_input=1 |
| 6 |  | `pack` | Pack and Instance | toggle | off |  |  |  |  |
| 7 |  | `pivot` | Pivot Location | ordinal | centroid |  | origin=Origin, centroid=Centroid | hide: { pack == 0 } |  |
| 8 |  | `viewportlod` | Display As | ordinal | full |  | full=Full Geometry, points=Point Cloud, box=Bounding Box, centroid=Centroid, hidden=Hidden | hide: { pack == 0 } | ViewportLOD |
| 9 |  | `transform` | Transform Using Target Point Orientations | toggle | on |  |  |  |  |
| 10 |  | `useimplicitn` | Transform Using Implicit Target Point Normals If No Point N Attribute | toggle | on |  |  | disa: { transform == 0 } | UseImplicitN |
| 11 |  | `resettargetattribs` | Reset Attributes from Target | button | 0 |  |  |  |  |
| 12 |  | `targetattribs` | Attributes from Target | multiparm | 0 |  |  |  | TargetAttribs |
| 13 | multiparm:targetattribs | `useapply#` | Apply Attributes | toggle | 1 |  |  |  | joinnext, nolabel |
| 14 | multiparm:targetattribs | `applyto#` | Apply to | ordinal | 0 |  | points=Points, verts=Vertices, prims=Primitives |  | joinnext |
| 15 | multiparm:targetattribs | `applymethod#` | by | ordinal | 0 |  | copy=Copying, none=Nothing, mult=Multiplying, add=Adding, sub=Subtracting |  | joinnext |
| 16 | multiparm:targetattribs | `applyattribs#` | Attributes | string |  |  |  |  | tags:sop_input=1 |

doc entries: Source Group; Source Group Type; Target Points; Piece Attribute; Pack and Instance; Pivot Location; Display As; Transform Using Point Orientations; Reset Attributes from Target; Attributes from Target; Apply to; by; Attributes

## copytopoints v1  — SOP_CopyToPoints.C (legacy v1)
- source: `libHoudiniOP2.dylib:42583741` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=85b6ae616113a639 · 3552 bytes
- harvested parm templates: **21** · help doc entries (`sop/copytopoints-.txt`): **9**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 |  | `sourcegroup` | Source Group | string |  |  |  |  | SourceGroup, tags:sop_input=0 |
| 2 |  | `targetgroup` | Target Points | string |  |  |  |  | TargetGroup, tags:sop_input=1 |
| 3 |  | `showguide` | Show Guide Geometry | toggle | on |  |  |  |  |
| 4 |  | `pack` | Pack and Instance | toggle | off |  |  |  |  |
| 5 |  | `pivot` | Pivot Location | ordinal | centroid |  | origin=Origin, centroid=Centroid | hide: { pack == 0 } |  |
| 6 |  | `viewportlod` | Display As | ordinal | full |  | full=Full Geometry, points=Point Cloud, box=Bounding Box, centroid=Centroid, hidden=Hidden | hide: { pack == 0 } |  |
| 7 |  | `transform` | Transform Using Point Orientations | toggle | on |  |  |  |  |
| 8 |  | `doattr` | Copy Point Attributes | toggle | on |  |  |  | DoAttr |
| 9 |  | `labelparm` | Label | label[4] | Set Mult Add Sub |  |  |  |  |
| 10 |  | `setpt` | To Point | string | *,^v,^Alpha,^N |  |  | disa: { doattr == 0 } | joinnext |
| 11 |  | `mulpt` | Multiply Point | string | Alpha |  |  | disa: { doattr == 0 } | joinnext, nolabel |
| 12 |  | `addpt` | Add Point | string | v |  |  | disa: { doattr == 0 } | joinnext, nolabel |
| 13 |  | `subpt` | Subtract Point | string |  |  |  | disa: { doattr == 0 } | nolabel |
| 14 |  | `setprim` | To Primitive | string |  |  |  | disa: { doattr == 0 } | joinnext |
| 15 |  | `mulprim` | Multiply Primitive | string |  |  |  | disa: { doattr == 0 } | joinnext, nolabel |
| 16 |  | `addprim` | Add Primitive | string |  |  |  | disa: { doattr == 0 } | joinnext, nolabel |
| 17 |  | `subprim` | Subtract Primitive | string |  |  |  | disa: { doattr == 0 } | nolabel |
| 18 |  | `setvtx` | To Vertex | string |  |  |  | disa: { doattr == 0 } | joinnext |
| 19 |  | `mulvtx` | Multiply Vertex | string |  |  |  | disa: { doattr == 0 } | joinnext, nolabel |
| 20 |  | `addvtx` | Add Vertex | string |  |  |  | disa: { doattr == 0 } | joinnext, nolabel |
| 21 |  | `subvtx` | Subtract Vertex | string |  |  |  | disa: { doattr == 0 } | nolabel |

doc entries: Source Group; Target Points; Show Guide Geometry; Transform Using Point Orientations; Pack and Instance; Pivot Location; Display As; Copy Point Attributes; Attribute fields
