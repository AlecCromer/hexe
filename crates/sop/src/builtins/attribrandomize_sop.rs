//! Attribute Randomize SOP (token `attribrandomize`): fill / combine a float attribute with random values (Houdini 20.5 `attribrandomize`).
//! STATISTICAL twin only: Houdini's stream is not reproducible.
//!
//! # Semantics (hython 20.5 probes, 4900-point grid, `fixtures/sop-attribrandomize`)
//! Per selected element and component `c < dimensions`: `r` = `uniform` in `[min_c, max_c)` (defaults 0..1: mean 0.5, std 0.2887) or
//! `normal` (`median_c`, `stddev_c`; defaults 0 / 1) or `constant` (`constvalue_c`); `v = scale * r`; result = `operation` (`set` | `add` |
//! `min` | `max` | `mult`) of the existing value and `v`. The attribute is created (zero filled, `Cd` typed Color, `N` Normal) when missing.
//! `P add uniform min -0.5 max 0.5` = a +-0.5 cube jitter (std 0.2876-0.291 per axis in the probe, scale 2 doubles it).
//! Classes: detail | primitive | point | vertex. Seed = float, deterministic splitmix64 stream keyed by seed, element and component.
//!
//! Not modelled: grouptype (guess/vertices/edges...), `bernoulli`, `uniformdiscrete`, `uniformorient`, `uniformball`, `exponential`, `lognormal`,
//! `cauchy`, `ramp`, `discrete` (-> `Unsupported`), `allsame`, seed attribute, int / string attributes (-> `Unsupported`).
use super::group_sop::parse_pattern;
use super::meshutil::{bad, float, int};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "attribrandomize";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const DISTS: [(&str, &str); 3] = [("constant", "Constant"), ("uniform", "Uniform"), ("normal", "Normal")];
const OPS: [(&str, &str); 5] = [("set", "Set"), ("add", "Add"), ("min", "Min"), ("max", "Max"), ("mult", "Multiply")];
const AX: [&str; 4] = ["x", "y", "z", "w"];
pub fn spec() -> Vec<ParamSpec> {
    let mut v = vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("class", ParamValue::Menu(2)).label("Attribute Class").menu(&[
            ("detail", "Detail"),
            ("primitive", "Primitive"),
            ("point", "Point"),
            ("vertex", "Vertex"),
        ]),
        ParamSpec::new("name", ParamValue::S("Cd".into())).label("Attribute Name"),
        ParamSpec::new("operation", ParamValue::Menu(0)).label("Operation").menu(&OPS),
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Scale"),
        ParamSpec::new("distribution", ParamValue::Menu(1)).label("Distribution").menu(&DISTS),
        ParamSpec::new("dimensions", ParamValue::I(3)).label("Dimensions").range(1.0, 4.0),
        ParamSpec::new("seed", ParamValue::F(9080.0)).label("Seed"),
    ];
    for (prefix, def) in [("min", 0.0), ("max", 1.0), ("median", 0.0), ("stddev", 1.0), ("constvalue", 0.0)] {
        for a in AX {
            v.push(ParamSpec::new(&format!("{prefix}{a}"), ParamValue::F(def)).label(prefix));
        }
    }
    v
}
pub fn new() -> Box<SopNode> {
    Box::new(AttribRandomize { specs: spec() })
}
struct AttribRandomize {
    specs: Vec<ParamSpec>,
}
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
fn unit(seed: u64, i: usize, c: u64, k: u64) -> f64 {
    // (0,1]: never 0 so ln() is safe
    ((mix(mix(seed ^ (i as u64).wrapping_mul(0xD1B5_4A32_D192_ED03)) ^ (c << 8 | k)) >> 11) as f64 + 1.0)
        / (1u64 << 53) as f64
}
fn rows(a: &AttrData) -> Option<(usize, Vec<[f64; 4]>)> {
    let w = |v: &[f32]| {
        let mut o = [0.0; 4];
        for (x, y) in o.iter_mut().zip(v) {
            *x = f64::from(*y);
        }
        o
    };
    Some(match a {
        AttrData::F32(v) => (1, v.iter().map(|x| w(&[*x])).collect()),
        AttrData::V2(v) => (2, v.iter().map(|x| w(x)).collect()),
        AttrData::V3(v) => (3, v.iter().map(|x| w(x)).collect()),
        AttrData::V4(v) => (4, v.iter().map(|x| w(x)).collect()),
        _ => return None,
    })
}
fn pack(dim: usize, r: &[[f64; 4]]) -> AttrData {
    let f = |x: &[f64; 4], k: usize| x[k] as f32;
    match dim {
        1 => AttrData::F32(r.iter().map(|x| f(x, 0)).collect()),
        2 => AttrData::V2(r.iter().map(|x| [f(x, 0), f(x, 1)]).collect()),
        3 => AttrData::V3(r.iter().map(|x| [f(x, 0), f(x, 1), f(x, 2)]).collect()),
        _ => AttrData::V4(r.iter().map(|x| [f(x, 0), f(x, 1), f(x, 2), f(x, 3)]).collect()),
    }
}
impl Node<Detail> for AttribRandomize {
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
        let name = p.string("name").ok_or_else(|| bad("`name`: missing or not a string"))?.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(bad("`name`: must be one non-empty token"));
        }
        let class = match p.menu("class") {
            Some(0) => AttrClass::Detail,
            Some(1) => AttrClass::Prim,
            Some(2) => AttrClass::Point,
            Some(3) => AttrClass::Vertex,
            _ => return Err(bad("`class`: missing or out of range")),
        };
        let op = p.menu("operation").filter(|&m| m < 5).ok_or_else(|| bad("`operation`: missing or out of range"))?;
        let dist =
            p.menu("distribution").filter(|&m| m < 3).ok_or_else(|| bad("`distribution`: missing or out of range"))?;
        let dim = int(p, "dimensions")?;
        if !(1..=4).contains(&dim) {
            return Err(bad(format!("`dimensions`: {dim} outside 1..=4")));
        }
        let dim = dim as usize;
        let (scale, seed) = (float(p, "scale")?, float(p, "seed")?);
        let mut par = [[0.0f64; 4]; 5];
        for (k, pre) in ["min", "max", "median", "stddev", "constvalue"].iter().enumerate() {
            for (c, a) in AX.iter().enumerate() {
                par[k][c] = float(p, &format!("{pre}{a}"))?;
            }
        }
        let Some(src) = inputs.first() else { return Ok(Arc::default()) };
        if name == "P" && class != AttrClass::Point {
            return Err(bad("`P` only exists on points"));
        }
        let n = src.class_len(class);
        let sel = if class == AttrClass::Detail { None } else { parse_pattern(src, class, pat, OP)? };
        let mut cur: Vec<[f64; 4]> = match src.attr(class, name) {
            Some(a) => match rows(&a.data) {
                Some((d, r)) if d == dim => r,
                Some((d, _)) => return Err(bad(format!("`dimensions`: existing `{name}` has {d} components"))),
                None => {
                    return Err(CookError::Unsupported("attribrandomize: only float attributes are modelled".into()));
                }
            },
            None if name == "P" => {
                src.p().iter().map(|v| [f64::from(v[0]), f64::from(v[1]), f64::from(v[2]), 0.0]).collect()
            }
            None => vec![[0.0; 4]; n],
        };
        if name == "P" && dim != 3 {
            return Err(bad("`P` needs `dimensions` 3"));
        }
        let key = seed.to_bits();
        for (i, row) in cur.iter_mut().enumerate() {
            if !sel.as_ref().is_none_or(|m| m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            for (c, x) in row.iter_mut().enumerate().take(dim) {
                let r = match dist {
                    0 => par[4][c],
                    1 => par[0][c] + (par[1][c] - par[0][c]) * unit(key, i, c as u64, 0),
                    _ => {
                        let (u1, u2) = (unit(key, i, c as u64, 1), unit(key, i, c as u64, 2));
                        par[2][c] + par[3][c] * (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
                    }
                };
                let v = scale * r;
                *x = match op {
                    0 => v,
                    1 => *x + v,
                    2 => x.min(v),
                    3 => x.max(v),
                    _ => *x * v,
                };
            }
        }
        let mut out = (**src).clone();
        if name == "P" && class == AttrClass::Point {
            *out.p_mut() = cur.iter().map(|r| [r[0] as f32, r[1] as f32, r[2] as f32]).collect();
        } else {
            let data = pack(dim, &cur);
            match out.attr_mut(class, name) {
                Some(a) => a.data = data,
                None => {
                    let type_info = match (name, dim) {
                        ("Cd", 3) => TypeInfo::Color,
                        ("N", 3) => TypeInfo::Normal,
                        _ => TypeInfo::None,
                    };
                    out.add_attr(Attribute { name: name.into(), class, data, type_info });
                }
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
    fn stats(v: &[f32]) -> (f64, f64) {
        let m = v.iter().map(|x| f64::from(*x)).sum::<f64>() / v.len() as f64;
        (m, (v.iter().map(|x| (f64::from(*x) - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt())
    }
    fn col(d: &Detail, name: &str, c: usize) -> Vec<f32> {
        match &d.attr(AttrClass::Point, name).expect("attr").data {
            AttrData::V3(v) => v.iter().map(|x| x[c]).collect(),
            AttrData::F32(v) => v.clone(),
            _ => panic!("type"),
        }
    }
    #[test]
    fn default_creates_uniform_color_in_unit_range() {
        let o = run(grid(60), &[]).expect("cooks");
        let a = o.attr(AttrClass::Point, "Cd").expect("Cd");
        assert_eq!(a.type_info, TypeInfo::Color);
        let (m, s) = stats(&col(&o, "Cd", 1));
        assert!((m - 0.5).abs() < 0.02 && (s - 0.2887).abs() < 0.01, "{m} {s}");
    }
    #[test]
    fn normal_and_scale_and_ops() {
        let (m, s) = stats(&col(
            &run(grid(60), &[("distribution", ParamValue::Menu(2)), ("scale", ParamValue::F(2.0))]).expect("cooks"),
            "Cd",
            0,
        ));
        assert!(m.abs() < 0.1 && (s - 2.0).abs() < 0.1, "{m} {s}");
        let c =
            run(grid(4), &[("distribution", ParamValue::Menu(0)), ("constvaluex", ParamValue::F(3.0))]).expect("cooks");
        assert!(col(&c, "Cd", 0).iter().all(|v| *v == 3.0));
        let m = run(
            c.as_ref().clone(),
            &[
                ("distribution", ParamValue::Menu(0)),
                ("constvaluex", ParamValue::F(2.0)),
                ("operation", ParamValue::Menu(4)),
            ],
        )
        .expect("cooks");
        assert!(col(&m, "Cd", 0).iter().all(|v| *v == 6.0));
    }
    #[test]
    fn add_on_p_is_a_zero_centred_jitter_with_min_max() {
        let g = grid(60);
        let o = run(
            g.clone(),
            &[
                ("name", ParamValue::S("P".into())),
                ("operation", ParamValue::Menu(1)),
                ("minx", ParamValue::F(-0.5)),
                ("maxx", ParamValue::F(0.5)),
            ],
        )
        .expect("cooks");
        let dx: Vec<f32> = o.p().iter().zip(g.p()).map(|(a, b)| a[0] - b[0]).collect();
        let (m, s) = stats(&dx);
        assert!(m.abs() < 0.02 && (s - 0.2887).abs() < 0.01, "{m} {s}");
    }
    #[test]
    fn deterministic_seeded_and_grouped() {
        let a = run(grid(8), &[]).expect("a");
        assert_eq!(a.attr(AttrClass::Point, "Cd"), run(grid(8), &[]).expect("b").attr(AttrClass::Point, "Cd"));
        assert_ne!(
            a.attr(AttrClass::Point, "Cd"),
            run(grid(8), &[("seed", ParamValue::F(3.0))]).expect("c").attr(AttrClass::Point, "Cd")
        );
        let mut g = grid(4);
        g.groups.push(hexe_geo::Group {
            name: "a".into(),
            class: AttrClass::Point,
            mask: (0..16).map(|i| i < 2).collect(),
        });
        let o = run(g, &[("group", ParamValue::S("a".into()))]).expect("cooks");
        assert!(col(&o, "Cd", 0)[2..].iter().all(|v| *v == 0.0));
    }
    #[test]
    fn hostile_params() {
        assert!(run(grid(3), &[("dimensions", ParamValue::I(9))]).is_err());
        assert!(run(grid(3), &[("name", ParamValue::S("a b".into()))]).is_err());
        assert!(run(grid(3), &[("name", ParamValue::S("P".into())), ("class", ParamValue::Menu(1))]).is_err());
        assert!(run(grid(3), &[("scale", ParamValue::F(f32::NAN))]).is_err());
        assert!(run(grid(3), &[("group", ParamValue::S("nope".into()))]).is_err());
        assert_eq!(run(Detail::default(), &[]).expect("empty").point_count, 0);
    }
    /// Statistical twin vs real Houdini 20.5 (`P add uniform -0.5..0.5` on a 70x70 point grid: mean / std of |P - base| within 5 %).
    #[test]
    fn attribrandomize_stat_matches_houdini_fixture() {
        let h = hexe_geo::read_geo(include_bytes!("../../../../fixtures/sop-attribrandomize/houdini.geo"))
            .expect("houdini.geo");
        let b =
            hexe_geo::read_geo(include_bytes!("../../../../fixtures/sop-attribrandomize/base.geo")).expect("base.geo");
        let (mut g, id) =
            crate::load_scene(include_str!("../../../../fixtures/sop-attribrandomize/scene.json")).expect("scene");
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
