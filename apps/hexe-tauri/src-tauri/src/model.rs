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
    /// Pane tab (Houdini folder) the param sits on; `None` = untabbed.
    pub folder: Option<String>,
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
    /// Inputs `0..required` must be wired (`Node::min_inputs`); the rest are optional.
    pub required: usize,
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
    /// Inputs `0..required` must be wired (`Node::min_inputs`): solid wires, red arrow when unwired; the rest dashed.
    pub required: usize,
    /// Bypass flag: the node passes input 0 through unchanged.
    pub bypass: bool,
    /// Every spec'd param with its current value (defaults included): the param pane edits this.
    pub params: BTreeMap<String, Value>,
    /// Spare (user-added) parameters, interface order, each with its current value (Edit Parameter Interface).
    pub spare: Vec<hexe_sop::SpareParm>,
    /// Channel expressions: channel (`scale`, `tx`, a spare name) -> HScript (`ch("../null1/p")`).
    pub chans: BTreeMap<String, String>,
    /// Channels whose expression failed in the last cook -> message.
    pub chan_errors: BTreeMap<String, String>,
    /// Locked parameters (Lock Parameter): read-only in the pane, refused by every setter.
    pub locked: Vec<String>,
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
    /// Sticky notes of this network (editor px, top-left, y down), from the scene / .hip file.
    pub notes: Vec<hexe_sop::UiNote>,
    /// Network boxes (titled backdrops behind their nodes), editor px, top-left, y down.
    pub boxes: Vec<hexe_sop::UiBox>,
    /// Node name -> user colour (rgb 0..1).
    pub colors: BTreeMap<String, [f32; 3]>,
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
    /// Prim groups: name -> ascending prim indices (the viewport's Fit excludes groups matching its `fitExclude` globs).
    pub prim_groups: BTreeMap<String, Vec<u32>>,
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
    /// Edge groups: name -> flat point pairs `[a0, b0, a1, b1, ...]` (the viewport highlights the Group node's own one).
    pub edge_groups: BTreeMap<String, Vec<u32>>,
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
    /// Timeline `$F` (1-based): the display cook runs at `time = ($F - 1) / fps`.
    frame: i32,
    /// Timeline rate (frames per second). Default [`DEFAULT_TIMELINE_FPS`] = the gaia-motion clip rate, so `$F` is the clip frame.
    fps: f32,
    /// Name of the Geometry object whose SOP network is `graph` (/obj: one network per object, `object_enter`).
    object: String,
    /// The other Geometry objects, parked while another one is current; each keeps its own network, layout and undo.
    parked: BTreeMap<String, Parked>,
    /// Editor extras of the current network kept through load / save / undo: sticky notes, network boxes, node colours.
    ui: hexe_sop::UiExtra,
    /// Object-level data per object name (kind, container, position, flags); absent = a Geometry object at /obj.
    meta: BTreeMap<String, ObjMeta>,
    /// Channel-expression failures of the last cook: node -> channel -> message (the pane shows them on the field).
    chan_err: BTreeMap<String, BTreeMap<String, String>>,
}

/// Object kinds at /obj: a SOP network, a Null (its network: `control` + `point1`), an object-level container.
pub const OBJ_KINDS: [&str; 3] = ["geo", "null", "subnet"];

/// Object-level data of one object (editor state saved with multi-object scenes).
#[derive(Clone, Debug, PartialEq)]
struct ObjMeta {
    kind: String,
    /// Container object it lives in (`None` = directly under /obj).
    net: Option<String>,
    pos: Option<[f32; 2]>,
    display: bool,
    selectable: bool,
    /// Object-level parameters that differ from their default (`object_specs`: a Null's Transform / Render / Misc).
    params: BTreeMap<String, ParamValue>,
}
impl Default for ObjMeta {
    fn default() -> Self {
        ObjMeta { kind: "geo".into(), net: None, pos: None, display: true, selectable: true, params: BTreeMap::new() }
    }
}

/// Object-level parameters of an object kind, Houdini 20.5 names / defaults / menus / tabs (hython dump of the `null`
/// OBJ's parm templates). Only the Null has them so far; Geometry / subnet objects: none yet.
pub fn object_specs(kind: &str) -> Vec<ParamSpec> {
    if kind != "null" {
        return Vec::new();
    }
    let (t, r, m) = ("Transform", "Render", "Misc");
    let v3 = |n: &str, l: &str, d: [f32; 3], hi: f32, tab: &str| {
        ParamSpec::new(n, ParamValue::V3(d)).label(l).range(0.0, hi).folder(tab)
    };
    let f =
        |n: &str, l: &str, d: f32, tab: &str| ParamSpec::new(n, ParamValue::F(d)).label(l).range(0.0, 10.0).folder(tab);
    let b = |n: &str, l: &str, d: bool, tab: &str| ParamSpec::new(n, ParamValue::B(d)).label(l).folder(tab);
    let s = |n: &str, l: &str, tab: &str| ParamSpec::new(n, ParamValue::S(String::new())).label(l).folder(tab);
    let menu = |n: &str, l: &str, items: &[(&str, &str)], tab: &str| {
        ParamSpec::new(n, ParamValue::Menu(0)).label(l).menu(items).folder(tab)
    };
    vec![
        menu(
            "xOrd",
            "Transform Order",
            &[
                ("srt", "Scale Rot Trans"),
                ("str", "Scale Trans Rot"),
                ("rst", "Rot Scale Trans"),
                ("rts", "Rot Trans Scale"),
                ("tsr", "Trans Scale Rot"),
                ("trs", "Trans Rot Scale"),
            ],
            t,
        ),
        menu(
            "rOrd",
            "Rotate Order",
            &[
                ("xyz", "Rx Ry Rz"),
                ("xzy", "Rx Rz Ry"),
                ("yxz", "Ry Rx Rz"),
                ("yzx", "Ry Rz Rx"),
                ("zxy", "Rz Rx Ry"),
                ("zyx", "Rz Ry Rx"),
            ],
            t,
        ),
        v3("t", "Translate", [0.0; 3], 10.0, t),
        v3("r", "Rotate", [0.0; 3], 360.0, t),
        v3("s", "Scale", [1.0; 3], 10.0, t),
        v3("p", "Pivot Translate", [0.0; 3], 10.0, t),
        v3("pr", "Pivot Rotate", [0.0; 3], 10.0, t),
        f("scale", "Uniform Scale", 1.0, t),
        menu(
            "pre_xform",
            "Modify Pre-Transform",
            &[
                ("clean", "Clean Transform"),
                ("cleantrans", "Clean Translates"),
                ("cleanrot", "Clean Rotates"),
                ("cleanscales", "Clean Scales"),
                ("extract", "Extract Pre-transform"),
                ("reset", "Reset Pre-transform"),
            ],
            t,
        ),
        b("keeppos", "Keep Position When Parenting", false, t),
        b("childcomp", "Child Compensation", false, t),
        b("constraints_on", "Enable Constraints", false, t),
        s("constraints_path", "Constraints", t),
        b("tdisplay", "Display", false, r),
        ParamSpec::new("display", ParamValue::I(1)).label("Display").range(0.0, 1.0).folder(r),
        b("renderspace", "Output transform as render space (RIB/IFD)", false, r),
        b("use_dcolor", "Set Wireframe Color", true, m),
        ParamSpec::new("dcolor", ParamValue::V3([0.0, 0.75, 0.0])).label("Wireframe Color").range(0.0, 1.0).folder(m),
        b("picking", "Viewport Selecting Enabled", true, m),
        s("pickscript", "Select Script", m),
        b("caching", "Cache Object Transform", true, m),
        f("geoscale", "Display Uniform Scale", 1.0, m),
        v3("geosize", "Display Scale", [1.0; 3], 10.0, m),
        v3("geocenter", "Display Center", [0.0; 3], 10.0, m),
        v3("georotate", "Display Rotate", [0.0; 3], 10.0, m),
        menu("displayicon", "Display", &[("icon", "Icon"), ("axis", "Axis"), ("iconandaxis", "Icon and Axis")], m),
        menu(
            "controltype",
            "Control Type",
            &[
                ("null", "Null"),
                ("circles", "Circles"),
                ("box", "Box"),
                ("planes", "Planes"),
                ("nullandcircles", "Null and Circles"),
                ("nullandbox", "Null and Box"),
                ("nullandplanes", "Null and Planes"),
                ("custom", "Custom"),
            ],
            m,
        ),
        s("geocustom", "Display Custom", m),
        menu(
            "orientation",
            "Orientation",
            &[
                ("xyz", "All planes"),
                ("x", "YZ plane"),
                ("y", "ZX plane"),
                ("z", "XY plane"),
                ("xy", "YZ, ZX planes"),
                ("xz", "YZ, XY planes"),
                ("yz", "ZX, XY planes"),
            ],
            m,
        ),
        b("shadedmode", "Shaded Mode", false, m),
    ]
}

/// Object parameter `name` = JSON `v`, checked against `kind`'s specs (finite, right type, menu in range).
fn object_param(kind: &str, name: &str, v: &Value) -> Result<(ParamSpec, ParamValue), String> {
    let spec = object_specs(kind)
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("{kind} object has no parameter `{name}`"))?;
    let val = param_from_json(&spec.default, v).map_err(|e| format!("{name}: {e}"))?;
    if !val.is_acceptable() {
        return Err(format!("{name}: not a finite / acceptable value"));
    }
    if let ParamValue::Menu(i) = val
        && i as usize >= spec.menu.len()
    {
        return Err(format!("{name}: menu index {i} out of range (0..{})", spec.menu.len()));
    }
    Ok((spec, val))
}

/// `objects_info`: every object + the current one's name.
pub type ObjInfoList = (Vec<ObjInfo>, String);

/// One object as the /obj editor sees it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ObjInfo {
    pub name: String,
    /// `geo` (SOP network) · `null` (Null object: `control` + `point1` SOPs) · `subnet` (object-level container such as a
    /// subnet or digital asset: holds other objects; its own network has only sticky notes)
    pub kind: String,
    /// Container object it lives in (`None` = directly under /obj).
    pub net: Option<String>,
    /// Network position from the file (editor px, top-left); `None` = the editor places it.
    pub pos: Option<[f32; 2]>,
    pub display: bool,
    pub selectable: bool,
    /// Sticky notes of a container's network (`subnet` only; a geo's notes come with its graph).
    pub notes: Vec<hexe_sop::UiNote>,
}
/// Timeline rate unless `--fps`: gaia-motion renders its clips at 30 fps (`motion` SOP `fps` default).
pub const DEFAULT_TIMELINE_FPS: f32 = 30.0;
/// Largest `$F` accepted (`--frame`, `timeline_set`).
pub const MAX_FRAME: i32 = 1_000_000;

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

/// Built-in SOPs + the hexe-native node crates (`sop-motion`). The node palette (`list_node_types`) is this registry.
pub fn app_registry() -> Registry {
    let mut r = hexe_sop::registry();
    r.register(sop_motion::TYPE_NAME, sop_motion::spec, sop_motion::new);
    r
}

/// Copied nodes: type, original name, param values, position; edges are index pairs `(from, to, input)` inside the set.
#[derive(Clone, Debug)]
struct Clip {
    nodes: Vec<ClipNode>,
    edges: Vec<(usize, usize, usize)>,
}

/// A Geometry object's SOP network while another object is current (moved, never copied: switching is O(1)).
#[derive(Default)]
struct Parked {
    graph: Graph<Detail>,
    pos: BTreeMap<String, [f32; 2]>,
    last: Option<Arc<Detail>>,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    last_key: Option<String>,
    ui: hexe_sop::UiExtra,
}
/// The object a fresh / single-network scene lives in.
pub const FIRST_OBJECT: &str = "geo1";
/// Most Geometry objects at /obj (hundreds are normal; a hostile multi-object file cannot exhaust memory past this).
pub const MAX_OBJECTS: usize = 10_000;

impl Default for Session {
    fn default() -> Self {
        Session {
            graph: Graph::new(),
            reg: app_registry(),
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
            frame: 1,
            fps: DEFAULT_TIMELINE_FPS,
            object: FIRST_OBJECT.to_string(),
            parked: BTreeMap::new(),
            ui: hexe_sop::UiExtra::default(),
            meta: BTreeMap::new(),
            chan_err: BTreeMap::new(),
        }
    }
}

/// Object-level data of a multi-object file entry (`kind`, `net`, `pos`, `display`, `selectable`; all optional).
fn obj_meta(o: &Value) -> Result<ObjMeta, String> {
    let mut m = ObjMeta::default();
    if let Some(k) = o.get("kind").filter(|k| !k.is_null()) {
        let k = k.as_str().ok_or("kind: not a string")?;
        if !OBJ_KINDS.contains(&k) {
            return Err(format!("kind `{k}` (one of {})", OBJ_KINDS.join(", ")));
        }
        m.kind = k.to_string();
    }
    if let Some(n) = o.get("net").filter(|n| !n.is_null()) {
        let n = n.as_str().ok_or("net: not a string")?;
        valid_object_name(n)?;
        m.net = Some(n.to_string());
    }
    if let Some(p) = o.get("pos").filter(|p| !p.is_null()) {
        let xy: Vec<f32> =
            p.as_array().ok_or("pos: not [x, y]")?.iter().filter_map(Value::as_f64).map(|v| v as f32).collect();
        match xy[..] {
            [x, y] if x.is_finite() && y.is_finite() => m.pos = Some([x, y]),
            _ => return Err("pos: not two finite numbers".into()),
        }
    }
    m.display = o.get("display").and_then(Value::as_bool).unwrap_or(true);
    m.selectable = o.get("selectable").and_then(Value::as_bool).unwrap_or(true);
    if let Some(ps) = o.get("params").filter(|p| !p.is_null()) {
        let ps = ps.as_object().ok_or("params: not an object")?;
        if ps.len() > 256 {
            return Err("params: more than 256".into());
        }
        for (k, v) in ps {
            let (spec, val) = object_param(&m.kind, k, v)?;
            if val != spec.default {
                m.params.insert(k.clone(), val);
            }
        }
    }
    Ok(m)
}

/// Object names: like node names (letters, digits, `_` and `-`), at most 64 characters.
fn valid_object_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("invalid object name `{name}` (letters, digits, `_` and `-`, at most 64)"));
    }
    Ok(())
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
    // `node.param` (canonical) or the Houdini-path spelling `node/param`; node names never contain either
    let (node, param) = key.split_once('.').or_else(|| key.split_once('/')).ok_or_else(bad)?;
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
        folder: s.folder.clone(),
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
                    required: node.min_inputs().min(node.inputs().len()),
                    params: self.reg.specs(t)?.iter().map(param_info).collect(),
                })
            })
            .collect()
    }

    /// Fresh empty graph.
    pub fn graph_new(&mut self) {
        self.graph = Graph::new();
        self.ui = hexe_sop::UiExtra::default();
        self.pos.clear();
        self.last = None;
        self.parked.clear();
        self.meta.clear();
        self.object = FIRST_OBJECT.to_string();
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
        let json = if hexe_sop::is_hip_path(path) {
            // File -> Open on a Houdini .hip: first /obj geo container with SOPs (no-twin nodes load as `unsupported:<type>`).
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            let imp = hexe_sop::import_hip(&bytes).map_err(|e| format!("{path}: {e}"))?;
            if imp.geos.is_empty() {
                return Err(format!("{path}: no /obj geo network with SOP nodes"));
            }
            if imp.geos.len() + imp.empty.len() == 1 && imp.containers.is_empty() {
                serde_json::to_string(&imp.geos[0].scene).map_err(|e| e.to_string())?
            } else {
                // every geo becomes a Geometry object (empty ones too), every subnet / asset a container object holding
                // the objects inside it; names made unique (nested assets can repeat them)
                let mut seen = std::collections::BTreeSet::new();
                let mut uniq = |n: &str| {
                    let base: String =
                        n.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').take(56).collect();
                    let base = if base.is_empty() { "geo".to_string() } else { base };
                    let name = (0u32..)
                        .map(|i| if i == 0 { base.clone() } else { format!("{base}{i}") })
                        .find(|c| !seen.contains(c))
                        .unwrap_or_default();
                    seen.insert(name.clone());
                    name
                };
                // containers first, outermost first: an object's `net` (its container's hip path) maps to a unique name
                let mut boxes: Vec<&hexe_sop::ContainerImport> = imp.containers.iter().collect();
                boxes.sort_by_key(|c| c.object.parent.matches('/').count() + usize::from(!c.object.parent.is_empty()));
                let mut path_names = BTreeMap::new();
                let full = |o: &hexe_sop::ObjMeta| {
                    if o.parent.is_empty() { o.name.clone() } else { format!("{}/{}", o.parent, o.name) }
                };
                let entry = |name: String, kind: &str, o: &hexe_sop::ObjMeta, paths: &BTreeMap<String, String>| {
                    serde_json::json!({
                        "name": name, "kind": kind, "net": paths.get(&o.parent), "pos": o.pos,
                        "display": o.display, "selectable": o.selectable,
                    })
                };
                let mut objects: Vec<Value> = Vec::new();
                for c in boxes {
                    let name = uniq(&c.object.name);
                    let mut e = entry(name.clone(), "subnet", &c.object, &path_names);
                    e["scene"] = Value::Null;
                    e["notes"] = Value::Array(c.notes.clone());
                    objects.push(e);
                    path_names.insert(full(&c.object), name);
                }
                for g in &imp.geos {
                    let mut e = entry(uniq(&g.name), "geo", &g.object, &path_names);
                    e["scene"] = g.scene.clone();
                    objects.push(e);
                }
                for o in &imp.empty {
                    let mut e = entry(uniq(&o.name), "geo", o, &path_names);
                    e["scene"] = Value::Null;
                    objects.push(e);
                }
                // open inside the first Geometry object that has nodes
                let current = objects
                    .iter()
                    .find(|o| o.get("kind").and_then(Value::as_str) == Some("geo") && !o["scene"].is_null())
                    .and_then(|o| o.get("name"))
                    .cloned()
                    .unwrap_or(Value::Null);
                serde_json::json!({ "hexe_objects": 1, "current": current, "objects": objects }).to_string()
            }
        } else {
            std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?
        };
        if self.load_objects(&json, Path::new(path).parent())? {
            self.apply_sets();
            self.reset_history();
            return Ok(());
        }
        let l =
            hexe_sop::load_scene_with_base(&json, &self.reg, Path::new(path).parent()).map_err(|e| e.to_string())?;
        self.graph = l.graph;
        self.pos = l.pos;
        self.ui = l.ui;
        self.last = None;
        self.parked.clear();
        self.meta.clear();
        self.object = FIRST_OBJECT.to_string();
        self.auto_layout();
        self.apply_sets();
        self.reset_history();
        Ok(())
    }

    /// Write the scene to `path`: one object = the plain scene format (nodes, non-default params, edges, display,
    /// `ui.pos`); several = `{"hexe_objects": 1, "current": .., "objects": [{"name", "scene"}]}` (`scene` null = empty).
    pub fn graph_save(&self, path: &str) -> Result<(), String> {
        let text = if self.parked.is_empty() {
            hexe_sop::save_scene_ext(&self.graph, &self.pos, &self.ui).map_err(|e| e.to_string())?
        } else {
            let scene = |g: &Graph<Detail>,
                         pos: &BTreeMap<String, [f32; 2]>,
                         ui: &hexe_sop::UiExtra|
             -> Result<Value, String> {
                if g.node_ids().into_iter().all(|n| g.name(n).is_none()) {
                    return Ok(Value::Null);
                }
                let t = hexe_sop::save_scene_ext(g, pos, ui).map_err(|e| e.to_string())?;
                serde_json::from_str(&t).map_err(|e| e.to_string())
            };
            let mut objects = vec![(self.object.clone(), scene(&self.graph, &self.pos, &self.ui)?, &self.ui)];
            for (name, p) in &self.parked {
                objects.push((name.clone(), scene(&p.graph, &p.pos, &p.ui)?, &p.ui));
            }
            objects.sort_by(|a, b| a.0.cmp(&b.0));
            let objects: Vec<Value> = objects
                .into_iter()
                .map(|(name, scene, ui)| {
                    let mut o = serde_json::json!({ "name": name, "scene": scene });
                    if let Some(m) = self.meta.get(&name).filter(|m| **m != ObjMeta::default()) {
                        o["kind"] = Value::from(m.kind.clone());
                        o["net"] = m.net.clone().map_or(Value::Null, Value::from);
                        o["pos"] = serde_json::json!(m.pos);
                        o["display"] = Value::Bool(m.display);
                        o["selectable"] = Value::Bool(m.selectable);
                        if !m.params.is_empty() {
                            o["params"] =
                                Value::Object(m.params.iter().map(|(k, v)| (k.clone(), param_json(v))).collect());
                        }
                        if m.kind == "subnet" {
                            o["notes"] = serde_json::json!(ui.notes);
                        }
                    }
                    o
                })
                .collect();
            serde_json::to_string_pretty(
                &serde_json::json!({ "hexe_objects": 1, "current": self.object, "objects": objects }),
            )
            .map_err(|e| e.to_string())?
        };
        std::fs::write(path, text).map_err(|e| format!("{path}: {e}"))
    }

    /// A multi-object file (`graph_save`): every object loaded (relative paths against `base`), `current` entered.
    /// `Ok(false)` = not that format (a plain scene). Nothing changes on error.
    fn load_objects(&mut self, json: &str, base: Option<&Path>) -> Result<bool, String> {
        let Ok(v) = serde_json::from_str::<Value>(json) else { return Ok(false) };
        let Some(list) = v.get("hexe_objects").and(v.get("objects")).and_then(Value::as_array) else {
            return Ok(false);
        };
        if list.is_empty() || list.len() > MAX_OBJECTS {
            return Err(format!("objects: {} (1..={MAX_OBJECTS})", list.len()));
        }
        let mut parked = BTreeMap::new();
        let mut meta = BTreeMap::new();
        for o in list {
            let name = o.get("name").and_then(Value::as_str).ok_or("object without a name")?;
            valid_object_name(name)?;
            let m = obj_meta(o).map_err(|e| format!("{name}: {e}"))?;
            let mut p = Parked::default();
            if m.kind == "subnet" {
                // a container's own network holds only sticky notes
                let notes = o.get("notes").cloned().unwrap_or(Value::Array(Vec::new()));
                let ui: hexe_sop::UiExtra = serde_json::from_value(serde_json::json!({ "notes": notes }))
                    .map_err(|e| format!("{name}: notes: {e}"))?;
                p.ui = ui.sanitized(|_| true);
            } else if let Some(scene) = o.get("scene").filter(|s| !s.is_null()) {
                let l = hexe_sop::load_scene_with_base(&scene.to_string(), &self.reg, base)
                    .map_err(|e| format!("{name}: {e}"))?;
                p.graph = l.graph;
                p.pos = l.pos;
                p.ui = l.ui;
            }
            if parked.insert(name.to_string(), p).is_some() {
                return Err(format!("two objects named `{name}`"));
            }
            if m != ObjMeta::default() {
                meta.insert(name.to_string(), m);
            }
        }
        // a container must exist and be a container; anything else lands at /obj
        let boxes: std::collections::BTreeSet<String> =
            meta.iter().filter(|(_, m)| m.kind == "subnet").map(|(k, _)| k.clone()).collect();
        for (name, m) in &mut meta {
            if m.net.as_ref().is_some_and(|n| !boxes.contains(n) || n == name) {
                m.net = None;
            }
        }
        // container loops (a in b, b in a): cut where the walk revisits
        for name in meta.keys().cloned().collect::<Vec<_>>() {
            let mut seen = std::collections::BTreeSet::from([name.clone()]);
            let mut at = name.clone();
            while let Some(up) = meta.get(&at).and_then(|m| m.net.clone()) {
                if !seen.insert(up.clone()) {
                    if let Some(m) = meta.get_mut(&at) {
                        m.net = None;
                    }
                    break;
                }
                at = up;
            }
        }
        let current = v.get("current").and_then(Value::as_str).filter(|c| parked.contains_key(*c));
        let current = current.map_or_else(|| parked.keys().next().cloned().unwrap_or_default(), str::to_string);
        let Some(p) = parked.remove(&current) else { return Err("no objects".into()) };
        self.parked = parked;
        self.meta = meta;
        self.object = current;
        self.swap_in(p);
        self.auto_layout();
        // parked objects get their layout when first entered (`object_enter` runs `auto_layout`)
        Ok(true)
    }

    // ---- /obj: Geometry objects, one SOP network each. Every SOP command works on the current object's network.

    /// The current object's name (the network every SOP command edits).
    pub fn object_name(&self) -> &str {
        &self.object
    }

    /// Every Geometry object's name, sorted.
    pub fn objects(&self) -> Vec<String> {
        let mut v: Vec<String> = std::iter::once(self.object.clone()).chain(self.parked.keys().cloned()).collect();
        v.sort();
        v
    }

    fn swap_out(&mut self) -> Parked {
        Parked {
            graph: std::mem::take(&mut self.graph),
            pos: std::mem::take(&mut self.pos),
            last: self.last.take(),
            undo: std::mem::take(&mut self.undo),
            redo: std::mem::take(&mut self.redo),
            last_key: self.last_key.take(),
            ui: std::mem::take(&mut self.ui),
        }
    }

    fn swap_in(&mut self, p: Parked) {
        self.graph = p.graph;
        self.pos = p.pos;
        self.last = p.last;
        self.undo = p.undo;
        self.redo = p.redo;
        self.last_key = p.last_key;
        self.ui = p.ui;
    }

    /// Make `name` the current object (double-click it at /obj): its network is what the SOP commands edit now.
    pub fn object_enter(&mut self, name: &str) -> Result<(), String> {
        if name == self.object {
            return Ok(());
        }
        let p = self.parked.remove(name).ok_or_else(|| format!("no object `{name}`"))?;
        let cur = self.swap_out();
        let old = std::mem::replace(&mut self.object, name.to_string());
        self.parked.insert(old, cur);
        self.swap_in(p);
        self.auto_layout();
        Ok(())
    }

    /// A name no other object has: `name` if free, else the next free number on its base (geo1 taken: `geo1` -> geo2).
    fn free_object_name(&self, name: &str, except: Option<&str>) -> String {
        let taken = |c: &str| Some(c) != except && (c == self.object || self.parked.contains_key(c));
        if !taken(name) {
            return name.to_string();
        }
        let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
        let base = if base.is_empty() { name } else { base };
        (1u32..).map(|i| format!("{base}{i}")).find(|c| !taken(c)).unwrap_or_default()
    }

    /// Every object with its kind, container, file position and flags (sorted by name).
    pub fn objects_info(&self) -> Vec<ObjInfo> {
        self.objects()
            .into_iter()
            .map(|name| {
                let m = self.meta.get(&name).cloned().unwrap_or_default();
                let ui = if name == self.object { Some(&self.ui) } else { self.parked.get(&name).map(|p| &p.ui) };
                let notes =
                    if m.kind == "subnet" { ui.map(|u| u.notes.clone()).unwrap_or_default() } else { Vec::new() };
                ObjInfo {
                    name,
                    kind: m.kind,
                    net: m.net,
                    pos: m.pos,
                    display: m.display,
                    selectable: m.selectable,
                    notes,
                }
            })
            .collect()
    }

    /// Object-level parameters of object `name`: specs (with tabs) + every value (defaults filled in).
    pub fn object_params(&self, name: &str) -> Result<(Vec<ParamInfo>, BTreeMap<String, Value>), String> {
        if !self.objects().iter().any(|o| o == name) {
            return Err(format!("no object `{name}`"));
        }
        let m = self.meta.get(name).cloned().unwrap_or_default();
        let specs = object_specs(&m.kind);
        let values =
            specs.iter().map(|s| (s.name.clone(), param_json(m.params.get(&s.name).unwrap_or(&s.default)))).collect();
        Ok((specs.iter().map(param_info).collect(), values))
    }

    /// Set object `name`'s parameter `parm` (a Null's Translate, ...); a value equal to the default is dropped.
    pub fn object_set_param(&mut self, name: &str, parm: &str, value: &Value) -> Result<(), String> {
        if !self.objects().iter().any(|o| o == name) {
            return Err(format!("no object `{name}`"));
        }
        let kind = self.meta.get(name).map_or_else(|| "geo".to_string(), |m| m.kind.clone());
        let (spec, val) = object_param(&kind, parm, value)?;
        let m = self.meta.entry(name.to_string()).or_default();
        if val == spec.default {
            m.params.remove(parm);
        } else {
            m.params.insert(parm.to_string(), val);
        }
        Ok(())
    }

    /// A Null object's network (Houdini): `control` (the viewport indicator, display flag) and `point1`, an Add SOP
    /// holding one point at the origin.
    fn null_network(&self) -> Result<Parked, String> {
        let mut p = Parked::default();
        let e = |e: GraphError| e.to_string();
        for (ty, name, at) in [("control", "control", [0.0, 0.0]), ("add", "point1", [LAYOUT_DX, 0.0])] {
            let node = self.reg.create(ty).ok_or_else(|| format!("unknown node type `{ty}`"))?;
            let id = p.graph.add_boxed(node, ParamSet::new()).map_err(e)?;
            p.graph.rename(id, name).map_err(e)?;
            p.pos.insert(name.to_string(), at);
            if ty == "control" {
                p.graph.set_display(Some(id)).map_err(e)?;
            } else {
                p.graph.set_param(id, "points", ParamValue::S("0,0,0".into())).map_err(e)?;
            }
        }
        hexe_sop::fill_unwired(&mut p.graph).map_err(|e| e.to_string())?;
        Ok(p)
    }

    /// New object (TAB menu at /obj): `name`, numbered up when taken; `kind` = `geo` (empty SOP network), `null`
    /// (`control` + `point1` inside) or `subnet`; `net` = the container it is created in. Returns its name.
    pub fn object_add_kind(&mut self, name: &str, kind: &str, net: Option<&str>) -> Result<String, String> {
        valid_object_name(name)?;
        if !OBJ_KINDS.contains(&kind) {
            return Err(format!("object kind `{kind}` (one of {})", OBJ_KINDS.join(", ")));
        }
        if let Some(n) = net.filter(|n| self.meta.get(*n).is_none_or(|m| m.kind != "subnet")) {
            return Err(format!("no container object `{n}`"));
        }
        if self.parked.len() + 1 >= MAX_OBJECTS {
            return Err(format!("at most {MAX_OBJECTS} objects"));
        }
        let p = if kind == "null" { self.null_network()? } else { Parked::default() };
        let name = self.free_object_name(name, None);
        self.parked.insert(name.clone(), p);
        let m = ObjMeta { kind: kind.to_string(), net: net.map(str::to_string), ..ObjMeta::default() };
        if m != ObjMeta::default() {
            self.meta.insert(name.clone(), m);
        }
        Ok(name)
    }

    /// New empty Geometry object at /obj (TAB menu > Geometry): `name`, numbered up when taken. Returns its name.
    pub fn object_add(&mut self, name: &str) -> Result<String, String> {
        self.object_add_kind(name, "geo", None)
    }

    /// Delete an object and its network; a container takes everything inside it along. Deleting the current one
    /// enters another; the last one is emptied instead (there is always a current network).
    pub fn object_remove(&mut self, name: &str) -> Result<(), String> {
        if name != self.object && !self.parked.contains_key(name) {
            return Err(format!("no object `{name}`"));
        }
        // the container and everything nested in it (breadth first: no recursion depth on deep nesting)
        let mut doomed = vec![name.to_string()];
        let mut i = 0;
        while let Some(c) = doomed.get(i).cloned() {
            doomed
                .extend(self.meta.iter().filter(|(_, m)| m.net.as_deref() == Some(c.as_str())).map(|(k, _)| k.clone()));
            i += 1;
        }
        for k in doomed.iter().rev() {
            self.meta.remove(k);
            self.object_remove_one(k)?;
        }
        Ok(())
    }

    fn object_remove_one(&mut self, name: &str) -> Result<(), String> {
        if name != self.object {
            return self.parked.remove(name).map(|_| ()).ok_or_else(|| format!("no object `{name}`"));
        }
        // the next current network: a SOP network (geo / null) before a container
        let sops = self.parked.keys().find(|k| self.meta.get(*k).is_none_or(|m| m.kind != "subnet"));
        match sops.or_else(|| self.parked.keys().next()).cloned() {
            Some(other) => {
                self.object_enter(&other)?;
                self.parked.remove(name);
            }
            None => {
                self.graph = Graph::new();
                self.pos.clear();
                self.ui = hexe_sop::UiExtra::default();
                self.last = None;
                self.reset_history();
            }
        }
        Ok(())
    }

    /// Rename an object (click its name at /obj); a taken name is numbered up. Returns the final name.
    pub fn object_rename(&mut self, old: &str, new: &str) -> Result<String, String> {
        valid_object_name(new)?;
        if old != self.object && !self.parked.contains_key(old) {
            return Err(format!("no object `{old}`"));
        }
        let new = self.free_object_name(new, Some(old));
        if new == old {
            return Ok(new);
        }
        if old == self.object {
            self.object = new.clone();
        } else if let Some(p) = self.parked.remove(old) {
            self.parked.insert(new.clone(), p);
        }
        if let Some(m) = self.meta.remove(old) {
            self.meta.insert(new.clone(), m);
        }
        for m in self.meta.values_mut().filter(|m| m.net.as_deref() == Some(old)) {
            m.net = Some(new.clone());
        }
        Ok(new)
    }

    /// Cook another object's display node (the viewport shows every visible object); the current object stays current.
    pub fn cook_object(&mut self, name: &str) -> Result<CookResult, String> {
        if name == self.object {
            return Ok(self.cook());
        }
        let back = self.object.clone();
        self.object_enter(name)?;
        let c = self.cook();
        self.object_enter(&back)?;
        Ok(c)
    }

    /// Give every named node without a stored position a level (y) = depth, slot (x) = order within the level.
    fn auto_layout(&mut self) {
        let ids: Vec<NodeId> = self.named();
        let mut depth: BTreeMap<u32, usize> = BTreeMap::new();
        // iterate to a fixpoint (depth = 1 + max input depth); bounded by node count, so a wired loop cannot spin it.
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
            let kind = self.reg.create(self.graph.node_type(id).unwrap_or_default());
            // a type outside the registry (`unsupported:<houdini type>` from a .hip) keeps the ports it was loaded with; like
            // most Houdini SOPs its first input is taken as the required one
            let (inputs, required) = match &kind {
                Some(n) => {
                    (n.inputs().iter().map(|i| i.name.to_string()).collect(), n.min_inputs().min(n.inputs().len()))
                }
                None => {
                    let k = self.graph.input_sources(id).len();
                    ((0..k).map(|i| format!("input{i}")).collect::<Vec<_>>(), k.min(1))
                }
            };
            let spare = self.ui.spare.get(&name).cloned().unwrap_or_default();
            let chans = self.ui.chan.get(&name).cloned().unwrap_or_default();
            let chan_errors = self.chan_err.get(&name).cloned().unwrap_or_default();
            let locked = self.ui.lock.get(&name).map(|l| l.iter().cloned().collect()).unwrap_or_default();
            nodes.push(NodeView {
                id: id.index(),
                ty: self.graph.node_type(id).unwrap_or_default().to_string(),
                name,
                x,
                y,
                inputs,
                required,
                bypass: self.graph.bypass(id),
                spare,
                chans,
                chan_errors,
                locked,
                params,
            });
            for (port, src) in self.graph.input_sources(id).into_iter().enumerate() {
                if let Some(s) = src.filter(|s| self.graph.name(*s).is_some()) {
                    edges.push(EdgeView { from: s.index(), to: id.index(), input: port });
                }
            }
        }
        GraphView {
            nodes,
            edges,
            display: self.graph.display().map(NodeId::index),
            notes: self.ui.notes.clone(),
            boxes: self.ui.boxes.clone(),
            colors: self.ui.color.clone(),
        }
    }

    fn node_add_raw(&mut self, ty: &str, x: f32, y: f32) -> Result<u32, String> {
        if !x.is_finite() || !y.is_finite() {
            return Err("non-finite position".into());
        }
        let node = self.reg.create(ty).ok_or_else(|| format!("unknown node type `{ty}`"))?;
        let id = self.graph.add_boxed(node, ParamSet::new()).map_err(|e| e.to_string())?;
        let dot = ty == hexe_sop::DOT_TYPE;
        let name = self.graph.unique_name(if dot { hexe_sop::DOT_NAME_BASE } else { ty });
        self.graph.rename(id, &name).map_err(|e| e.to_string())?;
        self.pos.insert(name, [x, y]);
        if self.graph.display().is_none() && !dot {
            self.graph.set_display(Some(id)).map_err(|e| e.to_string())?;
        }
        self.fill()?;
        Ok(id.index())
    }

    fn fill(&mut self) -> Result<(), String> {
        hexe_sop::fill_unwired(&mut self.graph).map_err(|e| e.to_string())
    }

    fn is_dot(&self, n: NodeId) -> bool {
        self.graph.node_type(n) == Some(hexe_sop::DOT_TYPE)
    }

    /// Every (node, input) that `n` feeds.
    fn consumers(&self, n: NodeId) -> Vec<(NodeId, usize)> {
        self.named()
            .into_iter()
            .flat_map(|k| {
                self.graph
                    .input_sources(k)
                    .into_iter()
                    .enumerate()
                    .filter(move |(_, s)| *s == Some(n))
                    .map(move |(i, _)| (k, i))
            })
            .collect()
    }

    fn node_remove_raw(&mut self, id: u32) -> Result<(), String> {
        let n = self.find(id)?;
        let name = self.graph.name(n).map(str::to_string);
        let was_display = self.graph.display() == Some(n);
        // a dot is a waypoint: its source takes over every input it fed (Houdini 20.5 `destroy()` on a network dot)
        let through = if self.is_dot(n) {
            let src = self.graph.input_sources(n).first().copied().flatten().filter(|s| self.graph.name(*s).is_some());
            src.map(|s| (s, self.consumers(n)))
        } else {
            None
        };
        self.graph.remove_node(n).map_err(|e| e.to_string())?;
        if let Some(name) = name {
            self.pos.remove(&name);
        }
        if let Some((src, outs)) = through {
            for (to, port) in outs {
                self.graph.connect(src, 0, to, port).map_err(|e| e.to_string())?;
            }
        }
        if was_display {
            let next = self.named().into_iter().find(|&k| !self.is_dot(k));
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
        self.refuse_locked(n, name)?;
        self.graph.set_param(n, name, v).map_err(|e| e.to_string())?;
        // a typed value replaces the parameter's channel expressions (Houdini: the field is no longer driven)
        self.drop_channels(n, name);
        Ok(())
    }

    /// `Err` if node `n`'s parameter `parm` is locked.
    fn refuse_locked(&self, n: NodeId, parm: &str) -> Result<(), String> {
        let name = self.graph.name(n).unwrap_or_default();
        if self.ui.lock.get(name).is_some_and(|l| l.contains(parm)) {
            return Err(format!("parameter `{parm}` is locked (unlock it first)"));
        }
        Ok(())
    }

    /// Lock / unlock node `id`'s parameter `parm` (built-in or spare). Undoable.
    pub fn node_set_lock(&mut self, id: u32, parm: &str, on: bool) -> Result<(), String> {
        let n = self.find(id)?;
        let name = self.graph.name(n).unwrap_or_default().to_string();
        let own = self.graph.param_specs(n).iter().any(|s| s.name == parm);
        let spare = self.ui.spare.get(&name).is_some_and(|ps| ps.iter().any(|p| p.name == parm));
        if !own && !spare {
            return Err(format!("{name} has no parameter `{parm}`"));
        }
        self.mutate(None, |s| {
            if on {
                s.ui.lock.entry(name.clone()).or_default().insert(parm.to_string());
            } else if let Some(l) = s.ui.lock.get_mut(&name) {
                l.remove(parm);
                if l.is_empty() {
                    s.ui.lock.remove(&name);
                }
            }
            Ok(())
        })
    }

    /// Remove the channel expressions of node `n`'s parameter `parm` (its scalar channel or every component).
    fn drop_channels(&mut self, n: NodeId, parm: &str) {
        let name = self.graph.name(n).unwrap_or_default().to_string();
        let spare = self.ui.spare.get(&name).cloned().unwrap_or_default();
        let chans = hexe_sop::chans::channels_of(&self.graph, &spare, n, parm);
        if let Some(c) = self.ui.chan.get_mut(&name) {
            for ch in chans {
                c.remove(&ch);
            }
            if c.is_empty() {
                self.ui.chan.remove(&name);
            }
        }
    }

    /// Set (`Some`) or delete (`None`) the channel expression of node `id`'s channel `chan` (a parameter, a component such
    /// as `tx`, or a spare parameter). A syntax error is refused; references are resolved at cook time. Undoable.
    pub fn node_set_channel(&mut self, id: u32, chan: &str, expr: Option<&str>) -> Result<(), String> {
        let n = self.find(id)?;
        let name = self.graph.name(n).unwrap_or_default().to_string();
        let spare = self.ui.spare.get(&name).cloned().unwrap_or_default();
        let Some(sl) = hexe_sop::chans::slot(&self.graph, &spare, n, chan) else {
            return Err(format!("{name} has no channel `{chan}`"));
        };
        self.refuse_locked(n, hexe_sop::chans::slot_parm(&sl, &spare))?;
        if let Some(e) = expr {
            if e.trim().is_empty() || e.len() > hexe_graph::hscript::MAX_EXPR {
                return Err("expression is empty or too long".into());
            }
            struct Probe;
            impl hexe_graph::hscript::Scope for Probe {
                fn var(&self, _: &str) -> Option<hexe_graph::hscript::Value> {
                    Some(hexe_graph::hscript::Value::Num(0.0))
                }
                fn channel(
                    &self,
                    _: &str,
                    _: bool,
                ) -> Result<hexe_graph::hscript::Value, hexe_graph::hscript::ExprError> {
                    Ok(hexe_graph::hscript::Value::Num(0.0))
                }
            }
            if let Err(hexe_graph::hscript::ExprError::Syntax(m)) = hexe_graph::hscript::eval(e, &Probe) {
                return Err(format!("expression: {m}"));
            }
        }
        self.mutate(None, |s| {
            match expr {
                Some(e) => {
                    s.ui.chan.entry(name.clone()).or_default().insert(chan.to_string(), e.to_string());
                }
                None => {
                    if let Some(c) = s.ui.chan.get_mut(&name) {
                        c.remove(chan);
                        if c.is_empty() {
                            s.ui.chan.remove(&name);
                        }
                    }
                }
            }
            Ok(())
        })
    }

    fn edge_connect_raw(&mut self, from: u32, to: u32, input: usize) -> Result<(), String> {
        let (s, d) = (self.find(from)?, self.find(to)?);
        if s == d {
            return Err("a node cannot be wired into itself".into());
        }
        // editor wiring may close a loop (Houdini parity): the cook flags it (`GraphError::Cycle`), see `Graph::connect_unchecked`
        self.graph.connect_unchecked(s, 0, d, input).map_err(|e| e.to_string())
    }

    fn edge_disconnect_raw(&mut self, to: u32, input: usize) -> Result<(), String> {
        let d = self.find(to)?;
        self.graph.disconnect(d, input).map_err(|e| e.to_string())?;
        self.fill()
    }

    fn set_display_raw(&mut self, id: Option<u32>) -> Result<(), String> {
        let n = id.map(|i| self.find(i)).transpose()?;
        if n.is_some_and(|n| self.is_dot(n)) {
            return Err("a network dot cannot carry the display flag".into());
        }
        self.graph.set_display(n).map_err(|e| e.to_string())
    }

    /// Add a node of `ty` at editor position (x, y); the first node becomes the display node.
    pub fn node_add(&mut self, ty: &str, x: f32, y: f32) -> Result<u32, String> {
        self.mutate(None, |s| s.node_add_raw(ty, x, y))
    }

    /// Add a network dot (`__dotN`, type `dot`) centred at editor (x, y), wired `from` -> dot and dot -> `to`'s input when
    /// given (a dot dropped on a wire, or at the end of a wire being drawn). ONE undo step; on any failure nothing changes.
    pub fn dot_add(&mut self, x: f32, y: f32, from: Option<u32>, to: Option<(u32, usize)>) -> Result<u32, String> {
        self.node_add_wired(hexe_sop::DOT_TYPE, x, y, from, to)
    }

    /// Add a `ty` node at editor (x, y) spliced into a wire: `from` -> its input 0 (skipped when `ty` takes no input:
    /// a generator such as Box only feeds `to`) and its output -> `to`'s input. TAB menu with a wire selected, dots.
    /// ONE undo step; on any failure nothing changes.
    pub fn node_add_wired(
        &mut self,
        ty: &str,
        x: f32,
        y: f32,
        from: Option<u32>,
        to: Option<(u32, usize)>,
    ) -> Result<u32, String> {
        if !x.is_finite() || !y.is_finite() {
            return Err("non-finite position".into());
        }
        if let Some(f) = from {
            self.find(f)?;
        }
        if let Some((t, input)) = to {
            let n = self.find(t)?;
            let ports = self.reg.create(self.graph.node_type(n).unwrap_or_default()).map_or(0, |k| k.inputs().len());
            if input >= ports {
                return Err(format!("`{}` has no input {input}", self.graph.name(n).unwrap_or_default()));
            }
        }
        let takes_input = self.reg.create(ty).is_some_and(|k| !k.inputs().is_empty());
        let before = self.snapshot();
        let r = self.mutate(None, |s| {
            let d = s.node_add_raw(ty, x, y)?;
            if let Some(f) = from.filter(|_| takes_input) {
                s.edge_connect_raw(f, d, 0)?;
            }
            if let Some((t, input)) = to {
                s.edge_connect_raw(d, t, input)?;
            }
            Ok(d)
        });
        // a refused wire after the node was added: put the graph back as it was
        if r.is_err() && self.snapshot() != before {
            self.restore(&before)?;
        }
        r
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

    /// Bypass flag (one undo step): a bypassed node passes its input 0 through unchanged.
    pub fn set_bypass(&mut self, id: u32, on: bool) -> Result<(), String> {
        self.mutate(None, |s| {
            let n = s.find(id)?;
            if s.is_dot(n) {
                return Err("a network dot cannot be bypassed".into());
            }
            s.graph.set_bypass(n, on).map_err(|e| e.to_string())
        })
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
            // a name another node has gets the next free number (Houdini): box + box1 taken, `box` -> box2, `box1` -> box2
            let name = match s.graph.node_by_name(name) {
                Some(other) if other != n => {
                    let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
                    let base = if base.is_empty() { name } else { base };
                    // the node's own current name counts as free (box2 renamed `box` stays box2)
                    (1u32..)
                        .map(|i| format!("{base}{i}"))
                        .find(|c| s.graph.node_by_name(c).is_none_or(|o| o == n))
                        .unwrap_or_default()
                }
                _ => name.to_string(),
            };
            if name == old {
                return Ok(());
            }
            s.graph.rename(n, &name).map_err(|e| e.to_string())?;
            if let Some(sp) = s.ui.spare.remove(&old) {
                s.ui.spare.insert(name.clone(), sp);
            }
            // its channels move with it, and references to it follow the new name (Houdini updates them)
            if let Some(c) = s.ui.chan.remove(&old) {
                s.ui.chan.insert(name.clone(), c);
            }
            if let Some(l) = s.ui.lock.remove(&old) {
                s.ui.lock.insert(name.clone(), l);
            }
            let (from, to) = (format!("../{old}/"), format!("../{name}/"));
            for e in s.ui.chan.values_mut().flat_map(|c| c.values_mut()) {
                if e.contains(&from) {
                    *e = e.replace(&from, &to);
                }
            }
            if let Some(p) = s.pos.remove(&old) {
                s.pos.insert(name, p);
            }
            Ok(())
        })
    }

    // ---- spare parameters (Houdini Edit Parameter Interface) ----

    /// Replace node `id`'s spare-parameter interface with `parms` (the dialog's Accept / Apply). Names must be valid,
    /// unique and not one of the node's own parameters; a param keeping its name, kind and size keeps its value. Undoable.
    pub fn node_spare_set(&mut self, id: u32, parms: Vec<hexe_sop::SpareParm>) -> Result<(), String> {
        if parms.len() > hexe_sop::MAX_SPARE {
            return Err(format!("{} spare parameters (at most {})", parms.len(), hexe_sop::MAX_SPARE));
        }
        let n = self.find(id)?;
        let name = self.graph.name(n).unwrap_or_default().to_string();
        let own: std::collections::BTreeSet<String> =
            self.graph.param_specs(n).iter().map(|s| s.name.clone()).collect();
        let old = self.ui.spare.get(&name).cloned().unwrap_or_default();
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::with_capacity(parms.len());
        for p in parms {
            if !hexe_sop::valid_spare_name(&p.name) {
                return Err(format!("invalid parameter name `{}` (a letter or _, then letters, digits, _)", p.name));
            }
            if own.contains(&p.name) {
                return Err(format!("`{}` is already a parameter of this node", p.name));
            }
            if !seen.insert(p.name.clone()) {
                return Err(format!("two parameters named `{}`", p.name));
            }
            if !hexe_sop::SPARE_KINDS.contains(&p.kind.as_str()) {
                return Err(format!("{}: unknown kind `{}`", p.name, p.kind));
            }
            let mut p = p.sanitized().ok_or_else(|| "bad spare parameter".to_string())?;
            if let Some(o) = old.iter().find(|o| o.name == p.name && o.kind == p.kind && o.size == p.size) {
                p.value = o.value.clone();
            }
            out.push(p);
        }
        // a folder reference must name a spare folder
        let folders: std::collections::BTreeSet<String> =
            out.iter().filter(|p| p.kind == "folder").map(|p| p.name.clone()).collect();
        for p in &mut out {
            if p.folder.as_ref().is_some_and(|f| !folders.contains(f) || *f == p.name) {
                p.folder = None;
            }
        }
        self.mutate(None, |s| {
            if out.is_empty() {
                s.ui.spare.remove(&name);
            } else {
                s.ui.spare.insert(name, out);
            }
            Ok(())
        })
    }

    /// Set the value of node `id`'s spare parameter `parm` (checked against its kind / size / menu). Undoable; repeated
    /// sets of the same parameter (a scrub) merge into one undo step.
    pub fn node_spare_value(&mut self, id: u32, parm: &str, value: &Value) -> Result<(), String> {
        let n = self.find(id)?;
        let name = self.graph.name(n).unwrap_or_default().to_string();
        let p = self
            .ui
            .spare
            .get(&name)
            .and_then(|ps| ps.iter().find(|p| p.name == parm))
            .ok_or_else(|| format!("no spare parameter `{parm}` on {name}"))?;
        self.refuse_locked(n, parm)?;
        if matches!(p.kind.as_str(), "folder" | "separator" | "label") {
            return Err(format!("{parm}: a {} holds no value", p.kind));
        }
        let v =
            p.coerce(value).ok_or_else(|| format!("{parm}: {value} does not fit a {} of size {}", p.kind, p.size))?;
        self.mutate(Some(format!("spare:{id}:{parm}")), |s| {
            if let Some(q) = s.ui.spare.get_mut(&name).and_then(|ps| ps.iter_mut().find(|q| q.name == parm)) {
                q.value = if v == q.default { Value::Null } else { v };
            }
            s.drop_channels(n, parm);
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
        Snap { scene: hexe_sop::save_scene_ext(&self.graph, &self.pos, &self.ui).ok(), display }
    }

    fn restore(&mut self, snap: &Snap) -> Result<(), String> {
        self.last = None;
        match &snap.scene {
            None => {
                self.graph = Graph::new();
                self.pos.clear();
                self.ui = hexe_sop::UiExtra::default();
            }
            Some(text) => {
                let l = hexe_sop::load_scene_with_base(text, &self.reg, None).map_err(|e| e.to_string())?;
                self.graph = l.graph;
                self.pos = l.pos;
                self.ui = l.ui;
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

    /// Timeline `$F` (1-based).
    pub fn frame(&self) -> i32 {
        self.frame
    }
    /// Timeline rate in frames per second.
    pub fn fps(&self) -> f32 {
        self.fps
    }
    /// Set `$F` (`1..=MAX_FRAME`); the next [`Session::cook`] runs at `time = ($F - 1) / fps`. Scrubbing is not an undoable edit.
    pub fn set_frame(&mut self, frame: i32) -> Result<(), String> {
        if !(1..=MAX_FRAME).contains(&frame) {
            return Err(format!("frame {frame} outside 1..={MAX_FRAME}"));
        }
        self.frame = frame;
        Ok(())
    }
    /// Set the timeline rate (finite, `0 < fps <= 1000`).
    pub fn set_fps(&mut self, fps: f32) -> Result<(), String> {
        if !fps.is_finite() || fps <= 0.0 || fps > 1000.0 {
            return Err(format!("fps {fps} must be in (0, 1000]"));
        }
        self.fps = fps;
        Ok(())
    }
    /// Cook time in seconds of the current `$F`.
    pub fn time(&self) -> f32 {
        (self.frame - 1) as f32 / self.fps
    }
    /// Cook the display node at the timeline time. Never panics, never `Err`: a failure is `CookResult.error` (+ `error_node`; the UI shows both).
    pub fn cook(&mut self) -> CookResult {
        self.last = None;
        let Some(display) = self.graph.display() else { return cook_fail("no display node".into(), None) };
        self.cook_at(display, true)
    }

    /// Cook any node (viewport template flag): same result shape as `cook`, but the spreadsheet's
    /// geometry (`last`) stays the display node's. Unknown id = an error result.
    pub fn cook_node(&mut self, id: u32) -> CookResult {
        match self.find(id) {
            Ok(n) => self.cook_at(n, false),
            Err(e) => cook_fail(e, None),
        }
    }

    fn cook_at(&mut self, node: NodeId, keep: bool) -> CookResult {
        let fail = cook_fail;
        // channel references first: their results are the parameter values this cook uses
        self.chan_err.clear();
        for (n, c, e) in
            hexe_sop::chans::apply_channels(&mut self.graph, &mut self.ui, f64::from(self.frame), f64::from(self.fps))
        {
            self.chan_err.entry(n).or_default().insert(c, e);
        }
        let (time, fps) = (self.time(), self.fps);
        let graph = &mut self.graph;
        graph.set_fps(fps);
        match catch_unwind(AssertUnwindSafe(|| graph.cook(node, time))) {
            Ok(Ok(detail)) => {
                let out = flatten(&detail);
                if keep {
                    self.last = Some(detail);
                }
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
                    (GraphError::Cycle(_), Some((_, nm))) => {
                        format!("{nm}: wired loop (its output feeds back into its own input)")
                    }
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
/// A failed cook: no geometry, the message, and the node it is tied to (red border in the editor).
fn cook_fail(error: String, error_node: Option<u32>) -> CookResult {
    CookResult { error: Some(error), error_node, ..CookResult::default() }
}

pub fn flatten(d: &Detail) -> CookResult {
    // NURBS/Bezier prims are drawn through their tessellation (8 samples per span)
    let tess;
    let d = if d.has_spline_prims() {
        tess = d.tessellate(8, 8);
        &tess
    } else {
        d
    };
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
    let prim_groups = d
        .groups
        .iter()
        .filter(|g| g.class == hexe_geo::AttrClass::Prim)
        .map(|g| (g.name.clone(), g.mask.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i as u32).collect()))
        .collect();
    let edge_groups =
        d.edge_groups.iter().map(|g| (g.name.clone(), g.edges.iter().flatten().copied().collect())).collect();
    CookResult {
        positions,
        prims,
        closed,
        prim_groups,
        shapes,
        cd,
        cd_point,
        counts,
        error: None,
        error_node: None,
        edge_groups,
    }
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
        assert_eq!(t.iter().map(|n| n.ty.as_str()).collect::<Vec<_>>().len(), app_registry().type_names().len());
    }

    /// hexe-native `motion` (sop-motion) is in the palette with its scene menu, though it is not a `hexe_sop` builtin.
    #[test]
    fn palette_lists_the_native_motion_node() {
        assert!(!hexe_sop::registry().type_names().contains(&"motion"));
        let t = Session::default().list_node_types();
        let m = t.iter().find(|n| n.ty == "motion").expect("motion in palette");
        assert!(m.inputs.is_empty());
        let scene = m.params.iter().find(|p| p.name == "scene").expect("scene param");
        assert_eq!(scene.kind, "menu");
        let labels: Vec<&str> = scene.menu.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["Stand", "Push", "Leg Hit", "Fall From Height"]);
    }

    /// Per-test scratch under `~/.gaia/cache/hexe/motion-test/` (never /tmp): `behaviours/{balance,stagger}.json` + `cache/` + the scene.
    fn motion_scene(tag: &str) -> (std::path::PathBuf, String) {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).expect("HOME");
        let dir = home.join(".gaia/cache/hexe/motion-test").join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("root/behaviours")).expect("dirs");
        for n in ["balance", "stagger"] {
            std::fs::write(dir.join(format!("root/behaviours/{n}.json")), format!(r#"{{"name":"{n}"}}"#))
                .expect("preset");
        }
        let stub = format!("{}/../../../fixtures/motion/stub.sh", env!("CARGO_MANIFEST_DIR"));
        let scene = json!({
            "nodes": [
                {"name": "motion", "type": "motion", "params": {
                    "mode": 2, "behaviour": "balance", "seed": 3, "duration_s": 2.0, "fps": 4.0,
                    "python_bin": stub, "motion_root": dir.join("root").to_string_lossy(), "cache_dir": dir.join("cache").to_string_lossy()}},
                {"name": "output1", "type": "output"}
            ],
            "edges": [["motion", 0, "output1", 0]],
            "display": "output1"
        });
        let path = dir.join("scene.json");
        std::fs::write(&path, scene.to_string()).expect("scene");
        (dir, path.to_string_lossy().into_owned())
    }
    /// Pelvis `P.y` (point 0) of a cook result.
    fn pelvis_y(c: &CookResult) -> f32 {
        c.positions[1]
    }

    /// Headless `--set motion.behaviour=stagger` round trip: override applies on load, shows in the param pane, reaches the CLI, and survives save/reload.
    #[test]
    fn set_behaviour_round_trip_headless() {
        let (dir, scene) = motion_scene("set-beh");
        let mut s = Session::new();
        s.set_overrides(vec!["motion.behaviour=stagger".into()]).expect("override parses");
        s.graph_load(&scene).expect("loads");
        assert!(s.set_errors.is_empty(), "{:?}", s.set_errors);
        let g = s.graph_get();
        let m = g.nodes.iter().find(|n| n.ty == "motion").expect("motion node");
        assert_eq!(m.params["behaviour"], json!("stagger"), "override beats the scene's balance");
        let nt = s.list_node_types();
        let b = nt
            .iter()
            .find(|n| n.ty == "motion")
            .and_then(|n| n.params.iter().find(|p| p.name == "behaviour"))
            .expect("param info");
        assert_eq!(b.kind, "string");
        assert!(!b.choices.is_empty(), "dropdown choices come from behaviours/*.json");
        let c = s.cook();
        assert_eq!(c.error, None);
        assert_eq!(c.counts.points, 24);
        let argv = std::fs::read_dir(dir.join("cache"))
            .expect("cache")
            .filter_map(Result::ok)
            .find_map(|e| std::fs::read_to_string(e.path().join("argv.txt")).ok())
            .expect("argv.txt");
        assert!(argv.contains("behaviour\n--name\nstagger\n") && argv.contains("--seed\n3"), "{argv}");
        // round trip: save, reload without the override -> the saved value is stagger
        let out = dir.join("saved.json");
        s.graph_save(&out.to_string_lossy()).expect("save");
        let mut t = Session::new();
        t.graph_load(&out.to_string_lossy()).expect("reload");
        let g2 = t.graph_get();
        assert_eq!(g2.nodes.iter().find(|n| n.ty == "motion").expect("motion").params["behaviour"], json!("stagger"));
        // a name outside the directory is a cook error that names the node and lists the choices
        let mut u = Session::new();
        u.set_overrides(vec!["motion.behaviour=nope".into()]).expect("override parses");
        u.graph_load(&scene).expect("loads");
        let e = u.cook().error.expect("unknown behaviour fails the cook");
        assert!(e.contains("nope") && e.contains("balance, stagger"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The timeline: `$F` drives the display cook time `($F - 1) / fps`; the motion node scrubs with it.
    #[test]
    fn timeline_frame_scrubs_the_motion_clip() {
        let (dir, scene) = motion_scene("timeline");
        let mut s = Session::new();
        s.graph_load(&scene).expect("loads");
        assert_eq!((s.frame(), s.fps()), (1, DEFAULT_TIMELINE_FPS));
        s.set_fps(4.0).expect("fps"); // timeline rate == clip rate (4) -> $F is the clip frame; stub pelvis y = (frame-1)/100
        for (f, want) in [(1, 0.0), (3, 0.02), (9, 0.08), (500, 0.08)] {
            s.set_frame(f).expect("frame");
            assert!((s.time() - (f - 1) as f32 / 4.0).abs() < 1e-6);
            let c = s.cook();
            assert_eq!(c.error, None, "F{f}");
            assert_eq!(pelvis_y(&c), want, "F{f}");
        }
        for bad in [0, -1, MAX_FRAME + 1] {
            assert!(s.set_frame(bad).is_err(), "{bad}");
        }
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY, 1001.0] {
            assert!(s.set_fps(bad).is_err(), "{bad}");
        }
        assert_eq!((s.frame(), s.fps()), (500, 4.0), "rejected values change nothing");
        let _ = std::fs::remove_dir_all(&dir);
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
        let attr = |class, v: Vec<[f32; 3]>| Attribute {
            name: "Cd".into(),
            class,
            data: AttrData::V3(v),
            type_info: TypeInfo::Color,
        };
        // prim Cd: repeated over each prim's corners
        d.attrs.push(attr(AttrClass::Prim, vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]));
        let c = flatten(&d);
        assert_eq!(
            c.cd.as_deref(),
            Some(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0][..])
        );
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
    fn objects_hold_separate_networks_and_round_trip() {
        let mut s = Session::new();
        assert_eq!((s.objects(), s.object_name()), (vec!["geo1".to_string()], "geo1"));
        s.node_add("box", 0.0, 0.0).expect("box in geo1");
        assert_eq!(s.object_add("geo1").expect("taken -> numbered"), "geo2");
        s.object_enter("geo2").expect("enter");
        assert!(s.graph_get().nodes.is_empty(), "geo2 starts empty");
        s.node_add("sphere", 0.0, 0.0).expect("sphere in geo2");
        s.node_add("null", 0.0, 100.0).expect("null in geo2");
        assert!(s.undo().expect("undo"), "geo2 has its own undo");
        assert_eq!(s.graph_get().nodes.len(), 1);
        // the other object cooks without leaving this one
        let c = s.cook_object("geo1").expect("cook geo1");
        assert_eq!((c.error, c.counts.points), (None, 8));
        assert_eq!(s.object_name(), "geo2");
        s.object_enter("geo1").expect("back");
        assert_eq!(s.graph_get().nodes.iter().map(|n| n.ty.as_str()).collect::<Vec<_>>(), ["box"]);
        assert!(!s.undo().expect("undo") || s.graph_get().nodes.is_empty(), "geo1's undo is its own");
        s.node_add("box", 0.0, 0.0).expect("box again");
        // rename (numbered when taken), remove, round trip through a multi-object file
        assert_eq!(s.object_rename("geo2", "geo1").expect("taken"), "geo2", "its own number is the next free one");
        assert_eq!(s.object_rename("geo2", "spheres").expect("free"), "spheres");
        s.object_add("empty").expect("empty object");
        let dir = std::env::temp_dir().join(format!("hexe-objects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let f = dir.join("multi.json");
        let fs = f.to_str().expect("utf8 path");
        s.graph_save(fs).expect("save");
        let mut t = Session::new();
        t.graph_load(fs).expect("load");
        assert_eq!(t.objects(), ["empty", "geo1", "spheres"]);
        assert_eq!(t.object_name(), "geo1", "current object kept");
        t.object_enter("spheres").expect("enter");
        assert_eq!(t.graph_get().nodes.iter().map(|n| n.ty.as_str()).collect::<Vec<_>>(), ["sphere"]);
        t.object_remove("spheres").expect("remove current");
        assert!(!t.objects().contains(&"spheres".to_string()) && t.objects().contains(&t.object_name().to_string()));
        // a single object still saves as a plain scene (old files and tools keep working)
        let mut one = Session::new();
        one.node_add("box", 0.0, 0.0).expect("box");
        let p1 = dir.join("one.json");
        one.graph_save(p1.to_str().expect("utf8")).expect("save one");
        assert!(!std::fs::read_to_string(&p1).expect("read").contains("hexe_objects"));
        // hostile: bad names, unknown objects, duplicate names / empty list in a file
        assert!(s.object_add("a b").is_err() && s.object_add("").is_err() && s.object_enter("nope").is_err());
        assert!(s.object_remove("nope").is_err() && s.object_rename("nope", "x").is_err());
        for bad in [
            r#"{"hexe_objects":1,"objects":[]}"#,
            r#"{"hexe_objects":1,"objects":[{"name":"a","scene":null},{"name":"a","scene":null}]}"#,
            r#"{"hexe_objects":1,"objects":[{"name":"../x","scene":null}]}"#,
        ] {
            let p = dir.join("bad.json");
            std::fs::write(&p, bad).expect("write");
            assert!(t.graph_load(p.to_str().expect("utf8")).is_err(), "{bad}");
        }
        // hundreds of objects are fine
        let mut many = Session::new();
        for _ in 0..300 {
            many.object_add("geo").expect("add");
        }
        assert_eq!(many.objects().len(), 301);
        many.graph_new();
        assert_eq!(many.objects(), ["geo1"], "File > New = one empty geo1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tab_insert_into_a_wire_is_one_undo_step() {
        let mut s = Session::new();
        let a = s.node_add("box", 0.0, 0.0).expect("box");
        let b = s.node_add("null", 0.0, 200.0).expect("null");
        s.edge_connect(a, b, 0).expect("wire");
        let edge = |s: &Session, from, to, input| s.graph_get().edges.contains(&EdgeView { from, to, input });
        // a filter goes between: a -> xform -> b
        let x = s.node_add_wired("xform", 0.0, 100.0, Some(a), Some((b, 0))).expect("xform spliced in");
        assert!(edge(&s, a, x, 0) && edge(&s, x, b, 0) && !edge(&s, a, b, 0));
        assert!(s.undo().expect("undo"), "one step back");
        let g = s.graph_get();
        assert_eq!(g.nodes.len(), 2, "the spliced node is gone");
        let (a, b) = (g.nodes[0].id, g.nodes[1].id);
        assert!(edge(&s, a, b, 0), "the original wire is back");
        // a generator takes no input: Box only feeds b, a is left unwired below
        let bx = s.node_add_wired("box", 0.0, 100.0, Some(a), Some((b, 0))).expect("box spliced in");
        assert!(edge(&s, bx, b, 0) && !s.graph_get().edges.iter().any(|e| e.to == bx));
        // hostile: bad input index / unknown ids / non-finite position change nothing
        let n = s.graph_get().nodes.len();
        assert!(s.node_add_wired("xform", 0.0, 0.0, Some(a), Some((b, 9))).is_err());
        assert!(s.node_add_wired("xform", 0.0, 0.0, Some(999), Some((b, 0))).is_err());
        assert!(s.node_add_wired("xform", f32::NAN, 0.0, Some(a), Some((b, 0))).is_err());
        assert!(s.node_add_wired("no-such-sop", 0.0, 0.0, Some(a), Some((b, 0))).is_err());
        assert_eq!(s.graph_get().nodes.len(), n);
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
        // a two-node loop wires (Houdini editor parity): the cook errors on a loop node instead of hanging, cut = cooks again
        let n = s.node_add("null", 0.0, 200.0).expect("null");
        s.edge_connect(pe, n, 0).expect("pe -> null");
        s.edge_connect(n, pe, 0).expect("null -> pe closes the loop");
        let c = s.cook();
        assert!(c.error.as_deref().is_some_and(|e| e.contains("wired loop")), "{:?}", c.error);
        assert!(c.error_node == Some(pe) || c.error_node == Some(n), "{:?}", c.error_node);
        s.edge_connect(b, pe, 0).expect("rewire breaks the loop");
        assert_eq!(s.cook().error, None);
        s.node_remove(n).expect("remove");
        s.node_remove(pe).expect("remove");
        let g = s.graph_get();
        assert_eq!((g.nodes.len(), g.display), (1, Some(b)), "display falls back to a live node");
        assert!(s.node_remove(pe).is_err());
        assert!(s.node_add("no-such-sop", 0.0, 0.0).is_err());
    }

    /// H40: File -> Open routes a Houdini `.hip` through the importer (4 nodes, 3 wires, merge1 displayed).
    #[test]
    fn load_hip_opens_houdini_scene() {
        let mut s = Session::new();
        let p = format!("{}/../../../fixtures/hip/authored-chain.hip", env!("CARGO_MANIFEST_DIR"));
        s.graph_load(&p).expect("hip loads");
        let g = s.graph_get();
        assert_eq!((g.nodes.len(), g.edges.len()), (4, 3));
        assert!(g.display.is_some());
        assert_eq!(s.cook().error, None);
        assert!(s.graph_load("/nonexistent/x.hip").is_err());
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
    fn cook_node_templates_without_touching_the_display_cook() {
        let mut s = Session::new();
        // hostile ids: unknown / huge -> an error result, never a panic
        assert!(s.cook_node(0).error.is_some());
        assert!(s.cook_node(u32::MAX).error.is_some());
        let a = s.node_add("box", 0.0, 0.0).expect("box");
        let b = s.node_add("sphere", 200.0, 0.0).expect("sphere");
        s.set_display(Some(b)).expect("display sphere");
        let disp = s.cook();
        let t = s.cook_node(a);
        assert!(t.error.is_none(), "{:?}", t.error);
        assert_eq!(t.counts.points, 8, "the template is the box");
        // the spreadsheet still shows the display (sphere) geometry
        assert_eq!(s.spreadsheet("point", 0, 1).expect("points").total, disp.counts.points as usize);
        assert_ne!(disp.counts.points, 8);
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
        // a taken name gets the next free number: box1 is the box, so `box1` and `box` both land on box2
        s.node_rename(pe, "box1").expect("taken name renumbers");
        assert!(names(&s).contains(&"box2".to_string()), "{:?}", names(&s));
        let third = s.node_add("null", 0.0, 0.0).expect("null");
        s.node_rename(third, "box2").expect("taken name renumbers");
        assert!(names(&s).contains(&"box3".to_string()), "box1 + box2 taken: {:?}", names(&s));
        s.node_rename(third, "box").expect("a free name is kept as typed");
        assert!(names(&s).contains(&"box".to_string()));
        s.node_rename(pe, "box").expect("box2 renamed `box` keeps its own number");
        assert!(names(&s).contains(&"box2".to_string()), "{:?}", names(&s));
        s.node_remove(third).expect("remove");
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

    /// Wires by node NAME (ids are not stable across undo / load), sorted.
    fn named_edges(s: &Session) -> Vec<(String, String, usize)> {
        let g = s.graph_get();
        let nm = |id: u32| g.nodes.iter().find(|n| n.id == id).map(|n| n.name.clone()).unwrap_or_default();
        let mut v: Vec<_> = g.edges.iter().map(|e| (nm(e.from), nm(e.to), e.input)).collect();
        v.sort();
        v
    }
    fn e3(a: &str, b: &str, i: usize) -> (String, String, usize) {
        (a.to_string(), b.to_string(), i)
    }
    /// Network dots (Houdini `__dotN`): dropped on a wire or at a drawn wire's end in ONE undo step, fan out, never the
    /// display node, deleting one rewires its source straight through to everything it fed, and they save -> load.
    #[test]
    fn dots_insert_fan_out_delete_through_and_round_trip() {
        let mut s = Session::new();
        let b = s.node_add("box", 0.0, 0.0).expect("box");
        let x = s.node_add("xform", 0.0, 200.0).expect("xform");
        let m = s.node_add("merge", 200.0, 300.0).expect("merge");
        s.edge_connect(b, x, 0).expect("wire");
        s.edge_connect(x, m, 1).expect("wire");
        s.set_display(Some(m)).expect("display");
        // on the box -> xform wire
        let undo0 = s.history().0;
        let d1 = s.dot_add(50.0, 100.0, Some(b), Some((x, 0))).expect("dot on a wire");
        assert_eq!(s.history().0, undo0 + 1, "one undo step");
        assert_eq!(s.name_of(d1).as_deref(), Some("__dot1"));
        // at the end of a wire drawn out of the dot (then wired on): __dot1 fans out to xform1 and __dot2 -> merge1.0
        let d2 = s.dot_add(150.0, 150.0, Some(d1), None).expect("dot at a wire end");
        s.edge_connect(d2, m, 0).expect("wire");
        assert_eq!(s.name_of(d2).as_deref(), Some("__dot2"));
        assert_eq!(
            named_edges(&s),
            [
                e3("__dot1", "__dot2", 0),
                e3("__dot1", "xform1", 0),
                e3("__dot2", "merge1", 0),
                e3("box1", "__dot1", 0),
                e3("xform1", "merge1", 1)
            ]
        );
        let c = s.cook();
        assert_eq!((c.error.as_deref(), c.counts.points), (None, 16), "dots pass geometry through");
        let dv = s.graph_get().nodes.into_iter().find(|n| n.id == d1).expect("dot view");
        assert_eq!((dv.ty.as_str(), dv.x, dv.y), ("dot", 50.0, 100.0));
        // never the display flag
        assert!(s.set_display(Some(d1)).is_err());
        assert_eq!(s.graph_get().display, Some(m));
        // hostile: nothing changes, no history
        let (n, h) = (s.graph_get().nodes.len(), s.history().0);
        assert!(s.dot_add(0.0, 0.0, Some(999), None).is_err());
        assert!(s.dot_add(0.0, 0.0, Some(b), Some((x, 7))).is_err(), "no such input port");
        assert!(s.dot_add(0.0, 0.0, None, Some((999, 0))).is_err());
        assert!(s.dot_add(f32::NAN, 0.0, None, None).is_err());
        assert!(s.dot_add(0.0, 0.0, Some(m), Some((b, 0))).is_err(), "box has no input");
        assert_eq!((s.graph_get().nodes.len(), s.history().0), (n, h));
        // save -> load keeps dots, their positions, pin flag and every wire through them
        s.node_set_param(d2, "pinned", &json!(true)).expect("pin");
        let out = std::env::temp_dir().join(format!("hexe-tauri-dots-{}.json", std::process::id()));
        let path = out.to_string_lossy().into_owned();
        s.graph_save(&path).expect("save");
        let mut t = Session::new();
        t.graph_load(&path).expect("reload");
        let _ = std::fs::remove_file(&out);
        assert_eq!(named_edges(&t), named_edges(&s));
        let back = t.graph_get();
        let d2b = back.nodes.iter().find(|n| n.name == "__dot2").expect("__dot2 reloaded");
        assert_eq!((d2b.ty.as_str(), d2b.x, d2b.y, &d2b.params["pinned"]), ("dot", 150.0, 150.0, &json!(true)));
        assert_eq!(back.display.and_then(|d| t.name_of(d)).as_deref(), Some("merge1"));
        // delete __dot1: box1 feeds xform1 and __dot2 directly (Houdini destroy()), one undo step
        s.nodes_remove(&[d1]).expect("remove dot");
        assert_eq!(
            named_edges(&s),
            [e3("__dot2", "merge1", 0), e3("box1", "__dot2", 0), e3("box1", "xform1", 0), e3("xform1", "merge1", 1)]
        );
        assert_eq!(s.cook().counts.points, 16);
        s.undo().expect("undo");
        assert_eq!(named_edges(&s).len(), 5);
        // a dot chain removed together: the first live source upstream takes over
        let ids: Vec<u32> = s.graph_get().nodes.iter().filter(|n| n.ty == "dot").map(|n| n.id).collect();
        s.nodes_remove(&ids).expect("remove both dots");
        assert_eq!(named_edges(&s), [e3("box1", "merge1", 0), e3("box1", "xform1", 0), e3("xform1", "merge1", 1)]);
        assert_eq!(s.graph_get().display.and_then(|d| s.name_of(d)).as_deref(), Some("merge1"));
    }
    /// The display node never falls back onto a dot, and a dot added to an empty graph does not take the flag.
    #[test]
    fn dots_never_inherit_the_display_flag() {
        let mut s = Session::new();
        let d = s.dot_add(0.0, 0.0, None, None).expect("lone dot");
        assert_eq!(s.graph_get().display, None);
        let b = s.node_add("box", 0.0, 0.0).expect("box");
        assert_eq!(s.graph_get().display, Some(b));
        let n = s.node_add("null", 0.0, 100.0).expect("null");
        s.set_display(Some(n)).expect("display");
        s.node_remove(n).expect("remove the display node");
        assert_eq!(s.graph_get().display, Some(b), "falls back past the dot");
        let _ = d;
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

    #[test]
    fn null_object_holds_control_and_point1() {
        let mut s = Session::new();
        assert_eq!(s.object_add_kind("null1", "null", None).expect("null"), "null1");
        s.object_enter("null1").expect("enter");
        let g = s.graph_get();
        let names: Vec<&str> = g.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["control", "point1"]);
        let ctl = &g.nodes[0];
        assert_eq!((ctl.ty.as_str(), g.display), ("control", Some(ctl.id)), "the control SOP carries the display flag");
        assert_eq!(g.nodes[1].params["points"], Value::from("0,0,0"), "point1 = one point at the origin");
        // Uniform Scale drives how far the indicator lines reach
        s.node_set_param(ctl.id, "scale", &Value::from(3.0)).expect("scale");
        let c = s.cook();
        assert_eq!((c.error.as_deref(), c.counts.points, c.counts.prims), (None, 7, 6));
        assert!(c.positions.chunks(3).any(|p| p == [3.0, 0.0, 0.0]));
        assert!(s.object_add_kind("x", "camera", None).is_err(), "unknown kind");
        assert!(s.object_add_kind("x", "geo", Some("null1")).is_err(), "a Null is no container");
    }

    fn spare(name: &str, kind: &str, size: u8, default: Value) -> hexe_sop::SpareParm {
        serde_json::from_value(
            serde_json::json!({ "name": name, "label": name, "kind": kind, "size": size, "default": default }),
        )
        .expect("spare")
    }

    #[test]
    fn spare_parms_set_value_undo_rename_and_round_trip() {
        let mut s = Session::new();
        let id = s.node_add("null", 0.0, 0.0).expect("null");
        // the null SOP's own parms (Houdini): Copy Input on, Cache Input off
        let g = s.graph_get();
        assert_eq!(
            (g.nodes[0].params["copyinput"].clone(), g.nodes[0].params["cacheinput"].clone()),
            (Value::Bool(true), Value::Bool(false))
        );
        let mut tab = spare("tab1", "folder", 1, Value::Null);
        tab.label = "Controls".into();
        let mut count = spare("count", "int", 1, Value::from(3));
        count.folder = Some("tab1".into());
        let offset = spare("offset", "float", 3, serde_json::json!([0.0, 1.0, 0.0]));
        s.node_spare_set(id, vec![tab, count.clone(), offset.clone()]).expect("set");
        let sp = &s.graph_get().nodes[0].spare;
        assert_eq!(sp.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["tab1", "count", "offset"]);
        assert_eq!((sp[1].current().clone(), sp[1].folder.as_deref()), (Value::from(3), Some("tab1")));
        // values: coerced to the kind (int rounds), checked
        s.node_spare_value(id, "count", &Value::from(7.6)).expect("value");
        assert_eq!(s.graph_get().nodes[0].spare[1].current(), &Value::from(8));
        s.node_spare_value(id, "offset", &serde_json::json!([1, 2, 3])).expect("v3");
        assert!(s.node_spare_value(id, "offset", &serde_json::json!([1, 2])).is_err(), "wrong size");
        assert!(s.node_spare_value(id, "count", &Value::from("x")).is_err(), "wrong type");
        assert!(s.node_spare_value(id, "tab1", &Value::from(1)).is_err(), "a folder holds no value");
        assert!(s.node_spare_value(id, "nope", &Value::from(1)).is_err(), "unknown");
        // re-setting the interface keeps values of unchanged params; undo restores
        s.node_spare_set(id, vec![count.clone(), offset]).expect("drop the folder");
        let sp = &s.graph_get().nodes[0].spare;
        assert_eq!(
            (sp.len(), sp[0].current().clone(), sp[0].folder.clone()),
            (2, Value::from(8), None),
            "folder ref to a gone folder is cleared"
        );
        assert!(s.undo().expect("undo"));
        assert_eq!(s.graph_get().nodes[0].spare.len(), 3);
        // rename carries the interface
        s.node_rename(id, "ctrl").expect("rename");
        let g = s.graph_get();
        let id = g.nodes[0].id;
        assert_eq!(g.nodes[0].spare.len(), 3);
        // bad interfaces are refused whole
        for bad in [
            vec![spare("1x", "int", 1, Value::from(0))],
            vec![spare("copyinput", "toggle", 1, Value::Bool(false))],
            vec![count.clone(), count.clone()],
            vec![spare("w", "matrix", 1, Value::Null)],
        ] {
            assert!(s.node_spare_set(id, bad).is_err());
        }
        assert_eq!(s.graph_get().nodes[0].spare.len(), 3, "a refused set changes nothing");
        // save / load round trip; a hostile file entry is dropped, not fatal
        let dir = std::env::temp_dir().join(format!("hexe-spare-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let out = dir.join("s.json");
        s.graph_save(&out.to_string_lossy()).expect("save");
        let mut t = Session::new();
        t.graph_load(&out.to_string_lossy()).expect("load");
        let sp = &t.graph_get().nodes[0].spare;
        assert_eq!(
            (sp.len(), sp[1].current().clone(), sp[2].current().clone()),
            (3, Value::from(8), serde_json::json!([1.0, 2.0, 3.0]))
        );
        let text = std::fs::read_to_string(&out).expect("read").replace("\"count\"", "\"9bad\"");
        std::fs::write(&out, text).expect("write");
        let mut u = Session::new();
        u.graph_load(&out.to_string_lossy()).expect("load");
        assert_eq!(u.graph_get().nodes[0].spare.len(), 2, "the invalid name is dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_reference_to_a_spare_parameter_drives_a_param_and_follows_renames() {
        let mut s = Session::new();
        let null = s.node_add("null", 0.0, 0.0).expect("null");
        let bx = s.node_add("box", 0.0, 100.0).expect("box");
        s.set_display(Some(bx)).expect("display");
        s.node_spare_set(null, vec![spare("newparameter2", "int", 1, Value::from(2))]).expect("spare");
        // Copy Parameter on null1.newparameter2 -> Paste Relative References on box1's Uniform Scale
        s.node_set_channel(bx, "scale", Some("ch(\"../null1/newparameter2\")")).expect("channel");
        let c = s.cook();
        assert!(c.error.is_none(), "{:?}", c.error);
        let scale =
            |s: &Session| s.graph_get().nodes.iter().find(|n| n.ty == "box").expect("box").params["scale"].clone();
        assert_eq!(scale(&s), Value::from(2.0));
        assert!(c.positions.iter().any(|p| (p.abs() - 1.0).abs() < 1e-6), "box of size 1 scaled by 2 reaches +-1");
        // the source changes -> the next cook follows
        s.node_spare_value(null, "newparameter2", &Value::from(5)).expect("value");
        s.cook();
        assert_eq!(scale(&s), Value::from(5.0));
        let g = s.graph_get();
        let b = g.nodes.iter().find(|n| n.ty == "box").expect("box");
        assert_eq!(b.chans["scale"], "ch(\"../null1/newparameter2\")");
        // renaming the source rewrites the reference; the expression keeps working
        s.node_rename(null, "ctrl").expect("rename");
        s.cook();
        let g = s.graph_get();
        let b = g.nodes.iter().find(|n| n.ty == "box").expect("box");
        assert_eq!((b.chans["scale"].as_str(), b.chan_errors.len()), ("ch(\"../ctrl/newparameter2\")", 0));
        // a broken reference is reported on the field, the cook still runs
        let bx = b.id;
        s.node_set_channel(bx, "scale", Some("ch(\"../ghost/p\")")).expect("set");
        assert!(s.cook().error.is_none());
        assert!(s.graph_get().nodes.iter().find(|n| n.ty == "box").expect("box").chan_errors.contains_key("scale"));
        // typing a value replaces the expression; bad channels / syntax are refused
        s.node_set_param(bx, "scale", &Value::from(3.0)).expect("typed");
        assert!(s.graph_get().nodes.iter().find(|n| n.ty == "box").expect("box").chans.is_empty());
        assert!(s.node_set_channel(bx, "nope", Some("1")).is_err());
        assert!(s.node_set_channel(bx, "scale", Some("1 +")).is_err());
        assert!(
            s.node_set_channel(bx, "sizex", Some("ch(\"../ctrl/newparameter2\") * 2")).is_ok(),
            "component channel"
        );
        s.cook();
        let size = s.graph_get().nodes.iter().find(|n| n.ty == "box").expect("box").params["size"].clone();
        assert_eq!(size[0], Value::from(10.0));
        // Lock Parameter: setters refuse, channel writes leave it alone, unlock restores editing
        s.node_set_lock(bx, "size", true).expect("lock");
        assert_eq!(s.graph_get().nodes.iter().find(|n| n.ty == "box").expect("box").locked, ["size"]);
        assert!(s.node_set_param(bx, "size", &serde_json::json!([1, 1, 1])).is_err());
        assert!(s.node_set_channel(bx, "sizey", Some("2")).is_err());
        s.node_spare_value(null, "newparameter2", &Value::from(9)).expect("source");
        s.cook();
        let size = s.graph_get().nodes.iter().find(|n| n.ty == "box").expect("box").params["size"].clone();
        assert_eq!(size[0], Value::from(10.0), "locked: the channel does not write");
        assert!(s.node_set_lock(bx, "nope", true).is_err());
        s.node_set_lock(bx, "size", false).expect("unlock");
        s.cook();
        let size = s.graph_get().nodes.iter().find(|n| n.ty == "box").expect("box").params["size"].clone();
        assert_eq!(size[0], Value::from(18.0));
        assert!(s.undo().expect("undo"));
    }

    #[test]
    fn null_object_transform_params_default_set_and_round_trip() {
        let mut s = Session::new();
        s.node_add("box", 0.0, 0.0).expect("box");
        s.object_add_kind("null1", "null", None).expect("null");
        let (specs, vals) = s.object_params("null1").expect("params");
        // Houdini 20.5 Null OBJ: Transform tab first, with its defaults
        let tr: Vec<&str> =
            specs.iter().filter(|p| p.folder.as_deref() == Some("Transform")).map(|p| p.name.as_str()).collect();
        assert_eq!(
            tr,
            [
                "xOrd",
                "rOrd",
                "t",
                "r",
                "s",
                "p",
                "pr",
                "scale",
                "pre_xform",
                "keeppos",
                "childcomp",
                "constraints_on",
                "constraints_path"
            ]
        );
        assert!(
            specs.iter().any(|p| p.folder.as_deref() == Some("Render"))
                && specs.iter().any(|p| p.folder.as_deref() == Some("Misc"))
        );
        assert_eq!(
            (vals["t"].clone(), vals["s"].clone(), vals["scale"].clone()),
            (serde_json::json!([0.0, 0.0, 0.0]), serde_json::json!([1.0, 1.0, 1.0]), Value::from(1.0))
        );
        assert_eq!(
            (vals["xOrd"].clone(), vals["rOrd"].clone(), vals["keeppos"].clone()),
            (Value::from(0), Value::from(0), Value::Bool(false))
        );
        assert_eq!(specs.iter().find(|p| p.name == "xOrd").expect("xOrd").menu[0].label, "Scale Rot Trans");
        // set, save, reload
        s.object_set_param("null1", "t", &serde_json::json!([1.5, 2, -3])).expect("t");
        s.object_set_param("null1", "rOrd", &Value::from(5)).expect("rOrd");
        let dir = std::env::temp_dir().join(format!("hexe-nullparms-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let out = dir.join("n.json");
        s.graph_save(&out.to_string_lossy()).expect("save");
        let mut t = Session::new();
        t.graph_load(&out.to_string_lossy()).expect("reload");
        let (_, v2) = t.object_params("null1").expect("params");
        assert_eq!((v2["t"].clone(), v2["rOrd"].clone()), (serde_json::json!([1.5, 2.0, -3.0]), Value::from(5)));
        // back to the default = not stored
        t.object_set_param("null1", "t", &serde_json::json!([0, 0, 0])).expect("reset");
        assert!(!t.meta["null1"].params.contains_key("t"));
        // hostile: unknown object / parm, wrong type, NaN, menu out of range, a geo has no object parms yet
        assert!(t.object_set_param("nope", "t", &serde_json::json!([0, 0, 0])).is_err());
        assert!(t.object_set_param("null1", "bogus", &Value::from(1)).is_err());
        assert!(t.object_set_param("null1", "t", &Value::from("x")).is_err());
        assert!(t.object_set_param("null1", "scale", &serde_json::json!(f64::NAN)).is_err());
        assert!(t.object_set_param("null1", "xOrd", &Value::from(6)).is_err());
        assert!(t.object_set_param("geo1", "t", &serde_json::json!([0, 0, 0])).is_err());
        // a file with a bad object param is refused, not half-loaded
        std::fs::write(
            &out,
            r#"{"hexe_objects":1,"objects":[{"name":"n","kind":"null","scene":null,"params":{"t":"oops"}}]}"#,
        )
        .expect("write");
        assert!(Session::new().graph_load(&out.to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn containers_hold_objects_round_trip_and_delete_with_them() {
        let mut s = Session::new();
        s.node_add("box", 0.0, 0.0).expect("box");
        let asset = s.object_add_kind("asset1", "subnet", None).expect("container");
        let inner = s.object_add_kind("inner_geo1", "geo", Some(&asset)).expect("geo inside");
        let info = s.objects_info();
        let get = |n: &str| info.iter().find(|o| o.name == n).cloned().expect("listed");
        assert_eq!((get(&asset).kind.as_str(), get(&asset).net.clone()), ("subnet", None));
        assert_eq!((get(&inner).kind.as_str(), get(&inner).net.as_deref()), ("geo", Some("asset1")));
        // rename the container: what is inside follows
        s.object_rename(&asset, "Featured").expect("rename");
        let net_of = |s: &Session, n: &str| s.objects_info().into_iter().find(|o| o.name == n).and_then(|o| o.net);
        assert_eq!(net_of(&s, &inner).as_deref(), Some("Featured"));
        let dir = std::env::temp_dir().join(format!("hexe-containers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let f = dir.join("boxes.json");
        let fs = f.to_str().expect("utf8 path");
        s.graph_save(fs).expect("save");
        let mut t = Session::new();
        t.graph_load(fs).expect("load");
        assert_eq!(t.objects_info(), s.objects_info(), "kinds + containers survive save / load");
        // deleting the container deletes what it holds
        t.object_remove("Featured").expect("remove");
        assert_eq!(t.objects(), ["geo1"]);
        // hostile: a container loop / a missing container lands at /obj, unknown kinds are refused
        let bad = r#"{"hexe_objects":1,"objects":[{"name":"a","kind":"subnet","net":"b","scene":null},
            {"name":"b","kind":"subnet","net":"a","scene":null},{"name":"c","net":"zz","scene":null}]}"#;
        std::fs::write(&f, bad).expect("write");
        let mut u = Session::new();
        u.graph_load(fs).expect("loads");
        let nets: Vec<Option<String>> = u.objects_info().into_iter().map(|o| o.net).collect();
        assert!(nets.iter().filter(|n| n.is_some()).count() <= 1, "the loop is cut: {nets:?}");
        assert_eq!(nets[2], None, "missing container -> /obj");
        std::fs::write(&f, r#"{"hexe_objects":1,"objects":[{"name":"a","kind":"rocket","scene":null}]}"#)
            .expect("write");
        assert!(Session::new().graph_load(fs).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bypass_is_undoable_and_required_inputs_are_reported() {
        let mut s = Session::new();
        let b = s.node_add("box", 0.0, 0.0).expect("box");
        let pr = s.node_add("polyreduce", 0.0, 90.0).expect("polyreduce");
        let x = s.node_add("xform", 0.0, 180.0).expect("xform");
        s.edge_connect(b, x, 0).expect("wire");
        let node = |s: &Session, id: u32| s.graph_get().nodes.into_iter().find(|n| n.id == id).expect("node");
        assert_eq!((node(&s, pr).required, node(&s, pr).inputs.len()), (1, 3), "only Triangular Mesh is required");
        assert_eq!(node(&s, x).required, 1);
        assert_eq!(node(&s, b).required, 0);
        s.set_bypass(x, true).expect("bypass");
        assert!(node(&s, x).bypass);
        assert!(s.undo().expect("undo"));
        let xf = s.graph_get().nodes.into_iter().find(|n| n.name == "xform1").expect("xform1 (ids change on undo)");
        assert!(!xf.bypass, "bypass is one undo step");
        assert!(s.set_bypass(9999, true).is_err());
    }
}
