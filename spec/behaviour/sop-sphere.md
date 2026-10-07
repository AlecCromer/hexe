# Behaviour: Sphere SOP (`sphere`)

feature ids: sop-sphere · observed_in: Houdini 20.5.487 (installed, not cooked)

## Provenance
Tags: `[D]` offline help (`sop/sphere.txt`), `[R]` recalled/UNVERIFIED, `[?]` unknown. hython blocked (no license). Probe: `PROBE.md`.

## Description
[D] "Creates a sphere or ovoid surface." Spheres/ellipsoids of any geometry type (primitive, polygon, mesh, NURBS, Bezier). With an input, radius is derived from input bounds (bounding sphere; `Accurate Bounds` = slower exact). Tags: create, model, core.

## Params (17 doc-labelled)
| name `[R]` | type | default `[R]` | range | menu | meaning `[D]` |
|---|---|---|---|---|---|
| type | menu | Primitive | – | Primitive, Polygon, Polygon Mesh, Mesh, NURBS, Bezier `[R]` | geometry type. wave0 oracle uses token `poly` for Polygon |
| surftype | menu | Quads | – | Rows, Columns, Rows&Cols, Triangles, Quads, Alternating Triangles | connectivity (mesh-ish types) |
| rad (radx,rady,radz) | float3 | 1,1,1 | free | – | per-axis radii → ellipsoid |
| t | float3 | 0 | free | – | center |
| r | float3 | 0 | free | – | rotation (deg) |
| scale | float | 1 | free | – | uniform scale |
| orient | menu | Y | – | X,Y,Z | pole axis |
| freq | int | 2 | ≥1 `[?]` | – | polygon sphere subdivision frequency; disabled for non-Polygon types |
| rows | int | 13 | ≥? | – | mesh/imperfect NURBS rows (V) |
| cols | int | 24 | ≥? | – | columns (U) |
| orderu / orderv | int | 4 / 4 | 2..11 `[D]` | – | spline orders (cubic default `[D]`) |
| upole/ wrap `[R]` | toggle | – | – | – | U wrap |
| imperfect | toggle | on `[R]` | – | – | non-rational NURBS/Bezier (vs perfect rational) |
| upoles `[R]` | toggle | off | – | – | unique points per pole; off = one shared pole point |
| accurate `[R]` | toggle | off | – | – | accurate bounds (input mode) |
| triangularpoles `[R]` | toggle | on `[R]` | – | – | triangles at poles of polygon-mesh sphere |

## Inputs
0–1: optional bounds source `[D]`.

## Outputs
- Default `[R]`: Primitive type → 1 point + 1 sphere-primitive (not polygons) — counts `[?]`. Polygon type: count formula `[?]` (freq-based, not rows/cols).
- Attr lists/groups `[?]`.

## Invariants
- [R] Every P of a polygon sphere lies on the ellipsoid (|x/rx|²+|y/ry|²+|z/rz|²=1 within fp) around t.
- [R] Mesh type with shared poles: points = (rows−2)·cols + 2 `[?]`.

## Edge cases (NOT observed)
| case | expected | fixture |
|---|---|---|
| default (primitive) | 1pt/1prim `[R]` | `sphere-default` |
| radius 0 or negative component, type=poly | [?] degenerate / flipped winding | `sphere-edge1` |
| freq=0 or 1, type=poly | [?] | `sphere-edge2` |

## Fixtures
`sphere-default`, `sphere-edge1`. NB wave0 oracle uses `type=poly` for the sphere fixture, so poly-sphere topology is the P0 target; the default Primitive-type sphere is probably out of scope (no packed/prim spheres, DESIGN non-goals) `[R]`.

## Open questions
- [ ] [?] poly-sphere point/prim counts per freq; default type; pole point ordering → PROBE.md.
