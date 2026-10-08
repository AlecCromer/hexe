//! Normal SOP (token `normal`): computes `N` on points / vertices / prims / the detail from polygon geometry.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; closed-form oracle only)
//! * Face normal = unit Newell vector of a closed polygon with `>= 3` corners; `(0,0,0)` for any other prim or a
//!   zero-area / non-finite / dangling-id polygon (such a prim contributes nothing anywhere).
//! * `type` menu `point | vertex | prim | detail` (indices 0..=3, default vertex): the class `N` is written to.
//! * `weighting` menu `area | angle | uniform` (0..=2, default area): weight of face `f` at a corner =
//!   `|Newell|/2` | interior angle at that corner | 1. A weighted sum of unit face normals is normalised;
//!   a zero sum -> `(0,0,0)`.
//! * point: weighted sum over every selected face touching the point. vertex: same but only faces whose normal is
//!   within `cusp_angle` degrees of the vertex's OWN face (point-local test, not edge-connectivity); `cusp_angle >= 180`
//!   = never split. prim: the face normal. detail: normalised sum of unit face normals weighted by area (uniform for
//!   `uniform`; angle behaves as area).
//! * `add_normals_to`: space-separated PRIM group names (union), empty = all. Unselected prims are ignored entirely
//!   (they do not contribute to neighbours); elements they alone own keep any existing `N` of that class, else zero.
//!
//! Output = the input with `N` (V3, type Normal) of the chosen class replaced/added; everything else untouched.
//! Hostile: non-finite `cusp_angle` / menu out of range -> `BadParam`; unknown group, or vertex-mode per-point work
//! above `MAX_WORK` pair tests -> `Failed`.
use super::meshutil::newell_corners;
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
/// `type` menu indices.
const TYPE_POINT: u32 = 0;
const TYPE_VERTEX: u32 = 1;
const TYPE_PRIM: u32 = 2;
const TYPE_DETAIL: u32 = 3;
/// `weighting` menu indices.
const W_AREA: u32 = 0;
const W_ANGLE: u32 = 1;
const W_UNIFORM: u32 = 2;
/// Newell magnitudes (twice the area) at or below this are degenerate.
const MIN_NEWELL: f64 = 1e-12;
/// Slack on the cusp comparison so a face pair at exactly `cusp_angle` (box edges at 90) counts as smooth.
const COS_EPS: f64 = 1e-9;
/// Cap on `sum(deg^2)` pair tests in vertex mode (AGENTS.md section 4).
const MAX_WORK: u64 = 1 << 28;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(TYPE_VERTEX)).label("Add Normals To"),
        ParamSpec::new("cusp_angle", ParamValue::F(60.0)).label("Cusp Angle").range(0.0, 180.0),
        ParamSpec::new("weighting", ParamValue::Menu(W_AREA)).label("Weighting Method"),
        ParamSpec::new("add_normals_to", ParamValue::S(String::new())).label("Group"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Normal { specs: spec() })
}
struct Normal {
    specs: Vec<ParamSpec>,
}
type V3 = [f64; 3];
fn bad(m: impl Into<String>) -> CookError {
    CookError::BadParam(format!("normal: {}", m.into()))
}
fn failed(m: &str) -> CookError {
    CookError::Failed(format!("normal: {m}"))
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add_scaled(acc: &mut V3, v: V3, w: f64) {
    for (a, c) in acc.iter_mut().zip(v) {
        *a += w * c;
    }
}
/// Normalised `v`, or zero when it is (near) zero / non-finite.
fn unit_or_zero(v: V3) -> V3 {
    let l = dot(v, v).sqrt();
    if l.is_finite() && l > MIN_NEWELL { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0; 3] }
}
fn to_f32(v: V3) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}
/// Per-prim geometry: corner point ids (closed poly, `>= 3`, ids in range) + unit normal + area.
struct Face<'a> {
    pts: &'a [u32],
    normal: V3,
    area: f64,
}
impl Face<'_> {
    fn pos(&self, p: &[[f32; 3]], k: usize) -> Option<V3> {
        let q = p.get(usize::try_from(*self.pts.get(k)?).ok()?)?;
        Some([f64::from(q[0]), f64::from(q[1]), f64::from(q[2])])
    }
    /// Interior angle (radians) at corner `c`; 0 for a zero-length edge.
    fn angle(&self, p: &[[f32; 3]], c: usize) -> f64 {
        let n = self.pts.len();
        let at = |k: usize| self.pos(p, k);
        let (Some(a), Some(o), Some(b)) = (at((c + n - 1) % n), at(c), at((c + 1) % n)) else { return 0.0 };
        let (u, v) = (sub(a, o), sub(b, o));
        let l = (dot(u, u) * dot(v, v)).sqrt();
        if l.is_finite() && l > 0.0 { (dot(u, v) / l).clamp(-1.0, 1.0).acos() } else { 0.0 }
    }
    fn weight(&self, mode: u32, p: &[[f32; 3]], c: usize) -> f64 {
        match mode {
            W_ANGLE => self.angle(p, c),
            W_UNIFORM => 1.0,
            _ => self.area,
        }
    }
}
/// `None` = this prim contributes nothing (not a usable closed polygon).
fn face_of<'a>(src: &'a Detail, prim: &hexe_geo::Prim) -> Option<Face<'a>> {
    if !matches!(prim.kind, PrimKind::Poly { closed: true }) {
        return None;
    }
    let (lo, hi) = (usize::try_from(prim.verts.start).ok()?, usize::try_from(prim.verts.end).ok()?);
    let pts = src.vertex_point.get(lo..hi)?;
    if pts.len() < 3 || pts.iter().any(|&q| q >= src.point_count) {
        return None;
    }
    let mut f = Face { pts, normal: [0.0; 3], area: 0.0 };
    let p = src.p();
    let corners: Vec<V3> = (0..pts.len()).map(|k| f.pos(p, k)).collect::<Option<_>>()?;
    let nw = newell_corners(&corners);
    let len = dot(nw, nw).sqrt();
    if !len.is_finite() || len <= MIN_NEWELL {
        return None;
    }
    f.normal = [nw[0] / len, nw[1] / len, nw[2] / len];
    f.area = len * 0.5;
    Some(f)
}
fn selection(src: &Detail, names: &str) -> Result<Vec<bool>, CookError> {
    let names: Vec<&str> = names.split_whitespace().collect();
    if names.is_empty() {
        return Ok(vec![true; src.prims.len()]);
    }
    let mut sel = vec![false; src.prims.len()];
    for n in names {
        let g = src.group(AttrClass::Prim, n).ok_or_else(|| failed(&format!("unknown primitive group `{n}`")))?;
        for (s, m) in sel.iter_mut().zip(&g.mask) {
            *s |= *m;
        }
    }
    Ok(sel)
}
/// Existing V3 `N` of `class` when its length matches, else zeros.
fn base(src: &Detail, class: AttrClass, n: usize) -> Vec<[f32; 3]> {
    match src.attr(class, "N").map(|a| &a.data) {
        Some(AttrData::V3(v)) if v.len() == n => v.clone(),
        _ => vec![[0.0; 3]; n],
    }
}
fn menu(p: &ParamSet, name: &str, max: u32) -> Result<u32, CookError> {
    match p.menu(name) {
        Some(v) if v <= max => Ok(v),
        Some(v) => Err(bad(format!("`{name}`: menu index {v} out of range 0..={max}"))),
        None => Err(bad(format!("`{name}`: missing or not a menu"))),
    }
}
impl Node<Detail> for Normal {
    fn type_name(&self) -> &'static str {
        "normal"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let ty = menu(p, "type", TYPE_DETAIL)?;
        let wm = menu(p, "weighting", W_UNIFORM)?;
        let cusp = match p.float("cusp_angle") {
            Some(v) if v.is_finite() => f64::from(v).clamp(0.0, 180.0),
            _ => return Err(bad("`cusp_angle`: missing or not a finite float")),
        };
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let sel = selection(src, p.string("add_normals_to").unwrap_or(""))?;
        let faces: Vec<Option<Face>> =
            src.prims.iter().zip(&sel).map(|(pr, &s)| if s { face_of(src, pr) } else { None }).collect();
        let pos = src.p();
        let (attr_class, data) = match ty {
            TYPE_PRIM => {
                let mut n = base(src, AttrClass::Prim, src.prims.len());
                for (slot, f) in n.iter_mut().zip(&faces) {
                    if let Some(f) = f {
                        *slot = to_f32(f.normal);
                    }
                }
                (AttrClass::Prim, n)
            }
            TYPE_DETAIL => {
                let mut sum = [0.0; 3];
                for f in faces.iter().flatten() {
                    add_scaled(&mut sum, f.normal, if wm == W_UNIFORM { 1.0 } else { f.area });
                }
                (AttrClass::Detail, vec![to_f32(unit_or_zero(sum))])
            }
            TYPE_POINT => {
                let np = src.point_count as usize;
                let mut acc = vec![[0.0; 3]; np];
                let mut touched = vec![false; np];
                for f in faces.iter().flatten() {
                    for (c, &pt) in f.pts.iter().enumerate() {
                        if let (Some(a), Some(t)) = (acc.get_mut(pt as usize), touched.get_mut(pt as usize)) {
                            add_scaled(a, f.normal, f.weight(wm, pos, c));
                            *t = true;
                        }
                    }
                }
                let mut n = base(src, AttrClass::Point, np);
                for ((slot, a), t) in n.iter_mut().zip(acc).zip(touched) {
                    if t {
                        *slot = to_f32(unit_or_zero(a));
                    }
                }
                (AttrClass::Point, n)
            }
            _ => (AttrClass::Vertex, vertex_normals(src, &faces, wm, cusp)?),
        };
        let mut out = (**src).clone();
        let data = AttrData::V3(data);
        out.add_attr(Attribute { name: "N".into(), class: attr_class, data, type_info: TypeInfo::Normal });
        Ok(Arc::new(out))
    }
}
/// Vertex mode: per incident face corner, weighted sum of the faces at the same point within `cusp` degrees.
fn vertex_normals(src: &Detail, faces: &[Option<Face>], wm: u32, cusp: f64) -> Result<Vec<[f32; 3]>, CookError> {
    let pos = src.p();
    // incident (point, vertex index, face index, corner) list, grouped by point
    let mut inc: Vec<(u32, usize, usize, usize)> = Vec::new();
    for (fi, (f, pr)) in faces.iter().zip(&src.prims).enumerate() {
        let Some(f) = f else { continue };
        let v0 = usize::try_from(pr.verts.start).map_err(|_| failed("vertex id overflow"))?;
        for (c, &pt) in f.pts.iter().enumerate() {
            inc.push((pt, v0 + c, fi, c));
        }
    }
    inc.sort_unstable_by_key(|e| (e.0, e.1));
    let mut out = base(src, AttrClass::Vertex, src.vertex_point.len());
    let cos_cusp = cusp.to_radians().cos() - COS_EPS;
    let mut work: u64 = 0;
    let mut i = 0;
    while i < inc.len() {
        let pt = inc.get(i).map_or(0, |e| e.0);
        let end = i + inc.get(i..).unwrap_or(&[]).iter().take_while(|e| e.0 == pt).count();
        let group = inc.get(i..end).unwrap_or(&[]);
        let term = |&(_, _, fi, c): &(u32, usize, usize, usize)| -> Option<(V3, f64)> {
            let f = faces.get(fi)?.as_ref()?;
            Some((f.normal, f.weight(wm, pos, c)))
        };
        let g = group.len() as u64;
        work = work.saturating_add(if cusp >= 180.0 { g } else { g.saturating_mul(g) });
        if work > MAX_WORK {
            return Err(failed("vertex-mode work exceeds the cap (too many faces on one point)"));
        }
        if cusp >= 180.0 {
            let mut sum = [0.0; 3];
            for (n, w) in group.iter().filter_map(term) {
                add_scaled(&mut sum, n, w);
            }
            let n = to_f32(unit_or_zero(sum));
            for e in group {
                if let Some(s) = out.get_mut(e.1) {
                    *s = n;
                }
            }
        } else {
            for e in group {
                let Some((own, _)) = term(e) else { continue };
                let mut sum = [0.0; 3];
                for (n, w) in group.iter().filter_map(term) {
                    if dot(own, n) >= cos_cusp {
                        add_scaled(&mut sum, n, w);
                    }
                }
                if let Some(s) = out.get_mut(e.1) {
                    *s = to_f32(unit_or_zero(sum));
                }
            }
        }
        i = end;
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::Group;
    fn cook(d: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        new().cook(&ctx, &[Arc::new(d)])
    }
    fn defaults() -> ParamSet {
        new().params().iter().fold(ParamSet::new(), |p, s| p.with(&s.name, s.default.clone()))
    }
    fn with(p: ParamSet, k: &str, v: ParamValue) -> ParamSet {
        p.with(k, v)
    }
    fn params(ty: u32, cusp: f32, w: u32) -> ParamSet {
        with(
            with(with(defaults(), "type", ParamValue::Menu(ty)), "cusp_angle", ParamValue::F(cusp)),
            "weighting",
            ParamValue::Menu(w),
        )
    }
    /// Unit box (corners +-0.5, 8 shared points), 6 quads in Houdini winding (clockwise from outside), closed form.
    fn cube() -> Detail {
        let mut d = Detail::default();
        d.add_points(8);
        let c = |b: u32, i: u32| if (i >> b) & 1 == 1 { 0.5 } else { -0.5 };
        *d.p_mut() = (0..8).map(|i| [c(2, i), c(1, i), c(0, i)]).collect();
        for f in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
            d.add_poly(&f, true);
        }
        d
    }
    fn v3(d: &Detail, class: AttrClass) -> Vec<[f32; 3]> {
        match d.attr(class, "N").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            other => panic!("no V3 N on {class:?}: {other:?}"),
        }
    }
    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|j| (a[j] - b[j]).abs() < 1e-6)
    }
    /// Independent unit normal of a prim: textbook (a-b)x(a+b) Newell form, NEGATED = Houdini's left-hand rule.
    fn newell(d: &Detail, i: usize) -> [f32; 3] {
        let pr = &d.prims[i];
        let q: Vec<[f64; 3]> = d.vertex_point[pr.verts.start as usize..pr.verts.end as usize]
            .iter()
            .map(|&p| d.p()[p as usize].map(f64::from))
            .collect();
        let mut n = [0.0f64; 3];
        for k in 0..q.len() {
            let (a, b) = (q[k], q[(k + 1) % q.len()]);
            n[0] += (a[1] - b[1]) * (a[2] + b[2]);
            n[1] += (a[2] - b[2]) * (a[0] + b[0]);
            n[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        let l = -(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
 [(n[0] / l) as f32, (n[1] / l) as f32, (n[2] / l) as f32]
    }
    const AXES: [[f32; 3]; 6] =
        [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [0.0, 0.0, 1.0]];
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "normal");
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("type", ParamValue::Menu(TYPE_VERTEX)),
                ("cusp_angle", ParamValue::F(60.0)),
                ("weighting", ParamValue::Menu(W_AREA)),
                ("add_normals_to", ParamValue::S(String::new())),
            ]
        );
    }
    #[test]
    fn box_prim_normals_are_the_six_axes_exact() {
        let out = cook(cube(), params(TYPE_PRIM, 60.0, W_AREA)).expect("cooks");
        let n = v3(&out, AttrClass::Prim);
        assert_eq!(n, AXES);
        // independent cross-check from P
        for (i, a) in n.iter().enumerate() {
            assert!(near(*a, newell(&out, i)), "prim {i}");
        }
        assert_eq!(out.attr(AttrClass::Prim, "N").map(|a| a.type_info), Some(TypeInfo::Normal));
    }
    #[test]
    fn box_point_normals_uniform_are_normalized_corner_diagonals() {
        for w in [W_UNIFORM, W_AREA, W_ANGLE] {
            let out = cook(cube(), params(TYPE_POINT, 60.0, w)).expect("cooks");
            let n = v3(&out, AttrClass::Point);
            assert_eq!(n.len(), 8);
            let s = 1.0 / 3.0f32.sqrt();
            for (i, q) in out.p().iter().enumerate() {
                let want = [q[0].signum() * s, q[1].signum() * s, q[2].signum() * s];
                assert!(near(n[i], want), "w={w} pt {i}: {:?} vs {want:?}", n[i]);
            }
        }
    }
    #[test]
    fn grid_3x3_flat_every_point_normal_is_the_plane_normal_from_p() {
        let json = r#"{"nodes":[{"name":"g","type":"grid","params":{"rows":3,"cols":3,"size":[2,2,0]}},
            {"name":"n","type":"normal","params":{"type":0}}],"edges":[["g",0,"n",0]],"display":"n"}"#;
        let (mut g, id) = load_scene(json).expect("loads");
        let out = g.cook(id, 0.0).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (9, 4));
        // plane derived from P: all y equal -> XZ plane; sign from the independent Newell of prim 0
        let ys: Vec<f32> = out.p().iter().map(|q| q[1]).collect();
        assert!(ys.iter().all(|y| *y == ys[0]), "grid not in an XZ plane");
        let sign = newell(&out, 0)[1];
        assert!(sign.abs() > 0.99);
        let want = [0.0, sign.signum(), 0.0];
        assert!(v3(&out, AttrClass::Point).iter().all(|n| near(*n, want)), "{:?}", v3(&out, AttrClass::Point));
        assert_eq!(want, [0.0, 1.0, 0.0], "grid generator documents +Y for the default zx orientation");
    }
    #[test]
    fn box_vertex_cusp_60_is_face_normal_cusp_100_is_corner_averaged() {
        let sharp = cook(cube(), params(TYPE_VERTEX, 60.0, W_AREA)).expect("cooks");
        let vn = v3(&sharp, AttrClass::Vertex);
        assert_eq!(vn.len(), 24);
        let pn = v3(&cook(cube(), params(TYPE_PRIM, 60.0, W_AREA)).expect("cooks"), AttrClass::Prim);
        for (i, pr) in sharp.prims.iter().enumerate() {
            for v in pr.verts.clone() {
                assert_eq!(vn[v as usize], pn[i], "prim {i} vertex {v}");
            }
        }
        // default params == cusp 60 vertex
        assert_eq!(cook(cube(), defaults()).expect("cooks"), sharp);
        let smooth = cook(cube(), params(TYPE_VERTEX, 100.0, W_AREA)).expect("cooks");
        let s = 1.0 / 3.0f32.sqrt();
        for (i, pr) in smooth.prims.iter().enumerate() {
            for v in pr.verts.clone() {
                let q = smooth.p()[smooth.vertex_point[v as usize] as usize];
                let want = [q[0].signum() * s, q[1].signum() * s, q[2].signum() * s];
                assert!(near(v3(&smooth, AttrClass::Vertex)[v as usize], want), "prim {i} vertex {v}");
            }
        }
        // exactly 90 degrees between box faces counts as smooth (<=)
        let edge = cook(cube(), params(TYPE_VERTEX, 90.0, W_AREA)).expect("cooks");
        assert!(v3(&edge, AttrClass::Vertex).iter().all(|n| n.iter().all(|c| (c.abs() - s).abs() < 1e-6)));
    }
    #[test]
    fn detail_normal_of_closed_box_is_zero_and_of_a_quad_is_its_normal() {
        let out = cook(cube(), params(TYPE_DETAIL, 60.0, W_AREA)).expect("cooks");
        assert_eq!(v3(&out, AttrClass::Detail), [[0.0; 3]]);
        let mut q = Detail::default();
        q.add_points(4);
        *q.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        q.add_poly(&[0, 3, 2, 1], true); // clockwise from +Z = Houdini normal +Z
        let out = cook(q, params(TYPE_DETAIL, 60.0, W_AREA)).expect("cooks");
        assert_eq!(v3(&out, AttrClass::Detail), [[0.0, 0.0, 1.0]]);
    }
    #[test]
    fn angle_weighting_differs_from_area_on_unequal_faces() {
        // fan around point 0: big quad (+Z), tiny triangle (-X, Houdini rule)
        let mut d = Detail::default();
        d.add_points(6);
        *d.p_mut() = vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
        ];
        d.add_poly(&[0, 3, 2, 1], true); // +Z, area 100, corner angle 90
 d.add_poly(&[0, 5, 4], true); // -X, area .5, corner angle 90
        let a = v3(&cook(d.clone(), params(TYPE_POINT, 60.0, W_AREA)).expect("cooks"), AttrClass::Point)[0];
        let g = v3(&cook(d.clone(), params(TYPE_POINT, 60.0, W_ANGLE)).expect("cooks"), AttrClass::Point)[0];
        let u = v3(&cook(d, params(TYPE_POINT, 60.0, W_UNIFORM)).expect("cooks"), AttrClass::Point)[0];
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!(near(g, [-h, 0.0, h]) && near(u, [-h, 0.0, h]), "angle {g:?} uniform {u:?}");
        assert!(a[2] > 0.99 && a[0] > -0.01 && a[0] < 0.0, "area-weighted should hug +Z: {a:?}");
    }
    #[test]
    fn group_limits_processing_and_preserves_existing_n() {
        let mut d = cube();
        d.groups.push(Group {
            name: "top".into(),
            class: AttrClass::Prim,
            mask: vec![false, true, false, false, false, false],
        });
        let out = cook(d.clone(), with(params(TYPE_PRIM, 60.0, W_AREA), "add_normals_to", ParamValue::S("top".into())))
            .expect("cooks");
        let n = v3(&out, AttrClass::Prim);
        assert_eq!(n[1], [1.0, 0.0, 0.0]);
        assert!(n.iter().enumerate().all(|(i, v)| i == 1 || *v == [0.0; 3]));
        // pre-existing N survives outside the group
        let pre = cook(d.clone(), params(TYPE_PRIM, 60.0, W_AREA)).expect("cooks");
        let mut pre = (*pre).clone();
        if let Some(Attribute { data: AttrData::V3(v), .. }) = pre.attr_mut(AttrClass::Prim, "N") {
            v[0] = [9.0, 9.0, 9.0];
        }
        let out = cook(pre, with(params(TYPE_PRIM, 60.0, W_AREA), "add_normals_to", ParamValue::S("top".into())))
            .expect("cooks");
        assert_eq!(v3(&out, AttrClass::Prim)[0], [9.0, 9.0, 9.0]);
        let unknown = cook(d, with(defaults(), "add_normals_to", ParamValue::S("nope".into())));
        assert!(matches!(unknown, Err(CookError::Failed(_))));
    }
    #[test]
    fn hostile_degenerate_open_and_bad_params() {
        let mut d = cube();
        d.add_poly(&[0, 0, 0, 0], true); // zero area
        d.add_poly(&[0, 1], false); // polyline
        d.add_poly(&[0, 1, 2], false); // open poly
        for ty in [TYPE_POINT, TYPE_VERTEX, TYPE_PRIM, TYPE_DETAIL] {
            let out = cook(d.clone(), params(ty, 60.0, W_ANGLE)).expect("cooks");
            let class = [AttrClass::Point, AttrClass::Vertex, AttrClass::Prim, AttrClass::Detail][ty as usize];
            assert!(v3(&out, class).iter().flatten().all(|c| c.is_finite()));
        }
        let prim = v3(&cook(d.clone(), params(TYPE_PRIM, 60.0, W_AREA)).expect("cooks"), AttrClass::Prim);
        assert_eq!(prim[6..], [[0.0; 3]; 3]);
        for (k, v) in [
            ("cusp_angle", ParamValue::F(f32::NAN)),
            ("type", ParamValue::Menu(4)),
            ("weighting", ParamValue::Menu(3)),
            ("type", ParamValue::I(1)),
        ] {
            assert!(matches!(cook(cube(), with(defaults(), k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        // empty / missing input
        assert_eq!(cook(Detail::default(), defaults()).expect("cooks").point_count, 0);
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &defaults(), log: &|_| {} };
        assert_eq!(new().cook(&ctx, &[]).expect("cooks").point_count, 0);
        // dangling point id + non-finite P
        let mut h = cube();
        h.vertex_point[0] = 99;
        h.p_mut()[7] = [f32::NAN, 0.0, 0.0];
        let out = cook(h, params(TYPE_VERTEX, 180.0, W_AREA)).expect("cooks");
        assert!(v3(&out, AttrClass::Vertex).iter().flatten().all(|c| c.is_finite()));
    }
    #[test]
    fn bare_scene_node_defaults_to_vertex_n_with_cusp_60_axis_aligned_on_a_box() {
        // h27-3: no params at all (as `hexe-cli`/window create it) => vertex-class N, cusp 60 => every box corner N is a face axis.
        let s = r#"{"nodes":[{"name":"b","type":"box"},{"name":"n","type":"normal"}],"edges":[["b",0,"n",0]],"display":"n"}"#;
        let (mut g, id) = load_scene(s).expect("loads");
        let out = g.cook(id, 0.0).expect("cooks");
        assert!(out.attr(AttrClass::Point, "N").is_none() && out.attr(AttrClass::Prim, "N").is_none());
        let vn = v3(&out, AttrClass::Vertex);
        assert_eq!(vn.len(), 24);
        for n in &vn {
            assert!(AXES.contains(n), "vertex N {n:?} not axis-aligned");
        }
        // point mode stays available: corner diagonals, not axes
        let pt = cook(cube(), params(TYPE_POINT, 60.0, W_AREA)).expect("cooks");
        assert!(v3(&pt, AttrClass::Point).iter().all(|n| !AXES.contains(n)));
    }
    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/normal.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert_eq!(v3(&out, AttrClass::Vertex).len(), 24);
        assert_eq!(crate::registry().create("normal").map(|n| n.type_name()), Some("normal"));
    }
}
