# parms-wrangle: harvested Houdini 20.5 parameter templates  [H]

[H] = read from Houdini 20.5.487 on-disk install (no hython). Columns straight from embedded dialog-script (DS). `range` `N!` = hard-clamped bound (min/max locked); no `!` = UI slider hint only. `export all` = parm exported to VEX/ch(). menu `tok=Label`. `cond` = disablewhen/hidewhen.

## attribwrangle  — attribwrangle HDA DialogScript
- source: `OPlibSop.hda:1441556 (len 7973; hda-index base 12761+off 1428544)` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=7b4dc7fe3995647d · 7973 bytes
- harvested parm templates: **23** · help doc entries (`sop/attribwrangle.txt`): **16**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 | Code | `group` | Group | string |  | 0 1 | (toggle)[script menu] |  |  |
| 2 | Code | `grouptype` | Group Type | ordinal | guess | 0 1 | guess=Guess from Group, vertices=Vertices, edges=Edges, points=Points, prims=Primitives |  |  |
| 3 | Code | `class` | Run Over | ordinal | point | 0 1 | detail=Detail (only once), primitive=Primitives, point=Points, vertex=Vertices, number=Numbers |  |  |
| 4 | Code | `vex_numcount` | Number Count | integer | 10 | 0! 10000 |  | hide: { class != number } |  |
| 5 | Code | `vex_threadjobsize` | Thread Job Size | integer | 1024 | 1! 10000 |  | hide: { class != number } |  |
| 6 | Code | `snippet` | VEXpression | string |  | 0 1 |  |  | export all, tags:editor=1;editorlang=VEX;editorlines=8-30 |
| 7 | Code | `exportlist` | Attributes to Create | string | * | 0 1 |  |  |  |
| 8 | Code | `vex_strict` | Enforce Prototypes | toggle | off | 0 1 |  |  | tags:script_callback= |
| 9 | Bindings | `autobind` | Autobind by Name | toggle | on | 0 1 |  |  |  |
| 10 | Bindings | `bindings` | Number of Bindings | multiparm | 0 |  |  |  |  |
| 11 | Bindings/multiparm:bindings | `bindname#` | Attribute Name | string |  | 0 1 |  |  |  |
| 12 | Bindings/multiparm:bindings | `bindparm#` | VEX Parameter | string |  | 0 1 |  |  |  |
| 13 | Bindings | `groupautobind` | Autobind Groups by Name | toggle | on | 0 1 |  |  |  |
| 14 | Bindings | `groupbindings` | Group Bindings | multiparm | 0 |  |  |  |  |
| 15 | Bindings/multiparm:groupbindings | `bindgroupname#` | Group Name | string |  | 0 1 |  |  |  |
| 16 | Bindings/multiparm:groupbindings | `bindgroupparm#` | VEX Parameter | string |  | 0 1 |  |  |  |
| 17 | Bindings | `vex_cwdpath` | Evaluation Node Path | oppath | . | 0 1 |  |  | tags:oprelative=. |
| 18 | Bindings | `vex_outputmask` | Export Parameters | string | * | 0 1 |  |  |  |
| 19 | Bindings | `vex_updatenmls` | Update Normals If Displaced | toggle | off | 0 1 |  |  | tags:script_callback= |
| 20 | Bindings | `vex_matchattrib` | Attribute to Match | string | id | 0 1 |  |  |  |
| 21 | Bindings | `vex_inplace` | Compute Results In Place | toggle | off | 0 1 |  |  |  |
| 22 | Bindings | `vex_selectiongroup` | Output Selection Group | string |  |  |  |  |  |
| 23 | Bindings | `vex_precision` | VEX Precision | string | auto |  | auto=Auto, 32=32-bit, 64=64-bit |  |  |

doc entries: Group; Group Type; :include _run_over; :include pointwrangle#snippet; Attributes to Create; :include /vex/_enforce_prototypes; Autobind by Name; Autobind Groups by Name; Group Bindings; Evaluation Node Path; Export Parameters; Update Normals If Displaced; Attribute To Match; Compute Results In Place; Output Selection Group; :include /nodes/sop/attribvop#vex_precision
