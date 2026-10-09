//! Copy to Points SOP (token `copytopoints`): one copy of input 0 (source) per POINT of input 1 (targets).
//!
//! # Semantics (doc-derived, UNVERIFIED vs Houdini: hython blocked)
//! Copy `i` (target point `i`, in point order) = the whole source with every position mapped
//! `p' = P_i + R_i * (s_i * p)`; output = copies concatenated in target order (copy 0 first), so
//! `points = src.points * n`, `prims = src.prims * n`, `vertices = src.vertices * n`; point `k` of copy `i` sits at
//! output id `i * src.points + k`. Copy topology is the source's, point ids offset by `i * src.points`.
//! * `s_i` = target point attr `pscale` (F32) when `use_pscale` (default on) and the attr exists, else 1. Uniform, applied
//!   about the SOURCE origin (so a source centred on the origin is scaled about `P_i`). Not clamped: 0 collapses, negative flips.
//! * `R_i` only when `use_orient` (default OFF) and the target has a `N` (V3) point attr: frame with +Z = `N`, +Y = target
//!   `up` (V3, default world +Y) made perpendicular to `N`: `x = up x n`, `y = n x x`, columns `(x, y, n)`. `N` ~ 0 or
//!   non-finite → identity. `up` ~ 0 / parallel to `N` → fallback up = world +Y, or world +X when `N` is parallel to Y.
//!   `orient` (quaternion) and `scale` target attrs are NOT read.
//! * Source `N` (point and vertex class, V3) is rotated by `R_i` (length kept; `pscale` is uniform) and never translated.
//! * All other source attrs / groups are repeated per copy (point, vertex, prim: same values; groups: masks repeated;
//!   detail attrs: kept once). NO target attrs are transferred (`targetattribs` multiparm, `transform` and `pack`
//!   toggles, source/target group filters are NOT implemented: the v2 parm table in `spec/harvest/parms-copytopoints.md`
//!   is read, only the three params below are declared).
//! * `copynum` (default off): adds I32 `copynum` = copy index `i` on points AND prims (replacing a same-name attr).
//!
//! QA additions (OUR names): `sourcegroup` (prim group of input 0 copied; unknown = Failed), `targetgroup` (point group of input 1;
//! `copynum` counts EMITTED copies), `pieceattrib` (source prim attr == target point attr (int/string): that point gets only the
//! matching source prims), `targetattribs` (globs; target point attrs copied onto each copy's points AND prims), target `orient`
//! (V4 xyzw quaternion, wins over N/up when `use_orient`) and `scale` (V3, times pscale; source N gets the inverse-scale),
//! target `N` falls back to the vertex-class `N` of each point's first vertex. NOT done: `pack`, implicit N without `use_orient`.
//!
//! Empty source, empty / missing targets → empty output. Hostile input: non-finite target `P` / `pscale` / `N` / `up`, an
//! f32-overflowing result, or an output above `MAX_ELEMS` points / vertices / prims → `CookError::Failed`.
use super::meshutil::{Remap, gather, rebuild, selection};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

/// Output allocation cap per element class (AGENTS.md §4).
const MAX_ELEMS: u64 = 1 << 24;

const INPUTS: [InputSpec; 2] = [InputSpec { name: "input0" }, InputSpec { name: "input1" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("use_orient", ParamValue::B(false)).label("Orient by N / up"),
        ParamSpec::new("use_pscale", ParamValue::B(true)).label("Scale by pscale"),
        ParamSpec::new("copynum", ParamValue::B(false)).label("Add copynum Attribute"),
        ParamSpec::new("sourcegroup", ParamValue::S(String::new())).label("Source Group"),
        ParamSpec::new("targetgroup", ParamValue::S(String::new())).label("Target Group"),
        ParamSpec::new("pieceattrib", ParamValue::S(String::new())).label("Piece Attribute"),
        ParamSpec::new("targetattribs", ParamValue::S(String::new())).label("Target Attributes"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(CopyToPoints { specs: spec() })
}

struct CopyToPoints {
    specs: Vec<ParamSpec>,
}

type V3 = [f64; 3];

fn toggle(p: &ParamSet, name: &str) -> Result<bool, CookError> {
    p.toggle(name).ok_or_else(|| CookError::BadParam(format!("`{name}`: missing or not a toggle")))
}

fn failed(m: &str) -> CookError {
    CookError::Failed(format!("copytopoints: {m}"))
}

fn d3(v: [f32; 3]) -> V3 {
    [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: V3) -> Option<V3> {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    (l.is_finite() && l > 1e-12).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

/// Rotation columns `(x, y, z)` with `z = n`, `y` from `up` (see module doc); `None` when `n` is degenerate.
fn frame(n: V3, up: Option<V3>) -> Option<[V3; 3]> {
    let n = norm(n)?;
    let x_of = |u: V3| norm(cross(u, n));
    let x = up.and_then(norm).and_then(x_of).or_else(|| x_of([0.0, 1.0, 0.0])).or_else(|| x_of([1.0, 0.0, 0.0]))?;
    Some([x, cross(n, x), n])
}

/// Per-copy transform: rotation columns (identity when not oriented), per-axis scale, translation.
struct Xf {
    rot: [V3; 3],
    s: V3,
    t: V3,
}
impl Xf {
    fn rotate(&self, v: V3) -> V3 {
        let [x, y, z] = self.rot;
        [0, 1, 2].map(|k| x[k] * v[0] + y[k] * v[1] + z[k] * v[2])
    }
    fn point(&self, p: [f32; 3]) -> Result<[f32; 3], CookError> {
        let q = d3(p);
        let r = self.rotate([q[0] * self.s[0], q[1] * self.s[1], q[2] * self.s[2]]);
        let o = [0, 1, 2].map(|k| (r[k] + self.t[k]) as f32);
        if o.iter().all(|c| c.is_finite()) { Ok(o) } else { Err(failed("copied position overflows f32")) }
    }
    /// Normal: inverse-scale (non-uniform scale), restore the input length, rotate; never translated.
    fn normal(&self, n: [f32; 3]) -> [f32; 3] {
        let v = d3(n);
        let l0 = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let w = [0, 1, 2].map(|k| if self.s[k].abs() > 1e-12 { v[k] / self.s[k] } else { v[k] });
        let l1 = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
        let w = if l1 > 1e-12 && l0.is_finite() { w.map(|c| c * l0 / l1) } else { v };
        let r = self.rotate(w);
        [r[0] as f32, r[1] as f32, r[2] as f32]
    }
}
/// Rotation columns of the unit quaternion `(x, y, z, w)`; `None` when degenerate.
fn quat_frame(q: [f32; 4]) -> Option<[V3; 3]> {
    let [x, y, z, w] = q.map(f64::from);
    let l = (x * x + y * y + z * z + w * w).sqrt();
    if !l.is_finite() || l < 1e-12 {
        return None;
    }
    let (x, y, z, w) = (x / l, y / l, z / l, w / l);
    Some([
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)],
        [2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)],
        [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)],
    ])
}
/// Output sizes must stay addressable and under the cap.
fn check_size(src: &Detail, n: usize) -> Result<(), CookError> {
    let n = n as u64;
    for per in [u64::from(src.point_count), u64::from(src.vertex_count()), u64::from(src.prim_count())] {
        match per.checked_mul(n) {
            Some(t) if t <= MAX_ELEMS => {}
            _ => return Err(failed("output element count exceeds the allocation cap")),
        }
    }
    Ok(())
}
fn point_v3<'a>(t: &'a Detail, name: &str) -> Option<&'a [[f32; 3]]> {
    match t.attr(AttrClass::Point, name).map(|a| &a.data) {
        Some(AttrData::V3(v)) => Some(v),
        _ => None,
    }
}
/// Target `N`: the point attr, else the vertex attr of the FIRST vertex using each point (zero for unused points).
fn target_n(t: &Detail) -> Option<Vec<[f32; 3]>> {
    if let Some(v) = point_v3(t, "N") {
        return Some(v.to_vec());
    }
    let Some(AttrData::V3(vn)) = t.attr(AttrClass::Vertex, "N").map(|a| &a.data) else { return None };
    let mut out = vec![[0.0f32; 3]; t.point_count as usize];
    let mut seen = vec![false; t.point_count as usize];
    for (vi, &pt) in t.vertex_point.iter().enumerate() {
        let pt = pt as usize;
        if let (Some(s), Some(o), Some(n)) = (seen.get_mut(pt), out.get_mut(pt), vn.get(vi))
            && !*s
        {
            *s = true;
            *o = *n;
        }
    }
    Some(out)
}
fn finite(v: [f32; 3], what: &str) -> Result<[f32; 3], CookError> {
    if v.iter().all(|c| c.is_finite()) { Ok(v) } else { Err(failed(&format!("non-finite target `{what}`"))) }
}
/// `*`-glob match (only `*` is special).
fn glob(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !s.starts_with(first) || s.len() < first.len() + last.len() || !s.ends_with(last) {
        return false;
    }
    let mut at = first.len();
    let end = s.len() - last.len();
    for mid in &parts[1..parts.len() - 1] {
        match s.get(at..end).and_then(|t| t.find(mid)) {
            Some(i) => at += i + mid.len(),
            None => return false,
        }
    }
    true
}
/// Source restricted to the prims with `keep[i]` (points compacted in original order; attrs / groups follow).
pub(super) fn subset(src: &Detail, keep: &[bool]) -> Result<Detail, CookError> {
    let mut used = vec![false; src.point_count as usize];
    let (mut vertex_point, mut prims) = (Vec::new(), Vec::new());
    let (mut vert_src, mut prim_src) = (Vec::new(), Vec::new());
    for (pi, pr) in src.prims.iter().enumerate() {
        if !keep.get(pi).copied().unwrap_or(false) {
            continue;
        }
        let lo = vertex_point.len();
        for vi in pr.verts.start as usize..pr.verts.end as usize {
            if let Some(&pt) = src.vertex_point.get(vi) {
                if let Some(u) = used.get_mut(pt as usize) {
                    *u = true;
                }
                vertex_point.push(pt);
                vert_src.push(Some(vi));
            }
        }
        let mut np = pr.clone();
        np.verts = u32::try_from(lo).map_err(|_| failed("vertex overflow"))?
            ..u32::try_from(vertex_point.len()).map_err(|_| failed("vertex overflow"))?;
        prims.push(np);
        prim_src.push(Some(pi));
    }
    let mut new_id = vec![u32::MAX; used.len()];
    let mut point_src: Vec<Vec<usize>> = Vec::new();
    for (i, u) in used.iter().enumerate() {
        if *u {
            new_id[i] = u32::try_from(point_src.len()).map_err(|_| failed("point overflow"))?;
            point_src.push(vec![i]);
        }
    }
    for v in &mut vertex_point {
        *v = new_id.get(*v as usize).copied().unwrap_or(0);
    }
    let p: Vec<[f32; 3]> = point_src.iter().map(|s| src.p().get(s[0]).copied().unwrap_or([0.0; 3])).collect();
    rebuild("copytopoints", src, vertex_point, prims, &Remap { point_src, vert_src, prim_src }, p)
}
/// Piece key of one element of an int / string attribute.
fn piece_key(d: &AttrData, i: usize) -> Option<String> {
    match d {
        AttrData::I32(v) => v.get(i).map(|x| x.to_string()),
        AttrData::Str { idx, table } => {
            idx.get(i).and_then(|&k| usize::try_from(k).ok()).and_then(|k| table.get(k)).cloned()
        }
        _ => None,
    }
}
/// Point-group mask (union of space-separated names); `None` = all.
fn point_mask(d: &Detail, names: &str) -> Result<Option<Vec<bool>>, CookError> {
    let names: Vec<&str> = names.split_whitespace().collect();
    if names.is_empty() {
        return Ok(None);
    }
    let mut m = vec![false; d.point_count as usize];
    for n in names {
        let g = d.group(AttrClass::Point, n).ok_or_else(|| failed(&format!("unknown target point group `{n}`")))?;
        for (s, b) in m.iter_mut().zip(&g.mask) {
            *s |= *b;
        }
    }
    Ok(Some(m))
}
impl Node<Detail> for CopyToPoints {
    fn type_name(&self) -> &'static str {
        "copytopoints"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let use_orient = toggle(ctx.params, "use_orient")?;
        let use_pscale = toggle(ctx.params, "use_pscale")?;
        let copynum = toggle(ctx.params, "copynum")?;
        let sgroup = ctx.params.string("sourcegroup").unwrap_or("");
        let tgroup = ctx.params.string("targetgroup").unwrap_or("");
        let piece = ctx.params.string("pieceattrib").unwrap_or("");
        let tattribs = ctx.params.string("targetattribs").unwrap_or("");
        let (Some(src0), Some(tgt)) = (inputs.first(), inputs.get(1)) else {
            return Ok(Arc::new(Detail::default()));
        };
        let targets = tgt.p();
        if (src0.point_count == 0 && src0.prims.is_empty()) || targets.is_empty() {
            return Ok(Arc::new(Detail::default()));
        }
        // source prim group
        let src_sel = selection("copytopoints", src0, sgroup).map_err(|e| match e {
            CookError::Failed(m) => CookError::Failed(m),
            o => o,
        })?;
        let src_base: Arc<Detail> = match &src_sel {
            Some(keep) => Arc::new(subset(src0, keep)?),
            None => Arc::clone(src0),
        };
        let tmask = point_mask(tgt, tgroup)?;
        // piece attribute: per source-base prim key, per target point key
        let piece_src =
            (!piece.is_empty()).then(|| src_base.attr(AttrClass::Prim, piece).map(|a| a.data.clone())).flatten();
        let piece_tgt =
            (!piece.is_empty()).then(|| tgt.attr(AttrClass::Point, piece).map(|a| a.data.clone())).flatten();
        check_size(&src_base, targets.len())?;
        let pscale: Option<&[f32]> = match tgt.attr(AttrClass::Point, "pscale").map(|a| &a.data) {
            Some(AttrData::F32(v)) if use_pscale => Some(v),
            _ => None,
        };
        let scale_a: Option<&[[f32; 3]]> = if use_pscale { point_v3(tgt, "scale") } else { None };
        let orient_a: Option<&[[f32; 4]]> = match tgt.attr(AttrClass::Point, "orient").map(|a| &a.data) {
            Some(AttrData::V4(v)) if use_orient => Some(v),
            _ => None,
        };
        let tn = if use_orient { target_n(tgt) } else { None };
        let up = if use_orient { point_v3(tgt, "up") } else { None };
        let ident = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut out = Detail::default();
        let mut emitted: Vec<(usize, usize, usize)> = Vec::new(); // (target point, points, prims)
        let mut piece_cache: std::collections::HashMap<String, Arc<Detail>> = std::collections::HashMap::new();
        for (i, p) in targets.iter().enumerate() {
            if tmask.as_ref().is_some_and(|m| !m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            let p = finite(*p, "P")?;
            let mut s = match pscale.and_then(|v| v.get(i)) {
                Some(s) if s.is_finite() => f64::from(*s),
                Some(_) => return Err(failed("non-finite target `pscale`")),
                None => 1.0,
            };
            if s.is_nan() {
                s = 1.0;
            }
            let sv = match scale_a.and_then(|v| v.get(i)) {
                Some(v) => d3(finite(*v, "scale")?).map(|c| c * s),
                None => [s; 3],
            };
            let rot = match orient_a.and_then(|v| v.get(i)).and_then(|q| quat_frame(*q)) {
                Some(r) => r,
                None => match tn.as_ref().and_then(|v| v.get(i)) {
                    Some(n) => {
                        let n = finite(*n, "N")?;
                        let up = match up.and_then(|v| v.get(i)) {
                            Some(u) => Some(d3(finite(*u, "up")?)),
                            None => None,
                        };
                        frame(d3(n), up).unwrap_or(ident)
                    }
                    None => ident,
                },
            };
            // which source for this target point
            let src: Arc<Detail> = match (&piece_src, &piece_tgt) {
                (Some(ps), Some(pt)) => {
                    let Some(key) = piece_key(pt, i) else { continue };
                    if let Some(c) = piece_cache.get(&key) {
                        Arc::clone(c)
                    } else {
                        let keep: Vec<bool> = (0..src_base.prims.len())
                            .map(|k| piece_key(ps, k).as_deref() == Some(key.as_str()))
                            .collect();
                        let sub = Arc::new(subset(&src_base, &keep)?);
                        piece_cache.insert(key, Arc::clone(&sub));
                        sub
                    }
                }
                (None, _) if !piece.is_empty() => continue, // piece attr named but the source has none: nothing matches
                (_, None) if !piece.is_empty() => continue,
                _ => Arc::clone(&src_base),
            };
            if src.point_count == 0 && src.prims.is_empty() {
                continue;
            }
            let xf = Xf { rot, s: sv, t: d3(p) };
            let has_n = src.attrs.iter().any(|a| a.name == "N" && matches!(a.data, AttrData::V3(_)));
            let uniform = sv[0] == sv[1] && sv[1] == sv[2];
            let mut c: Detail = (*src).clone();
            for v in c.p_mut().iter_mut() {
                *v = xf.point(*v)?;
            }
            if has_n && (rot != ident || !uniform) {
                for a in c.attrs.iter_mut().filter(|a| a.name == "N") {
                    if let (AttrClass::Point | AttrClass::Vertex, AttrData::V3(n)) = (a.class, &mut a.data) {
                        for v in n.iter_mut() {
                            *v = xf.normal(*v);
                        }
                    }
                }
            }
            emitted.push((i, c.point_count as usize, c.prims.len()));
            out.merge(&c);
        }
        if emitted.is_empty() {
            return Ok(Arc::new(Detail::default()));
        }
        if copynum {
            let ids = |f: &dyn Fn(&(usize, usize, usize)) -> usize| -> Vec<i32> {
                emitted
                    .iter()
                    .enumerate()
                    .flat_map(|(k, e)| std::iter::repeat_n(i32::try_from(k).unwrap_or(i32::MAX), f(e)))
                    .collect()
            };
            for (class, data) in [(AttrClass::Point, ids(&|e| e.1)), (AttrClass::Prim, ids(&|e| e.2))] {
                out.add_attr(Attribute {
                    name: "copynum".into(),
                    class,
                    data: AttrData::I32(data),
                    type_info: TypeInfo::None,
                });
            }
        }
        // target attribute transfer (points + prims): value of the target point each copy sits on
        let pats: Vec<&str> = tattribs.split_whitespace().collect();
        if !pats.is_empty() {
            let idx = |f: &dyn Fn(&(usize, usize, usize)) -> usize| -> Vec<Option<usize>> {
                emitted.iter().flat_map(|e| std::iter::repeat_n(Some(e.0), f(e))).collect()
            };
            let (pidx, qidx) = (idx(&|e| e.1), idx(&|e| e.2));
            for a in tgt.attrs.iter().filter(|a| a.class == AttrClass::Point) {
                if matches!(a.name.as_str(), "P" | "pscale" | "N" | "up" | "orient" | "scale")
                    || !pats.iter().any(|g| glob(g, &a.name))
                {
                    continue;
                }
                for (class, ix) in [(AttrClass::Point, &pidx), (AttrClass::Prim, &qidx)] {
                    out.add_attr(Attribute {
                        name: a.name.clone(),
                        class,
                        data: gather(&a.data, ix),
                        type_info: a.type_info,
                    });
                }
            }
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use hexe_geo::PrimKind;

    const EPS: f32 = 1e-5;

    fn ctx(p: &ParamSet) -> CookCtx<'_> {
        CookCtx { time: 0.0, fps: 24.0, params: p, log: &|_| {} }
    }
    fn params(over: &[(&str, ParamValue)]) -> ParamSet {
        let mut p = ParamSet::new();
        for s in spec() {
            p = p.with(&s.name, s.default.clone());
        }
        for (k, v) in over {
            p = p.with(k, v.clone());
        }
        p
    }
    fn run(src: &Detail, tgt: &Detail, over: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        new().cook(&ctx(&params(over)), &[Arc::new(src.clone()), Arc::new(tgt.clone())])
    }
    /// Hand-built box: corners (±0.5), x-major / z fastest; 6 quads over them (closed form, independent of any SOP).
    fn box_corner(k: u32) -> [f32; 3] {
        let c = |b: u32| if (k >> b) & 1 == 1 { 0.5 } else { -0.5 };
        [c(2), c(1), c(0)]
    }
    const FACES: [[u32; 4]; 6] = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
    fn hand_box() -> Detail {
        let mut d = Detail::default();
        d.add_points(8);
        *d.p_mut() = (0..8).map(box_corner).collect();
        for f in FACES {
            d.add_poly(&f, true);
        }
        d
    }
    /// Hand-built 3x3 point lattice, deliberately asymmetric: P[i] = (10*(i%3) - 10, i/3, 7*(i/3)).
    fn grid_p(i: usize) -> [f32; 3] {
        [10.0 * (i % 3) as f32 - 10.0, (i / 3) as f32, 7.0 * (i / 3) as f32]
    }
    fn targets() -> Detail {
        let mut d = Detail::default();
        d.add_points(9);
        *d.p_mut() = (0..9).map(grid_p).collect();
        d
    }
    fn with_attr(mut d: Detail, name: &str, data: AttrData) -> Detail {
        d.add_attr(Attribute { name: name.into(), class: AttrClass::Point, data, type_info: TypeInfo::None });
        d
    }
    fn close(g: [f32; 3], w: [f32; 3]) {
        assert!((0..3).all(|k| (g[k] - w[k]).abs() <= EPS), "got {g:?} want {w:?}");
    }

    #[test]
    fn declares_two_inputs_and_three_params() {
        let n = new();
        assert_eq!(n.inputs().iter().map(|i| i.name).collect::<Vec<_>>(), ["input0", "input1"]);
        assert_eq!(n.type_name(), "copytopoints");
        let defaults: Vec<_> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            defaults,
            [
                ("use_orient", ParamValue::B(false)),
                ("use_pscale", ParamValue::B(true)),
                ("copynum", ParamValue::B(false)),
                ("sourcegroup", ParamValue::S(String::new())),
                ("targetgroup", ParamValue::S(String::new())),
                ("pieceattrib", ParamValue::S(String::new())),
                ("targetattribs", ParamValue::S(String::new())),
            ]
        );
    }

    #[test]
    fn box_times_grid_3x3_counts_and_translation_oracle() {
        let out = run(&hand_box(), &targets(), &[]).expect("cooks");
        // oracle: 8*9 points, 6*9 prims, 24*9 vertices
        assert_eq!((out.point_count, out.prim_count(), out.vertex_count()), (72, 54, 216));
        assert_eq!(out.p().len(), 72);
        for i in 0..9usize {
            for k in 0..8u32 {
                let (b, t) = (box_corner(k), grid_p(i));
                close(out.p()[i * 8 + k as usize], [b[0] + t[0], b[1] + t[1], b[2] + t[2]]);
            }
        }
        // topology: copy i = source topology, point ids + 8i, prim vertex ranges + 24i, all closed polys
        for i in 0..9u32 {
            for (j, f) in FACES.iter().flatten().enumerate() {
                assert_eq!(out.vertex_point[(24 * i) as usize + j], f + 8 * i);
            }
            for q in 0..6u32 {
                let p = &out.prims[(6 * i + q) as usize];
                let lo = 24 * i + 4 * q;
                assert_eq!(p.verts, lo..lo + 4);
                assert!(matches!(p.kind, PrimKind::Poly { closed: true }));
            }
        }
    }

    fn s(v: &str) -> ParamValue {
        ParamValue::S(v.into())
    }
    /// Source group = 1 of the box's 6 quads: each copy = 4 points / 1 prim -> 36 / 9; unknown group is an error.
    #[test]
    fn source_group_copies_only_group_prims() {
        let mut b = hand_box();
        let mut mask = vec![false; 6];
        mask[1] = true;
        b.groups.push(hexe_geo::Group { name: "top".into(), class: AttrClass::Prim, mask });
        let out = run(&b, &targets(), &[("sourcegroup", s("top"))]).expect("cooks");
        assert_eq!((out.point_count, out.prim_count(), out.vertex_count()), (36, 9, 36));
        // face 1 = corners 4,6,7,5 -> compacted ascending: 4,5,6,7 (all z-major bit2=1 -> x = +0.5)
        for i in 0..9usize {
            let t = grid_p(i);
            for k in 0..4usize {
                assert!((out.p()[i * 4 + k][0] - (t[0] + 0.5)).abs() <= EPS);
            }
        }
        assert!(matches!(run(&b, &targets(), &[("sourcegroup", s("nope"))]), Err(CookError::Failed(_))));
    }
    /// Target group {1,4,7}: 3 copies in point order; copynum numbers emitted copies 0,1,2.
    #[test]
    fn target_group_limits_copies_and_copynum_counts_emitted() {
        let mut t = targets();
        let mut mask = vec![false; 9];
        for i in [1, 4, 7] {
            mask[i] = true;
        }
        t.groups.push(hexe_geo::Group { name: "sel".into(), class: AttrClass::Point, mask });
        let out = run(&hand_box(), &t, &[("targetgroup", s("sel")), ("copynum", ParamValue::B(true))]).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (24, 18));
        let Some(AttrData::I32(c)) = out.attr(AttrClass::Point, "copynum").map(|a| &a.data) else { panic!("copynum") };
        assert_eq!(c.iter().step_by(8).copied().collect::<Vec<_>>(), [0, 1, 2]);
        for (n, i) in [1usize, 4, 7].into_iter().enumerate() {
            close(
                out.p()[n * 8],
                [box_corner(0)[0] + grid_p(i)[0], box_corner(0)[1] + grid_p(i)[1], box_corner(0)[2] + grid_p(i)[2]],
            );
        }
    }
    /// Piece attr: source quad 0 (x in 0..1, piece 0) and quad 1 (x in 10..11, piece 1); targets piece [0,1,1] -> 4+4+4 points, 3 prims.
    #[test]
    fn piece_attribute_routes_source_prims_to_matching_targets() {
        let mut src = Detail::default();
        src.add_points(8);
        *src.p_mut() = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [10.0, 0.0, 0.0],
            [11.0, 0.0, 0.0],
            [11.0, 1.0, 0.0],
            [10.0, 1.0, 0.0],
        ];
        src.add_poly(&[0, 1, 2, 3], true);
        src.add_poly(&[4, 5, 6, 7], true);
        src.add_attr(Attribute {
            name: "piece".into(),
            class: AttrClass::Prim,
            data: AttrData::I32(vec![0, 1]),
            type_info: TypeInfo::None,
        });
        let mut t = Detail::default();
        t.add_points(3);
        *t.p_mut() = vec![[0.0, 0.0, 100.0], [0.0, 0.0, 200.0], [0.0, 0.0, 300.0]];
        t.add_attr(Attribute {
            name: "piece".into(),
            class: AttrClass::Point,
            data: AttrData::I32(vec![0, 1, 1]),
            type_info: TypeInfo::None,
        });
        let out = run(&src, &t, &[("pieceattrib", s("piece"))]).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (12, 3));
        close(out.p()[0], [0.0, 0.0, 100.0]);
        close(out.p()[4], [10.0, 0.0, 200.0]);
        close(out.p()[8], [10.0, 0.0, 300.0]);
        // target without the attr -> nothing copied
        let t2 = targets();
        assert_eq!(run(&src, &t2, &[("pieceattrib", s("piece"))]).expect("cooks").point_count, 0);
    }
    /// Target attr Cd (per target point) lands on the points and prims of that target's copy; pscale is not transferred.
    #[test]
    fn targetattribs_transfers_target_point_attrs_to_copies() {
        let cd: Vec<[f32; 3]> = (0..9).map(|i| [i as f32, 0.5, 1.0]).collect();
        let tgt = with_attr(with_attr(targets(), "Cd", AttrData::V3(cd)), "pscale", AttrData::F32(vec![1.0; 9]));
        let out = run(&hand_box(), &tgt, &[("targetattribs", s("Cd pscale"))]).expect("cooks");
        let (Some(AttrData::V3(pc)), Some(AttrData::V3(qc))) =
            (out.attr(AttrClass::Point, "Cd").map(|a| &a.data), out.attr(AttrClass::Prim, "Cd").map(|a| &a.data))
        else {
            panic!("Cd")
        };
        assert_eq!((pc.len(), qc.len()), (72, 54));
        for i in 0..9usize {
            assert!(pc[i * 8..i * 8 + 8].iter().all(|c| *c == [i as f32, 0.5, 1.0]));
            assert!(qc[i * 6..i * 6 + 6].iter().all(|c| *c == [i as f32, 0.5, 1.0]));
        }
        assert!(out.attr(AttrClass::Point, "pscale").is_none());
        let off = run(&hand_box(), &tgt, &[]).expect("cooks");
        assert!(off.attr(AttrClass::Point, "Cd").is_none());
    }
    /// scale (2,1,1) then orient = 90 deg about +Z (x->y): (0.5,0.5,0.5) -> (1,.5,.5) -> (-.5,1,.5) + t.
    #[test]
    fn orient_quaternion_and_scale_attr_compose_scale_then_rotate() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let mut t = Detail::default();
        t.add_points(1);
        *t.p_mut() = vec![[10.0, 20.0, 30.0]];
        let t = with_attr(
            with_attr(t, "scale", AttrData::V3(vec![[2.0, 1.0, 1.0]])),
            "orient",
            AttrData::V4(vec![[0.0, 0.0, h, h]]),
        );
        let out = run(&hand_box(), &t, &[("use_orient", ParamValue::B(true))]).expect("cooks");
        close(out.p()[7], [10.0 - 0.5, 20.0 + 1.0, 30.0 + 0.5]);
        // without use_orient the quaternion is ignored; scale alone: (1,.5,.5)+t
        let plain = run(&hand_box(), &t, &[]).expect("cooks");
        close(plain.p()[7], [11.0, 20.5, 30.5]);
    }
    /// Target with only a VERTEX-class N = +X: source point (0,0,1) (the local +Z) maps to +X.
    #[test]
    fn target_vertex_class_n_orients_when_no_point_n() {
        let mut src = Detail::default();
        src.add_points(1);
        *src.p_mut() = vec![[0.0, 0.0, 1.0]];
        let mut t = Detail::default();
        t.add_points(3);
        *t.p_mut() = vec![[0.0, 0.0, 0.0], [5.0, 0.0, 0.0], [0.0, 5.0, 0.0]];
        t.add_poly(&[0, 1, 2], true);
        t.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(vec![[1.0, 0.0, 0.0]; 3]),
            type_info: TypeInfo::Normal,
        });
        let out = run(&src, &t, &[("use_orient", ParamValue::B(true))]).expect("cooks");
        assert_eq!(out.point_count, 3);
        close(out.p()[0], [1.0, 0.0, 0.0]);
        close(out.p()[1], [6.0, 0.0, 0.0]);
        close(out.p()[2], [1.0, 5.0, 0.0]);
    }
    #[test]
    fn pscale_2_doubles_box_extents_about_each_target_point() {
        let tgt = with_attr(targets(), "pscale", AttrData::F32(vec![2.0; 9]));
        let out = run(&hand_box(), &tgt, &[]).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (72, 54));
        for i in 0..9usize {
            let t = grid_p(i);
            for k in 0..8u32 {
                let b = box_corner(k);
                close(out.p()[i * 8 + k as usize], [t[0] + 2.0 * b[0], t[1] + 2.0 * b[1], t[2] + 2.0 * b[2]]);
            }
            // extents: unit box (1 wide) -> exactly 2 wide, centred on P_i
            let mut c = Detail::default();
            c.add_points(8);
            *c.p_mut() = out.p()[i * 8..i * 8 + 8].to_vec();
            let (lo, hi) = c.bbox();
            for a in 0..3 {
                assert!((hi[a] - lo[a] - 2.0).abs() <= EPS && ((hi[a] + lo[a]) / 2.0 - t[a]).abs() <= EPS);
            }
        }
    }

    #[test]
    fn per_point_pscale_and_use_pscale_off() {
        let mut s = vec![1.0f32; 9];
        s[4] = 3.0;
        s[7] = 0.0;
        let tgt = with_attr(targets(), "pscale", AttrData::F32(s));
        let out = run(&hand_box(), &tgt, &[]).expect("cooks");
        for k in 0..8usize {
            let (b, t4, t7, t0) = (box_corner(k as u32), grid_p(4), grid_p(7), grid_p(0));
            close(out.p()[4 * 8 + k], [t4[0] + 3.0 * b[0], t4[1] + 3.0 * b[1], t4[2] + 3.0 * b[2]]);
            close(out.p()[7 * 8 + k], t7); // pscale 0 collapses onto P
            close(out.p()[k], [t0[0] + b[0], t0[1] + b[1], t0[2] + b[2]]);
        }
        let off = run(&hand_box(), &tgt, &[("use_pscale", ParamValue::B(false))]).expect("cooks");
        let plain = run(&hand_box(), &targets(), &[]).expect("cooks");
        assert_eq!(off.p(), plain.p());
    }

    #[test]
    fn orient_n_maps_source_z_to_n_with_default_up() {
        // N = +X, up = +Y(default): x = up x n = (0,0,-1), y = n x x = (0,1,0), z = n = (1,0,0)
        let tgt = with_attr(targets(), "N", AttrData::V3(vec![[1.0, 0.0, 0.0]; 9]));
        let mut src = Detail::default();
        src.add_points(3);
        *src.p_mut() = vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let out = run(&src, &tgt, &[("use_orient", ParamValue::B(true))]).expect("cooks");
        let t = grid_p(5);
        close(out.p()[15], [t[0], t[1], t[2] - 1.0]);
        close(out.p()[16], [t[0], t[1] + 1.0, t[2]]);
        close(out.p()[17], [t[0] + 1.0, t[1], t[2]]);
        // orient off (default): N ignored, plain translation
        let off = run(&src, &tgt, &[]).expect("cooks");
        close(off.p()[17], [t[0], t[1], t[2] + 1.0]);
    }

    #[test]
    fn orient_uses_up_attr_and_degenerate_normals_fall_back() {
        // N = +X, up = +Z: x = Z x X = (0,1,0), y = X x Y = (0,0,1), z = (1,0,0)  (cyclic permutation)
        let tgt = with_attr(targets(), "N", AttrData::V3(vec![[2.0, 0.0, 0.0]; 9])); // un-normalised N is fine
        let tgt = with_attr(tgt, "up", AttrData::V3(vec![[0.0, 0.0, 5.0]; 9]));
        let mut src = Detail::default();
        src.add_points(3);
        *src.p_mut() = vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let on = [("use_orient", ParamValue::B(true))];
        let out = run(&src, &tgt, &on).expect("cooks");
        let t = grid_p(0);
        close(out.p()[0], [t[0], t[1] + 1.0, t[2]]);
        close(out.p()[1], [t[0], t[1], t[2] + 1.0]);
        close(out.p()[2], [t[0] + 1.0, t[1], t[2]]);
        // N parallel to default up (Y): fallback up = X; z -> N, frame stays orthonormal
        let tgt = with_attr(targets(), "N", AttrData::V3(vec![[0.0, 1.0, 0.0]; 9]));
        let out = run(&src, &tgt, &on).expect("cooks");
        close(out.p()[2], [t[0], t[1] + 1.0, t[2]]);
        for a in 0..2 {
            let q = [out.p()[a][0] - t[0], out.p()[a][1] - t[1], out.p()[a][2] - t[2]];
            assert!((q[0] * q[0] + q[1] * q[1] + q[2] * q[2] - 1.0).abs() <= EPS && q[1].abs() <= EPS);
        }
        // zero-length N -> identity
        let tgt = with_attr(targets(), "N", AttrData::V3(vec![[0.0; 3]; 9]));
        close(run(&src, &tgt, &on).expect("cooks").p()[2], [t[0], t[1], t[2] + 1.0]);
    }

    #[test]
    fn source_normals_rotate_with_orient_and_other_attrs_repeat() {
        let mut src = with_attr(hand_box(), "N", AttrData::V3(vec![[0.0, 0.0, 1.0]; 8]));
        src = with_attr(src, "Cd", AttrData::V3((0..8).map(|k| [k as f32, 0.0, 0.0]).collect()));
        let tgt = with_attr(targets(), "N", AttrData::V3(vec![[1.0, 0.0, 0.0]; 9]));
        let out = run(&src, &tgt, &[("use_orient", ParamValue::B(true))]).expect("cooks");
        let Some(AttrData::V3(n)) = out.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("no N") };
        assert_eq!(n.len(), 72);
        assert!(
            n.iter().all(|v| (0..3).all(|k| (v[k] - [1.0, 0.0, 0.0][k]).abs() <= EPS)),
            "source +Z normal -> target N"
        );
        let Some(AttrData::V3(cd)) = out.attr(AttrClass::Point, "Cd").map(|a| &a.data) else { panic!("no Cd") };
        assert!((0..72).all(|i| cd[i][0] == (i % 8) as f32));
    }

    #[test]
    fn copynum_off_by_default_and_numbers_points_and_prims_when_on() {
        let off = run(&hand_box(), &targets(), &[]).expect("cooks");
        assert!(off.attr(AttrClass::Point, "copynum").is_none() && off.attr(AttrClass::Prim, "copynum").is_none());
        let on = run(&hand_box(), &targets(), &[("copynum", ParamValue::B(true))]).expect("cooks");
        let Some(AttrData::I32(p)) = on.attr(AttrClass::Point, "copynum").map(|a| &a.data) else { panic!("no pt") };
        let Some(AttrData::I32(q)) = on.attr(AttrClass::Prim, "copynum").map(|a| &a.data) else { panic!("no prim") };
        assert_eq!((p.len(), q.len()), (72, 54));
        assert!(p.iter().enumerate().all(|(k, v)| *v == (k / 8) as i32));
        assert!(q.iter().enumerate().all(|(k, v)| *v == (k / 6) as i32));
    }

    #[test]
    fn empty_and_missing_inputs_cook_to_empty() {
        let e = Detail::default();
        assert_eq!(*run(&hand_box(), &e, &[]).expect("no targets"), e);
        assert_eq!(*run(&e, &targets(), &[]).expect("no source"), e);
        let p = params(&[]);
        assert_eq!(*new().cook(&ctx(&p), &[Arc::new(hand_box())]).expect("unwired input1"), e);
        assert_eq!(*new().cook(&ctx(&p), &[]).expect("none"), e);
    }

    #[test]
    fn single_target_at_origin_reproduces_the_source() {
        let mut t = Detail::default();
        t.add_points(1);
        let out = run(&hand_box(), &t, &[]).expect("cooks");
        assert_eq!(out.p(), hand_box().p());
        assert_eq!((out.vertex_point.clone(), out.prims.clone()), (hand_box().vertex_point, hand_box().prims));
    }

    #[test]
    fn hostile_inputs_are_errors() {
        let src = hand_box();
        let nan = with_attr(targets(), "pscale", AttrData::F32(vec![f32::NAN; 9]));
        assert!(matches!(run(&src, &nan, &[]), Err(CookError::Failed(_))));
        assert!(run(&src, &nan, &[("use_pscale", ParamValue::B(false))]).is_ok(), "pscale unread when off");
        let mut bad = targets();
        bad.p_mut()[3] = [f32::INFINITY, 0.0, 0.0];
        assert!(matches!(run(&src, &bad, &[]), Err(CookError::Failed(_))));
        let nn = with_attr(targets(), "N", AttrData::V3(vec![[f32::NAN, 0.0, 0.0]; 9]));
        assert!(matches!(run(&src, &nn, &[("use_orient", ParamValue::B(true))]), Err(CookError::Failed(_))));
        let huge = with_attr(targets(), "pscale", AttrData::F32(vec![3.0e38; 9]));
        let mut far = hand_box();
        far.p_mut()[0] = [1.0e3, 0.0, 0.0];
        assert!(matches!(run(&far, &huge, &[]), Err(CookError::Failed(_))));
        // allocation cap: 24 vertices x 2^20 targets > 2^24 (limiting class = vertices)
        assert!(check_size(&src, 1 << 20).is_err() && check_size(&src, 1 << 19).is_ok());
        assert!(check_size(&src, usize::MAX).is_err());
        // bad param type
        let p = ParamSet::new().with("use_orient", ParamValue::F(1.0));
        let e = new().cook(&ctx(&p), &[Arc::new(src.clone()), Arc::new(targets())]);
        assert!(matches!(e, Err(CookError::BadParam(_))));
    }

    #[test]
    fn scene_loader_wires_box_and_grid_and_example_cooks() {
        let scene = r#"{"nodes":[{"name":"b","type":"box"},{"name":"g","type":"grid","params":{"rows":3,"cols":3,"type":4}},
            {"name":"c","type":"copytopoints","params":{"use_pscale":true,"use_orient":false}}],
            "edges":[["b",0,"c",0],["g",0,"c",1]],"display":"c"}"#;
        let (mut g, id) = crate::load_scene(scene).expect("loads");
        let out = g.cook(id, 0.0).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (72, 54));
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/copytopoints.json"))
            .expect("example present");
        let (mut g, id) = crate::load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert_eq!((out.point_count, out.prim_count()), (72, 54));
        assert!(out.attr(AttrClass::Point, "copynum").is_some());
        let bogus = r#"{"nodes":[{"name":"c","type":"copytopoints","params":{"bogus":1}}],"display":"c"}"#;
        assert!(crate::load_scene(bogus).is_err());
        assert_eq!(crate::registry().create("copytopoints").map(|n| n.type_name()), Some("copytopoints"));
    }
}
