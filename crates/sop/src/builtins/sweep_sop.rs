//! Sweep SOP (token `sweep`): sweep the curves of input 1 (cross-section) along the curves of input 0 (backbone).
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! * Backbone = every `Poly`/`Polyline` prim of input 0 with >= 2 distinct consecutive points (duplicates dropped; a closed
//!   backbone wraps). Cross-section = every curve prim of input 1, in ITS local frame: `(x, y, z)` -> `P + s*(x*N + y*B + z*T)`.
//!   Every backbone x every section is swept (cap: allocation cap -> `Failed`).
//! * Frame: tangent `T` = normalised mean of the adjacent unit segments (one-sided at open ends); `N` starts as `up` made
//!   orthogonal to `T` (an `up` parallel to `T` falls back to the least-aligned axis) and is carried along by parallel
//!   transport (double reflection, rotation-minimising); `B = T x N`, so `N x B = T` and a section wound CCW in `(x, y)` is
//!   CCW about `T`. A closed backbone does not correct the transport holonomy (seam can twist for non-planar loops).
//! * `scale` F 1 (>= 0) multiplies the section; `useptscale` B on additionally multiplies by the backbone point `pscale`.
//!   `roll` F 0 (degrees) rotates the section about `T`; `twist` F 0 (degrees) is the extra rotation reached at the far end
//!   (linear in backbone arclength).
//! * `startcap` / `endcap` B off: closed sections get one n-gon at the first / last station (open backbone only), wound so
//!   the Houdini normal faces away from the swept body.
//! * Side faces are quads wound so the Houdini normal is `ring direction x T` (outward for a CCW section): closed section +
//!   both caps = closed solid with positive signed volume. Vertex `uv` V3: `u` = section arclength fraction (1 at the
//!   wrap), `v` = backbone arclength fraction (1 at the wrap); cap uv = `(x, y, 0)`.
//! * QB additions: `surfaceshape` menu `input`(0) | `tube`(1: `cols` points, `radius`) | `square`(2: side `width`) |
//!   `ribbon`(3: open line of length `width`, along x) replaces the cross-section input with a generated one; `surftype`
//!   `quads`(0) | `triangles`(1: each side quad split along its A-C diagonal); `scaleend` F 1 = linear scale ramp over the
//!   backbone arclength (`1 + (scaleend-1)*v`); `useptattrs` B on honours backbone point attrs `scale` (F, multiplies),
//!   `twist` / `roll` (F degrees, added), `up` (V3, else `N`: per-point up hint replacing the transported normal) and
//!   `orient` (V4 quaternion xyzw: frame `(N,B,T)` = rotation of `(X,Y,Z)`, overrides everything); `carryattrs` B on copies
//!   every backbone point attr to the ring points of its station and every cross-section point attr (not shadowed by a
//!   backbone name) to the ring points of its column. Closed backbone: holonomy correction - the transport mismatch
//!   after one loop is spread linearly over the arclength, so the seam has no twist jump (planar loops: 0).
//! * NOT modelled (DEFERRED): third (surface) input to sweep, per-prim attr carry, ramp UI (2-key linear only).
use super::meshutil::{V3, add, bad, cross, dot, gather, positions, scale, sub, toggle};
use super::surfutil::{Curve, Mesh, arc_fractions, curves, normalize};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 2] = [InputSpec { name: "backbone" }, InputSpec { name: "crosssection" }];
const OP: &str = "sweep";
const SHAPE_INPUT: u32 = 0;
const SHAPE_TUBE: u32 = 1;
const SHAPE_SQUARE: u32 = 2;
const SHAPE_RIBBON: u32 = 3;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Scale").range(0.0, 10.0),
        ParamSpec::new("useptscale", ParamValue::B(true)).label("Use Point pscale"),
        ParamSpec::new("roll", ParamValue::F(0.0)).label("Roll").range(-360.0, 360.0),
        ParamSpec::new("twist", ParamValue::F(0.0)).label("Twist").range(-720.0, 720.0),
        ParamSpec::new("up", ParamValue::V3([0.0, 1.0, 0.0])).label("Up Vector"),
        ParamSpec::new("startcap", ParamValue::B(false)).label("Cap Start"),
        ParamSpec::new("endcap", ParamValue::B(false)).label("Cap End"),
        ParamSpec::new("surfaceshape", ParamValue::Menu(SHAPE_INPUT)).label("Surface Shape").menu(&[
            ("input", "Input Cross Section"),
            ("tube", "Round Tube"),
            ("square", "Square Tube"),
            ("ribbon", "Ribbon"),
        ]),
        ParamSpec::new("radius", ParamValue::F(0.5)).label("Radius").range(0.0, 10.0),
        ParamSpec::new("width", ParamValue::F(1.0)).label("Width").range(0.0, 10.0),
        ParamSpec::new("cols", ParamValue::I(8)).label("Columns").range(3.0, 64.0),
        ParamSpec::new("surftype", ParamValue::Menu(0))
            .label("Surface Type")
            .menu(&[("quads", "Quads"), ("triangles", "Triangles")]),
        ParamSpec::new("scaleend", ParamValue::F(1.0)).label("Scale At End").range(0.0, 10.0),
        ParamSpec::new("useptattrs", ParamValue::B(true)).label("Use Point Attributes"),
        ParamSpec::new("carryattrs", ParamValue::B(true)).label("Carry Point Attributes"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Sweep { specs: spec() })
}
struct Sweep {
    specs: Vec<ParamSpec>,
}

fn finite_f(p: &ParamSet, n: &str) -> Result<f64, CookError> {
    match p.float(n) {
        Some(v) if v.is_finite() => Ok(f64::from(v)),
        _ => Err(bad(format!("sweep: `{n}`: missing or not a finite float"))),
    }
}

/// Backbone stations: positions with consecutive duplicates (and a closed loop's repeated first point) removed.
fn stations(c: &Curve, pos: &[V3]) -> Vec<usize> {
    let mut s: Vec<usize> = Vec::with_capacity(c.pts.len());
    for &p in &c.pts {
        let p = p as usize;
        if s.last().is_none_or(|&l| pos[l] != pos[p]) {
            s.push(p);
        }
    }
    if c.closed && s.len() > 1 && pos[s[0]] == pos[s[s.len() - 1]] {
        s.pop();
    }
    s
}

/// Unit tangents, one per station.
fn tangents(q: &[V3], closed: bool) -> Vec<V3> {
    let m = q.len();
    let seg = |a: usize, b: usize| normalize(sub(q[b], q[a])).unwrap_or([0.0, 0.0, 1.0]);
    (0..m)
        .map(|k| {
            let inc = if k > 0 {
                Some(seg(k - 1, k))
            } else if closed {
                Some(seg(m - 1, 0))
            } else {
                None
            };
            let out = if k + 1 < m {
                Some(seg(k, k + 1))
            } else if closed {
                Some(seg(k, 0))
            } else {
                None
            };
            match (inc, out) {
                (Some(a), Some(b)) => normalize(add(a, b)).unwrap_or(a),
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => [0.0, 0.0, 1.0],
            }
        })
        .collect()
}

fn ortho(v: V3, t: V3) -> Option<V3> {
    normalize(sub(v, scale(t, dot(v, t))))
}
/// One parallel-transport step (double reflection): carry the normal `r` of station `a` to station `b`.
fn transport(r: V3, qa: V3, qb: V3, ta: V3, tb: V3) -> V3 {
    let v1 = sub(qb, qa);
    let c1 = dot(v1, v1);
    let (r_l, t_l) = if c1 > 1e-24 {
        (sub(r, scale(v1, 2.0 / c1 * dot(v1, r))), sub(ta, scale(v1, 2.0 / c1 * dot(v1, ta))))
    } else {
        (r, ta)
    };
    let v2 = sub(tb, t_l);
    let c2 = dot(v2, v2);
    let r2 = if c2 > 1e-24 { sub(r_l, scale(v2, 2.0 / c2 * dot(v2, r_l))) } else { r_l };
    ortho(r2, tb).unwrap_or(r)
}
/// Rotation-minimising normals: start from `up` (made orthogonal to `t[0]`), carry by double reflection.
fn normals(q: &[V3], t: &[V3], up: V3) -> Vec<V3> {
    let n0 = ortho(up, t[0]).or_else(|| {
        let axis = (0..3).min_by(|&a, &b| t[0][a].abs().total_cmp(&t[0][b].abs())).unwrap_or(0);
        let mut e = [0.0; 3];
        e[axis] = 1.0;
        ortho(e, t[0])
    });
    let mut n = vec![n0.unwrap_or([1.0, 0.0, 0.0])];
    for k in 1..q.len() {
        n.push(transport(n[k - 1], q[k - 1], q[k], t[k - 1], t[k]));
    }
    n
}
/// Rotate `v` about the unit `axis` by `a` radians (right-hand).
fn rotate_about(v: V3, axis: V3, a: f64) -> V3 {
    let (s, c) = a.sin_cos();
    add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)))
}
/// Closed backbone: the transported frame comes back to station 0 rotated by the holonomy angle `theta` about `t[0]`.
/// The inverse twist is spread linearly over the arclength (`vfr`, station fractions), so frame + distributed twist close
/// the loop exactly (no seam jump). Returns `theta` (0 for planar loops).
fn close_loop(q: &[V3], t: &[V3], n: &mut [V3], vfr: &[f64]) -> f64 {
    let m = q.len();
    if m < 3 {
        return 0.0;
    }
    let r_end = transport(n[m - 1], q[m - 1], q[0], t[m - 1], t[0]);
    let theta = dot(t[0], cross(r_end, n[0])).atan2(dot(r_end, n[0]));
    if !theta.is_finite() {
        return 0.0;
    }
    for k in 1..m {
        n[k] = rotate_about(n[k], t[k], theta * vfr[k]);
    }
    theta
}
/// Unit quaternion `(x, y, z, w)` rotation of `v` (non-unit normalised; zero quaternion = identity).
fn quat_rot(q: [f32; 4], v: V3) -> V3 {
    let l = q.iter().map(|c| f64::from(*c).powi(2)).sum::<f64>().sqrt();
    if !(l.is_finite() && l > 1e-12) {
        return v;
    }
    let (u, w) = ([f64::from(q[0]) / l, f64::from(q[1]) / l, f64::from(q[2]) / l], f64::from(q[3]) / l);
    let uv = cross(u, v);
    add(add(v, scale(uv, 2.0 * w)), scale(cross(u, uv), 2.0))
}
/// One cross-section: local `(x, y, z)` points, closedness, and the source cross-section point of each (generated = none).
struct Shape {
    local: Vec<V3>,
    closed: bool,
    src: Vec<Option<usize>>,
}
fn point_attr<'a>(d: &'a Detail, name: &str) -> Option<&'a AttrData> {
    d.attr(AttrClass::Point, name).map(|a| &a.data)
}
fn point_f32<'a>(d: &'a Detail, name: &str) -> Option<&'a Vec<f32>> {
    match point_attr(d, name) {
        Some(AttrData::F32(v)) => Some(v),
        _ => None,
    }
}
fn point_v3<'a>(d: &'a Detail, name: &str) -> Option<&'a Vec<[f32; 3]>> {
    match point_attr(d, name) {
        Some(AttrData::V3(v)) => Some(v),
        _ => None,
    }
}
fn point_v4<'a>(d: &'a Detail, name: &str) -> Option<&'a Vec<[f32; 4]>> {
    match point_attr(d, name) {
        Some(AttrData::V4(v)) => Some(v),
        _ => None,
    }
}
fn v3_at(v: Option<&Vec<[f32; 3]>>, i: usize) -> Option<V3> {
    let c = v?.get(i)?;
    let r = [f64::from(c[0]), f64::from(c[1]), f64::from(c[2])];
    r.iter().all(|x| x.is_finite()).then_some(r)
}
fn generated(shape: u32, radius: f64, width: f64, cols: usize) -> Shape {
    let local: Vec<V3> = match shape {
        SHAPE_TUBE => (0..cols)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / cols as f64;
                [radius * a.cos(), radius * a.sin(), 0.0]
            })
            .collect(),
        SHAPE_SQUARE => {
            let h = width / 2.0;
            vec![[-h, -h, 0.0], [h, -h, 0.0], [h, h, 0.0], [-h, h, 0.0]]
        }
        _ => vec![[-width / 2.0, 0.0, 0.0], [width / 2.0, 0.0, 0.0]],
    };
    let closed = shape != SHAPE_RIBBON;
    let n = local.len();
    Shape { local, closed, src: vec![None; n] }
}
impl Node<Detail> for Sweep {
    fn type_name(&self) -> &'static str {
        OP
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let sc = finite_f(p, "scale")?;
        if sc < 0.0 {
            return Err(bad("sweep: `scale` must be >= 0"));
        }
        let scale_end = finite_f(p, "scaleend")?;
        if scale_end < 0.0 {
            return Err(bad("sweep: `scaleend` must be >= 0"));
        }
        let (roll, twist) = (finite_f(p, "roll")?.to_radians(), finite_f(p, "twist")?.to_radians());
        let up = match p.vec3("up") {
            Some(v) if v.iter().all(|c| c.is_finite()) => v.map(f64::from),
            _ => return Err(bad("sweep: `up`: missing or not a finite vector")),
        };
        let (useps, startcap, endcap) = (toggle(p, "useptscale")?, toggle(p, "startcap")?, toggle(p, "endcap")?);
        let (useattrs, carry) = (toggle(p, "useptattrs")?, toggle(p, "carryattrs")?);
        let shape = match p.menu("surfaceshape") {
            Some(v) if v <= SHAPE_RIBBON => v,
            Some(v) => return Err(bad(format!("sweep: `surfaceshape`: menu index {v} out of range 0..=3"))),
            None => return Err(bad("sweep: `surfaceshape`: missing or not a menu")),
        };
        let tris = match p.menu("surftype") {
            Some(0) => false,
            Some(1) => true,
            Some(v) => return Err(bad(format!("sweep: `surftype`: menu index {v} out of range 0..=1"))),
            None => return Err(bad("sweep: `surftype`: missing or not a menu")),
        };
        let (radius, width) = (finite_f(p, "radius")?, finite_f(p, "width")?);
        if radius < 0.0 || width < 0.0 {
            return Err(bad("sweep: `radius` / `width` must be >= 0"));
        }
        let cols = match p.int("cols") {
            Some(c) if (3..=4096).contains(&c) => c as usize,
            _ => return Err(bad("sweep: `cols` must be an int in 3..=4096")),
        };
        let Some(bb) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let bpos = positions(bb);
        let sections: Vec<Shape> = if shape == SHAPE_INPUT {
            let Some(cs) = inputs.get(1) else { return Ok(Arc::new(Detail::default())) };
            let spos = positions(cs);
            curves(cs)
                .into_iter()
                .map(|c| Shape {
                    local: c.pts.iter().map(|&i| spos[i as usize]).collect(),
                    closed: c.closed,
                    src: c.pts.iter().map(|&i| Some(i as usize)).collect(),
                })
                .collect()
        } else {
            vec![generated(shape, radius, width, cols)]
        };
        let pscale = if useps { point_f32(bb, "pscale") } else { None };
        let (a_scale, a_twist, a_roll) = if useattrs {
            (point_f32(bb, "scale"), point_f32(bb, "twist"), point_f32(bb, "roll"))
        } else {
            (None, None, None)
        };
        let a_up = if useattrs { point_v3(bb, "up").or_else(|| point_v3(bb, "N")) } else { None };
        let a_orient = if useattrs { point_v4(bb, "orient") } else { None };
        let at =
            |v: Option<&Vec<f32>>, i: usize| v.and_then(|v| v.get(i)).map(|c| f64::from(*c)).filter(|c| c.is_finite());
        let mut m = Mesh::default();
        let (mut src_b, mut src_s): (Vec<Option<usize>>, Vec<Option<usize>>) = (Vec::new(), Vec::new());
        for back in curves(bb) {
            let st = stations(&back, &bpos);
            if st.len() < 2 {
                continue;
            }
            let q: Vec<V3> = st.iter().map(|&i| bpos[i]).collect();
            let mut t = tangents(&q, back.closed);
            let mut nrm = normals(&q, &t, up);
            let vfr = arc_fractions(&q, back.closed);
            let rows = q.len();
            let segs = if back.closed { rows } else { rows - 1 };
            if a_up.is_some() {
                for k in 0..rows {
                    if let Some(n) = v3_at(a_up, st[k]).and_then(|u| ortho(u, t[k])) {
                        nrm[k] = n;
                    }
                }
            }
            if back.closed && a_up.is_none() && a_orient.is_none() {
                close_loop(&q, &t, &mut nrm, &vfr);
            }
            if let Some(o) = a_orient {
                for k in 0..rows {
                    if let Some(c) = o.get(st[k])
                        && c.iter().all(|x| x.is_finite())
                        && c.iter().any(|x| *x != 0.0)
                    {
                        t[k] = quat_rot(*c, [0.0, 0.0, 1.0]);
                        nrm[k] = quat_rot(*c, [1.0, 0.0, 0.0]);
                    }
                }
            }
            for sec in &sections {
                let cols = sec.local.len();
                let ufr = arc_fractions(&sec.local, sec.closed);
                // ring points, row-major
                let mut ids: Vec<u32> = Vec::with_capacity(rows * cols);
                for k in 0..rows {
                    let ang = roll
                        + twist * vfr[k]
                        + (at(a_roll, st[k]).unwrap_or(0.0) + at(a_twist, st[k]).unwrap_or(0.0)).to_radians();
                    let (sn, cs_) = ang.sin_cos();
                    let n = add(scale(nrm[k], cs_), scale(cross(t[k], nrm[k]), sn));
                    let b = cross(t[k], n);
                    let ramp = 1.0 + (scale_end - 1.0) * vfr[k];
                    let s = sc
                        * pscale.and_then(|v| v.get(st[k]).copied()).map_or(1.0, f64::from)
                        * at(a_scale, st[k]).unwrap_or(1.0)
                        * ramp;
                    for (l, ls) in sec.local.iter().zip(&sec.src) {
                        let off = add(add(scale(n, l[0]), scale(b, l[1])), scale(t[k], l[2]));
                        ids.push(m.point(OP, add(q[k], scale(off, s)))?);
                        src_b.push(Some(st[k]));
                        src_s.push(*ls);
                    }
                }
                let wrap_cols = sec.closed;
                let ring_segs = if wrap_cols { cols } else { cols - 1 };
                for i in 0..segs {
                    let i2 = (i + 1) % rows;
                    let (v0, v1) = (vfr[i] as f32, if i + 1 == rows { 1.0 } else { vfr[i + 1] as f32 });
                    for j in 0..ring_segs {
                        let j2 = (j + 1) % cols;
                        let (u0, u1) = (ufr[j] as f32, if j + 1 == cols { 1.0 } else { ufr[j + 1] as f32 });
                        let at = |r: usize, c: usize| ids[r * cols + c];
                        // A=(i,j) B=(i,j2) C=(i2,j2) D=(i2,j); wound A,D,C,B
                        m.quad(
                            OP,
                            [at(i, j), at(i2, j), at(i2, j2), at(i, j2)],
                            [[u0, v0, 0.0], [u0, v1, 0.0], [u1, v1, 0.0], [u1, v0, 0.0]],
                            tris,
                        )?;
                    }
                }
                if wrap_cols && cols >= 3 && !back.closed {
                    let cap_uv: Vec<[f32; 3]> = sec.local.iter().map(|l| [l[0] as f32, l[1] as f32, 0.0]).collect();
                    if startcap {
                        m.poly(OP, &ids[..cols], &cap_uv)?;
                    }
                    if endcap {
                        let last: Vec<u32> = ids[(rows - 1) * cols..].iter().rev().copied().collect();
                        let uv: Vec<[f32; 3]> = cap_uv.iter().rev().copied().collect();
                        m.poly(OP, &last, &uv)?;
                    }
                }
            }
        }
        let mut out = m.finish();
        if carry {
            let mut taken: Vec<String> = vec!["P".into()];
            for a in bb.attrs.iter().filter(|a| a.class == AttrClass::Point && a.name != "P") {
                out.add_attr(Attribute {
                    name: a.name.clone(),
                    class: AttrClass::Point,
                    data: gather(&a.data, &src_b),
                    type_info: a.type_info,
                });
                taken.push(a.name.clone());
            }
            if shape == SHAPE_INPUT
                && let Some(cs) = inputs.get(1)
            {
                for a in cs.attrs.iter().filter(|a| a.class == AttrClass::Point && !taken.contains(&a.name)) {
                    out.add_attr(Attribute {
                        name: a.name.clone(),
                        class: AttrClass::Point,
                        data: gather(&a.data, &src_s),
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
    use super::super::meshutil::{positions, prim_normal, testkit};
    use super::super::surfutil::testkit::{curve, signed_volume};
    use super::*;
    use crate::load_scene;

    fn run(back: Detail, sec: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        n.cook(&ctx, &[Arc::new(back), Arc::new(sec)])
    }
    fn defaults() -> ParamSet {
        testkit::defaults(&*new())
    }
    fn square() -> Detail {
        curve(&[[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.5, 0.5, 0.0], [-0.5, 0.5, 0.0]], true)
    }
    fn zline() -> Detail {
        curve(&[[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 2.0]], false)
    }
    fn caps(p: ParamSet) -> ParamSet {
        p.with("startcap", ParamValue::B(true)).with("endcap", ParamValue::B(true))
    }

    #[test]
    fn square_along_z_with_caps_is_a_closed_outward_solid_of_volume_two() {
        let d = run(zline(), square(), caps(defaults())).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (12, 8 + 2));
        let v = signed_volume(&d);
        assert!((v - 2.0).abs() < 1e-5, "volume {v}");
        // uncapped: 8 side quads only, every normal perpendicular to the backbone
        let o = run(zline(), square(), defaults()).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (12, 8));
        let pos = positions(&o);
        for pr in &o.prims {
            let ids: Vec<usize> =
                o.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&i| i as usize).collect();
            let n = prim_normal(&pos, &ids).expect("normal");
            assert!(n[2].abs() < 1e-9);
            // outward = away from the axis
            let c: V3 = ids.iter().fold([0.0; 3], |a, &i| add(a, scale(pos[i], 0.25)));
            assert!(n[0] * c[0] + n[1] * c[1] > 0.0);
        }
    }

    #[test]
    fn frame_follows_up_and_section_is_in_n_b_plane() {
        // T=+Z, up=+Y -> N=+Y, B=T x N=-X: section point (x,y) -> (-y, x)... (x*N + y*B) = (-y, x, 0)
        let sec = curve(&[[1.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 0.0]], false);
        let d = run(zline(), sec, defaults()).expect("cooks");
        let p = d.p();
        assert_eq!(p[0], [0.0, 1.0, 0.0]); // x=1 -> N=+Y
        assert_eq!(p[1], [-2.0, 0.0, 0.0]); // y=2 -> B=-X
        // a degenerate up (parallel to T) still builds a frame
        let d = run(zline(), square(), defaults().with("up", ParamValue::V3([0.0, 0.0, 1.0]))).expect("cooks");
        assert!(d.p().iter().flatten().all(|c| c.is_finite()) && d.prim_count() == 8);
    }

    #[test]
    fn bent_backbone_keeps_sections_perpendicular_and_circular_scale_roll_twist_apply() {
        // L-shaped backbone in the XZ plane: ring stays in the plane perpendicular to the (mitred) tangent
        let back = curve(&[[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]], false);
        let d = run(back, square(), defaults()).expect("cooks");
        let p = d.p();
        // end station: tangent +X -> ring in the YZ plane about (1,0,1)
        assert!(p[8..12].iter().all(|q| (q[0] - 1.0).abs() < 1e-6));
        // scale 2 doubles the ring, pscale multiplies on top
        let mut b = zline();
        b.add_attr(hexe_geo::Attribute {
            name: "pscale".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![1.0, 3.0, 1.0]),
            type_info: hexe_geo::TypeInfo::None,
        });
        let s = run(b.clone(), square(), defaults().with("scale", ParamValue::F(2.0))).expect("cooks");
        assert!((s.p()[0][1].abs() - 1.0).abs() < 1e-6 && (s.p()[4][1].abs() - 3.0).abs() < 1e-6, "{:?}", s.p());
        let off = run(b, square(), defaults().with("useptscale", ParamValue::B(false))).expect("cooks");
        assert!((off.p()[4][1].abs() - 0.5).abs() < 1e-6);
        // roll 90 turns the first corner (-0.5,-0.5) -> N x B quarter turn
        let r = run(zline(), square(), defaults().with("roll", ParamValue::F(90.0))).expect("cooks");
        let q = run(zline(), square(), defaults()).expect("cooks");
        assert!(r.p()[0] != q.p()[0]);
        // twist 90 over the length: the first ring is untouched, the last is rolled by a quarter turn
        let t = run(zline(), square(), defaults().with("twist", ParamValue::F(90.0))).expect("cooks");
        assert_eq!(&t.p()[..4], &q.p()[..4]);
        let (a, b) = (t.p()[8], q.p()[8]);
        assert!(((a[0] * a[0] + a[1] * a[1]).sqrt() - (b[0] * b[0] + b[1] * b[1]).sqrt()).abs() < 1e-6 && a != b);
    }

    #[test]
    fn closed_backbone_wraps_into_a_closed_torus_and_ignores_caps() {
        let ring = curve(&[[-2.0, 0.0, -2.0], [2.0, 0.0, -2.0], [2.0, 0.0, 2.0], [-2.0, 0.0, 2.0]], true);
        let d = run(ring, square(), caps(defaults())).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (16, 16));
        assert!(signed_volume(&d).abs() > 1.0);
    }

    #[test]
    fn open_section_makes_a_ribbon_and_uv_runs_zero_to_one() {
        let sec = curve(&[[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]], false);
        let d = run(zline(), sec, defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (6, 2));
        let uv = match d.attr(AttrClass::Vertex, "uv").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            o => panic!("{o:?}"),
        };
        assert_eq!(uv.len(), d.vertex_point.len());
        assert!(uv.iter().all(|c| (0.0..=1.0).contains(&c[0]) && (0.0..=1.0).contains(&c[1])));
        assert!(uv.iter().any(|c| c[1] == 1.0) && uv.iter().any(|c| c[0] == 1.0) && uv.contains(&[0.0; 3]));
    }

    #[test]
    fn hostile_params_and_geometry() {
        for (k, v) in [
            ("scale", ParamValue::F(f32::NAN)),
            ("scale", ParamValue::F(-1.0)),
            ("roll", ParamValue::F(f32::INFINITY)),
            ("up", ParamValue::V3([f32::NAN, 0.0, 0.0])),
            ("startcap", ParamValue::I(1)),
        ] {
            assert!(matches!(run(zline(), square(), defaults().with(k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        // empty / single-point / coincident backbones and sections cook to nothing, never panic
        for b in [Detail::default(), curve(&[[0.0; 3]], false), curve(&[[1.0; 3], [1.0; 3]], false)] {
            assert_eq!(run(b, square(), defaults()).expect("cooks").prim_count(), 0);
        }
        assert_eq!(run(zline(), Detail::default(), defaults()).expect("cooks").prim_count(), 0);
        let mut dangling = zline();
        dangling.vertex_point[0] = 99;
        assert_eq!(run(dangling, square(), defaults()).expect("cooks").prim_count(), 0);
        let mut huge = zline();
        huge.p_mut()[1] = [f32::MAX, f32::MAX, f32::MAX];
        let r = run(huge, square(), defaults().with("scale", ParamValue::F(f32::MAX)));
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
    }

    fn with_attr(mut d: Detail, name: &str, data: AttrData) -> Detail {
        d.add_attr(Attribute { name: name.into(), class: AttrClass::Point, data, type_info: hexe_geo::TypeInfo::None });
        d
    }
    fn shape(s: u32, p: ParamSet) -> ParamSet {
        p.with("surfaceshape", ParamValue::Menu(s))
    }

    #[test]
    fn generated_tube_square_ribbon_have_analytic_volume_and_counts() {
        // 8-gon tube r=1 along z length 2 with caps: V = 1/2 * 8 * r^2 * sin(45deg) * 2
        let p = caps(shape(SHAPE_TUBE, defaults().with("radius", ParamValue::F(1.0))));
        let d = run(zline(), Detail::default(), p.clone()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (24, 16 + 2));
        let want = 0.5 * 8.0 * (std::f64::consts::PI / 4.0).sin() * 2.0;
        assert!((signed_volume(&d) - want).abs() < 1e-5, "{} vs {want}", signed_volume(&d));
        // cols = 4 -> a square of area 2 (diamond r=1): V = 2*2
        let d4 = run(zline(), Detail::default(), p.with("cols", ParamValue::I(4))).expect("cooks");
        assert!((signed_volume(&d4) - 4.0).abs() < 1e-5);
        // square tube width 2 -> V = 4 * 2 = 8
        let s =
            run(zline(), Detail::default(), caps(shape(SHAPE_SQUARE, defaults().with("width", ParamValue::F(2.0)))))
                .expect("cooks");
        assert_eq!((s.point_count, s.prim_count()), (12, 10));
        assert!((signed_volume(&s) - 8.0).abs() < 1e-5);
        // ribbon width 2: open section -> 2 quads, x spans [-1, 1]
        let r = run(zline(), Detail::default(), shape(SHAPE_RIBBON, defaults().with("width", ParamValue::F(2.0))))
            .expect("cooks");
        assert_eq!((r.point_count, r.prim_count()), (6, 2));
        assert!(r.p().iter().all(|q| q[0].abs() <= 1.0 + 1e-6 || q[1].abs() <= 1.0 + 1e-6));
        assert!(r.p().iter().map(|q| q[0].abs().max(q[1].abs())).fold(0.0_f32, f32::max) > 0.999);
        // out-of-range menu / cols are BadParam
        for (k, v) in
            [("surfaceshape", ParamValue::Menu(4)), ("cols", ParamValue::I(2)), ("surftype", ParamValue::Menu(2))]
        {
            assert!(matches!(run(zline(), square(), defaults().with(k, v)), Err(CookError::BadParam(_))), "{k}");
        }
    }

    #[test]
    fn surftype_triangles_double_the_side_prims_and_keep_the_volume() {
        let q = run(zline(), square(), caps(defaults())).expect("cooks");
        let t = run(zline(), square(), caps(defaults()).with("surftype", ParamValue::Menu(1))).expect("cooks");
        assert_eq!((q.prim_count(), t.prim_count()), (10, 16 + 2));
        assert!((signed_volume(&q) - signed_volume(&t)).abs() < 1e-5);
    }

    #[test]
    fn backbone_and_section_point_attrs_are_carried_to_ring_points() {
        let back = with_attr(zline(), "id", AttrData::I32(vec![10, 20, 30]));
        let sec = with_attr(square(), "w", AttrData::F32(vec![0.5, 1.5, 2.5, 3.5]));
        let d = run(back, sec, defaults()).expect("cooks");
        let Some(AttrData::I32(id)) = d.attr(AttrClass::Point, "id").map(|a| a.data.clone()) else { panic!("id") };
        let Some(AttrData::F32(w)) = d.attr(AttrClass::Point, "w").map(|a| a.data.clone()) else { panic!("w") };
        assert_eq!((id.len(), w.len()), (12, 12));
        for k in 0..3 {
            for j in 0..4 {
                assert_eq!(id[k * 4 + j], 10 * (k as i32 + 1));
                assert_eq!(w[k * 4 + j], j as f32 + 0.5);
            }
        }
        // carry off: nothing extra
        let n = run(zline(), square(), defaults().with("carryattrs", ParamValue::B(false))).expect("cooks");
        assert!(n.attr(AttrClass::Point, "id").is_none());
    }

    #[test]
    fn scale_ramp_and_per_point_scale_twist_attrs() {
        // scaleend 3: far ring = 3x, mid ring = 2x (unit square corner |c| = 0.5)
        let d = run(zline(), square(), defaults().with("scaleend", ParamValue::F(3.0))).expect("cooks");
        let r = |i: usize| d.p()[i][0].abs();
        assert!((r(0) - 0.5).abs() < 1e-6 && (r(4) - 1.0).abs() < 1e-6 && (r(8) - 1.5).abs() < 1e-6, "{:?}", d.p());
        // per-point `scale` attr multiplies
        let b = with_attr(zline(), "scale", AttrData::F32(vec![1.0, 4.0, 1.0]));
        let s = run(b, square(), defaults()).expect("cooks");
        assert!((s.p()[4][0].abs() - 2.0).abs() < 1e-6 && (s.p()[0][0].abs() - 0.5).abs() < 1e-6);
        // `twist` attr = 90 at the far station only: mid ring untwisted, far ring == twist param 90 over the length
        let b = with_attr(zline(), "twist", AttrData::F32(vec![0.0, 0.0, 90.0]));
        let a = run(b, square(), defaults()).expect("cooks");
        let plain = run(zline(), square(), defaults()).expect("cooks");
        let param = run(zline(), square(), defaults().with("twist", ParamValue::F(90.0))).expect("cooks");
        assert_eq!(&a.p()[..8], &plain.p()[..8]);
        for i in 8..12 {
            for c in 0..3 {
                assert!((a.p()[i][c] - param.p()[i][c]).abs() < 1e-6);
            }
        }
        // useptattrs off ignores them
        let b = with_attr(zline(), "scale", AttrData::F32(vec![1.0, 4.0, 1.0]));
        let o = run(b, square(), defaults().with("useptattrs", ParamValue::B(false))).expect("cooks");
        assert!((o.p()[4][0].abs() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn up_and_orient_point_attrs_set_the_frame() {
        let sec = curve(&[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], false);
        // default frame: x -> N = +Y
        let base = run(zline(), sec.clone(), defaults()).expect("cooks");
        assert_eq!(base.p()[0], [0.0, 1.0, 0.0]);
        // per-point up = +X -> N = +X
        let up = with_attr(zline(), "up", AttrData::V3(vec![[1.0, 0.0, 0.0]; 3]));
        let u = run(up, sec.clone(), defaults()).expect("cooks");
        assert_eq!(u.p()[0], [1.0, 0.0, 0.0]);
        assert_eq!(u.p()[1], [0.0, 1.0, 0.0]); // y -> B = T x N = Z x X = +Y
        // orient = 180 deg about Z: N = -X, B = T x N = -Y
        let o = with_attr(zline(), "orient", AttrData::V4(vec![[0.0, 0.0, 1.0, 0.0]; 3]));
        let r = run(o.clone(), sec.clone(), defaults()).expect("cooks");
        let (a, b) = (r.p()[0], r.p()[1]);
        assert!((a[0] + 1.0).abs() < 1e-6 && a[1].abs() < 1e-6, "{a:?}");
        assert!(b[0].abs() < 1e-6 && (b[1] + 1.0).abs() < 1e-6, "{b:?}");
        // switched off -> default frame
        let off = run(o, sec, defaults().with("useptattrs", ParamValue::B(false))).expect("cooks");
        assert_eq!(off.p()[0], [0.0, 1.0, 0.0]);
    }

    #[test]
    fn closed_loop_holonomy_is_corrected_so_frame_plus_distributed_twist_closes() {
        // skew closed loop: plain transport comes back rotated
        let q: Vec<V3> = vec![[2.0, 0.0, 0.0], [0.0, 2.0, 1.0], [-2.0, 0.0, 2.0], [0.0, -3.0, 0.5], [1.0, -1.0, -1.0]];
        let t = tangents(&q, true);
        let mut n = normals(&q, &t, [0.0, 1.0, 0.0]);
        let vfr = arc_fractions(&q, true);
        let m = q.len();
        let before = transport(n[m - 1], q[m - 1], q[0], t[m - 1], t[0]);
        let miss = dot(t[0], cross(before, n[0])).atan2(dot(before, n[0]));
        assert!(miss.abs() > 1e-3, "loop must have holonomy, got {miss}");
        let theta = close_loop(&q, &t, &mut n, &vfr);
        assert!((theta - miss).abs() < 1e-12);
        // closing step: transport of the corrected last normal + the last segment's share of the twist == n[0]
        let back = transport(n[m - 1], q[m - 1], q[0], t[m - 1], t[0]);
        let back = rotate_about(back, t[0], theta * (1.0 - vfr[m - 1]));
        for c in 0..3 {
            assert!((back[c] - n[0][c]).abs() < 1e-9, "{back:?} vs {:?}", n[0]);
        }
        // normals stay unit and perpendicular to the tangent
        for k in 0..m {
            assert!((dot(n[k], n[k]) - 1.0).abs() < 1e-9 && dot(n[k], t[k]).abs() < 1e-9);
        }
        // planar loop: no correction
        let sq: Vec<V3> = vec![[-2.0, 0.0, -2.0], [2.0, 0.0, -2.0], [2.0, 0.0, 2.0], [-2.0, 0.0, 2.0]];
        let ts = tangents(&sq, true);
        let mut ns = normals(&sq, &ts, [0.0, 1.0, 0.0]);
        assert!(close_loop(&sq, &ts, &mut ns, &arc_fractions(&sq, true)).abs() < 1e-9);
        // and the whole cook of the skew loop is a closed finite torus of 16 quads
        let skew = curve(&[[2.0, 0.0, 0.0], [0.0, 2.0, 1.0], [-2.0, 0.0, 0.0], [0.0, -2.0, 1.0]], true);
        let d = run(skew, square(), defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (16, 16));
        assert!(d.p().iter().flatten().all(|c| c.is_finite()));
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/sweep.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert!(out.prim_count() > 100 && signed_volume(&out) > 0.0);
        assert_eq!(crate::registry().create("sweep").map(|n| n.type_name()), Some("sweep"));
    }
}
