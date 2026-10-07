//! Graph JSON → `Graph<Detail>`.
//! ```json
//! { "nodes": [{"name":"box1","type":"box","params":{"size":[1,1,1],"divrate":[4,4,4]}}],
//!   "edges": [["box1",0,"xform1",0]],   // [src name, src output, dst name, dst input]
//!   "display": "xform1" }
//! ```
//! Param values are typed by the node's `ParamSpec` default: F number · I integer · B bool or 0/1 · V3 `[x,y,z]` ·
//! S string · Menu non-negative integer. Unknown keys / params / types are errors. An input left unconnected is fed an
//! empty `Detail` (hidden source node) so a lone `null` cooks to empty geometry instead of `MissingInput`.
use crate::{Registry, SopNode, registry};
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, Graph, InputSpec, Node, NodeId, ParamSet, ParamSpec, ParamValue};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

/// Why a scene failed to load. Message only; never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneError(pub String);

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scene: {}", self.0)
    }
}

impl std::error::Error for SceneError {}

fn err<T>(msg: impl Into<String>) -> Result<T, SceneError> {
    Err(SceneError(msg.into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneDef {
    nodes: Vec<NodeDef>,
    #[serde(default)]
    edges: Vec<(String, usize, String, usize)>,
    display: String,
    /// Editor-only data (node positions); the cook ignores it.
    #[serde(default)]
    ui: UiDef,
}

#[derive(Deserialize, Default)]
struct UiDef {
    #[serde(default)]
    pos: BTreeMap<String, [f32; 2]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeDef {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    params: BTreeMap<String, Value>,
}

/// A loaded scene: the graph, its display node, and the name → id map.
pub struct Loaded {
    pub graph: Graph<Detail>,
    pub display: NodeId,
    pub names: BTreeMap<String, NodeId>,
    /// `ui.pos` of the file: node name -> editor position (unknown names dropped, non-finite dropped).
    pub pos: BTreeMap<String, [f32; 2]>,
}

/// Hidden generator feeding unconnected inputs.
struct EmptySource;

impl Node<Detail> for EmptySource {
    fn type_name(&self) -> &'static str {
        "(empty)"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &[]
    }
    fn cook(&self, _: &CookCtx, _: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        Ok(Arc::new(Detail::default()))
    }
}

/// Load with the built-in registry. Returns the graph and the display node (cook it with `graph.cook(id, time)`).
pub fn load_scene(json: &str) -> Result<(Graph<Detail>, NodeId), SceneError> {
    let l = load_scene_with(json, &registry())?;
    Ok((l.graph, l.display))
}

/// Like `load_scene`, resolving relative `file` paths against `base` (the scene file's directory) when the file exists there;
/// otherwise the path stays as written (resolved against the cwd at cook time).
pub fn load_scene_in(json: &str, base: Option<&Path>) -> Result<(Graph<Detail>, NodeId), SceneError> {
    let l = load_scene_with_base(json, &registry(), base)?;
    Ok((l.graph, l.display))
}
/// Load against an explicit registry (tests, custom node sets).
pub fn load_scene_with(json: &str, reg: &Registry) -> Result<Loaded, SceneError> {
    load_scene_with_base(json, reg, None)
}
/// Node types whose string param is an input path: `(type, param)`.
const INPUT_PATHS: &[(&str, &str)] = &[("file", "file")];
pub fn load_scene_with_base(json: &str, reg: &Registry, base: Option<&Path>) -> Result<Loaded, SceneError> {
    let def: SceneDef = serde_json::from_str(json).map_err(|e| SceneError(format!("invalid scene JSON: {e}")))?;
    let mut graph: Graph<Detail> = Graph::new();
    let mut names: BTreeMap<String, NodeId> = BTreeMap::new();
    for n in &def.nodes {
        if n.name.is_empty() {
            return err("node with empty name");
        }
        if names.contains_key(&n.name) {
            return err(format!("duplicate node name `{}`", n.name));
        }
        let node: Box<SopNode> = reg
            .create(&n.ty)
            .ok_or_else(|| SceneError(format!("node `{}`: unknown type `{}` (see `hexe-cli nodes`)", n.name, n.ty)))?;
        let mut overrides = overrides_for(&n.name, &n.ty, node.params(), &n.params)?;
        if let Some(base) = base {
            for (_, token) in INPUT_PATHS.iter().filter(|(t, _)| *t == n.ty) {
                if let Some(rel) = overrides.string(token).filter(|s| Path::new(s).is_relative()) {
                    let joined = base.join(rel);
                    if joined.exists() {
                        overrides = overrides.with(token, ParamValue::S(joined.to_string_lossy().into_owned()));
                    }
                }
            }
        }
        let id = graph.add_boxed(node, overrides).map_err(|e| SceneError(format!("node `{}`: {e}", n.name)))?;
        graph.rename(id, &n.name).map_err(|e| SceneError(format!("node `{}`: {e}", n.name)))?;
        names.insert(n.name.clone(), id);
    }
    let find = |name: &str| names.get(name).copied().ok_or_else(|| SceneError(format!("edge: unknown node `{name}`")));
    for (src, out, dst, input) in &def.edges {
        let (s, d) = (find(src)?, find(dst)?);
        graph.connect(s, *out, d, *input).map_err(|e| SceneError(format!("edge {src}:{out} -> {dst}:{input}: {e}")))?;
    }
    fill_unwired(&mut graph)?;
    let display = find(&def.display).map_err(|_| SceneError(format!("display: unknown node `{}`", def.display)))?;
    graph.set_display(Some(display)).map_err(|e| SceneError(format!("display: {e}")))?;
    let pos =
        def.ui.pos.into_iter().filter(|(k, p)| names.contains_key(k) && p.iter().all(|c| c.is_finite())).collect();
    Ok(Loaded { graph, display, names, pos })
}

/// Feed every unconnected input of every named node from the hidden empty source (created on first need), so a
/// lone `null` or a half-wired node cooks to empty geometry instead of `MissingInput`. Idempotent; the editor
/// calls it after every structural edit.
pub fn fill_unwired(graph: &mut Graph<Detail>) -> Result<(), SceneError> {
    let he = |e: hexe_graph::GraphError| SceneError(format!("hidden empty source: {e}"));
    let mut empty = graph.node_ids().into_iter().find(|&id| graph.node_type(id) == Some(EmptySource.type_name()));
    let named: Vec<NodeId> = graph.node_ids().into_iter().filter(|&id| graph.name(id).is_some()).collect();
    for id in named {
        let open: Vec<usize> =
            graph.input_sources(id).iter().enumerate().filter(|(_, s)| s.is_none()).map(|(i, _)| i).collect();
        for input in open {
            let src = match empty {
                Some(e) => e,
                None => {
                    let e = graph.add_node(EmptySource, ParamSet::new()).map_err(he)?;
                    empty = Some(e);
                    e
                }
            };
            graph.connect(src, 0, id, input).map_err(he)?;
        }
    }
    Ok(())
}

/// `ParamValue` -> JSON, the inverse of `param_value` (f32 via its shortest decimal so reload is bit-exact).
fn param_json(v: &ParamValue) -> Value {
    let num = |x: f32| {
        format!("{x}").parse::<f64>().ok().and_then(serde_json::Number::from_f64).map_or(Value::Null, Value::Number)
    };
    match v {
        ParamValue::F(x) => num(*x),
        ParamValue::I(x) => Value::from(*x),
        ParamValue::B(x) => Value::Bool(*x),
        ParamValue::V3(c) => Value::Array(c.iter().map(|x| num(*x)).collect()),
        ParamValue::S(s) => Value::String(s.clone()),
        ParamValue::Menu(m) => Value::from(*m),
    }
}

/// Graph -> scene JSON in the exact schema `load_scene_with` reads: named nodes (creation order), non-default
/// params only, typed edges (the hidden empty source is not written), display (falls back to the first node),
/// plus `"ui": {"pos": ..}` from `pos` (entries for unknown names are dropped). Errors on an empty graph.
pub fn save_scene(graph: &Graph<Detail>, pos: &BTreeMap<String, [f32; 2]>) -> Result<String, SceneError> {
    let named: Vec<(NodeId, &str)> =
        graph.node_ids().into_iter().filter_map(|id| Some((id, graph.name(id)?))).collect();
    let Some(&(first, _)) = named.first() else { return err("nothing to save: the graph has no nodes") };
    let (mut nodes, mut edges) = (Vec::new(), Vec::new());
    for &(id, name) in &named {
        let ty = graph.node_type(id).unwrap_or_default();
        let mut params = serde_json::Map::new();
        for spec in graph.param_specs(id) {
            if let Some(v) = graph.param(id, &spec.name).filter(|v| **v != spec.default) {
                params.insert(spec.name.clone(), param_json(v));
            }
        }
        let mut node = serde_json::json!({ "name": name, "type": ty });
        if !params.is_empty() {
            node["params"] = Value::Object(params);
        }
        nodes.push(node);
        for (port, src) in graph.input_sources(id).into_iter().enumerate() {
            if let Some(s) = src.and_then(|s| graph.name(s)) {
                edges.push(serde_json::json!([s, 0, name, port]));
            }
        }
    }
    let display = graph.display().and_then(|d| graph.name(d)).or_else(|| graph.name(first)).unwrap_or_default();
    let pos: serde_json::Map<String, Value> = pos
        .iter()
        .filter(|(k, p)| graph.node_by_name(k).is_some() && p.iter().all(|c| c.is_finite()))
        .map(|(k, p)| (k.clone(), Value::Array(p.iter().map(|c| param_json(&ParamValue::F(*c))).collect())))
        .collect();
    let doc = serde_json::json!({ "nodes": nodes, "edges": edges, "display": display, "ui": { "pos": pos } });
    serde_json::to_string_pretty(&doc).map_err(|e| SceneError(format!("serialize: {e}")))
}

fn overrides_for(
    name: &str,
    ty: &str,
    specs: &[ParamSpec],
    given: &BTreeMap<String, Value>,
) -> Result<ParamSet, SceneError> {
    let mut set = ParamSet::new();
    for (token, v) in given {
        let spec = specs
            .iter()
            .find(|s| &s.name == token)
            .ok_or_else(|| SceneError(format!("node `{name}` ({ty}): unknown parameter `{token}`")))?;
        let value = param_value(&spec.default, v)
            .map_err(|m| SceneError(format!("node `{name}` ({ty}) parameter `{token}`: {m}")))?;
        set = set.with(token, value);
    }
    Ok(set)
}

fn finite_f32(v: &Value) -> Result<f32, String> {
    let x = v.as_f64().ok_or("expected a number")?;
    let f = x as f32;
    if f.is_finite() { Ok(f) } else { Err(format!("{x} is not representable as a finite f32")) }
}

fn whole(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| {
        let f = v.as_f64()?;
        (f.fract() == 0.0 && f.abs() < 9.0e15).then_some(f as i64)
    })
}

/// JSON value → `ParamValue` of the same kind as `like` (the spec default).
fn param_value(like: &ParamValue, v: &Value) -> Result<ParamValue, String> {
    Ok(match like {
        ParamValue::F(_) => ParamValue::F(finite_f32(v)?),
        ParamValue::I(_) => {
            let n = whole(v).ok_or("expected an integer")?;
            ParamValue::I(i32::try_from(n).map_err(|_| format!("{n} does not fit i32"))?)
        }
        ParamValue::B(_) => match v {
            Value::Bool(b) => ParamValue::B(*b),
            other => match whole(other) {
                Some(0) => ParamValue::B(false),
                Some(1) => ParamValue::B(true),
                _ => return Err("expected true/false or 0/1".into()),
            },
        },
        ParamValue::V3(_) => {
            let a = v.as_array().ok_or("expected [x,y,z]")?;
            match a.as_slice() {
                [x, y, z] => ParamValue::V3([finite_f32(x)?, finite_f32(y)?, finite_f32(z)?]),
                _ => return Err(format!("expected 3 components, got {}", a.len())),
            }
        }
        ParamValue::S(_) => ParamValue::S(v.as_str().ok_or("expected a string")?.to_string()),
        ParamValue::Menu(_) => {
            let n = whole(v).ok_or("expected a menu index (integer)")?;
            ParamValue::Menu(u32::try_from(n).map_err(|_| format!("{n} is not a valid menu index"))?)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test SOP with one param of every kind; cooks to an empty Detail (params are read via `Graph::param`).
    struct Probe {
        specs: Vec<ParamSpec>,
    }
    fn probe_spec() -> Vec<ParamSpec> {
        vec![
            ParamSpec::new("f", ParamValue::F(1.5)),
            ParamSpec::new("i", ParamValue::I(2)),
            ParamSpec::new("b", ParamValue::B(false)),
            ParamSpec::new("v", ParamValue::V3([0.0; 3])),
            ParamSpec::new("s", ParamValue::S("x".into())),
            ParamSpec::new("m", ParamValue::Menu(0)),
        ]
    }
    fn probe_new() -> Box<SopNode> {
        Box::new(Probe { specs: probe_spec() })
    }
    impl Node<Detail> for Probe {
        fn type_name(&self) -> &'static str {
            "probe"
        }
        fn inputs(&self) -> &[InputSpec] {
            &[InputSpec { name: "a" }, InputSpec { name: "b" }]
        }
        fn params(&self) -> &[ParamSpec] {
            &self.specs
        }
        fn cook(&self, _: &CookCtx, i: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
            let mut d = Detail::default();
            for x in i {
                d.merge(x);
            }
            Ok(Arc::new(d))
        }
    }
    fn reg() -> Registry {
        let mut r = registry();
        r.register("probe", probe_spec, probe_new);
        r
    }
    fn load(json: &str) -> Result<Loaded, SceneError> {
        load_scene_with(json, &reg())
    }
    fn fails(json: &str, needle: &str) {
        match load(json) {
            Ok(_) => panic!("expected failure containing `{needle}`"),
            Err(e) => assert!(e.0.contains(needle), "`{}` lacks `{needle}`", e.0),
        }
    }

    #[test]
    fn lone_null_cooks_to_empty_detail() {
        let (mut g, id) = load_scene(r#"{"nodes":[{"name":"n","type":"null"}],"display":"n"}"#).expect("loads");
        assert_eq!(*g.cook(id, 0.0).expect("cooks"), Detail::default());
    }

    #[test]
    fn params_typed_by_spec() {
        let l = load(
            r#"{"nodes":[{"name":"p","type":"probe","params":{"f":3,"i":4.0,"b":1,"v":[1,2,3.5],"s":"hi","m":2}}],"display":"p"}"#,
        )
        .expect("loads");
        let p = l.names["p"];
        let g = &l.graph;
        assert_eq!(g.param(p, "f"), Some(&ParamValue::F(3.0)));
        assert_eq!(g.param(p, "i"), Some(&ParamValue::I(4)));
        assert_eq!(g.param(p, "b"), Some(&ParamValue::B(true)));
        assert_eq!(g.param(p, "v"), Some(&ParamValue::V3([1.0, 2.0, 3.5])));
        assert_eq!(g.param(p, "s"), Some(&ParamValue::S("hi".into())));
        assert_eq!(g.param(p, "m"), Some(&ParamValue::Menu(2)));
    }

    #[test]
    fn unset_params_keep_spec_defaults() {
        let l = load(r#"{"nodes":[{"name":"p","type":"probe"}],"display":"p"}"#).expect("loads");
        assert_eq!(l.graph.param(l.names["p"], "f"), Some(&ParamValue::F(1.5)));
    }

    #[test]
    fn edges_chain_and_partial_inputs_get_empty() {
        let json = r#"{"nodes":[{"name":"a","type":"null"},{"name":"p","type":"probe"},{"name":"z","type":"null"}],
            "edges":[["a",0,"p",1],["p",0,"z",0]],"display":"z"}"#;
        let mut l = load(json).expect("loads");
        assert_eq!(l.display, l.names["z"]);
        assert!(l.graph.cook(l.display, 0.0).is_ok());
    }

    #[test]
    fn hostile_scenes_are_errors() {
        fails("not json", "invalid scene JSON");
        fails(r#"{"nodes":[],"display":"x"}"#, "display: unknown node `x`");
        fails(r#"{"nodes":[]}"#, "display");
        fails(r#"{"nodes":[],"display":"x","extra":1}"#, "extra");
        fails(r#"{"nodes":[{"name":"","type":"null"}],"display":""}"#, "empty name");
        fails(r#"{"nodes":[{"name":"a","type":"nope"}],"display":"a"}"#, "unknown type `nope`");
        let dup = r#"{"nodes":[{"name":"a","type":"null"},{"name":"a","type":"null"}],"display":"a"}"#;
        fails(dup, "duplicate node name");
        let node =
            |params: &str| format!(r#"{{"nodes":[{{"name":"p","type":"probe","params":{params}}}],"display":"p"}}"#);
        fails(&node(r#"{"zz":1}"#), "unknown parameter `zz`");
        fails(&node(r#"{"f":"x"}"#), "expected a number");
        fails(&node(r#"{"f":1e40}"#), "finite f32");
        fails(&node(r#"{"i":1.5}"#), "integer");
        fails(&node(r#"{"i":99999999999}"#), "i32");
        fails(&node(r#"{"b":2}"#), "true/false");
        fails(&node(r#"{"v":[1,2]}"#), "3 components");
        fails(&node(r#"{"v":[1,2,"x"]}"#), "number");
        fails(&node(r#"{"s":5}"#), "string");
        fails(&node(r#"{"m":-1}"#), "menu index");
        let two = r#"{"nodes":[{"name":"a","type":"null"},{"name":"b","type":"null"}],"display":"b","edges":"#;
        fails(&format!(r#"{two}[["a",0,"q",0]]}}"#), "unknown node `q`");
        fails(&format!(r#"{two}[["a",0,"b",3]]}}"#), "no input 3");
        fails(&format!(r#"{two}[["a",1,"b",0]]}}"#), "no output 1");
        fails(&format!(r#"{two}[["a",0,"b",0],["b",0,"a",0]]}}"#), "cycle");
        fails(&format!(r#"{two}[["a",0]]}}"#), "invalid scene JSON");
    }

    fn counts(l: &mut Loaded) -> (u32, u32) {
        let d = l.graph.cook(l.display, 0.0).expect("cooks");
        (d.point_count, d.prim_count())
    }

    /// load -> save -> load again: same cook counts, same params; second save is a fixed point.
    #[test]
    fn save_round_trips_examples() {
        for src in [
            include_str!("../../../examples/copytopoints.json"),
            include_str!("../../../examples/polyextrude.json"),
            include_str!("../../../examples/scatter.json"),
            include_str!("../../../examples/attribnoise.json"),
        ] {
            let mut a = load(src).expect("loads");
            let json = save_scene(&a.graph, &a.pos).expect("saves");
            let mut b = load(&json).expect("saved scene reloads");
            assert_eq!(counts(&mut a), counts(&mut b));
            assert!(counts(&mut a).0 > 0, "example cooks to something");
            assert_eq!(save_scene(&b.graph, &b.pos).expect("saves"), json, "save is a fixed point");
            assert_eq!(a.names.keys().collect::<Vec<_>>(), b.names.keys().collect::<Vec<_>>());
        }
    }

    #[test]
    fn ui_pos_persists_and_hostile_ui_is_dropped() {
        let s = r#"{"nodes":[{"name":"a","type":"null"}],"display":"a","ui":{"pos":{"a":[10.5,-3],"ghost":[1,1]}}}"#;
        let l = load(s).expect("ui accepted");
        assert_eq!(l.pos.get("a"), Some(&[10.5, -3.0]));
        assert!(!l.pos.contains_key("ghost"), "pos of an unknown node is dropped");
        let again = load(&save_scene(&l.graph, &l.pos).expect("saves")).expect("reloads");
        assert_eq!(again.pos, l.pos);
        fails(&s.replace("[10.5,-3]", "[1]"), "invalid scene JSON");
        assert!(load(&s.replace("\"ui\"", "\"junk\"")).is_err(), "other unknown top-level keys stay errors");
    }

    #[test]
    fn edited_graph_saves_and_cooks() {
        let mut l = load(include_str!("../../../examples/polyextrude.json")).expect("loads");
        let before = counts(&mut l);
        let reg = registry();
        let b2 = l.graph.add_boxed(reg.create("box").expect("box"), ParamSet::new()).expect("adds");
        let nm = l.graph.unique_name("box");
        assert_eq!(nm, "box2");
        l.graph.rename(b2, &nm).expect("names");
        fill_unwired(&mut l.graph).expect("fills");
        let pe = l.graph.node_by_name("pe1").expect("pe1");
        l.graph.connect(b2, 0, pe, 0).expect("connects");
        let mut json_l = load(&save_scene(&l.graph, &BTreeMap::new()).expect("saves")).expect("reloads");
        assert_eq!(counts(&mut json_l).1, before.1, "rewired to the second box: same topology");
        assert_eq!(json_l.names.len(), 3);
        // removing a node leaves its dependent with an empty-source input after fill_unwired
        l.graph.remove_node(b2).expect("removes");
        fill_unwired(&mut l.graph).expect("fills");
        assert_eq!(l.graph.cook(pe, 0.0).expect("cooks empty").point_count, 0);
        assert!(save_scene(&l.graph, &BTreeMap::new()).expect("saves").contains("pe1"));
        assert!(!save_scene(&l.graph, &BTreeMap::new()).expect("saves").contains("(empty)"));
        assert!(save_scene(&Graph::<Detail>::new(), &BTreeMap::new()).is_err());
    }

    /// The oracle fixture's `expected.geo` is the analytical answer: an empty Detail written by our writer.
    #[test]
    fn null_passthrough_fixture_expected_is_empty_detail() {
        let expected = include_bytes!("../../../fixtures/null-passthrough/expected.geo");
        assert_eq!(hexe_geo::write_geo_json(&Detail::default()), expected.to_vec());
        let scene = include_str!("../../../fixtures/null-passthrough/scene.json");
        let (mut g, id) = load_scene(scene).expect("fixture scene loads");
        assert_eq!(hexe_geo::write_geo_json(&g.cook(id, 0.0).expect("cooks")), expected.to_vec());
    }
}
