# Behaviour: SOP cook model

feature ids: core.cook · observed_in: Houdini 20.5.487 (hython probe `oracle/houdini/probe-cook-model.py` -> `fixtures/core-cook-model/houdini.json`, 2026-10-10)

## Provenance
`[R]` = recalled from public Houdini docs/knowledge, UNVERIFIED. `[?]` = to probe. Probe recipe: `PROBE.md` (`n.cook()`, `n.needsToCook()`, `n.isTimeDependent()`).

## Description
- [R] Pull-based, lazy: nothing cooks until something asks for a node's geometry (display flag in a viewer, `geometry()`, a downstream cook). Cooking a node first cooks its inputs (recursively, on demand), then itself.
- [R] Each node caches its last output. A node is *dirty* when a parm changed, an input changed/dirtied, an input connection changed, or (if time-dependent) the current time changed. `n.needsToCook()` reports dirty. Dirtying propagates DOWNSTREAM to all dependents immediately; cooking is deferred.
- [R] Time dependence: a node is time-dependent if any parm has an expression referencing time (`$F`, `$T`, `ch()` on animated parm, keyframes) or it reads time internally (noise Animate). `n.isTimeDependent()` true → changing frame dirties it and everything downstream. Non-time-dependent subgraphs reuse cache across frames.
- [R] `n.cook(force=False)`: cooks if dirty; `force=True` ignores cache. Errors/warnings are stored per node (`n.errors()`, `n.warnings()`); an erroring node outputs empty geometry (or last good, \`[?]\`).
- [R] Bypass flag: node outputs its first input unchanged, no cook of own parms. Display/render flags choose what is shown/rendered, not what cooks (only flagged + upstream cook).
- [R] Geometry is value-like: downstream nodes get copies (copy-on-write); a node never mutates its input's cache.

## State machine
| state | event | next | effect |
|---|---|---|---|
| clean | parm edit / input change | dirty | downstream marked dirty |
| clean (time-dep) | frame change | dirty | downstream dirty |
| clean (not time-dep) | frame change | clean | cache reused |
| dirty | cook request | cooking | inputs cooked first |
| cooking | success | clean | output cached |
| cooking | error | clean+error | error recorded, empty/prev output \`[?]\` |
| any | bypass on | dirty | output = input 0 |

## Invariants
- [R] second cook with no change performs no work (`needsToCook()==False`).
- [R] result of cook depends only on (parms, inputs, time-if-dependent) → deterministic cache.
- [R] dirty flag of node ⊇ dirty of its inputs after propagation.
- [R] unflagged subgraphs not connected to display flag don't cook in UI.

## Edge cases (to probe)
| case | expected `[?]` | fixture |
|---|---|---|
| `needsToCook()` on fresh node | True | `cook-default` |
| set parm to same value | stays clean? | `cook-edge1` |
| `$F`-driven parm, cook at two frames | time-dependent true, outputs differ | `cook-edge2` |
| cycle in graph | error, no hang | `cook-edge3` |

## hexe mapping
DESIGN §Graph: `Graph::cook(id,time)` cached unless dirty or time-dependent; `set_param` marks dirty + downstream. Matches [R] model.

## Open questions
- [ ] [?] value of `needsToCook()` after param set to identical value; error-node output; unflagged-node cooking in hython.


## Observed (hython 20.5.487, chain box -> xform(tx) -> null)
| step | needsToCook b/x/n | note |
|---|---|---|
| fresh | T/T/T | cookCount 0 |
| cook tail | F/F/F | whole chain cooked once (pull); 2nd cook = no work |
| parm set to SAME value | F/T/T | [O] dirties node + downstream (hexe: no-op, DEVIATION) |
| parm set new value | F/T/T | upstream stays clean |
| `cook(force=True)` on tail | - | cooks ONLY that node (x count unchanged) |
| frame change, static chain | F/F/F | cache reused |
| `$F` on x | F/T/T | isTimeDependent b F, x T, n T (propagates DOWNSTREAM, known before cook) |
| frame change, time-dep | F/T/T | box stays clean |
| upstream parm set | T/T/T | whole cone below dirtied |
| input disconnect / reconnect | F/T/T | downstream dirtied |
| connect forming a cycle | - | raises `hou.OperationFailed` at connect |
Test: `crates/sop/src/cook_model_oracle.rs::engine_follows_houdinis_recorded_cook_model_step_by_step` (+ `time_dependence_propagates_downstream_and_frame_change_dirties_only_that_cone`).
