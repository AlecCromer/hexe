# Behaviour: Grid SOP (`grid`)

feature ids: sop-grid · observed_in: Houdini 20.5.487 (installed, not cooked)

## Provenance
Tags as in `sop-box.md`: `[D]` offline help (`sop/grid.txt`), `[R]` recalled/UNVERIFIED, `[?]` unknown. hython blocked (no license) → nothing hython-observed. Probe: `PROBE.md`.

## Description
[D] "Creates planar geometry." Mesh, Bezier, NURBS surfaces, or open-polygon lines. Tags: create, surface, model. Points type → free points at the surface positions, no prims.

## Params (12 doc-labelled)
| name `[R]` | type | default `[R]` | range | menu | meaning `[D]` |
|---|---|---|---|---|---|
| type | menu | Polygon | – | Polygon, Mesh, NURBS, Bezier, Points, … `[?]` | primitive type |
| surftype | menu | Quads | – | Rows, Columns, Rows&Cols, Triangles, Quads, Alternating Triangles, Reverse Triangles | connectivity; polygons/meshes only; Rows/Cols → open polys |
| orient | menu | ZX plane | – | XY, YZ, ZX | plane orientation (ZX = normal +Y) |
| size (sizex,sizey) | float2 | 10,10 | free | – | width/height |
| t | float3 | 0,0,0 | free | – | center |
| r | float3 | 0,0,0 | free | – | rotation about center (deg) |
| rows | int | 10 | ≥2 `[?]` | – | point rows |
| cols | int | 10 | ≥2 `[?]` | – | point columns |
| orderu / orderv | int | 4 / 4 `[R]` | 2..11 `[?]` | – | NURBS/Bezier order |
| interpu / interpv | toggle | on `[R]` | – | – | end-point interpolate |

Note [R]: rows/cols count POINTS, not cells → default 100 points, 81 quads.

## Inputs
0–1 `[?]` (docs list none; legacy bbox-from-input behaviour unconfirmed).

## Outputs
- Default: 100 points, 81 quads, 324 vertices `[R]`; extent [-5,5] in X,Z at Y=0 `[R]`. Attrs `[?]`: P; suspicion also point `N` (+Y). Groups: none `[R]`.

## Invariants
- [R] Quads: (rows−1)(cols−1) prims, rows·cols points.
- [R] Triangles: 2·(rows−1)(cols−1) prims, same point count.
- [R] Flat: all P on plane through center per orient; normals uniform.
- [D] Points type: 0 prims.

## Edge cases (NOT observed)
| case | expected | fixture |
|---|---|---|
| default | 100pt/81prim | `grid-default` |
| rows=1 or cols=1 or negative size | [?] clamp/error/empty? | `grid-edge1` |
| rows=cols=2 | 4pt/1prim | `grid-edge2` |

## Fixtures
`grid-default`, `grid-edge1` planned. wave0 `tools/oracle.py` already plans `grid rows=3 cols=3`.

## Open questions
- [ ] [?] everything marked `[?]/[R]` → PROBE.md.
