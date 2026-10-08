//! Bend + Twist SOPs (tokens `bend`, `twist`): space deformers along an axis with a capture region.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Frame: `a = normalize(dir)` (default +Y) is the capture axis through `origin`. A point has axis coordinate
//! `h = (P - origin) . a`. The capture region is `0 <= h <= length` (`length` F 1, must be > 0). Below it (`h < 0`) points do
//! not move; above it they follow the end of the region rigidly.
//! * `bend`: `angle` F (degrees, 0 = input `Arc` shared) is the total bend over `length`; curvature `k = angle / length`,
//!   radius `R = 1 / k`. The spine bends toward `side` V3 (default +X; orthogonalised against `a`, parallel = `BadParam`).
//!   With `s` = unit side, `c = (P - origin) . s`, `w` = the remaining component along `a x s`, `phi = k * min(h, length)`:
//!   `P' = origin + R sin(phi) a + R (1 - cos(phi)) s + c (cos(phi) s - sin(phi) a) + w`
//!   `+ max(h - length, 0) (cos(phi) a + sin(phi) s)`  (spine point, rotated cross-section, straight tail past the region).
//!   Line of 5 points y = 0..4, `length` 4, 90 degrees: `P_i = (R (1 - cos(pi i / 8)), R sin(pi i / 8), 0)`, `R = 8 / pi`.
//! * `twist`: `angle` F (degrees) rotates the cross-section about the axis by `angle * clamp(h / length, 0, 1)` (right-handed
//!   about `a`; past the region the full angle). Points on the axis never move.
//! NOT modelled (DEFERRED, both): point `group` filter, `capture` regions with falloff / non-linear profile, `rot` pivot
//! offset, `N` rotation (normals are NOT updated), `ZYX` deformation-order switch, `fixedpts`.
use super::meshutil::{V3, add, bad, dot, failed, positions, scale, sub, to_f32};
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
/// Smallest |angle| (radians) treated as a real deformation.
const MIN_ANGLE: f64 = 1e-9;

fn common_specs(extra: Vec<ParamSpec>) -> Vec<ParamSpec> {
    let mut v = vec![
        ParamSpec::new("origin", ParamValue::V3([0.0; 3])).label("Origin"),
        ParamSpec::new("dir", ParamValue::V3([0.0, 1.0, 0.0])).label("Direction"),
        ParamSpec::new("length", ParamValue::F(1.0)).label("Length").range(0.0, 10.0),
        ParamSpec::new("angle", ParamValue::F(0.0)).label("Angle").range(-360.0, 360.0),
    ];
    v.extend(extra);
    v
}
pub fn bend_spec() -> Vec<ParamSpec> {
    common_specs(vec![ParamSpec::new("side", ParamValue::V3([1.0, 0.0, 0.0])).label("Bend Toward")])
}
pub fn twist_spec() -> Vec<ParamSpec> {
    common_specs(Vec::new())
}
pub fn bend_new() -> Box<SopNode> {
    Box::new(Deform { specs: bend_spec(), bend: true })
}
pub fn twist_new() -> Box<SopNode> {
    Box::new(Deform { specs: twist_spec(), bend: false })
}
struct Deform {
    specs: Vec<ParamSpec>,
    bend: bool,
}

fn v3(op: &str, p: &ParamSet, n: &str) -> Result<V3, CookError> {
    match p.vec3(n) {
        Some(v) if v.iter().all(|c| c.is_finite()) => Ok(v.map(f64::from)),
        _ => Err(bad(format!("{op}: `{n}`: missing or not a finite vector"))),
    }
}
fn unit(op: &str, n: &str, v: V3) -> Result<V3, CookError> {
    let l = dot(v, v).sqrt();
    if l.is_finite() && l > 1e-12 { Ok(scale(v, 1.0 / l)) } else { Err(bad(format!("{op}: `{n}` must be non-zero"))) }
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

impl Node<Detail> for Deform {
    fn type_name(&self) -> &'static str {
        if self.bend { "bend" } else { "twist" }
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let op = self.type_name();
        let p = ctx.params;
        let o = v3(op, p, "origin")?;
        let a = unit(op, "dir", v3(op, p, "dir")?)?;
        let len = match p.float("length") {
            Some(l) if l.is_finite() && l > 0.0 => f64::from(l),
            _ => return Err(bad(format!("{op}: `length` must be a finite float > 0"))),
        };
        let theta = match p.float("angle") {
            Some(v) if v.is_finite() => f64::from(v).to_radians(),
            _ => return Err(bad(format!("{op}: `angle`: missing or not a finite float"))),
        };
        let side = if self.bend {
            let s = v3(op, p, "side")?;
            let s = sub(s, scale(a, dot(s, a)));
            Some(unit(op, "side (parallel to dir)", s)?)
        } else {
            None
        };
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if theta.abs() < MIN_ANGLE || src.point_count == 0 {
            return Ok(Arc::clone(src));
        }
        let moved: Vec<V3> = positions(src)
            .into_iter()
            .map(|q| {
                let r = sub(q, o);
                let h = dot(r, a);
                if h <= 0.0 {
                    return q;
                }
                match side {
                    Some(s) => add(o, bend_point(r, h, a, s, len, theta)),
                    None => {
                        let alpha = theta * (h / len).min(1.0);
                        let axial = scale(a, h);
                        let v = sub(r, axial);
                        let rot = add(
                            add(scale(v, alpha.cos()), scale(cross(a, v), alpha.sin())),
                            scale(a, dot(a, v) * (1.0 - alpha.cos())),
                        );
                        add(o, add(axial, rot))
                    }
                }
            })
            .collect();
        if moved.iter().flatten().any(|c| !c.is_finite()) {
            return Err(failed(op, "non-finite result"));
        }
        let mut out = (**src).clone();
        *out.p_mut() = to_f32(op, &moved)?;
        Ok(Arc::new(out))
    }
}

/// Bent offset from `origin` of the point at offset `r` (axis coordinate `h > 0`).
fn bend_point(r: V3, h: f64, a: V3, s: V3, len: f64, theta: f64) -> V3 {
    let radius = len / theta;
    let phi = theta / len * h.min(len);
    let (sn, cs) = phi.sin_cos();
    let c = dot(r, s);
    let w = sub(sub(r, scale(a, h)), scale(s, c));
    let mut p = add(scale(a, radius * sn), scale(s, radius * (1.0 - cs)));
    p = add(p, add(scale(sub(scale(s, cs), scale(a, sn)), c), w));
    if h > len {
        p = add(p, scale(add(scale(a, cs), scale(s, sn)), h - len));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults};
    use super::*;
    use std::f64::consts::PI;

    fn line(ys: &[f32], x: f32, z: f32) -> Detail {
        let mut d = Detail::default();
        d.add_points(ys.len() as u32);
        *d.p_mut() = ys.iter().map(|&y| [x, y, z]).collect();
        let ids: Vec<u32> = (0..ys.len() as u32).collect();
        d.add_poly(&ids, false);
        d
    }
    fn bend(len: f32, deg: f32) -> ParamSet {
        defaults(&*bend_new()).with("length", ParamValue::F(len)).with("angle", ParamValue::F(deg))
    }
    fn twist(len: f32, deg: f32) -> ParamSet {
        defaults(&*twist_new()).with("length", ParamValue::F(len)).with("angle", ParamValue::F(deg))
    }
    fn close(got: [f32; 3], want: [f64; 3], tag: &str) {
        for j in 0..3 {
            assert!((f64::from(got[j]) - want[j]).abs() < 2e-6, "{tag}: {got:?} vs {want:?}");
        }
    }

    /// Line of 5 points y=0..4 bent 90 degrees over length 4: points ride the circle of radius R = 8/pi.
    #[test]
    fn bend_line_of_five_by_90_degrees_lands_on_the_derived_arc() {
        let out = cook(&*bend_new(), line(&[0.0, 1.0, 2.0, 3.0, 4.0], 0.0, 0.0), &bend(4.0, 90.0)).expect("cooks");
        let r = 8.0 / PI;
        for i in 0..5 {
            let phi = PI / 2.0 * f64::from(i) / 4.0;
            close(out.p()[i as usize], [r * (1.0 - phi.cos()), r * phi.sin(), 0.0], &format!("i={i}"));
        }
        close(out.p()[4], [r, r, 0.0], "end");
        assert_eq!(out.prims, line(&[0.0; 5], 0.0, 0.0).prims);
    }

    #[test]
    fn bend_tail_cross_section_and_untouched_regions() {
        let r = 8.0 / PI;
        // past the region: end point + (h-L) along the end tangent (+X for 90 degrees)
        let out = cook(&*bend_new(), line(&[5.0, 6.0], 0.0, 0.0), &bend(4.0, 90.0)).expect("cooks");
        close(out.p()[0], [r + 1.0, r, 0.0], "h=5");
        close(out.p()[1], [r + 2.0, r, 0.0], "h=6");
        // cross-section offset c=1 toward the bend at h=2 (phi=45 deg), z=7 along a x s untouched
        let out = cook(&*bend_new(), line(&[2.0], 1.0, 7.0), &bend(4.0, 90.0)).expect("cooks");
        let phi = PI / 4.0;
        close(out.p()[0], [r * (1.0 - phi.cos()) + phi.cos(), r * phi.sin() - phi.sin(), 7.0], "offset");
        // below the origin plane nothing moves
        let below = cook(&*bend_new(), line(&[-1.0, 0.0], 3.0, 2.0), &bend(4.0, 90.0)).expect("cooks");
        assert_eq!(below.p(), &[[3.0, -1.0, 2.0], [3.0, 0.0, 2.0]]);
        // negative angle mirrors toward -X
        let neg = cook(&*bend_new(), line(&[4.0], 0.0, 0.0), &bend(4.0, -90.0)).expect("cooks");
        close(neg.p()[0], [-r, r, 0.0], "neg");
    }

    #[test]
    fn bend_dir_side_origin_move_the_frame() {
        // axis +X through (0,5,0), bending toward +Z: point (4,5,0) -> (r, 5, r)
        let p = bend(4.0, 90.0)
            .with("dir", ParamValue::V3([1.0, 0.0, 0.0]))
            .with("side", ParamValue::V3([0.3, 0.0, 2.0]))
            .with("origin", ParamValue::V3([0.0, 5.0, 0.0]));
        let mut d = Detail::default();
        d.add_points(1);
        *d.p_mut() = vec![[4.0, 5.0, 0.0]];
        let out = cook(&*bend_new(), d, &p).expect("cooks");
        let r = 8.0 / PI;
        close(out.p()[0], [r, 5.0, r], "frame");
    }

    #[test]
    fn zero_angle_shares_the_arc() {
        let a = Arc::new(cube());
        assert!(Arc::ptr_eq(&cook_arc(&*bend_new(), &a, &bend(1.0, 0.0)).expect("cooks"), &a));
        assert!(Arc::ptr_eq(&cook_arc(&*twist_new(), &a, &twist(1.0, 0.0)).expect("cooks"), &a));
    }

    /// (1,y,0) twisted 90 degrees over length 4 about +Y: angle 22.5 deg * y, right-handed: (cos a, y, -sin a).
    #[test]
    fn twist_rotates_linearly_with_height_right_handed() {
        let out = cook(&*twist_new(), line(&[0.0, 1.0, 2.0, 3.0, 4.0, 6.0, -2.0], 1.0, 0.0), &twist(4.0, 90.0))
            .expect("cooks");
        for (i, y) in [0.0f64, 1.0, 2.0, 3.0, 4.0].iter().enumerate() {
            let a = (PI / 2.0) * y / 4.0;
            close(out.p()[i], [a.cos(), *y, -a.sin()], &format!("y={y}"));
        }
        close(out.p()[5], [0.0, 6.0, -1.0], "past the region: full angle");
        assert_eq!(out.p()[6], [1.0, -2.0, 0.0], "below: untouched");
        // on-axis points never move; axial component z along an offset axis works too
        let axis = cook(&*twist_new(), line(&[1.0, 3.0], 0.0, 0.0), &twist(4.0, 90.0)).expect("cooks");
        assert_eq!(axis.p(), &[[0.0, 1.0, 0.0], [0.0, 3.0, 0.0]]);
    }

    #[test]
    fn hostile_params() {
        for p in [
            bend(0.0, 90.0),
            bend(-1.0, 90.0),
            bend(f32::NAN, 90.0),
            bend(1.0, f32::INFINITY),
            bend(1.0, 90.0).with("dir", ParamValue::V3([0.0; 3])),
            bend(1.0, 90.0).with("side", ParamValue::V3([0.0, 5.0, 0.0])),
            ParamSet::new(),
        ] {
            assert!(matches!(cook(&*bend_new(), cube(), &p), Err(CookError::BadParam(_))));
        }
        assert!(matches!(cook(&*twist_new(), cube(), &twist(0.0, 10.0)), Err(CookError::BadParam(_))));
        assert_eq!(cook(&*bend_new(), Detail::default(), &bend(1.0, 90.0)).expect("cooks").point_count, 0);
        let r = cook(&*bend_new(), cube(), &bend(f32::MIN_POSITIVE, 360.0));
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
        let r = cook(&*twist_new(), line(&[f32::MAX], f32::MAX, f32::MAX), &twist(1.0, 90.0));
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
    }

    #[test]
    fn example_scenes_cook_and_registry_has_slots() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/bend.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        let r = 8.0 / PI;
        assert_eq!(d.point_count, 5);
        close(d.p()[4], [r, r, 0.0], "bend example end point");
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/twist.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        // box (+-.5), twist 90 over y in [-.5,.5]: top corner (.5,.5,.5) -> (.5,.5,-.5)
        let src = cube();
        let i = src.p().iter().position(|p| *p == [0.5, 0.5, 0.5]).expect("corner");
        close(d.p()[i], [0.5, 0.5, -0.5], "top corner");
        let j = src.p().iter().position(|p| *p == [0.5, -0.5, 0.5]).expect("corner");
        close(d.p()[j], [0.5, -0.5, 0.5], "bottom corner stays");
        let reg = crate::registry();
        assert_eq!(reg.create("bend").map(|n| n.type_name()), Some("bend"));
        assert_eq!(reg.create("twist").map(|n| n.type_name()), Some("twist"));
    }
}
