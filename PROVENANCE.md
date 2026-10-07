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
