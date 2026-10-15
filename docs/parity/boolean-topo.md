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
