# Behaviour: Scatter SOP (`scatter`)

feature ids: sop-scatter · observed_in: Houdini 20.5.487 (installed, not cooked)

## Provenance
Tags: `[D]` offline help shipped with install (`houdini/help/nodes.zip`), `[R]` recalled/UNVERIFIED, `[?]` unknown. hython blocked (no license) → nothing hython-observed. Probe: `PROBE.md`.

## Description
[D] Scatters points randomly over the surface (or volume) of input geometry; density- or count-based; optional relaxation (blue-noise-ish) and attribute transfer. Deterministic given seed `[R]`; exact RNG sequence is NOT specified publicly → bit-exact parity probably impossible without oracle; target = statistical + count parity.

## Params (~35 doc-labelled; names `[R]`)
| label [D] | name [R] | default [R] | meaning |
|---|---|---|---|
| Group | group | empty | prims to scatter on |
| Generate | generateby | By Density | By Density / Count per Primitive / In Texture Space (+ `[?]` others) |
| Force Total Count | npts | 100 | total points (count mode) |
| Count per Primitive | – | – | per-prim count |
| Density Scale | densityscale | 10 | points per unit area `[R]` |
| Density Attribute | densityattrib | empty | scale density by point/prim attr |
| Override Prim Area / Use Area for 2D Volumes / Scatter on Voxels Independently | – | off | misc |
| Density Texture, UV Attribute | – | – | texture-driven density |
| Count per Prim Attribute | – | – | |
| Max Point Limit | – | – | cap |
| Global Seed | seed | 0 | RNG seed |
| Primitive Seed Attribute | – | – | per-prim seed |
| Randomize Point Order | – | on `[R]` | |
| Relax Iterations | relaxpoints | 0 | blue-noise relaxation |
| Use Geometric Normals, Scale Radii By, Max Relax Radius | – | – | relax tuning |
| Prim Num / Prim UVW / Output Density / Radius / ID Attribute | – | off | optional outputs |
| Point/Vertex/Primitive/Detail Attributes (+Detail as Detail) | – | – | transfer patterns from source |

## Inputs
1 required (surface geometry).

## Outputs
- [R] only points (0 prims) — point cloud. Attrs: P; N (from source surface normals) `[R]`; optional primnum/uv/density/pscale/id when enabled `[D]`.

## Invariants
- [R] 0 prims in output; every point on an input prim (relax may move within surface).
- [R] same params+seed+input → identical output (deterministic).
- [R] count mode: exactly npts points (if input has area).
- [R] seed change → different distribution, same count.

## Edge cases (NOT observed)
| case | expected | fixture |
|---|---|---|
| grid default, npts=100 | 100 pts on plane | `scatter-default` |
| input with 0 prims / 0 area | [?] empty + warning? | `scatter-edge1` |
| density 0 / count 0 | [?] empty | `scatter-edge2` |

## Fixtures
`scatter-default`, `scatter-edge1` planned; compare statistically (count, bbox, min-dist), not per-point.

## Open questions
- [ ] [?] RNG algorithm; default generate mode; param list → PROBE.md.
