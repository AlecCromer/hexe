# Oracle corpus gaps — 2026-10-15, branch parity/oracle-corpus
LAW: hexe = ours, Houdini = recorded oracle only. Default `cargo xtask oracle --houdini` reads `fixtures/sop-*/houdini.json|.geo` (+`x-*`); no hython. `--live-houdini` = explicit opt-in; hython failure → row `NO-ORACLE`, run continues.

## Run (recorded corpus, hython NOT touched) — `cargo xtask oracle --houdini`, 174 rows (169 sop-*, 5 x-*)
| verdict | sop-* (N=169) | all (174) | meaning |
|---|---|---|---|
| MATCH | 156 | 161 | counts, attrib names, P index order ±1e-4, prim cycles equal |
| STAT-PASS | 12 | 12 | statistical oracle (scatter/noise/mountain) |
| MATCH(cyclic) | (in MATCH: sop-boolean) | (in MATCH) | sop-boolean: 4 split polys differ only by start vertex; rule = prim point lists equal up to rotation, same winding (reversal still WINDING-DIFF); `compare.py` counts them as `cyclic_prims`=4. Start-vertex rule itself stays NOT DERIVED (boolean-topo.md), engine unchanged |
| TOPO-DIFF | 0 | 0 | none |
| HEXE-ERR | 1 | 1 | sop-motion-hitreact: hexe cook needs `~/.gaia/cache/gaia-motion/venv/bin/python` (missing here), env not geometry |
| NO-ORACLE | 0 | 0 | none in corpus run |
| DOC-VERIFIED | (6 SOPs, separate table below, not in the 174 rows) | | doc-semantics tests, no recorded Houdini |
pass = 168/169 · partial = 0 · diff = 0 · env-blocked = 1. Semantically identical to the earlier hython run (`results.json` 174/174 rows equal).
Live proof: `compare.py --live-houdini sop-box sop-boolean sop-grid` (hython license-blocked) → `{'NO-ORACLE': 3}`, rc 0.

## SOPs with NO recorded corpus (registry = 84 types, `hexe-cli nodes`)
**6 of 84** have no `fixtures/sop-*` row: `attribcreate` `color` `convert` `mirror` `polyreduce` `rop_geometry`.
**State `DOC-VERIFIED`** (new, branch `parity/six-sops-nohou`): NOT Houdini-recorded. Oracle = (a) Houdini 20.5 help text `nodes.zip:sop/<name>.txt` (read-only text; no hython/licence/Houdini runtime), (b) related recorded rows, (c) hexe invariants; asserted by `crates/sop/tests/<name>_semantics.rs` (>= 3 cases: defaults, non-default, edge). Weaker than MATCH: proves documented semantics, never numeric identity (RNG streams, tessellation layout, QEM tie-breaks stay unverified).
| SOP | state | tests | builtin changed | doc facts NOT modelled |
|---|---|---|---|---|
| mirror | DOC-VERIFIED | `mirror_semantics.rs` (7) | yes: Keep Original off drops unselected prims+points; `createoutputgroup`/`outputgroup` | Operation=Clip, dirtype=Transform (t/r), unshared-edge-only consolidate, reverse U/V |
| attribcreate | DOC-VERIFIED | `attribcreate_semantics.rs` (6) | yes: type qualifier guess-from-name for float N/Cd/v/up | Existing Name modes, array types, local vars, info block, precision |
| color | DOC-VERIFIED | `color_semantics.rs` (6) | yes: `bbox` colortype, `deleteallcolorattribs`, `enable` | Ramp-from-attribute type, Random stream != Houdini |
| convert | DOC-VERIFIED | `convert_semantics.rs` (5) | yes: `keeporiginal` (Preserve Original) | Group, divisions U/V/trim, order, paste coords/attrs, interpolate-through-hulls, volume offset, mesh lifting |
| polyreduce | DOC-VERIFIED | `polyreduce_semantics.rs` (5) | no | Group, preserve quads, equalize lengths, seam weights, hard points/edges, visibility; boundary is HARD (Houdini: weight) so targets can hit a boundary floor |
| rop_geometry | DOC-VERIFIED | `rop_geometry_semantics.rs` (4) | no | background save, `$N`/`$NRENDER` vars |
- `convert`: also file-format twins in `parity/ref/convert/` (not a SOP cook oracle)
- `polyreduce`: branch `p1/polyreduce`, no recorded Houdini side on main
- `rop_geometry`: out of oracle scope by design (not a `sop-*` ledger row)
- `attribcreate` `color` `mirror`: no twin script `oracle/houdini/sop-*.py`
Non-gating extra corpora: `parity/ref/boolean` (61 files), `parity/ref/polybevel`, `parity/ref/smooth-probes`, `parity/ref/uvlayout`.

## Thin corpus (exactly 1 recorded case, default params only) — 42 types
add attribcopy attribdelete attribpromote attribrandomize attribtransfer bend blast boolean box circle connectivity delete divide facet file fuse grid group groupdelete grouppromote line matchsize measure merge motion normal output peak pointjitter polybevel polyextrude ray resample reverse smooth sort sphere subdivide switch twist uvproject uvunwrap wrangle
(+ `xform`/`null`/`block_begin|end` reached via fixtures named `sop-transform`/`sop-registry`/`sop-foreach`.) A MATCH on these proves ONE parameter point; non-default params are UNVERIFIED (ledger honesty).

## Not compared even when MATCH
Attribute VALUES (N, uv, area, class…) — names only (HOUDINI-ORACLE.md caveats).
