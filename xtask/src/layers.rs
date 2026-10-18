//! Dependency layering L0..L7, enforced by `cargo xtask layers`. Register every new crate in `TABLE`.
//!
//! L0 foundation (standalone) · L1 formats/objects · L2 model · L3 services · L4 features · L5 interaction ·
//! L6 engine façade · L7 platform + UI. Rule: a crate depends only on LOWER layers (or a listed sideways edge).
//! UI toolkits (egui, winit, wgpu…) only at L7+. Apps/tooling are `Exempt`. Test support is dev-dependency only.
//! The engine works on a small metadata-independent model so it is unit-tested; `from_metadata` builds it.

use crate::Res;
use serde_json::Value;

#[allow(dead_code)] // Layer(n) is used by crates the port adds to TABLE
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Layer(u8),
    /// Layer 0/1 crate that may depend only on the workspace crates listed in `STANDALONE_DEPS`.
    Standalone(u8),
    /// Test tooling: dev-dependency only for other crates.
    Testkit,
    /// Binaries, build tooling: exempt.
    Exempt,
}

impl Class {
    fn layer(self) -> Option<u8> {
        match self {
            Class::Layer(l) | Class::Standalone(l) => Some(l),
            Class::Testkit => Some(7),
            Class::Exempt => None,
        }
    }
}

/// Package-name prefix stripped before the `TABLE` lookup.
pub const PREFIX: &str = "hexe-";

/// Every workspace crate (package name without the prefix) and its layer.
pub const TABLE: &[(&str, Class)] = &[
    ("core", Class::Standalone(0)),
    ("geo", Class::Layer(0)),
    ("graph", Class::Layer(1)),
    ("vex", Class::Standalone(0)),
    ("sop", Class::Layer(2)),
    ("endo-sim", Class::Layer(1)),
    ("sop-motion", Class::Layer(2)),
    // L1 formats · L2 model · L3 services · L4 features · L5 interaction · L6 engine · L7 platform/ui: add rows here
    // ("testkit", Class::Testkit),
    ("cli", Class::Exempt),
    ("tauri", Class::Exempt),
    ("tauri-debug-server", Class::Exempt), // vendored under third_party/
    ("xtask", Class::Exempt),
];
/// Workspace crates a standalone crate may use, e.g. `("cos", &["filters", "geom"])`.
pub const STANDALONE_DEPS: &[(&str, &[&str])] = &[];
/// Allowed same-layer edges `(from, to)`.
pub const SIDEWAYS: &[(&str, &str)] = &[];
/// External crates that are UI toolkits / windowing (`*` suffix = prefix match).
pub const UI_CRATES: &[&str] = &["egui", "eframe", "tauri*", "winit", "egui_kittest", "egui_extras", "rfd", "wgpu*"];
pub const UI_MIN_LAYER: u8 = 7;

pub struct Rules<'a> {
    pub table: &'a [(&'a str, Class)],
    pub standalone: &'a [(&'a str, &'a [&'a str])],
    pub sideways: &'a [(&'a str, &'a str)],
}

pub const RULES: Rules<'static> = Rules { table: TABLE, standalone: STANDALONE_DEPS, sideways: SIDEWAYS };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    pub workspace: bool,
}

#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unregistered { krate: String },
    Upward { krate: String, dep: String, from: u8, to: u8 },
    StandaloneHasWorkspaceDep { krate: String, dep: String },
    TestkitAsNormalDep { krate: String, dep: String },
    UiBelowL7 { krate: String, dep: String, layer: u8 },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => {
                write!(f, "{krate}: unregistered crate; add it to xtask/src/layers.rs TABLE")
            }
            Violation::Upward { krate, dep, from, to } => {
                write!(f, "{krate} (L{from}) -> {dep} (L{to}): only lower layers or a listed sideways edge")
            }
            Violation::StandaloneHasWorkspaceDep { krate, dep } => {
                write!(f, "{krate}: standalone crate must not depend on {dep}")
            }
            Violation::TestkitAsNormalDep { krate, dep } => write!(f, "{krate}: {dep} may only be a dev-dependency"),
            Violation::UiBelowL7 { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) uses UI crate `{dep}`; UI is L7+ only")
            }
        }
    }
}

impl Rules<'_> {
    fn short<'b>(&self, pkg: &'b str) -> &'b str {
        pkg.strip_prefix(PREFIX).unwrap_or(pkg)
    }
    fn classify(&self, pkg: &str) -> Option<Class> {
        let s = self.short(pkg);
        self.table.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
    }
    fn sideways_ok(&self, from: &str, to: &str) -> bool {
        self.sideways.contains(&(self.short(from), self.short(to)))
    }
    fn standalone_ok(&self, from: &str, to: &str) -> bool {
        self.standalone.iter().any(|(f, ok)| *f == self.short(from) && ok.contains(&self.short(to)))
    }

    pub fn check(&self, crates: &[Crate]) -> Vec<Violation> {
        let mut out = Vec::new();
        for c in crates {
            let Some(class) = self.classify(&c.name) else {
                out.push(Violation::Unregistered { krate: c.name.clone() });
                continue;
            };
            let Some(layer) = class.layer() else { continue };
            for d in c.deps.iter().filter(|d| d.name != c.name) {
                if d.workspace {
                    if matches!(class, Class::Standalone(_)) && !self.standalone_ok(&c.name, &d.name) {
                        out.push(Violation::StandaloneHasWorkspaceDep { krate: c.name.clone(), dep: d.name.clone() });
                        continue;
                    }
                    match self.classify(&d.name) {
                        None => {}
                        Some(Class::Testkit) => {
                            if d.kind != DepKind::Dev && class != Class::Testkit {
                                out.push(Violation::TestkitAsNormalDep { krate: c.name.clone(), dep: d.name.clone() });
                            }
                        }
                        Some(dc) => {
                            let to = dc.layer().unwrap_or(u8::MAX);
                            if !(to < layer || (to == layer && self.sideways_ok(&c.name, &d.name))) {
                                out.push(Violation::Upward {
                                    krate: c.name.clone(),
                                    dep: d.name.clone(),
                                    from: layer,
                                    to: to.min(8),
                                });
                            }
                        }
                    }
                } else if layer < UI_MIN_LAYER
                    && is_ui_crate(&d.name)
                    && !(class == Class::Testkit || d.kind == DepKind::Dev)
                {
                    out.push(Violation::UiBelowL7 { krate: c.name.clone(), dep: d.name.clone(), layer });
                }
            }
        }
        out.sort_by_key(ToString::to_string);
        out.dedup();
        out
    }
}

fn is_ui_crate(name: &str) -> bool {
    UI_CRATES.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *p,
    })
}

pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let pkgs = meta["packages"].as_array().ok_or("metadata: no packages array")?;
    let members: Vec<&str> = pkgs.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut out = Vec::new();
    for p in pkgs {
        let name = p["name"].as_str().ok_or("package without name")?.to_owned();
        let mut deps = Vec::new();
        for d in p["dependencies"].as_array().into_iter().flatten() {
            let dname = d["name"].as_str().unwrap_or_default().to_owned();
            let kind = match d["kind"].as_str() {
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                _ => DepKind::Normal,
            };
            let workspace = members.contains(&dname.as_str());
            deps.push(Dep { name: dname, kind, workspace });
        }
        out.push(Crate { name, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn run(_: &[String]) -> Res {
    let out = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(crate::root())
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let crates = from_metadata(&meta)?;
    let violations = RULES.check(&crates);
    for v in &violations {
        println!("  - {v}");
    }
    if violations.is_empty() {
        println!("layers: OK, {} crates, no violations", crates.len());
        Ok(())
    } else {
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::DepKind::*;
    use super::*;

    const T: &[(&str, Class)] = &[
        ("geom", Class::Standalone(0)),
        ("cos", Class::Standalone(1)),
        ("model", Class::Layer(2)),
        ("forms", Class::Layer(3)),
        ("js", Class::Layer(3)),
        ("redact", Class::Layer(4)),
        ("edit", Class::Layer(4)),
        ("engine", Class::Layer(6)),
        ("ui", Class::Layer(7)),
        ("testkit", Class::Testkit),
        ("app", Class::Exempt),
    ];
    const SA: &[(&str, &[&str])] = &[("cos", &["geom"])];
    const SW: &[(&str, &str)] = &[("redact", "edit")];
    const R: Rules<'static> = Rules { table: T, standalone: SA, sideways: SW };

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate {
            name: name.into(),
            deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect(),
        }
    }

    #[test]
    fn valid_graph_passes() {
        let g = [
            c("geom", &[]),
            c("engine", &[("model", Normal, true), ("serde", Normal, false)]),
            c("ui", &[("engine", Normal, true), ("egui", Normal, false)]),
            c("app", &[("ui", Normal, true), ("eframe", Normal, false)]),
        ];
        assert!(R.check(&g).is_empty(), "{:?}", R.check(&g));
    }

    #[test]
    fn upward_and_unlisted_sideways_flagged() {
        let v = R.check(&[c("model", &[("engine", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 2, to: 6, .. }]));
        assert!(matches!(
            R.check(&[c("forms", &[("js", Normal, true)])])[..],
            [Violation::Upward { from: 3, to: 3, .. }]
        ));
        assert!(R.check(&[c("redact", &[("edit", Normal, true)])]).is_empty());
        assert!(!R.check(&[c("edit", &[("redact", Normal, true)])]).is_empty());
    }

    #[test]
    fn standalone_crates_use_only_their_listed_deps() {
        assert!(R.check(&[c("cos", &[("geom", Normal, true)])]).is_empty());
        let v = R.check(&[c("geom", &[("cos", Normal, true)])]);
        assert!(matches!(v[..], [Violation::StandaloneHasWorkspaceDep { .. }]));
    }

    #[test]
    fn ui_crates_below_l7_flagged() {
        for dep in ["egui", "eframe", "tauri*", "winit", "wgpu", "wgpu-core"] {
            let v = R.check(&[c("engine", &[(dep, Normal, false)])]);
            assert!(matches!(v[..], [Violation::UiBelowL7 { layer: 6, .. }]), "{dep}");
        }
    }

    #[test]
    fn testkit_only_as_dev_dependency_and_unregistered_is_error() {
        assert!(R.check(&[c("model", &[("testkit", Dev, true)])]).is_empty());
        let v = R.check(&[c("model", &[("testkit", Normal, true)])]);
        assert!(matches!(v[..], [Violation::TestkitAsNormalDep { .. }]));
        assert!(
            matches!(&R.check(&[c("mystery", &[])])[..], [Violation::Unregistered { krate }] if krate == "mystery")
        );
    }

    #[test]
    fn shipped_table_registers_the_template_crates() {
        let g = [c("hexe-core", &[]), c("hexe-cli", &[("hexe-core", Normal, true)]), c("xtask", &[])];
        assert!(RULES.check(&g).is_empty());
    }
}
