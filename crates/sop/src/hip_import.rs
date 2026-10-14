//! `.hip` -> hexe scene JSON (H40). `hexe_graph::hip` reads the file (no node semantics); this module maps Houdini SOP types and
//! parm tuples onto hexe nodes. Rules (all honest, nothing silently dropped):
//! * type: trailing `::N.M` version stripped, alias table (`attribwrangle` -> `wrangle`); unknown -> node `unsupported:<type>`
//!   keeping EVERY parm (see `scene.rs` `UnsupportedNode`: loads, wires, cooks to `Unsupported`).
//! * parm: a tuple whose name matches a hexe `ParamSpec` is converted by the spec default's kind (menu tokens via `spec.menu`);
//!   emitted only when it differs from the hexe default ("set"); keyframed components use the value saved in the file.
//! * no hexe twin for a parm / conversion failure -> reported in `notes`, never invented.
//! * wires: `.def` `inputs {}` (same-network sources); bypass flag is NOT applied (noted).
use crate::{Registry, SceneError, registry};
use hexe_graph::hip::{HipNetwork, HipNode, HipParm, HipScene, HipValue, read_hip};
use hexe_graph::{ParamSpec, ParamValue};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
/// One `/obj/<container>` imported as a hexe scene.
#[derive(Debug, Clone)]
pub struct GeoImport {
    pub name: String,
    /// Container type (`geo`).
    pub container: String,
    /// Scene JSON in the `load_scene` schema.
    pub scene: Value,
    pub display: String,
    pub node_count: usize,
    pub wire_count: usize,
    /// `node:type` of every `unsupported:` node.
    pub unsupported: Vec<String>,
    /// Nodes with keyframed / expression parms (value at save time used).
    pub animated: Vec<String>,
    pub notes: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct HipImport {
    pub frame: f64,
    pub fps: f64,
    pub geos: Vec<GeoImport>,
    /// Non-SOP containers under /obj (`topnet`, `subnet`...) not imported.
    pub skipped: Vec<String>,
}
/// Houdini operator type -> hexe registry token (`None` = no twin).
pub fn hexe_type(reg: &Registry, ty: &str) -> Option<String> {
    let base = match ty.rfind("::") {
        Some(i) if ty[i + 2..].chars().all(|c| c.is_ascii_digit() || c == '.') && !ty[..i].contains("::") => &ty[..i],
        _ => ty,
    };
    let t = match base {
        "attribwrangle" => "wrangle",
        "ropgeometry" => "rop_geometry",
        o => o,
    };
    reg.create(t).map(|_| t.to_string())
}
fn num_json(x: f64) -> Value {
    serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number)
}
fn plain_json(v: &HipValue) -> Value {
    match v.plain() {
        HipValue::Num(x) if x.fract() == 0.0 && x.abs() < 1e15 => json!(*x as i64),
        HipValue::Num(x) => num_json(*x),
        HipValue::Str(s) => json!(s),
        HipValue::Chan(..) => Value::Null,
    }
}
fn parm_json(p: &HipParm) -> Value {
    match p.values.as_slice() {
        [one] => plain_json(one),
        many => Value::Array(many.iter().map(plain_json).collect()),
    }
}
fn f(v: &HipValue) -> Result<f64, String> {
    match v.plain() {
        HipValue::Num(x) => Ok(*x),
        HipValue::Str(s) => s.trim().parse().map_err(|_| format!("non-numeric `{s}`")),
        HipValue::Chan(..) => Err("nested channel".into()),
    }
}
/// Houdini menu tokens per hexe type / parm (`oracle/houdini/hip-menus.py`, hython 20.5 introspection): token -> position.
fn menu_position(ty: &str, parm: &str, token: &str) -> Option<usize> {
    static MENUS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let m = MENUS.get_or_init(|| serde_json::from_str(include_str!("hip_menus.json")).unwrap_or(Value::Null));
    m.get(ty)?.get(parm)?.as_array()?.iter().position(|t| t.as_str() == Some(token))
}
/// Convert one parm tuple against a hexe spec; `Ok(None)` = equals the hexe default.
fn convert(ty: &str, spec: &ParamSpec, p: &HipParm) -> Result<Option<Value>, String> {
    let first = p.values.first().ok_or("empty tuple")?;
    let val = match &spec.default {
        ParamValue::F(_) => ParamValue::F(f(first)? as f32),
        ParamValue::I(_) => ParamValue::I(f(first)?.round() as i32),
        ParamValue::B(_) => ParamValue::B(match first.plain() {
            HipValue::Str(s) => matches!(s.as_str(), "on" | "1" | "true"),
            HipValue::Num(x) => *x != 0.0,
            HipValue::Chan(..) => false,
        }),
        ParamValue::V3(def) => {
            // 2-tuples (grid `size`) pad from the hexe default; longer ones are not a vec3.
            if !(2..=3).contains(&p.values.len()) {
                return Err(format!("tuple size {} for a vec3", p.values.len()));
            }
            let mut c = *def;
            for (slot, v) in c.iter_mut().zip(&p.values) {
                *slot = f(v)? as f32;
            }
            ParamValue::V3(c)
        }
        ParamValue::S(_) => ParamValue::S(match first.plain() {
            HipValue::Str(s) => s.clone(),
            HipValue::Num(x) => format!("{x}"),
            HipValue::Chan(..) => String::new(),
        }),
        ParamValue::Menu(_) => match first.plain() {
            HipValue::Num(x) => ParamValue::Menu(*x as u32),
            HipValue::Str(s) => match spec.menu.iter().position(|(tok, _)| tok == s).or_else(|| menu_position(ty, &spec.name, s)) {
                Some(i) => ParamValue::Menu(i as u32),
                None => match s.parse::<u32>() {
                    Ok(i) => ParamValue::Menu(i),
                    Err(_) => return Err(format!("menu token `{s}` unknown for {ty}.{}", spec.name)),
                },
            },
            HipValue::Chan(..) => return Err("nested channel".into()),
        },
    };
    if !val.is_acceptable() {
        return Err("non-finite or over-long value".into());
    }
    if val == spec.default {
        return Ok(None);
    }
    Ok(Some(match val {
        ParamValue::F(x) => num_json(format!("{x}").parse().unwrap_or(0.0)),
        ParamValue::I(x) => json!(x),
        ParamValue::B(x) => json!(x),
        ParamValue::V3(c) => Value::Array(c.iter().map(|x| num_json(format!("{x}").parse().unwrap_or(0.0))).collect()),
        ParamValue::S(s) => json!(s),
        ParamValue::Menu(m) => json!(m),
    }))
}
fn import_network(reg: &Registry, net: &HipNetwork) -> GeoImport {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let (mut unsupported, mut animated, mut notes) = (Vec::new(), Vec::new(), Vec::new());
    let names: BTreeSet<&str> = net.nodes.iter().map(|n| n.name.as_str()).collect();
    let mut ordered: Vec<&HipNode> = net.nodes.iter().collect();
    ordered.sort_by(|a, b| a.name.cmp(&b.name));
    let mut pos = Map::new();
    for n in &ordered {
        let mut node = json!({ "name": n.name });
        if n.parms.iter().flat_map(|p| &p.values).any(HipValue::is_animated) || n.has_channels {
            animated.push(n.name.clone());
        }
        match hexe_type(reg, &n.ty) {
            Some(t) => {
                node["type"] = json!(t);
                let specs = reg.specs(&t).unwrap_or_default();
                let mut params = Map::new();
                let mut no_twin: Vec<&str> = Vec::new();
                for p in &n.parms {
                    match specs.iter().find(|s| s.name == p.name) {
                        None => no_twin.push(&p.name),
                        Some(spec) => match convert(&t, spec, p) {
                            Ok(Some(v)) => {
                                params.insert(p.name.clone(), v);
                            }
                            Ok(None) => {}
                            Err(e) => notes.push(format!("{}.{}: {e}", n.name, p.name)),
                        },
                    }
                }
                if !params.is_empty() {
                    node["params"] = Value::Object(params);
                }
                if !no_twin.is_empty() {
                    notes.push(format!("{}: parms without hexe twin: {}", n.name, no_twin.join(",")));
                }
            }
            None => {
                node["type"] = json!(format!("unsupported:{}", n.ty));
                let params: Map<String, Value> = n.parms.iter().map(|p| (p.name.clone(), parm_json(p))).collect();
                if !params.is_empty() {
                    node["params"] = Value::Object(params);
                }
                unsupported.push(format!("{}:{}", n.name, n.ty));
            }
        }
        if n.bypass {
            notes.push(format!("{}: bypass flag set (not applied)", n.name));
        }
        if n.has_children {
            notes.push(format!("{}: has child nodes (not imported)", n.name));
        }
        if let Some(p) = n.pos {
            pos.insert(n.name.clone(), json!([p[0], p[1]]));
        }
        nodes.push(node);
        for (i, inp) in n.inputs.iter().enumerate() {
            if let Some(inp) = inp {
                if names.contains(inp.src.as_str()) {
                    edges.push(json!([inp.src, inp.output, n.name, i]));
                } else {
                    notes.push(format!("{}: input {i} from `{}` outside the network (not wired)", n.name, inp.src));
                }
            }
        }
    }
    let display = ordered
        .iter()
        .find(|n| n.display)
        .or_else(|| ordered.iter().find(|n| n.render))
        .or_else(|| ordered.last())
        .map(|n| n.name.clone())
        .unwrap_or_default();
    let wire_count = edges.len();
    GeoImport {
        name: net.name.clone(),
        container: net.ty.clone(),
        scene: json!({ "nodes": nodes, "edges": edges, "display": display, "ui": { "pos": pos } }),
        display,
        node_count: net.nodes.len(),
        wire_count,
        unsupported,
        animated,
        notes,
    }
}
/// Map a parsed scene onto hexe scenes (one per `geo` container with nodes).
pub fn import_scene(hip: &HipScene, reg: &Registry) -> HipImport {
    let mut geos = Vec::new();
    let mut skipped = Vec::new();
    for net in &hip.networks {
        if net.ty == "geo" && !net.nodes.is_empty() {
            geos.push(import_network(reg, net));
        } else {
            skipped.push(format!("{}:{} ({} children)", net.name, net.ty, net.nodes.len()));
        }
    }
    geos.sort_by(|a, b| a.name.cmp(&b.name));
    HipImport { frame: hip.frame(), fps: hip.fps, geos, skipped }
}
/// `.hip` / `.hipnc` / `.hiplc` / `.hip.gz` bytes -> hexe scenes.
pub fn import_hip(bytes: &[u8]) -> Result<HipImport, SceneError> {
    let hip = read_hip(bytes).map_err(|e| SceneError(e.to_string()))?;
    Ok(import_scene(&hip, &registry()))
}
/// Is this path a Houdini scene file by extension?
pub fn is_hip_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [".hip", ".hipnc", ".hiplc", ".hip.gz"].iter().any(|e| p.ends_with(e))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn type_aliases_and_versions() {
        let r = registry();
        assert_eq!(hexe_type(&r, "box").as_deref(), Some("box"));
        assert_eq!(hexe_type(&r, "attribcreate::2.0").as_deref(), Some("attribcreate"));
        assert_eq!(hexe_type(&r, "attribwrangle").as_deref(), Some("wrangle"));
        assert_eq!(hexe_type(&r, "hda::lib::1.0"), None);
        assert_eq!(hexe_type(&r, "nosuch"), None);
    }
    #[test]
    fn path_extensions() {
        for p in ["a.hip", "B.HIPNC", "c.hiplc", "d.hip.gz"] {
            assert!(is_hip_path(p), "{p}");
        }
        assert!(!is_hip_path("a.json"));
    }
    use crate::builtins::meshutil::testkit::houdini_twin_at;
    use std::collections::BTreeSet;
    const CHAIN: &[u8] = include_bytes!("../../../fixtures/hip/authored-chain.hip");
    const CHAIN_GZ: &[u8] = include_bytes!("../../../fixtures/hip/authored-chain.hip.gz");
    const CHAIN_GEO: &[u8] = include_bytes!("../../../fixtures/hip/authored-chain.houdini.geo");
    const UNSUP: &[u8] = include_bytes!("../../../fixtures/hip/authored-unsup.hip");
    fn time_of(imp: &HipImport) -> f32 {
        ((imp.frame - 1.0) / imp.fps) as f32
    }
    /// Authored `.hip` (box -> xform, grid, merge) imports, wires and cooks EQUAL to the real hython 20.5 cook of the same file.
    #[test]
    fn hip_authored_chain_matches_hython() {
        let imp = import_hip(CHAIN).unwrap();
        assert_eq!(imp.geos.len(), 1);
        let g = &imp.geos[0];
        assert_eq!((g.node_count, g.wire_count, g.display.as_str()), (4, 3, "merge1"));
        assert!(g.unsupported.is_empty(), "{:?}", g.unsupported);
        let scene = serde_json::to_string(&g.scene).unwrap();
        houdini_twin_at(&scene, CHAIN_GEO, time_of(&imp)).expect("hexe == Houdini");
    }
    #[test]
    fn hip_gzip_wrapper_imports_identically() {
        let a = import_hip(CHAIN).unwrap();
        let b = import_hip(CHAIN_GZ).unwrap();
        assert_eq!(a.geos[0].scene, b.geos[0].scene);
    }
    /// A SOP with no hexe twin stays in the graph as `unsupported:<type>` with its parms and wire; it loads but cooks to Unsupported.
    #[test]
    fn hip_unsupported_node_is_kept_wired_and_cooks_to_unsupported() {
        let imp = import_hip(UNSUP).unwrap();
        let g = &imp.geos[0];
        assert_eq!(g.unsupported, vec!["point1:point".to_string()]);
        assert_eq!((g.node_count, g.wire_count), (2, 1));
        let node = g.scene["nodes"].as_array().unwrap().iter().find(|n| n["name"] == "point1").unwrap();
        assert_eq!(node["type"], "unsupported:point");
        assert!(node["params"].as_object().is_some_and(|p| !p.is_empty()), "parms kept");
        let (mut graph, id) = crate::load_scene(&serde_json::to_string(&g.scene).unwrap()).unwrap();
        let e = graph.cook(id, 0.0).unwrap_err().to_string();
        assert!(e.contains("point") && e.contains("no hexe twin"), "{e}");
    }
    #[test]
    fn hip_hostile_inputs_are_errors() {
        assert!(import_hip(b"").is_err());
        assert!(import_hip(b"junk junk junk").is_err());
        assert!(import_hip(&CHAIN[..CHAIN.len() / 2]).is_err());
    }
    fn upstream(scene: &Value, start: &str) -> BTreeSet<String> {
        let edges = scene["edges"].as_array().cloned().unwrap_or_default();
        let mut seen = BTreeSet::new();
        let mut todo = vec![start.to_string()];
        while let Some(n) = todo.pop() {
            if seen.insert(n.clone()) {
                todo.extend(edges.iter().filter(|e| e[2] == n.as_str()).filter_map(|e| e[0].as_str().map(String::from)));
            }
        }
        seen
    }
    /// Corpus twin (hython-gated, NO-OP unless `HEXE_HIP_TWIN_DIR` is set): `<dir>/list.tsv` = `id<TAB>path`, `<dir>/<id>/<geo>.geo` from
    /// `oracle/houdini/hip-twin.py`. Writes `<dir>/verdicts.tsv` (id, file, geo, nodes, wires, verdict). Never fails on a DIFF: honest table.
    #[test]
    fn hip_corpus_twin() {
        let Ok(dir) = std::env::var("HEXE_HIP_TWIN_DIR") else { return };
        let list = std::fs::read_to_string(format!("{dir}/list.tsv")).unwrap();
        let mut rows = String::new();
        for line in list.lines().filter(|l| !l.is_empty()) {
            let (id, path) = line.split_once('\t').unwrap();
            let name = path.rsplit('/').next().unwrap();
            let imp = match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| import_hip(&b).map_err(|e| e.to_string())) {
                Ok(i) => i,
                Err(e) => {
                    rows += &format!("{id}\t{name}\t-\t0\t0\tPARSE-ERR {e}\n");
                    continue;
                }
            };
            for g in &imp.geos {
                let up = upstream(&g.scene, &g.display);
                let bad: Vec<&str> = g.unsupported.iter().filter(|u| up.contains(u.split(':').next().unwrap_or(""))).map(String::as_str).collect();
                let scene = serde_json::to_string(&g.scene).unwrap();
                let verdict = if !bad.is_empty() {
                    format!("UNSUPPORTED-NODE {}", bad.join(" ").replace(|c: char| c == ':', "/"))
                } else {
                    match std::fs::read(format!("{dir}/{id}/{}.geo", g.name)) {
                        Err(_) => "NO-HOUDINI-GEO".to_string(),
                        Ok(h) => match houdini_twin_at(&scene, &h, time_of(&imp)) {
                            Ok(()) => "MATCH".to_string(),
                            Err(e) if e.starts_with("cook:") => format!("HEXE-ERR {}", e.chars().take(110).collect::<String>()),
                            Err(e) => {
                                let kind = if g.animated.iter().any(|a| up.contains(a)) { "PARM-DIFF(animated)" } else if e.starts_with("counts") { "COUNT-DIFF" } else if e.starts_with("P[") { "P-DIFF" } else if e.starts_with("prim") { "TOPO-DIFF" } else { "ATTR-DIFF" };
                                format!("{kind} {}", e.chars().take(110).collect::<String>())
                            }
                        },
                    }
                };
                rows += &format!("{id}\t{name}\t{}\t{}\t{}\t{verdict}\n", g.name, g.node_count, g.wire_count);
            }
        }
        std::fs::write(format!("{dir}/verdicts.tsv"), rows).unwrap();
    }
}
