//! Revolve SOP (token `revolve`): spin the curves of input 0 about an axis into a quad surface.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! * Params (Houdini names/defaults): `origin` V3 (0,0,0), `dir` V3 (0,1,0) axis direction (zero / non-finite -> `BadParam`),
//!   `startangle` F 0, `endangle` F 360 (degrees; right-hand rotation about `dir`), `divs` I 12 (>= 1; >= 3 for a full turn).
//!   Optional `tris` B off: split every quad into two triangles.
//! * A sweep of `|endangle - startangle| >= 360` is a FULL turn: `divs` columns, the seam wraps (no duplicated points). Else
//!   `divs + 1` columns (partial surface, open). A negative span turns the other way.
//! * Curves = every `Poly`/`Polyline` prim with >= 2 corners (closed ones wrap their last segment). A profile point on the axis
//!   (radial distance < 1e-6) becomes ONE shared point, so quads touching it collapse to triangles (a half circle profile
//!   revolves into a closed sphere); a segment lying on the axis emits nothing.
//! * Quad `(i, j)` = `[(i,j), (i+1,j), (i+1,j+1), (i,j+1)]`: for a profile running along `+axis` at positive radius the Houdini
//!   normal points away from the axis (full revolve of an upward profile = outward-facing shell).
//! * Output has only `uv` (vertex V3): `u` = column / divs (0..1), `v` = profile arclength fraction. No attribute carry.
//!   NOT modelled (DEFERRED): caps, NURBS/Bezier output, `rotate` about a local frame per point, attribute inheritance.
use super::meshutil::{V3, add, bad, cross, dot, positions, scale, sub, toggle};
use super::surfutil::{Mesh, arc_fractions, curves, normalize};
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const OP: &str = "revolve";
const AXIS_EPS: f64 = 1e-6;
/// Divisions cap (a single curve point already costs `divs` points).
const MAX_DIVS: i32 = 1 << 16;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("origin", ParamValue::V3([0.0; 3])).label("Origin"),
        ParamSpec::new("dir", ParamValue::V3([0.0, 1.0, 0.0])).label("Direction"),
        ParamSpec::new("startangle", ParamValue::F(0.0)).label("Start Angle").range(0.0, 360.0),
        ParamSpec::new("endangle", ParamValue::F(360.0)).label("End Angle").range(0.0, 360.0),
        ParamSpec::new("divs", ParamValue::I(12)).label("Divisions").range(1.0, 100.0),
        ParamSpec::new("tris", ParamValue::B(false)).label("Triangles"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Revolve { specs: spec() })
}
struct Revolve {
    specs: Vec<ParamSpec>,
}

fn vec3(p: &ParamSet, n: &str) -> Result<V3, CookError> {
    match p.vec3(n) {
        Some(v) if v.iter().all(|c| c.is_finite()) => Ok(v.map(f64::from)),
        _ => Err(bad(format!("revolve: `{n}`: missing or not a finite vector"))),
    }
}
fn angle(p: &ParamSet, n: &str) -> Result<f64, CookError> {
    match p.float(n) {
        Some(v) if v.is_finite() => Ok(f64::from(v)),
        _ => Err(bad(format!("revolve: `{n}`: missing or not a finite float"))),
    }
}

impl Node<Detail> for Revolve {
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
        let origin = vec3(p, "origin")?;
        let axis = normalize(vec3(p, "dir")?).ok_or_else(|| bad("revolve: `dir` must be a non-zero vector"))?;
        let (a0, a1) = (angle(p, "startangle")?, angle(p, "endangle")?);
        let divs = p.int("divs").ok_or_else(|| bad("revolve: `divs`: missing or not an int"))?;
        let tris = toggle(p, "tris")?;
        let span = a1 - a0;
        let full = span.abs() >= 360.0 - 1e-9;
        if divs < if full { 3 } else { 1 } || divs > MAX_DIVS {
            return Err(bad(format!("revolve: `divs` {divs} out of range ({}..={MAX_DIVS})", if full { 3 } else { 1 })));
        }
        let divs = divs as usize;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = positions(src);
        let cols = if full { divs } else { divs + 1 };
        let sweep = if full { 360.0f64.copysign(span) } else { span };
        let mut m = Mesh::default();
        for c in curves(src) {
            let q: Vec<V3> = c.pts.iter().map(|&i| pos[i as usize]).collect();
            let vfr = arc_fractions(&q, c.closed);
            // ids[i][j]
            let mut ids: Vec<Vec<u32>> = Vec::with_capacity(q.len());
            for pt in &q {
                let rel = sub(*pt, origin);
                let h = dot(rel, axis);
                let radial = sub(rel, scale(axis, h));
                if dot(radial, radial).sqrt() < AXIS_EPS {
                    let id = m.point(OP, add(origin, scale(axis, h)))?;
                    ids.push(vec![id; cols]);
                    continue;
                }
                let mut row = Vec::with_capacity(cols);
                for j in 0..cols {
                    let th = (a0 + sweep * j as f64 / divs as f64).to_radians();
                    let (s, co) = th.sin_cos();
                    let rot = add(add(scale(radial, co), scale(cross(axis, radial), s)), scale(axis, h));
                    row.push(m.point(OP, add(origin, rot))?);
                }
                ids.push(row);
            }
            let segs = if c.closed { q.len() } else { q.len() - 1 };
            let ncols = if full { divs } else { divs };
            for i in 0..segs {
                let i2 = (i + 1) % q.len();
                let (v0, v1) = (vfr[i] as f32, if i + 1 == q.len() { 1.0 } else { vfr[i + 1] as f32 });
                for j in 0..ncols {
                    let j2 = (j + 1) % cols;
                    let (u0, u1) = (j as f32 / divs as f32, (j + 1) as f32 / divs as f32);
                    m.quad(
                        OP,
                        [ids[i][j], ids[i2][j], ids[i2][j2], ids[i][j2]],
                        [[u0, v0, 0.0], [u0, v1, 0.0], [u1, v1, 0.0], [u1, v0, 0.0]],
                        tris,
                    )?;
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
    use hexe_geo::{AttrClass, AttrData};

    fn run(d: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        n.cook(&ctx, &[Arc::new(d)])
    }
    fn defaults() -> ParamSet {
        testkit::defaults(&*new())
    }
    fn wall() -> Detail {
        curve(&[[1.0, 0.0, 0.0], [1.0, 1.0, 0.0]], false)
    }
    fn half_circle() -> Detail {
        let pts: Vec<[f32; 3]> = (0..=6).map(|k| {
            let phi = std::f32::consts::PI * (1.0 - k as f32 / 6.0);
            [phi.sin(), phi.cos(), 0.0]
        }).collect();
        curve(&pts, false)
    }

    #[test]
    fn full_revolve_of_a_wall_is_an_outward_cylinder_wall_with_a_wrapped_seam() {
        let d = run(wall(), defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (24, 12));
        let pos = positions(&d);
        for pr in &d.prims {
            let ids: Vec<usize> = d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&i| i as usize).collect();
            let n = prim_normal(&pos, &ids).expect("normal");
            let c: V3 = ids.iter().fold([0.0; 3], |a, &i| add(a, scale(pos[i], 0.25)));
            assert!(n[1].abs() < 1e-9 && n[0] * c[0] + n[2] * c[2] > 0.0, "{n:?} {c:?}");
        }
        // right-hand rotation about +Y: +X -> -Z
        let q = run(wall(), defaults().with("divs", ParamValue::I(4))).expect("cooks");
        let p = q.p();
        assert!((p[1][0]).abs() < 1e-6 && (p[1][2] + 1.0).abs() < 1e-6, "{:?}", p[1]);
    }

    #[test]
    fn half_circle_profile_makes_a_closed_sphere_with_shared_pole_points() {
        let d = run(half_circle(), defaults().with("divs", ParamValue::I(24))).expect("cooks");
        // 2 poles + 5 rings x 24; polar bands are triangles
        assert_eq!(d.point_count, 2 + 5 * 24);
        assert_eq!(d.prim_count(), 24 + 4 * 24 + 24);
        let v = signed_volume(&d);
        let ball = 4.0 / 3.0 * std::f64::consts::PI;
        assert!(v > 0.0 && (v - ball).abs() / ball < 0.1, "{v}");
        let tri = run(half_circle(), defaults().with("tris", ParamValue::B(true))).expect("cooks");
        assert!(tri.prims.iter().all(|p| p.verts.len() == 3));
    }

    #[test]
    fn partial_angle_origin_and_axis_params() {
        let d = run(wall(), defaults().with("endangle", ParamValue::F(90.0)).with("divs", ParamValue::I(4))).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (10, 4));
        // axis dir=Z through origin (1,0,0): the wall is now ON the axis? (1,0,0)-(1,1,0) is perpendicular -> second point swings
        let z = run(wall(), defaults().with("dir", ParamValue::V3([0.0, 0.0, 1.0])).with("origin", ParamValue::V3([1.0, 0.0, 0.0]))).expect("cooks");
        assert_eq!((z.point_count, z.prim_count()), (13, 12));
        assert!(z.prims.iter().all(|p| p.verts.len() == 3));
        // a wall lying ON the axis emits nothing
        let on = run(curve(&[[0.0; 3], [0.0, 1.0, 0.0]], false), defaults()).expect("cooks");
        assert_eq!((on.point_count, on.prim_count()), (2, 0));
        // negative span turns the other way: +X -> +Z after a quarter
        let r = run(wall(), defaults().with("startangle", ParamValue::F(0.0)).with("endangle", ParamValue::F(-90.0)).with("divs", ParamValue::I(1))).expect("cooks");
        assert!((r.p()[1][2] - 1.0).abs() < 1e-6, "{:?}", r.p());
    }

    #[test]
    fn uv_is_vertex_class_with_u_over_divs_and_v_arclength() {
        let d = run(wall(), defaults().with("divs", ParamValue::I(4))).expect("cooks");
        let uv = match d.attr(AttrClass::Vertex, "uv").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            o => panic!("{o:?}"),
        };
        assert_eq!(uv.len(), d.vertex_point.len());
        assert!(uv.iter().any(|c| c[0] == 1.0) && uv.iter().any(|c| c[1] == 1.0) && uv.iter().all(|c| c[0] * 4.0 == (c[0] * 4.0).round()));
    }

    #[test]
    fn hostile_params_and_geometry() {
        for (k, v) in [
            ("dir", ParamValue::V3([0.0; 3])),
            ("dir", ParamValue::V3([f32::NAN, 1.0, 0.0])),
            ("origin", ParamValue::V3([f32::INFINITY, 0.0, 0.0])),
            ("startangle", ParamValue::F(f32::NAN)),
            ("divs", ParamValue::I(2)),
            ("divs", ParamValue::I(0)),
            ("divs", ParamValue::I(1 << 20)),
            ("tris", ParamValue::I(1)),
        ] {
            assert!(matches!(run(wall(), defaults().with(k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        assert!(run(wall(), defaults().with("endangle", ParamValue::F(10.0)).with("divs", ParamValue::I(1))).is_ok());
        assert_eq!(run(Detail::default(), defaults()).expect("cooks").prim_count(), 0);
        let mut dangling = wall();
        dangling.vertex_point[0] = 99;
        assert_eq!(run(dangling, defaults()).expect("cooks").prim_count(), 0);
        let mut huge = wall();
        huge.p_mut()[1] = [f32::MAX, f32::MAX, f32::MAX];
        let r = run(huge, defaults());
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/revolve.json")).expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert!(out.prim_count() > 50 && signed_volume(&out) > 0.0);
        assert_eq!(crate::registry().create("revolve").map(|n| n.type_name()), Some("revolve"));
    }
}
