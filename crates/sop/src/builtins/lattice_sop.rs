//! Lattice SOP (token `lattice`): free-form deformation of input 0 by a rest lattice (input 1) and a deformed lattice (input 2)
//! (Houdini 20.5 `lattice`, the `interptype` linear / bezier modes).
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-lattice*`)
//! Both lattices are point sets with `(divsx+1)(divsy+1)(divsz+1)` points (`divs*` = cells per axis, default 3; a different count is
//! `Failed`, like Houdini's "bounding divisions do not match"). The grid cell of a lattice point is read from its REST POSITION inside the
//! rest bounding box (`round(frac * divs)`), NOT from the point order (Houdini accepts x-fastest and z-fastest rest orders alike), and
//! its deformed position is the node value. A point of input 0 outside the rest bounding box is unchanged; inside,
//! `(s,t,u)` = its fraction of the box and `P' = sum w_ijk * deformed_ijk` with `w` = tensor Bernstein weights of degree `divs` per axis
//! (`interptype` 1 bezier = Sederberg-Parry FFD, default) or the trilinear weights of its cell (`interptype` 0 linear). `group` = point
//! pattern of the points to deform (others unchanged). Only `P` changes; topology, attributes, groups travel.
//!
//! Not modelled: `restgroup`, `kernel` / `radius` / `normthreshold` / `bspheres` (point-cloud interpolation modes), Houdini's behaviour for
//! rest lattices that are not a regular grid (hexe takes the nearest grid cell, duplicates are `Failed`).
use super::group_sop::parse_pattern;
use super::meshutil::{bad, failed, int, menu, to_f32};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "lattice";
const INPUTS: [InputSpec; 3] =
    [InputSpec { name: "input0" }, InputSpec { name: "rest" }, InputSpec { name: "deformed" }];
const MAX_DIVS: i32 = 32;
const MAX_WORK: u64 = 1 << 28;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("divsx", ParamValue::I(3)).label("X Divisions").range(1.0, 10.0),
        ParamSpec::new("divsy", ParamValue::I(3)).label("Y Divisions").range(1.0, 10.0),
        ParamSpec::new("divsz", ParamValue::I(3)).label("Z Divisions").range(1.0, 10.0),
        ParamSpec::new("interptype", ParamValue::Menu(1))
            .label("Interpolation")
            .menu(&[("linear", "Linear"), ("bezier", "Bezier")]),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Lattice { specs: spec() })
}
struct Lattice {
    specs: Vec<ParamSpec>,
}
/// Bernstein weights `B_i^n(s)`, `i = 0..=n`.
fn bernstein(n: usize, s: f64) -> Vec<f64> {
    let mut c = 1.0f64;
    (0..=n)
        .map(|i| {
            if i > 0 {
                c = c * (n - i + 1) as f64 / i as f64;
            }
            c * s.powi(i as i32) * (1.0 - s).powi((n - i) as i32)
        })
        .collect()
}
/// Per-axis `(weights, first node index)` of fraction `s` in `[0,1]`: all nodes (bezier) or the 2 nodes of its cell (linear).
fn axis_weights(n: usize, s: f64, bezier: bool) -> (Vec<f64>, usize) {
    if bezier {
        return (bernstein(n, s), 0);
    }
    let x = s * n as f64;
    let c = (x.floor() as usize).min(n - 1);
    let f = x - c as f64;
    (vec![1.0 - f, f], c)
}
impl Node<Detail> for Lattice {
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
        let mut dv = [0usize; 3];
        for (slot, name) in dv.iter_mut().zip(["divsx", "divsy", "divsz"]) {
            let v = int(p, name)?;
            if !(1..=MAX_DIVS).contains(&v) {
                return Err(bad(format!("`{name}`: {v} outside 1..={MAX_DIVS}")));
            }
            *slot = v as usize;
        }
        let bezier = match menu(p, "interptype")? {
            0 => false,
            1 => true,
            m => return Err(bad(format!("`interptype`: menu index {m} out of range (linear | bezier)"))),
        };
        let pat = p.string("group").ok_or_else(|| bad("`group`: missing or not a string"))?;
        let (Some(src), Some(rest), Some(def)) = (inputs.first(), inputs.get(1), inputs.get(2)) else {
            return Err(failed(OP, "needs 3 inputs: geometry, rest lattice, deformed lattice"));
        };
        let sel = parse_pattern(src, AttrClass::Point, pat, OP)?;
        let n = [dv[0] + 1, dv[1] + 1, dv[2] + 1];
        let total = n[0] * n[1] * n[2];
        if rest.point_count as usize != total || def.point_count as usize != total {
            return Err(failed(
                OP,
                &format!(
                    "rest ({}) / deformed ({}) lattice points do not match the {total} divisions imply",
                    rest.point_count, def.point_count
                ),
            ));
        }
        if u64::from(src.point_count).saturating_mul(total as u64) > MAX_WORK {
            return Err(failed(OP, "points x lattice size too large"));
        }
        let (rp, dp) = (rest.p(), def.p());
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for q in rp {
            for j in 0..3 {
                lo[j] = lo[j].min(f64::from(q[j]));
                hi[j] = hi[j].max(f64::from(q[j]));
            }
        }
        if (0..3).any(|j| {
            let e = hi[j] - lo[j];
            e <= 0.0 || e.is_nan() || !e.is_finite()
        }) {
            return Err(failed(OP, "rest lattice has zero or non-finite extent"));
        }
        let mut disp: Vec<Option<[f64; 3]>> = vec![None; total];
        for (q, d) in rp.iter().zip(dp) {
            let mut ix = [0usize; 3];
            for j in 0..3 {
                let f = (f64::from(q[j]) - lo[j]) / (hi[j] - lo[j]);
                ix[j] = ((f * dv[j] as f64).round().max(0.0) as usize).min(dv[j]);
            }
            let slot = (ix[0] * n[1] + ix[1]) * n[2] + ix[2];
            if disp[slot].is_some() {
                return Err(failed(OP, "rest lattice is not a regular grid (two points share a cell)"));
            }
            disp[slot] = Some([0, 1, 2].map(|j| f64::from(d[j])));
        }
        let disp: Vec<[f64; 3]> = disp.into_iter().map(|d| d.unwrap_or([0.0; 3])).collect();
        let mut out = src.p().iter().map(|q| q.map(f64::from)).collect::<Vec<_>>();
        for (k, q) in out.iter_mut().enumerate() {
            if !sel.as_ref().is_none_or(|m| m.get(k).copied().unwrap_or(false)) {
                continue;
            }
            let mut s = [0.0f64; 3];
            let mut inside = true;
            for j in 0..3 {
                s[j] = (q[j] - lo[j]) / (hi[j] - lo[j]);
                inside &= (0.0..=1.0).contains(&s[j]);
            }
            if !inside {
                continue;
            }
            let w: Vec<(Vec<f64>, usize)> = (0..3).map(|j| axis_weights(dv[j], s[j], bezier)).collect();
            let mut acc = [0.0f64; 3];
            for (a, wa) in w[0].0.iter().enumerate() {
                for (b, wb) in w[1].0.iter().enumerate() {
                    for (c, wc) in w[2].0.iter().enumerate() {
                        let wt = wa * wb * wc;
                        let d = disp[((w[0].1 + a) * n[1] + w[1].1 + b) * n[2] + w[2].1 + c];
                        for j in 0..3 {
                            acc[j] += wt * d[j];
                        }
                    }
                }
            }
            q.copy_from_slice(&acc);
        }
        let mut d = (**src).clone();
        *d.p_mut() = to_f32(OP, &out)?;
        Ok(Arc::new(d))
    }
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, defaults, houdini_twin};
    use super::*;
    use hexe_graph::ParamSet;
    fn lattice(div: [usize; 3], bump: Option<(usize, [f32; 3])>) -> (Detail, Detail) {
        let mut rest = Detail::default();
        let mut def = Detail::default();
        let mut pts = Vec::new();
        for i in 0..=div[0] {
            for j in 0..=div[1] {
                for k in 0..=div[2] {
                    pts.push([i as f32 / div[0] as f32, j as f32 / div[1] as f32, k as f32 / div[2] as f32]);
                }
            }
        }
        rest.add_points(pts.len() as u32);
        def.add_points(pts.len() as u32);
        *rest.p_mut() = pts.clone();
        if let Some((i, d)) = bump {
            pts[i] = [pts[i][0] + d[0], pts[i][1] + d[1], pts[i][2] + d[2]];
        }
        *def.p_mut() = pts;
        (rest, def)
    }
    fn run(
        src: Detail,
        rest: Detail,
        def: Detail,
        f: impl FnOnce(ParamSet) -> ParamSet,
    ) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &f(defaults(&*n)), log: &|_| {} };
        n.cook(&ctx, &[Arc::new(src), Arc::new(rest), Arc::new(def)])
    }
    fn pts(p: &[[f32; 3]]) -> Detail {
        let mut d = Detail::default();
        d.add_points(p.len() as u32);
        *d.p_mut() = p.to_vec();
        d
    }
    #[test]
    fn bezier_weights_of_one_moved_node() {
        // 3 cells per axis; node index 1 = (0,0,1) (x slowest, z fastest) moved +1 in y; probe (1/3, 0, 1/3)
        let (r, d) = lattice([3, 3, 3], Some((1, [0.0, 1.0, 0.0])));
        let o = run(pts(&[[1.0 / 3.0, 0.0, 1.0 / 3.0], [0.9, 0.9, 0.9]]), r, d, |p| p).expect("cooks");
        let w = 0.296_296_3 * 0.444_444_4;
        assert!((o.p()[0][1] - w).abs() < 1e-5, "{:?}", o.p()[0]);
        assert!((o.p()[1][1] - 0.9).abs() < 1e-3);
    }
    #[test]
    fn linear_is_trilinear_and_outside_is_untouched() {
        let (r, d) = lattice([1, 1, 1], Some((7, [0.0, 0.0, 1.0])));
        let o = run(pts(&[[0.5, 0.5, 0.5], [2.0, 2.0, 2.0], [1.0, 1.0, 1.0]]), r, d, |p| {
            p.with("divsx", ParamValue::I(1))
                .with("divsy", ParamValue::I(1))
                .with("divsz", ParamValue::I(1))
                .with("interptype", ParamValue::Menu(0))
        })
        .expect("cooks");
        assert!((o.p()[0][2] - 0.625).abs() < 1e-6 && o.p()[1] == [2.0, 2.0, 2.0] && (o.p()[2][2] - 2.0).abs() < 1e-6);
    }
    #[test]
    fn group_limits_the_deformed_points() {
        let (r, d) = lattice([1, 1, 1], Some((7, [0.0, 0.0, 1.0])));
        let o = run(pts(&[[1.0, 1.0, 1.0], [1.0, 1.0, 1.0]]), r, d, |p| {
            p.with("divsx", ParamValue::I(1))
                .with("divsy", ParamValue::I(1))
                .with("divsz", ParamValue::I(1))
                .with("group", ParamValue::S("1".into()))
        })
        .expect("cooks");
        assert_eq!((o.p()[0][2], o.p()[1][2]), (1.0, 2.0));
    }
    #[test]
    fn hostile_inputs() {
        let (r, d) = lattice([3, 3, 3], None);
        assert!(run(pts(&[[0.5; 3]]), r.clone(), d.clone(), |p| p.with("divsx", ParamValue::I(2))).is_err());
        assert!(run(pts(&[[0.5; 3]]), r.clone(), d.clone(), |p| p.with("divsx", ParamValue::I(0))).is_err());
        assert!(run(pts(&[[0.5; 3]]), r.clone(), d.clone(), |p| p.with("interptype", ParamValue::Menu(7))).is_err());
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &defaults(&*n), log: &|_| {} };
        assert!(n.cook(&ctx, &[Arc::new(pts(&[[0.5; 3]]))]).is_err());
        let mut flat = r.clone();
        *flat.p_mut() = vec![[0.0; 3]; 64];
        assert!(run(pts(&[[0.5; 3]]), flat, d, |p| p).is_err());
        assert!(cook(&*new(), Detail::default(), &defaults(&*new())).is_err());
    }
    macro_rules! twin {
        ($n:literal) => {
            houdini_twin(
                include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
            )
        };
    }
    /// Hexe twin == real Houdini 20.5 `.geo`: bezier 3x3x3, linear 3x2x4, bezier 3x2x4 on a point group (jittered lattices).
    #[test]
    fn lattice_matches_houdini_fixtures() {
        for n in [twin!("sop-lattice"), twin!("sop-lattice-linear"), twin!("sop-lattice-group")] {
            assert_eq!(n, Ok(()));
        }
    }
}
