//! `cargo xtask assets [--write]`: provenance as code (AGENTS.md §7).
//! Every binary/asset file outside fixtures/ needs an `[[asset]]` row in ATTRIBUTION.toml with matching sha256 and an
//! allowlisted SPDX licence; no row may be authored by `[policy] original_vendor`; `[[bundled]]` crates must be in
//! Cargo.lock; fixtures/MANIFEST.toml hashes must match. ATTRIBUTION.md is generated (`--write`) and must be fresh.
use crate::{Res, root};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

/// Licences (SPDX ids) accepted for assets and bundled data. Copyleft is never on this list.
pub const ALLOWED: &[&str] = &[
    "MIT",
    "MIT-0",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "BSL-1.0",
    "CC0-1.0",
    "CC-BY-4.0",
    "OFL-1.1",
    "Ubuntu-font-1.0",
];
/// Extensions that count as assets (binary or media); any such file in the repo needs an ATTRIBUTION row.
const ASSET_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "icns", "svg", "ttf", "otf", "woff", "woff2", "pdf", "icc",
    "p12", "wav", "mp3", "ogg", "mp4", "onnx", "bin", "dat", "zip", "gz", "wasm",
];
const SKIP_DIRS: &[&str] =
    &["target", ".git", "dirty", "plan", "corpus", "fuzz-out", "dist", "fixtures", "spec", "node_modules"];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Case-insensitive author substrings that may never appear (the original's vendor).
    #[serde(default)]
    pub original_vendor: Vec<String>,
}

#[allow(dead_code)] // schema fields (url, kind, usage) are documentation; deny_unknown_fields needs them declared
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: Option<String>,
    #[serde(rename = "crate")]
    pub krate: Option<String>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub url: Option<String>,
    pub title: String,
    pub author: String,
    pub source: String,
    pub licence: String,
    pub kind: Option<String>,
    pub usage: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attribution {
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub asset: Vec<Entry>,
    #[serde(default)]
    pub bundled: Vec<Entry>,
    #[serde(default)]
    pub fetched: Vec<Entry>,
    #[serde(default)]
    pub build_input: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(default)]
    fixture: Vec<FixtureRow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureRow {
    case: String,
    file: String,
    sha256: String,
    produced_by: String,
    original_version: String,
    date: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Every id in an SPDX expression must be allowed (`AND`), or at least one alternative (`OR`).
pub fn licence_ok(expr: &str) -> bool {
    expr.split(" AND ").all(|conj| {
        conj.split(" OR ").any(|alt| {
            let id = alt.trim().trim_matches(|c| c == '(' || c == ')');
            ALLOWED.contains(&id.split(" WITH ").next().unwrap_or(id).trim())
        })
    })
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(&p, root, out);
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| ASSET_EXT.contains(&x.to_ascii_lowercase().as_str()))
        {
            out.push(p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/"));
        }
    }
}

pub fn check(root: &Path, a: &Attribution, files: &[String], lock: &str) -> Vec<String> {
    let mut errs = Vec::new();
    let listed: BTreeSet<&str> = a.asset.iter().filter_map(|e| e.path.as_deref()).collect();
    for f in files.iter().filter(|f| !listed.contains(f.as_str())) {
        errs.push(format!("{f}: asset file without an [[asset]] row in ATTRIBUTION.toml"));
    }
    let all = a.asset.iter().chain(&a.bundled).chain(&a.fetched).chain(&a.build_input);
    for e in all {
        let who = e.path.as_deref().or(e.krate.as_deref()).or(e.name.as_deref()).unwrap_or(&e.title);
        if !licence_ok(&e.licence) {
            errs.push(format!("{who}: licence `{}` is not on the allowlist", e.licence));
        }
        let author = e.author.to_lowercase();
        if let Some(v) = a.policy.original_vendor.iter().find(|v| !v.is_empty() && author.contains(&v.to_lowercase())) {
            errs.push(format!("{who}: authored by the original's vendor (`{v}`): forbidden"));
        }
    }
    for e in &a.asset {
        let (Some(p), Some(want)) = (e.path.as_deref(), e.sha256.as_deref()) else {
            errs.push(format!("{}: [[asset]] needs path and sha256", e.title));
            continue;
        };
        match std::fs::read(root.join(p)) {
            Ok(b) if sha256_hex(&b) == want => {}
            Ok(_) => errs.push(format!("{p}: sha256 mismatch with ATTRIBUTION.toml")),
            Err(err) => errs.push(format!("{p}: listed but unreadable ({err})")),
        }
    }
    for e in &a.bundled {
        let Some(c) = e.krate.as_deref() else {
            errs.push(format!("{}: [[bundled]] needs `crate`", e.title));
            continue;
        };
        let hit = lock.split("[[package]]").find(|b| b.contains(&format!("name = \"{c}\"")));
        match (hit, e.version.as_deref()) {
            (None, _) => errs.push(format!("{c}: bundled crate not in Cargo.lock")),
            (Some(b), Some(v)) if !b.contains(&format!("version = \"{v}\"")) => {
                errs.push(format!("{c}: version {v} != Cargo.lock"))
            }
            _ => {}
        }
    }
    errs
}

fn manifest_errors(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("fixtures/MANIFEST.toml")) else {
        return vec!["fixtures/MANIFEST.toml missing".into()];
    };
    let m: Manifest = match toml::from_str(&text) {
        Ok(m) => m,
        Err(e) => return vec![format!("fixtures/MANIFEST.toml: {e}")],
    };
    let mut errs = Vec::new();
    for r in &m.fixture {
        if r.produced_by.is_empty() || r.original_version.is_empty() || r.date.is_empty() {
            errs.push(format!("fixture {}: produced_by/original_version/date must be filled", r.case));
        }
        match std::fs::read(root.join("fixtures").join(&r.case).join(&r.file)) {
            Ok(b) if sha256_hex(&b) == r.sha256 => {}
            _ => errs.push(format!("fixture {}/{}: missing or sha256 mismatch", r.case, r.file)),
        }
    }
    errs
}

pub fn render_md(a: &Attribution) -> String {
    let mut s = String::from(
        "<!-- generated by `cargo xtask assets --write` from ATTRIBUTION.toml; do not edit -->\n# Attribution\n\n",
    );
    s.push_str("| kind | what | author | source | licence |\n|---|---|---|---|---|\n");
    for (kind, rows) in
        [("asset", &a.asset), ("bundled", &a.bundled), ("fetched", &a.fetched), ("build_input", &a.build_input)]
    {
        for e in rows {
            let what = e.path.as_deref().or(e.krate.as_deref()).or(e.name.as_deref()).unwrap_or(&e.title);
            s.push_str(&format!("| {kind} | {what} | {} | {} | {} |\n", e.author, e.source, e.licence));
        }
    }
    s
}

pub fn run(args: &[String]) -> Res {
    let root = root();
    let text =
        std::fs::read_to_string(root.join("ATTRIBUTION.toml")).map_err(|e| format!("read ATTRIBUTION.toml: {e}"))?;
    let a: Attribution = toml::from_str(&text).map_err(|e| format!("ATTRIBUTION.toml: {e}"))?;
    let md = render_md(&a);
    if args.iter().any(|x| x == "--write") {
        std::fs::write(root.join("ATTRIBUTION.md"), &md).map_err(|e| e.to_string())?;
        println!("assets: wrote ATTRIBUTION.md");
    }
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).map_err(|e| format!("read Cargo.lock: {e}"))?;
    let mut errs = check(&root, &a, &files, &lock);
    errs.extend(manifest_errors(&root));
    if std::fs::read_to_string(root.join("ATTRIBUTION.md")).ok().as_deref() != Some(md.as_str()) {
        errs.push("ATTRIBUTION.md missing or stale: run `cargo xtask assets --write`".into());
    }
    for e in &errs {
        println!("  - {e}");
    }
    if errs.is_empty() {
        println!("assets: OK, {} asset files scanned", files.len());
        Ok(())
    } else {
        Err(format!("{} asset-policy error(s)", errs.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, sha: &str, licence: &str, author: &str) -> Entry {
        Entry {
            path: Some(path.into()),
            krate: None,
            version: None,
            name: None,
            url: None,
            title: "t".into(),
            author: author.into(),
            source: "s".into(),
            licence: licence.into(),
            kind: None,
            usage: None,
            sha256: Some(sha.into()),
        }
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn licence_expressions() {
        assert!(
            licence_ok("MIT OR Apache-2.0")
                && licence_ok("Apache-2.0 WITH LLVM-exception")
                && licence_ok("OFL-1.1 AND Ubuntu-font-1.0")
        );
        assert!(
            !licence_ok("GPL-3.0-only") && !licence_ok("MIT AND GPL-2.0-only") && licence_ok("GPL-2.0-only OR MIT")
        );
    }

    #[test]
    fn unlisted_mismatched_and_vendor_assets_fail() {
        let dir = crate::target_dir().join(format!("xtask-assets-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("a.png"), b"abc");
        let good = sha256_hex(b"abc");
        let mut a = Attribution::default();
        assert_eq!(check(&dir, &a, &["a.png".into()], "").len(), 1);
        a.asset.push(entry("a.png", &good, "MIT", "Me"));
        assert!(check(&dir, &a, &["a.png".into()], "").is_empty());
        a.asset[0].sha256 = Some("00".into());
        assert_eq!(check(&dir, &a, &["a.png".into()], "").len(), 1);
        a.asset[0] = entry("a.png", &good, "GPL-3.0-only", "ACME Corp");
        a.policy.original_vendor.push("acme".into());
        assert_eq!(check(&dir, &a, &["a.png".into()], "").len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
