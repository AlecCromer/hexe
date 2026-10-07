# Behaviour: Attribute Wrangle SOP (`attribwrangle`)

feature ids: sop-wrangle · observed_in: Houdini 20.5.487 (installed, not cooked)

## Provenance
Tags: `[D]` offline help shipped with install (`houdini/help/nodes.zip`), `[R]` recalled/UNVERIFIED, `[?]` unknown. hython blocked (no license) → nothing hython-observed. Probe: `PROBE.md`. hexe replaces VEX with **rhai** (DESIGN §waves) → spec covers node surface, NOT VEX semantics.

## Description
[D] "Runs a VEX snippet to modify attribute values." Snippet runs once per element of the chosen class (points/prims/vertices/detail/numeric); reads/writes attributes via @-bindings, auto-bound by name; may create attrs.

## Params (12 doc-labelled + snippet; names `[R]`)
| label [D] | name [R] | default [R] | meaning |
|---|---|---|---|
| Group | group | empty | elements to run on |
| Group Type | grouptype | Guess | |
| (Run Over) | class | Points | detail/prims/points/vertices/numbers [R] |
| (VEXpression) | snippet | empty | the code |
| Attributes to Create | bindings | empty | explicit new-attr declarations |
| Autobind by Name | autobind | on | bind @name to attr |
| Autobind Groups by Name | groupautobind | on | |
| Group Bindings | groupbindings | – | multiparm |
| Evaluation Node Path | exportpath | `.` | |
| Export Parameters | exports | off | |
| Update Normals If Displaced | updatenorms | on `[?]` | recompute N when P changes |
| Attribute To Match | matchattrib | – | numeric/second-input matching |
| Compute Results In Place | inplace | – | |
| Output Selection Group | outputgroup | – | |

## Inputs
1 required; up to 4 total (inputs 2–4 readable in VEX) `[R]`.

## Outputs
- [R] input geometry + whatever the snippet wrote; new attrs inferred from first write type (@Cd=..., f@x, v@y …).
- [R] with Update Normals: displacing P recomputes N if present.

## Invariants
- [R] empty snippet → output == input.
- [R] elements outside Group untouched.
- [R] topology unchanged unless snippet removes elements (removepoint etc).
- [R] evaluation order = element index; per-element independent (parallel-safe).

## Edge cases (NOT observed)
| case | expected | fixture |
|---|---|---|
| `@P.y += 1` on box | all 8 Y +1 | `wrangle-default` |
| syntax error in snippet | [?] node error state, output = input? empty? | `wrangle-edge1` |
| run over points on 0-pt input | [?] empty, no error | `wrangle-edge2` |

## Fixtures
`wrangle-default`, `wrangle-edge1` planned (inputs: VEX; hexe rhai equivalents authored separately).

## Open questions
- [ ] [?] error behaviour; hexe rhai binding syntax decision (DESIGN) → PROBE.md + DESIGN.
