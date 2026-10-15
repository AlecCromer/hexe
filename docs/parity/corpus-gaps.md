# Oracle corpus gaps — 2026-10-15, branch parity/oracle-corpus
LAW: hexe = ours, Houdini = recorded oracle only. Default `cargo xtask oracle --houdini` reads `fixtures/sop-*/houdini.json|.geo` (+`x-*`); no hython. `--live-houdini` = explicit opt-in; hython failure → row `NO-ORACLE`, run continues.

## Run (recorded corpus, hython NOT touched) — `cargo xtask oracle --houdini`, 174 rows (169 sop-*, 5 x-*)
| verdict | sop-* (N=169) | all (174) | meaning |
|---|---|---|---|
| MATCH | 155 | 160 | counts, attrib names, P index order ±1e-4, prim cycles equal |
| STAT-PASS | 12 | 12 | statistical oracle (scatter/noise/mountain) |
| TOPO-DIFF | 1 | 1 | sop-boolean: 4 split polys, start vertex (see boolean-topo.md) |
| HEXE-ERR | 1 | 1 | sop-motion-hitreact: hexe cook needs `~/.gaia/cache/gaia-motion/venv/bin/python` (missing here), env not geometry |
| NO-ORACLE | 0 | 0 | none in corpus run |
pass = 167/169 · partial = 0 · diff = 1 · env-blocked = 1. Semantically identical to the earlier hython run (`results.json` 174/174 rows equal).
Live proof: `compare.py --live-houdini sop-box sop-boolean sop-grid` (hython license-blocked) → `{'NO-ORACLE': 3}`, rc 0.

## SOPs with NO recorded corpus (registry = 84 types, `hexe-cli nodes`)
**6 of 84** have no `fixtures/sop-*` row: `attribcreate` `color` `convert` `mirror` `polyreduce` `rop_geometry`.
- `convert`: only file-format twins in `parity/ref/convert/` (not a SOP cook oracle)
- `polyreduce`: branch `p1/polyreduce`, no recorded Houdini side on main
- `rop_geometry`: out of oracle scope by design (not a `sop-*` ledger row)
- `attribcreate` `color` `mirror`: no twin script `oracle/houdini/sop-*.py` at all
Non-gating extra corpora: `parity/ref/boolean` (61 files), `parity/ref/polybevel`, `parity/ref/smooth-probes`, `parity/ref/uvlayout`.

## Thin corpus (exactly 1 recorded case, default params only) — 42 types
add attribcopy attribdelete attribpromote attribrandomize attribtransfer bend blast boolean box circle connectivity delete divide facet file fuse grid group groupdelete grouppromote line matchsize measure merge motion normal output peak pointjitter polybevel polyextrude ray resample reverse smooth sort sphere subdivide switch twist uvproject uvunwrap wrangle
(+ `xform`/`null`/`block_begin|end` reached via fixtures named `sop-transform`/`sop-registry`/`sop-foreach`.) A MATCH on these proves ONE parameter point; non-default params are UNVERIFIED (ledger honesty).

## Not compared even when MATCH
Attribute VALUES (N, uv, area, class…) — names only (HOUDINI-ORACLE.md caveats).
