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
//! * `surftype` menu `auto` (0, default) | `polygon` (1) | `nurbs` (2). auto: Poly/Polyline profiles -> quad mesh as above, `NurbsCurve` /
//!   `BezierCurve` profiles -> ONE exact rational `NurbsSurf` / `BezierSurf` each (u = profile in its own basis, v = angle: rational
//!   quadratic arcs, 4 for a full turn else `ceil(span/90deg)`, weights = point attr `Pw`; `divs` / `tris` / `caps` / `carryattrs` /
//!   `useptaxis` do not apply to NURBS output). polygon: spline profiles are tessellated first (8 samples per span) and revolved as
//!   polys. nurbs: Poly/Polyline profiles ALSO revolve into NURBS surfaces (linear order-2 u basis). Surface-of-revolution
//!   geometry is analytical-oracle only (UNVERIFIED vs Houdini).
//!   NOT modelled (DEFERRED): caps on NURBS output, `rotate` about a local frame per point.
use super::meshutil::{V3, add, bad, cross, dot, newell_corners, positions, scale, sub, toggle};
use super::splinesurf::{TESS_LOD, linear_curve, revolve_surface, spline_curves};
use super::surfutil::{Mesh, arc_fractions, curves, normalize};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail};
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
        ParamSpec::new("caps", ParamValue::B(false)).label("Cap Partial Sweeps"),
        ParamSpec::new("carryattrs", ParamValue::B(true)).label("Inherit Attributes"),
        ParamSpec::new("useptaxis", ParamValue::B(false)).label("Per-Point Axis Attributes"),
        ParamSpec::new("surftype", ParamValue::Menu(0)).label("Surface Type").menu(&[
            ("auto", "Auto (Polygon, NURBS from NURBS)"),
            ("polygon", "Polygon"),
            ("nurbs", "NURBS"),
        ]),
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
        let (caps, carry, ptaxis) = (toggle(p, "caps")?, toggle(p, "carryattrs")?, toggle(p, "useptaxis")?);
        let span = a1 - a0;
        let full = span.abs() >= 360.0 - 1e-9;
        if divs < if full { 3 } else { 1 } || divs > MAX_DIVS {
            return Err(bad(format!(
                "revolve: `divs` {divs} out of range ({}..={MAX_DIVS})",
                if full { 3 } else { 1 }
            )));
        }
        let divs = divs as usize;
        let mode = match p.menu("surftype") {
            Some(v @ 0..=2) => v,
            Some(v) => return Err(bad(format!("revolve: `surftype`: menu index {v} out of range 0..=2"))),
            None => 0,
        };
        let Some(src0) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let tessed = if mode == 1 && src0.has_spline_prims() {
            Some(Arc::new(src0.tessellate(TESS_LOD, TESS_LOD)))
        } else {
            None
        };
        let src: &Arc<Detail> = tessed.as_ref().unwrap_or(src0);
        let pos = positions(src);
        let cols = if full { divs } else { divs + 1 };
        let sweep = if full { 360.0f64.copysign(span) } else { span };
        let mut m = Mesh::default();
        for c in if mode == 2 { Vec::new() } else { curves(src) } {
            let q: Vec<V3> = c.pts.iter().map(|&i| pos[i as usize]).collect();
            let vfr = arc_fractions(&q, c.closed);
            // ids[i][j]
            let mut ids: Vec<Vec<u32>> = Vec::with_capacity(q.len());
            m.cur_prim = Some(c.prim);
            for (qi, pt) in q.iter().enumerate() {
                let srcp = c.pts[qi] as usize;
                m.cur_pt = Some(srcp);
                // per-point axis: attrs `axisorigin` / `axisdir` on the profile point (missing / zero = the node's axis)
                let (origin, axis) = if ptaxis {
                    let g = |n: &str| match src.attr(AttrClass::Point, n).map(|a| &a.data) {
                        Some(AttrData::V3(v)) => {
                            v.get(srcp).map(|c| c.map(f64::from)).filter(|c| c.iter().all(|x| x.is_finite()))
                        }
                        _ => None,
                    };
                    (g("axisorigin").unwrap_or(origin), g("axisdir").and_then(normalize).unwrap_or(axis))
                } else {
                    (origin, axis)
                };
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
            let ncols = divs;
            for i in 0..segs {
                let i2 = (i + 1) % q.len();
                let (v0, v1) = (vfr[i] as f32, if i + 1 == q.len() { 1.0 } else { vfr[i + 1] as f32 });
                for j in 0..ncols {
                    let j2 = (j + 1) % cols;
                    let (u0, u1) = (j as f32 / divs as f32, (j + 1) as f32 / divs as f32);
                    m.quad(
                        OP,
                        // oracle (Houdini 20.5): cell wound (i,j) (i,j+1) (i+1,j+1) (i+1,j)
                        [ids[i][j], ids[i][j2], ids[i2][j2], ids[i2][j]],
                        [[u0, v0, 0.0], [u1, v0, 0.0], [u1, v1, 0.0], [u0, v1, 0.0]],
                        tris,
                    )?;
                }
            }
            // caps: a closed profile swept through a partial angle is closed by one n-gon per end, facing away from the body
            if caps && !full && c.closed && q.len() >= 3 {
                let dir = span.signum();
                for (col, sign) in [(0usize, -1.0), (cols - 1, 1.0)] {
                    let mut ring: Vec<u32> = Vec::with_capacity(q.len());
                    let mut uvs: Vec<[f32; 3]> = Vec::with_capacity(q.len());
                    for (i, row) in ids.iter().enumerate() {
                        if ring.last() != Some(&row[col]) {
                            ring.push(row[col]);
                            uvs.push([col as f32 / divs as f32, vfr[i] as f32, 0.0]);
                        }
                    }
                    if ring.len() > 1 && ring.first() == ring.last() {
                        ring.pop();
                        uvs.pop();
                    }
                    if ring.len() < 3 {
                        continue;
                    }
                    let pts: Vec<V3> = ring.iter().map(|&id| m.pts[id as usize].map(f64::from)).collect();
                    let cen = scale(pts.iter().fold([0.0; 3], |a, &b| add(a, b)), 1.0 / pts.len() as f64);
                    let rel = sub(cen, origin);
                    let radial = sub(rel, scale(axis, dot(rel, axis)));
                    let want = scale(cross(axis, radial), sign * dir);
                    if dot(newell_corners(&pts), want) < 0.0 {
                        ring.reverse();
                        uvs.reverse();
                    }
                    m.poly(OP, &ring, &uvs)?;
                }
            }
        }
        let mut out = if carry { m.finish_with(src) } else { m.finish() };
        if mode != 1 {
            let mut profiles: Vec<(usize, super::splinesurf::SplineCurve)> =
                spline_curves(src).into_iter().map(|c| (c.prim, c)).collect();
            if mode == 2 {
                profiles.extend(curves(src).iter().map(|c| (c.prim, linear_curve(c))));
                profiles.sort_by_key(|x| x.0);
            }
            let n_poly = out.point_count as usize;
            for (_, c) in &profiles {
                let s = revolve_surface(src, c, origin, axis, a0, span, full)?;
                out.merge(&s);
            }
            // merge default-fills `Pw` with 0 on the polygon points: those carry weight 1
            if let Some(hexe_geo::Attribute { data: AttrData::F32(w), .. }) = out.attr_mut(AttrClass::Point, "Pw") {
                w.iter_mut().take(n_poly).for_each(|x| *x = 1.0);
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
    /// profile runs top -> bottom: Houdini winds revolve cells (i,j)(i,j+1)(i+1,j+1)(i+1,j), outward (clockwise-front) only that way
    fn wall_down() -> Detail {
        curve(&[[1.0, 1.0, 0.0], [1.0, 0.0, 0.0]], false)
    }
    fn half_circle() -> Detail {
        let pts: Vec<[f32; 3]> = (0..=6)
            .map(|k| {
                let phi = std::f32::consts::PI * (k as f32 / 6.0);
                [phi.sin(), phi.cos(), 0.0]
            })
            .collect();
        curve(&pts, false)
    }

    #[test]
    fn full_revolve_of_a_wall_is_an_outward_cylinder_wall_with_a_wrapped_seam() {
        let d = run(wall_down(), defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (24, 12));
        let pos = positions(&d);
        for pr in &d.prims {
            let ids: Vec<usize> =
                d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&i| i as usize).collect();
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
        let d = run(wall(), defaults().with("endangle", ParamValue::F(90.0)).with("divs", ParamValue::I(4)))
            .expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (10, 4));
        // axis dir=Z through origin (1,0,0): the wall is now ON the axis? (1,0,0)-(1,1,0) is perpendicular -> second point swings
        let z = run(
            wall(),
            defaults().with("dir", ParamValue::V3([0.0, 0.0, 1.0])).with("origin", ParamValue::V3([1.0, 0.0, 0.0])),
        )
        .expect("cooks");
        assert_eq!((z.point_count, z.prim_count()), (13, 12));
        assert!(z.prims.iter().all(|p| p.verts.len() == 3));
        // a wall lying ON the axis emits nothing
        let on = run(curve(&[[0.0; 3], [0.0, 1.0, 0.0]], false), defaults()).expect("cooks");
        assert_eq!((on.point_count, on.prim_count()), (2, 0));
        // negative span turns the other way: +X -> +Z after a quarter
        let r = run(
            wall(),
            defaults()
                .with("startangle", ParamValue::F(0.0))
                .with("endangle", ParamValue::F(-90.0))
                .with("divs", ParamValue::I(1)),
        )
        .expect("cooks");
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
        assert!(
            uv.iter().any(|c| c[0] == 1.0)
                && uv.iter().any(|c| c[1] == 1.0)
                && uv.iter().all(|c| c[0] * 4.0 == (c[0] * 4.0).round())
        );
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

    fn attr(d: &mut Detail, name: &str, class: AttrClass, data: AttrData) {
        d.add_attr(hexe_geo::Attribute { name: name.into(), class, data, type_info: hexe_geo::TypeInfo::None });
    }

    #[test]
    fn caps_close_a_partial_sweep_of_a_closed_profile_into_a_pappus_volume() {
        // unit square profile at x in [1,2], y in [0,1]: area 1, centroid radius 1.5 -> V(90deg) = 1.5 * pi/2 (Pappus)
        let prof = curve(&[[1.0, 1.0, 0.0], [2.0, 1.0, 0.0], [2.0, 0.0, 0.0], [1.0, 0.0, 0.0]], true);
        let p = defaults().with("endangle", ParamValue::F(90.0)).with("divs", ParamValue::I(16));
        let open = run(prof.clone(), p.clone()).expect("cooks");
        let capped = run(prof.clone(), p.clone().with("caps", ParamValue::B(true))).expect("cooks");
        assert_eq!((open.prim_count(), capped.prim_count()), (64, 66));
        assert_eq!(open.point_count, capped.point_count);
        let v = signed_volume(&capped);
        let want = 1.5 * std::f64::consts::FRAC_PI_2;
        assert!(v > 0.0 && (v - want).abs() / want < 0.01, "{v} vs {want}");
        // cap normals: the start cap faces -tangent (+Z at +X, x=1.5), the end cap faces +tangent (-X at the -Z side)
        let pos = positions(&capped);
        let ids = |i: usize| -> Vec<usize> {
            let pr = &capped.prims[i];
            capped.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&k| k as usize).collect()
        };
        let (s, e) = (prim_normal(&pos, &ids(64)).expect("n"), prim_normal(&pos, &ids(65)).expect("n"));
        assert!(s[2] > 0.99 && e[0] < -0.99, "{s:?} {e:?}");
        // full revolve and open profiles ignore caps
        let full =
            run(prof, defaults().with("caps", ParamValue::B(true)).with("divs", ParamValue::I(8))).expect("cooks");
        assert_eq!(full.prim_count(), 32);
        let wall_caps = run(wall(), p.with("caps", ParamValue::B(true))).expect("cooks");
        assert_eq!(wall_caps.prim_count(), 16);
    }

    #[test]
    fn point_and_prim_attrs_are_inherited_by_the_swept_points_and_quads() {
        let mut w = wall();
        attr(&mut w, "id", AttrClass::Point, AttrData::I32(vec![7, 9]));
        attr(&mut w, "cls", AttrClass::Prim, AttrData::F32(vec![2.5]));
        let d = run(w.clone(), defaults().with("divs", ParamValue::I(4))).expect("cooks");
        assert_eq!(
            d.attr(AttrClass::Point, "id").map(|a| a.data.clone()),
            Some(AttrData::I32(vec![7, 7, 7, 7, 9, 9, 9, 9]))
        );
        assert_eq!(d.attr(AttrClass::Prim, "cls").map(|a| a.data.clone()), Some(AttrData::F32(vec![2.5; 4])));
        let n =
            run(w, defaults().with("divs", ParamValue::I(4)).with("carryattrs", ParamValue::B(false))).expect("cooks");
        assert!(n.attr(AttrClass::Point, "id").is_none() && n.attr(AttrClass::Prim, "cls").is_none());
        // a profile point on the axis stays ONE point carrying its source value
        let mut h = half_circle();
        attr(&mut h, "id", AttrClass::Point, AttrData::I32((0..7).collect()));
        let s = run(h, defaults().with("divs", ParamValue::I(6))).expect("cooks");
        let Some(AttrData::I32(id)) = s.attr(AttrClass::Point, "id").map(|a| a.data.clone()) else { panic!("id") };
        assert_eq!(id.len(), s.point_count as usize);
        assert_eq!(id.iter().filter(|&&v| v == 0).count(), 1);
        assert_eq!(id.iter().filter(|&&v| v == 3).count(), 6);
    }

    #[test]
    fn per_point_axis_attrs_override_the_node_axis_per_profile_point() {
        let mut w = wall(); // (1,0,0) -> (1,1,0)
        attr(&mut w, "axisdir", AttrClass::Point, AttrData::V3(vec![[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]));
        attr(&mut w, "axisorigin", AttrClass::Point, AttrData::V3(vec![[0.0; 3], [0.0; 3]]));
        let p = defaults().with("divs", ParamValue::I(4)).with("useptaxis", ParamValue::B(true));
        let d = run(w.clone(), p).expect("cooks");
        let q = d.p();
        // point 0 about +Y: +X -> -Z; point 1 about +X: +Y -> +Z
        assert!((q[1][0]).abs() < 1e-6 && (q[1][2] + 1.0).abs() < 1e-6, "{:?}", q[1]);
        assert!((q[4][0] - 1.0).abs() < 1e-6 && (q[4][1] - 1.0).abs() < 1e-6);
        assert!((q[5][0] - 1.0).abs() < 1e-6 && q[5][1].abs() < 1e-6 && (q[5][2] - 1.0).abs() < 1e-6, "{:?}", q[5]);
        // off: both points use the node axis (+Y): point 1 ring stays at height 1
        let o = run(w, defaults().with("divs", ParamValue::I(4))).expect("cooks");
        assert!(o.p()[4..8].iter().all(|c| (c[1] - 1.0).abs() < 1e-6));
    }

    use super::super::splinesurf::testkit::{area, nurbs, spline_curve};
    use hexe_geo::{Basis, PrimKind};

    fn nurbs_wall() -> Detail {
        spline_curve(&[[1.0, 0.0, 0.0], [1.0, 1.0, 0.0]], nurbs(2))
    }

    /// NURBS profile -> ONE exact rational NurbsSurf: every tessellated point is on the unit cylinder, area = 2 pi r h (inscribed
    /// 64-gon: within 0.2%), evaluation at v = 1/4 is the profile rotated 90deg (+X -> -Z).
    #[test]
    fn nurbs_wall_revolves_into_an_exact_cylinder_surface() {
        let d = run(nurbs_wall(), defaults()).expect("cooks");
        assert_eq!((d.prim_count(), d.point_count), (1, 2 * 9));
        assert!(matches!(&d.prims[0].kind, PrimKind::NurbsSurf { cols: 2, rows: 9, uwrap: false, vwrap: false, .. }));
        let q = d.evaluate_uv(0, 0.5, 0.25).expect("evaluates");
        assert!(q[0].abs() < 1e-5 && (q[1] - 0.5).abs() < 1e-5 && (q[2] + 1.0).abs() < 1e-5, "{q:?}");
        for k in 0..=40 {
            let q = d.evaluate_uv(0, 0.3, k as f32 / 40.0).expect("evaluates");
            assert!((q[0] * q[0] + q[2] * q[2] - 1.0).abs() < 1e-5 && (q[1] - 0.3).abs() < 1e-5, "{q:?}");
        }
        let t = d.tessellate(1, 16);
        assert!(!t.has_spline_prims());
        let want = std::f64::consts::TAU;
        assert!((area(&t) - want).abs() / want < 2e-3, "area {}", area(&t));
        // outward normals (same winding as the polygon path)
        let pos = positions(&t);
        for pr in &t.prims {
            let ids: Vec<usize> =
                t.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&i| i as usize).collect();
            let n = prim_normal(&pos, &ids).expect("normal");
            let c: V3 = ids.iter().fold([0.0; 3], |a, &i| add(a, scale(pos[i], 0.25)));
            assert!(n[0] * c[0] + n[2] * c[2] > 0.0, "{n:?} {c:?}");
        }
    }

    /// Partial sweep: 90deg = ONE arc (3 CVs, weight cos 45deg), 100deg = 2 arcs; end of the sweep hits the rotated profile.
    #[test]
    fn partial_sweeps_use_one_rational_arc_per_quarter() {
        let q = run(nurbs_wall(), defaults().with("endangle", ParamValue::F(90.0))).expect("cooks");
        assert_eq!(q.point_count, 2 * 3);
        let e = q.evaluate_uv(0, 1.0, 1.0).expect("evaluates");
        assert!(e[0].abs() < 1e-5 && (e[1] - 1.0).abs() < 1e-5 && (e[2] + 1.0).abs() < 1e-5, "{e:?}");
        let want = std::f64::consts::FRAC_PI_2;
        assert!((area(&q.tessellate(1, 32)) - want).abs() / want < 2e-3);
        let h = run(nurbs_wall(), defaults().with("endangle", ParamValue::F(100.0))).expect("cooks");
        assert_eq!(h.point_count, 2 * 5);
        let want = 100.0f64.to_radians();
        assert!((area(&h.tessellate(1, 32)) - want).abs() / want < 2e-3);
        // every sample on the circle
        for k in 0..=20 {
            let s = h.evaluate_uv(0, 1.0, k as f32 / 20.0).expect("evaluates");
            assert!((s[0] * s[0] + s[2] * s[2] - 1.0).abs() < 1e-5, "{s:?}");
        }
    }

    #[test]
    fn bezier_profile_gives_a_bezier_surface_and_polygon_mode_tessellates_first() {
        let b = spline_curve(
            &[[1.0, 0.0, 0.0], [2.0, 0.5, 0.0], [2.0, 1.0, 0.0], [1.0, 1.5, 0.0]],
            PrimKind::BezierCurve { basis: Basis::new(4), closed: false },
        );
        let d = run(b.clone(), defaults()).expect("cooks");
        assert!(matches!(&d.prims[0].kind, PrimKind::BezierSurf { cols: 4, rows: 9, .. }));
        // v = 0 edge is the profile itself
        assert_eq!(d.evaluate_uv(0, 0.5, 0.0), b.evaluate(0, 0.5));
        let poly =
            run(b, defaults().with("surftype", ParamValue::Menu(1)).with("divs", ParamValue::I(8))).expect("cooks");
        assert!(!poly.has_spline_prims() && poly.prim_count() > 8);
    }

    #[test]
    fn nurbs_surftype_lifts_polygon_profiles_and_closed_profiles_wrap() {
        let d = run(wall(), defaults().with("surftype", ParamValue::Menu(2))).expect("cooks");
        assert!(matches!(&d.prims[0].kind, PrimKind::NurbsSurf { cols: 2, rows: 9, .. }) && d.prim_count() == 1);
        assert!((area(&d.tessellate(1, 16)) - std::f64::consts::TAU).abs() < 0.02);
        // closed NURBS ring profile revolved a half turn: torus patch, u wraps
        let ring = spline_curve(
            &[[2.0, 0.0, 0.0], [3.0, 1.0, 0.0], [2.0, 2.0, 0.0], [1.0, 1.0, 0.0]],
            PrimKind::NurbsCurve { basis: Basis { order: 3, knots: Vec::new(), end: false }, closed: true },
        );
        let t = run(ring, defaults().with("endangle", ParamValue::F(180.0))).expect("cooks");
        assert!(matches!(&t.prims[0].kind, PrimKind::NurbsSurf { cols: 4, rows: 5, uwrap: true, vwrap: false, .. }));
        assert!(t.tessellate(4, 4).p().iter().flatten().all(|x| x.is_finite()));
        // poly + spline profiles in one input: polys stay polys, the spline adds one surface
        let mut mixed = wall();
        mixed.merge(&nurbs_wall());
        let m = run(mixed, defaults().with("divs", ParamValue::I(4))).expect("cooks");
        assert_eq!(m.prims.iter().filter(|p| p.kind.is_spline_surf()).count(), 1);
        assert_eq!(m.prims.iter().filter(|p| matches!(p.kind, PrimKind::Poly { .. })).count(), 4);
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/revolve.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert!(out.prim_count() > 50 && signed_volume(&out) > 0.0);
        assert_eq!(crate::registry().create("revolve").map(|n| n.type_name()), Some("revolve"));
    }

    /// promote-1: revolve equals real Houdini 20.5 on every recorded case (closed circle full sweep, open arc partial sweep, triangles, open wall); point/prim order + winding exact. GAP: vertex uv VALUES differ (names only compared).
    #[test]
    fn revolve_matches_houdini_on_all_recorded_oracle_cases() {
        use super::super::meshutil::testkit::houdini_twin;
        macro_rules! case {
            ($n:literal) => {
                (
                    $n,
                    houdini_twin(
                        include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                        include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
                    ),
                )
            };
        }
        for (n, r) in
            [case!("sop-revolve"), case!("sop-revolve-arc"), case!("sop-revolve-tris"), case!("sop-revolve-wall")]
        {
            assert_eq!(r, Ok(()), "{n}");
        }
    }
}
