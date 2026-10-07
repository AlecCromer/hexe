# houdini-ship parse findings (H8, Houdini 20.5.487)
Ungated files (`gate = false` in MANIFEST.toml): reader cannot open them. FINDING, not a gate failure. Test `ship_tests::houdini_ship_ungated_files_are_documented` requires each ungated name here and reports when one starts parsing (then promote to gate).

| name | file | container | reader result | scope |
|---|---|---|---|---|
| gpd-all-legacy | all.bgeo | legacy binary `BgeoV` (header `42 67 65 6f 56`, i32be version=2, i32be npoints=0x16f=367) | `Err(Unsupported("legacy binary BgeoV is not supported"))` via `read_geo` sniff `B` | 7 of 169 shipped geometry files <2 MB are legacy `BgeoV` (slug, mountain, carousel, fabric, head_poly, deflamp, GPD/all) |
| sphere-sc | sphere.bgeo.sc | blosc-chunked `scf1` (`73 63 66 31`) | `Err(Syntax("unrecognised geometry container"))` (head byte `s`) | 10 shipped `.bgeo.sc` <2 MB (box/sphere/torus, pig, bear_with_vel, 5 apex_cop preview_network) |

Expected values: gpd-all-legacy = from twin `all.geo` header (points 367, prims 13, vertices UNVERIFIED-from-reader); sphere-sc = all UNVERIFIED-from-reader (no twin).
Next: `scf1` needs blosc chunk decode (inner payload = bjson) → new reader layer; legacy `BgeoV` needs a binary reader (parity ledger row).
