//! Channel expressions on node parameters (Houdini: a parm holding `ch("../null1/newparameter2")`). Each node's
//! `UiExtra::chan` maps a channel (a parameter name, or a component such as `tx` / `newparameter2y`) to an HScript
//! expression; `apply_channels` evaluates them with `hexe_graph::hscript` and writes the results into the graph's
//! parameters and the spare parameters before a cook. Paths are relative to the evaluating node, inside one network:
//! `parm` (same node), `../node/parm` (a sibling), `/obj/<object>/<node>/parm` (the node in this network).
use crate::scene::{SpareParm, UiExtra};
use hexe_geo::Detail;
use hexe_graph::hscript::{self, ExprError, Scope, Value};
use hexe_graph::{Graph, NodeId, ParamValue};
use std::collections::BTreeMap;

/// Passes over all channels (a chain of references settles in as many passes as it is long; a cycle stops here).
pub const MAX_PASSES: usize = 16;
/// Most channels kept per node.
pub const MAX_CHANS: usize = 1024;

/// Component suffixes of a tuple parameter, in component order (Houdini: `tx`, `colorr`, `uv1` ...).
const SUFFIXES: [[&str; 4]; 4] =
    [["x", "y", "z", "w"], ["r", "g", "b", "a"], ["1", "2", "3", "4"], ["u", "v", "w", ""]];

/// Where a channel lives on a node: a built-in parameter (component `None` = the whole scalar) or a spare parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    Param(String, Option<usize>),
    Spare(usize, Option<usize>),
}

/// Split `chan` into (base, component); `base_ok` gives a base parameter's tuple size: `tx` -> (`t`, Some(0)), `t` -> (`t`, None).
fn component(chan: &str, base_ok: impl Fn(&str) -> Option<usize>) -> Option<(String, Option<usize>)> {
    if base_ok(chan).is_some() {
        return Some((chan.to_string(), None));
    }
    for set in SUFFIXES {
        for (i, s) in set.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
            if let Some(base) = chan.strip_suffix(s)
                && let Some(n) = base_ok(base)
                && n > 1
                && i < n
            {
                return Some((base.to_string(), Some(i)));
            }
        }
    }
    None
}

/// Resolve channel `chan` of node `id` to a slot (built-in parameters first, then spare ones).
pub fn slot(graph: &Graph<Detail>, spare: &[SpareParm], id: NodeId, chan: &str) -> Option<Slot> {
    let size = |v: &ParamValue| if matches!(v, ParamValue::V3(_)) { 3 } else { 1 };
    let specs = graph.param_specs(id);
    if let Some((base, c)) = component(chan, |b| specs.iter().find(|s| s.name == b).map(|s| size(&s.default))) {
        return Some(Slot::Param(base, c));
    }
    let holds = |p: &SpareParm| matches!(p.kind.as_str(), "float" | "int" | "toggle" | "string" | "menu");
    component(chan, |b| spare.iter().find(|p| p.name == b && holds(p)).map(|p| usize::from(p.size)))
        .and_then(|(base, c)| spare.iter().position(|p| p.name == base).map(|i| Slot::Spare(i, c)))
}

/// The parameter (built-in or spare) a slot belongs to.
pub fn slot_parm<'a>(s: &'a Slot, spare: &'a [SpareParm]) -> &'a str {
    match s {
        Slot::Param(n, _) => n,
        Slot::Spare(i, _) => spare.get(*i).map_or("", |p| p.name.as_str()),
    }
}

/// Every channel name of a node's parameter `parm` (built-in or spare): `[parm]` for a scalar, its components for a tuple.
pub fn channels_of(graph: &Graph<Detail>, spare: &[SpareParm], id: NodeId, parm: &str) -> Vec<String> {
    let n = graph
        .param_specs(id)
        .iter()
        .find(|s| s.name == parm)
        .map(|s| if matches!(s.default, ParamValue::V3(_)) { 3 } else { 1 })
        .or_else(|| spare.iter().find(|p| p.name == parm).map(|p| usize::from(p.size)));
    match n {
        Some(1) => vec![parm.to_string()],
        Some(n) => SUFFIXES[0].iter().take(n).map(|s| format!("{parm}{s}")).collect(),
        None => Vec::new(),
    }
}

fn read(graph: &Graph<Detail>, spare: &[SpareParm], id: NodeId, s: &Slot) -> Option<Value> {
    let num = |v: &serde_json::Value| {
        v.as_f64().map(Value::Num).or_else(|| v.as_bool().map(|b| Value::Num(f64::from(u8::from(b)))))
    };
    match s {
        Slot::Param(name, c) => Some(match (graph.param(id, name)?, c) {
            (ParamValue::F(x), _) => Value::Num(f64::from(*x)),
            (ParamValue::I(x), _) => Value::Num(f64::from(*x)),
            (ParamValue::B(b), _) => Value::Num(f64::from(u8::from(*b))),
            (ParamValue::Menu(m), _) => Value::Num(f64::from(*m)),
            (ParamValue::S(t), _) => Value::Str(t.clone()),
            (ParamValue::V3(v), Some(i)) => Value::Num(f64::from(*v.get(*i)?)),
            (ParamValue::V3(_), None) => return None,
        }),
        Slot::Spare(i, c) => {
            let v = spare.get(*i)?.current();
            match (v, c) {
                (serde_json::Value::Array(a), Some(k)) => num(a.get(*k)?),
                (serde_json::Value::String(t), None) => Some(Value::Str(t.clone())),
                (v, None) => num(v),
                _ => None,
            }
        }
    }
}

/// Write `v` into slot `s`; `Ok(true)` = the value changed.
fn write(graph: &mut Graph<Detail>, spare: &mut [SpareParm], id: NodeId, s: &Slot, v: &Value) -> Result<bool, String> {
    match s {
        Slot::Param(name, c) => {
            let cur = graph.param(id, name).cloned().ok_or_else(|| format!("no parameter `{name}`"))?;
            let x = v.num();
            if !x.is_finite() {
                return Err(format!("{name}: not a finite number"));
            }
            let new = match (&cur, c) {
                (ParamValue::F(_), _) => ParamValue::F(x as f32),
                (ParamValue::I(_), _) => {
                    ParamValue::I(x.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
                }
                (ParamValue::B(_), _) => ParamValue::B(x != 0.0),
                (ParamValue::Menu(_), _) => ParamValue::Menu(x.round().clamp(0.0, f64::from(u32::MAX)) as u32),
                (ParamValue::S(_), _) => ParamValue::S(v.text()),
                (ParamValue::V3(a), Some(i)) => {
                    let mut a = *a;
                    *a.get_mut(*i).ok_or("component out of range")? = x as f32;
                    ParamValue::V3(a)
                }
                (ParamValue::V3(_), None) => return Err(format!("{name}: a tuple needs a component channel")),
            };
            if new == cur || !new.is_acceptable() {
                return Ok(false);
            }
            graph.set_param(id, name, new).map_err(|e| e.to_string())?;
            Ok(true)
        }
        Slot::Spare(i, c) => {
            let p = spare.get_mut(*i).ok_or("spare parameter gone")?;
            let mut cur = p.current().clone();
            let x = || serde_json::Value::from(v.num());
            match (&mut cur, c) {
                (serde_json::Value::Array(a), Some(k)) => *a.get_mut(*k).ok_or("component out of range")? = x(),
                (serde_json::Value::String(_), None) => cur = serde_json::Value::from(v.text()),
                (_, None) => cur = if p.kind == "toggle" { serde_json::Value::Bool(v.num() != 0.0) } else { x() },
                _ => return Err(format!("{}: a tuple needs a component channel", p.name)),
            }
            let new = p.coerce(&cur).ok_or_else(|| format!("{}: value does not fit", p.name))?;
            if new == *p.current() {
                return Ok(false);
            }
            p.value = new;
            Ok(true)
        }
    }
}

struct At<'a> {
    graph: &'a Graph<Detail>,
    ui: &'a UiExtra,
    me: NodeId,
    frame: f64,
    fps: f64,
}
impl Scope for At<'_> {
    fn var(&self, name: &str) -> Option<Value> {
        match name {
            "F" => Some(Value::Num(self.frame.round())),
            "FF" => Some(Value::Num(self.frame)),
            "T" => Some(Value::Num((self.frame - 1.0) / self.fps)),
            "FPS" => Some(Value::Num(self.fps)),
            _ => None,
        }
    }
    fn channel(&self, path: &str, _string: bool) -> Result<Value, ExprError> {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
        let (node, chan) = match parts.as_slice() {
            [c] => (self.me, *c),
            ["..", n, c] => {
                (self.graph.node_by_name(n).ok_or_else(|| ExprError::Unsupported(format!("no node `{n}`")))?, *c)
            }
            [.., n, c] if path.starts_with('/') => {
                (self.graph.node_by_name(n).ok_or_else(|| ExprError::Unsupported(format!("no node `{n}` here")))?, *c)
            }
            _ => return Err(ExprError::Unsupported(format!("path `{path}` (only this network)"))),
        };
        let name = self.graph.name(node).unwrap_or_default();
        let spare = self.ui.spare.get(name).map_or(&[][..], Vec::as_slice);
        let s = slot(self.graph, spare, node, chan)
            .ok_or_else(|| ExprError::Unsupported(format!("no channel `{chan}` on {name}")))?;
        read(self.graph, spare, node, &s).ok_or_else(|| ExprError::Unsupported(format!("`{chan}` has no value")))
    }
}

/// Evaluate every channel expression of `ui.chan` at `frame` / `fps` and write the results (parameters + spare values)
/// until nothing changes or `MAX_PASSES` passes. Returns (node, channel, error) for each expression that failed in the last
/// pass (its parameter keeps its value). Never panics.
pub fn apply_channels(
    graph: &mut Graph<Detail>,
    ui: &mut UiExtra,
    frame: f64,
    fps: f64,
) -> Vec<(String, String, String)> {
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 24.0 };
    let mut errors = Vec::new();
    for _ in 0..MAX_PASSES {
        errors.clear();
        let mut results = Vec::new();
        for (node, chans) in &ui.chan {
            let Some(id) = graph.node_by_name(node) else { continue };
            for (chan, expr) in chans {
                let at = At { graph, ui, me: id, frame, fps };
                match hscript::eval(expr, &at) {
                    Ok(v) => results.push((node.clone(), id, chan.clone(), v)),
                    Err(e) => errors.push((node.clone(), chan.clone(), e.to_string())),
                }
            }
        }
        let mut changed = false;
        for (node, id, chan, v) in results {
            let spare = ui.spare.entry(node.clone()).or_default();
            let res = match slot(graph, spare, id, &chan) {
                // a locked parameter keeps its value (Houdini Lock Parameter)
                Some(s) if ui.lock.get(&node).is_some_and(|l| l.contains(slot_parm(&s, spare))) => Ok(false),
                Some(s) => write(graph, spare, id, &s, &v),
                None => Err(format!("no channel `{chan}`")),
            };
            if spare.is_empty() {
                ui.spare.remove(&node);
            }
            match res {
                Ok(c) => changed |= c,
                Err(e) => errors.push((node, chan, e)),
            }
        }
        if !changed {
            break;
        }
    }
    errors
}

/// Channel table of one node with only resolvable channels and capped expressions (load-time sanitizing).
pub fn sanitize_chans(chans: BTreeMap<String, String>) -> BTreeMap<String, String> {
    chans
        .into_iter()
        .filter(|(c, e)| !c.is_empty() && c.len() <= 128 && !e.trim().is_empty() && e.len() <= hscript::MAX_EXPR)
        .take(MAX_CHANS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;
    use crate::scene::load_scene_with;

    const SCENE: &str = r#"{"nodes":[{"name":"null1","type":"null"},{"name":"box1","type":"box"},{"name":"xform1","type":"xform"}],
      "edges":[["box1",0,"xform1",0]],"display":"xform1",
      "ui":{"spare":{"null1":[{"name":"newparameter2","label":"Count","kind":"int","default":3},
                              {"name":"off","label":"Off","kind":"float","size":3,"default":[0.5,1,2]}]},
            "chan":{"box1":{"sizex":"ch(\"../null1/newparameter2\") * 2","sizez":"ch(\"../null1/offz\")"},
                    "xform1":{"scale":"ch(\"../box1/sizex\") + $F","ty":"ch(\"ty\")"},
                    "null1":{"offx":"ch(\"newparameter2\")"}}}}"#;

    #[test]
    fn relative_references_drive_parameters_and_spare_values() {
        let mut l = load_scene_with(SCENE, &registry()).expect("loads");
        let errs = apply_channels(&mut l.graph, &mut l.ui, 4.0, 24.0);
        assert!(errs.is_empty(), "{errs:?}");
        let g = &l.graph;
        let (b, x) = (g.node_by_name("box1").expect("box"), g.node_by_name("xform1").expect("xform"));
        let ParamValue::V3(size) = g.param(b, "size").cloned().expect("size") else { panic!("size is a vector") };
        assert_eq!((size[0], size[2]), (6.0, 2.0), "sizex = 3*2, sizez = the spare vector's z");
        // a chain (xform1.scale <- box1.sizex <- null1.newparameter2) settles; $F = 4
        assert_eq!(g.param(x, "scale"), Some(&ParamValue::F(10.0)));
        assert_eq!(l.ui.spare["null1"][1].current(), &serde_json::json!([3.0, 1.0, 2.0]), "spare driven by a spare");
        // editing the source and re-applying follows
        l.ui.spare.get_mut("null1").expect("null1")[0].value = serde_json::json!(5);
        assert!(apply_channels(&mut l.graph, &mut l.ui, 4.0, 24.0).is_empty());
        assert_eq!(l.graph.param(b, "size").cloned(), Some(ParamValue::V3([10.0, size[1], 2.0])));
        assert_eq!(l.graph.param(x, "scale"), Some(&ParamValue::F(14.0)));
    }

    #[test]
    fn channels_of_list_components_and_bad_references_are_errors_not_panics() {
        let mut l = load_scene_with(SCENE, &registry()).expect("loads");
        let n = l.graph.node_by_name("null1").expect("null1");
        let b = l.graph.node_by_name("box1").expect("box1");
        assert_eq!(channels_of(&l.graph, &l.ui.spare["null1"], n, "off"), ["offx", "offy", "offz"]);
        assert_eq!(channels_of(&l.graph, &[], b, "size"), ["sizex", "sizey", "sizez"]);
        assert_eq!(channels_of(&l.graph, &[], b, "nope"), Vec::<String>::new());
        for bad in [
            "ch(\"../ghost/p\")",
            "ch(\"../null1/nope\")",
            "ch(\"../../obj2/n/p\")",
            "1 +",
            "ch(\"../box1/size\")",
            "ch(\"../box1/sizex\") / 0 + 1e400",
        ] {
            l.ui.chan.insert("xform1".into(), BTreeMap::from([("tx".to_string(), bad.to_string())]));
            let errs = apply_channels(&mut l.graph, &mut l.ui, 1.0, f64::NAN);
            assert!(
                errs.iter().any(|(node, c, _)| node == "xform1" && c == "tx") || bad.contains("1e400"),
                "{bad}: {errs:?}"
            );
        }
        // a cycle stops after MAX_PASSES instead of hanging
        l.ui.chan.insert("box1".into(), BTreeMap::from([("sizey".to_string(), "ch(\"sizey\") + 1".to_string())]));
        apply_channels(&mut l.graph, &mut l.ui, 1.0, 24.0);
        let ParamValue::V3(s) = l.graph.param(b, "size").cloned().expect("size") else { panic!() };
        assert!(s[1] <= 1.0 + MAX_PASSES as f32 + 1.0);
    }
}
