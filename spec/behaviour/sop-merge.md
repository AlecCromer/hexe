# Behaviour: Merge SOP (`merge`)

feature ids: sop-merge · observed_in: Houdini 20.5.487 (installed, not cooked)

## Provenance
Tags: `[D]` offline help shipped with install (`houdini/help/nodes.zip`), `[R]` recalled/UNVERIFIED, `[?]` unknown. hython blocked (no license) → nothing hython-observed. Probe: `PROBE.md`.

## Description
[D] "Merges geometry from its inputs" into one stream. Max 9999 inputs. Distinguish parts later via Group SOP before merging. Bypassing the node exports ONLY input 1 (no merge) [D].

## Params
Doc lists none beyond inputs. Recalled `[R]`: `group` (string), `grouptype`, `mergetype`-style options; real count `[?]` (hython needed). Doc-labelled count: 0.

## Inputs
0..9999 [D], variadic; order = concatenation order [R].

## Outputs
- [R] points/prims concatenated in input order, indices of input k offset by sum of counts of inputs <k; vertex→point refs rewired.
- [R] attribute union: attr missing on an input gets its default (0/empty) for that input's elements; type mismatch → [?] (probably dropped/warning).
- [R] groups of the same name union; detail attrs: first input wins `[?]`.

## Invariants
- [R] #points = Σ, #prims = Σ, #vertices = Σ.
- [R] merge(A) == A (single input); merge() = empty geometry.
- [R] input order determines index order, not geometry content.

## Edge cases (NOT observed)
| case | expected | fixture |
|---|---|---|
| box + grid | 8+100 pts `[R]`, 6+81 prims | `merge-default` |
| zero connected inputs | [?] empty detail, no error | `merge-edge1` |
| same attr name, float vs int | [?] | `merge-edge2` |

## Fixtures
`merge-default`, `merge-edge1` planned.

## Open questions
- [ ] [?] param list, attr conflict rules, group merge rules → PROBE.md.
