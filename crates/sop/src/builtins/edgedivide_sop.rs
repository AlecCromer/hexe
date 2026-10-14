//! Edge Divide SOP (token `edgedivide`, Houdini 20.5 `edgedivide`): insert points along polygon edges.
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-edgedivide*`; 3x3 point grid)
//! `group` = edge tokens `pA-B` (point pair; the written ORDER is the edge's direction). Empty `group` = passthrough. Tokens naming no existing polygon
//! edge select nothing; with `applytoall` (default on) an empty selection means EVERY polygon edge (direction = first walk that meets the edge).
//! * `numdivs` = n: `max(n-1, 1)` new points at `t = j/(k+1)` from the edge start (1 and 2 both insert the midpoint; 3 inserts thirds).
//! * Default (`sharedpoints` off): every polygon using the edge gets its OWN copy of the new points, created in direction order, prims in
//!   prim order, edges in token order (all-edge mode: prim order, walk order, a shared edge creates its copies when first met). `sharedpoints`
//!   = one set of points used by all prims. Each prim inserts them in its own walk direction.
//! * New points: float attributes interpolated linearly, integer / string attributes copied from the edge START point; point groups false.
//! * `connectpoints`, `closepath`, `tolerance`: no effect seen on the probes (ignored). Edge-group OUTPUT is not modelled.
//!
//! Not modelled (-> `Unsupported`): vertex attributes or vertex groups on the input, non-`pA-B` tokens (edge group names, ranges).
//! Non-polygon prims pass through untouched. UNVERIFIED: `applytoall` off with an empty selection, non-manifold edges.
use super::group_sop::prim_verts;
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::BTreeMap;
use std::sync::Arc;
const OP: &str = "edgedivide";
const MAX_DIVS: i32 = 1024;
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
fn parse_edges(pat: &str) -> Result<Vec<(u32, u32)>, CookError> {
    let mut v = Vec::new();
    for t in pat.split_whitespace() {
        let pr = || CookError::Unsupported(format!("{OP}: group token `{t}`: only edge tokens `pA-B` are modelled"));
        let body = t.strip_prefix('p').ok_or_else(pr)?;
        let (a, b) = body.split_once('-').ok_or_else(pr)?;
        v.push((a.parse::<u32>().map_err(|_| pr())?, b.parse::<u32>().map_err(|_| pr())?));
    }
    Ok(v)
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
        let all_on = p.toggle("applytoall").ok_or_else(|| bad("`applytoall` missing or not a toggle"))?;
        let shared = p.toggle("sharedpoints").ok_or_else(|| bad("`sharedpoints` missing or not a toggle"))?;
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let toks = parse_edges(pat)?;
        if pat.trim().is_empty() {
            return Ok(Arc::clone(d));
        }
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
            let segs = if closed { n } else { n.saturating_sub(1) };
            (0..segs).map(|k| (ps[k], ps[(k + 1) % n])).collect()
        };
        let key = |a: u32, b: u32| (a.min(b), a.max(b));
        let mut present: BTreeMap<(u32, u32), ()> = BTreeMap::new();
        for i in (0..d.prims.len()).filter(|&i| is_poly(i)) {
            for (a, b) in walk(i) {
                present.insert(key(a, b), ());
            }
        }
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for &(a, b) in &toks {
            if a != b && present.contains_key(&key(a, b)) && !edges.iter().any(|&(x, y)| key(x, y) == key(a, b)) {
                edges.push((a, b));
            }
        }
        if edges.is_empty() {
            if !all_on {
                return Ok(Arc::clone(d));
            }
            for i in (0..d.prims.len()).filter(|&i| is_poly(i)) {
                for (a, b) in walk(i) {
                    if a != b && !edges.iter().any(|&(x, y)| key(x, y) == key(a, b)) {
                        edges.push((a, b));
                    }
                }
            }
        }
        let k_new = usize::try_from((nd - 1).max(1)).unwrap_or(1);
        let mut out = (**d).clone();
        let mut ins: Vec<BTreeMap<usize, Vec<u32>>> = vec![BTreeMap::new(); d.prims.len()];
        // in all-edge mode points are created when the edge is first met (prim order); explicit mode: token order
        let order: Vec<(u32, u32)> = edges;
        let all_mode = toks.is_empty() || !order.iter().any(|&(a, b)| toks.contains(&(a, b)));
        let mut seq: Vec<(u32, u32)> = order.clone();
        if all_mode {
            seq.clear();
            for i in (0..d.prims.len()).filter(|&i| is_poly(i)) {
                for (a, b) in walk(i) {
                    if let Some(&e) = order.iter().find(|&&(x, y)| key(x, y) == key(a, b))
                        && !seq.iter().any(|&(x, y)| key(x, y) == key(e.0, e.1))
                    {
                        seq.push(if e == (a, b) || e == (b, a) { e } else { (a, b) });
                    }
                }
            }
        }
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
            let users: Vec<(usize, usize, bool)> = (0..d.prims.len())
                .filter(|&i| is_poly(i))
                .flat_map(|i| {
                    walk(i)
                        .into_iter()
                        .enumerate()
                        .filter(|&(_, (x, y))| key(x, y) == key(a, b))
                        .map(move |(k, (x, _))| (i, k, x == a))
                })
                .collect();
            let shared_ids = if shared { Some(make_points(&mut out, a, b)?) } else { None };
            for (i, k, fwd) in users {
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
    #[test]
    fn all_edges_and_passthrough() {
        let all = ed(&[("group", s("p90-91"))]);
        assert_eq!(all.point_count, 25);
        assert_eq!(prim(&all, 0), [0, 9, 1, 10, 4, 12, 3, 14]);
        assert_eq!(prim(&all, 1), [1, 15, 2, 16, 5, 17, 4, 11]);
        assert_eq!(*ed(&[]), *g3());
        assert_eq!(*ed(&[("group", s("p90-91")), ("applytoall", ParamValue::B(false))]), *g3());
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
        assert!(r(&[("group", s("0-3"))]).is_err());
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
}
