# parms-scatter: harvested Houdini 20.5 parameter templates  [H]

[H] = read from Houdini 20.5.487 on-disk install (no hython). Columns straight from embedded dialog-script (DS). `range` `N!` = hard-clamped bound (min/max locked); no `!` = UI slider hint only. `export all` = parm exported to VEX/ch(). menu `tok=Label`. `cond` = disablewhen/hidewhen.

## scatter  — SOP_Scatter2.C
- source: `libHoudiniOP2.dylib:43406793` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=af5d679f95e1f5ed · 9178 bytes
- harvested parm templates: **43** · help doc entries (`sop/scatter.txt`): **30**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 |  | `group` | Group | string |  |  |  |  |  |
| 2 | Options | `generateby` | Generate | ordinal | 0 |  | bydensity=By Density, countperprimitive=Count per Primitive, texturespace=In Texture Space |  | GenerateBy |
| 3 | Options | `indepvoxel` | Scatter on Voxels Independently | toggle | 0 |  |  | hide: { generateby != bydensity } | IndepVoxel |
| 4 | Options | `forcetotal` | Force Total Count | toggle | 1 |  |  | disa: { indepvoxel == 1 }; hide: { generateby != bydensity } | ForceTotal, joinnext, nolabel |
| 5 | Options | `npts` | Force Total Count | intlog | 1000 | 1 100000 |  | disa: { forcetotal == 0 } { indepvoxel == 1 }; hide: { generateby != bydensity } |  |
| 6 | Options | `densityscale` | Density Scale | log | 10 | 0.1 10000 |  | disa: { generateby == bydensity forcetotal == 1 indepvoxel == 0 }; hide: { generateby == countperprimitive } | DensityScale, tags:units=m-2 |
| 7 | Options | `usedensityattrib` | Use Density Attribute | toggle | 0 |  |  | hide: { generateby != bydensity } | UseDensityAttrib, joinnext, nolabel |
| 8 | Options | `densityattrib` | Density Attribute | string | density |  |  | disa: { usedensityattrib == 0 }; hide: { generateby != bydensity } | DensityAttrib |
| 9 | Options | `useareaattrib` | Override Prim Area | toggle | 0 |  |  | hide: { generateby != bydensity } | UseAreaAttrib, joinnext, nolabel |
| 10 | Options | `areaattrib` | Override Prim Area | string | area |  |  | disa: { useareaattrib == 0 }; hide: { generateby != bydensity } | AreaAttrib |
| 11 | Options | `useareaforvolumes` | Use Area for 2D Volumes | toggle | 0 |  |  | disa: { forcetotal == 1 indepvoxel == 0 }; hide: { generateby != bydensity } | UseAreaForVolumes |
| 12 | Options | `usedensitytexture` | Use Density Texture | toggle | 0 |  |  | hide: { generateby != texturespace } | UseDensityTexture, joinnext, nolabel |
| 13 | Options | `densitytexture` | Density Texture | file | default.pic |  |  | disa: { usedensitytexture == 0 }; hide: { generateby != texturespace } | DensityTexture |
| 14 | Options | `uvattrib` | UV Attribute | string | uv |  |  | hide: { generateby != texturespace } | UVAttrib, tags:sidefx::attrib_access=read |
| 15 | Options | `primcountattrib` | Count per Prim Attribute | string | count |  |  | hide: { generateby != countperprimitive } | CountPerPrimAttrib |
| 16 | Options | `useemergencylimit` | Use Max Points | toggle | 1 |  |  |  | UseEmergencyLimit, joinnext, nolabel |
| 17 | Options | `emergencylimit` | Max Point Limit | intlog | 1e+06 | 1000 1e+07 |  | disa: { useemergencylimit == 0 } | EmergencyLimit |
| 18 | Options | `seed` | Global Seed | float | 0 | 0 100 |  |  | tags:sidefx::slider=snap_to_int |
| 19 | Options | `overrideprimseed` | Override Primitive Seed | toggle | 0 |  |  | hide: { generateby == texturespace } | OverridePrimSeed, joinnext, nolabel |
| 20 | Options | `primseedattrib` | Primitive Seed Attribute | string | primid |  |  | disa: { overrideprimseed == 0 }; hide: { generateby == texturespace } | PrimSeedAttrib |
| 21 | Options | `randomizeorder` | Randomize Point Order | toggle | 1 |  |  |  | RandomizeOrder |
| 22 | Options | `relaxpoints` | Relax Points | toggle | 1 |  |  |  | RelaxPoints, joinnext, nolabel |
| 23 | Options | `relaxiterations` | Relax Iterations | integer | 10 | 0! 100 |  | disa: { relaxpoints == 0 } | RelaxIterations |
| 24 | Options | `usegeometricnormals` | Use Geometric Normals | toggle | 0 |  |  | disa: { relaxpoints == 0 } | UseGeometricNormals |
| 25 | Options | `scaleradiiby` | Scale Radii By | float | 1 | 0! 2 |  | disa: { relaxpoints == 0 useoutputradiusattrib == 0 } | ScaleRadiiBy |
| 26 | Options | `usemaxradius` | Use Max Relax Radius | toggle | 1 |  |  | disa: { relaxpoints == 0 useoutputradiusattrib == 0 } | UseMaxRadius, joinnext, nolabel |
| 27 | Options | `maxradius` | Max Relax Radius | float | 10 | 0! 100 |  | disa: { relaxpoints == 0 useoutputradiusattrib == 0 } { usemaxradius == 0 } | MaxRadius, tags:units=m-1 |
| 28 | Output Attributes | `useprimnumattrib` | Use Prim Num Attribute | toggle | 0 |  |  |  | UsePrimNumAttrib, joinnext, nolabel |
| 29 | Output Attributes | `primnumattrib` | Prim Num Attribute | string | sourceprim |  |  | disa: { useprimnumattrib == 0 } | PrimNumAttrib |
| 30 | Output Attributes | `useprimuvwattrib` | Use Prim uvw Attribute | toggle | 0 |  |  |  | UsePrimUVWAttrib, joinnext, nolabel |
| 31 | Output Attributes | `primuvwattrib` | Prim UVW Attribute | string | sourceprimuv |  |  | disa: { useprimuvwattrib == 0 } | PrimUVWAttrib |
| 32 | Output Attributes | `useoutputdensityattrib` | Use Output Density Attribute | toggle | 0 |  |  |  | UseOutputDensityAttrib, joinnext, nolabel |
| 33 | Output Attributes | `outputdensityattrib` | Output Density Attribute | string | density |  |  | disa: { useoutputdensityattrib == 0 } | OutputDensityAttrib |
| 34 | Output Attributes | `useoutputradiusattrib` | Use Output Radius Attribute | toggle | 0 |  |  |  | UseOutputRadiusAttrib, joinnext, nolabel |
| 35 | Output Attributes | `outputradiusattrib` | Output Radius Attribute | string | pscale |  |  | disa: { useoutputradiusattrib == 0 } | OutputRadiusAttrib |
| 36 | Output Attributes | `useoutputidattrib` | Use Output ID Attribute | toggle | 0 |  |  |  | UseOutputIdAttrib, joinnext, nolabel |
| 37 | Output Attributes | `outputidattrib` | Output ID Attribute | string | id |  |  | disa: { useoutputidattrib == 0 } | OutputIdAttrib |
| 38 | Output Attributes | `radiusintexturespace` | Output Radius and Density in Texture Space | toggle | 0 |  |  | disa: { generateby != texturespace } { useoutputradiusattrib == 0 useoutputdensityattrib == 0 } | RadiusInTextureSpace |
| 39 | Output Attributes | `pointattribs` | Point Attributes | string | * |  |  |  | PointAttribs |
| 40 | Output Attributes | `vertattribs` | Vertex Attributes | string | N uv* |  |  |  | VertAttribs |
| 41 | Output Attributes | `primattribs` | Primitive Attributes | string |  |  |  |  | PrimAttribs |
| 42 | Output Attributes | `detailattribs` | Detail Attributes | string |  |  |  |  | DetailAttribs |
| 43 | Output Attributes | `detailattribsasdetail` | Detail as Detail | string |  |  |  |  | DetailAttribsAsDetail |

doc entries: Group; Generate; Scatter on Voxels Independently; Force Total Count; Density Scale; Density Attribute; Override Prim Area; Use Area for 2D Volumes; Density Texture; UV Attribute; Count per Prim Attribute; Max Point Limit; Global Seed; Primitive Seed Attribute; Randomize Point Order; Relax Iterations; Use Geometric Normals; Scale Radii By; Max Relax Radius; Prim Num Attribute; Prim UVW Attribute; Output Density Attribute; Output Radius Attribute; Output ID Attribute; Output Radius and Density in Texture Space; Point Attributes; Vertex Attributes; Primitive Attributes; Detail Attributes; Detail as Detail
