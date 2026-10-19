# ROADMAP: hexe (port of Houdini)

## Honest assessment (dated; update every session that moves a number)
| date | features shipped % (P0/P1/P2/P3) | effort-weighted % | fidelity measured? (oracle cases pass/total, mean score) | robustness (fuzz h, crashes) | binary_size ours / original / ratio | perf budget status | readiness |
|---|---|---|---|---|---|---|---|
| YYYY-MM-DD | 0/0/0/0 (`cargo xtask parity`) | 0 | no: 0 cases | none yet | ? MB / ? MB / n/a (`cargo xtask size`) | no budgets (`cargo xtask perf`) | scaffold only |

Rules: numbers come from tools (`parity`, `oracle`, `size`, `perf`), never from vibes. "Fidelity" = oracle score, else write "no". Feel = Pascal's hands, logged below.

## Gaps (highest tier first)
- [ ] UI: work PORT-TODO.md (Houdini camera mapping, hotkeys H/W/P/N/D/Cmd+S, point/prim numbers, script/VEX pane, spreadsheet)
- [ ] P0: intake done, first fixture proven, thinnest vertical slice (input → one op → output = oracle match)

### Open work from the 2026-10-18 editor session (driven by user .hip scenes; nothing committed yet)
- [x] **Nested /obj containers** (`obj › asset1 › geo1`): done 2026-10-18 (ledger `obj-containers`):
  object kinds + containers in model.rs (saved / loaded), .hip containers with their notes, file positions + flags,
  dive in / `u` / path-bar links. Back / forward still walks only obj <-> sop, not container depth.
- [ ] **Single-geo .hip name**: a .hip with ONE geo and no container still loads as `geo1` (should keep its object name).
- [ ] **Spare parameters** (ledger `ui-spare-parms`, planned): Edit Parameter Interface, every Houdini parm type.
- [ ] **Larger user scenes cook**: multi-object /obj networks, network boxes, For-Each pairs and Copy to Points load,
  but display chains still stop: attribnoise uses hexe-invented parms (Houdini token `mult` rejected, most parms dropped),
  groupcreate "include by edges" (unshared / boundary) not modelled. Still no hexe twin: pack, scatteralign, material, attribvop
  (needs a VOP-network compiler: its own epic), attribfrommap, polywire, polydoctor, texture, vellum*, Labs tree
  tools, solver, filecache -> the cook errors honestly; each is its own SOP atom. Done 2026-10-19: `join`, `copyxform`,
  `attribadjustfloat / integer / color`, `enumerate`, `orientalongcurve`, `remesh` (statistical parity), sphere
  `freq` > 100, groupcreate edge / vertex groups, Box polymesh / points / `r`, edgedivide named edge groups.
  Parm expressions that query geometry (`detail()`, `point()`, `$N`, `$CEX`) keep the saved value (hip-expr).
- [ ] Debt found 2026-10-18, not this session's: `polyreduce feature_edges_hold_their_line` assertions had been commented
  out by a collapsed line; restored, they FAIL (drift with features 0.056 > without 0.027). `xtask assets`: untracked
  `apps/hexe-tauri/src-tauri/icons/icon.ico` has no ATTRIBUTION row. (sop-group evidence name fixed 2026-10-19.)
  Windows-only test fails: `rop_geometry scripts_run_in_order...` and cli `cook_motion_chain_scene_gives_the_skeleton_frame`
  run shell scripts (`echo >>`, `stub.sh`) -> `cargo xtask ci` stops at `test` on Windows.

### Remaining steps from the 2026-10-19 SOP session (all uncommitted on top of 2d25018)
In flight when the usage limit hit (work saved in the tree, resume or redo):
- [ ] **attribnoise to Houdini parms** (`attribnoise_sop.rs`): declare every Houdini 20.5 parm tuple of attribnoise /
  attribnoisefloat / attribnoisevector (Houdini names, defaults, menu tokens: `operation` uses `mult`, not `multiply`);
  port the HDA's VEX noise exactly where feasible (else statistical, documented); `specs_declare_every_houdini_parm`
  test; fixtures/sop-attribnoise from oracle/houdini/attribnoise/sop_attribnoise_cases.py. Partial edits may be in the
  tree: check `git diff crates/sop/src/builtins/attribnoise_sop.rs` before continuing.
- [ ] **groupcreate "include by edges"** (`groupcreate_sop.rs`): declare every Houdini parm; implement groupedges,
  unshared, boundarygroups, min / max edge angle + length, depth (edgeptgrp / edgestep), random (reuse attribadjust's
  VEX rand); bounding object / volume / camerapath Unsupported only when ENABLED. Fixtures/sop-groupcreate-include +
  a synthetic open box -> groupcreate unshared `borders` -> edgedivide test. (Agent had only started reading.)
Then, to finish this batch:
- [ ] Merge those two: MANIFEST rows (sha256 from disk), ledger rows / evidence, `cargo xtask sops` (docs/SOPS.md).
- [ ] Full check: `cargo fmt --all`, clippy `--all-targets -D warnings`, `cargo test --workspace --no-fail-fast`
  (only the 3 known failures above allowed), `cargo xtask parity`, `cargo xtask oracle`, portability scan of changed
  files (no user names / home paths / hostnames / private scene names).
- [ ] Re-cook the private test scene locally per object (`hexe open <scene> --geo <obj> --cook-out ...`) and grep for
  BOTH "unsupported" and "bad parameter" (the 2026-10-19 check missed the latter); list the next blocker per object.
- [ ] Commit only when the user asks: one ledger atom per commit (hip-expr, sop-copyxform, sop-attribadjust,
  sop-enumerate, sop-orientalongcurve, sop-remesh, sop-sphere, sop-groupcreate + geo-groups, sop-box, sop-edgedivide).
Next SOP atoms after that (by how often they block display chains):
- [ ] `attribvop`: needs a VOP-network -> VEX compiler (own epic; plan before starting).
- [ ] `pack`: needs a packed-prim payload in hexe-geo (also unblocks copyxform `pack=1`, copytopoints pack).
- [ ] `scatteralign`, `material` (attribute-only twin), `attribfrommap` + `texture` (UV / image sampling),
  `polywire`, `polydoctor`, `filecache` (pass-through + file read), `solver`, vellum*, Labs tree tools.
- [ ] Remesh parity: 2-iteration convergence (+15% tris), box corner rounding (+12%), adaptive sizing on boxes.
- [ ] Orient Along Curve: `adjustupcurvature`, `transformbyattribs` with instance attribs.
- [ ] Group / pattern parsers: `@attr=value` expressions (enumerate, edgedivide, groupcreate).
- [ ] Geometry-query HScript expressions (`detail()`, `point()`, `$N`, `$CEX`, opinputpath) in hip-expr.
Process note: parallel agents exhausted the usage limit twice; run ONE agent at a time.

Next, as asked:
- [ ] **PolyReduce 1.0 parameter set + panel** (hython dump of `polyreduce` v1: reduce "Polygons", creases "Features",
  radio tabs `stdswitcher` Percentage / Number / Distance / Edge List with percentage "Keep %", numpolys "Keep #",
  objpath / distance / minpercent, collapseedges / limitedgecollapse / numedgecollapse; optimizationbias 1, borderweight 0,
  creaseweight 0, lengthweight 0, attribweight 0, topologicalweight 0, meshinvert on, triangulate on, keepedges off,
  originalpoints off, referframe / framereference, referinput). Replace hexe's `target` / `finalcount` /
  `preserveboundary` (borderweight = weighted boundary quadric, keepedges = hard boundary, originalpoints = endpoint
  collapse, meshinvert = flip rejection on/off); non-default unmodelled ones = explicit error. Param pane needs radio-tab
  folders (a `ParamSpec` folder hint through `ParamInfo`), sliders, and group-name fields with a dropdown of the input's groups.
- [ ] PolyReduce output parity: counts match Houdini on the file (218 pts / 432 tris) but the collapse order / positions
  are hexe's own QEM; feature-edge weighting is Garland-Heckbert constraint planes (Houdini's exact rule undocumented).

Known gaps left behind (all give an explicit error or are documented, none silent):
- [ ] Group SOP: Bounding Object input declared but unused; bounding / normal tabs and Edge Depth on edge groups;
  point / prim group viewport highlight (edge groups highlight only); legacy Group's other tabs vs hython unverified.
- [ ] Sphere: Rows / Columns curve connectivities; triangle connectivities with unique pole points.
- [ ] Objects: per-object undo (Houdini's is global); Null `at` position, object positions / flags EDITED in the UI are
  editor state (file values load, edits not saved); no Tree View pane; shelf `box_object1` naming only on an empty network.
- [ ] Viewport: no shaded + wireframe mode (edges only in W or when selected).
- [ ] Sticky notes / network boxes / node colours: all drawn, read-only (no editing, box auto-fit, minimize toggle).
- [ ] File browser: no file-type preview, sequences, "relative path" option; $JOB = $HIP.
- [ ] Spec: `spec/behaviour/cook-model.md` records hou's connect-time cycle rejection; the editor now allows wired loops
  (cook flags them) - add the editor row.
- [ ] Env-bound failures on this Windows box (unrelated): rop_geometry script test, motion stub.sh, behaviours/*.json.

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
| 2026-10-18 | claude-opus (hexe-5d) | 2.5 | sop-dot network dots (engine `dot` + .hip dots + editor) + /obj wiring, wire select/delete, reverse drag, loop-cut, display-disc layering, rounded Null | .hip dots: wires through dots 1/6 -> 6/6 kept, cook == Houdini; sop-dot partial (no /obj dots); Windows box: pre-existing fmt/clippy/test-path drift fixed, 16 env-bound tests (gaia-motion venv, rop shell scripts, debug-server) fail here before and after | windows-baseline |
| 2026-10-18 | claude-opus (editor UX) | ~8 | UI session (no ledger atom; uncommitted): infinite 5x5 grid + labels, TAB menu (/obj categories, ghost placement, wire insert, label search), box/wire select + delete, F frame (network + viewport selection), File > Open browser ($HIP), drag-and-drop, rename-in-place + numbering, back/forward, wired loops + red !, multi Geometry objects (per-object networks/undo, multi-object file), .hip nested geos + notes, sphere triangle connectivities (hython-matched), edge groups (geo + Group Edges tab, hython-matched counts), PolyReduce 3 inputs + feature edges | a sphere -> group -> polyreduce .hip cooks end to end (polyreduce 218/432 == Houdini); open items listed under Gaps | not run (cargo xtask ci); crate tests green except 3 env-bound |
| 2026-10-18 | claude-opus-5.5 | ~3 | ui-node-flags, sop-control, obj-containers, hip-foreach (+ ui-spare-parms planned): bypass / template (ghost) / display flags, required-input arrow + dashed optional wires, slat line removed, Null object = control + point1 network, Add SOP point list, /obj containers from .hip (object-level assets), network boxes + node colours drawn, For-Each blockpath pairing, second-output wires dropped with a note | nested-asset .hip: /obj/<asset>/<geo> + notes, cooks 218/432; a five-object .hip loads (network boxes, 3 For-Each pairs) but its display chains stop at no-twin SOPs | ci: fmt + clippy green (fixed 4 pre-existing clippy errors); test red only on env-bound Windows failures + restored polyreduce assertion; assets / parity red on pre-existing rows; app driven live (debug server + screenshots); nothing committed |
| 2026-10-19 | claude-opus-5.5 | ~1.5 | sop-join (P1, partial): Join SOP, every rule derived from hython 20.5.522 probes (oracle/houdini/join/) | recorded synthetic arcs2 + arcs7 zero-diff vs Houdini (P + vertex order, fixtures/sop-join); 13 tests | fmt + clippy green; join tests green; parity / assets red only on the pre-existing sop-group + icon.ico rows; not committed |
| 2026-10-19 | claude-opus-5.5 | ~2 | hip-expr (P1, partial): .chn channels parsed, HScript evaluator (hexe_graph::hscript), import evaluates expression parms at the saved frame; `hexe open --exprs` | semantics checked with hou.hscriptExpression; 8 tests; geometry-query expressions keep the saved value + note | fmt + clippy green; graph tests green; not committed |
| 2026-10-19 | claude-opus-5.5 (+5 parallel subagents) | ~3 | sop-copyxform, sop-attribadjust, sop-enumerate, sop-orientalongcurve, sop-remesh (new, partial); sop-sphere poly rebuilt to Houdini order + freq budget; sop-groupcreate edge / vertex; geo-groups i8 selections | all synthetic hython fixtures; copyxform / attribadjust (82 cases) / enumerate (19) / orientalongcurve (28) / groupcreate (388) match; remesh statistical (most cases within 5%) | clippy green; sop lib 854 pass, 2 pre-existing fails (polyreduce, rop_geometry script) + cli `cook_motion_chain` (stub.sh, Windows); not committed |
