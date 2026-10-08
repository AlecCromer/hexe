//! UVProject SOP (token `uvproject`): writes the VERTEX attribute `uv` (V3, type None, same storage as `N`/`Cd` on
//! `AttrClass::Vertex`; vertex-class attribs already existed, no geo change) from point positions.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; closed-form oracle only)
//! * `type` menu `planar | cylindrical | spherical` (0..=2, default planar). `axis` menu `x | y | z` (0..=2, default z):
//!   planar = projection direction, cylindrical = cylinder axis, spherical = polar axis.
//! * In-plane pair `(a, b)` of an axis (right-handed, `a x b` = axis): x -> (Y, Z), y -> (Z, X), z -> (X, Y); `h` = the axis
//!   coordinate. Positions are taken relative to `center` (default origin) for cylindrical / spherical; planar ignores it
//!   (`offset` does that job).
//! * planar: raw `(u, v) = (a, b)`. cylindrical: `u = turn(atan2(b, a))`, `v = h`. spherical: `u` as cylindrical,
//!   `v = asin(h / r) / pi + 0.5` (`r` = distance from `center`; `r = 0` -> `v = 0.5`). `turn(t) = t / 2pi` wrapped into
//!   `[0, 1)` (so `u = angle / 2pi` exactly for angles in `[0, 2pi)`; a point on the axis has `u = 0`).
//! * `uv = (raw_u * scale.x + offset.x, raw_v * scale.y + offset.y, raw_w * scale.z + offset.z)` with `raw_w = 0`.
//! * `fixseam` (default on; cylindrical / spherical only): per prim, if the raw `u` span exceeds 0.5 turn, corners with
//!   `u < 0.5` get `+1` (before scale), so a face over the seam does not smear across the map. Vertices shared by
//!   several prims keep the LAST prim's value (vertex-class: the seam fix is why the attribute is per vertex).
//! * Non-finite positions / results -> `uv = (0, 0, 0)` for that vertex; dangling point ids likewise.
//!
//! Output = the input with vertex `uv` replaced/added; everything else untouched. Hostile: menu out of range /
//! non-finite `scale`, `offset`, `center` -> `BadParam`.
use super::meshutil::bad;
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
/// `type` menu indices.
const TYPE_PLANAR: u32 = 0;
const TYPE_CYLINDRICAL: u32 = 1;
const TYPE_SPHERICAL: u32 = 2;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(TYPE_PLANAR)).label("Projection").menu(&[
            ("planar", "Planar"),
            ("cylindrical", "Cylindrical"),
            ("spherical", "Spherical"),
        ]),
        ParamSpec::new("axis", ParamValue::Menu(2)).label("Axis").menu(&[
            ("x", "X Axis"),
            ("y", "Y Axis"),
            ("z", "Z Axis"),
        ]),
        ParamSpec::new("center", ParamValue::V3([0.0; 3])).label("Center"),
        ParamSpec::new("scale", ParamValue::V3([1.0; 3])).label("Scale"),
        ParamSpec::new("offset", ParamValue::V3([0.0; 3])).label("Offset"),
        ParamSpec::new("fixseam", ParamValue::B(true)).label("Fix Seam"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(UvProject { specs: spec() })
}
struct UvProject {
    specs: Vec<ParamSpec>,
}

fn menu(p: &ParamSet, name: &str, max: u32) -> Result<u32, CookError> {
    match p.menu(name) {
        Some(v) if v <= max => Ok(v),
        Some(v) => Err(bad(format!("uvproject: `{name}`: menu index {v} out of range 0..={max}"))),
        None => Err(bad(format!("uvproject: `{name}`: missing or not a menu"))),
    }
}
fn vec3(p: &ParamSet, name: &str) -> Result<[f64; 3], CookError> {
    match p.vec3(name) {
        Some(v) if v.iter().all(|c| c.is_finite()) => Ok(v.map(f64::from)),
        _ => Err(bad(format!("uvproject: `{name}`: missing or not a finite vector"))),
    }
}
/// `(a, b, h)` axis indices for `axis` (0 x, 1 y, 2 z): in-plane pair right-handed around the axis.
fn axes(axis: u32) -> (usize, usize, usize) {
    match axis {
        0 => (1, 2, 0),
        1 => (2, 0, 1),
        _ => (0, 1, 2),
    }
}
/// `t` radians -> turns wrapped into `[0, 1)`.
fn turn(t: f64) -> f64 {
    let u = t / std::f64::consts::TAU;
    let w = u - u.floor();
    // u.floor() rounding can leave exactly 1.0 for tiny negative t
    if w >= 1.0 { 0.0 } else { w }
}
/// Raw `(u, v)` before scale / offset; `None` for non-finite input.
fn raw(kind: u32, axis: u32, q: [f32; 3], center: [f64; 3]) -> Option<[f64; 2]> {
    let (ia, ib, ih) = axes(axis);
    let at = |i: usize| Some(f64::from(*q.get(i)?) - center.get(i).copied()?);
    let (a, b, h) = if kind == TYPE_PLANAR {
        (f64::from(*q.get(ia)?), f64::from(*q.get(ib)?), 0.0)
    } else {
        (at(ia)?, at(ib)?, at(ih)?)
    };
    let r = match kind {
        TYPE_PLANAR => [a, b],
        TYPE_CYLINDRICAL => [turn(b.atan2(a)), h],
        _ => {
            let rad = (a * a + b * b + h * h).sqrt();
            let v = if rad > 0.0 { (h / rad).clamp(-1.0, 1.0).asin() / std::f64::consts::PI + 0.5 } else { 0.5 };
            [turn(b.atan2(a)), v]
        }
    };
    (r[0].is_finite() && r[1].is_finite()).then_some(r)
}

impl Node<Detail> for UvProject {
    fn type_name(&self) -> &'static str {
        "uvproject"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let kind = menu(p, "type", TYPE_SPHERICAL)?;
        let axis = menu(p, "axis", 2)?;
        let (center, scale, offset) = (vec3(p, "center")?, vec3(p, "scale")?, vec3(p, "offset")?);
        let fixseam = p.toggle("fixseam").ok_or_else(|| bad("uvproject: `fixseam`: missing or not a toggle"))?;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = src.p();
        // per-vertex raw (u, v); None = no usable position
        let mut rawuv: Vec<Option<[f64; 2]>> = src
            .vertex_point
            .iter()
            .map(|&pt| usize::try_from(pt).ok().and_then(|i| pos.get(i)).and_then(|&q| raw(kind, axis, q, center)))
            .collect();
        if fixseam && kind != TYPE_PLANAR {
            for pr in &src.prims {
                let (Ok(lo), Ok(hi)) = (usize::try_from(pr.verts.start), usize::try_from(pr.verts.end)) else {
                    continue;
                };
                let Some(vs) = rawuv.get_mut(lo..hi) else { continue };
                let us = vs.iter().flatten().map(|r| r[0]);
                let (mn, mx) = us.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), u| (a.min(u), b.max(u)));
                if mx - mn > 0.5 {
                    for r in vs.iter_mut().flatten().filter(|r| r[0] < 0.5) {
                        r[0] += 1.0;
                    }
                }
            }
        }
        let uv: Vec<[f32; 3]> = rawuv
            .iter()
            .map(|r| match r {
                Some([u, v]) => {
                    let o = [u * scale[0] + offset[0], v * scale[1] + offset[1], offset[2]];
                    let f = o.map(|c| c as f32);
                    if f.iter().all(|c| c.is_finite()) { f } else { [0.0; 3] }
                }
                None => [0.0; 3],
            })
            .collect();
        let mut out = (**src).clone();
        out.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(uv),
            type_info: TypeInfo::None,
        });
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use std::f64::consts::{PI, TAU};
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
    fn uvs(d: &Detail) -> Vec<[f32; 3]> {
        match d.attr(AttrClass::Vertex, "uv").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            other => panic!("no V3 vertex uv: {other:?}"),
        }
    }
    /// Generator output of the registry grid SOP (2x2 points, size 2 -> corners at +-1).
    fn grid2() -> Detail {
        let g = super::super::grid_sop::new();
        let p = g
            .params()
            .iter()
            .fold(ParamSet::new(), |p, s| p.with(&s.name, s.default.clone()))
            .with("rows", ParamValue::I(2))
            .with("cols", ParamValue::I(2))
            .with("size", ParamValue::V3([2.0, 2.0, 0.0]));
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        (*g.cook(&ctx, &[]).expect("grid cooks")).clone()
    }
    /// Points only (no prims needed: vertex-class needs a vertex per point, one open 1-corner poly each).
    fn points_as_prims(pts: &[[f32; 3]]) -> Detail {
        let mut d = Detail::default();
        d.add_points(pts.len() as u32);
        *d.p_mut() = pts.to_vec();
        for i in 0..pts.len() as u32 {
            d.add_poly(&[i], false);
        }
        d
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "uvproject");
        let names: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["type", "axis", "center", "scale", "offset", "fixseam"]);
        assert_eq!(n.inputs().len(), 1);
    }
    #[test]
    fn planar_grid_2x2_is_exact_for_all_three_axes() {
        let g = grid2();
        assert_eq!((g.point_count, g.prims.len(), g.vertex_point.len()), (4, 1, 4));
        for (axis, (ia, ib)) in [(0u32, (1, 2)), (1, (2, 0)), (2, (0, 1))] {
            let out = cook(g.clone(), with(defaults(), "axis", ParamValue::Menu(axis))).expect("cooks");
            let uv = uvs(&out);
            assert_eq!(uv.len(), 4);
            for (v, &pt) in out.vertex_point.iter().enumerate() {
                let q = out.p()[pt as usize];
                assert_eq!(uv[v], [q[ia], q[ib], 0.0], "axis {axis} vertex {v}");
            }
        }
        // scale .5 / offset .5: the +-1 corners land exactly on the unit square corners {0,1}^2, all four distinct
        let p = with(with(defaults(), "scale", ParamValue::V3([0.5, 0.5, 1.0])), "offset", ParamValue::V3([0.5; 3]));
        let uv = uvs(&cook(g, with(p, "axis", ParamValue::Menu(1))).expect("cooks"));
        let mut got: Vec<[i32; 2]> = uv
            .iter()
            .map(|c| {
                assert!(c[0] == 0.0 || c[0] == 1.0);
                assert!(c[1] == 0.0 || c[1] == 1.0);
                assert_eq!(c[2], 0.5, "w = raw_w*scale.z + offset.z");
                [c[0] as i32, c[1] as i32]
            })
            .collect();
        got.sort_unstable();
        assert_eq!(got, [[0, 0], [0, 1], [1, 0], [1, 1]]);
    }
    #[test]
    fn cylindrical_circle_points_u_is_angle_over_tau_exact() {
        let n = 16u32;
        for axis in 0..3u32 {
            let (ia, ib, ih) = axes(axis);
            let pts: Vec<[f32; 3]> = (0..n)
                .map(|k| {
                    let t = TAU * f64::from(k) / f64::from(n);
                    let mut q = [0.0f32; 3];
                    q[ia] = (2.0 * t.cos()) as f32;
                    q[ib] = (2.0 * t.sin()) as f32;
                    q[ih] = k as f32 * 0.25;
                    q
                })
                .collect();
            let p = with(with(defaults(), "type", ParamValue::Menu(TYPE_CYLINDRICAL)), "axis", ParamValue::Menu(axis));
            let uv = uvs(&cook(points_as_prims(&pts), p).expect("cooks"));
            for (k, c) in uv.iter().enumerate() {
                let want = k as f64 / f64::from(n);
                assert!((f64::from(c[0]) - want).abs() < 2e-7, "axis {axis} k {k}: {} vs {want}", c[0]);
                assert_eq!(c[1], k as f32 * 0.25, "v = height along the axis");
                assert_eq!(c[2], 0.0);
            }
        }
        // quarter turns are exact in f32 (0, .25, .5, .75)
        let q = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]];
        let p = with(defaults(), "type", ParamValue::Menu(TYPE_CYLINDRICAL));
        let u: Vec<f32> = uvs(&cook(points_as_prims(&q), p).expect("cooks")).iter().map(|c| c[0]).collect();
        assert_eq!(u, [0.0, 0.25, 0.5, 0.75]);
    }
    #[test]
    fn spherical_v_is_asin_over_pi_plus_half() {
        // latitudes via a unit-radius point at height sin(lat), z axis (default)
        let lats = [-90.0f64, -60.0, -30.0, 0.0, 30.0, 45.0, 60.0, 90.0];
        let pts: Vec<[f32; 3]> = lats
            .iter()
            .map(|l| {
                let l = l.to_radians();
                [(3.0 * l.cos()) as f32, 0.0, (3.0 * l.sin()) as f32]
            })
            .collect();
        // axis y: pole along Y; relabel so that pole is the height coordinate
        let pts_y: Vec<[f32; 3]> = pts.iter().map(|q| [q[0], q[2], 0.0]).collect();
        let p = with(with(defaults(), "type", ParamValue::Menu(TYPE_SPHERICAL)), "axis", ParamValue::Menu(1));
        let uv = uvs(&cook(points_as_prims(&pts_y), p).expect("cooks"));
        for (i, l) in lats.iter().enumerate() {
            let h = f64::from(pts_y[i][1]);
            let r = (f64::from(pts_y[i][0]).powi(2) + h * h).sqrt();
            let want = (h / r).asin() / PI + 0.5;
            assert!((f64::from(uv[i][1]) - want).abs() < 1e-6, "lat {l}: {} vs {want}", uv[i][1]);
            // closed form: v = lat/180 + .5
            assert!((f64::from(uv[i][1]) - (l / 180.0 + 0.5)).abs() < 1e-6, "lat {l}");
        }
        assert_eq!(uv[3][1], 0.5, "equator");
        assert_eq!(uv[7][1], 1.0, "north pole");
        assert_eq!(uv[0][1], 0.0, "south pole");
    }
    #[test]
    fn center_shifts_and_origin_point_is_finite_defined() {
        let p = with(defaults(), "type", ParamValue::Menu(TYPE_SPHERICAL));
        // point at the center: r = 0 -> (0, .5)
        let uv =
            uvs(&cook(points_as_prims(&[[1.0, 2.0, 3.0]]), with(p.clone(), "center", ParamValue::V3([1.0, 2.0, 3.0])))
                .expect("cooks"));
        assert_eq!(uv, [[0.0, 0.5, 0.0]]);
        // center (1,0,0): point (1,2,0) is straight up the y axis from it: angle .25 turn, equator
        let uv = uvs(&cook(points_as_prims(&[[1.0, 2.0, 0.0]]), with(p, "center", ParamValue::V3([1.0, 0.0, 0.0])))
            .expect("cooks"));
        assert_eq!(uv, [[0.25, 0.5, 0.0]]);
    }
    #[test]
    fn seam_fix_unwraps_a_face_across_the_seam() {
        // quad spanning angle -10..+10 degrees around z: raw u = .972, .028
        let t = 10.0f64.to_radians();
        let (c, s) = (t.cos() as f32, t.sin() as f32);
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![[c, -s, 0.0], [c, s, 0.0], [c, s, 1.0], [c, -s, 1.0]];
        d.add_poly(&[0, 1, 2, 3], true);
        let p = with(defaults(), "type", ParamValue::Menu(TYPE_CYLINDRICAL));
        let fixed = uvs(&cook(d.clone(), p.clone()).expect("cooks"));
        let want_hi = 1.0 - 10.0 / 360.0;
        let want_lo = 1.0 + 10.0 / 360.0;
        assert!((f64::from(fixed[0][0]) - want_hi).abs() < 1e-6);
        assert!((f64::from(fixed[1][0]) - want_lo).abs() < 1e-6, "{}", fixed[1][0]);
        assert!(fixed.iter().all(|c| (c[0] - fixed[0][0]).abs() < 0.5));
        let raw = uvs(&cook(d, with(p, "fixseam", ParamValue::B(false))).expect("cooks"));
        assert!((f64::from(raw[1][0]) - 10.0 / 360.0).abs() < 1e-6);
    }
    #[test]
    fn uv_is_vertex_class_and_leaves_everything_else_alone() {
        let g = grid2();
        let out = cook(g.clone(), defaults()).expect("cooks");
        let a = out.attr(AttrClass::Vertex, "uv").expect("vertex uv");
        assert_eq!((a.class, a.data.len(), a.type_info), (AttrClass::Vertex, 4, TypeInfo::None));
        assert!(out.attr(AttrClass::Point, "uv").is_none());
        let mut back = (*out).clone();
        back.attrs.retain(|x| x.name != "uv");
        assert_eq!(back, g);
        // replaces an existing uv of the same class
        let again = cook((*out).clone(), with(defaults(), "scale", ParamValue::V3([2.0; 3]))).expect("cooks");
        assert_eq!(again.attrs.iter().filter(|x| x.name == "uv").count(), 1);
    }
    #[test]
    fn hostile_params_and_geometry() {
        for (k, v) in [
            ("type", ParamValue::Menu(3)),
            ("axis", ParamValue::Menu(3)),
            ("type", ParamValue::I(0)),
            ("scale", ParamValue::V3([f32::NAN, 1.0, 1.0])),
            ("offset", ParamValue::V3([0.0, f32::INFINITY, 0.0])),
            ("center", ParamValue::V3([f32::NAN; 3])),
            ("fixseam", ParamValue::I(1)),
        ] {
            assert!(matches!(cook(grid2(), with(defaults(), k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &defaults(), log: &|_| {} };
        assert_eq!(new().cook(&ctx, &[]).expect("cooks").point_count, 0);
        assert_eq!(cook(Detail::default(), defaults()).expect("cooks").point_count, 0);
        for kind in [TYPE_PLANAR, TYPE_CYLINDRICAL, TYPE_SPHERICAL] {
            let mut h = grid2();
            h.vertex_point[0] = 99; // dangling
            h.p_mut()[1] = [f32::NAN, 0.0, f32::INFINITY];
            h.p_mut()[2] = [f32::MAX, f32::MAX, f32::MAX];
            let uv = uvs(&cook(h, with(defaults(), "type", ParamValue::Menu(kind))).expect("cooks"));
            assert!(uv.iter().flatten().all(|c| c.is_finite()), "kind {kind}");
            assert_eq!(uv[0], [0.0; 3]);
        }
    }
    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/uvproject.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert_eq!(uvs(&out).len(), 24);
        assert_eq!(crate::registry().create("uvproject").map(|n| n.type_name()), Some("uvproject"));
    }
}
