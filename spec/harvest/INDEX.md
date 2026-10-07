# spec/harvest INDEX  (wave1/harvest, MODE=pirate, 2026-10-07)

Tag [H] = harvested from on-disk Houdini 20.5.487 (no hython, no licence). `$H`=`/Applications/Houdini/Current/Frameworks/Houdini.framework/Versions/Current`. Resources=`$H/Resources`, dylibs=`$H/Libraries`.

## Files
| file | what | source |
|---|---|---|
| `parms-box.md` | 14 parm templates | `libHoudiniOP2.dylib:42504445` (SOP_Box.C) |
| `parms-grid.md` | 12 | `libHoudiniOP2.dylib:42859284` (SOP_Grid.C) |
| `parms-sphere.md` | 16 | `libHoudiniOP2.dylib:43449449` (SOP_Sphere.C) |
| `parms-transform.md` | 27 | `libHoudiniOP2.dylib:43571261` (SOP_Transform.C, type `xform`) |
| `parms-merge.md` | 0 (negative) | none found; help has no `@parameters` |
| `parms-scatter.md` | 43 | `libHoudiniOP2.dylib:43406793` (SOP_Scatter2.C) |
| `parms-copytopoints.md` | v2 16 (+multiparm kids) · v1 21 | `libHoudiniOP2.dylib:42587532` (CopyToPoints-2.0.C) · `:42583741` (v1) |
| `parms-wrangle.md` | 23 | `OPlibSop.hda:1441556` (Sop/attribwrangle DialogScript) |
| `parms-attribnoise.md` | `::2.0` 82 · v1 41 | `OPlibSop.hda:1197867` (::2.0) · `:1137649` (v1) |
| `data-model.md` | GA/GEO/GU data model, 96 lines | `$H/Resources/toolkit/include/GA/*.h, GEO/*.h, GU/*.h` |
| `cook-idiom.md` | cook structure, modes, locking, errors, data-ids | `$H/Resources/toolkit/samples/SOP/{SOP_Star,SOP_PointWave,SOP_Flatten,...}` + `include/SOP/` |
| `CORRECTIONS.md` | harvest vs wave1/spec contradictions, per SOP | all of the above + `nodes.zip` |

## Parm counts (harvested templates incl. multiparm kids & buttons vs help-doc entries)
| SOP | [H] DS | doc (`nodes.zip`) | note |
|---|---|---|---|
| box | 14 | 13 | doc merges dodivs+divs |
| grid | 12 | 12 | |
| sphere | 16 | 17 | doc has "U Wrap", DS has none |
| transform | 27 | 24 | +movecentroid/movepivot buttons, ± split parms |
| merge | 0 | 0 | |
| scatter | 43 | 30 | doc nests relax/toggle+field pairs |
| copytopoints (v2) | 16 | 13 | +useimplicitn, multiparm kids (4) |
| wrangle | 23 | 16 | folders/multiparm kids/vex_* extras |
| attribnoise::2.0 | 82 | 38 | doc folds per-component strips |
("doc count" = heading entries at top indent of `@parameters`; heuristic, ±3.)

## Where the SOP parms live (the answer)
- **Built-in C++ SOPs** (box, grid, sphere, xform, scatter, copytopoints, …): plain-text DS in `Libraries/libHoudiniOP2.dylib` (66 MB), `__cstring`-like NUL-terminated blobs, **not** in OP3. Layout: `\n{\n    name    box\n    parm {` … up to NUL. Preceded by the source-file literal (`SOP_Box.C`) → grep/\x00-scan: find `parm {` runs, look 1–6 NUL-strings back for `SOP_*.C`. Other op families' DS sit in neighbouring libs (OP1 holds e.g. `COP_DenoiseAI.C`; OPZ has a `wrangle` DS, not the SOP one).
- **HDA-based SOPs** (attribwrangle, attribnoise, mountain, ripple, …; 365 entries): `Resources/houdini/otls/OPlibSop.hda` = `INDX` container. Format: `INDX`,u32 0,u32 0,u32 count(BE), then `count`×[u32 namelen, name, u32 off, u32 size, u32 mtime] (all big-endian); data base = end-of-index (12761 for this file) + off; each node blob is itself `INDX` with sections `DialogScript`, `CreateScript`, `TypePropertiesOptions`, `Help`, `Contents.gz`, …; `DialogScript` is plain text.
- `houdini/**/*.ds` (94 files on disk) = only autorig, config/Scripts (renderer shaders), vop include, vex/{Sop,Cop2,Chop,Surface}, properties/{obj,top,lop,op,vop}, soho/parameters, `public/dsencoder/example.ds`. **ZERO SOP node parms among them** (`vex/Sop/{null,restpos}.ds` = VEX-operator DS, not our 9). Not harvested further.
- DS grammar seen: `parm { name label type size default{} range{} menu{} disablewhen hidewhen export cppname joinnext nolabel parmtag{} }`, containers `group|groupsimple|groupcollapsible { name label }`, `multiparm { name default parm{} }`; `range { a! b }` `!`=locked.

## Provenance / reproduce
- sha256[:16]: libHoudiniOP2.dylib `01ebc06640c7d9d9`, OPlibSop.hda `1234eab8572a514b`, nodes.zip `5c5d5818d4eed8cf` (full hashes: re-run `shasum -a 256`).
- Extraction was scripted ad hoc (python under `.scratch/`, gitignored-by-omission, NOT committed: repo is Rust-only). Reproduce = byte-range reads above (file:offset in each table header, DS text NUL-terminated) — a Rust `xtask harvest` can replace the scripts if wanted.
- Raw DS text is NOT committed (tables only); each table header carries offset + DS sha256[:16] for re-verification.

## Still [?] (NOT recoverable from disk without hython)
- Merge: any hidden/legacy parms; attribute-conflict, group-merge, detail-attr rules.
- All cook OUTPUT facts: point/prim/vertex counts, attr/group lists, winding, index order (spec's `[R]` numbers e.g. box 8/6/24, grid 100/81) — behavioural, needs oracle fixtures.
- Exact noise functions/tables (simplex, perlin, worley, sparse convolution), exact RNG of Scatter and its relax algorithm, Copy-to-Points matrix composition order & attr-transfer arithmetic.
- Attribute tuple sizes/typeinfo of standard names (P=3? orient=4?) are [R] — header lists names only. Packed prim runtime ids (no factory id).
- DS `default` for ordinal menus is sometimes an index (`0`) rather than token (attribnoise `noiserange`=0 → first menu entry `positive`; copytopoints `sourcegrouptype`=0 → `guess`).
- Compiled-in (not DS) behaviour: enablement beyond `disablewhen`/`hidewhen`, callbacks, warnings/errors text beyond string tables (e.g. box: "Changed divisions to match order.", "Can't generate so many points" seen in `libHoudiniOP2.dylib` strings near SOP_Box.C).
- Doc drift: help is 2024-05-28, binaries 2025-01-23 (see CORRECTIONS DOC-DRIFT).
