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
/// Convert one parm tuple against a hexe spec; `Ok(None)` = equals the hexe default.
fn convert(spec: &ParamSpec, p: &HipParm) -> Result<Option<Value>, String> {
    let first = p.values.first().ok_or("empty tuple")?;
    let val = match &spec.default {
        ParamValue::F(_) => ParamValue::F(f(first)? as f32),
        ParamValue::I(_) => ParamValue::I(f(first)?.round() as i32),
        ParamValue::B(_) => ParamValue::B(match first.plain() {
            HipValue::Str(s) => matches!(s.as_str(), "on" | "1" | "true"),
            HipValue::Num(x) => *x != 0.0,
            HipValue::Chan(..) => false,
        }),
        ParamValue::V3(_) => {
            if p.values.len() != 3 {
                return Err(format!("tuple size {} for a vec3", p.values.len()));
            }
            ParamValue::V3([f(&p.values[0])? as f32, f(&p.values[1])? as f32, f(&p.values[2])? as f32])
        }
        ParamValue::S(_) => ParamValue::S(match first.plain() {
            HipValue::Str(s) => s.clone(),
            HipValue::Num(x) => format!("{x}"),
            HipValue::Chan(..) => String::new(),
        }),
        ParamValue::Menu(_) => match first.plain() {
            HipValue::Num(x) => ParamValue::Menu(*x as u32),
            HipValue::Str(s) => match spec.menu.iter().position(|(tok, _)| tok == s) {
                Some(i) => ParamValue::Menu(i as u32),
                None => match s.parse::<u32>() {
                    Ok(i) => ParamValue::Menu(i),
                    Err(_) => return Err(format!("menu token `{s}` not in hexe menu")),
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
                        Some(spec) => match convert(spec, p) {
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
}
