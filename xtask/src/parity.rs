//! `cargo xtask parity`: validate parity/features.toml (the ledger) and print per-tier progress.
//! A `shipped` row must cite evidence `path::fn` (file exists and defines `fn <name>`) and every command it lists
//! must appear as a quoted string literal in apps/ or crates/. Unknown fields are errors (deny_unknown_fields).
use crate::{Res, root};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub schema: Schema,
    #[serde(default)]
    pub feature: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    pub version: u32,
    #[serde(default)]
    pub areas: BTreeMap<String, String>,
}

#[allow(dead_code)] // title is schema documentation
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub id: String,
    pub title: String,
    pub area: String,
    /// P0..P3 or N/A
    pub tier: String,
    /// planned | partial | shipped | na
    pub status: String,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

const TIERS: &[&str] = &["P0", "P1", "P2", "P3", "N/A"];
const STATUSES: &[&str] = &["planned", "partial", "shipped", "na"];

/// Source text of every .rs file under apps/ and crates/ (path, text).
fn sources(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("apps"), root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs")
                && let Ok(t) = std::fs::read_to_string(&p)
            {
                out.push((p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/"), t));
            }
        }
    }
    out
}

pub fn validate(l: &Ledger, src: &[(String, String)]) -> Vec<String> {
    let mut errs = Vec::new();
    let mut seen = BTreeSet::new();
    if l.schema.version != 1 {
        errs.push(format!("schema.version {} unsupported (want 1)", l.schema.version));
    }
    for f in &l.feature {
        let id = &f.id;
        if !seen.insert(id.clone()) {
            errs.push(format!("{id}: duplicate id"));
        }
        if !TIERS.contains(&f.tier.as_str()) {
            errs.push(format!("{id}: tier `{}` not in {TIERS:?}", f.tier));
        }
        if !STATUSES.contains(&f.status.as_str()) {
            errs.push(format!("{id}: status `{}` not in {STATUSES:?}", f.status));
        }
        if !l.schema.areas.is_empty() && !l.schema.areas.contains_key(&f.area) {
            errs.push(format!("{id}: area `{}` not in [schema].areas", f.area));
        }
        if f.status == "partial" && f.notes.trim().is_empty() {
            errs.push(format!("{id}: partial rows must say in notes what is missing"));
        }
        if f.status != "shipped" {
            continue;
        }
        if f.evidence.is_empty() {
            errs.push(format!("{id}: shipped row without evidence"));
        }
        for ev in &f.evidence {
            let Some((path, func)) = ev.split_once("::") else {
                errs.push(format!("{id}: evidence `{ev}` is not path::fn"));
                continue;
            };
            let needle = format!("fn {func}");
            let found = src.iter().any(|(p, t)| p == path && t.contains(&needle));
            if !found {
                errs.push(format!("{id}: evidence `{ev}` not found (no `{needle}` in {path})"));
            }
        }
        for cmd in &f.commands {
            let lit = format!("\"{cmd}\"");
            if !src.iter().any(|(_, t)| t.contains(&lit)) {
                errs.push(format!("{id}: command {lit} not found in apps/ or crates/"));
            }
        }
    }
    errs
}

/// (tier, shipped, counted) with `na` rows excluded from the denominator.
pub fn progress(l: &Ledger) -> Vec<(&'static str, usize, usize)> {
    TIERS
        .iter()
        .map(|t| {
            let rows = l.feature.iter().filter(|f| f.tier == *t && f.status != "na");
            (*t, rows.clone().filter(|f| f.status == "shipped").count(), rows.count())
        })
        .collect()
}

pub fn load(root: &Path) -> Result<Ledger, String> {
    let p = root.join("parity/features.toml");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
    toml::from_str(&text).map_err(|e| format!("parse {}: {e}", p.display()))
}

pub fn run(_: &[String]) -> Res {
    let root = root();
    let ledger = load(&root)?;
    let errs = validate(&ledger, &sources(&root));
    println!("{:<6} {:>8} {:>6}", "tier", "shipped", "%");
    for (tier, done, total) in progress(&ledger) {
        let pct = if total == 0 { 0.0 } else { 100.0 * done as f64 / total as f64 };
        println!("{tier:<6} {done:>4}/{total:<3} {pct:>5.1}");
    }
    for e in &errs {
        println!("  - {e}");
    }
    if errs.is_empty() {
        println!("parity: OK, {} rows", ledger.feature.len());
        Ok(())
    } else {
        Err(format!("{} ledger error(s)", errs.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger(extra: &str) -> Ledger {
        let t = format!(
            "[schema]\nversion = 1\n[[feature]]\nid = \"a.b\"\ntitle = \"t\"\narea = \"A\"\ntier = \"P0\"\n{extra}"
        );
        toml::from_str(&t).unwrap()
    }
    fn src() -> Vec<(String, String)> {
        vec![("crates/core/src/lib.rs".into(), "fn real_test() {}\nconst C: &str = \"cmd\";".into())]
    }

    #[test]
    fn shipped_needs_existing_evidence_and_commands() {
        let ok =
            ledger("status = \"shipped\"\nevidence = [\"crates/core/src/lib.rs::real_test\"]\ncommands = [\"cmd\"]");
        assert!(validate(&ok, &src()).is_empty());
        let bad = ledger("status = \"shipped\"\nevidence = [\"crates/core/src/lib.rs::ghost\"]\ncommands = [\"nope\"]");
        assert_eq!(validate(&bad, &src()).len(), 2);
        assert_eq!(validate(&ledger("status = \"shipped\""), &src()).len(), 1);
    }

    #[test]
    fn unknown_fields_and_bad_enums_rejected() {
        let t = "[schema]\nversion = 1\n[[feature]]\nid = \"x\"\ntitle = \"t\"\narea = \"A\"\ntier = \"P0\"\nstatus = \"planned\"\nwat = 1";
        assert!(toml::from_str::<Ledger>(t).is_err());
        assert_eq!(validate(&ledger("status = \"done\""), &src()).len(), 1);
    }

    #[test]
    fn progress_excludes_na_rows() {
        let l = ledger("status = \"na\"");
        assert_eq!(progress(&l)[0], ("P0", 0, 0));
    }
}
