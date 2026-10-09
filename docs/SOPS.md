# SOP coverage

Src: `crates/sop/src/builtins/mod.rs` → 63 `r.register(...)` (type list; `registry.rs` = container only).
Parity: `parity/features.toml` row `sop-<x>` (exceptions: `null`→`sop-registry`, `block_begin|end`→`sop-foreach`, `attribnoise`→`sop-noise`, `xform`→`sop-transform`, `rop_geometry`→`rop-geometry`).
Example: `examples/<x>.json` = dedicated · `via <f>.json` = SOP only inside multi-SOP scene `<f>`.
Render: `hexe-tauri examples/<f>.json --render <png> --size 480x300`, exit 0 = ok. PNGs not committed (`~/.gaia/cache/hexe/proof/book/`).

| SOP | parity | example | render |
|---|---|---|---|
| add | shipped (sop-add) | via `curves.json` | ok |
| attribcopy | shipped (sop-attribcopy) | via `attrib-family.json` | ok |
| attribcreate | shipped (sop-attribcreate) | `attribcreate.json` | ok |
| attribdelete | shipped (sop-attribdelete) | via `attrib-family.json` | ok |
| attribnoise | partial (sop-noise) | `attribnoise.json` | ok |
| attribpromote | shipped (sop-attribpromote) | via `attrib-family.json` | ok |
| attribtransfer | shipped (sop-attribtransfer) | via `attrib-family.json` | ok |
| bend | shipped (sop-bend) | `bend.json` | ok |
| blast | shipped (sop-blast) | via `group-blast.json` | ok |
| block_begin | shipped (sop-foreach) | via `foreach.json` | ok |
| block_end | shipped (sop-foreach) | via `foreach.json` | ok |
| boolean | partial (sop-boolean) | `boolean.json` | ok |
| box | shipped (sop-box) | via `attribcreate.json` | ok |
| circle | shipped (sop-circle) | via `curves.json` | ok |
| clip | partial (sop-clip) | `clip.json` | ok |
| color | shipped (sop-color) | `color.json` | ok |
| connectivity | shipped (sop-connectivity) | `connectivity.json` | ok |
| convert | partial (sop-convert) | `convert.json` | ok |
| copytopoints | partial (sop-copytopoints) | `copytopoints.json` | ok |
| curve | partial (sop-curve) | via `curves.json` | ok |
| delete | shipped (sop-delete) | `delete.json` | ok |
| divide | partial (sop-divide) | `divide.json` | ok |
| facet | partial (sop-facet) | `facet.json` | ok |
| file | shipped (sop-file) | via `file-rop.json` | ok |
| fuse | shipped (sop-fuse) | `fuse.json` | ok |
| grid | shipped (sop-grid) | via `attrib-family.json` | ok |
| group | shipped (sop-group) | via `group-blast.json` | ok |
| groupdelete | shipped (sop-groupdelete) | via `group-blast.json` | ok |
| grouppromote | shipped (sop-grouppromote) | via `group-blast.json` | ok |
| knife | partial (sop-knife) | `knife.json` | ok |
| line | shipped (sop-line) | via `bend.json` | ok |
| matchsize | shipped (sop-matchsize) | `matchsize.json` | ok |
| measure | shipped (sop-measure) | `measure.json` | ok |
| merge | shipped (sop-merge) | via `connectivity.json` | ok |
| mirror | shipped (sop-mirror) | `mirror.json` | ok |
| mountain | partial (sop-mountain) | `mountain.json` | ok |
| normal | shipped (sop-normal) | `normal.json` | ok |
| null | shipped (sop-registry) | via `switch.json` | ok |
| output | shipped (sop-output) | via `switch.json` | ok |
| peak | shipped (sop-peak) | `peak.json` | ok |
| polybevel | partial (sop-polybevel) | `polybevel.json` | ok |
| polyextrude | shipped (sop-polyextrude) | `polyextrude.json` | ok |
| polyfill | partial (sop-polyfill) | `polyfill.json` | ok |
| polyreduce | partial (sop-polyreduce) | `polyreduce.json` | ok |
| ray | shipped (sop-ray) | `ray.json` | ok |
| resample | shipped (sop-resample) | via `curves.json` | ok |
| reverse | shipped (sop-reverse) | `reverse.json` | ok |
| revolve | partial (sop-revolve) | `revolve.json` | ok |
| rop_geometry | partial (rop-geometry) | via `file-rop.json` | ok |
| scatter | partial (sop-scatter) | `scatter.json` | ok |
| skin | partial (sop-skin) | `skin.json` | ok |
| smooth | partial (sop-smooth) | `smooth.json` | ok |
| sort | shipped (sop-sort) | `sort.json` | ok |
| sphere | shipped (sop-sphere) | via `convert.json` | ok |
| subdivide | partial (sop-subdivide) | `subdivide.json` | ok |
| sweep | partial (sop-sweep) | `sweep.json` | ok |
| switch | shipped (sop-switch) | `switch.json` | ok |
| twist | shipped (sop-twist) | `twist.json` | ok |
| uvlayout | partial (sop-uvlayout) | via `uv.json` | ok |
| uvproject | shipped (sop-uvproject) | `uvproject.json` | ok |
| uvunwrap | shipped (sop-uvunwrap) | via `uv.json` | ok |
| wrangle | shipped (sop-wrangle) | `wrangle.json` | ok |
| xform | shipped (sop-transform) | via `boolean.json` | ok |

Totals: 63 SOPs · 63 covered (37 dedicated + 26 via) · parity 42 shipped / 21 partial / 0 planned / 0 no-row · renders 44/44 exit 0.
Gaps: 0 SOP w/o example. Partial = ledger `status=partial`, see `notes` in ledger.
