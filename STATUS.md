# STATUS — h1/bgeo (bjson decoder, WIP salvage)

- crates/geo/src/bjson.rs: binary-JSON (`0x7f NSJb`) → serde Value; json_read.rs: p_r/c_r/run prim abbreviations → Detail.
- Probe over Houdini 20.5.487 install (tests/bjson_tests.rs, path /Applications/Houdini/...; no-op when absent): 160 files; 153 bjson → 153/153 value-ok, 153/153 detail-ok; 7 NOT bjson (legacy `BgeoV` magic 42 67 65 6f 56 00 00 00: slug, mountain, carousel, fabric, head_poly, GPD/all.bgeo, deflamp) → legacy binary reader NOT implemented (tests::bgeo_binary_reads still #[ignore]).
- Unknown token bytes encountered: NONE (all 153 decode). Decoder errs on unknown token / length prefix / uniform elem (bjson.rs).
- Failing fixtures: none in bjson set. Legacy BgeoV: 7/7 unsupported.
- fixtures/bgeo-bjson/: 8 files (axis cross ctrl-box ctrl-disc-xy ctrl-planes-xy sg_geolight simple_sphere tree0) + MANIFEST rows; NO golden-compare tests yet (bjson_tests are probe/survey, no value assertions).
- Prim types seen: p_r 42, c_r 90, run 16 (PackedGeometry 55, Mesh/NURBMesh/Circle/Sphere/Tube/Bezier/NURBCurve/PackedDisk) — only polygon/curve runs mapped to Detail; others pass-through/unverified.
- TODO: assert tests vs fixtures (point/prim counts, vs .geo twins); legacy BgeoV; writer; 84-fixture target unmeasured.
