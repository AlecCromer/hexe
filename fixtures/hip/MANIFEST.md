# fixtures/hip — H40 `.hip` corpus
Source: Houdini 20.5.487 install `.../houdini/help/files/**/*.hip` (`help/examples/nodes/sop/**` ships ZERO .hip; only 3 under examples/nodes, none SOP → corpus = SOP-bearing files of `help/files`). Files NOT copied (install-gated tests, no-op when absent).
Parse = `read_hip` node+wire count == independent cpio count (`hip_corpus_parse_12`). Cook = `hexe-cli open` scene vs hython `oracle/houdini/hip-twin.py` .geo, display SOP (`hip_corpus_twin`, HEXE_HIP_TWIN_DIR).
Houdini frame/parm = value at save time; animated parms flagged.

| id | file (rel. help/files/) | geos | nodes | parse | cook verdict vs hython |
|----|----|----|----|----|----|
| 55 | expression_cookbook/AnimationCycleExample.hip | 1 | 1 | FULL | MATCH 1/1 |
| 42 | obj_xform.hip | 6 | 6 | FULL | MATCH 6/6 |
| 41 | xforms_from_disk.hip | 1 | 1 | FULL | MATCH 1/1 |
| 63 | file_to_load.hip | 2 | 2 | FULL | MATCH 2/2 |
| 31 | pdg_examples/top_rop/input/external.hip | 1 | 1 | FULL | MATCH 1/1 |
| 53 | expression_cookbook/RippleExample.hip | 2 | 10 | FULL | UNSUPPORTED-NODE (point) ×1; HEXE-ERR wrangle `relbbox` unknown fn ×1 |
| 60 | expression_cookbook/LogSpiral.hip | 3 | 10 | FULL | HEXE-ERR tube only `poly`; UNSUPPORTED-NODE material, log_spiral HDA, object_merge |
| 57 | expression_cookbook/ModulusExample.hip | 2 | 14 | FULL | UNSUPPORTED-NODE copy (stamp) ×2 |
| 14 | pdg_examples/top_roadbuilder/roadbuilder_tile.hip | 3 | 20 | FULL | MATCH road_curve (1/3); HEXE-ERR block_end without block_begin ×2 (for-each block not modelled) |
| 23 | pdg_examples/top_invoke/invoke_lite.hip | 2 | 22 | FULL | MATCH source (1/2, 14 nodes); HEXE-ERR block_end ×1 |
| 45 | vary_per_copy.hip | 1 | 10 | FULL | UNSUPPORTED-NODE grouprange |
| 43 | color_falloff.hip | 1 | 2 | FULL | UNSUPPORTED-NODE color_falloff (HDA) |

Totals: parse 12/12 FULL graph; cook 25 geo containers = 13 MATCH / 5 HEXE-ERR / 7 UNSUPPORTED-NODE; files with ≥1 MATCH: 7/12, files fully MATCH: 5/12. 0 PARSE-ERR, 0 diff-after-cook (no P/TOPO/ATTR-DIFF).
Caveat: 11 of 13 MATCHes are 1-node box/torus/grid geos (object-level xforms); 2 are multi-node (invoke_lite `source` 14 nodes/14 wires; roadbuilder `road_curve`).
Own fixtures: authored-chain.hip(.gz) (box,grid,xform,merge; hython-authored, `.contents` = real `hexpand` listing, `.houdini.geo` twin) MATCH; authored-unsup.hip (point SOP → `unsupported:point`) cooks Unsupported.
MIME-wrapped .hip: rejected with explicit error (never panic).
Regenerate: `hython oracle/houdini/hip-twin.py list.tsv outdir` + `HEXE_HIP_TWIN_DIR=outdir cargo test -p hexe-sop hip_corpus_twin`.
