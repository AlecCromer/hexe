//! Scatter SOP: input0 polygons → `npts` points distributed uniformly by AREA (optionally weighted by a float attribute).
//! Clean-room semantics (UNVERIFIED vs Houdini; bit-exact RNG unspecified → statistical parity only):
//! - n-gons fan-triangulated from their first vertex; prims with <3 verts / non-Poly kinds skipped.
//! - triangle chosen by area-weighted CDF (inverse transform over a cumulative f64 table), point by sqrt-uniform barycentric.
//! - `density_attrib` (prim attr wins over point attr, float): triangle weight = area · mean(corner weights); inside the
//!   triangle, rejection against the max corner weight gives density ∝ interpolated weight. Negative weights clamp to 0.
//! - PRNG: SplitMix64 seeded by `seed` (i32 param, sign-extended to u64). Same seed + input → byte-identical output.
//! - output: points only. `P`, `sourceprim` (int, input prim index), `N` (prim attr `N` if present, else geometric normal).
use super::meshutil::{cross, newell_corners};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;
/// Hard cap on `npts` (AGENTS.md §4: capped allocations).
const MAX_POINTS: i32 = 4_000_000;
/// Cap on rejection retries per point (density variation inside one triangle) before accepting the last candidate.
const MAX_REJECT: u32 = 64;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("npts", ParamValue::I(1000)).label("Force Total Count").range(0.0, 10000.0),
        ParamSpec::new("seed", ParamValue::I(0)).label("Global Seed").range(0.0, 10.0),
        ParamSpec::new("density_attrib", ParamValue::S(String::new())).label("Density Attribute"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Scatter { specs: spec() })
}
struct Scatter {
    specs: Vec<ParamSpec>,
}
/// SplitMix64: tiny, full-period, passes BigCrush when used as a stream.
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0,1) with 53 bits.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}
/// One fan triangle: corner positions, corner weights, owning prim, area.
struct Tri {
    p: [[f64; 3]; 3],
    w: [f64; 3],
    prim: usize,
    area: f64,
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn widen(v: [f32; 3]) -> [f64; 3] {
    v.map(f64::from)
}
/// Per-corner weight source.
enum Density<'a> {
    None,
    Prim(&'a [f32]),
    Point(&'a [f32]),
}
fn density<'a>(d: &'a Detail, name: &str) -> Result<Density<'a>, CookError> {
    if name.is_empty() {
        return Ok(Density::None);
    }
    let f32_of = |c: AttrClass| match d.attr(c, name).map(|a| &a.data) {
        Some(AttrData::F32(v)) => Some(v.as_slice()),
        _ => None,
    };
    match (f32_of(AttrClass::Prim), f32_of(AttrClass::Point)) {
        (Some(v), _) => Ok(Density::Prim(v)),
        (None, Some(v)) => Ok(Density::Point(v)),
        _ => {
            Err(CookError::BadParam(format!("scatter: `density_attrib` `{name}` is not a float prim/point attribute")))
        }
    }
}
fn triangles(d: &Detail, dens: &Density) -> Vec<Tri> {
    let pos = d.p();
    let at = |pt: u32| pos.get(pt as usize).copied().map(widen);
    let mut out = Vec::new();
    for (pi, prim) in d.prims.iter().enumerate() {
        if !matches!(prim.kind, PrimKind::Poly { .. }) {
            continue;
        }
        let vs = d.vertex_point.get(prim.verts.start as usize..prim.verts.end as usize).unwrap_or(&[]);
        let Some((&first, rest)) = vs.split_first() else { continue };
        let weight = |pt: u32| -> f64 {
            let raw = match dens {
                Density::None => 1.0,
                Density::Prim(v) => v.get(pi).copied().map_or(0.0, f64::from),
                Density::Point(v) => v.get(pt as usize).copied().map_or(0.0, f64::from),
            };
            if raw.is_finite() { raw.max(0.0) } else { 0.0 }
        };
        for pair in rest.windows(2) {
            let (Some(a), Some(b), Some(c)) = (at(first), at(pair[0]), at(pair[1])) else { continue };
            let area = 0.5 * len(cross(sub(b, a), sub(c, a)));
            if area.is_finite() && area > 0.0 {
                out.push(Tri { p: [a, b, c], w: [weight(first), weight(pair[0]), weight(pair[1])], prim: pi, area });
            }
        }
    }
    out
}
/// Prim normal: prim attr `N` if present, else Houdini normal of the polygon (`meshutil::newell_corners`; zero if degenerate).
fn prim_normal(d: &Detail, prim: usize) -> [f32; 3] {
    if let Some(AttrData::V3(n)) = d.attr(AttrClass::Prim, "N").map(|a| &a.data)
        && let Some(v) = n.get(prim)
    {
        return *v;
    }
    let Some(pr) = d.prims.get(prim) else { return [0.0; 3] };
    let vs = d.vertex_point.get(pr.verts.start as usize..pr.verts.end as usize).unwrap_or(&[]);
    let pos = d.p();
    let corners: Vec<[f64; 3]> = vs.iter().filter_map(|&a| pos.get(a as usize).copied().map(widen)).collect();
    let n = newell_corners(&corners);
    let l = len(n);
    if l > 0.0 && l.is_finite() { [(n[0] / l) as f32, (n[1] / l) as f32, (n[2] / l) as f32] } else { [0.0; 3] }
}
/// Core: scatter `npts` points over `d`'s polygons. Pure function of (d, npts, seed, density).
fn scatter(d: &Detail, npts: usize, seed: u64, density_name: &str) -> Result<Detail, CookError> {
    let dens = density(d, density_name)?;
    let tris = triangles(d, &dens);
    // cumulative weight table: area · mean corner weight
    let mut cdf = Vec::with_capacity(tris.len());
    let mut total = 0.0f64;
    for t in &tris {
        total += t.area * (t.w[0] + t.w[1] + t.w[2]) / 3.0;
        cdf.push(total);
    }
    let mut out = Detail::default();
    out.add_points(0);
    if tris.is_empty() || total.is_nan() || total <= 0.0 || !total.is_finite() || npts == 0 {
        return Ok(out);
    }
    let n_u32 = u32::try_from(npts).map_err(|_| CookError::Failed("scatter: npts overflow".into()))?;
    out.add_points(n_u32);
    let mut src = vec![0i32; npts];
    let mut nrm = vec![[0.0f32; 3]; npts];
    let mut rng = Rng(seed);
    let mut pos = vec![[0.0f32; 3]; npts];
    let mut normals: Vec<Option<[f32; 3]>> = vec![None; d.prims.len()];
    for i in 0..npts {
        let pick = |target: f64| cdf.partition_point(|&c| c <= target).min(tris.len() - 1);
        let ti = pick(rng.unit() * total);
        let Some(t) = tris.get(ti) else { continue };
        let wmax = t.w[0].max(t.w[1]).max(t.w[2]);
        let (mut u, mut v);
        let mut tries = 0;
        loop {
            // sqrt-uniform barycentric: b0 = 1-√r1, b1 = √r1·(1-r2), b2 = √r1·r2
            let (r1, r2) = (rng.unit(), rng.unit());
            let s = r1.sqrt();
            (u, v) = (s * (1.0 - r2), s * r2);
            let w = (1.0 - u - v) * t.w[0] + u * t.w[1] + v * t.w[2];
            tries += 1;
            if wmax <= 0.0 || tries >= MAX_REJECT || rng.unit() * wmax <= w {
                break;
            }
        }
        let b0 = 1.0 - u - v;
        let q: [f64; 3] = std::array::from_fn(|k| b0 * t.p[0][k] + u * t.p[1][k] + v * t.p[2][k]);
        if let Some(slot) = pos.get_mut(i) {
            *slot = [q[0] as f32, q[1] as f32, q[2] as f32];
        }
        if let Some(s) = src.get_mut(i) {
            *s = i32::try_from(t.prim).unwrap_or(i32::MAX);
        }
        let n = match normals.get_mut(t.prim) {
            Some(slot) => *slot.get_or_insert_with(|| prim_normal(d, t.prim)),
            None => [0.0; 3],
        };
        if let Some(s) = nrm.get_mut(i) {
            *s = n;
        }
    }
    *out.p_mut() = pos;
    out.add_attr(Attribute {
        name: "sourceprim".into(),
        class: AttrClass::Point,
        data: AttrData::I32(src),
        type_info: TypeInfo::None,
    });
    out.add_attr(Attribute {
        name: "N".into(),
        class: AttrClass::Point,
        data: AttrData::V3(nrm),
        type_info: TypeInfo::Normal,
    });
    Ok(out)
}
fn bad(name: &str) -> CookError {
    CookError::BadParam(format!("scatter: missing or mistyped `{name}`"))
}
fn read(p: &ParamSet) -> Result<(usize, u64, String), CookError> {
    let npts = p.int("npts").ok_or_else(|| bad("npts"))?;
    if !(0..=MAX_POINTS).contains(&npts) {
        return Err(CookError::BadParam(format!("scatter: `npts` {npts} outside 0..={MAX_POINTS}")));
    }
    let seed = p.int("seed").ok_or_else(|| bad("seed"))?;
    let name = p.string("density_attrib").ok_or_else(|| bad("density_attrib"))?;
    Ok((usize::try_from(npts).unwrap_or(0), i64::from(seed) as u64, name.to_string()))
}
impl Node<Detail> for Scatter {
    fn type_name(&self) -> &'static str {
        "scatter"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let (npts, seed, name) = read(ctx.params)?;
        let empty = Detail::default();
        let d = inputs.first().map_or(&empty, |a| &**a);
        scatter(d, npts, seed, &name).map(Arc::new)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::write_geo_json;
    /// Detail from explicit quads (point list + 4-index quads).
    fn quads(pts: &[[f32; 3]], qs: &[[u32; 4]]) -> Detail {
        let mut d = Detail::default();
        d.add_points(pts.len() as u32);
        d.p_mut().copy_from_slice(pts);
        for q in qs {
            d.add_poly(q, true);
        }
        d
    }
    fn unit_quad() -> Detail {
        quads(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]], &[[0, 1, 2, 3]])
    }
    /// Quad A = 2x2 (area 4) at x∈[0,2], quad B = 1x1 (area 1) at x∈[3,4].
    fn a4_b1() -> Detail {
        quads(
            &[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 2.0, 0.0],
                [0.0, 2.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 1.0, 0.0],
                [3.0, 1.0, 0.0],
            ],
            &[[0, 1, 2, 3], [4, 5, 6, 7]],
        )
    }
    fn counts(d: &Detail) -> [usize; 2] {
        let Some(AttrData::I32(s)) = d.attr(AttrClass::Point, "sourceprim").map(|a| &a.data) else { return [0, 0] };
        [s.iter().filter(|&&i| i == 0).count(), s.iter().filter(|&&i| i == 1).count()]
    }
    #[test]
    fn unit_quad_10000_inside_and_mean_centered() {
        let d = scatter(&unit_quad(), 10_000, 0, "").expect("cooks");
        assert_eq!(d.point_count, 10_000);
        let p = d.p();
        assert!(p.iter().all(|q| (0.0..=1.0).contains(&q[0]) && (0.0..=1.0).contains(&q[1]) && q[2] == 0.0));
        let n = p.len() as f64;
        let (mx, my) =
            (p.iter().map(|q| f64::from(q[0])).sum::<f64>() / n, p.iter().map(|q| f64::from(q[1])).sum::<f64>() / n);
        assert!((mx - 0.5).abs() < 0.02 && (my - 0.5).abs() < 0.02, "mean ({mx},{my})");
    }
    #[test]
    fn area_ratio_4_to_1_within_5_percent() {
        let d = scatter(&a4_b1(), 50_000, 7, "").expect("cooks");
        let [a, b] = counts(&d);
        assert_eq!(a + b, 50_000);
        let ratio = a as f64 / b as f64;
        assert!((ratio - 4.0).abs() / 4.0 < 0.05, "ratio {ratio} (a={a}, b={b})");
        // every point lies in its source quad
        let Some(AttrData::I32(s)) = d.attr(AttrClass::Point, "sourceprim").map(|a| &a.data) else {
            panic!("sourceprim")
        };
        for (q, &sp) in d.p().iter().zip(s) {
            let ok = if sp == 0 { q[0] <= 2.0 && q[1] <= 2.0 } else { q[0] >= 3.0 && q[1] <= 1.0 };
            assert!(ok, "{q:?} sp={sp}");
        }
    }
    #[test]
    fn same_seed_is_byte_equal_other_seed_differs() {
        let src = a4_b1();
        let (a, b) = (scatter(&src, 2000, 42, "").expect("a"), scatter(&src, 2000, 42, "").expect("b"));
        assert_eq!(write_geo_json(&a), write_geo_json(&b));
        let c = scatter(&src, 2000, 43, "").expect("c");
        assert_ne!(write_geo_json(&a), write_geo_json(&c));
    }
    #[test]
    fn ngon_fan_covers_whole_hexagon_and_n_is_geometric() {
        let hex: Vec<[f32; 3]> = (0..6)
            .map(|k| {
                let a = std::f32::consts::TAU * k as f32 / 6.0;
                [a.cos(), 0.0, a.sin()]
            })
            .collect();
        let mut d = Detail::default();
        d.add_points(6);
        d.p_mut().copy_from_slice(&hex);
        d.add_poly(&[0, 1, 2, 3, 4, 5], true);
        let o = scatter(&d, 6000, 1, "").expect("cooks");
        let Some(AttrData::V3(n)) = o.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N") };
        assert!(n.iter().all(|v| v[0].abs() < 1e-6 && (v[1].abs() - 1.0).abs() < 1e-6 && v[2].abs() < 1e-6));
        // all four +/- quadrants hit (fan from vertex 0 spans the whole polygon)
        let q = |sx: bool, sz: bool| o.p().iter().filter(|p| (p[0] > 0.0) == sx && (p[2] > 0.0) == sz).count();
        assert!(q(true, true) > 500 && q(true, false) > 500 && q(false, true) > 500 && q(false, false) > 500);
    }
    #[test]
    fn prim_n_attribute_is_copied() {
        let mut d = unit_quad();
        d.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Prim,
            data: AttrData::V3(vec![[0.0, 0.6, 0.8]]),
            type_info: TypeInfo::Normal,
        });
        let o = scatter(&d, 10, 0, "").expect("cooks");
        let Some(AttrData::V3(n)) = o.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N") };
        assert!(n.iter().all(|v| *v == [0.0, 0.6, 0.8]));
    }
    #[test]
    fn prim_density_weights_prims() {
        // equal areas, weights 3:1 → counts 3:1
        let mut d = quads(
            &[
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 1.0, 0.0],
                [3.0, 1.0, 0.0],
            ],
            &[[0, 1, 2, 3], [4, 5, 6, 7]],
        );
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Prim,
            data: AttrData::F32(vec![3.0, 1.0]),
            type_info: TypeInfo::None,
        });
        let [a, b] = counts(&scatter(&d, 40_000, 3, "w").expect("cooks"));
        let r = a as f64 / b as f64;
        assert!((r - 3.0).abs() / 3.0 < 0.05, "ratio {r}");
    }
    #[test]
    fn point_density_gradient_skews_mean() {
        // weight = x on the unit quad → density ∝ x → E[x] = 2/3, E[y] = 1/2
        let mut d = unit_quad();
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![0.0, 1.0, 1.0, 0.0]),
            type_info: TypeInfo::None,
        });
        let o = scatter(&d, 40_000, 5, "w").expect("cooks");
        let n = o.p().len() as f64;
        let mx = o.p().iter().map(|q| f64::from(q[0])).sum::<f64>() / n;
        let my = o.p().iter().map(|q| f64::from(q[1])).sum::<f64>() / n;
        assert!((mx - 2.0 / 3.0).abs() < 0.02 && (my - 0.5).abs() < 0.02, "mean ({mx},{my})");
    }
    #[test]
    fn hostile_inputs_never_panic() {
        assert_eq!(scatter(&Detail::default(), 100, 0, "").expect("empty").point_count, 0);
        assert_eq!(scatter(&unit_quad(), 0, 0, "").expect("zero").point_count, 0);
        assert!(matches!(scatter(&unit_quad(), 5, 0, "nope"), Err(CookError::BadParam(_))));
        let mut z = unit_quad();
        z.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Prim,
            data: AttrData::F32(vec![0.0]),
            type_info: TypeInfo::None,
        });
        assert_eq!(scatter(&z, 100, 0, "w").expect("zero weight").point_count, 0);
        let mut nan = unit_quad();
        nan.p_mut()[2] = [f32::NAN; 3];
        assert!(scatter(&nan, 10, 0, "").is_ok());
        let set = ParamSet::new()
            .with("npts", ParamValue::I(-1))
            .with("seed", ParamValue::I(0))
            .with("density_attrib", ParamValue::S(String::new()));
        assert!(read(&set).is_err());
    }
    #[test]
    fn defaults_are_1000_seed0_no_density() {
        let s = spec();
        assert_eq!(s[0].default, ParamValue::I(1000));
        assert_eq!(s[1].default, ParamValue::I(0));
        assert_eq!(s[2].default, ParamValue::S(String::new()));
    }
    #[test]
    fn example_scene_cooks() {
        let json = include_str!("../../../../examples/scatter.json");
        let (mut g, id) = load_scene(json).map_err(|e| e.to_string()).expect("loads");
        let d = g.cook(id, 0.0).map_err(|e| e.to_string()).expect("cooks");
        assert_eq!(d.point_count, 500);
        assert!(d.p().iter().all(|p| p[1] == 0.0 && p[0].abs() <= 5.0 && p[2].abs() <= 5.0));
    }
}
