# PORT-TODO: egui `apps/hexe` -> `apps/hexe-tauri`

Source = `apps/hexe/src/{main,network,params,script,viewport,gpu}.rs` @ main `805b5aa` (git history keeps it: `git show 805b5aa:apps/hexe/src/<f>`).
Checklist = in egui, NOT yet in Tauri. `[ ]` open.

## network (network.rs)
- [ ] multi-select: shift-click toggles node in selection (egui `Act::Select{add}`); Delete removes ALL selected (tauri: single sel)
- [ ] right-click node = display flag (tauri: dblclick only)
- [ ] `D` key = display flag on hovered/active node (pane-hover scoped, no text focus, not Cmd+D)
- [ ] display flag is a TOGGLE (`Scene::toggle_display`, 2nd press clears); tauri `set_display` only sets
- [ ] drop wire on node BODY / near a port (DROP radius 14px) = connect to nearest free port (`resolve`); tauri: exact port-dot drop only (verified ui/src/network.ts `up`)
- [x] drag wire OFF connected input = disconnect + reroute (tauri HAS it)
- [ ] wire port-index label next to each input dot
- [ ] verify Tab popup: new node lands at cursor canvas pos (UNVERIFIED in tauri)
- [x] depth layout (tauri `auto_layout` Rust-side, vertical flow)
- [x] tauri extras (keep): canvas pan/zoom, edge selection, dblclick + flag-dot display

## params (params.rs)
- [ ] drag-value widgets (click-drag on number to scrub) for F/I/Menu/V3; tauri = number input + range slider
- [ ] `Err` from graph (rejected value) -> status bar (tauri: per-pane err-msg; verify status bar too)
- [ ] per-param registry range `(min,max)` -> drag speed

## viewport (viewport.rs)
- [ ] Houdini camera mapping: LMB / alt+LMB orbit · MMB or shift+LMB pan · RMB drag dolly · scroll dolly (tauri OrbitControls: RMB = pan, no shift+LMB pan, no alt rule)
- [ ] pitch clamp, dist range (1e-3..1e6), near/far = multiples of orbit dist, FIT_MARGIN 1.15, fov 0.9 rad
- [ ] `H` / `F` = frame all (tauri: Fit button only, NO hotkey); default camera when nothing displayed
- [ ] `W` = wireframe toggle hotkey (tauri: checkbox only; egui W = edges ONLY, tauri wire = overlay on shaded)
- [ ] `P` = point numbers overlay (up to 500 labels)
- [ ] `N` = prim numbers overlay (up to 500 labels, scan <=100k prims)
- [ ] axis gizmo (corner)
- [ ] view-state label in status (`shaded +P +N`)
- [ ] viewport `Style` tunables struct (colours, light ambient/diffuse, point px radius + depth pull, grid lines/spacing) — tauri hard-coded in viewport.ts
- [ ] ground grid with coloured axes (X red / Z blue) + Y axis line (tauri GridHelper, no axes)

## gpu (gpu.rs)
- [ ] shading: headlight `ambient + diffuse*|N.V|` flat per-face Newell normal, double-sided (tauri = three.js Hemisphere+Directional lights, flat, ~similar not identical)
- [ ] round screen-space points w/ depth pull (tauri: square-ish PointsMaterial px)
- [ ] draw caps: 1M tris · 2M edge verts · 500k points · 8192px target (tauri: no caps -> huge cook may freeze webview)
- [ ] headless PNG render: `hexe scene.json --render out.png [--size WxH] [--wire]` (egui `render_offscreen`, same Scene3d); tauri has NO headless path. Lost capability: GPU-less CI proof renders
- [ ] Sphere/Tube/Volume prims not drawn (also not in egui; tauri counts them `undrawn`)

## app shell (main.rs / script.rs)
- [ ] Cmd+S = save, Cmd+O = reload/open (tauri: buttons only)
- [ ] Enter in path field = open (tauri HAS this)
- [ ] CLI: `hexe [scene.json]` positional (tauri: `HEXE_OPEN` env only); `--version`
- [ ] CLI `--set node.param=value` (repeatable), re-applied on EVERY reload incl. Cmd+O
- [ ] CLI `--script ops` proof hook: `add|connect|disconnect|remove|display|select|set|view w|p|n|save|shot|quit`
- [ ] status bar: `display: <name>` · `points · prims · verts · cook ms` · green note · `no display node (right-click a node or press D)` hint (tauri: no verts count, no display name)
- [ ] MAX_SCENE_BYTES 64 MiB Open limit (tauri HAS it, model.rs)
- [ ] save writes `ui.pos` (tauri HAS node_move; verify save round-trip keeps pos)

## never existed in egui (DESIGN.md plans, still TODO for Tauri)
- [ ] script pane / VEX editor (hexe-vex crate exists; no UI in either app)
- [ ] geometry spreadsheet (points/prims/vertices attribute table)
- [ ] rename node UI · undo/redo · copy/paste · node picking in viewport · MSAA · shaded+edges overlay · per-prim/vertex N
