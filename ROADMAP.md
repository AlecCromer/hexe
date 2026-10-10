# ROADMAP: hexe (port of Houdini)

## Honest assessment (dated; update every session that moves a number)
| date | features shipped % (P0/P1/P2/P3) | effort-weighted % | fidelity measured? (oracle cases pass/total, mean score) | robustness (fuzz h, crashes) | binary_size ours / original / ratio | perf budget status | readiness |
|---|---|---|---|---|---|---|---|
| YYYY-MM-DD | 0/0/0/0 (`cargo xtask parity`) | 0 | no: 0 cases | none yet | ? MB / ? MB / n/a (`cargo xtask size`) | no budgets (`cargo xtask perf`) | scaffold only |

Rules: numbers come from tools (`parity`, `oracle`, `size`, `perf`), never from vibes. "Fidelity" = oracle score, else write "no". Feel = Pascal's hands, logged below.

## Gaps (highest tier first)
- [ ] UI: work PORT-TODO.md (Houdini camera mapping, hotkeys H/W/P/N/D/Cmd+S, point/prim numbers, script/VEX pane, spreadsheet)
- [ ] P0: intake done, first fixture proven, thinnest vertical slice (input → one op → output = oracle match)

## Session log
- T2a: app = Tauri (`apps/hexe-tauri`); egui `apps/hexe` retired, port backlog in apps/hexe-tauri/PORT-TODO.md; bundle config on (Hexe / org.gaia-systems.hexe, app+dmg).
| date | agent | hours | atom (ledger id) | result | CI |
|---|---|---|---|---|---|
| YYYY-MM-DD | scaffold (new-port.sh) | 0 | template | workspace compiles, `xtask ci` green on empty | green |
| 2026-10-08 | nyari (h32) | 0.7 | h32 Houdini winding global + reverse/twist/polyextrude splittype | oracle N=52: MATCH 9->39 (H31 start 9 MATCH/26 WINDING-DIFF -> 39 MATCH, 0 WINDING-DIFF); 24 rows partial->shipped | green |
| 2026-10-09 | ghoul-sonnet (nurbs) | 0.5 | geo-nurbs-prims + sop-curve/revolve/skin/convert/clip/boolean/polyfill NURBS | 4 first-class prim kinds (evaluate/tessellate/IO), 7 SOP rows lose the no-NURBS caveat, no row flipped (UNVERIFIED lists remain); Houdini NURBS JSON layout not in corpus (ours) | green |
| 2026-10-10 | ghoul-sonnet (promote-1b) | 0.4 | sop-polyfill / sop-revolve / sop-sweep shipped; sop-skin pinned partial; gap notes on 8 rows | hython zero-diff on 4+4+6 recorded cases (polyfill quad fan/odd holes, revolve+skin quad winding, sweep frame/mitre/caps/uv); skin resample + polybevel patches + boolean order + convert icosphere + noise/RNG rows stay partial | green |
| 2026-10-10 | ghoul-sonnet (trio) | 0.3 | knife: cut-point mint rule found (closing edge first) -> P zero-diff on 5 cases, grid case zero-diff; skin: edge-midpoint resample rule (longest f32 edge) implemented, residual = circle f32 noise; uvlayout: stat-uv oracle, sizes+coverage pass, layout extent gap | hython sop-knife-grid/sop-skin-2ring/sop-uvlayout-stat recorded; knife slot rule + skin tie order + uvlayout 3x2 pack stay partial | green |
| 2026-10-10 | ghoul-sonnet (stat-oracle) | 0.6 | oracle modes `stat` / `stat-disp` (xtask + compare.py); sop-scatter / sop-noise / sop-mountain / sop-curve shipped | scatter 4 cases + noise 3 + mountain 3 pass the statistical oracle (counts exact; bbox/NN/std or |disp| mean+std within 5 %, worst 4.2 %); curve 5 cases zero-diff vs a real curve::2.0 twin (savedstashgeo); hexe scatter loses default N/sourceprim, gains countperprim; noise/mountain perlin gains are FITTED to the recorded cases (UNVERIFIED beyond); streams stay non-reproducible | green |
