# Behaviour: Box SOP (`box`)

feature ids: sop-box · observed_in: Houdini 20.5.487 (INSTALLED, **not cooked**; see §Provenance)

## Provenance (READ FIRST)
- hython BLOCKED at spec time: `No licenses could be found to run this application` (sesinetd up on :1715, hython refuses; reproduced 4×, 12:05–12:25 CEST 2026-10-07). → ZERO values below are hython-observed.
- Tags: `[D]` = from offline help shipped with the install (`houdini/help/nodes.zip` → `sop/box.txt`, same text as sidefx.com/docs/houdini/nodes/sop/box.html) · `[R]` = recalled from prior Houdini knowledge, UNVERIFIED · `[?]` = unknown, needs hython.
- Re-probe recipe: `spec/behaviour/PROBE.md`.

## Description
[D] "Creates a cube or six-sided rectangular box." Tags: create, model. With geometry wired to input 0 it instead builds the bounding box of that geometry (same as Bound SOP); usable as a Lattice cage. Without input: size/center/rotate/scale params drive the box.

## Params (doc-labelled; 13 UI params; internal names/defaults `[R]`)
| name `[R]` | type | default `[R]` | range | menu | meaning `[D]` |
|---|---|---|---|---|---|
| type | menu | Polygon | – | Polygon, Polygon Mesh, Mesh, NURBS, Bezier, Points, Primitive | primitive type. Mesh=single mesh prim w/ natural UV; Points=points only (same order as Polygon w/ divisions else as Polygon Mesh); Primitive=hexahedra in dense lattice |
| surftype | menu | Quadrilaterals | – | Rows, Columns, Rows&Cols, Triangles, Quadrilaterals, Alternating Triangles, Reverse Triangles | connectivity; enabled when type≠Polygon; Triangles/Quads ignored for NURBS/Bezier; Rows/Cols give polylines/curves |
| consolidatepts | toggle | on | – | – | weld face corner points (8 pts total) |
| size (sizex,sizey,sizez) | float3 | 1,1,1 | free | – | extent per axis |
| t (tx,ty,tz) | float3 | 0,0,0 | free | – | box center |
| r (rx,ry,rz) | float3 | 0,0,0 | free | – | rotation about center, degrees |
| scale | float | 1 | free | – | uniform scale |
| divrate | int3 | 2,2,2 `[R]` | ≥2 `[?]` | – | divisions per axis; only Polygon Mesh / Mesh |
| orderrate | int3 | 4,4,4 `[R]` | – | – | spline order per axis (NURBS/Bezier) |
| dodivs + divs | toggle + int3 | off; 3,3,3 `[R]` | ≥2 `[?]` | – | lattice mode: pts per axis; Polygon/Points/Primitive types |
| rebar `[R]` | toggle | off | – | – | enforcement bars: diagonals in lattice; Polygon + divisions on |
| (oriented bbox) | toggle | off | – | – | input-driven: orient bbox to input's principal axes; point-based, fails for sphere prims/packed |
| vertexnormals `[R]` | toggle | off | – | – | add vertex normals `N` (cusped look) |

## Inputs
1 optional: "Bounding source" `[D]`. 0 inputs → generator; ≥1 → bbox of input.

## Outputs
- Default cook (Polygon, divs off, consolidate on): 8 points, 6 quad prims, 24 vertices `[R]`; extent [-0.5,0.5]³ `[R]`. Attr lists (`pointAttribs/primAttribs/vertexAttribs/primGroups`) `[?]` — suspicion: point `P` (+`N` iff vertexnormals) , no groups. (Planned probe.)
- Points type: points only, 0 prims `[D]`.

## Invariants
- [R] topology independent of size/center/rotate/scale (transform only moves P).
- [R] Polygon default: closed manifold, every edge shared by exactly 2 prims, outward winding.
- [D] Point order of Points type == Polygon type w/ divisions, else == Polygon Mesh order.
- [D] Rows/Cols connectivity → open polylines, no surface.

## Edge cases (NOT observed)
| case | expected `[?]` | fixture |
|---|---|---|
| default | 8pt/6prim/24vtx | `box-default` |
| negative/zero size (e.g. size=(−1,0,2)) | [?] hython: does flat/inverted box cook, flip winding? | `box-edge1` |
| input with 0 points | [?] bbox of empty geo (degenerate at origin? error?) | `box-edge2` |

## Fixtures
`box-default`, `box-edge1` (planned; none produced — no oracle available) → `fixtures/MANIFEST.toml` untouched.

## Open questions
- [ ] [?] exact parm tokens, default int divisions, attr lists, edge cases → rerun PROBE.md under licensed hython.
