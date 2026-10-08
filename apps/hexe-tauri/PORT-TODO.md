# PORT-TODO: egui `apps/hexe` -> `apps/hexe-tauri`

Source = `apps/hexe/src/{main,network,params,script,viewport,gpu}.rs` @ main `805b5aa` (git history keeps it: `git show 805b5aa:apps/hexe/src/<f>`).
Checklist = in egui, NOT yet in Tauri. `[ ]` open.

## network (network.rs)
- [x] multi-select: shift-click toggles node in selection (egui `Act::Select{add}`); Delete removes ALL selected (tauri: single sel) (T3: Set selection, group drag, Delete = one undo step)
- [x] right-click node = display flag (tauri: dblclick only)
- [x] `D` key = display flag on hovered/active node (pane-hover scoped, no text focus, not Cmd+D)
- [x] display flag is a TOGGLE (`Scene::toggle_display`, 2nd press clears); tauri `set_display` only sets
- [x] drop wire on node BODY / near a port (DROP radius 14px) = connect to nearest free port (`resolve`); tauri: exact port-dot drop only (verified ui/src/network.ts `up`) (T3: netlogic.resolveDrop, 14 screen px, body = first free input)
- [x] drag wire OFF connected input = disconnect + reroute (tauri HAS it)
- [x] wire port-index label next to each input dot
- [x] verify Tab popup: new node lands at cursor canvas pos (UNVERIFIED in tauri) (T3: VERIFIED live, `ui addat grid 300 60` -> grid1 ui.pos [205,20] = cursor - (NODE_W/2,NODE_H/2); bun test pins the transform)
- [x] depth layout (tauri `auto_layout` Rust-side, vertical flow)
- [x] tauri extras (keep): canvas pan/zoom, edge selection, dblclick + flag-dot display

## params (params.rs)
- [x] drag-value widgets (click-drag on number to scrub) for F/I/Menu/V3; tauri = number input + range slider
- [x] `Err` from graph (rejected value) -> status bar (tauri: per-pane err-msg; verify status bar too) (T3: status bar + in-pane err-msg + network toast; NB main.ts structural() status is overwritten by the follow-up cook, network/params route errors past that)
- [x] per-param registry range `(min,max)` -> drag speed

## viewport (viewport.rs)
- [x] Houdini camera mapping: LMB / alt+LMB orbit · MMB or shift+LMB pan · RMB drag dolly · scroll dolly (tauri OrbitControls: RMB = pan, no shift+LMB pan, no alt rule) -> T2b camera.ts (alt+LMB = LMB orbit)
- [x] pitch clamp, dist range (1e-3..1e6), near/far = multiples of orbit dist, FIT_MARGIN 1.15, fov 0.9 rad
- [x] `H` / `F` = frame all (tauri: Fit button only, NO hotkey); default camera when nothing displayed
- [x] `W` = wireframe toggle hotkey (tauri: checkbox only; egui W = edges ONLY, tauri wire = overlay on shaded)
- [x] `P` = point numbers overlay (up to 500 labels)
- [x] `N` = prim numbers overlay (up to 500 labels, scan <=100k prims)
- [x] axis gizmo (corner)
- [x] view-state label in status (`shaded +P +N`)
- [x] viewport `Style` tunables struct (colours, light ambient/diffuse, point px radius + depth pull, grid lines/spacing) — tauri hard-coded in viewport.ts
- [x] ground grid with coloured axes (X red / Z blue) + Y axis line (tauri GridHelper, no axes)

## gpu (gpu.rs)
- [x] shading: headlight `ambient + diffuse*|N.V|` flat per-face Newell normal, double-sided (tauri = three.js Hemisphere+Directional lights, flat, ~similar not identical)
- [x] round screen-space points w/ depth pull (tauri: square-ish PointsMaterial px)
- [x] draw caps: 1M tris · 2M edge verts · 500k points · 8192px target (tauri: no caps -> huge cook may freeze webview)
- [ ] headless PNG render: `hexe scene.json --render out.png [--size WxH] [--wire]` (egui `render_offscreen`, same Scene3d); tauri has NO headless path. Lost capability: GPU-less CI proof renders
- [x] Sphere/Tube/Volume prims not drawn (also not in egui; tauri counts them `undrawn`) -> T2b: sphere/tube = wire proxies (volume still undrawn)

## app shell (main.rs / script.rs)
- [x] Cmd+S = save, Cmd+O = reload/open (tauri: buttons only)
- [ ] Enter in path field = open (tauri HAS this)
- [x] CLI: `hexe [scene.json]` positional (tauri: `HEXE_OPEN` env only); `--version`
- [x] CLI `--set node.param=value` (repeatable), re-applied on EVERY reload incl. Cmd+O
- [x] CLI `--script ops` proof hook: `add|connect|disconnect|remove|display|select|set|view w|p|n|save|shot|quit` (T3: all ops except `view` (viewport is T2b-rest); extras `rename undo redo copy paste ui <verb>`; `proof/*.ops`)
- [x] status bar: `display: <name>` · `points · prims · verts · cook ms` · green note · `no display node (right-click a node or press D)` hint (tauri: no verts count, no display name)
- [ ] MAX_SCENE_BYTES 64 MiB Open limit (tauri HAS it, model.rs)
- [x] save writes `ui.pos` (tauri HAS node_move; verify save round-trip keeps pos)

## never existed in egui (DESIGN.md plans, still TODO for Tauri)
- [ ] script pane / VEX editor (hexe-vex crate exists; no UI in either app)
- [x] geometry spreadsheet (points/prims/vertices attribute table)
- [x] rename node UI · undo/redo [x T3: Cmd+Z, Rust snapshot stack 100] · copy/paste [x T3: Cmd+C/V] · node picking in viewport · MSAA · shaded+edges overlay · per-prim/vertex N (T3: dblclick name / F2)
