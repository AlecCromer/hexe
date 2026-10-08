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
//! * Output has NO attributes besides `uv`. NOT modelled (DEFERRED): third (surface) input, generated tube/ribbon shapes
//!   (use `circle` / `line`), `up`/`N`/`orient` point attributes, `skinning` modes, per-point twist/scale ramps.
use super::meshutil::{V3, add, bad, cross, dot, positions, scale, sub, toggle};
use super::surfutil::{Curve, Mesh, arc_fractions, curves, normalize};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 2] = [InputSpec { name: "backbone" }, InputSpec { name: "crosssection" }];
const OP: &str = "sweep";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Scale").range(0.0, 10.0),
        ParamSpec::new("useptscale", ParamValue::B(true)).label("Use Point pscale"),
        ParamSpec::new("roll", ParamValue::F(0.0)).label("Roll").range(-360.0, 360.0),
        ParamSpec::new("twist", ParamValue::F(0.0)).label("Twist").range(-720.0, 720.0),
        ParamSpec::new("up", ParamValue::V3([0.0, 1.0, 0.0])).label("Up Vector"),
        ParamSpec::new("startcap", ParamValue::B(false)).label("Cap Start"),
        ParamSpec::new("endcap", ParamValue::B(false)).label("Cap End"),
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

/// Rotation-minimising normals: start from `up` (made orthogonal to `t[0]`), carry by double reflection.
fn normals(q: &[V3], t: &[V3], up: V3) -> Vec<V3> {
    let ortho = |v: V3, t: V3| normalize(sub(v, scale(t, dot(v, t))));
    let n0 = ortho(up, t[0]).or_else(|| {
        let axis = (0..3).min_by(|&a, &b| t[0][a].abs().total_cmp(&t[0][b].abs())).unwrap_or(0);
        let mut e = [0.0; 3];
        e[axis] = 1.0;
        ortho(e, t[0])
    });
    let mut n = vec![n0.unwrap_or([1.0, 0.0, 0.0])];
    for k in 1..q.len() {
        let r = n[k - 1];
        let v1 = sub(q[k], q[k - 1]);
        let c1 = dot(v1, v1);
        let (r_l, t_l) = if c1 > 1e-24 {
            (sub(r, scale(v1, 2.0 / c1 * dot(v1, r))), sub(t[k - 1], scale(v1, 2.0 / c1 * dot(v1, t[k - 1]))))
        } else {
            (r, t[k - 1])
        };
        let v2 = sub(t[k], t_l);
        let c2 = dot(v2, v2);
        let r2 = if c2 > 1e-24 { sub(r_l, scale(v2, 2.0 / c2 * dot(v2, r_l))) } else { r_l };
        n.push(ortho(r2, t[k]).unwrap_or(r));
    }
    n
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
        let (roll, twist) = (finite_f(p, "roll")?.to_radians(), finite_f(p, "twist")?.to_radians());
        let up = match p.vec3("up") {
            Some(v) if v.iter().all(|c| c.is_finite()) => v.map(f64::from),
            _ => return Err(bad("sweep: `up`: missing or not a finite vector")),
        };
        let (useps, startcap, endcap) = (toggle(p, "useptscale")?, toggle(p, "startcap")?, toggle(p, "endcap")?);
        let (Some(bb), Some(cs)) = (inputs.first(), inputs.get(1)) else { return Ok(Arc::new(Detail::default())) };
        let (bpos, spos) = (positions(bb), positions(cs));
        let pscale: Option<&Vec<f32>> = useps
            .then(|| {
                bb.attr(AttrClass::Point, "pscale")
                    .and_then(|a| if let AttrData::F32(v) = &a.data { Some(v) } else { None })
            })
            .flatten();
        let sections = curves(cs);
        let mut m = Mesh::default();
        for back in curves(bb) {
            let st = stations(&back, &bpos);
            if st.len() < 2 {
                continue;
            }
            let q: Vec<V3> = st.iter().map(|&i| bpos[i]).collect();
            let t = tangents(&q, back.closed);
            let nrm = normals(&q, &t, up);
            let vfr = arc_fractions(&q, back.closed);
            let rows = q.len();
            let segs = if back.closed { rows } else { rows - 1 };
            for sec in &sections {
                let local: Vec<V3> = sec.pts.iter().map(|&i| spos[i as usize]).collect();
                let cols = local.len();
                let ufr = arc_fractions(&local, sec.closed);
                // ring points, row-major
                let mut ids: Vec<u32> = Vec::with_capacity(rows * cols);
                for k in 0..rows {
                    let ang = roll + twist * vfr[k];
                    let (sn, cs_) = ang.sin_cos();
                    let n = add(scale(nrm[k], cs_), scale(cross(t[k], nrm[k]), sn));
                    let b = cross(t[k], n);
                    let s = sc * pscale.and_then(|v| v.get(st[k]).copied()).map_or(1.0, f64::from);
                    for l in &local {
                        let off = add(add(scale(n, l[0]), scale(b, l[1])), scale(t[k], l[2]));
                        ids.push(m.point(OP, add(q[k], scale(off, s)))?);
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
                            false,
                        )?;
                    }
                }
                if wrap_cols && cols >= 3 && !back.closed {
                    let cap_uv: Vec<[f32; 3]> = local.iter().map(|l| [l[0] as f32, l[1] as f32, 0.0]).collect();
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
        Ok(Arc::new(m.finish()))
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
        assert!(uv.iter().any(|c| c[1] == 1.0) && uv.iter().any(|c| c[0] == 1.0) && uv.iter().any(|c| *c == [0.0; 3]));
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
