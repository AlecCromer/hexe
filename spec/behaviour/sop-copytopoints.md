# Behaviour: Copy to Points SOP (`copytopoints`, v2 in 20.5)

feature ids: sop-copytopoints · observed_in: Houdini 20.5.487 (installed, not cooked)

## Provenance
Tags: `[D]` offline help shipped with install (`houdini/help/nodes.zip`), `[R]` recalled/UNVERIFIED, `[?]` unknown. hython blocked (no license) → nothing hython-observed. Probe: `PROBE.md`. Internal type name in 20.5 is likely `copytopoints::2.0` (docs show the pack-and-instance form) `[R]`.

## Description
[D] "Copies geometry in the first input onto the points of the second input." Each target point yields one copy of the source, transformed by that point's transform attributes (P, N, up, orient, pscale, scale, trans, rot, … `[R]`). Can pack-and-instance, copy by piece attribute, and transfer target attributes to copies.

## Params (13 doc-labelled; names `[R]`)
| label [D] | name [R] | default [R] | meaning |
|---|---|---|---|
| Source Group | sourcegroup | empty | subset of source prims |
| Source Group Type | sourcegrouptype | Guess | |
| Target Points | targetgroup | empty | subset of target points |
| Piece Attribute | pieceattrib | empty | choose source piece per target point (needs matching attr) |
| Pack and Instance | pack | off | pack source as packed prims (out of scope: DESIGN non-goal) |
| Pivot Location | pivot | Origin | packed only |
| Display As | viewportlod | Full Geometry | packed only |
| Transform Using Point Orientations | transform | on | apply target xform attrs |
| Reset Attributes from Target | resettargetattribs | on `[?]` | |
| Attributes from Target | targetattribs (multiparm) | none | list: Apply to (Point/Prim/Vertex/Detail) · by (Copy/Add/Mul/…) · Attributes (pattern) |

## Inputs
2 required: 0 "Geometry to Copy", 1 "Target Points to Copy to" [D]. (Input 2 "Piece"-style optional input not in doc.)

## Outputs
- [R] #points = N_target · #points_src; #prims likewise; order = copy k contiguous block, within block source order.
- [R] attributes of source preserved; target attrs only if listed under Attributes from Target (non-transform attrs NOT auto-copied — differs from legacy Copy Stamp).
- [R] source groups preserved with union over copies.
- [D] N/up/orient on target point rotate the copy.

## Invariants
- [R] count multiplicative as above; empty target → empty output.
- [R] copy k = src transformed by M_k, M_k built from point k's attrs in fixed order (scale → rotate/orient → translate=P).
- [R] no target attrs → identity transform + translate by P only.

## Edge cases (NOT observed)
| case | expected | fixture |
|---|---|---|
| box onto 3 pts of grid | 24 pts/18 prims | `copytopoints-default` |
| target 0 points | [?] empty vs source passthrough | `copytopoints-edge1` |
| no source input | [?] error | `copytopoints-edge2` |

## Fixtures
`copytopoints-default`, `copytopoints-edge1` planned.

## Open questions
- [ ] [?] exact transform composition order, default param set, multiparm layout → PROBE.md.
