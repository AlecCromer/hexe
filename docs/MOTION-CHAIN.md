# motion SOP, mode=chain (Endorphin chain inside hexe)
Crate `crates/sop-motion` · registered by hexe-tauri (`app_registry`) + hexe-cli (`registry()`).
- params: `mode` push|chain · chain: `impulse_ns` (base shove N*s) · `seed` (-1 none = +X @0.5s; >=0 = gaia-motion `shove_params(seed)`: mag +-10%, yaw +-15 deg, t0 +-0.1s) · `duration_s` (1..60, default 12) · `fps` (geo resample + time->frame) · `frame` (0 = follow time, else 1-based) · `python_bin`/`motion_root`/`cache_dir`/`timeout_s`. `impulse_dir`/`impulse_body`/`scene`/`frames` = push only.
- cook: `python -m gaia_motion.cli chain --out <run> --ns --run-s --fps [--seed]` (gaia-motion `11275fc`; wraps `scripts/endorphin_chain.py run_chain`, limp 0.6 s, settle 0.5 s) -> `<cache>/<hash>/{geo/frame.%04d.geo, chain.npz (t,qpos,phase), chain.json}`; hash = FNV(ns, seed, duration, fps); atomic publish, failures leave nothing.
- default python missing -> `~/.gaia/cache/gaia-motion/venv/bin/python`.
- output Detail (frame `round(time*fps)+1`, clamped): 24 joint points `P` (Y-up) + string `name`; 23 polylines (`BONES`, mirror of `PAR` in gaia-motion `scripts/houdini_chain.py`); detail `pelvis_z` = pelvis `P.y`. Points identical to the exported .geo.
- scene: `demo/endorphin-chain.hexe` = motion1(chain, seed 8, ns 100, 12 s, 30 fps) -> output1.
- cook: `hexe-cli cook demo/endorphin-chain.hexe --out f.geo --time <(F-1)/30>`.
- render (headless, hexe's own viewport): `cd apps/hexe-tauri && bun run dev` (debug build loads devUrl :1520) then `target/debug/hexe-tauri demo/endorphin-chain.hexe --set motion1.frame=F --render F.png --size 640x360`. Sheet: gaia-motion `scripts/hexe_chain_sheet.py` -> `proof/endorphin-hexe.png`.
- parity: test `parity_real_chain_matches_sealed_houdini_chain_geo` (real run vs gaia-motion `proof/houdini-chain/geo/frame.*.geo`, <=1e-4; measured 0).
- chain caveat (gaia-motion): seed 8 stands at t=8.30 s (misses 8 s gate from run start); fragile across seeds (8/120).
