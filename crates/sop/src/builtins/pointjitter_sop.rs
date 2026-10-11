//! Point Jitter SOP (token `pointjitter`): random per-point displacement (Houdini 20.5 `pointjitter`). STATISTICAL twin only.
//!
//! # Semantics (hython 20.5 probes, 3600-point grid; `fixtures/sop-pointjitter`)
//! Each selected point moves by an independent displacement, each axis UNIFORM in `[-s/2, s/2)` with `s = scale * axisscale[axis]`
//! (scale 1 -> per-axis std 0.2887 = 1/sqrt(12), max |axis| 0.5; scale 2 doubles both; mean |d| 0.483 at scale 1). Houdini's random stream is
//! not reproducible (seed changes the stream but the generator is unknown), so the twin is scored `stat-disp` (mean / std of |P - base|
//! within 5 %; parity/README.md), NOT per-point. Hexe stream: splitmix64 keyed by `seed` and the point number, so the result is deterministic
//! and independent of point order edits elsewhere.
//!
//! Not modelled: `usepscale`, `useidattrib` (-> `Unsupported`), `updatenmls` (normals untouched).
use super::group_sop::parse_pattern;
use super::meshutil::{bad, float, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "pointjitter";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Scale"),
        ParamSpec::new("axisscale", ParamValue::V3([1.0; 3])).label("Axis Scale"),
        ParamSpec::new("seed", ParamValue::F(1.0)).label("Seed"),
        ParamSpec::new("usepscale", ParamValue::B(false)).label("Use Pscale"),
        ParamSpec::new("useidattrib", ParamValue::B(false)).label("Use ID Attribute"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(PointJitter { specs: spec() })
}
struct PointJitter {
    specs: Vec<ParamSpec>,
}
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
/// Uniform in [0,1) from (seed, point, axis).
fn unit(seed: u64, i: usize, axis: u64) -> f64 {
    (mix(mix(seed ^ (i as u64).wrapping_mul(0xD1B5_4A32_D192_ED03)) ^ axis) >> 11) as f64 / (1u64 << 53) as f64
}
impl Node<Detail> for PointJitter {
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
        let pat = p.string("group").ok_or_else(|| bad("`group`: missing or not a string"))?;
        let scale = float(p, "scale")?;
        let seed = float(p, "seed")?;
        let ax = match p.vec3("axisscale") {
            Some(v) if v.iter().all(|c| c.is_finite()) => v,
            _ => return Err(bad("`axisscale`: missing or not a finite V3")),
        };
        if toggle(p, "usepscale")? || toggle(p, "useidattrib")? {
            return Err(CookError::Unsupported("pointjitter: usepscale / useidattrib are not modelled".into()));
        }
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let sel = parse_pattern(d, AttrClass::Point, pat, OP)?;
        let mut out = (**d).clone();
        let key = seed.to_bits();
        for (i, q) in out.p_mut().iter_mut().enumerate() {
            if !sel.as_ref().is_none_or(|m| m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            for (a, qa) in q.iter_mut().enumerate() {
                let s = scale * f64::from(ax[a]);
                *qa = (f64::from(*qa) + (unit(key, i, a as u64) - 0.5) * s) as f32;
            }
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, defaults};
    use super::*;
    fn grid(n: u32) -> Detail {
        let mut d = Detail::default();
        d.add_points(n * n);
        *d.p_mut() = (0..n * n).map(|i| [(i % n) as f32, 0.0, (i / n) as f32]).collect();
        d
    }
    fn run(d: Detail, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        let n = new();
        cook(&*n, d, &ov.iter().fold(defaults(&*n), |p, (k, v)| p.with(k, v.clone())))
    }
    fn disp(a: &Detail, b: &Detail) -> Vec<[f64; 3]> {
        a.p().iter().zip(b.p()).map(|(x, y)| [0, 1, 2].map(|j| f64::from(x[j]) - f64::from(y[j]))).collect()
    }
    #[test]
    fn displacement_is_uniform_in_a_cube_of_edge_scale() {
        let g = grid(60);
        let o = run(g.clone(), &[]).expect("cooks");
        let d = disp(&o, &g);
        assert!(d.iter().all(|v| v.iter().all(|c| c.abs() <= 0.5 + 1e-6)));
        let var = d.iter().map(|v| v[0] * v[0]).sum::<f64>() / d.len() as f64;
        assert!((var.sqrt() - 0.2887).abs() < 0.01, "std {}", var.sqrt());
        let o2 = run(g.clone(), &[("scale", ParamValue::F(2.0)), ("axisscale", ParamValue::V3([1.0, 0.0, 1.0]))])
            .expect("cooks");
        let d2 = disp(&o2, &g);
        assert!(d2.iter().all(|v| v[1] == 0.0 && v[0].abs() <= 1.0 + 1e-6));
    }
    #[test]
    fn deterministic_and_seeded() {
        let g = grid(10);
        let a = run(g.clone(), &[]).expect("a");
        assert_eq!(a.p(), run(g.clone(), &[]).expect("b").p());
        assert_ne!(a.p(), run(g, &[("seed", ParamValue::F(5.0))]).expect("c").p());
    }
    #[test]
    fn group_limits_the_jitter() {
        let mut g = grid(4);
        g.groups.push(hexe_geo::Group {
            name: "a".into(),
            class: AttrClass::Point,
            mask: (0..16).map(|i| i < 2).collect(),
        });
        let o = run(g.clone(), &[("group", ParamValue::S("a".into()))]).expect("cooks");
        assert_ne!(o.p()[0], g.p()[0]);
        assert_eq!(&o.p()[2..], &g.p()[2..]);
    }
    #[test]
    fn hostile_params() {
        assert!(run(grid(3), &[("usepscale", ParamValue::B(true))]).is_err());
        assert!(run(grid(3), &[("group", ParamValue::S("nope".into()))]).is_err());
        assert_eq!(run(Detail::default(), &[]).expect("empty").point_count, 0);
        assert!(run(grid(3), &[("scale", ParamValue::F(f32::NAN))]).is_err());
    }
    /// Statistical twin vs real Houdini 20.5 (mean / std of |P - base| within 5 %, 70x70 point grid; same metric as `cargo xtask oracle` stat-disp).
    #[test]
    fn pointjitter_stat_matches_houdini_fixture() {
        let h = hexe_geo::read_geo(include_bytes!("../../../../fixtures/sop-pointjitter/houdini.geo"))
            .expect("houdini.geo");
        let b = hexe_geo::read_geo(include_bytes!("../../../../fixtures/sop-pointjitter/base.geo")).expect("base.geo");
        let (mut g, id) =
            crate::load_scene(include_str!("../../../../fixtures/sop-pointjitter/scene.json")).expect("scene");
        let d = g.cook(id, 0.0).expect("cook");
        assert_eq!(d.point_count, h.point_count);
        let ms = |x: &Detail| {
            let r: Vec<f64> = x
                .p()
                .iter()
                .zip(b.p())
                .map(|(a, c)| (0..3).map(|j| (f64::from(a[j]) - f64::from(c[j])).powi(2)).sum::<f64>().sqrt())
                .collect();
            let m = r.iter().sum::<f64>() / r.len() as f64;
            (m, (r.iter().map(|v| (v - m).powi(2)).sum::<f64>() / r.len() as f64).sqrt())
        };
        let ((mh, sh), (md, sd)) = (ms(&h), ms(&d));
        assert!((md / mh - 1.0).abs() < 0.05 && (sd / sh - 1.0).abs() < 0.05, "mean {md}/{mh} std {sd}/{sh}");
    }
}
