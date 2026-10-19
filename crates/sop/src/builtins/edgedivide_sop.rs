//! Edge Divide SOP (token `edgedivide`, Houdini 20.5 `edgedivide`): insert points along polygon edges.
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-edgedivide*`; 3x3 grid, box, open box)
//! `group` = an edge group pattern (ORDERED list of oriented edges; the order is the order points are created in, the
//! orientation is the direction they are created along). Empty `group` = passthrough. The selection is `N ∪ A`, N first
//! (`fixtures/sop-edgedivide-groups`, `oracle/houdini/edgedivide/sop_edgedivide_groups.py`):
//! * N (named part): the EDGE groups (only) whose name matches any plain name token (`*` / `?` globs), concatenated in
//!   NAME order (not token order); then, token by token, `!X` appends every mesh edge not in an edge group matching X
//!   (mesh order; X naming no edge group = every edge), `^X` removes the edges of edge groups matching X.
//! * A (ad-hoc part): the Group Create edge pattern (`groupcreate_sop` module docs: `N`, `N-M`, `NeM`, `pA-B-C`, `pA`,
//!   prim group names, edge group names, `!`, `^`), except that `pA-B` pairs which are no mesh edge are kept (they divide
//!   nothing, but make the group non-empty). An id past the end / `:step` / `NvM` empties the whole of A.
//! * A single `*` = every mesh edge in mesh order (prim order, corner order).
//! * Houdini errors ("not a valid group") when the result is empty and no plain name token matched an edge group and no
//!   `!` token was given (`nonexist`, a point group name, `5`, `^mid`, `p0-1 ^p0-1`); `empty` (an existing empty edge
//!   group), `p0-4` (no mesh edge) or `mid ^mid` pass through. `@attr` tokens select nothing (so mostly error), like Houdini.
//! * `numdivs` = n: `max(n-1, 1)` new points at `t = j/(k+1)` from the edge start (1 and 2 both insert the midpoint; 3 inserts thirds).
//! * Default (`sharedpoints` off): every polygon using the edge gets its OWN copy of the new points, created in direction
//!   order, prims in prim order, edges in selection order. `sharedpoints` = one set of points used by all prims. Each
//!   prim inserts them in its own walk direction.
//! * New points: float attributes interpolated linearly, integer / string attributes copied from the edge START point; point groups false.
//! * Output edge groups: every divided edge is removed from every edge group (the sub-edges are not added).
//! * `applytoall`, `connectpoints`, `closepath`, `tolerance`: no effect seen on the probes (ignored).
//!
//! Not modelled (-> `Unsupported`): vertex attributes or vertex groups on the input; a bare `*` (or `!*` / `^*`) mixed
//! with `!` / `^` tokens, a `!` token together with a plain name token, a plain name token after a `^name` token (their
//! Houdini ordering was not pinned). Non-polygon prims pass through untouched. UNVERIFIED: non-manifold edges.
use super::group_sop::{glob_match, prim_verts};
use super::groupcreate_sop::{Edge, EdgeRun, EdgeTable, edge_table, ekey, eval_edge_pattern_with};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
const OP: &str = "edgedivide";
const MAX_DIVS: i32 = 1024;
/// One use of an edge: (prim, walk segment index, the segment's start point).
type Use = (usize, usize, u32);
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("numdivs", ParamValue::I(2)).label("Number of Divisions").range(1.0, 20.0),
        ParamSpec::new("applytoall", ParamValue::B(true)).label("Apply to All"),
        ParamSpec::new("sharedpoints", ParamValue::B(false)).label("Shared Points"),
        ParamSpec::new("connectpoints", ParamValue::B(false)).label("Connect Points"),
        ParamSpec::new("closepath", ParamValue::B(false)).label("Close Path"),
        ParamSpec::new("tolerance", ParamValue::F(1e-5)).label("Tolerance"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(EdgeDivide { specs: spec() })
}
struct EdgeDivide {
    specs: Vec<ParamSpec>,
}
fn bad(m: &str) -> CookError {
    CookError::BadParam(format!("{OP}: {m}"))
}
/// A token body that names groups (not `*`, not an id / `NeM` / `NvM` token, not a `pA-B` path).
fn is_name(body: &str) -> bool {
    let path = body.strip_prefix('p').is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
    body != "*" && !body.starts_with(|c: char| c.is_ascii_digit()) && !path
}
/// The selected edges, ordered + oriented (module docs); `Ok(None)` = Houdini's "not a valid group" (the caller errors).
fn select(d: &Detail, t: &EdgeTable, pat: &str) -> Result<Option<Vec<Edge>>, CookError> {
    let toks: Vec<(char, &str)> = pat
        .split_whitespace()
        .map(|raw| match raw.chars().next() {
            Some(k @ ('!' | '^')) => (k, &raw[1..]),
            _ => (' ', raw),
        })
        .collect();
    if let [(' ', "*")] = toks.as_slice() {
        return Ok(Some(t.list.clone()));
    }
    let unsup = |m: &str| Err(CookError::Unsupported(format!("{OP}: group `{pat}`: {m} is not modelled")));
    let negated = toks.iter().any(|&(k, _)| k != ' ');
    if negated && toks.iter().any(|&(_, b)| b == "*") {
        return unsup("`*` mixed with `!` / `^` tokens");
    }
    if toks.iter().any(|&(k, _)| k == '!') && toks.iter().any(|&(k, b)| k == ' ' && is_name(b)) {
        return unsup("a `!` token together with a group name");
    }
    if let Some(i) = toks.iter().position(|&(k, b)| k == '^' && is_name(b))
        && toks[i..].iter().any(|&(k, b)| k == ' ' && is_name(b))
    {
        return unsup("a group name after a `^name` token");
    }
    let mut groups: Vec<&hexe_geo::EdgeGroup> = d.edge_groups.iter().collect();
    groups.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let edges_of = |glob: &str| -> HashSet<(u32, u32)> {
        groups.iter().filter(|g| glob_match(glob, &g.name)).flat_map(|g| g.edges.iter().map(|&e| ekey(e))).collect()
    };
    // N: plain names (edge groups only, name order), then `!` / `^` token by token
    let mut valid = false;
    let mut run = EdgeRun::default();
    let named =
        |g: &&&hexe_geo::EdgeGroup| toks.iter().any(|&(k, b)| k == ' ' && is_name_or_star(b) && glob_match(b, &g.name));
    for g in groups.iter().filter(named) {
        valid = true;
        g.edges.iter().for_each(|&e| run.push(e));
    }
    for &(k, body) in &toks {
        match k {
            '!' => {
                valid = true;
                let not = edges_of(body);
                t.list.iter().filter(|&&e| !not.contains(&ekey(e))).for_each(|&e| run.push(e));
            }
            '^' if is_name(body) => run.remove(&edges_of(body)),
            _ => {}
        }
    }
    // A: the Group Create edge pattern (loose pairs kept), appended
    for e in eval_edge_pattern_with(d, t, pat, true) {
        run.push(e);
    }
    Ok((valid || !run.list.is_empty()).then_some(run.list))
}
fn is_name_or_star(body: &str) -> bool {
    body == "*" || is_name(body)
}
/// Interpolated / copied element of every point attribute: `(a, b, t)` -> appended to the data.
fn push_between(data: &mut AttrData, a: usize, b: usize, t: f32) {
    fn lerp<const N: usize>(v: &mut Vec<[f32; N]>, a: usize, b: usize, t: f32) {
        let (x, y) = (v.get(a).copied().unwrap_or([0.0; N]), v.get(b).copied().unwrap_or([0.0; N]));
        let mut o = [0.0; N];
        for i in 0..N {
            o[i] = x[i] + (y[i] - x[i]) * t;
        }
        v.push(o);
    }
    match data {
        AttrData::F32(v) => {
            let (x, y) = (v.get(a).copied().unwrap_or(0.0), v.get(b).copied().unwrap_or(0.0));
            v.push(x + (y - x) * t);
        }
        AttrData::V2(v) => lerp(v, a, b, t),
        AttrData::V3(v) => lerp(v, a, b, t),
        AttrData::V4(v) => lerp(v, a, b, t),
        AttrData::I32(v) => {
            let x = v.get(a).copied().unwrap_or(0);
            v.push(x);
        }
        AttrData::Str { idx, .. } => {
            let x = idx.get(a).copied().unwrap_or(-1);
            idx.push(x);
        }
        AttrData::FN { size, data } => {
            let s = usize::try_from(*size).unwrap_or(0);
            for k in 0..s {
                let (x, y) = (data.get(a * s + k).copied().unwrap_or(0.0), data.get(b * s + k).copied().unwrap_or(0.0));
                data.push(x + (y - x) * t);
            }
        }
        AttrData::IN { size, data } => {
            let s = usize::try_from(*size).unwrap_or(0);
            for k in 0..s {
                let x = data.get(a * s + k).copied().unwrap_or(0);
                data.push(x);
            }
        }
    }
}
impl Node<Detail> for EdgeDivide {
    fn type_name(&self) -> &'static str {
        OP
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let pat = p.string("group").ok_or_else(|| bad("`group` missing or not a string"))?;
        let nd = p.int("numdivs").ok_or_else(|| bad("`numdivs` missing or not an int"))?;
        if !(1..=MAX_DIVS).contains(&nd) {
            return Err(bad(&format!("numdivs {nd} outside 1..={MAX_DIVS}")));
        }
        p.toggle("applytoall").ok_or_else(|| bad("`applytoall` missing or not a toggle"))?; // no effect seen
        let shared = p.toggle("sharedpoints").ok_or_else(|| bad("`sharedpoints` missing or not a toggle"))?;
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        if pat.trim().is_empty() {
            return Ok(Arc::clone(d));
        }
        let Some(selected) = select(d, &edge_table(d), pat)? else {
            return Err(bad(&format!("'{pat}' is not a valid group")));
        };
        if d.groups.iter().any(|g| g.class == AttrClass::Vertex) || d.attrs.iter().any(|a| a.class == AttrClass::Vertex)
        {
            return Err(CookError::Unsupported(format!("{OP}: vertex attributes / groups are not modelled")));
        }
        // prim point lists + edges in walk order: (prim, idx, start, end)
        let pts: Vec<Vec<u32>> = d
            .prims
            .iter()
            .map(|pr| d.vertex_point.get(prim_verts(d, pr)).map(<[u32]>::to_vec).unwrap_or_default())
            .collect();
        let is_poly = |i: usize| matches!(d.prims.get(i).map(|q| &q.kind), Some(PrimKind::Poly { .. }));
        let walk = |i: usize| -> Vec<(u32, u32)> {
            let (ps, closed) = (&pts[i], matches!(d.prims[i].kind, PrimKind::Poly { closed: true }));
            let n = ps.len();
            let segs = if closed && n > 2 { n } else { n.saturating_sub(1) };
            (0..segs).map(|k| (ps[k], ps[(k + 1) % n])).collect()
        };
        let key = |a: u32, b: u32| (a.min(b), a.max(b));
        // edge key -> its uses (prim, segment index, start point), prim order then walk order
        let mut uses: BTreeMap<(u32, u32), Vec<Use>> = BTreeMap::new();
        for i in (0..d.prims.len()).filter(|&i| is_poly(i)) {
            for (k, (a, b)) in walk(i).into_iter().enumerate() {
                if a != b {
                    uses.entry(key(a, b)).or_default().push((i, k, a));
                }
            }
        }
        let seq: Vec<(u32, u32)> =
            selected.iter().map(|&[a, b]| (a, b)).filter(|&(a, b)| uses.contains_key(&key(a, b))).collect();
        if seq.is_empty() {
            return Ok(Arc::clone(d));
        }
        let k_new = usize::try_from((nd - 1).max(1)).unwrap_or(1);
        let mut out = (**d).clone();
        let mut ins: Vec<BTreeMap<usize, Vec<u32>>> = vec![BTreeMap::new(); d.prims.len()];
        let make_points = |out: &mut Detail, a: u32, b: u32| -> Result<Vec<u32>, CookError> {
            let mut ids = Vec::with_capacity(k_new);
            let (au, bu) = (a as usize, b as usize);
            for j in 1..=k_new {
                let t = j as f32 / (k_new + 1) as f32;
                let id = out.point_count;
                out.point_count = id.checked_add(1).ok_or_else(|| bad("point count overflow"))?;
                for at in out.attrs.iter_mut().filter(|x| x.class == AttrClass::Point) {
                    push_between(&mut at.data, au, bu, t);
                }
                for g in out.groups.iter_mut().filter(|g| g.class == AttrClass::Point) {
                    g.mask.push(false);
                }
                ids.push(id);
            }
            Ok(ids)
        };
        for &(a, b) in &seq {
            let users = uses.get(&key(a, b)).map(Vec::as_slice).unwrap_or_default();
            let shared_ids = if shared { Some(make_points(&mut out, a, b)?) } else { None };
            for &(i, k, x) in users {
                let fwd = x == a;
                let mut ids = match &shared_ids {
                    Some(s) => s.clone(),
                    None => make_points(&mut out, a, b)?,
                };
                if !fwd {
                    ids.reverse();
                }
                ins[i].insert(k, ids);
            }
        }
        // rebuild vertex storage
        let (mut vp, mut prims) = (Vec::new(), Vec::new());
        for (i, pr) in d.prims.iter().enumerate() {
            let start = u32::try_from(vp.len()).map_err(|_| bad("vertex count overflow"))?;
            for (k, &pt) in pts[i].iter().enumerate() {
                vp.push(pt);
                if let Some(ids) = ins[i].get(&k) {
                    vp.extend_from_slice(ids);
                }
            }
            let end = u32::try_from(vp.len()).map_err(|_| bad("vertex count overflow"))?;
            let mut q = pr.clone();
            q.verts = start..end;
            prims.push(q);
        }
        out.vertex_point = vp;
        out.prims = prims;
        // a divided edge no longer exists: drop it from every edge group
        let gone: HashSet<(u32, u32)> = seq.iter().map(|&(a, b)| key(a, b)).collect();
        for g in &mut out.edge_groups {
            g.edges.retain(|&e| !gone.contains(&ekey(e)));
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::{run, s};
    use super::*;
    fn g3() -> Arc<Detail> {
        run("grid", &[("rows", ParamValue::I(3)), ("cols", ParamValue::I(3))], None).expect("grid")
    }
    fn ed(ov: &[(&str, ParamValue)]) -> Arc<Detail> {
        run("edgedivide", ov, Some(&g3())).expect("cooks")
    }
    fn prim(d: &Detail, i: usize) -> Vec<u32> {
        d.vertex_point[d.prims[i].verts.start as usize..d.prims[i].verts.end as usize].to_vec()
    }
    #[test]
    fn explicit_edge_gives_each_prim_its_own_midpoint() {
        let o = ed(&[("group", s("p1-4"))]);
        assert_eq!(o.point_count, 11);
        assert_eq!(prim(&o, 0), [0, 1, 9, 4, 3]);
        assert_eq!(prim(&o, 1), [1, 2, 5, 4, 10]);
        assert_eq!(o.p()[9], o.p()[10]);
        assert_eq!(o.p()[9], [0.0, 0.0, -2.5]);
    }
    #[test]
    fn divisions_direction_shared_and_boundary() {
        let o = ed(&[("group", s("p4-1")), ("numdivs", ParamValue::I(3))]);
        assert_eq!(prim(&o, 0), [0, 1, 10, 9, 4, 3]);
        assert_eq!(prim(&o, 1), [1, 2, 5, 4, 11, 12]);
        assert!((o.p()[9][2] + 1.667).abs() < 1e-3 && (o.p()[10][2] + 3.333).abs() < 1e-3);
        let sh = ed(&[("group", s("p1-4")), ("sharedpoints", ParamValue::B(true))]);
        assert_eq!(sh.point_count, 10);
        assert_eq!(prim(&sh, 1), [1, 2, 5, 4, 9]);
        let two = ed(&[("group", s("p1-4 p3-4"))]);
        assert_eq!(prim(&two, 0), [0, 1, 9, 4, 11, 3]);
        assert_eq!(prim(&two, 2), [3, 12, 4, 7, 6]);
        assert_eq!(ed(&[("group", s("p0-1"))]).point_count, 10);
    }
    /// hython: `*` = every edge in mesh order; empty group / a pair that is no mesh edge = passthrough; a token naming no
    /// existing point is "not a valid group" (`applytoall` changes nothing).
    #[test]
    fn all_edges_and_passthrough() {
        let all = ed(&[("group", s("*"))]);
        assert_eq!(all.point_count, 25);
        assert_eq!(prim(&all, 0), [0, 9, 1, 10, 4, 12, 3, 14]);
        assert_eq!(prim(&all, 1), [1, 15, 2, 16, 5, 17, 4, 11]);
        assert_eq!(*ed(&[]), *g3());
        assert_eq!(*ed(&[("group", s("p0-4"))]), *g3());
        let r = |ov: &[(&str, ParamValue)]| run("edgedivide", ov, Some(&g3()));
        assert!(matches!(r(&[("group", s("p90-91"))]), Err(CookError::BadParam(_))));
        assert!(matches!(
            r(&[("group", s("p90-91")), ("applytoall", ParamValue::B(false))]),
            Err(CookError::BadParam(_))
        ));
    }
    #[test]
    fn attributes_interpolate_ints_copy() {
        let w = run("wrangle", &[("snippet", s("f@h = @ptnum; i@k = @ptnum;"))], Some(&g3())).expect("wrangle");
        let o = run("edgedivide", &[("group", s("p1-4")), ("numdivs", ParamValue::I(3))], Some(&w)).expect("cooks");
        let h = match &o.attr(AttrClass::Point, "h").expect("h").data {
            AttrData::F32(v) => v.clone(),
            _ => panic!("h type"),
        };
        assert_eq!(&h[9..], [2.0, 3.0, 2.0, 3.0]);
        let k = match &o.attr(AttrClass::Point, "k").expect("k").data {
            AttrData::I32(v) => v.clone(),
            _ => panic!("k type"),
        };
        assert_eq!(&k[9..], [1, 1, 1, 1]);
    }
    #[test]
    fn hostile_params() {
        let r = |ov: &[(&str, ParamValue)]| run("edgedivide", ov, Some(&g3()));
        assert!(r(&[("group", s("0-3"))]).is_ok_and(|o| o.point_count == 25), "prims 0-3 = every edge");
        assert!(r(&[("group", s("e1"))]).is_err());
        assert!(r(&[("numdivs", ParamValue::I(0))]).is_err());
        assert!(r(&[("numdivs", ParamValue::I(100000))]).is_err());
        assert_eq!(run("edgedivide", &[], None).expect("empty").point_count, 0);
    }
    /// Real Houdini 20.5 twins (3x3 grid): explicit edge / thirds + reversed direction / shared / two edges / boundary. All-edges mode is pinned from the hython probe (empty edge GROUP + applytoall), not a twin: a literal nonexistent token errors in Houdini.
    #[test]
    fn edgedivide_matches_hython() {
        let cases: [(&str, &[u8]); 5] = [
            (
                include_str!("../../../../fixtures/sop-edgedivide/scene.json"),
                include_bytes!("../../../../fixtures/sop-edgedivide/houdini.geo"),
            ),
            (
                include_str!("../../../../fixtures/sop-edgedivide-rev3/scene.json"),
                include_bytes!("../../../../fixtures/sop-edgedivide-rev3/houdini.geo"),
            ),
            (
                include_str!("../../../../fixtures/sop-edgedivide-shared/scene.json"),
                include_bytes!("../../../../fixtures/sop-edgedivide-shared/houdini.geo"),
            ),
            (
                include_str!("../../../../fixtures/sop-edgedivide-two/scene.json"),
                include_bytes!("../../../../fixtures/sop-edgedivide-two/houdini.geo"),
            ),
            (
                include_str!("../../../../fixtures/sop-edgedivide-bound/scene.json"),
                include_bytes!("../../../../fixtures/sop-edgedivide-bound/houdini.geo"),
            ),
        ];
        for (scene, geo) in cases {
            super::super::meshutil::testkit::houdini_twin(scene, geo).expect("hexe == Houdini");
            super::super::meshutil::testkit::houdini_values(scene, geo, 1e-4).expect("values == Houdini");
        }
    }

    // ---- `group` syntax vs Houdini 20.5 (fixtures/sop-edgedivide-groups, made by
    // oracle/houdini/edgedivide/sop_edgedivide_groups.py from synthetic grid / box / open box + Group Create groups) ----
    fn fx(file: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sop-edgedivide-groups").join(file)
    }
    fn load(file: &str) -> Arc<Detail> {
        Arc::new(hexe_geo::read_geo(&std::fs::read(fx(file)).expect("fixture")).expect("parses"))
    }
    /// Every hython case: same error-or-not, point count, prim point lists (exact), new point positions (1e-5) and output
    /// edge groups (ordered). Patterns hexe declines (`Unsupported`) must not be in the fixture.
    #[test]
    fn group_syntax_matches_houdini() {
        let text = std::fs::read_to_string(fx("cases.json")).expect("cases.json");
        let cases: Vec<serde_json::Value> = serde_json::from_str(&text).expect("json");
        assert!(cases.len() > 120, "{}", cases.len());
        let mut inputs: std::collections::HashMap<String, Arc<Detail>> = std::collections::HashMap::new();
        let mut fails = Vec::new();
        for c in &cases {
            let src = c["input"].as_str().expect("input").to_string();
            let d = Arc::clone(inputs.entry(src.clone()).or_insert_with(|| load(&format!("{src}_in.geo"))));
            let pat = c["group"].as_str().expect("group");
            let nd = i32::try_from(c["numdivs"].as_i64().expect("numdivs")).expect("i32");
            let sh = c["sharedpoints"].as_bool().expect("sharedpoints");
            let ov = [("group", s(pat)), ("numdivs", ParamValue::I(nd)), ("sharedpoints", ParamValue::B(sh))];
            let tag = format!("{src} {pat:?} numdivs {nd} shared {sh}");
            let got = run("edgedivide", &ov, Some(&d));
            if c["error"].as_bool() == Some(true) {
                if !matches!(got, Err(CookError::BadParam(_))) {
                    fails.push(format!("{tag}: Houdini errors, hexe {:?}", got.map(|o| o.point_count)));
                }
                continue;
            }
            let o = match got {
                Ok(o) => o,
                Err(e) => {
                    fails.push(format!("{tag}: hexe error {e:?}"));
                    continue;
                }
            };
            let prims: Vec<Vec<u32>> = (0..o.prims.len()).map(|i| prim(&o, i)).collect();
            let want_prims: Vec<Vec<u32>> = serde_json::from_value(c["prims"].clone()).expect("prims");
            let want_n = u32::try_from(c["points"].as_u64().expect("points")).expect("u32");
            if o.point_count != want_n || prims != want_prims {
                fails.push(format!("{tag}: hexe {} {prims:?} != houdini {want_n} {want_prims:?}", o.point_count));
                continue;
            }
            let want_p: Vec<[f32; 3]> = serde_json::from_value(c["new_P"].clone()).expect("new_P");
            let got_p = &o.p()[d.point_count as usize..];
            let close = got_p.len() == want_p.len()
                && got_p.iter().zip(&want_p).all(|(a, b)| (0..3).all(|k| (a[k] - b[k]).abs() <= 1e-5));
            if !close {
                fails.push(format!("{tag}: new P {got_p:?} != {want_p:?}"));
            }
            let want_eg: std::collections::BTreeMap<String, Vec<[u32; 2]>> =
                serde_json::from_value(c["edge_groups"].clone()).expect("edge_groups");
            let got_eg: std::collections::BTreeMap<String, Vec<[u32; 2]>> =
                o.edge_groups.iter().map(|g| (g.name.clone(), g.edges.clone())).collect();
            if got_eg != want_eg {
                fails.push(format!("{tag}: edge groups {got_eg:?} != {want_eg:?}"));
            }
        }
        assert!(fails.is_empty(), "{} of {} cases differ:\n{}", fails.len(), cases.len(), fails.join("\n"));
    }
    /// Hostile / unmodelled group strings: an error or a result, never a panic; unpinned mixes are `Unsupported`.
    #[test]
    fn hostile_group_strings_never_panic() {
        let g = load("grid3_in.geo");
        let pats = [
            "p",
            "p-",
            "p--",
            "p0-",
            "p99999999999999999999",
            "p4294967295-0",
            "p0-4294967295",
            "0e",
            "e0",
            "0e99999999999999999999",
            "0v",
            "99999999999999999999",
            "0-99999999999999999999",
            "!",
            "^",
            "!!0",
            "^^0",
            "!^mid",
            "^!mid",
            "*v*",
            "**",
            "***?",
            "\u{0}",
            "p0-1-2-5-4-3-0-1-2",
            ":",
            "-",
            "0--1",
            "@P.x>0",
            "@",
            "mid ^",
            "! ^ *",
            "[mid]",
            "m\u{301}id",
            "é ^é !é",
            "p1-4 p1-4 p4-1",
        ];
        for pat in pats {
            for sh in [false, true] {
                let ov = [("group", s(pat)), ("sharedpoints", ParamValue::B(sh)), ("numdivs", ParamValue::I(20))];
                let _ = run("edgedivide", &ov, Some(&g));
            }
        }
        for pat in ["* ^mid", "!* *", "^*", "!mid zeta", "mid ^zeta alpha"] {
            assert!(
                matches!(run("edgedivide", &[("group", s(pat))], Some(&g)), Err(CookError::Unsupported(_))),
                "{pat}"
            );
        }
        // edge groups naming points / prims that do not exist, a prim range past the vertex list: no panic
        let mut broken = (*g).clone();
        broken.set_edge_group("bad", [[0, 4_000_000_000], [7, 7], [u32::MAX, 0]]);
        let broken = Arc::new(broken);
        for pat in ["bad", "!bad", "bad p1-4", "*"] {
            let _ = run("edgedivide", &[("group", s(pat))], Some(&broken));
        }
        let mut cut = (*g).clone();
        cut.vertex_point.truncate(5);
        let _ = run("edgedivide", &[("group", s("*"))], Some(&Arc::new(cut)));
    }
}
