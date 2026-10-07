MODE: pirate   (pirate (default) | clean)

# AGENTS.md: hexe (port of Houdini)
This file wins over every other instruction, role or summon text on conflict. Pipeline: `~/.gaia/skills/clean-room-port/SKILL.md`.

## 0. Mode
- `MODE: pirate` (default) → implementers MAY read `dirty/`, aletheia `out/`, the original binary, decompile output. §1 wall rules do NOT apply. §2–§9 do.
- `MODE: clean` → §1 applies in full; `cargo xtask ci` also runs `leakscan` on `spec/`.
- `.gitignore` ignores `dirty/` in both modes (size, not law).
- Mode is read by `xtask` from the first lines of this file. Change it here only, and log it in PROVENANCE.md.

## 1. Wall (MODE=clean only)
1. Repo never contains/links/cites the original binary, decompile output, screenshots of the original, or `dirty/`.
2. Clean lanes see `./` + `spec/` + `fixtures/` ONLY. NEVER open `~/projects/hexe-spec/dirty/`, any `.exe/.app/.so` of the original, or aletheia `out/`. Fact missing from `spec/` → STOP, return `SPEC-GAP: <question>`.
3. Spec = prose + tables + fixtures. No code, no decompiler identifiers (`sub_`, `dword_`, `__thiscall`, `this+0x`), no source paths, no quoted original strings > 12 words. Format constants (magic, enums, offsets, opcodes) are interface facts: allowed, tabled.
4. A lane is never in both rooms in one life. Publishing spec/fixtures into this repo = monad only, logged in PROVENANCE.md.

## 2. Rust only
Product, xtask, tools: Rust. No python/JS/TS anywhere in the repo. (`tools/` is empty; the only shell script is the skill-side `new-port.sh`, outside the repo.) New tooling = an `xtask` subcommand.

## 3. Oracle
Oracle = the original run on inputs WE authored → `fixtures/<case>/{case.toml,input,expected}` + `fixtures/MANIFEST.toml` (how produced, original's version, sha256). No corpus files with unclear rights, no personal data. Open-tool oracles (poppler, qpdf, ffmpeg…) = subprocess only, never linked.

## 4. Never crash (hostile input is the default)
`unsafe_code = forbid`; every crate: `#![deny(clippy::unwrap_used, expect_used, panic, unimplemented, todo, unreachable)]` (tests exempt via clippy.toml); checked arithmetic; capped allocations; bounded recursion; `catch_unwind` at command dispatch; every fuzz finding → synthetic regression test BEFORE the fix.

## 5. Ledger decides done
`parity/features.toml`: one row per feature (id, area, tier P0–P3, status planned|partial|shipped|na, commands, evidence `path::fn`, notes). `cargo xtask parity` fails a `shipped` row whose evidence/command doesn't exist. Shipped = exists + tested, not "as good as original". Oracle floors, perf ceilings, size max only ratchet the right way.

## 6. Provenance as code
`ATTRIBUTION.toml` (asset|bundled|fetched|build_input; author, source+version, SPDX, sha256) → `cargo xtask assets` fails on unlisted file, hash mismatch, off-allowlist licence, vendor-authored asset (`[policy] original_vendor`). `deny.toml` for crates. `.gitattributes * -text`. Vendored patches marked `<Name> patch:` + test per patch + upstream plan. Copyleft never vendored or linked.

## 7. Honesty
`ROADMAP.md` dated honest-assessment table + per-session log with agent-hours. No "100 % in a month". Fidelity is a measured number (oracle), never a claim.

## 8. Budgets
`perf/budgets.toml`: `[binary] max_mb` + `original_mb` (size ratio) and per-fixture `startup_ms/run_ms/peak_rss_mb`. `cargo xtask perf|size` gate both; baselines only tighten.

## 9. GAIA laws
Never /tmp · never `pkill -f` · one live test at a time · each lane own worktree + own `CARGO_TARGET_DIR` · trunk merges · commit per ledger id (`<id>: <what>`) · green only (`cargo xtask ci`).

## Layout
`crates/*` libs (layers in `xtask/src/layers.rs` TABLE) · `apps/hexe` egui window · `apps/hexe-cli` CLI + oracle entry · `xtask/` gates · `parity/` ledger · `fixtures/` oracle cases · `spec/` prose spec · `perf/` budgets.
Commands: `cargo xtask ci|layers|assets|parity|oracle|perf|size|deny|leakscan|version`.
