//! Ray SOP (token `ray`): project the points of input 0 onto the surface of input 1.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Each point of input 0 casts one ray `P + t d`. `method` menu 0 `normal` (default): `d` = the point `N` attribute (V3) if
//! present with the right length, else the uniform-averaged Newell point normal (as `peak`); 1 `dir`: `d` = `dir` V3
//! (default (0,-1,0)). `d` is normalised; a zero direction = a miss for that point (`dir` itself zero = `BadParam`).
//! Target = every closed poly of input 1 (>= 3 valid corners), fan-triangulated from its first corner (exact for convex
//! polys). Moller-Trumbore in f64, edges inclusive (1e-9). The NEAREST hit wins: forward only (`t >= 0`) by default;
//! `bidirectional` B on = nearest by `|t|` over both directions. A hit moves the point onto the hit position
//! (`P' = P + t d`); a miss leaves it where it is. Optional `hitgroup` S: name of a POINT group that is set on hit points.
//! Everything but `P` (and that group) is untouched; point / prim counts are unchanged. Input 1 without triangles = all
//! misses. Work cap: points x triangles <= 2^26, else `Failed`.
//! NOT modelled (DEFERRED): `lift`, `scale`, normal / uv transfer from the hit, farthest / all-hits intersect modes,
//! `minimum distance` method, hit-prim/uv outputs, point group filter.
use super::meshutil::{V3, add, bad, dot, failed, positions, scale, sub, to_f32};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail, Group};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 2] = [InputSpec { name: "input0" }, InputSpec { name: "target" }];
const METHOD_NORMAL: u32 = 0;
const METHOD_DIR: u32 = 1;
const EPS: f64 = 1e-9;
const MAX_WORK: u64 = 1 << 26;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("method", ParamValue::Menu(METHOD_NORMAL)).label("Method"),
        ParamSpec::new("dir", ParamValue::V3([0.0, -1.0, 0.0])).label("Direction"),
        ParamSpec::new("bidirectional", ParamValue::B(false)).label("Bidirectional"),
        ParamSpec::new("hitgroup", ParamValue::S(String::new())).label("Hit Group"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Ray { specs: spec() })
}
struct Ray {
    specs: Vec<ParamSpec>,
}

type Tri = [V3; 3];

fn triangles(d: &Detail) -> Vec<Tri> {
    let pos = positions(d);
    let mut out = Vec::new();
    for pr in &d.prims {
        let Some(pts) = super::meshutil::closed_poly_pts(d, pr) else { continue };
        let v = |k: usize| pts.get(k).and_then(|&p| pos.get(p).copied());
        for k in 1..pts.len().saturating_sub(1) {
            if let (Some(a), Some(b), Some(c)) = (v(0), v(k), v(k + 1)) {
                out.push([a, b, c]);
            }
        }
    }
    out
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Ray parameter `t` of the hit with `tri`, if any (`d` need not be unit; `t` is in units of `d`).
fn hit(o: V3, d: V3, tri: &Tri) -> Option<f64> {
    let (e1, e2) = (sub(tri[1], tri[0]), sub(tri[2], tri[0]));
    let pv = cross(d, e2);
    let det = dot(e1, pv);
    if det.abs() < 1e-14 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = sub(o, tri[0]);
    let u = dot(tv, pv) * inv;
    if !(-EPS..=1.0 + EPS).contains(&u) {
        return None;
    }
    let qv = cross(tv, e1);
    let v = dot(d, qv) * inv;
    if v < -EPS || u + v > 1.0 + EPS {
        return None;
    }
    let t = dot(e2, qv) * inv;
    t.is_finite().then_some(t)
}

impl Node<Detail> for Ray {
    fn type_name(&self) -> &'static str {
        "ray"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let method = match p.menu("method") {
            Some(m @ (METHOD_NORMAL | METHOD_DIR)) => m,
            Some(m) => return Err(bad(format!("ray: `method` menu {m} out of range 0..=1"))),
            None => return Err(bad("ray: `method`: missing or not a menu")),
        };
        let dir = match p.vec3("dir") {
            Some(v) if v.iter().all(|c| c.is_finite()) => v.map(f64::from),
            _ => return Err(bad("ray: `dir`: missing or not a finite vector")),
        };
        if method == METHOD_DIR && dot(dir, dir) < 1e-24 {
            return Err(bad("ray: `dir` must be non-zero"));
        }
        let both = p.toggle("bidirectional").ok_or_else(|| bad("ray: `bidirectional`: missing or not a toggle"))?;
        let group =
            p.string("hitgroup").ok_or_else(|| bad("ray: `hitgroup`: missing or not a string"))?.trim().to_owned();
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let tris = inputs.get(1).map(|d| triangles(d)).unwrap_or_default();
        if (tris.is_empty() && group.is_empty()) || src.point_count == 0 {
            return Ok(Arc::clone(src));
        }
        if u64::from(src.point_count).saturating_mul(tris.len() as u64) > MAX_WORK {
            return Err(failed("ray", "points x triangles exceeds the work cap"));
        }
        let pos = positions(src);
        let dirs: Vec<V3> = if method == METHOD_DIR {
            vec![dir; pos.len()]
        } else {
            match src.attr(AttrClass::Point, "N").map(|a| &a.data) {
                Some(AttrData::V3(n)) if n.len() == pos.len() => {
                    n.iter().map(|v| [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]).collect()
                }
                _ => super::meshutil::point_normals(src, &pos),
            }
        };
        let mut moved = pos.clone();
        let mut hits = vec![false; pos.len()];
        for (i, (&o, &d)) in pos.iter().zip(&dirs).enumerate() {
            let len = dot(d, d).sqrt();
            if !(len.is_finite() && len > 1e-12) {
                continue;
            }
            let d = scale(d, 1.0 / len);
            let best = tris
                .iter()
                .filter_map(|t| hit(o, d, t))
                .filter(|&t| both || t >= -EPS)
                .min_by(|a, b| a.abs().total_cmp(&b.abs()));
            if let (Some(t), Some(m), Some(h)) = (best, moved.get_mut(i), hits.get_mut(i)) {
                *m = add(o, scale(d, t));
                *h = true;
            }
        }
        let mut out = (**src).clone();
        *out.p_mut() = to_f32("ray", &moved)?;
        if !group.is_empty() {
            out.groups.retain(|g| !(g.class == AttrClass::Point && g.name == group));
            out.groups.push(Group { name: group, class: AttrClass::Point, mask: hits });
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cube, defaults, grid3};
    use super::*;
    use hexe_geo::{Attribute, TypeInfo};
    use hexe_graph::ParamSet;

    fn run(p: &ParamSet, a: Detail, b: Detail) -> Result<Arc<Detail>, CookError> {
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: p, log: &|_| {} };
        new().cook(&ctx, &[Arc::new(a), Arc::new(b)])
    }
    fn dir_mode() -> ParamSet {
        defaults(&*new()).with("method", ParamValue::Menu(METHOD_DIR))
    }
    /// 3x3 points (x, z in {-.4, 0, .4}) at height `y`, no prims.
    fn cloud(y: f32) -> Detail {
        let mut d = Detail::default();
        d.add_points(9);
        *d.p_mut() = (0..9).map(|i| [(i % 3) as f32 * 0.4 - 0.4, y, (i / 3) as f32 * 0.4 - 0.4]).collect();
        d
    }

    /// Grid of points above the unit box, rays straight down: all land on the TOP face (y = 0.5), not the bottom one.
    #[test]
    fn grid_above_box_lands_on_the_top_face() {
        let src = cloud(3.0);
        let out = run(&dir_mode(), src.clone(), cube()).expect("cooks");
        for (a, b) in src.p().iter().zip(out.p()) {
            assert_eq!(*b, [a[0], 0.5, a[2]]);
        }
        assert_eq!(out.point_count, 9);
    }

    #[test]
    fn forward_only_misses_backwards_and_bidirectional_finds_it() {
        let below = cloud(-3.0); // ray (0,-1,0) points away from the box
        let miss = run(&dir_mode(), below.clone(), cube()).expect("cooks");
        assert_eq!(miss.p(), below.p());
        let p = dir_mode().with("bidirectional", ParamValue::B(true)).with("hitgroup", ParamValue::S("hit".into()));
        let out = run(&p, below.clone(), cube()).expect("cooks");
        assert!(out.p().iter().zip(below.p()).all(|(a, b)| *a == [b[0], -0.5, b[2]]), "nearest = bottom face");
        assert_eq!(out.group(AttrClass::Point, "hit").map(|g| g.mask.clone()), Some(vec![true; 9]));
        // lateral miss: group marks exactly the hits
        let mut off = cloud(3.0);
        off.p_mut()[4] = [5.0, 3.0, 0.0];
        let out = run(&p, off, cube()).expect("cooks");
        let mask = out.group(AttrClass::Point, "hit").map(|g| g.mask.clone()).expect("group");
        assert_eq!(mask.iter().filter(|&&m| m).count(), 8);
        assert!(!mask[4]);
        assert_eq!(out.p()[4], [5.0, 3.0, 0.0]);
    }

    #[test]
    fn normal_method_uses_n_attribute_or_computed_normals() {
        // explicit N pointing down
        let mut pts = cloud(3.0);
        pts.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[0.0, -2.0, 0.0]; 9]),
            type_info: TypeInfo::Normal,
        });
        let out = run(&defaults(&*new()), pts, cube()).expect("cooks");
        assert!(out.p().iter().all(|p| p[1] == 0.5));
        // computed normals: flat grid3 at z=-3 has +Z normals -> rays hit a big quad at z=0
        let mut g = grid3();
        for q in g.p_mut().iter_mut() {
            q[2] = -3.0;
        }
        let mut wall = Detail::default();
        wall.add_points(4);
        *wall.p_mut() = vec![[-5.0, -5.0, 0.0], [5.0, -5.0, 0.0], [5.0, 5.0, 0.0], [-5.0, 5.0, 0.0]];
        wall.add_poly(&[0, 1, 2, 3], true);
        let out = run(&defaults(&*new()), g.clone(), wall).expect("cooks");
        assert!(out.p().iter().zip(g.p()).all(|(a, b)| *a == [b[0], b[1], 0.0]));
        // no prims and no N -> no direction -> unchanged
        let c = cloud(3.0);
        assert_eq!(run(&defaults(&*new()), c.clone(), cube()).expect("cooks").p(), c.p());
    }

    #[test]
    fn empty_target_and_hostile_params() {
        let c = cloud(3.0);
        let a = Arc::new(c.clone());
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &dir_mode(), log: &|_| {} };
        let out = new().cook(&ctx, &[Arc::clone(&a)]).expect("cooks");
        assert!(Arc::ptr_eq(&out, &a), "no target = shared");
        for p in [
            dir_mode().with("dir", ParamValue::V3([0.0; 3])),
            dir_mode().with("dir", ParamValue::V3([f32::NAN, 1.0, 0.0])),
            defaults(&*new()).with("method", ParamValue::Menu(2)),
            ParamSet::new(),
        ] {
            assert!(matches!(run(&p, c.clone(), cube()), Err(CookError::BadParam(_))));
        }
        // dangling target prim ids are skipped
        let mut t = cube();
        t.vertex_point.push(99);
        t.prims.push(hexe_geo::Prim { kind: hexe_geo::PrimKind::Poly { closed: true }, verts: 24..25 });
        assert_eq!(run(&dir_mode(), c.clone(), t).expect("cooks").p()[4][1], 0.5);
        assert_eq!(run(&dir_mode(), Detail::default(), cube()).expect("cooks").point_count, 0);
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/ray.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        assert_eq!(d.point_count, 9);
        assert!(d.p().iter().all(|p| p[1] == 0.5), "3x3 grid at y=3 projected onto the box top");
        assert_eq!(crate::registry().create("ray").map(|n| n.type_name()), Some("ray"));
    }
}
