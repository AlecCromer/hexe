//! `.hip` -> hexe scene JSON (H40). `hexe_graph::hip` reads the file (no node semantics); this module maps Houdini SOP types and
//! parm tuples onto hexe nodes. Rules (all honest, nothing silently dropped):
//! * type: trailing `::N.M` version stripped, alias table (`attribwrangle` -> `wrangle`); unknown -> node `unsupported:<type>`
//!   keeping EVERY parm (see `scene.rs` `UnsupportedNode`: loads, wires, cooks to `Unsupported`).
//! * parm: a tuple whose name matches a hexe `ParamSpec` is converted by the spec default's kind (menu tokens via `spec.menu`);
//!   emitted only when it differs from the hexe default ("set"); keyframed components use the value saved in the file.
//! * no hexe twin for a parm / conversion failure -> reported in `notes`, never invented.
//! * wires: `.def` `inputs {}` (same-network sources); network dots (`__dotN.networkdotinit`) become `dot` nodes (centre
//!   position, pin flag) so wires through them survive; bypass flags, node colours, network boxes and sticky notes are kept
//!   (editor units: 1 Houdini unit = `HIP_PX` px, y flipped).
use crate::{Registry, SceneError, registry};
use hexe_graph::hip::{HipDot, HipNetwork, HipNode, HipParm, HipScene, HipValue, read_hip};
use hexe_graph::{ParamSpec, ParamValue};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
/// Object-level data of an `/obj` container: editor position (px, top-left), display / selectable flags, user colour.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjMeta {
    pub name: String,
    pub pos: Option<[f32; 2]>,
    pub display: bool,
    pub selectable: bool,
    pub color: Option<[f32; 3]>,
    /// Object-level path of the container holding it (`""` = /obj, `"asset1"` = inside that asset / subnet).
    pub parent: String,
}
/// One `/obj/<container>` imported as a hexe scene.
#[derive(Debug, Clone)]
pub struct GeoImport {
    pub name: String,
    /// Container type (`geo`).
    pub container: String,
    /// The container's own object-level data.
    pub object: ObjMeta,
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
    /// Wires of the `.hip` NOT in the scene: the hexe twin has fewer input ports than Houdini's node (reported, never silent).
    pub dropped_wires: usize,
}
#[derive(Debug, Clone)]
pub struct HipImport {
    pub frame: f64,
    pub fps: f64,
    pub geos: Vec<GeoImport>,
    /// Non-SOP containers under /obj (`topnet`, `subnet`...) not imported.
    pub skipped: Vec<String>,
    /// `geo` objects with no SOP nodes yet (shown at /obj, nothing to import inside).
    pub empty: Vec<ObjMeta>,
    /// Object-level containers that are not `geo`s (subnets / assets), with their networks' sticky notes.
    pub containers: Vec<ContainerImport>,
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
pub(crate) fn menu_position(ty: &str, parm: &str, token: &str) -> Option<usize> {
    static MENUS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let m = MENUS.get_or_init(|| serde_json::from_str(include_str!("hip_menus.json")).unwrap_or(Value::Null));
    m.get(ty)?.get(parm)?.as_array()?.iter().position(|t| t.as_str() == Some(token))
}
/// Convert one parm tuple against a hexe spec; `Ok(None)` = equals the hexe default.
/// Houdini parm name -> the hexe param it feeds, where hexe named it differently (legacy `group` SOP: `crname` is the
/// group name, `entity` the class, the tab toggles `groupbounding` / `groupnormal`, the box `size` / `t`).
fn parm_alias<'a>(ty: &str, name: &'a str) -> &'a str {
    match (ty, name) {
        ("group", "crname") => "groupname",
        ("group", "entity") => "grouptype",
        ("group", "mergeop") => "merge_op",
        ("group", "groupbounding") => "bbox",
        ("group", "groupnormal") => "normal",
        ("group", "size") => "bbox_size",
        ("group", "t") => "bbox_center",
        _ => name,
    }
}
fn convert(ty: &str, spec: &ParamSpec, p: &HipParm) -> Result<Option<Value>, String> {
    // an empty tuple (a separator: `sepparm [ 0 locks=0 ] ( )`) carries no value
    let Some(first) = p.values.first() else { return Ok(None) };
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
            HipValue::Str(s) => {
                // exact token, the recorded Houdini menu, then a long-enough prefix (Houdini `primitive` = hexe `prim`)
                let prefix = || spec.menu.iter().position(|(tok, _)| tok.len() >= 4 && s.starts_with(tok.as_str()));
                match spec
                    .menu
                    .iter()
                    .position(|(tok, _)| tok == s)
                    .or_else(|| menu_position(ty, &spec.name, s))
                    .or_else(prefix)
                {
                    Some(i) => ParamValue::Menu(i as u32),
                    None => match s.parse::<u32>() {
                        Ok(i) => ParamValue::Menu(i),
                        Err(_) => return Err(format!("menu token `{s}` unknown for {ty}.{}", spec.name)),
                    },
                }
            }
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
/// A parm's first value as text (`"pieces"`, `"../foreach_begin1"`, `10`), `None` when absent.
fn parm_text(n: &HipNode, name: &str) -> Option<String> {
    let v = n.parms.iter().find(|p| p.name == name)?.values.first()?;
    match v.plain() {
        HipValue::Str(s) => Some(s.trim().to_string()),
        HipValue::Num(x) => Some(format!("{x}")),
        HipValue::Chan(..) => None,
    }
}
/// Houdini For-Each pairs keep the iteration settings on the End (`itermethod` pieces / count / auto, `class`,
/// `useattrib` + `attrib`, `iterations`, `method` merge / feedback) and link the two with `blockpath` (`../foreachA`).
/// hexe keeps the iteration method on the Begin (`method` piece / count / points / prims, `class`, `attrib`, `count`)
/// and the pairing + gather mode on the End (`begin`, `mode`). Rewrites the imported nodes accordingly; an End whose
/// `blockpath` names no Begin in this network is reported and keeps hexe's nearest-upstream pairing.
fn pair_blocks(reg: &Registry, net: &HipNetwork, nodes: &mut [Value], notes: &mut Vec<String>) {
    let mut set = |name: &str, k: &str, v: Value| {
        if let Some(node) = nodes.iter_mut().find(|n| n["name"] == name) {
            if !node["params"].is_object() {
                node["params"] = json!({});
            }
            node["params"][k] = v;
        }
    };
    let is = |n: &HipNode, t: &str| hexe_type(reg, &n.ty).as_deref() == Some(t);
    for end in net.nodes.iter().filter(|n| is(n, "block_end")) {
        let path = parm_text(end, "blockpath").unwrap_or_default();
        let target = path.rsplit('/').next().unwrap_or_default();
        let Some(begin) = net.nodes.iter().find(|n| n.name == target && is(n, "block_begin")) else {
            notes.push(format!("{}: block path `{path}` names no Block Begin here (nearest upstream used)", end.name));
            continue;
        };
        set(&end.name, "begin", json!(begin.name));
        set(&end.name, "mode", json!(u32::from(parm_text(end, "method").as_deref() == Some("feedback"))));
        // Auto Detect: pieces when the Begin extracts pieces, else a fixed count (the Begin fetches feedback / input)
        let count = match parm_text(end, "itermethod").as_deref() {
            Some("count") => true,
            Some("pieces") => false,
            _ => parm_text(begin, "method").as_deref() != Some("piece"),
        };
        let attrib =
            parm_text(end, "attrib").filter(|a| !a.is_empty() && parm_text(end, "useattrib").as_deref() == Some("on"));
        let point = parm_text(end, "class").as_deref() == Some("point");
        if count {
            let n = parm_text(end, "iterations").and_then(|s| s.parse::<f64>().ok()).unwrap_or(1.0);
            set(&begin.name, "method", json!(1));
            set(&begin.name, "count", json!(n.clamp(0.0, f64::from(i32::MAX)) as i64));
        } else if let Some(a) = attrib {
            set(&begin.name, "method", json!(0));
            set(&begin.name, "class", json!(u32::from(!point)));
            set(&begin.name, "attrib", json!(a));
        } else {
            // per point / per primitive
            set(&begin.name, "method", json!(if point { 2 } else { 3 }));
        }
    }
}
fn import_network(reg: &Registry, net: &HipNetwork) -> GeoImport {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let (mut unsupported, mut animated, mut notes) = (Vec::new(), Vec::new(), Vec::new());
    let names: BTreeSet<&str> =
        net.nodes.iter().map(|n| n.name.as_str()).chain(net.dots.iter().map(|d| d.name.as_str())).collect();
    let mut ordered: Vec<&HipNode> = net.nodes.iter().collect();
    ordered.sort_by(|a, b| a.name.cmp(&b.name));
    let mut pos = Map::new();
    let mut colors = Map::new();
    let mut dropped_wires = 0usize;
    for n in &ordered {
        let mut ports = 16usize;
        let mut node = json!({ "name": n.name });
        if n.parms.iter().flat_map(|p| &p.values).any(HipValue::is_animated) || n.has_channels {
            animated.push(n.name.clone());
        }
        match hexe_type(reg, &n.ty) {
            Some(t) => {
                node["type"] = json!(t);
                ports = reg.create(&t).map_or(0, |x| x.inputs().len());
                let specs = reg.specs(&t).unwrap_or_default();
                let mut params = Map::new();
                let mut no_twin: Vec<&str> = Vec::new();
                for p in &n.parms {
                    let hname = parm_alias(&t, &p.name);
                    match specs.iter().find(|s| s.name == hname) {
                        None => no_twin.push(&p.name),
                        Some(spec) => match convert(&t, spec, p) {
                            Ok(Some(v)) => {
                                params.insert(hname.to_string(), v);
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
            node["bypass"] = json!(true);
        }
        if n.has_children {
            notes.push(format!("{}: has child nodes (not imported)", n.name));
        }
        if let Some(p) = n.pos {
            pos.insert(n.name.clone(), json!(node_px(p)));
        }
        if let Some(c) = n.color {
            colors.insert(n.name.clone(), json!(c));
        }
        nodes.push(node);
        for (i, inp) in n.inputs.iter().enumerate() {
            if let Some(inp) = inp {
                if i >= ports {
                    dropped_wires += 1;
                    notes.push(format!(
                        "{}: input {i} from `{}` dropped (hexe `{}` has {ports} input port(s))",
                        n.name, inp.src, n.ty
                    ));
                } else if inp.output > 0 {
                    // hexe nodes have one output: a wire from a second output (Vellum constraints, Split...) is reported
                    dropped_wires += 1;
                    notes.push(format!(
                        "{}: input {i} from output {} of `{}` dropped (hexe nodes have one output)",
                        n.name, inp.output, inp.src
                    ));
                } else if names.contains(inp.src.as_str()) {
                    edges.push(json!([inp.src, inp.output, n.name, i]));
                } else {
                    notes.push(format!("{}: input {i} from `{}` outside the network (not wired)", n.name, inp.src));
                }
            }
        }
    }
    // network dots: wire waypoints -> `dot` nodes (Houdini position = the dot's centre, as hexe stores a dot)
    let mut dots: Vec<&HipDot> = net.dots.iter().collect();
    dots.sort_by(|a, b| a.name.cmp(&b.name));
    for d in dots {
        let mut node = json!({ "name": d.name, "type": crate::DOT_TYPE });
        if d.pinned {
            node["params"] = json!({ "pinned": true });
        }
        nodes.push(node);
        pos.insert(d.name.clone(), json!([d.pos[0] * HIP_PX, -d.pos[1] * HIP_PX]));
        match &d.input {
            Some(inp) if names.contains(inp.src.as_str()) && inp.output == 0 => {
                edges.push(json!([inp.src, inp.output, d.name, 0]))
            }
            Some(inp) => notes.push(format!("{}: input from `{}` outside the network (not wired)", d.name, inp.src)),
            None => {}
        }
    }
    pair_blocks(reg, net, &mut nodes, &mut notes);
    let display = ordered
        .iter()
        .find(|n| n.display)
        .or_else(|| ordered.iter().find(|n| n.render))
        .or_else(|| ordered.last())
        .map(|n| n.name.clone())
        .unwrap_or_default();
    let wire_count = edges.len();
    // network boxes / sticky notes: Houdini lower-left + size (units, y up) -> editor top-left + size (px, y down)
    let rect = |p: [f32; 2], s: [f32; 2]| [p[0] * HIP_PX, -(p[1] + s[1]) * HIP_PX, s[0] * HIP_PX, s[1] * HIP_PX];
    let boxes: Vec<Value> = net
        .boxes
        .iter()
        .map(|b| {
            let [x, y, w, h] = rect(b.pos, b.size);
            let members: Vec<&str> = net.nodes.iter().filter(|n| n.netbox.as_deref() == Some(b.name.as_str())).map(|n| n.name.as_str()).collect();
            json!({ "name": b.name, "title": b.comment, "color": b.color, "x": x, "y": y, "w": w, "h": h, "minimized": b.minimized, "nodes": members })
        })
        .collect();
    let stickies: Vec<Value> = net
        .notes
        .iter()
        .map(|n| {
            let [x, y, w, h] = rect(n.pos, n.size);
            json!({ "name": n.name, "text": n.text, "color": n.color, "x": x, "y": y, "w": w, "h": h, "minimized": n.minimized })
        })
        .collect();
    let mut ui = json!({ "pos": pos });
    if !colors.is_empty() {
        ui["color"] = Value::Object(colors);
    }
    if !boxes.is_empty() {
        ui["boxes"] = Value::Array(boxes);
    }
    if !stickies.is_empty() {
        ui["notes"] = Value::Array(stickies);
    }
    GeoImport {
        name: net.name.clone(),
        container: net.ty.clone(),
        object: object_meta(net),
        scene: json!({ "nodes": nodes, "edges": edges, "display": display, "ui": ui }),
        display,
        node_count: net.nodes.len(),
        wire_count,
        unsupported,
        animated,
        notes,
        dropped_wires,
    }
}
/// Houdini network units -> hexe editor px (a Houdini node is ~1 unit wide; hexe's is 100 px).
pub const HIP_PX: f32 = 100.0;
/// Height of a hexe node box in Houdini units (30 px): Houdini positions are a node's LOWER-left corner, hexe's the top-left.
const NODE_H_UNITS: f32 = 0.3;
/// Houdini node position (lower-left, y up, units) -> hexe editor position (top-left, y down, px).
fn node_px(p: [f32; 2]) -> [f32; 2] {
    [p[0] * HIP_PX, -(p[1] + NODE_H_UNITS) * HIP_PX]
}
fn object_meta(net: &HipNetwork) -> ObjMeta {
    ObjMeta {
        name: net.name.clone(),
        pos: net.pos.map(node_px),
        display: net.display,
        selectable: net.selectable,
        color: net.color,
        parent: net.parent.clone(),
    }
}
/// An object-level container that is not a `geo` (subnet / digital asset such as `asset1`): the editor shows it as
/// a node at /obj you can dive into; its own network holds the geos (`ObjMeta::parent`) and sticky notes.
#[derive(Debug, Clone)]
pub struct ContainerImport {
    pub object: ObjMeta,
    /// Houdini operator type (`subnet`, `myasset`, ...).
    pub ty: String,
    /// Sticky notes of its network, in the scene `ui.notes` schema (editor px, top-left, y down).
    pub notes: Vec<Value>,
}
/// Sticky notes of a network: Houdini lower-left + size (units, y up) -> editor top-left + size (px, y down).
fn ui_notes(net: &HipNetwork) -> Vec<Value> {
    net.notes
        .iter()
        .map(|n| {
            let (x, y, w, h) = (n.pos[0] * HIP_PX, -(n.pos[1] + n.size[1]) * HIP_PX, n.size[0] * HIP_PX, n.size[1] * HIP_PX);
            json!({ "name": n.name, "text": n.text, "color": n.color, "x": x, "y": y, "w": w, "h": h, "minimized": n.minimized })
        })
        .collect()
}
/// Map a parsed scene onto hexe scenes (one per `geo` container with nodes; empty `geo`s are listed in `empty`).
pub fn import_scene(hip: &HipScene, reg: &Registry) -> HipImport {
    // parm expressions -> their values at the saved frame (saved numbers kept + noted where not evaluable)
    let (evaluated, expr_notes) = crate::hip_expr::evaluate(hip);
    let hip = &evaluated;
    let mut geos = Vec::new();
    let mut skipped = Vec::new();
    let mut empty = Vec::new();
    let mut containers = Vec::new();
    for net in &hip.networks {
        if net.ty == "geo" && !net.nodes.is_empty() {
            let mut g = import_network(reg, net);
            g.notes.extend(expr_notes.iter().filter(|(n, _)| *n == net.name).map(|(_, t)| t.clone()));
            geos.push(g);
        } else if net.ty == "geo" {
            empty.push(object_meta(net));
        } else {
            skipped.push(format!("{}:{} ({} children)", net.name, net.ty, net.nodes.len()));
            containers.push(ContainerImport { object: object_meta(net), ty: net.ty.clone(), notes: ui_notes(net) });
        }
    }
    geos.sort_by(|a, b| a.name.cmp(&b.name));
    HipImport { frame: hip.frame(), fps: hip.fps, geos, skipped, empty, containers }
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
    fn empty_tuple_is_no_value() {
        let spec = ParamSpec::new("sepparm", ParamValue::S(String::new()));
        let p = HipParm { name: "sepparm".into(), flags: 0, values: Vec::new() };
        assert_eq!(convert("attribadjustfloat", &spec, &p), Ok(None));
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
    const DOTS: &[u8] = include_bytes!("../../../fixtures/hip/authored-dots.hip");
    const DOTS_GEO: &[u8] = include_bytes!("../../../fixtures/hip/authored-dots.houdini.geo");
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
    /// Houdini 20.5.522-authored network dots (`oracle/houdini/hip-author.py`, HouNC container): box -> __dot1 -> xform1,
    /// __dot1 -> __dot2 -> merge1.0 (fan-out + chain), pinned __dot3 off box1. Every wire survives as `dot` nodes (centre
    /// position, pin flag kept) and the cook EQUALS Houdini's.
    #[test]
    fn hip_network_dots_import_wired_and_cook_like_houdini() {
        let imp = import_hip(DOTS).unwrap();
        let g = &imp.geos[0];
        assert_eq!((g.node_count, g.wire_count, g.dropped_wires, g.display.as_str()), (3, 6, 0, "merge1"));
        assert!(g.notes.iter().all(|n| !n.contains("not wired")), "{:?}", g.notes);
        let nodes = g.scene["nodes"].as_array().unwrap();
        let dot = |n: &str| nodes.iter().find(|x| x["name"] == n).unwrap_or_else(|| panic!("{n} missing"));
        for d in ["__dot1", "__dot2", "__dot3"] {
            assert_eq!(dot(d)["type"], "dot");
        }
        assert_eq!(dot("__dot3")["params"]["pinned"], true);
        assert!(dot("__dot1").get("params").is_none(), "unpinned = default, not written");
        assert_eq!(g.scene["ui"]["pos"]["__dot1"], json!([50.0, 200.0]), "Houdini (0.5, -2) = the dot's centre");
        let mut edges: Vec<String> = g.scene["edges"].as_array().unwrap().iter().map(|e| e.to_string()).collect();
        edges.sort();
        assert_eq!(
            edges,
            [
                r#"["__dot1",0,"__dot2",0]"#,
                r#"["__dot1",0,"xform1",0]"#,
                r#"["__dot2",0,"merge1",0]"#,
                r#"["box1",0,"__dot1",0]"#,
                r#"["box1",0,"__dot3",0]"#,
                r#"["xform1",0,"merge1",1]"#
            ]
        );
        let scene = serde_json::to_string(&g.scene).unwrap();
        houdini_twin_at(&scene, DOTS_GEO, time_of(&imp)).expect("hexe == Houdini");
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
                todo.extend(
                    edges.iter().filter(|e| e[2] == n.as_str()).filter_map(|e| e[0].as_str().map(String::from)),
                );
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
            let imp = match std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|b| import_hip(&b).map_err(|e| e.to_string()))
            {
                Ok(i) => i,
                Err(e) => {
                    rows += &format!("{id}\t{name}\t-\t0\t0\tPARSE-ERR {e}\n");
                    continue;
                }
            };
            for g in &imp.geos {
                let up = upstream(&g.scene, &g.display);
                let bad: Vec<&str> = g
                    .unsupported
                    .iter()
                    .filter(|u| up.contains(u.split(':').next().unwrap_or("")))
                    .map(String::as_str)
                    .collect();
                let scene = serde_json::to_string(&g.scene).unwrap();
                let verdict = if !bad.is_empty() {
                    format!("UNSUPPORTED-NODE {}", bad.join(" ").replace(':', "/"))
                } else {
                    match std::fs::read(format!("{dir}/{id}/{}.geo", g.name)) {
                        Err(_) => "NO-HOUDINI-GEO".to_string(),
                        Ok(h) => match houdini_twin_at(&scene, &h, time_of(&imp)) {
                            Ok(()) => "MATCH".to_string(),
                            Err(e) if e.starts_with("cook:") => {
                                format!("HEXE-ERR {}", e.chars().take(110).collect::<String>())
                            }
                            Err(e) => {
                                let kind = if e.starts_with("scene:") {
                                    "LOAD-ERR"
                                } else if g.animated.iter().any(|a| up.contains(a)) {
                                    "PARM-DIFF(animated)"
                                } else if e.starts_with("counts") {
                                    "COUNT-DIFF"
                                } else if e.starts_with("P[") {
                                    "P-DIFF"
                                } else if e.starts_with("prim") {
                                    "TOPO-DIFF"
                                } else {
                                    "ATTR-DIFF"
                                };
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
    fn hnode(name: &str, ty: &str, parms: &[(&str, &str)]) -> HipNode {
        HipNode {
            name: name.into(),
            ty: ty.into(),
            parms: parms
                .iter()
                .map(|(k, v)| HipParm {
                    name: (*k).into(),
                    flags: 0,
                    values: vec![v.parse::<f64>().map_or_else(|_| HipValue::Str((*v).into()), HipValue::Num)],
                })
                .collect(),
            inputs: Vec::new(),
            display: false,
            render: false,
            bypass: false,
            template: false,
            has_channels: false,
            channels: Vec::new(),
            pos: None,
            has_children: false,
            lock: false,
            color: None,
            netbox: None,
        }
    }
    /// Houdini 20.5 shapes: the End carries the iteration settings and names its Begin by `blockpath`.
    #[test]
    fn block_pairs_move_iteration_settings_onto_the_begin() {
        let nodes = vec![
            hnode("foreachA", "block_begin", &[("method", "piece"), ("blockpath", "../foreach_end1")]),
            hnode(
                "foreach_end1",
                "block_end",
                &[
                    ("itermethod", "pieces"),
                    ("method", "merge"),
                    ("class", "point"),
                    ("useattrib", "off"),
                    ("blockpath", "../foreachA"),
                ],
            ),
            hnode("fb", "block_begin", &[("method", "feedback")]),
            hnode(
                "fe",
                "block_end",
                &[("itermethod", "count"), ("method", "feedback"), ("iterations", "7"), ("blockpath", "../fb")],
            ),
            hnode("pb", "block_begin", &[("method", "piece")]),
            hnode(
                "pe",
                "block_end",
                &[
                    ("itermethod", "auto"),
                    ("class", "primitive"),
                    ("useattrib", "on"),
                    ("attrib", "class"),
                    ("blockpath", "../pb"),
                ],
            ),
            hnode("lost", "block_end", &[("blockpath", "../nowhere")]),
        ];
        let mut out: Vec<Value> = nodes.iter().map(|n| json!({ "name": n.name, "type": n.ty })).collect();
        let net = HipNetwork {
            name: "g".into(),
            parent: String::new(),
            ty: "geo".into(),
            nodes,
            pos: None,
            display: true,
            selectable: true,
            color: None,
            boxes: vec![],
            notes: vec![],
            dots: vec![],
        };
        let mut notes = Vec::new();
        pair_blocks(&registry(), &net, &mut out, &mut notes);
        let p = |i: usize| out[i]["params"].clone();
        assert_eq!(p(0), json!({ "method": 2 }), "pieces by point, no attribute = one point per iteration");
        assert_eq!(p(1), json!({ "begin": "foreachA", "mode": 0 }));
        assert_eq!(p(2), json!({ "method": 1, "count": 7 }));
        assert_eq!(p(3), json!({ "begin": "fb", "mode": 1 }));
        assert_eq!(
            p(4),
            json!({ "method": 0, "class": 1, "attrib": "class" }),
            "auto + piece begin = pieces by attribute"
        );
        assert!(
            p(6).is_null() && notes.iter().any(|n| n.contains("lost")),
            "unknown block path reported, not invented"
        );
        // the converted pair loads and cooks as a hexe scene
        let scene = json!({ "nodes": [{"name": "b", "type": "box"}, out[2], out[3]], "edges": [["b", 0, "fb", 0], ["fb", 0, "fe", 0]], "display": "fe" });
        let (mut g, id) = crate::load_scene(&scene.to_string()).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 8);
    }
}
