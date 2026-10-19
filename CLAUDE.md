# CLAUDE.md: session protocol (hexe)
Read `AGENTS.md` first (wins on conflict; check `MODE:`). One session = one ledger atom.

1. STATUS: `git status`, `git log -5`, `cargo xtask ci` baseline (green before you touch anything).
2. ROADMAP gaps: read `ROADMAP.md` honest table + last session-log entry → pick the highest-tier gap.
3. Ledger row: pick ONE `parity/features.toml` row (P0 first; `planned`/`partial`). No row → add it first.
4. Orient: read `spec/` section + `fixtures/<case>/` for the row (MODE=clean: nothing else; gap → `SPEC-GAP: <q>` and stop). Pirate: dirty/ allowed.
5. Plan: 3 lines max in the commit message draft; no plan files outside `plan/` (git-ignored).
6. Implement + test: smallest slice; test first for hostile input; evidence fn name = `path::fn` for the ledger.
7. Verify: `cargo xtask ci` GREEN + the real thing run (`cargo run -p hexe-cli -- …`; app launched for UI work). Never claim unrun.
8. Record: ledger row status/evidence/notes · oracle `--raise` if score rose · `perf --tighten` if faster · ROADMAP session-log line (date, agent, hours, atom, result) · honest table if numbers moved.
9. Commit: `<ledger id>: <what>`; green only; one atom per commit.

PORTABILITY (anything that can be committed: code, tests, fixtures, oracle scripts, ledger, ROADMAP, docs): nothing
specific to the maintainer or their machine. No user names, home / Desktop paths, drive letters of a personal box,
hostnames, or the names / contents of the maintainer's own `.hip` / `.hipnc` scenes (no geometry exported from them,
no node names or parm values copied from them). Fixtures and tests must be built from synthetic inputs (hython scripts
that construct the case) so they run on any computer; a private scene may be used to FIND a gap, never as evidence.
Tool paths (hython, Houdini install) come from an env var / argument, never hardcoded.

STOP conditions: `SPEC-GAP` (clean) · ci red you can't fix in the atom · scope creep → write it in ROADMAP, don't do it.
