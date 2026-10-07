# CORRECTIONS: where harvest [H] contradicts wave1/spec [R]/[D]

Base compared: `wave1/spec:spec/behaviour/sop-*.md` (commit 9cad42f). [H] = embedded dialog scripts of Houdini 20.5.487 (`libHoudiniOP2.dylib`, `OPlibSop.hda`), see `parms-<sop>.md` for the row + byte offset. Spec tags: `[R]` recalled (UNVERIFIED), `[D]` offline help text. **No [D] cell contradicted by [H] on its own wording**; every contradiction below hits a `[R]`/`[?]` cell (names/defaults/ranges) or a [D] help *summary* that the DS refines. Help text (`nodes.zip`, built 2024-05-28) is OLDER than the binaries (2025-01-23): see DOC-DRIFT.
Legend: spec → harvest. `N!` = hard clamp in DS.

## box (`parms-box.md`)
| # | spec cell | spec says | harvest [H] |
|---|---|---|---|
| B1 | `divrate` default | 2,2,2 [R] | **4,4,4** |
| B2 | `divrate` range | ≥2 [?] | **min 1! (hard), slider max 50** (int3, export all) |
| B3 | `divs` (dodivs) default/range | 3,3,3 [R]; ≥2 [?] | default 3,3,3 ✓; range **1!–50** (min 1 not 2) |
| B4 | oriented-bbox token | unnamed | **`orientedbbox`** toggle, default off, label "Oriented Bounding Box (points only)" |
| B5 | param count | 13 UI | **14 templates** (doc merges `dodivs`+`divs` into one entry) |
Confirmed ✓: `type` menu tokens `poly polymesh mesh nurbs bezier points prim` default poly; `surftype` default quads (rows cols rowcol triangles quads alttriangles revtriangles); `consolidatepts` on; `size` 1,1,1; `t` 0; `r` 0; `scale` 1; `orderrate` 4,4,4 (range 2!–11!); `rebar` off; `vertexnormals` off.

## grid (`parms-grid.md`)
| # | spec | harvest [H] |
|---|---|---|
| G1 | `type` menu "Polygon, Mesh, NURBS, Bezier, Points, …[?]" | exactly 6: `poly mesh nurbs bezier points polysoup` (**Polygon Soup** is the missing one; NO polymesh) |
| G2 | `rows`/`cols` range ≥2 [?] | **1! hard min**, slider max 50; both export all |
| G3 | `orderu/orderv` range 2..11 [?] | 2!–11! hard both; **disabled unless type is nurbs/bezier** (cond `type != nurbs type != bezier`) |
| G4 | `interpu/v` default on [R] | on ✓ but **disabled unless type==nurbs** |
| G5 | `surftype` | enabled when type≠Polygon (help) | DS: **disabled only when `type == points`** (cond) — help and DS differ; trust DS |
Confirmed ✓: `orient` menu xy/yz/zx default **zx**; `size` 10,10 (vector2); `t`,`r`; rows=cols=10.

## sphere (`parms-sphere.md`)
| # | spec | harvest [H] |
|---|---|---|
| S1 | `accurate` default off [R] | **on** |
| S2 | `upoles` [R] | token is **`upole`** (default off); disabled unless type is polymesh/mesh |
| S3 | `upole/ wrap [R]` "U wrap" param | **no wrap parm exists in DS** (help lists "U Wrap"; DS has 16 templates, help 17) → DOC-DRIFT |
| S4 | `rows`,`cols` range ≥? | **3! hard min** both, slider max 50; disabled for prim/poly/polysoup and for nurbs/bezier when `imperfect==0` |
| S5 | `type` menu (no polysoup) | adds `polysoup` (Polygon Soup); `surftype` adds `revtriangles` |
Confirmed ✓: type default **prim**; `rad` 1,1,1; `scale`; `orient` default y (x/y/z); `freq` default 2 range 1!–10; `imperfect` on; `triangularpoles` on; orders 4 (2!–11!). `rad`,`t`,`scale` disabled when `hasinput(0)==1`; `accurate` disabled when no input.

## transform (`parms-transform.md`)
| # | spec token/cell | harvest [H] |
|---|---|---|
| T1 | `recomputenorms` | **`updatenmls`** (default off) |
| T2 | `recomputeaffectednorms` default [?] | **`updateaffectednmls`** default **on** (disabled unless updatenmls) |
| T3 | `preservelength` default [?] | **`vlength`** default **on** |
| T4 | `invert` | **`invertxform`** |
| T5 | `outputattrib` toggle+string | split: **`addattrib`** toggle (off) + **`outputattrib`** string default `xform` |
| T6 | `combine` menu [?] | **`outputmerge`** ordinal tokens `replace pre post`, default **post** (Post-Multiply) |
| T7 | `grouptype` "points/prims/edges/vertices" | menu `guess breakpoints edges points prims` — **breakpoints yes, vertices NO** |
| T8 | `shear` float3 / `s` | `shear` type float[3] (not vector) range 0–10; `s` vector default 1,1,1 |
Extra undocumented-in-spec: buttons `movecentroid`, `movepivot` (token only; actions). Confirmed ✓: `xOrd` default srt, `rOrd` default xyz, `attribs` default `*`, `p`/`pr`, `prexform_*` family names.

## merge (`parms-merge.md`)
| # | spec | harvest |
|---|---|---|
| M1 | "Recalled [R]: group, grouptype, mergetype-style options" | **No dialog script found** in any `libHoudini*.dylib` nor `OPlibSop.hda`; help `sop/merge.txt` has no `@parameters`. Weak evidence the 20.5 Merge has NO UI parms (compiled `SOP_MergeParms` exists, template not recoverable) → recalled params unsupported; stays [?] but [R] list likely wrong. |
Confirmed ✓ [D]: 9999 inputs max; bypass exports first input only.

## scatter (`parms-scatter.md`) — type `scatter`, impl `SOP_Scatter2.C`
| # | spec | harvest [H] |
|---|---|---|
| C1 | `npts` default 100 | **1000** (intlog 1–100000, enabled only when forcetotal on) |
| C2 | `relaxpoints` "Relax Iterations" default 0 | `relaxpoints` is a **toggle, default ON**; iterations = **`relaxiterations`** int default **10** (0!–100) |
| C3 | `seed` | float default 0, slider 0–100 |
| C4 | `generateby` values "+[?] others" | exactly 3: `bydensity countperprimitive texturespace`; default bydensity |
| C5 | Force Total Count default (unlisted) | `forcetotal` default **on**; densityscale (10, log 0.1–10000) disabled when forcetotal on |
| C6 | output attrs "P; N from source normals [R]" | transfer is pattern-driven: `pointattribs` default `*`, **`vertattribs` default `N uv*`**, prim/detail attribs empty → N arrives via vertex pattern |
| C7 | count-per-prim attr "–" | `primcountattrib` default `count`; `primseedattrib` default `primid`; `densityattrib` default `density`; `areaattrib` `area` |
| C8 | max point limit | `useemergencylimit` default **on**, `emergencylimit` default **1e6** |
Confirmed ✓: `densityscale` default 10; `randomizeorder` on; `group`. Relax tuning defaults: usegeometricnormals off, scaleradiiby 1, usemaxradius on, maxradius 10.

## copytopoints (`parms-copytopoints.md`) — v2 = `SOP_CopyToPoints-2.0.C`
| # | spec | harvest [H] |
|---|---|---|
| P1 | `pieceattrib` string | **`useidattrib`** toggle (default off) + **`idattrib`** string default `name` |
| P2 | `pivot` default Origin [R] | **centroid** (menu origin/centroid) |
| P3 | `resettargetattribs` toggle default on [?] | **button** (action, no state) |
| P4 | multiparm `Apply to (Point/Prim/Vertex/Detail)` | menu `points verts prims` — **NO detail** |
| P5 | `by (Copy/Add/Mul/…)` | menu `copy none mult add sub` ("Nothing" exists; token `mult`) |
| P6 | 13 doc params | adds undocumented-in-spec **`useimplicitn`** toggle (default on; disabled when transform off); multiparm per-entry `useapply#` default 1 |
Confirmed ✓: `sourcegroup`, `sourcegrouptype` (guess/prims/points), `targetgroup`, `pack` off, `viewportlod` default full, `transform` on. v1 `copytopoints` (legacy `SOP_CopyToPoints.C`, 21 templates) coexists — spec must pin which.

## wrangle (`parms-wrangle.md`) — `attribwrangle` HDA
| # | spec token/cell | harvest [H] |
|---|---|---|
| W1 | `bindings` = explicit new-attr declarations | `bindings` = **multiparm** (`bindname#`,`bindparm#`: attr→channel parm bindings); "Attributes to Create" = **`exportlist`** default `*` |
| W2 | `exportpath` | **`vex_cwdpath`** (oppath) default `.` |
| W3 | `exports` "Export Parameters" off | **`vex_outputmask`** string default `*` |
| W4 | `updatenorms` default on [?] | **`vex_updatenmls`** default **off** |
| W5 | `matchattrib` / `inplace` / `outputgroup` | **`vex_matchattrib`** default `id`; **`vex_inplace`** off; **`vex_selectiongroup`** |
| W6 | `class` menu "detail/prims/points/vertices/numbers [R]" | tokens `detail primitive point vertex number`, default **point**; number-mode adds `vex_numcount`=10, `vex_threadjobsize`=1024 |
| W7 | (absent) | extras: `vex_strict` off, `vex_precision` auto/32/64, `snippet`; `grouptype` menu guess,vertices,edges,points,prims |
Confirmed ✓: `autobind` on, `groupautobind` on, `groupbindings` multiparm (`bindgroupname#`,`bindgroupparm#`), 4 inputs.

## noise = attribnoise::2.0 (`parms-attribnoise.md`; v1 `attribnoise` also harvested, 41 templates)
| # | spec | harvest [H] |
|---|---|---|
| N1 | target attribute default **P** | `attribs` default **`Cd`**; `displace` toggle (off) + `displaceattrib` default `N` for vector-displacement mode |
| N2 | Operation "Set Initial / Set Always / Add / …" | menu `set add sub mult min max` — **no Set-Initial/Set-Always split**; default **add** |
| N3 | Range Values: Positive/Zero Centered/Negative (3) | **6**: `positive zcentered custom(Min/Max) negative minplusrange midplusminusrange`; default positive |
| N4 | "Offset (2×: translate / seed)" | `offset` float (+ `offsetv` vector via per-component strip `offsettype`), default 0 |
| N5 | "Output Raw Value" off [implied] / Make Unit Length | `outputraw` default **on**; `normalize` off; `updatenmls` default on |
| N6 | basis list "Perlin Cloud/Simplex Cloud/… [?]" | `basis` default **simplex**; menu tokens incl. `value_fast sparse alligator perlin flow simplex worleyFA… pcloud scloud fscloud` (full list in table) |
Confirmed ✓: `animated` off + `pulseduration`; `locationattrib` default `P`; `elementsize` default 1; fractal menu `none fBm mfT hmfT` default fBm, `oct` 3, `lac` 2.01234, `rough` 0.5; blend (`doblend`,`blendweight` 1,`blendmode` value/attrib).

## DOC-DRIFT (help vs binary)
- `nodes.zip` mtime 2024-05-28; dylibs/hda 2025-01-23 (build 20.5.487 per spec). Help may describe an older build: sphere "U Wrap" (S3), box doc count 13 vs 14 templates (B5), grid surftype wording (G5).
- Rule for spec authors: DS wins over help for tokens/defaults/ranges/enablement; help wins for prose semantics.
