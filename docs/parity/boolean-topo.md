# sop-boolean TOPO-DIFF (start vertex) — 2026-10-15, branch parity/boolean-topo
Status: NOT DERIVED, no engine change. hython BLOCKED this session (see §Blocker) → atom 2 offline-only.

## Atom 1 — diff summary (fixture sop-boolean = A minus box t(.5,.5,.5); == corpus bx2_2_2_sub)
Oracle path: `cargo xtask oracle --houdini` needs hython to load hexe .geo → all rows HEXE-ERR now (env). Offline diff of hexe cook vs `parity/ref/boolean/bx2_2_2_sub.topo`:
- P order: identical (14 pts). Prims 0,1,2,6,8 identical cycles.
- Differ (same cycle, rotated start): F3 H `7 8 9 1 0 6` / hexe `0 6 7 8 9 1`; F4 H `4 10 11 7 6 5` / hexe `6 5 4 10 11 7`; F5 H `4 2 1 9 12 10` / hexe `1 9 12 10 4 2`; F7 H `12 13 11 10` / hexe `11 10 12 13`.
- `cargo xtask boolstart` (128 split polys): best simple hypothesis 42/128 (last kept corner of kept run); best linear key 37/128; best axis-lex 39/128.

## Atom 2 — Houdini rule (corpus-only, parity/ref/boolean/*.topo + start-sweep.txt)
Tested this session, all REJECTED:
| hypothesis | result |
| start = lowest/highest input point id (box ids) | F4,F5 start at box pt 7 (highest) but F3 starts at a NEW point; boolstart 29-30/128 |
| first intersection along A edge 0 | F3 start = first new pt on cycle (7) but F4/F5 start at a kept corner |
| min (x,y,z) lexicographic | best of 144 orders 39/128 |
| Delaunay/in-circle flip of the L-hexagon | cocircular roots only at s=0; sweep thresholds 1/15,1/6,1/3,5/14,1/2,5/9 not reproduced |
Facts: sweep (A=unit box, B=box t(s,s,s), aminusb) flips start at identical topology at s=1/15,.167,1/3,.357,1/2,.556 → depends on continuous geometry (exact-predicate/hash/sort order inside Houdini), not topology or point id. Same-shape faces (A+x vs A+y, mirror ring) flip at DIFFERENT s → axis-asymmetric key.

## Blocker (hython)
`hython -c 'import hou'` → "No licenses could be found". Cause: licenses keyed to host `MacBookPro.fritz.box` (/Library/Preferences/sesi/licenses); box is now `Pascals-MacBook-Pro.local` @10.28.20.78; /etc/hosts maps MacBookPro.fritz.box → stale 192.168.178.49. Fix needs sudo (hosts entry → current IP, or scutil --set HostName MacBookPro.fritz.box). No sudo in lane.

## Next (when hython works)
`hython oracle/houdini/boolean-order-ref.py` extended: dense sweep s over 0..1 for all 6 face kinds, dump P + prim cycles, log per-face start vs (x,y,z) of every vertex; fit key (candidates: float32 hash of P, sort by (x,y,z) after axis permutation per face normal, intermediate point ids new=creation order). Implement in `crates/sop/src/builtins/boolean_sop.rs` (crate hexe-sop, no `sop-boolean` crate) only with rule fitting ALL sweep samples; no per-case hack.


## Atom 3 (2026-10-15, branch parity/oracle-corpus) — rule READ FROM THE INSTALL, not fitted. RESULT: NOT IMPLEMENTED, rule not closed
Install = oracle. HDK has NO GU_Boolean headers (toolkit/include/GU has no *Bool*, GEO has none) → implementation only in `Frameworks/Houdini.framework/Versions/Current/Libraries/libHoudiniGEO.dylib` (x86_64 Mach-O; `nm -C`). No Ghidra project in ~/.gaia/cache/tools (only `ghidraRun` brew stub, not used); `objdump --macho` ignores --start-address on this build → full-dylib dump = 3.5 min/35 M lines, address range sliced with sed.
### Symbols (GU_Boolean, libHoudiniGEO)
| addr | symbol | role |
|---|---|---|
| 1e45ee0 | GU_Boolean(GU_Detail*, GA_PrimitiveGroup const*×2, bool×3) | ctor |
| 1e4bb30 | resolveMeshIntersections | arrangement of A,B |
| 1e4c200 | extractResolvedMesh | triangulated arrangement out |
| 1e46240 | calculateCellWindings | winding per cell |
| 1e4e320 | assignSubsetIndices(SubsetIndexMap&, UT_Array<GU_Delaunay::FlaggedIntT<ll>> const&) | |
| 1e4e470 | classifyTrisIntoVirtualPolys(Detriangulation, SubsetIndexMap const&, …, UT_BitArray&) | merges result tris into "virtual polys" (detriangulate); inner sort `$_4` @1e5f720 over `hedge_data` |
| **1e50e10** | **extractVirtualPolys(SubsetIndexMap&, UT_Array<FlaggedIntT<ll>> const&, Detriangulation, bool, bool, UT_Array<VpolyVertex>&, UT_ValArray<ll>&, UT_ValArray<ll>&)** | **output-polygon vertex emitter = the start-vertex site** |
| 1e54720 | buildFacets(…) (sort `$_11` on FlaggedIntT) | facet list per op |
| 1e5a130 / 1e58a00 | generateFacets / generateCells | per-op output |
| 1e5b810 | generateSeams | |
SOP side: `SOP_Boolean::cookMySop` libHoudiniOP2 @335c30 (thin; verb `SOP_BooleanVerb`). Excerpt of the builder (first 60 insns) + comparator: `docs/parity/boolean-topo-disasm.txt`.
### What the code says (read, with offsets)
1. `extractVirtualPolys` 1e50e10: `UTgetInterrupt` → opStart `"Detriangulating"` (Detriangulation≠0) else `"Generating Triangles"` (1e50e51); calls `assignSubsetIndices` (1e50eb4), builds `UT_BitArray(3*N)` (1e50f00), calls `classifyTrisIntoVirtualPolys` (1e50f21) → `UT_Classifier`, `UT_Classifier::buildIndex(false,1)` (1e50f34): vpoly id → member list.
2. Outer loop 1e50ff9..1e52254: iterates the **FlaggedIntT facet list in array order** (`movq (%rax,%rcx,8),%r14` @1e5100e; `>>1` = facet/vpoly id, bit0 = orientation flag XORed with the per-prim flag at 1e510f0).
3. Per vpoly: pre-loop 1e51195..1e51193 clears a `UT_BitArray` bit (-0x110(%rbp)) for every member hedge; main loop 1e5127b..1e51250 walks the vpoly's member hedge list **from index 0 upward** (`-0xc0(%rbp)` counter from 0, `(%rax,%rcx,8)` @1e512c0); a hedge whose bit is already set is skipped (`btq`/`jb 1e51250` @1e51365), else bit set + hedge appended to the emitted ring (`UT_Array<ll>::setCapacity` @1e51444, `incq 0x10(%r13)` @1e5146d) and the ring walk continues (calls `determineSeamType(GEO_Hedge)` @1e51bbc).
   ⇒ **start vertex = vertex of the FIRST not-yet-visited hedge (tail/head side NOT read) in the vpoly's member-hedge list order.** No angle, no axis, no longest-edge, no curve-origin test appears in the emitter.
4. Member-hedge list order is produced by `classifyTrisIntoVirtualPolys` (1e4e470..1e50400): hedge_data{hedge@0, int@+8, float@+0xc} filled (float = `sqrtss` of a sum of three squares, 1e4edbb..1e4ede4), `introsort` with `$_4` (1e5f720): lexicographic `int@+8` → `float@+0xc` (ucomiss) → an int point/vertex attribute read through the hedge → … (insns after 1e5f836 not fully read). This sort pairs hedges (interior edge cancels), the surviving boundary hedges keep **triangle (arrangement-triangulation) order**.
### Conclusion (UNVERIFIED beyond the reading)
Start = first surviving boundary hedge of the vpoly's first triangle in the arrangement's triangle order = order of the `GU_Delaunay` triangulation of each source face (+ cut chain). Triangle order is a function of the continuous Delaunay/in-circle decisions → explains flips at s=1/15,1/6,1/3,5/14,1/2,5/9 at identical topology and axis asymmetry. The three candidate rules (smallest angle to axis / first along longest edge / nearest curve origin) are therefore NOT what Houdini does; none scored. Implementing the real rule = port `GU_Delaunay` triangle emission order + `resolveMeshIntersections` fragment order (symbols in libHoudiniGEO, not read): out of the 25 min budget.
### Next
- read `GU_Delaunay` (nm -C libHoudiniGEO | grep GU_Delaunay) triangle output order + `GU_Boolean::extractResolvedMesh` 1e4c200; finish `$_4` tail (1e5f836+)
- then rule = (Delaunay tri order of the face) → first boundary hedge; verify on 128 samples (`cargo xtask boolstart`) before touching `crates/sop/src/builtins/boolean_sop.rs`
- engine unchanged this branch; `cargo test -p hexe-sop boolean` not re-run (no change).
