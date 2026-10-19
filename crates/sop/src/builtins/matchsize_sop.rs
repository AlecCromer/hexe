//! MatchSize SOP (token `matchsize`): scale and move input 0 so its bounding box fits a target box.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Target box: `targettype` menu 0 `explicit` (default; `targetsize` V3 (1,1,1), `targetcenter` V3 (0,0,0)) or 1 `input1`
//! (axis-aligned bbox of input 1's `P`; empty input 1 = `Failed`). Source box = bbox of input 0's `P`.
//! * `doscale` B on: per axis `f = target_size / source_size`; an axis whose source size is <= 1e-12 (flat) keeps `f = 1`.
//!   `uniformscale` B off: when on, the smallest `f` over the non-flat axes is used for every axis (box fits INSIDE target).
//! * `justify{x,y,z}` menu 0 min / 1 center (default) / 2 max: the source anchor `a` = that face/center of the source
//!   box, the target anchor `t` = the same face/center of the target box.
//! * `dotranslate` B on (default): `P' = t + (P - a) * f` (anchor lands on the target anchor). Off: `P' = a + (P - a) * f`
//!   (scales about the anchor, no move). Scale and translate both off = input `Arc` shared.
//!
//! Only `P` changes (N, uv, ... untouched: N is NOT rescaled). Negative / non-finite target size = `BadParam`.
//! NOT modelled (DEFERRED): `scale`/`offset` post-modifiers, rotate-to-match, per-component N update, point group filter.
use super::meshutil::{V3, bad, failed, positions, to_f32, toggle};
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 2] = [InputSpec { name: "input0" }, InputSpec { name: "target" }];
const FLAT: f64 = 1e-12;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("targettype", ParamValue::Menu(0))
            .label("Target Type")
            .menu(&[("explicit", "Explicit Size and Center"), ("input1", "Second Input Bounds")]),
        ParamSpec::new("targetsize", ParamValue::V3([1.0; 3])).label("Target Size"),
        ParamSpec::new("targetcenter", ParamValue::V3([0.0; 3])).label("Target Center"),
        ParamSpec::new("doscale", ParamValue::B(true)).label("Scale to Match Size"),
        ParamSpec::new("uniformscale", ParamValue::B(false)).label("Uniform Scale"),
        ParamSpec::new("dotranslate", ParamValue::B(true)).label("Move to Match Center"),
        ParamSpec::new("justifyx", ParamValue::Menu(1)).label("Justify X").menu(&[
            ("min", "Min"),
            ("center", "Center"),
            ("max", "Max"),
        ]),
        ParamSpec::new("justifyy", ParamValue::Menu(1)).label("Justify Y").menu(&[
            ("min", "Min"),
            ("center", "Center"),
            ("max", "Max"),
        ]),
        ParamSpec::new("justifyz", ParamValue::Menu(1)).label("Justify Z").menu(&[
            ("min", "Min"),
            ("center", "Center"),
            ("max", "Max"),
        ]),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(MatchSize { specs: spec() })
}
struct MatchSize {
    specs: Vec<ParamSpec>,
}

fn v3(p: &ParamSet, n: &str) -> Result<V3, CookError> {
    match p.vec3(n) {
        Some(v) if v.iter().all(|c| c.is_finite()) => Ok(v.map(f64::from)),
        _ => Err(bad(format!("matchsize: `{n}`: missing or not a finite vector"))),
    }
}
fn justify(p: &ParamSet, n: &str) -> Result<f64, CookError> {
    match p.menu(n) {
        Some(0) => Ok(0.0),
        Some(1) => Ok(1.0),
        Some(2) => Ok(2.0),
        Some(m) => Err(bad(format!("matchsize: `{n}` menu {m} out of range 0..=2"))),
        None => Err(bad(format!("matchsize: `{n}`: missing or not a menu"))),
    }
}
fn bounds(pos: &[V3]) -> Option<(V3, V3)> {
    let first = *pos.first()?;
    Some(pos.iter().fold((first, first), |(mut lo, mut hi), q| {
        for j in 0..3 {
            lo[j] = lo[j].min(q[j]);
            hi[j] = hi[j].max(q[j]);
        }
        (lo, hi)
    }))
}

impl Node<Detail> for MatchSize {
    fn type_name(&self) -> &'static str {
        "matchsize"
    }
    /// Houdini: only the first input is required.
    fn min_inputs(&self) -> usize {
        1
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let (doscale, uniform, dotrans) =
            (toggle(p, "doscale")?, toggle(p, "uniformscale")?, toggle(p, "dotranslate")?);
        let jus = [justify(p, "justifyx")?, justify(p, "justifyy")?, justify(p, "justifyz")?];
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let (tmin, tsize) = match p.menu("targettype") {
            Some(0) => {
                let (size, c) = (v3(p, "targetsize")?, v3(p, "targetcenter")?);
                if size.iter().any(|&s| s < 0.0) {
                    return Err(bad("matchsize: `targetsize` must be >= 0"));
                }
                ([c[0] - size[0] / 2.0, c[1] - size[1] / 2.0, c[2] - size[2] / 2.0], size)
            }
            Some(1) => {
                let t = inputs.get(1).map(|d| positions(d)).unwrap_or_default();
                let (lo, hi) = bounds(&t).ok_or_else(|| failed("matchsize", "input 1 (target) has no points"))?;
                (lo, [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]])
            }
            Some(m) => return Err(bad(format!("matchsize: `targettype` menu {m} out of range 0..=1"))),
            None => return Err(bad("matchsize: `targettype`: missing or not a menu")),
        };
        if !doscale && !dotrans {
            return Ok(Arc::clone(src));
        }
        let pos = positions(src);
        let Some((lo, hi)) = bounds(&pos) else { return Ok(Arc::clone(src)) };
        let ssize = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let mut f = [1.0; 3];
        if doscale {
            for j in 0..3 {
                if ssize[j] > FLAT {
                    f[j] = tsize[j] / ssize[j];
                }
            }
            if uniform {
                let m = (0..3).filter(|&j| ssize[j] > FLAT).map(|j| f[j]).fold(f64::INFINITY, f64::min);
                if m.is_finite() {
                    f = [m; 3];
                }
            }
        }
        let anchor = |lo: V3, size: V3| -> V3 { [0, 1, 2].map(|j| lo[j] + jus[j] * size[j] / 2.0) };
        let a = anchor(lo, ssize);
        let t = if dotrans { anchor(tmin, tsize) } else { a };
        let moved: Vec<V3> = pos.iter().map(|q| [0, 1, 2].map(|j| t[j] + (q[j] - a[j]) * f[j])).collect();
        let mut out = (**src).clone();
        *out.p_mut() = to_f32("matchsize", &moved)?;
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cube, defaults, grid3};
    use super::*;

    fn with(size: [f32; 3], c: [f32; 3]) -> ParamSet {
        defaults(&*new()).with("targetsize", ParamValue::V3(size)).with("targetcenter", ParamValue::V3(c))
    }

    #[test]
    fn unit_box_to_2_4_6_is_exactly_the_scaled_box() {
        let out = cook(&*new(), cube(), &with([2.0, 4.0, 6.0], [0.0; 3])).expect("cooks");
        for (a, b) in cube().p().iter().zip(out.p()) {
            assert_eq!(*b, [a[0] * 2.0, a[1] * 4.0, a[2] * 6.0]);
        }
        assert_eq!(out.prims, cube().prims);
    }

    /// 3x3 point grid (0..2 x 0..2, flat z) -> 4x4 about (10,0,0): center (1,1,0) -> (10,0,0), f = 2; z is flat so f_z = 1.
    #[test]
    fn grid_fits_target_box_exactly_and_flat_axis_is_left_alone() {
        let out = cook(&*new(), grid3(), &with([4.0, 4.0, 4.0], [10.0, 0.0, 0.0])).expect("cooks");
        for (a, b) in grid3().p().iter().zip(out.p()) {
            assert_eq!(*b, [10.0 + (a[0] - 1.0) * 2.0, (a[1] - 1.0) * 2.0, 0.0]);
        }
        let (lo, hi) = out.bbox();
        assert_eq!((lo, hi), ([8.0, -2.0, 0.0], [12.0, 2.0, 0.0]));
    }

    #[test]
    fn justify_min_and_max_pin_the_chosen_faces() {
        let p = with([2.0, 2.0, 2.0], [10.0, 0.0, 0.0])
            .with("justifyx", ParamValue::Menu(0))
            .with("justifyy", ParamValue::Menu(2));
        let out = cook(&*new(), cube(), &p).expect("cooks");
        let (lo, hi) = out.bbox();
        assert_eq!(lo[0], 9.0, "x min face pinned to target min x (10-1)");
        assert_eq!(hi[1], 1.0, "y max face pinned to target max y");
        assert_eq!((hi[0] - lo[0], hi[2] - lo[2]), (2.0, 2.0));
    }

    #[test]
    fn uniform_scale_fits_inside_and_scale_only_keeps_the_anchor() {
        let p = with([4.0, 8.0, 2.0], [0.0; 3]).with("uniformscale", ParamValue::B(true));
        let out = cook(&*new(), cube(), &p).expect("cooks");
        assert_eq!(out.bbox(), ([-1.0; 3], [1.0; 3]), "min factor 2 (z)");
        let q = with([4.0, 4.0, 4.0], [7.0, 7.0, 7.0]).with("dotranslate", ParamValue::B(false));
        assert_eq!(
            cook(&*new(), cube(), &q).expect("cooks").bbox(),
            ([-2.0; 3], [2.0; 3]),
            "scaled about center, not moved"
        );
        let both_off =
            with([4.0; 3], [7.0; 3]).with("dotranslate", ParamValue::B(false)).with("doscale", ParamValue::B(false));
        let a = Arc::new(cube());
        assert!(Arc::ptr_eq(&super::super::meshutil::testkit::cook_arc(&*new(), &a, &both_off).expect("cooks"), &a));
    }

    #[test]
    fn input1_bbox_is_the_target() {
        let p = defaults(&*new()).with("targettype", ParamValue::Menu(1));
        let mut target = cube();
        for q in target.p_mut().iter_mut() {
            *q = [q[0] * 4.0 + 5.0, q[1] * 2.0, q[2] * 8.0];
        }
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        let out = new().cook(&ctx, &[Arc::new(cube()), Arc::new(target)]).expect("cooks");
        assert_eq!(out.bbox(), ([3.0, -1.0, -4.0], [7.0, 1.0, 4.0]));
        let none = new().cook(&ctx, &[Arc::new(cube())]);
        assert!(matches!(none, Err(CookError::Failed(_))), "unconnected target");
    }

    #[test]
    fn hostile_params_and_inputs() {
        for bad in [
            with([-1.0, 1.0, 1.0], [0.0; 3]),
            with([f32::NAN, 1.0, 1.0], [0.0; 3]),
            defaults(&*new()).with("targettype", ParamValue::Menu(2)),
            defaults(&*new()).with("justifyz", ParamValue::Menu(3)),
            ParamSet::new(),
        ] {
            assert!(matches!(cook(&*new(), cube(), &bad), Err(CookError::BadParam(_))));
        }
        assert_eq!(cook(&*new(), Detail::default(), &with([1.0; 3], [0.0; 3])).expect("cooks").point_count, 0);
        let r = cook(&*new(), cube(), &with([f32::MAX; 3], [0.0; 3]));
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
        let flat = cook(&*new(), cube(), &with([0.0; 3], [1.0, 2.0, 3.0])).expect("cooks");
        assert_eq!(flat.bbox(), ([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]), "zero target collapses to the center");
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/matchsize.json")).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").bbox(), ([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]));
        assert_eq!(crate::registry().create("matchsize").map(|n| n.type_name()), Some("matchsize"));
    }
}
