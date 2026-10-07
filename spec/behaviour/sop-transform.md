# Behaviour: Transform SOP (`xform`)

feature ids: sop-transform · observed_in: Houdini 20.5.487 (installed, not cooked)

## Provenance
Tags: `[D]` offline help shipped with install (`houdini/help/nodes.zip`), `[R]` recalled/UNVERIFIED, `[?]` unknown. hython blocked (no license) → nothing hython-observed. Probe: `PROBE.md`.

## Description
[D] "Transforms the source geometry in object space using a transformation matrix." Translate/rotate/scale/shear around a pivot, with optional pre-transform; can invert; can output the matrix to a detail attribute instead of/besides moving geometry.

## Params (24 doc-labelled; names `[R]`)
| name | type | default | menu / meaning |
|---|---|---|---|
| group | string | empty=all | [D] elements to transform |
| grouptype | menu | Guess from Group | [D] points/prims/edges/vertices |
| xOrd | menu | Scale Rotate Translate | [D] Transform Order (6 perms) |
| rOrd | menu | Rx Ry Rz | [D] Rotate Order (6 perms) |
| t (tx,ty,tz) | float3 | 0 | [D] Translate |
| r (rx,ry,rz) | float3 | 0 | [D] Rotate (deg) |
| s (sx,sy,sz) | float3 | 1 | [D] Scale |
| shear (shx,shy,shz) | float3 | 0 | [D] Shear |
| scale | float | 1 | [D] Uniform Scale |
| p (px,py,pz) | float3 | 0 | [D] Pivot Translate |
| pr (prx,pry,prz) | float3 | 0 | [D] Pivot Rotation |
| prexform_xOrd, prexform_rOrd | menu | as above | [D] Pre-Transform order / rotate order |
| prexform_t, _r, _s, _shear | float3 | 0 / 0 / 1 / 0 | [D] Pre-Translate/Rotate/Scale/Shear |
| attribs | string | `*` `[?]` | [D] Attributes to transform (P, N, v, … by type info) |
| recomputenorms | toggle | off | [D] Recompute Point Normals |
| recomputeaffectednorms | toggle | `[?]` | [D] Recompute Affected Normals |
| preservelength | toggle | `[?]` | [D] Preserve Normal Length (non-uniform scale) |
| invert | toggle | off | [D] Invert Transformation |
| outputattrib | toggle+string | off | [D] Output Attribute (store matrix as detail attr) |
| combine | menu | `[?]` | [D] Combine (with existing matrix attr) |

## Inputs
1 required (geometry to transform).

## Outputs
- Same topology/prims/groups/attrs as input `[R]`. P always transformed; N (normal type), v, etc. transformed per attribute type info (points get inverse-transpose for normals) `[R]`.
- Optional detail matrix attr when output-attribute on.

## Invariants
- [R] identity params → output P bit-identical to input.
- [R] point/prim counts, prim types, group membership unchanged.
- [R] invert=on ∘ forward = identity within fp.
- [R] normals: non-uniform scale keeps N perpendicular to surface (unless attribute not typed as normal).
- [D] Transform order setting changes the composed matrix (non-commutative).

## Edge cases (NOT observed)
| case | expected | fixture |
|---|---|---|
| default params on box | unchanged | `transform-default` |
| scale 0 on an axis; singular + invert=on | [?] error vs warning vs NaN | `transform-edge1` |
| input with 0 points | [?] empty passthrough | `transform-edge2` |

## Fixtures
`transform-default`, `transform-edge1` planned.

## Open questions
- [ ] [?] attrs default, combine menu, normal handling rules → PROBE.md.
