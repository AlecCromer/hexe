//! `.hip` parm expressions evaluated at import (`hexe_graph::hscript`): every channel-driven parm component
//! (`[ tx 0 ]` in `.parm` + `channel tx { ... expr = ch("../ctrl/width")/2 }` in `.chn`) gets the value its
//! expression has at the file's saved frame, so a value Houdini computes (`6`) replaces the stale number saved in the
//! file (`0`). `ch()` paths resolve across the whole scene (relative to the evaluating node, `..` = its network,
//! absolute `/obj/...`); referenced parms that are channels themselves evaluate recursively (depth-capped: a reference
//! cycle stops with a note). Segment choice: the segment whose time span holds the saved time (`start` + lengths).
//! NOT evaluated (saved value kept + a note naming why): python channels, keyframe-interpolation segments
//! (`bezier()` / `linear()` / ...), geometry queries (`detail()`, `point()`, `$N`, `centroid()`), `rand()`, `@` refs,
//! backtick expressions inside strings. The result is a constant: the link to the driving parm is not kept live.
use hexe_graph::hip::{HipChannel, HipNode, HipScene, HipValue};
use hexe_graph::hscript::{self, ExprError, Scope, Value};
use std::cell::Cell;
use std::collections::BTreeMap;

/// Deepest chain of channel references followed (`a` -> `ch(b)` -> `ch(c)` ...).
const MAX_REF_DEPTH: usize = 32;
/// Keyframe-interpolation segment functions (their value comes from keys, not from the expression).
const KEY_FUNCS: [&str; 12] = [
    "bezier", "linear", "constant", "cubic", "ease", "easein", "easeout", "easep", "easeinp", "easeoutp", "qlinear",
    "quintic",
];

struct Index<'a> {
    hip: &'a HipScene,
    /// `obj/net/node` -> (network, node)
    nodes: BTreeMap<String, (usize, usize)>,
    frame: f64,
    time: f64,
}

fn net_path(hip: &HipScene, i: usize) -> String {
    let n = &hip.networks[i];
    if n.parent.is_empty() { format!("obj/{}", n.name) } else { format!("obj/{}/{}", n.parent, n.name) }
}

impl<'a> Index<'a> {
    fn new(hip: &'a HipScene) -> Self {
        let mut nodes = BTreeMap::new();
        for (i, net) in hip.networks.iter().enumerate() {
            let base = net_path(hip, i);
            for (j, n) in net.nodes.iter().enumerate() {
                nodes.insert(format!("{base}/{}", n.name), (i, j));
            }
        }
        let fps = if hip.fps.is_finite() && hip.fps > 0.0 { hip.fps } else { 24.0 };
        let frame = hip.frame();
        Index { hip, nodes, frame, time: (frame - 1.0) / fps }
    }
    fn node(&self, path: &str) -> Option<&'a HipNode> {
        let &(i, j) = self.nodes.get(path)?;
        self.hip.networks.get(i)?.nodes.get(j)
    }
    /// Value of channel `chan` of node `path` (by channel name or parm-name + component suffix). `ch()` (`string`
    /// false) of a menu parm saved as a token gives the token's position, like Houdini.
    fn value(&self, path: &str, chan: &str, depth: usize, string: bool) -> Result<Value, ExprError> {
        let v = self.raw_value(path, chan, depth)?;
        if let (false, Value::Str(tok)) = (string, &v) {
            let node = self.node(path);
            let ty = node.map_or("", |n| n.ty.as_str());
            let base = ty.split("::").next().unwrap_or(ty);
            let menu = |t: &str| crate::hip_import::menu_position(t, chan, tok);
            if let Some(i) = menu(ty).or_else(|| menu(base)).or_else(|| ramp_basis(chan, tok)) {
                return Ok(Value::Num(i as f64));
            }
        }
        Ok(v)
    }
    fn raw_value(&self, path: &str, chan: &str, depth: usize) -> Result<Value, ExprError> {
        if depth > MAX_REF_DEPTH {
            return Err(ExprError::Unsupported(format!("channel references deeper than {MAX_REF_DEPTH} (cycle?)")));
        }
        let node = self.node(path).ok_or_else(|| ExprError::Unsupported(format!("no node `/{path}`")))?;
        if let Some(c) = node.channels.iter().find(|c| c.name == chan) {
            return self.channel(path, c, depth);
        }
        let comp =
            component(node, chan).ok_or_else(|| ExprError::Unsupported(format!("no parm `{chan}` on /{path}")))?;
        Ok(match comp.plain() {
            HipValue::Num(x) => Value::Num(*x),
            HipValue::Str(s) => Value::Str(s.clone()),
            HipValue::Chan(..) => Value::Num(0.0),
        })
    }
    fn channel(&self, path: &str, c: &HipChannel, depth: usize) -> Result<Value, ExprError> {
        let mut t0 = c.start;
        let mut pick = c.segments.first();
        for s in &c.segments {
            pick = Some(s);
            if s.length <= 0.0 || self.time < t0 + s.length {
                break;
            }
            t0 += s.length;
        }
        let seg = pick.ok_or_else(|| ExprError::Unsupported(format!("channel {} has no segment", c.name)))?;
        if seg.python {
            return Err(ExprError::Unsupported("python expression".into()));
        }
        let expr = seg.expr.as_deref().unwrap_or("").trim();
        if let Some(f) = expr.strip_suffix("()").filter(|f| KEY_FUNCS.contains(f)) {
            return Err(ExprError::Unsupported(format!("keyframe segment `{f}()`")));
        }
        hscript::eval(expr, &At { ix: self, path, depth: Cell::new(depth + 1) })
    }
}

/// Ramp key interpolation (`<ramp>Ninterp`) token -> position (hou.rampBasis order: hython `ch()` of a `"linear"` key = 1).
fn ramp_basis(chan: &str, tok: &str) -> Option<usize> {
    const BASIS: [&str; 7] = ["constant", "linear", "catmull-rom", "monotonecubic", "bezier", "bspline", "hermite"];
    if chan.ends_with("interp") { BASIS.iter().position(|b| *b == tok) } else { None }
}
/// Parm component named `chan`: the channel name stored in `[ chan v ]`, the parm itself (1 component), or
/// parm name + a component suffix (`tx` = t[0], `colorr` = color[0], `size2` = size[1]).
fn component<'n>(node: &'n HipNode, chan: &str) -> Option<&'n HipValue> {
    for p in &node.parms {
        for v in &p.values {
            if let HipValue::Chan(c, _) = v
                && c == chan
            {
                return Some(v);
            }
        }
        if p.name == chan {
            return p.values.first();
        }
        if let Some(suf) = chan.strip_prefix(p.name.as_str()) {
            for set in [["x", "y", "z", "w"], ["r", "g", "b", "a"], ["1", "2", "3", "4"], ["u", "v", "w", ""]] {
                if let Some(k) = set.iter().position(|s| !s.is_empty() && *s == suf)
                    && let Some(v) = p.values.get(k)
                {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Scope of one evaluating node.
struct At<'a, 'b> {
    ix: &'b Index<'a>,
    path: &'b str,
    depth: Cell<usize>,
}
impl Scope for At<'_, '_> {
    fn var(&self, name: &str) -> Option<Value> {
        let h = self.ix.hip;
        Some(Value::Num(match name {
            "F" => self.ix.frame.round(),
            "FF" => self.ix.frame,
            "T" => self.ix.time,
            "FPS" => h.fps,
            "FSTART" => h.frange[0],
            "FEND" => h.frange[1],
            "NFRAMES" => h.frange[1] - h.frange[0] + 1.0,
            "OS" => return Some(Value::Str(self.path.rsplit('/').next().unwrap_or("").to_string())),
            _ => return None,
        }))
    }
    fn channel(&self, path: &str, string: bool) -> Result<Value, ExprError> {
        let (node, chan) =
            resolve(self.path, path).ok_or_else(|| ExprError::Unsupported(format!("bad path `{path}`")))?;
        let v = self.ix.value(&node, &chan, self.depth.get(), string)?;
        Ok(if string { Value::Str(v.text()) } else { Value::Num(v.num()) })
    }
}

/// `ch()` path relative to node `from` (`obj/net/node`) -> (`obj/net/other`, channel). `chan` alone = `from`'s own.
fn resolve(from: &str, path: &str) -> Option<(String, String)> {
    let (dir, chan) = match path.rsplit_once('/') {
        Some((d, c)) => (Some(d), c),
        None => (None, path),
    };
    if chan.is_empty() {
        return None;
    }
    let Some(dir) = dir else { return Some((from.to_string(), chan.to_string())) };
    let mut segs: Vec<&str> =
        if dir.starts_with('/') || dir.is_empty() { Vec::new() } else { from.split('/').collect() };
    for s in dir.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop()?;
            }
            s => segs.push(s),
        }
    }
    Some((segs.join("/"), chan.to_string()))
}

/// Every channel of every node: (`/obj/net/node`, channel, value at the saved frame or why not) - oracle / CLI report.
pub fn channel_report(hip: &HipScene) -> Vec<(String, String, Result<Value, String>)> {
    let ix = Index::new(hip);
    let mut out = Vec::new();
    for (i, net) in hip.networks.iter().enumerate() {
        let base = net_path(hip, i);
        for node in &net.nodes {
            let path = format!("{base}/{}", node.name);
            for c in &node.channels {
                out.push((format!("/{path}"), c.name.clone(), ix.channel(&path, c, 0).map_err(|e| e.to_string())));
            }
        }
    }
    out
}

/// The scene with every channel-driven parm component replaced by its evaluated value, plus notes
/// `(network name, text)` for the components that kept the saved value.
pub fn evaluate(hip: &HipScene) -> (HipScene, Vec<(String, String)>) {
    let ix = Index::new(hip);
    let mut out = hip.clone();
    let mut notes = Vec::new();
    for (i, net) in hip.networks.iter().enumerate() {
        let base = net_path(hip, i);
        for (j, node) in net.nodes.iter().enumerate() {
            let path = format!("{base}/{}", node.name);
            for (k, p) in node.parms.iter().enumerate() {
                for (m, v) in p.values.iter().enumerate() {
                    let HipValue::Chan(chan, _) = v else { continue };
                    let Some(c) = node.channels.iter().find(|c| &c.name == chan) else { continue };
                    let val = match ix.channel(&path, c, 0) {
                        Ok(val) => val,
                        Err(e) => {
                            notes.push((net.name.clone(), format!("{}.{chan}: saved value kept ({e})", node.name)));
                            continue;
                        }
                    };
                    let new = match (val, v.plain()) {
                        (Value::Num(x), _) if x.is_finite() => HipValue::Num(x),
                        (Value::Str(s), HipValue::Str(_)) => HipValue::Str(s),
                        (Value::Str(s), _) => HipValue::Num(s.trim().parse().unwrap_or(0.0)),
                        (Value::Num(_), _) => continue,
                    };
                    out.networks[i].nodes[j].parms[k].values[m] = HipValue::Chan(chan.clone(), Box::new(new));
                }
            }
        }
    }
    (out, notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hexe_graph::hip::read_hip;

    #[test]
    fn relative_paths_resolve_like_houdini() {
        let f = "obj/geo1/copy1";
        assert_eq!(resolve(f, "../ctrl/width"), Some(("obj/geo1/ctrl".into(), "width".into())));
        assert_eq!(resolve(f, "f1"), Some((f.into(), "f1".into())));
        assert_eq!(resolve(f, "bend/dirx"), Some(("obj/geo1/copy1/bend".into(), "dirx".into())));
        assert_eq!(resolve(f, "/obj/geo1/box1/sizex"), Some(("obj/geo1/box1".into(), "sizex".into())));
        assert_eq!(resolve(f, "../../../../x/y"), None);
        assert_eq!(resolve(f, "../a/"), None);
    }

    /// Build a two-node network: ctrl (plain spare parms) + a node whose `t` channels reference it.
    fn scene(chn: &str, parm: &str) -> HipScene {
        let mut s = String::new();
        let entry = |name: &str, body: &str| format!("HouNC\x1a{}{name}\0{body}", "0".repeat(28));
        s.push_str(&entry(".start", "tcur 9.9583333333333339\nfps 24\nframe 1 240\nfrange 1 240\n"));
        s.push_str(&entry("obj/box1.init", "type = geo\n"));
        s.push_str(&entry("obj/box1/ctrl.init", "type = null\n"));
        s.push_str(&entry(
            "obj/box1/ctrl.parm",
            "{\nversion 0.8\nwidth\t[ 0\tlocks=0 ]\t(\t10\t)\ndepth\t[ 0\tlocks=0 ]\t(\t2\t)\nlink\t[ 8\tlocks=0 ]\t(\t[ link 0 ] \t)\n}\n",
        ));
        s.push_str(&entry(
            "obj/box1/ctrl.chn",
            "{\n    channel link {\n      segment { length = 0 expr = ch(\\\"tx_loop\\\") }\n    }\n  }\n",
        ));
        s.push_str(&entry("obj/box1/copy1.init", "type = xform\n"));
        s.push_str(&entry("obj/box1/copy1.parm", parm));
        s.push_str(&entry("obj/box1/copy1.chn", chn));
        read_hip(s.as_bytes()).expect("hip")
    }

    #[test]
    fn sibling_references_evaluate_at_the_saved_frame() {
        let parm = "{\nversion 0.8\nt\t[ 8\tlocks=0 ]\t(\t[ tx\t0 ] \t[ ty\t0 ] \t[ tz\t0 ] \t)\n}\n";
        let chn = "{\n    channel tx {\n      segment { length = 0 value = 0.5 0.5 expr = ch(\\\"../ctrl/width\\\")/2+ch(\\\"../ctrl/depth\\\")/2 }\n    }\n    channel ty {\n      segment { length = 0 expr = $F }\n    }\n    channel tz {\n      segment { length = 0 expr = \"1 / __import__('x').y()\" language = python }\n    }\n  }\n";
        let hip = scene(chn, parm);
        let (out, notes) = evaluate(&hip);
        let node = out.networks.iter().flat_map(|n| &n.nodes).find(|n| n.name == "copy1").expect("copy1");
        let t: Vec<&HipValue> = node.parms[0].values.iter().map(HipValue::plain).collect();
        assert!(matches!(t[0], HipValue::Num(x) if (x - 6.0).abs() < 1e-12));
        // saved time 9.9583 s at 24 fps = frame 240
        assert_eq!(t[1], &HipValue::Num(240.0));
        assert_eq!(t[2], &HipValue::Num(0.0), "python channel keeps the saved value");
        assert_eq!(notes.len(), 1 + 1, "{notes:?}");
        assert!(notes.iter().any(|(_, n)| n.contains("copy1.tz") && n.contains("python")));
        // ctrl.link references a missing channel: kept + noted, no panic
        assert!(notes.iter().any(|(_, n)| n.contains("ctrl.link")));
    }

    #[test]
    fn reference_cycles_stop_with_a_note() {
        let parm = "{\nversion 0.8\nt\t[ 8\tlocks=0 ]\t(\t[ tx\t0 ] \t[ ty\t0 ] \t0\t)\n}\n";
        let chn = "{\n    channel tx {\n      segment { length = 0 expr = ch(\\\"ty\\\") + 1 }\n    }\n    channel ty {\n      segment { length = 0 expr = ch(\\\"tx\\\") }\n    }\n  }\n";
        let (_, notes) = evaluate(&scene(chn, parm));
        assert!(notes.iter().any(|(_, n)| n.contains("copy1.tx") && n.contains("deeper")), "{notes:?}");
    }
}
