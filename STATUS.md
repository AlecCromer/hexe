# STATUS: app = Tauri (T2a)

- Desktop app = `apps/hexe-tauri` (Tauri 2: Rust commands over hexe-graph + vite/TS UI, three.js viewport). egui `apps/hexe` DELETED (last at 805b5aa).
- Bundle: `cd apps/hexe-tauri && bun run tauri build` -> `target/release/bundle/{macos/Hexe.app,dmg/Hexe_0.1.0_aarch64.dmg}`; ad-hoc `codesign --force --deep --sign -`.
- egui->Tauri port checklist: apps/hexe-tauri/PORT-TODO.md (0 open).

---

# STATUS — Oracle 2026-10-11

- Oracle 2026-10-11: 143 MATCH / 1 TOPO-DIFF (boolean split-poly start vertex) / 12 STAT-PASS over 156 (N=156); 79 SOPs; /Applications/Hexe.app = h38 merge (sops-b + sops-c), built from main HEAD.
- Oracle 2026-10-12 (H35b): histogram unchanged (143/12/1). h33/order-twins = already on main (e039ca3, rebase = no-op); h34/model-twins merged (probes only, dc956f8). sop-boolean start vertex NOT derived: `cargo xtask boolstart` table (best 42/128), hython sweep parity/ref/boolean/start-sweep.txt (start flips with offset at identical topology) -> TOPO-DIFF stays, no per-case hack.

---

# STATUS — h1/bgeo (bjson decoder, WIP salvage)

- crates/geo/src/bjson.rs: binary-JSON (`0x7f NSJb`) → serde Value; json_read.rs: p_r/c_r/run prim abbreviations → Detail.
- Probe over Houdini 20.5.487 install (tests/bjson_tests.rs, path /Applications/Houdini/...; no-op when absent): 160 files; 153 bjson → 153/153 value-ok, 153/153 detail-ok; 7 NOT bjson (legacy `BgeoV` magic 42 67 65 6f 56 00 00 00: slug, mountain, carousel, fabric, head_poly, GPD/all.bgeo, deflamp) → legacy reader: see below.
- Unknown token bytes encountered: NONE (all 153 decode). Decoder errs on unknown token / length prefix / uniform elem (bjson.rs).
- Failing fixtures: none in bjson set. Legacy BgeoV: 7/7 read (H13+H39), counts + P bbox == Houdini (fixtures/bgeo-legacy/counts.json, hython bgeo-legacy-counts.py); bgeo_binary_reads un-ignored.
- fixtures/bgeo-bjson/: 8 files (axis cross ctrl-box ctrl-disc-xy ctrl-planes-xy sg_geolight simple_sphere tree0) + MANIFEST rows; golden tests (H39): crates/geo/src/bjson_golden_tests.rs vs hython-saved .geo twins (counts + vertex map + P max|dP| 0).
- Prim types seen: p_r 42, c_r 90, run 16 (PackedGeometry 55, Mesh/NURBMesh/Circle/Sphere/Tube/Bezier/NURBCurve/PackedDisk) — only polygon/curve runs mapped to Detail; others pass-through/unverified.
- Writer round trip on the 8 fixtures: bjson_golden_tests::writer_round_trips_all_shipped_fixtures (H39).
- TODO: 84-fixture target unmeasured.

---

# STATUS — Endorphin hitreact oracle 2026-10-12 (branch endo/finish)

- Oracle row `sop-motion-hitreact` (oracle/houdini/sop-motion-hitreact.py): hexe `motion` mode=hitreact seed 12 f30 vs gaia-motion HDA `gaia::motion::1.0` hit_reaction cooked in hython 20.5.487 → MATCH (24/23/46, max|dP| 0.0, name + pelvis_z equal). Histogram 144 MATCH / 12 STAT-PASS / 1 TOPO-DIFF (N=157).
- OPEN: reader-vs-reader only (both read the same MuJoCo CLI sim, no Houdini-native twin of the hit reaction); scene motion_root = `$GAIA_MOTION_ROOT` (env, default `~/projects/gaia-motion` main); re-record if hitreact.py changes seed-12 f30. Details: gaia-motion docs/STATUS.md.
