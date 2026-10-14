# houdini-ship parse findings (H8, Houdini 20.5.487)
Ungated files (`gate = false` in MANIFEST.toml): reader cannot open them. FINDING, not a gate failure. Test `ship_tests::houdini_ship_ungated_files_are_documented` requires each ungated name here and reports when one starts parsing (then promote to gate).

| name | file | container | reader result | scope |
|---|---|---|---|---|
| ~~gpd-all-legacy~~ (RESOLVED H13, now gated) | all.bgeo | legacy binary `BgeoV` v2 | `read_geo_bgeo_legacy`: 367 pts / 455 vtx / 13 prims, equals twin | 7 of 169 shipped files <2 MB are legacy `BgeoV` |

RESOLVED (H13): legacy `BgeoV` reader (`crates/geo/src/bgeo_legacy.rs`); gpd-all-legacy gated against its text twin. RESOLVED (H11): `scf1` blosc-chunked `.bgeo.sc` (sphere-sc, bear-sc, box-sc now gated).
RESOLVED (H12): `pig.bgeo.sc` (pig-sc, gated; 4.3 MB size exception) = `run` prims with `runtype Poly` carry a vertex LIST per item; reader treated it as one packed-prim vertex (`as_i64` of an array -> -1). `preview_network_1.bgeo.sc` (preview-network-1, gated) = point/detail attrs of type `dict` (parms, properties, ...) and `stringarray` (tags) have no `AttrData` form; now listed in `Detail::skipped_attribs` (never silent; `hexe-cli info` prints them); the gate compares kept + skipped to the full oracle name list.

## H13 legacy `BgeoV` findings (install files WITHOUT a text twin, not fixtures; `bgeo_legacy_tests::installed_legacy_files_do_not_panic` prints outcomes)
Self-check only (no oracle values): parse must consume the file exactly to the `00 ff` trailer and counts must equal the header. UNVERIFIED vs Houdini beyond that.
| file | version | result |
|---|---|---|
| carousel.bgeo | 5 | Ok 640 pts / 640 vtx / 3 prims, consumes to trailer |
| slug.bgeo | 5 | Ok 2178 / 8704 / 2176 (quads), consumes to trailer |
| mountain.bgeo | 5 | Ok 27426 / 109696 / 27424 (quads), consumes to trailer |
| head_poly.bgeo | 5 | Ok 1302 / 4864 / 1216, 3 prim groups, consumes to trailer |
| deflamp.bgeo | 3 | Ok 109 / 114 / 3, 1 prim group + prim attrib, consumes to trailer |
| fabric.bgeo | 5 | Ok 13020 / 13020 / 62 NURBMesh (H39: trim/profile block sits AFTER the prim attrib values, not before; fixed) |
Prim type tokens (u32 bit flags): Poly 0x1, NURBCurve 0x2, BezierCurve 0x4, Mesh 0x10, NURBMesh 0x20, BezierMesh 0x40, Circle 0x1000, Sphere 0x2000, Tube 0x4000, MetaBall 0x100000, MetaSQuad 0x200000. Any other token (0x8, 0x80, 0x400, particle, volume, ...) -> `Unsupported("legacy bgeo primitive type 0x.. (no model)")`. Only types seen in the corpus are mapped; NURBS/mesh/quadric prims are kept as `PrimKind::Unknown{ty}` + vertices (same model as the GPD reader).
