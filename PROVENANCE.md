# PROVENANCE: hexe (port of Houdini)
MODE at scaffold: **pirate**. Every change of mode or crossing of material into this repo is logged here.

## Crossings (what entered the repo from the spec/dirty side)
| date | what (paths) | from | by (agent/human) | leakscan (`cargo xtask leakscan`) | notes |
|---|---|---|---|---|---|
| YYYY-MM-DD | scaffold only (no spec yet) | template | new-port.sh | n/a (0 files) | MODE=pirate |

## Rules
- MODE=clean: only `spec/`, `fixtures/`, `parity/` cross, via monad, with leakscan 0 hits recorded in this table.
- MODE=pirate: dirty-side reading is allowed; still log what spec/fixtures were published and how fixtures were produced (`fixtures/MANIFEST.toml`).
- Assets/crates provenance lives in `ATTRIBUTION.toml` / `deny.toml`, not here.

## bgeo formats (H39, 2026-10-14)
- bjson (`0x7f NSJb`, `crates/geo/src/bjson.rs`) + writer: token table derived from the 153 Houdini 20.5.487 install files + public format description https://www.sidefx.com/docs/houdini/io/formats/geo.html (JSON schema: topology/attributes/primitives). No SideFX code copied.
- Legacy `BgeoV` (magic 42 67 65 6f 56, `crates/geo/src/bgeo_legacy.rs`): clean-room from bytes. v2 layout decoded from Houdini `GPD/all.bgeo` and checked against its shipped text twin `GPD/all.geo` (fixtures/gpd-all, fixtures/houdini-ship/gpd-all-legacy); v3/v5 (carousel slug mountain head_poly deflamp) are self-consistency checked only (parse to the `00 ff` trailer, counts = header). Public context only: HDK docs for `GA_IO`/`GEO_IOTranslator` (https://www.sidefx.com/docs/hdk/) name the translator interface, give no byte layout; no HDK source read or copied.
- fixtures/bgeo-bjson/*.geo: ASCII twins saved by hython 20.5.487 (`oracle/houdini/bgeo-twins.py`), rows in fixtures/MANIFEST.toml.
