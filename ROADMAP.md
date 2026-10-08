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
