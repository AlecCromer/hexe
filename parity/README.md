# parity/
Ledger `features.toml` (legend = its header). Oracle = real Houdini 20.5 (hython) on inputs WE author; AGENTS.md §3.

## Oracle modes (`fixtures/<case>/case.toml` `compare`, run by `cargo xtask oracle`; Houdini report by `cargo xtask oracle --houdini`)
| mode | for | pass rule |
|---|---|---|
| `geo` | deterministic SOPs | point/vertex/prim counts equal, `P` ±1e-5 index order, (class,name) attr set equal |
| `stat` | point generators with an RNG (scatter) | below |
| `stat-disp` | noise displacement (attribnoise, mountain); needs `base = "base.geo"` = cook input | below |

### `stat` (oracle/houdini/statoracle.py mirrors xtask/src/oracle.rs `stat_mismatch`; tolerances in sync)
- point count exact
- bbox min/max per axis: |Δ| ≤ 2 % of the Houdini extent (+1e-4)
- mean nearest-neighbour distance ±5 %
- `P` std per axis ±5 % (degenerate axis: ±1e-4 abs)
- attribute names exact: (class,name) set
### `stat-disp`
- point count exact; mean and std of |P − base| ±5 %; attribute names exact

## Add a statistical case
1. `oracle/houdini/sop-<x>.py`: `finish(SOP, SCENE, build, NOTE, oracle="stat")` (scatter) or `…, oracle="stat", stat="disp")` (writes `base.geo` = first input of the twin)
2. size for the tolerance: ≥ 1500 points, ≥ ~600 independent noise cells (std-of-std ≈ 1/√(2n), `P_all` is capped at 5000 points)
3. `hython oracle/houdini/run_all.py sop-<x>` → `fixtures/sop-<x>/{scene.json,houdini.geo,houdini.json,base.geo}`; write `case.toml` (cmd `cook {input} --out {out} --time 0`, input scene.json, expected houdini.geo, compare stat|stat-disp, feature `<ledger id>`, base for disp)
4. `python3 oracle/houdini/manifest.py`; `cargo xtask oracle --raise`; compare.py verdict `STAT-PASS|STAT-FAIL` lands in docs/HOUDINI-ORACLE.md
5. ledger row: `shipped`, notes tag `stat-oracle` + measured deviations + fitted constants (honest: fits are UNVERIFIED outside the recorded cases)

## Limits
- passing `stat` ≠ same points: streams differ by construction, only distributions are compared
- a fitted gain (GAIN_* in attribnoise_sop.rs / mountain_sop.rs) is calibrated on the recorded cases, not proven
