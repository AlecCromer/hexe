# houdini-ship parse findings (H8, Houdini 20.5.487)
Ungated files (`gate = false` in MANIFEST.toml): reader cannot open them. FINDING, not a gate failure. Test `ship_tests::houdini_ship_ungated_files_are_documented` requires each ungated name here and reports when one starts parsing (then promote to gate).

| name | file | container | reader result | scope |
|---|---|---|---|---|
| gpd-all-legacy | all.bgeo | legacy binary `BgeoV` (header `42 67 65 6f 56`, i32be version=2, i32be npoints=0x16f=367) | `Err(Unsupported("legacy binary BgeoV is not supported"))` via `read_geo` sniff `B` | 7 of 169 shipped geometry files <2 MB are legacy `BgeoV` (slug, mountain, carousel, fabric, head_poly, deflamp, GPD/all) |

Expected values: gpd-all-legacy = from twin `all.geo` header (points 367, prims 13, vertices UNVERIFIED-from-reader).
Next: legacy `BgeoV` needs a binary reader (parity ledger row). RESOLVED (H11): `scf1` blosc-chunked `.bgeo.sc` (sphere-sc, bear-sc, box-sc now gated). OPEN finding: `pig.bgeo.sc` (4.3 MB, 11 chunks) decompresses byte-identical to python-blosc but the bjson layer then fails `Index("primitive vertex -1 outside 880906 vertices")` (not .sc related).
OPEN finding (reader, not .sc): `preview_network_1.bgeo.sc` (apex_cop, 6623 B) decompresses byte-identical and parses (9 pts / 24 vtx / 12 prims, all match oracle) but the reader drops point attrs `parms`, `properties`, `tags` (oracle: P callback Cd comment name parms properties tags; reader: no parms/properties/tags) → not gated.
