//! Engine session behind the Tauri commands: one `Graph<Detail>` + editor positions. Pure Rust (no Tauri types) so it
//! is unit-tested without a window. Every failure is an `Err(String)` (hostile input is the default, AGENTS.md §4).
use hexe_geo::{Detail, PrimKind};
use hexe_graph::{Graph, NodeId, ParamSet, ParamSpec, ParamValue};
use hexe_sop::Registry;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

/// Largest scene file `graph_load` reads.
pub const MAX_SCENE_BYTES: u64 = 64 << 20;
/// Sibling / level pitch of the auto-layout (flow runs top to bottom, like Houdini) for nodes without a stored position.
const LAYOUT_DX: f32 = 190.0;
const LAYOUT_DY: f32 = 90.0;

#[derive(Serialize, Debug, PartialEq)]
pub struct ParamInfo {
    pub name: String,
    pub label: String,
    /// `float` `int` `toggle` `menu` `string` `vector3`.
    pub kind: &'static str,
    pub default: Value,
    pub min: Option<f32>,
    pub max: Option<f32>,
    /// Menu entries from the engine's `ParamSpec::menu` (empty for non-menu params).
    pub menu: Vec<MenuEntry>,
}

/// One menu choice: `value` is the `ParamValue::Menu` index, `token` the stable scene-file name, `label` the UI text.
#[derive(Serialize, Debug, PartialEq)]
pub struct MenuEntry {
    pub value: u32,
    pub token: String,
    pub label: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct NodeTypeInfo {
    #[serde(rename = "type")]
    pub ty: String,
    pub label: String,
    pub inputs: Vec<String>,
    pub params: Vec<ParamInfo>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct NodeView {
    pub id: u32,
    #[serde(rename = "type")]
    pub ty: String,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub inputs: Vec<String>,
    /// Every spec'd param with its current value (defaults included): the param pane edits this.
    pub params: BTreeMap<String, Value>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct EdgeView {
    pub from: u32,
    pub to: u32,
    pub input: usize,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct GraphView {
    pub nodes: Vec<NodeView>,
    pub edges: Vec<EdgeView>,
    pub display: Option<u32>,
}

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct Counts {
    pub points: u32,
    pub prims: u32,
    pub closed: u32,
    pub open: u32,
    /// Prims the viewport cannot draw (spheres, volumes, ...): sent as empty entries.
    pub unsupported: u32,
}

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct CookResult {
    /// Flat `x,y,z` per point (indexable by the `prims` entries).
    pub positions: Vec<f32>,
    /// Point indices per prim.
    pub prims: Vec<Vec<u32>>,
    pub closed: Vec<bool>,
    pub counts: Counts,
    pub error: Option<String>,
}

pub struct Session {
    graph: Graph<Detail>,
    reg: Registry,
    pos: BTreeMap<String, [f32; 2]>,
}

impl Default for Session {
    fn default() -> Self {
        Session { graph: Graph::new(), reg: hexe_sop::registry(), pos: BTreeMap::new() }
    }
}

fn kind_of(v: &ParamValue) -> &'static str {
    match v {
        ParamValue::F(_) => "float",
        ParamValue::I(_) => "int",
        ParamValue::B(_) => "toggle",
        ParamValue::V3(_) => "vector3",
        ParamValue::S(_) => "string",
        ParamValue::Menu(_) => "menu",
    }
}

/// f32 via its shortest decimal, so the UI shows `0.1` and not `0.10000000149`.
fn num(x: f32) -> Value {
    format!("{x}").parse::<f64>().ok().and_then(serde_json::Number::from_f64).map_or(Value::Null, Value::Number)
}

pub fn param_json(v: &ParamValue) -> Value {
    match v {
        ParamValue::F(x) => num(*x),
        ParamValue::I(x) => Value::from(*x),
        ParamValue::B(x) => Value::Bool(*x),
        ParamValue::V3(c) => Value::Array(c.iter().map(|x| num(*x)).collect()),
        ParamValue::S(s) => Value::String(s.clone()),
        ParamValue::Menu(m) => Value::from(*m),
    }
}

fn finite(v: &Value) -> Result<f32, String> {
    let x = v.as_f64().ok_or("expected a number")?;
    let f = x as f32;
    if f.is_finite() { Ok(f) } else { Err(format!("{x} is not a finite f32")) }
}

fn whole(v: &Value) -> Result<i64, String> {
    if let Some(i) = v.as_i64() {
        return Ok(i);
    }
    let f = v.as_f64().ok_or("expected a number")?;
    if f.fract() == 0.0 && f.abs() < 9.0e15 { Ok(f as i64) } else { Err(format!("{f} is not a whole number")) }
}

/// JSON from the UI -> `ParamValue` of the same kind as `like` (the spec default).
pub fn param_from_json(like: &ParamValue, v: &Value) -> Result<ParamValue, String> {
    Ok(match like {
        ParamValue::F(_) => ParamValue::F(finite(v)?),
        ParamValue::I(_) => ParamValue::I(i32::try_from(whole(v)?).map_err(|_| "integer out of range")?),
        ParamValue::Menu(_) => ParamValue::Menu(u32::try_from(whole(v)?).map_err(|_| "menu index out of range")?),
        ParamValue::B(_) => match v {
            Value::Bool(b) => ParamValue::B(*b),
            other => ParamValue::B(whole(other)? != 0),
        },
        ParamValue::S(_) => ParamValue::S(v.as_str().ok_or("expected a string")?.to_string()),
        ParamValue::V3(_) => {
            let a = v.as_array().filter(|a| a.len() == 3).ok_or("expected [x,y,z]")?;
            ParamValue::V3([finite(&a[0])?, finite(&a[1])?, finite(&a[2])?])
        }
    })
}

fn param_info(s: &ParamSpec) -> ParamInfo {
    ParamInfo {
        name: s.name.clone(),
        label: s.label.clone(),
        kind: kind_of(&s.default),
        default: param_json(&s.default),
        min: s.range.map(|r| r.0),
        max: s.range.map(|r| r.1),
        menu: s
            .menu
            .iter()
            .enumerate()
            .map(|(i, (t, l))| MenuEntry {
                value: u32::try_from(i).unwrap_or(u32::MAX),
                token: t.clone(),
                label: l.clone(),
            })
            .collect(),
    }
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    fn find(&self, id: u32) -> Result<NodeId, String> {
        self.graph
            .node_ids()
            .into_iter()
            .find(|n| n.index() == id && self.graph.name(*n).is_some())
            .ok_or_else(|| format!("no such node {id}"))
    }

    /// Every registered SOP with its param schema (sorted by type).
    pub fn list_node_types(&self) -> Vec<NodeTypeInfo> {
        self.reg
            .type_names()
            .into_iter()
            .filter_map(|t| {
                let node = self.reg.create(t)?;
                Some(NodeTypeInfo {
                    ty: t.to_string(),
                    label: t.to_string(),
                    inputs: node.inputs().iter().map(|i| i.name.to_string()).collect(),
                    params: self.reg.specs(t)?.iter().map(param_info).collect(),
                })
            })
            .collect()
    }

    /// Fresh empty graph.
    pub fn graph_new(&mut self) {
        self.graph = Graph::new();
        self.pos.clear();
    }

    /// Replace the session graph with the scene at `path` (relative `file` SOPs resolve against its directory).
    pub fn graph_load(&mut self, path: &str) -> Result<(), String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
        if meta.len() > MAX_SCENE_BYTES {
            return Err(format!("{path}: scene larger than {MAX_SCENE_BYTES} bytes"));
        }
        let json = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let l =
            hexe_sop::load_scene_with_base(&json, &self.reg, Path::new(path).parent()).map_err(|e| e.to_string())?;
        self.graph = l.graph;
        self.pos = l.pos;
        self.auto_layout();
        Ok(())
    }

    /// Write the scene (nodes, non-default params, edges, display, `ui.pos`) to `path`.
    pub fn graph_save(&self, path: &str) -> Result<(), String> {
        let text = hexe_sop::save_scene(&self.graph, &self.pos).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{path}: {e}"))
    }

    /// Give every named node without a stored position a level (y) = depth, slot (x) = order within the level.
    fn auto_layout(&mut self) {
        let ids: Vec<NodeId> = self.named();
        let mut depth: BTreeMap<u32, usize> = BTreeMap::new();
        // iterate to a fixpoint (depth = 1 + max input depth); bounded by node count (cycles cannot be wired).
        for _ in 0..=ids.len() {
            let mut changed = false;
            for &id in &ids {
                let d = self
                    .graph
                    .input_sources(id)
                    .into_iter()
                    .flatten()
                    .filter(|s| self.graph.name(*s).is_some())
                    .map(|s| depth.get(&s.index()).copied().unwrap_or(0) + 1)
                    .max()
                    .unwrap_or(0);
                if depth.insert(id.index(), d) != Some(d) {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut rows: BTreeMap<usize, usize> = BTreeMap::new();
        for &id in &ids {
            let Some(name) = self.graph.name(id).map(str::to_string) else { continue };
            let d = depth.get(&id.index()).copied().unwrap_or(0);
            let row = rows.entry(d).or_insert(0);
            self.pos.entry(name).or_insert([40.0 + *row as f32 * LAYOUT_DX, 30.0 + d as f32 * LAYOUT_DY]);
            *row += 1;
        }
    }

    fn named(&self) -> Vec<NodeId> {
        self.graph.node_ids().into_iter().filter(|n| self.graph.name(*n).is_some()).collect()
    }

    pub fn graph_get(&self) -> GraphView {
        let ids = self.named();
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        for &id in &ids {
            let name = self.graph.name(id).unwrap_or_default().to_string();
            let [x, y] = self.pos.get(&name).copied().unwrap_or([0.0, 0.0]);
            let params = self
                .graph
                .param_specs(id)
                .iter()
                .map(|s| (s.name.clone(), param_json(self.graph.param(id, &s.name).unwrap_or(&s.default))))
                .collect();
            let inputs = self
                .reg
                .create(self.graph.node_type(id).unwrap_or_default())
                .map_or_else(Vec::new, |n| n.inputs().iter().map(|i| i.name.to_string()).collect());
            nodes.push(NodeView {
                id: id.index(),
                ty: self.graph.node_type(id).unwrap_or_default().to_string(),
                name,
                x,
                y,
                inputs,
                params,
            });
            for (port, src) in self.graph.input_sources(id).into_iter().enumerate() {
                if let Some(s) = src.filter(|s| self.graph.name(*s).is_some()) {
                    edges.push(EdgeView { from: s.index(), to: id.index(), input: port });
                }
            }
        }
        GraphView { nodes, edges, display: self.graph.display().map(NodeId::index) }
    }

    /// Add a node of `ty` at editor position (x, y); the first node becomes the display node.
    pub fn node_add(&mut self, ty: &str, x: f32, y: f32) -> Result<u32, String> {
        if !x.is_finite() || !y.is_finite() {
            return Err("non-finite position".into());
        }
        let node = self.reg.create(ty).ok_or_else(|| format!("unknown node type `{ty}`"))?;
        let id = self.graph.add_boxed(node, ParamSet::new()).map_err(|e| e.to_string())?;
        let name = self.graph.unique_name(ty);
        self.graph.rename(id, &name).map_err(|e| e.to_string())?;
        self.pos.insert(name, [x, y]);
        if self.graph.display().is_none() {
            self.graph.set_display(Some(id)).map_err(|e| e.to_string())?;
        }
        self.fill()?;
        Ok(id.index())
    }

    fn fill(&mut self) -> Result<(), String> {
        hexe_sop::fill_unwired(&mut self.graph).map_err(|e| e.to_string())
    }

    pub fn node_remove(&mut self, id: u32) -> Result<(), String> {
        let n = self.find(id)?;
        let name = self.graph.name(n).map(str::to_string);
        let was_display = self.graph.display() == Some(n);
        self.graph.remove_node(n).map_err(|e| e.to_string())?;
        if let Some(name) = name {
            self.pos.remove(&name);
        }
        if was_display {
            let next = self.named().first().copied();
            self.graph.set_display(next).map_err(|e| e.to_string())?;
        }
        self.fill()
    }

    /// Move a node in the editor (UI-only state, saved as `ui.pos`).
    pub fn node_move(&mut self, id: u32, x: f32, y: f32) -> Result<(), String> {
        if !x.is_finite() || !y.is_finite() {
            return Err("non-finite position".into());
        }
        let n = self.find(id)?;
        let name = self.graph.name(n).ok_or("unnamed node")?.to_string();
        self.pos.insert(name, [x, y]);
        Ok(())
    }

    pub fn node_set_param(&mut self, id: u32, name: &str, value: &Value) -> Result<(), String> {
        let n = self.find(id)?;
        let spec = self
            .graph
            .param_specs(n)
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("node has no parameter `{name}`"))?;
        let v = param_from_json(&spec.default, value).map_err(|m| format!("parameter `{name}`: {m}"))?;
        self.graph.set_param(n, name, v).map_err(|e| e.to_string())
    }

    pub fn edge_connect(&mut self, from: u32, to: u32, input: usize) -> Result<(), String> {
        let (s, d) = (self.find(from)?, self.find(to)?);
        self.graph.connect(s, 0, d, input).map_err(|e| e.to_string())
    }

    /// Cut the wire into `to`'s `input` (the port is re-fed from the hidden empty source).
    pub fn edge_disconnect(&mut self, to: u32, input: usize) -> Result<(), String> {
        let d = self.find(to)?;
        self.graph.disconnect(d, input).map_err(|e| e.to_string())?;
        self.fill()
    }

    pub fn set_display(&mut self, id: u32) -> Result<(), String> {
        let n = self.find(id)?;
        self.graph.set_display(Some(n)).map_err(|e| e.to_string())
    }

    /// Cook the display node. Never panics, never `Err`: a failure is `CookResult.error` (the UI shows it).
    pub fn cook(&mut self) -> CookResult {
        let Some(display) = self.graph.display() else {
            return CookResult { error: Some("no display node".into()), ..CookResult::default() };
        };
        let graph = &mut self.graph;
        match catch_unwind(AssertUnwindSafe(|| graph.cook(display, 0.0))) {
            Ok(Ok(detail)) => flatten(&detail),
            Ok(Err(e)) => CookResult { error: Some(e.to_string()), ..CookResult::default() },
            Err(_) => CookResult { error: Some("cook panicked (engine bug)".into()), ..CookResult::default() },
        }
    }
}

/// `Detail` -> the viewport payload. Polys/polylines become point-index lists; other prim kinds are counted, not drawn.
pub fn flatten(d: &Detail) -> CookResult {
    let positions: Vec<f32> = d.p().iter().flat_map(|p| p.iter().copied()).collect();
    let mut counts = Counts { points: d.point_count, ..Counts::default() };
    let (mut prims, mut closed) = (Vec::new(), Vec::new());
    for pr in &d.prims {
        let (is_closed, drawable) = match pr.kind {
            PrimKind::Poly { closed } => (closed, true),
            PrimKind::Polyline => (false, true),
            _ => (false, false),
        };
        let verts: Vec<u32> = if drawable {
            let range = pr.verts.start as usize..pr.verts.end as usize;
            d.vertex_point.get(range).unwrap_or(&[]).to_vec()
        } else {
            Vec::new()
        };
        match (drawable, is_closed) {
            (false, _) => counts.unsupported += 1,
            (true, true) => counts.closed += 1,
            (true, false) => counts.open += 1,
        }
        prims.push(verts);
        closed.push(is_closed);
    }
    counts.prims = d.prim_count();
    CookResult { positions, prims, closed, counts, error: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn example(name: &str) -> String {
        format!("{}/../../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn node_types_carry_schema() {
        let s = Session::new();
        let t = s.list_node_types();
        let pe = t.iter().find(|n| n.ty == "polyextrude").expect("polyextrude registered");
        let dist = pe.params.iter().find(|p| p.name == "distance").expect("distance");
        assert_eq!((dist.kind, dist.min, dist.max), ("float", Some(-1.0), Some(1.0)));
        let split = pe.params.iter().find(|p| p.name == "splittype").expect("splittype");
        assert_eq!(split.kind, "menu");
        let labels: Vec<&str> = split.menu.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["Connected Components", "Individual Elements", "Groups"]);
        assert_eq!(split.menu[1].value, 1);
        assert!(t.iter().flat_map(|n| &n.params).filter(|p| p.kind == "menu").all(|p| !p.menu.is_empty()));
        assert_eq!(t.iter().map(|n| n.ty.as_str()).collect::<Vec<_>>().len(), hexe_sop::registry().type_names().len());
    }

    #[test]
    fn load_polyextrude_get_and_cook() {
        let mut s = Session::new();
        s.graph_load(&example("polyextrude.json")).expect("loads");
        let g = s.graph_get();
        assert_eq!(g.nodes.len(), 2, "hidden empty source never reaches the UI");
        assert_eq!(g.edges.len(), 1);
        let pe = g.nodes.iter().find(|n| n.ty == "polyextrude").expect("pe");
        assert_eq!(g.display, Some(pe.id));
        assert_eq!(pe.params["distance"], json!(0.5));
        let c = s.cook();
        assert_eq!(c.error, None);
        assert_eq!(c.positions.len(), c.counts.points as usize * 3);
        assert!(c.counts.closed > 6, "extruded box has more than the 6 box faces");
        assert!(c.prims.iter().flatten().all(|&i| (i as usize) < c.counts.points as usize));
    }

    #[test]
    fn set_param_recooks_to_different_geometry() {
        let mut s = Session::new();
        s.graph_load(&example("polyextrude.json")).expect("loads");
        let pe = s.graph_get().nodes.iter().find(|n| n.ty == "polyextrude").map(|n| n.id).expect("pe");
        let a = s.cook();
        s.node_set_param(pe, "distance", &json!(1.0)).expect("set");
        let b = s.cook();
        assert_eq!(b.error, None);
        assert_ne!(a.positions, b.positions);
        assert!(s.node_set_param(pe, "distance", &json!("x")).is_err());
        assert!(s.node_set_param(pe, "nope", &json!(1)).is_err());
        assert!(s.node_set_param(pe, "distance", &json!(null)).is_err());
    }

    #[test]
    fn build_in_ui_add_wire_display_remove() {
        let mut s = Session::new();
        let b = s.node_add("box", 0.0, 0.0).expect("box");
        let pe = s.node_add("polyextrude", 200.0, 0.0).expect("pe");
        assert_eq!(s.graph_get().display, Some(b), "first node is the display node");
        s.edge_connect(b, pe, 0).expect("wire");
        s.set_display(pe).expect("display");
        assert_eq!(s.graph_get().edges, vec![EdgeView { from: b, to: pe, input: 0 }]);
        assert_eq!(s.cook().error, None);
        s.edge_disconnect(pe, 0).expect("cut");
        assert!(s.graph_get().edges.is_empty());
        assert_eq!(s.cook().error, None, "unwired input is fed empty geometry");
        assert!(s.edge_connect(pe, pe, 0).is_err(), "self loop refused, not panicked");
        s.node_remove(pe).expect("remove");
        let g = s.graph_get();
        assert_eq!((g.nodes.len(), g.display), (1, Some(b)), "display falls back to a live node");
        assert!(s.node_remove(pe).is_err());
        assert!(s.node_add("no-such-sop", 0.0, 0.0).is_err());
    }

    #[test]
    fn save_roundtrips_through_load() {
        let mut s = Session::new();
        s.graph_load(&example("polyextrude.json")).expect("loads");
        let pe = s.graph_get().nodes.iter().find(|n| n.ty == "polyextrude").map(|n| n.id).expect("pe");
        s.node_set_param(pe, "distance", &json!(0.25)).expect("set");
        s.node_move(pe, 321.0, 12.0).expect("move");
        let out = std::env::temp_dir().join(format!("hexe-tauri-rt-{}.json", std::process::id()));
        let path = out.to_string_lossy().into_owned();
        s.graph_save(&path).expect("save");
        let mut t = Session::new();
        t.graph_load(&path).expect("reload");
        let _ = std::fs::remove_file(&out);
        let (a, b) = (s.graph_get(), t.graph_get());
        assert_eq!(a.edges.len(), b.edges.len());
        let pe2 = b.nodes.iter().find(|n| n.ty == "polyextrude").expect("pe2");
        assert_eq!((pe2.x, pe2.y, &pe2.params["distance"]), (321.0, 12.0, &json!(0.25)));
        assert_eq!(s.cook().positions, t.cook().positions);
    }

    #[test]
    fn hostile_input_is_err_not_panic() {
        let mut s = Session::new();
        assert!(s.graph_load("/definitely/not/here.json").is_err());
        assert_eq!(s.cook().error.as_deref(), Some("no display node"));
        assert!(s.graph_save("/definitely/not/here/out.json").is_err());
        assert!(s.node_move(99, f32::NAN, 0.0).is_err());
        assert!(s.set_display(5).is_err());
    }
}
