//! Engine session behind the Tauri commands: one `Graph<Detail>` + editor positions. Pure Rust (no Tauri types) so it
//! is unit-tested without a window. Every failure is an `Err(String)` (hostile input is the default, AGENTS.md §4).
use crate::sheet::{self, Sheet};
use hexe_geo::{Detail, PrimKind};
use hexe_graph::{Graph, GraphError, NodeId, ParamSet, ParamSpec, ParamValue};
use hexe_sop::Registry;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;

/// Largest scene file `graph_load` reads.
pub const MAX_SCENE_BYTES: u64 = 64 << 20;
/// Undo / redo depth (oldest snapshot dropped beyond it).
pub const MAX_UNDO: usize = 100;
/// Offset (editor px) of each successive paste from the copied nodes.
pub const PASTE_OFFSET: f32 = 20.0;
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
    /// `Some("vex")` = source-code string (the UI draws an editor with inline errors); from `ParamSpec::language`.
    pub language: Option<String>,
    /// Allowed values of a string param (dropdown, e.g. the wrangle's `class` = Run Over); empty = free text.
    pub choices: Vec<String>,
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
    /// Sphere / tube prims: drawn as wire proxies from `CookResult.shapes`.
    pub shapes: u32,
}

/// A sphere / tube prim for the viewport's wire proxy: `center` = its point, `xform` = row-major 3x3 (unit shape -> world).
#[derive(Serialize, Debug, PartialEq)]
pub struct Shape {
    pub kind: &'static str,
    pub center: [f32; 3],
    pub xform: [f32; 9],
    pub taper: f32,
}

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct CookResult {
    /// Flat `x,y,z` per point (indexable by the `prims` entries).
    pub positions: Vec<f32>,
    /// Point indices per prim.
    pub prims: Vec<Vec<u32>>,
    pub closed: Vec<bool>,
    pub shapes: Vec<Shape>,
    /// rgb per prim CORNER (flattened `prims` order): vertex `Cd`, else point `Cd` via the corner's point, else prim `Cd`.
    /// `None` when the geometry has no V3 `Cd` at all.
    pub cd: Option<Vec<f32>>,
    /// rgb per point, only when a point `Cd` exists (colours the points layer).
    pub cd_point: Option<Vec<f32>>,
    pub counts: Counts,
    pub error: Option<String>,
    /// The node whose cook failed (the UI borders it red); `None` when the failure is not tied to one node.
    pub error_node: Option<u32>,
}

pub struct Session {
    graph: Graph<Detail>,
    reg: Registry,
    pos: BTreeMap<String, [f32; 2]>,
    /// Output of the last successful display cook (what the spreadsheet shows); `None` after a failed / empty cook.
    last: Option<Arc<Detail>>,
    /// `--set node.param=value` overrides: re-applied on EVERY `graph_load`.
    sets: Vec<String>,
    /// Failures of the last `--set` re-application (the load itself succeeded).
    pub set_errors: Vec<String>,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// Coalescing key of the last recorded edit (a scrub = many `node_set_param` = one undo step).
    last_key: Option<String>,
    clip: Option<Clip>,
    pastes: u32,
    /// Viewport point selection (ptnums of the display geometry), pushed by the UI; see `selection_get`.
    selection: Vec<u32>,
}

/// Whole-graph snapshot for undo / redo: the scene text (names, params, wires, `ui.pos`) + the display node name
/// (the scene format cannot say "no display"). `scene = None` = empty graph.
#[derive(Clone, Debug, PartialEq)]
struct Snap {
    scene: Option<String>,
    display: Option<String>,
}

#[derive(Clone, Debug)]
struct ClipNode {
    ty: String,
    name: String,
    params: Vec<(String, ParamValue)>,
    pos: [f32; 2],
}

/// Copied nodes: type, original name, param values, position; edges are index pairs `(from, to, input)` inside the set.
#[derive(Clone, Debug)]
struct Clip {
    nodes: Vec<ClipNode>,
    edges: Vec<(usize, usize, usize)>,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            graph: Graph::new(),
            reg: hexe_sop::registry(),
            pos: BTreeMap::new(),
            last: None,
            sets: Vec::new(),
            set_errors: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            last_key: None,
            clip: None,
            pastes: 0,
            selection: Vec::new(),
        }
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

/// `node.param=value` -> `(node, param, value)`.
pub fn split_set(arg: &str) -> Result<(&str, &str, &str), String> {
    let bad = || format!("--set {arg}: want node.param=value");
    let (key, val) = arg.split_once('=').ok_or_else(bad)?;
    let (node, param) = key.split_once('.').ok_or_else(bad)?;
    if node.is_empty() || param.is_empty() { Err(bad()) } else { Ok((node, param, val)) }
}

/// Open limit: a scene file larger than `MAX_SCENE_BYTES` is refused before it is read.
fn check_scene_size(path: &str, len: u64) -> Result<(), String> {
    if len > MAX_SCENE_BYTES {
        return Err(format!(
            "{path}: scene is {len} bytes, larger than the {MAX_SCENE_BYTES} byte (64 MiB) open limit"
        ));
    }
    Ok(())
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
        language: s.language.clone(),
        choices: s.choices.clone(),
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
        self.reset_history();
    }

    fn reset_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.last_key = None;
    }

    /// Replace the session graph with the scene at `path` (relative `file` SOPs resolve against its directory).
    pub fn graph_load(&mut self, path: &str) -> Result<(), String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
        check_scene_size(path, meta.len())?;
        let json = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let l =
            hexe_sop::load_scene_with_base(&json, &self.reg, Path::new(path).parent()).map_err(|e| e.to_string())?;
        self.graph = l.graph;
        self.pos = l.pos;
        self.auto_layout();
        self.apply_sets();
        self.reset_history();
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

    fn node_add_raw(&mut self, ty: &str, x: f32, y: f32) -> Result<u32, String> {
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

    fn node_remove_raw(&mut self, id: u32) -> Result<(), String> {
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

    fn node_move_raw(&mut self, id: u32, x: f32, y: f32) -> Result<(), String> {
        if !x.is_finite() || !y.is_finite() {
            return Err("non-finite position".into());
        }
        let n = self.find(id)?;
        let name = self.graph.name(n).ok_or("unnamed node")?.to_string();
        self.pos.insert(name, [x, y]);
        Ok(())
    }

    fn node_set_param_raw(&mut self, id: u32, name: &str, value: &Value) -> Result<(), String> {
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

    fn edge_connect_raw(&mut self, from: u32, to: u32, input: usize) -> Result<(), String> {
        let (s, d) = (self.find(from)?, self.find(to)?);
        self.graph.connect(s, 0, d, input).map_err(|e| e.to_string())
    }

    fn edge_disconnect_raw(&mut self, to: u32, input: usize) -> Result<(), String> {
        let d = self.find(to)?;
        self.graph.disconnect(d, input).map_err(|e| e.to_string())?;
        self.fill()
    }

    fn set_display_raw(&mut self, id: Option<u32>) -> Result<(), String> {
        let n = id.map(|i| self.find(i)).transpose()?;
        self.graph.set_display(n).map_err(|e| e.to_string())
    }

    /// Add a node of `ty` at editor position (x, y); the first node becomes the display node.
    pub fn node_add(&mut self, ty: &str, x: f32, y: f32) -> Result<u32, String> {
        self.mutate(None, |s| s.node_add_raw(ty, x, y))
    }

    pub fn node_remove(&mut self, id: u32) -> Result<(), String> {
        self.mutate(None, |s| s.node_remove_raw(id))
    }

    /// Remove several nodes as ONE undo step (all ids are validated first: nothing is removed on a bad id).
    pub fn nodes_remove(&mut self, ids: &[u32]) -> Result<(), String> {
        for &i in ids {
            self.find(i)?;
        }
        self.mutate(None, |s| ids.iter().try_for_each(|&i| s.node_remove_raw(i)))
    }

    /// Move a node in the editor (UI-only state, saved as `ui.pos`).
    pub fn node_move(&mut self, id: u32, x: f32, y: f32) -> Result<(), String> {
        self.mutate(None, |s| s.node_move_raw(id, x, y))
    }

    /// Move several nodes as ONE undo step (all ids validated first).
    pub fn nodes_move(&mut self, moves: &[(u32, f32, f32)]) -> Result<(), String> {
        for &(i, x, y) in moves {
            self.find(i)?;
            if !x.is_finite() || !y.is_finite() {
                return Err("non-finite position".into());
            }
        }
        self.mutate(None, |s| moves.iter().try_for_each(|&(i, x, y)| s.node_move_raw(i, x, y)))
    }

    /// Set a parameter; consecutive sets of the same parameter coalesce into one undo step (a scrub).
    pub fn node_set_param(&mut self, id: u32, name: &str, value: &Value) -> Result<(), String> {
        self.mutate(Some(format!("{id}:{name}")), |s| s.node_set_param_raw(id, name, value))
    }

    pub fn edge_connect(&mut self, from: u32, to: u32, input: usize) -> Result<(), String> {
        self.mutate(None, |s| s.edge_connect_raw(from, to, input))
    }

    /// Cut the wire into `to`'s `input` (the port is re-fed from the hidden empty source).
    pub fn edge_disconnect(&mut self, to: u32, input: usize) -> Result<(), String> {
        self.mutate(None, |s| s.edge_disconnect_raw(to, input))
    }

    /// Set the display node, or clear it (`None`; the UI toggles by sending `None` for the current display node).
    pub fn set_display(&mut self, id: Option<u32>) -> Result<(), String> {
        self.mutate(None, |s| s.set_display_raw(id))
    }

    /// Rename a node. Names are unique and `[A-Za-z0-9_-]+` (they key `ui.pos`, edges and `--set node.param`).
    pub fn node_rename(&mut self, id: u32, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(format!("invalid node name `{name}` (letters, digits, `_` and `-` only)"));
        }
        self.mutate(None, |s| {
            let n = s.find(id)?;
            let old = s.graph.name(n).unwrap_or_default().to_string();
            if old == name {
                return Ok(());
            }
            s.graph.rename(n, name).map_err(|e| e.to_string())?;
            if let Some(p) = s.pos.remove(&old) {
                s.pos.insert(name.to_string(), p);
            }
            Ok(())
        })
    }

    // ---- viewport point selection ----

    /// Replace the point selection (sorted, deduplicated). Ids beyond the displayed point count are kept as given: the UI owns
    /// validity, `group_from_selection` re-checks against the last cook.
    pub fn selection_set(&mut self, ids: &[u32]) {
        let mut v = ids.to_vec();
        v.sort_unstable();
        v.dedup();
        self.selection = v;
    }

    /// The selected ptnums (ascending).
    pub fn selection_get(&self) -> Vec<u32> {
        self.selection.clone()
    }

    /// "Group from selection": a new `group` SOP (point group `selection`, `pattern` = the explicit ptnum list) wired under the
    /// display node and made the display node: ONE undo step. Returns the new node id.
    pub fn group_from_selection(&mut self) -> Result<u32, String> {
        let ids = self.selection.clone();
        if ids.is_empty() {
            return Err("no points selected (press S in the viewport, then click or drag-box points)".into());
        }
        let disp = self.graph.display().ok_or("no display node to group points of")?;
        if let Some(d) = &self.last
            && let Some(&bad) = ids.iter().find(|&&i| i >= d.point_count)
        {
            return Err(format!("selected point {bad} is outside the displayed geometry ({} points)", d.point_count));
        }
        let disp_id = disp.index();
        let name = self.graph.name(disp).unwrap_or_default().to_string();
        let [x, y] = self.pos.get(&name).copied().unwrap_or([0.0, 0.0]);
        let pattern = ptnum_pattern(&ids);
        self.mutate(None, |s| {
            let id = s.node_add_raw("group", x, y + LAYOUT_DY)?;
            s.node_set_param_raw(id, "grouptype", &Value::from(0))?;
            s.node_set_param_raw(id, "groupname", &Value::from("selection"))?;
            s.node_set_param_raw(id, "pattern", &Value::from(pattern))?;
            s.edge_connect_raw(disp_id, id, 0)?;
            s.set_display_raw(Some(id))?;
            Ok(id)
        })
    }

    // ---- undo / redo ----

    fn snapshot(&self) -> Snap {
        let display = self.graph.display().and_then(|d| self.graph.name(d)).map(str::to_string);
        Snap { scene: hexe_sop::save_scene(&self.graph, &self.pos).ok(), display }
    }

    fn restore(&mut self, snap: &Snap) -> Result<(), String> {
        self.last = None;
        match &snap.scene {
            None => {
                self.graph = Graph::new();
                self.pos.clear();
            }
            Some(text) => {
                let l = hexe_sop::load_scene_with_base(text, &self.reg, None).map_err(|e| e.to_string())?;
                self.graph = l.graph;
                self.pos = l.pos;
                let d = snap.display.as_deref().and_then(|n| self.graph.node_by_name(n));
                self.graph.set_display(d).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Run one graph edit and record the pre-edit snapshot (skipped if the edit changed nothing, or if `key` equals the
    /// previous edit's key: a scrub). Any edit clears redo.
    fn mutate<T>(&mut self, key: Option<String>, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let before = self.snapshot();
        let out = f(self)?;
        if self.snapshot() == before {
            return Ok(out); // changed nothing: no history entry, the scrub chain stays intact
        }
        if key.is_none() || key != self.last_key {
            self.undo.push(before);
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.last_key = key;
        self.redo.clear();
        Ok(out)
    }

    /// Step back one edit. `Ok(false)` = nothing to undo. Node ids are NOT stable across an undo (the UI re-reads the graph).
    pub fn undo(&mut self) -> Result<bool, String> {
        let Some(prev) = self.undo.pop() else { return Ok(false) };
        let now = self.snapshot();
        self.restore(&prev)?;
        self.redo.push(now);
        self.last_key = None;
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, String> {
        let Some(next) = self.redo.pop() else { return Ok(false) };
        let now = self.snapshot();
        self.restore(&next)?;
        self.undo.push(now);
        self.last_key = None;
        Ok(true)
    }

    /// `(undo depth, redo depth)`.
    pub fn history(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    // ---- copy / paste ----

    /// Copy `ids` (type, params, position, and the wires with both ends inside the set). Returns how many nodes.
    pub fn copy(&mut self, ids: &[u32]) -> Result<usize, String> {
        let mut nodes = Vec::new();
        let mut ord: BTreeMap<u32, usize> = BTreeMap::new();
        for &i in ids {
            let n = self.find(i)?;
            if ord.contains_key(&i) {
                continue;
            }
            let name = self.graph.name(n).unwrap_or_default().to_string();
            let params = self
                .graph
                .param_specs(n)
                .iter()
                .filter_map(|sp| Some((sp.name.clone(), self.graph.param(n, &sp.name)?.clone())))
                .collect();
            let pos = self.pos.get(&name).copied().unwrap_or([0.0, 0.0]);
            ord.insert(i, nodes.len());
            nodes.push(ClipNode { ty: self.graph.node_type(n).unwrap_or_default().to_string(), name, params, pos });
        }
        if nodes.is_empty() {
            return Err("nothing selected to copy".into());
        }
        let mut edges = Vec::new();
        for (&i, &to) in &ord {
            let n = self.find(i)?;
            for (port, src) in self.graph.input_sources(n).into_iter().enumerate() {
                if let Some(&from) = src.and_then(|s| ord.get(&s.index())) {
                    edges.push((from, to, port));
                }
            }
        }
        let count = nodes.len();
        self.clip = Some(Clip { nodes, edges });
        self.pastes = 0;
        Ok(count)
    }

    /// Paste the clipboard as fresh uniquely-named nodes, `PASTE_OFFSET` px further per paste; one undo step.
    /// Returns the new ids (clipboard order).
    pub fn paste(&mut self) -> Result<Vec<u32>, String> {
        let clip = self.clip.clone().ok_or("clipboard is empty (copy nodes first)")?;
        self.pastes += 1;
        let off = PASTE_OFFSET * self.pastes as f32;
        self.mutate(None, |s| {
            let mut ids: Vec<NodeId> = Vec::new();
            for ClipNode { ty, name, params, pos } in &clip.nodes {
                let node = s.reg.create(ty).ok_or_else(|| format!("unknown node type `{ty}`"))?;
                let id = s.graph.add_boxed(node, ParamSet::new()).map_err(|e| e.to_string())?;
                for (k, v) in params {
                    s.graph.set_param(id, k, v.clone()).map_err(|e| e.to_string())?;
                }
                let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
                let fresh = s.graph.unique_name(if base.is_empty() { ty } else { base });
                s.graph.rename(id, &fresh).map_err(|e| e.to_string())?;
                s.pos.insert(fresh, [pos[0] + off, pos[1] + off]);
                ids.push(id);
            }
            for &(from, to, input) in &clip.edges {
                if let (Some(&a), Some(&b)) = (ids.get(from), ids.get(to)) {
                    s.graph.connect(a, 0, b, input).map_err(|e| e.to_string())?;
                }
            }
            s.fill()?;
            Ok(ids.into_iter().map(NodeId::index).collect())
        })
    }

    /// `--set node.param=value` overrides, re-applied on every `graph_load` (failures land in `set_errors`).
    pub fn set_overrides(&mut self, sets: Vec<String>) -> Result<(), String> {
        for a in &sets {
            split_set(a)?;
        }
        self.sets = sets;
        Ok(())
    }

    fn apply_sets(&mut self) {
        let sets = self.sets.clone();
        self.set_errors = sets.iter().filter_map(|a| self.set_arg(a).err()).collect();
        for e in &self.set_errors {
            eprintln!("hexe: {e}");
        }
    }

    /// `node.param=value` typed by the param's kind (`vector3` = `x,y,z`; `toggle` = `1|true|on`). An edit like any other.
    pub fn set_arg(&mut self, arg: &str) -> Result<(), String> {
        let (node, param, val) = split_set(arg)?;
        let id = self.id_of(node).ok_or_else(|| format!("--set {arg}: no node `{node}`"))?;
        let n = self.find(id)?;
        let spec = self
            .graph
            .param_specs(n)
            .into_iter()
            .find(|s| s.name == param)
            .ok_or_else(|| format!("--set {arg}: node `{node}` has no parameter `{param}`"))?;
        let bad = || format!("--set {arg}: bad value `{val}`");
        let json = match spec.default {
            ParamValue::F(_) | ParamValue::I(_) | ParamValue::Menu(_) => {
                Value::from(val.trim().parse::<f64>().map_err(|_| bad())?)
            }
            ParamValue::B(_) => Value::Bool(matches!(val.trim(), "1" | "true" | "on")),
            ParamValue::S(_) => Value::String(val.to_string()),
            ParamValue::V3(_) => {
                let c: Vec<f64> =
                    val.split(',').map(|c| c.trim().parse::<f64>().map_err(|_| bad())).collect::<Result<_, _>>()?;
                if c.len() != 3 {
                    return Err(bad());
                }
                Value::from(c)
            }
        };
        self.node_set_param(id, param, &json)
    }

    /// Node id by editor name.
    pub fn id_of(&self, name: &str) -> Option<u32> {
        self.named().into_iter().find(|n| self.graph.name(*n) == Some(name)).map(NodeId::index)
    }

    /// Number of named nodes (script auto-placement).
    pub fn node_count(&self) -> usize {
        self.named().len()
    }

    /// Name of node `id`.
    pub fn name_of(&self, id: u32) -> Option<String> {
        let n = self.find(id).ok()?;
        self.graph.name(n).map(str::to_string)
    }

    /// Cook the display node. Never panics, never `Err`: a failure is `CookResult.error` (+ `error_node`; the UI shows both).
    pub fn cook(&mut self) -> CookResult {
        self.last = None;
        let fail = |error: String, error_node: Option<u32>| CookResult {
            error: Some(error),
            error_node,
            ..CookResult::default()
        };
        let Some(display) = self.graph.display() else { return fail("no display node".into(), None) };
        let graph = &mut self.graph;
        match catch_unwind(AssertUnwindSafe(|| graph.cook(display, 0.0))) {
            Ok(Ok(detail)) => {
                let out = flatten(&detail);
                self.last = Some(detail);
                out
            }
            Ok(Err(e)) => {
                let node = match &e {
                    GraphError::Node { node, .. }
                    | GraphError::MissingInput { node, .. }
                    | GraphError::NoSuchParam { node, .. }
                    | GraphError::ParamType { node, .. }
                    | GraphError::BadParamValue { node, .. }
                    | GraphError::Cycle(node) => Some(*node),
                    _ => None,
                };
                let named = node.and_then(|n| self.graph.name(n).map(|nm| (n, nm.to_string())));
                let text = match (&e, &named) {
                    (GraphError::Node { source, .. }, Some((_, nm))) => format!("{nm}: {source}"),
                    _ => e.to_string(),
                };
                fail(text, named.map(|(n, _)| n.index()))
            }
            Err(_) => fail("cook panicked (engine bug)".into(), None),
        }
    }

    /// Rows `offset..offset+limit` of the last cooked geometry for `class` (`point|vertex|prim|detail`).
    pub fn spreadsheet(&self, class: &str, offset: usize, limit: usize) -> Result<Sheet, String> {
        let class = sheet::parse_class(class)?;
        let d = self.last.as_ref().ok_or("nothing cooked (no display geometry)")?;
        Ok(sheet::sheet(d, class, offset, limit))
    }
}

/// Ascending, deduplicated ptnums -> the group SOP `pattern` syntax: `0-3 7 9-10`.
pub fn ptnum_pattern(ids: &[u32]) -> String {
    let mut v = ids.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
            j += 1;
        }
        out.push(if j > i { format!("{}-{}", v[i], v[j]) } else { v[i].to_string() });
        i = j + 1;
    }
    out.join(" ")
}

/// `Detail` -> the viewport payload. Polys/polylines become point-index lists; other prim kinds are counted, not drawn.
pub fn flatten(d: &Detail) -> CookResult {
    let positions: Vec<f32> = d.p().iter().flat_map(|p| p.iter().copied()).collect();
    let mut counts = Counts { points: d.point_count, ..Counts::default() };
    let (mut prims, mut closed, mut shapes) = (Vec::new(), Vec::new(), Vec::new());
    for pr in &d.prims {
        let shape = match pr.kind {
            PrimKind::Sphere { xform } => Some(("sphere", xform, 0.0)),
            PrimKind::Tube { xform, taper, .. } => Some(("tube", xform, taper)),
            _ => None,
        };
        if let Some((kind, xform, taper)) = shape {
            let at = d.vertex_point.get(pr.verts.start as usize).and_then(|&i| d.p().get(i as usize).copied());
            if let Some(center) = at {
                shapes.push(Shape { kind, center, xform, taper });
                counts.shapes += 1;
            } else {
                counts.unsupported += 1;
            }
            prims.push(Vec::new());
            closed.push(false);
            continue;
        }
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
    let (cd, cd_point) = colors(d, &prims);
    CookResult { positions, prims, closed, shapes, cd, cd_point, counts, error: None, error_node: None }
}
/// `Cd` (V3) for the viewport: per prim corner and per point, see `CookResult::cd`.
fn colors(d: &Detail, prims: &[Vec<u32>]) -> (Option<Vec<f32>>, Option<Vec<f32>>) {
    use hexe_geo::{AttrClass, AttrData};
    let get = |c: AttrClass| match d.attr(c, "Cd").map(|a| &a.data) {
        Some(AttrData::V3(v)) => Some(v),
        _ => None,
    };
    let (vtx, pt, pr) = (get(AttrClass::Vertex), get(AttrClass::Point), get(AttrClass::Prim));
    if vtx.is_none() && pt.is_none() && pr.is_none() {
        return (None, None);
    }
    const WHITE: [f32; 3] = [1.0; 3];
    let mut cd: Vec<f32> = Vec::with_capacity(prims.iter().map(Vec::len).sum::<usize>() * 3);
    for (pi, (prim, corners)) in d.prims.iter().zip(prims).enumerate() {
        for k in 0..corners.len() {
            let vid = prim.verts.start as usize + k;
            let c = vtx
                .and_then(|v| v.get(vid))
                .or_else(|| pt.and_then(|v| d.vertex_point.get(vid).and_then(|&p| v.get(p as usize))))
                .or_else(|| pr.and_then(|v| v.get(pi)))
                .unwrap_or(&WHITE);
            cd.extend_from_slice(c);
        }
    }
    let cd_point = pt.filter(|v| v.len() == d.point_count as usize).map(|v| v.iter().flatten().copied().collect());
    (Some(cd), cd_point)
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
    fn sphere_and_tube_prims_become_shapes() {
        let mut s = Session::new();
        s.graph_load(&example("polyextrude.json")).expect("loads");
        s.cook();
        let mut d = (**s.last.as_ref().expect("cooked")).clone();
        let x = [1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0];
        d.prims[0].kind = PrimKind::Sphere { xform: x };
        d.prims[1].kind = PrimKind::Tube { xform: x, taper: 0.5, closed: true };
        let c = flatten(&d);
        assert_eq!((c.counts.shapes, c.shapes.len()), (2, 2));
        assert_eq!((c.shapes[0].kind, c.shapes[1].kind, c.shapes[1].taper), ("sphere", "tube", 0.5));
        assert_eq!(c.shapes[0].xform, x);
        assert!(c.prims[0].is_empty() && c.prims[1].is_empty(), "shape prims carry no point list");
        assert_eq!(c.prims.len(), c.closed.len());
    }

    #[test]
    fn cd_is_absent_without_color_and_expanded_per_corner_with_it() {
        use hexe_geo::{AttrClass, AttrData, Attribute, TypeInfo};
        let mut d = Detail::default();
        d.add_points(4);
        d.add_poly(&[0, 1, 2], true);
        d.add_poly(&[1, 2, 3], true);
        let none = flatten(&d);
        assert!(none.cd.is_none() && none.cd_point.is_none());
        let attr = |class, v: Vec<[f32; 3]>| Attribute { name: "Cd".into(), class, data: AttrData::V3(v), type_info: TypeInfo::Color };
        // prim Cd: repeated over each prim's corners
        d.attrs.push(attr(AttrClass::Prim, vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]));
        let c = flatten(&d);
        assert_eq!(c.cd.as_deref(), Some(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0][..]));
        assert!(c.cd_point.is_none());
        // point Cd beats prim Cd and also fills cd_point
        d.attrs.push(attr(AttrClass::Point, (0..4).map(|i| [i as f32, 0.5, 0.0]).collect()));
        let c = flatten(&d);
        assert_eq!(&c.cd.expect("cd")[..9], &[0.0, 0.5, 0.0, 1.0, 0.5, 0.0, 2.0, 0.5, 0.0]);
        assert_eq!(c.cd_point.map(|p| p.len()), Some(12));
        // vertex Cd wins over both
        d.attrs.push(attr(AttrClass::Vertex, vec![[0.25; 3]; 6]));
        assert!(flatten(&d).cd.expect("cd").iter().all(|&x| x == 0.25));
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
        s.set_display(Some(pe)).expect("display");
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
    fn spreadsheet_follows_the_last_cook() {
        let mut s = Session::new();
        assert!(s.spreadsheet("point", 0, 10).is_err(), "nothing cooked yet");
        s.graph_load(&example("polyextrude.json")).expect("loads");
        let c = s.cook();
        let pts = s.spreadsheet("point", 0, 5).expect("points");
        assert_eq!((pts.total, pts.rows.len()), (c.counts.points as usize, 5));
        assert_eq!(&pts.columns[..4], ["ptnum", "P.x", "P.y", "P.z"]);
        assert_eq!(s.spreadsheet("prim", 0, 1000).expect("prims").total, c.counts.prims as usize);
        assert!(s.spreadsheet("edge", 0, 1).is_err());
    }

    #[test]
    fn cook_error_names_the_failing_node() {
        let mut s = Session::new();
        let k = s.node_add("knife", 0.0, 0.0).expect("knife");
        let c = s.cook();
        eprintln!("default knife, no input: {:?} / {:?}", c.error, c.error_node);
        // a knife whose plane normal is zero fails to cook; the error carries the node (UI: red border + status strip)
        s.node_set_param(k, "dir", &json!([0.0, 0.0, 0.0])).expect("dir");
        let c = s.cook();
        assert_eq!(c.error_node, Some(k));
        let msg = c.error.expect("error text");
        assert!(msg.starts_with("knife1: ") && msg.contains("non-zero"), "{msg}");
        assert!(c.positions.is_empty());
        assert!(s.spreadsheet("point", 0, 1).is_err(), "a failed cook leaves no stale sheet");
        s.node_set_param(k, "dir", &json!([0.0, 1.0, 0.0])).expect("dir");
        let c = s.cook();
        assert_eq!((c.error, c.error_node), (None, None));
    }

    fn names(s: &Session) -> Vec<String> {
        s.graph_get().nodes.into_iter().map(|n| n.name).collect()
    }

    #[test]
    fn undo_redo_walks_graph_edits() {
        let mut s = Session::new();
        assert_eq!(s.undo(), Ok(false));
        let b = s.node_add("box", 10.0, 20.0).expect("box");
        let pe = s.node_add("polyextrude", 200.0, 0.0).expect("pe");
        s.edge_connect(b, pe, 0).expect("wire");
        s.set_display(Some(pe)).expect("display");
        s.node_move(b, 55.0, 66.0).expect("move");
        assert_eq!(s.history(), (5, 0));
        // undo the move, then the display change, then the wire
        assert_eq!(s.undo(), Ok(true));
        assert_eq!(s.graph_get().nodes.iter().find(|n| n.name == "box1").map(|n| (n.x, n.y)), Some((10.0, 20.0)));
        assert_eq!(s.undo(), Ok(true));
        let g = s.graph_get();
        assert_eq!(g.display, g.nodes.iter().find(|n| n.name == "box1").map(|n| n.id), "display back on box1");
        assert_eq!(s.undo(), Ok(true));
        assert!(s.graph_get().edges.is_empty());
        assert_eq!(s.history(), (2, 3));
        // redo restores them in order
        for _ in 0..3 {
            assert_eq!(s.redo(), Ok(true));
        }
        assert_eq!(s.redo(), Ok(false));
        let g = s.graph_get();
        assert_eq!((g.edges.len(), g.nodes.iter().find(|n| n.name == "box1").map(|n| n.x)), (1, Some(55.0)));
        assert_eq!(s.cook().error, None);
        // a fresh edit clears redo
        s.undo().expect("undo");
        s.node_add("null", 0.0, 0.0).expect("null");
        assert_eq!(s.history().1, 0);
        // undo all the way back to the empty graph
        while s.undo().expect("undo") {}
        assert!(names(&s).is_empty());
        assert_eq!(s.history().0, 0);
    }

    #[test]
    fn undo_keeps_a_cleared_display_and_is_bounded() {
        let mut s = Session::new();
        let b = s.node_add("box", 0.0, 0.0).expect("box");
        s.set_display(None).expect("clear");
        assert_eq!(s.graph_get().display, None);
        assert_eq!(s.cook().error.as_deref(), Some("no display node"));
        s.node_move(b, 1.0, 1.0).expect("move");
        s.undo().expect("undo move");
        assert_eq!(s.graph_get().display, None, "cleared display survives the snapshot round trip");
        s.undo().expect("undo clear");
        assert!(s.graph_get().display.is_some());
        for i in 0..(MAX_UNDO + 30) {
            s.node_move(b, i as f32, 0.0).expect("move");
        }
        assert_eq!(s.history().0, MAX_UNDO);
    }

    #[test]
    fn param_scrub_is_one_undo_step() {
        let mut s = Session::new();
        let b = s.node_add("box", 0.0, 0.0).expect("box");
        let before = s.graph_get().nodes[0].params["size"].clone();
        for i in 1..=20 {
            s.node_set_param(b, "size", &json!([i as f64, 1.0, 1.0])).expect("scrub");
        }
        assert_eq!(s.history().0, 2, "add + ONE coalesced scrub");
        assert_eq!(s.graph_get().nodes[0].params["size"], json!([20.0, 1.0, 1.0]));
        s.undo().expect("undo");
        assert_eq!(s.graph_get().nodes[0].params["size"], before);
        // setting a value that changes nothing records nothing
        let n = s.history().0;
        s.node_set_param(s.id_of("box1").expect("id"), "size", &before).expect("same");
        assert_eq!(s.history().0, n);
    }

    #[test]
    fn rename_is_unique_and_keeps_pos_and_wires() {
        let mut s = Session::new();
        let b = s.node_add("box", 7.0, 8.0).expect("box");
        let pe = s.node_add("polyextrude", 0.0, 0.0).expect("pe");
        s.edge_connect(b, pe, 0).expect("wire");
        assert!(s.node_rename(pe, "box1").is_err(), "name taken");
        assert!(s.node_rename(pe, "").is_err());
        assert!(s.node_rename(pe, "a b").is_err());
        assert!(s.node_rename(pe, "a.b").is_err());
        s.node_rename(b, "ground").expect("rename");
        let g = s.graph_get();
        let n = g.nodes.iter().find(|n| n.name == "ground").expect("renamed");
        assert_eq!((n.x, n.y, g.edges.len()), (7.0, 8.0, 1));
        s.node_rename(b, "ground").expect("same name is a no-op");
        s.undo().expect("undo");
        assert!(names(&s).contains(&"box1".to_string()));
    }

    #[test]
    fn copy_paste_keeps_inner_wires_and_offsets() {
        let mut s = Session::new();
        let b = s.node_add("box", 10.0, 10.0).expect("box");
        let pe = s.node_add("polyextrude", 10.0, 100.0).expect("pe");
        let out = s.node_add("null", 10.0, 200.0).expect("null");
        s.edge_connect(b, pe, 0).expect("w1");
        s.edge_connect(pe, out, 0).expect("w2");
        s.node_set_param(pe, "distance", &json!(0.75)).expect("set");
        assert!(s.paste().is_err(), "empty clipboard");
        assert_eq!(s.copy(&[b, pe]), Ok(2));
        let new = s.paste().expect("paste");
        assert_eq!(new.len(), 2);
        let g = s.graph_get();
        assert_eq!(g.nodes.len(), 5);
        let nb = g.nodes.iter().find(|n| n.id == new[0]).expect("b2");
        let npe = g.nodes.iter().find(|n| n.id == new[1]).expect("pe2");
        assert_eq!((nb.name.as_str(), npe.name.as_str()), ("box2", "polyextrude2"));
        assert_eq!((nb.x, nb.y), (30.0, 30.0));
        assert_eq!(npe.params["distance"], json!(0.75));
        assert!(g.edges.contains(&EdgeView { from: nb.id, to: npe.id, input: 0 }), "inner wire kept");
        assert!(!g.edges.iter().any(|e| e.from == npe.id && e.to == out), "outer wire NOT copied");
        assert_eq!(g.edges.len(), 3);
        // second paste goes another 20 px; one undo removes the whole paste
        let again = s.paste().expect("paste 2");
        assert_eq!(s.graph_get().nodes.iter().find(|n| n.id == again[0]).map(|n| n.x), Some(50.0));
        s.undo().expect("undo");
        assert_eq!(s.graph_get().nodes.len(), 5);
        assert!(s.copy(&[]).is_err());
        assert!(s.copy(&[999]).is_err());
    }

    #[test]
    fn multi_remove_is_one_step_and_validated() {
        let mut s = Session::new();
        let a = s.node_add("box", 0.0, 0.0).expect("a");
        let b = s.node_add("null", 0.0, 0.0).expect("b");
        let c = s.node_add("null", 0.0, 0.0).expect("c");
        assert!(s.nodes_remove(&[a, 999]).is_err());
        assert_eq!(s.graph_get().nodes.len(), 3, "nothing removed on a bad id");
        s.nodes_remove(&[a, b]).expect("remove two");
        assert_eq!(names(&s), ["null2"]);
        assert_eq!(s.graph_get().display, Some(c).map(|_| s.graph_get().nodes[0].id));
        s.undo().expect("undo");
        assert_eq!(s.graph_get().nodes.len(), 3);
    }

    #[test]
    fn set_overrides_reapply_on_every_load() {
        let mut s = Session::new();
        s.set_overrides(vec!["polyextrude1.distance=0.9".into()]).expect("sets");
        assert!(s.set_overrides(vec!["nonsense".into()]).is_err());
        let dist = |s: &Session| {
            s.graph_get().nodes.iter().find(|n| n.ty == "polyextrude").map(|n| n.params["distance"].clone())
        };
        s.graph_load(&example("polyextrude.json")).expect("loads");
        // the example's node is not named polyextrude1: the override reports instead of silently vanishing
        assert_eq!(s.set_errors.len(), usize::from(s.id_of("polyextrude1").is_none()));
        let name = s.graph_get().nodes.iter().find(|n| n.ty == "polyextrude").map(|n| n.name.clone()).expect("pe");
        s.set_overrides(vec![format!("{name}.distance=0.9")]).expect("sets");
        s.graph_load(&example("polyextrude.json")).expect("loads");
        assert!(s.set_errors.is_empty(), "{:?}", s.set_errors);
        assert_eq!(dist(&s), Some(json!(0.9)));
        let id = s.id_of(&name).expect("id");
        s.node_set_param(id, "distance", &json!(0.1)).expect("user edit");
        s.graph_load(&example("polyextrude.json")).expect("reload");
        assert_eq!(dist(&s), Some(json!(0.9)), "override wins again after a reload");
        assert_eq!(s.history(), (0, 0), "loading resets history");
        assert!(s.set_arg(&format!("{name}.nope=1")).is_err());
        assert!(s.set_arg(&format!("{name}.distance=abc")).is_err());
        assert!(s.set_arg("ghost.distance=1").is_err());
    }

    #[test]
    fn set_arg_types_by_param_kind() {
        let mut s = Session::new();
        let b = s.node_add("box", 0.0, 0.0).expect("box");
        s.set_arg("box1.size=1,2.5,3").expect("v3");
        assert_eq!(s.graph_get().nodes[0].params["size"], json!([1.0, 2.5, 3.0]));
        assert!(s.set_arg("box1.size=1,2").is_err());
        let _ = b;
    }

    #[test]
    fn save_roundtrip_keeps_ui_pos_after_edits() {
        let mut s = Session::new();
        let b = s.node_add("box", 12.5, 34.0).expect("box");
        let pe = s.node_add("polyextrude", 300.0, 150.0).expect("pe");
        s.edge_connect(b, pe, 0).expect("wire");
        s.node_rename(pe, "ext").expect("rename");
        s.copy(&[b]).expect("copy");
        s.paste().expect("paste");
        s.undo().expect("undo");
        s.redo().expect("redo");
        let out = std::env::temp_dir().join(format!("hexe-tauri-pos-{}.json", std::process::id()));
        let path = out.to_string_lossy().into_owned();
        s.graph_save(&path).expect("save");
        let text = std::fs::read_to_string(&out).expect("read");
        let mut t = Session::new();
        t.graph_load(&path).expect("reload");
        let _ = std::fs::remove_file(&out);
        let doc: Value = serde_json::from_str(&text).expect("json");
        assert_eq!(doc["ui"]["pos"]["box1"], json!([12.5, 34.0]));
        assert_eq!(doc["ui"]["pos"]["ext"], json!([300.0, 150.0]));
        let pos = |s: &Session| s.graph_get().nodes.into_iter().map(|n| (n.name, n.x, n.y)).collect::<Vec<_>>();
        let (mut a, mut b2) = (pos(&s), pos(&t));
        a.sort_by(|x, y| x.0.cmp(&y.0));
        b2.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(a, b2, "every node keeps its ui.pos through save -> load");
    }

    #[test]
    fn hostile_input_is_err_not_panic() {
        let mut s = Session::new();
        assert!(s.graph_load("/definitely/not/here.json").is_err());
        assert_eq!(s.cook().error.as_deref(), Some("no display node"));
        assert!(s.graph_save("/definitely/not/here/out.json").is_err());
        assert!(s.node_move(99, f32::NAN, 0.0).is_err());
        assert!(s.set_display(Some(5)).is_err());
    }

    #[test]
    fn ptnum_pattern_compresses_runs() {
        assert_eq!(ptnum_pattern(&[]), "");
        assert_eq!(ptnum_pattern(&[5]), "5");
        assert_eq!(ptnum_pattern(&[9, 0, 1, 2, 3, 7, 10, 3]), "0-3 7 9-10");
    }

    #[test]
    fn scene_size_limit_is_64_mib() {
        assert_eq!(MAX_SCENE_BYTES, 64 * 1024 * 1024);
        assert!(check_scene_size("a.json", MAX_SCENE_BYTES).is_ok());
        let e = check_scene_size("a.json", MAX_SCENE_BYTES + 1).expect_err("over the limit");
        assert!(e.contains("a.json") && e.contains("64 MiB"), "{e}");
    }

    #[test]
    fn wrangle_params_flag_code_and_run_over() {
        let s = Session::new();
        let w = s.list_node_types().into_iter().find(|t| t.ty == "wrangle").expect("wrangle type");
        let snip = w.params.iter().find(|p| p.name == "snippet").expect("snippet");
        assert_eq!(snip.language.as_deref(), Some("vex"));
        let class = w.params.iter().find(|p| p.name == "class").expect("class");
        assert_eq!(class.choices, ["point", "prim", "vertex", "detail"]);
        assert!(class.language.is_none());
        let other = s.list_node_types().into_iter().flat_map(|t| t.params).filter(|p| p.language.is_some()).count();
        assert_eq!(other, 1, "only the wrangle snippet is code so far");
    }

    #[test]
    fn wrangle_vex_errors_carry_line_and_col() {
        let mut s = Session::new();
        let g = s.node_add("grid", 0.0, 0.0).expect("grid");
        let w = s.node_add("wrangle", 0.0, 90.0).expect("wrangle");
        s.edge_connect(g, w, 0).expect("wire");
        s.set_display(Some(w)).expect("display");
        s.node_set_param(w, "snippet", &json!("@P.y = 1;\n@P.x = ;")).expect("set");
        let c = s.cook();
        assert_eq!(c.error_node, Some(w));
        let msg = c.error.expect("parse error");
        assert!(msg.contains("2:") && msg.contains("wrangle"), "line 2 reported: {msg}");
        s.node_set_param(w, "snippet", &json!("@P.y = 1;")).expect("fixed");
        assert_eq!(s.cook().error, None);
    }

    #[test]
    fn group_from_selection_adds_a_point_group_sop() {
        let mut s = Session::new();
        let g = s.node_add("grid", 10.0, 20.0).expect("grid");
        assert!(s.group_from_selection().is_err(), "empty selection refused");
        assert_eq!(s.cook().error, None);
        s.selection_set(&[3, 1, 2, 3, 7]);
        assert_eq!(s.selection_get(), [1, 2, 3, 7]);
        let before = s.graph_get().nodes.len();
        let id = s.group_from_selection().expect("group");
        let gv = s.graph_get();
        assert_eq!(gv.nodes.len(), before + 1);
        assert_eq!(gv.display, Some(id), "the new group node is displayed");
        assert!(gv.edges.iter().any(|e| e.from == g && e.to == id && e.input == 0));
        let n = gv.nodes.iter().find(|n| n.id == id).expect("node");
        assert_eq!(n.ty, "group");
        assert_eq!(n.params["pattern"], json!("1-3 7"));
        assert_eq!(n.params["grouptype"], json!(0));
        assert_eq!(s.cook().error, None, "the group cooks");
        // one undo step removes it again
        assert!(s.undo().expect("undo"));
        assert_eq!(s.graph_get().nodes.len(), before);
        // a selection outside the displayed geometry is refused
        s.selection_set(&[1_000_000]);
        assert_eq!(s.cook().error, None);
        assert!(s.group_from_selection().expect_err("out of range").contains("outside"));
    }
}
