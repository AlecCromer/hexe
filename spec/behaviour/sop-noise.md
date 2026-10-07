# Behaviour: Noise SOP → Attribute Noise (`attribnoise::2.0`)

feature ids: sop-noise · observed_in: Houdini 20.5.487 (installed, not cooked)

## Provenance
Tags: `[D]` offline help shipped with install (`houdini/help/nodes.zip`), `[R]` recalled/UNVERIFIED, `[?]` unknown. hython blocked (no license) → nothing hython-observed. Probe: `PROBE.md`.

## Scope decision
DESIGN says "noise". 20.5 candidates: **Attribute Noise** (`attribnoise::2.0`, successor of Attribute Noise v1 / old Point Noise) and legacy **Mountain** (`mountain::2.0`, `mountain` v1 also installed). Chosen: Attribute Noise (current, modern default for displacing P). Mountain = follow-up if user wants it. `[R]`: user may mean Mountain → confirm.

## Description
[D] "Adds or generates noise in geometry attributes." Evaluates a noise field (Perlin/Simplex/Fast Simplex/... cloud) at each element's location and combines with a target attribute (default P) by an operation.

## Params (~50 doc-labelled)
| label [D] | meaning |
|---|---|
| Group | elements |
| Blend / Blend Attribute | mix output with original by factor/attr |
| Attribute Type / Attribute Names / Component Scope / Attribute Class | which attr(s) to noise, e.g. P, point class |
| Noise Along Vector / Vector Attribute | noise displacement along normal-like vec |
| Range Values: Positive / Zero Centered / Negative | output range: [0,1] / [-0.5,0.5]... `[?]` / [-1,0] |
| Operation: Set Initial / Set Always / Add / Subtract / Multiply / Minimum / Maximum | combine mode |
| Amplitude (+Scale), Min Value, Max Value, Middle Value, Range Length (+Scale each) | output scaling variants |
| Output Raw Value | skip range remap |
| Enable Remap Ramp / Remap Ramp | curve remap |
| Noise Type: Perlin Cloud / Simplex Cloud / Fast Simplex Cloud / … | basis (list `[?]` full) |
| Location Attribute, Element Size, Element Scale | sample position attr (default P) and frequency |
| Offset (2×: translate / seed) | |
| Use VEXpression / VEX Expression | |
| Animate Noise / Pulse Duration | time dependence (4th dim) |
| Lattice Warp / Lattice Warp Size / Gradient Warp / Accumulate … | domain warp |
| Enable Minimum/Maximum + values | clamp |
| Make Vectors Unit Length | normalize |
| Recompute Normals | |

## Inputs
1 required.

## Outputs
- [R] same topology; modifies target attribute (P default) — creates attribute if absent (Set Initial). Normal recompute optional.

## Invariants
- [R] deterministic for params+offset (no hidden seed except Offset).
- [R] noise is a function of sample position only → same point positions → same values regardless of topology.
- [R] time-dependent iff Animate Noise on (observable via `isTimeDependent()`).
- [R] noise bit-exactness requires porting the exact Perlin/Simplex tables — NOT in public docs; plan = distribution parity + oracle sampling fixtures at fixed points.

## Edge cases (NOT observed)
| case | expected | fixture |
|---|---|---|
| default on grid | P displaced | `noise-default` |
| Element Size 0 / negative | [?] error vs NaN vs identity | `noise-edge1` |
| 0 points | [?] empty | `noise-edge2` |

## Fixtures
`noise-default`, `noise-edge1` planned.

## Open questions
- [ ] [?] confirm Attribute Noise vs Mountain; all defaults; exact noise functions → PROBE.md.
