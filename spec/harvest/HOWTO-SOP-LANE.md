# HOWTO-SOP-LANE: add one SOP (box, grid, …) to crates/sop
Prereq: worktree `~/projects/hexe-wt-<sop>` from main, own `CARGO_TARGET_DIR`. Inputs: `spec/harvest/parms-<sop>.md`, `spec/behaviour/sop-<sop>.md`, `spec/harvest/cook-idiom.md`.
## 1 File + registry (touch NOTHING else in crates/sop)
- Create `crates/sop/src/builtins/<sop>_sop.rs` (`box` → `box_sop.rs`; Rust keywords force the suffix). Copy `null_sop.rs` shape: `pub fn spec() -> Vec<ParamSpec>`, `pub fn new() -> Box<SopNode>`, a private struct holding `specs: Vec<ParamSpec>`, `impl Node<Detail>`.
- `type_name()` == registered token == Houdini internal name (`"box"`). `inputs()` = declared input ports in order (generator: `&[]`).
- `builtins/mod.rs`: uncomment YOUR two pre-seeded lines (`// mod <sop>_sop;` and `// r.register("<sop>", <sop>_sop::spec, <sop>_sop::new);`). Not listed → insert in sorted slot between `//` separator lines. Never reorder/format other lines.
- Registry contract is auto-tested (`registry::tests::every_builtin_is_self_consistent`): unique tokens, `params()==spec()`, defaults acceptable.
## 2 Param mapping (parms-<sop>.md table → `ParamSpec`)
- `ParamSpec::new(<token>, default).label(<label>).range(lo, hi)` per row; skip rows the lane's behaviour spec excludes (say which in the ledger notes).
- type column: float→`F` · integer→`I` · toggle(on/off)→`B` · vector[3]→`V3` · integer[3]/intvector[3]→`V3` (no I3 variant: round on read, doc it) · string→`S` · ordinal/menu→`Menu(i)` (i = position in the table's `tok=Label` list, 0-based; scene JSON passes the integer).
- `range` `N!` = hard clamp → cook returns `CookError::BadParam` (or clamps if behaviour spec says so); no `!` = UI hint only (`.range()` still set).
- Read params in cook via `ctx.params.float("t")` etc.; missing/wrong-typed → `CookError::BadParam`. NEVER unwrap/expect/panic/index (crate denies it). Cap allocations (e.g. `divrate` product) → `CookError::Failed`.
## 3 Cook rules (cook-idiom.md §3§10)
- Generator: build fresh `Detail` (contiguous point block, `add_poly(.., closed)`); modifier: clone input 0 (`(*inputs[0]).clone()`), edit, `Arc::new`.
- Unconnected inputs reach cook as an EMPTY Detail (scene loader), so empty input must work, not panic.
## 4 Tests (same file, `#[cfg(test)]`)
- Pattern: build scene JSON string → `crate::load_scene(json)` → `graph.cook(id, 0.0)` → assert counts, specific `P` values from closed-form maths, attr names; plus params-by-token override, hostile params (negative divs, huge divs) = `Err`.
- Evidence for the ledger = `crates/sop/src/builtins/<sop>_sop.rs::<test_fn>`.
## 5 Oracle fixture (analytical expected)
- `fixtures/<sop>-<case>/{scene.json, expected.geo, case.toml}`; `case.toml` = copy `fixtures/null-passthrough/case.toml`, set `feature = "sop-<sop>"`, `compare = "geo"` (counts, P ± 1e-5, attr names).
- `expected.geo`: the test builds the expected `Detail` by INDEPENDENT closed-form maths (not by calling the SOP), writes it with `hexe_geo::write_geo_json`; commit the file via an `#[ignore]` test (`cargo test -- --ignored write_fixture`) and add a normal test asserting the SOP output `read_geo`-equals it. Never generate expected.geo with `hexe-cli cook` (circular).
- `fixtures/MANIFEST.toml`: one `[[fixture]]` per file, `produced_by = "analytical"`, `original_version = "n/a"`, sha256 (`shasum -a 256`). Houdini-produced expecteds are NOT available (hython blocked).
## 6 Ledger + gate
- `parity/features.toml`: row `sop-<sop>` planned→shipped, `commands = ["<sop>"]`, evidence paths above, notes = what is UNVERIFIED vs Houdini.
- `cargo xtask ci` GREEN (fmt, clippy -D warnings, test, layers, assets, parity, oracle N/N); then `cargo xtask oracle --raise`. Commit per stage `sop-<sop>: <what>`. No merge; monad merges.
- Merge conflicts expected only in `parity/features.toml`, `fixtures/MANIFEST.toml`, `Cargo.lock` (append-only/regenerable): resolve by keeping both sides.
