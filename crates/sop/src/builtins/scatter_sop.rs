//! Scatter SOP: input0 polygons → `npts` points distributed uniformly by AREA (optionally weighted by a float attribute).
//! Clean-room semantics (UNVERIFIED vs Houdini; bit-exact RNG unspecified → statistical parity only):
//! - n-gons fan-triangulated from their first vertex; prims with <3 verts / non-Poly kinds skipped.
//! - triangle chosen by area-weighted CDF (inverse transform over a cumulative f64 table), point by sqrt-uniform barycentric.
//! - `density_attrib` (prim attr wins over point attr, float): triangle weight = area · mean(corner weights); inside the
//!   triangle, rejection against the max corner weight gives density ∝ interpolated weight. Negative weights clamp to 0.
//! - PRNG: SplitMix64 seeded by `seed` (i32 param, sign-extended to u64). Same seed + input → byte-identical output.
//! - QA params: `group` (prim group restricts the source), `generateby` Count|Density (Density: n = round(sum area*meanweight * `density`)),
//!   `relaxpoints`/`relaxiterations` (repulsion radius 1.2*sqrt(area/n), points stay in their source triangle),
//!   `attribs` (globs; point attrs barycentric-interpolated (ints/strings nearest corner), prim attrs copied), `primnumattrib` (name, empty = skip).
//! - Depth: `emergencylimit` (0 = off) caps the point count; `overridedensity`+`overridedensityattrib`+`densityscale` (attr = points
//!   per unit area, replaces `density_attrib`/`density`); `outputprimuv` adds point attrs `primnum` + `uv` (see `prim_uv`).
//! - output: points only. `P`, `sourceprim` (int, input prim index), `N` (prim attr `N` if present, else geometric normal).
use super::meshutil::{cross, gather, newell_corners, selection};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;
/// Hard cap on `npts` (AGENTS.md §4: capped allocations).
const MAX_POINTS: i32 = 4_000_000;
/// Cap on rejection retries per point (density variation inside one triangle) before accepting the last candidate.
const MAX_REJECT: u32 = 64;
const MAX_RELAX_ITERS: i32 = 50;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("npts", ParamValue::I(1000)).label("Force Total Count").range(0.0, 10000.0),
        ParamSpec::new("seed", ParamValue::I(0)).label("Global Seed").range(0.0, 10.0),
        ParamSpec::new("density_attrib", ParamValue::S(String::new())).label("Density Attribute"),
        ParamSpec::new("relaxpoints", ParamValue::B(false)).label("Relax Points"),
        ParamSpec::new("relaxiterations", ParamValue::I(5)).label("Relax Iterations").range(0.0, 50.0),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("generateby", ParamValue::Menu(0)).label("Generate By").menu(&[
            ("count", "Count"),
            ("density", "Density"),
            ("countperprim", "Count per Primitive"),
        ]),
        ParamSpec::new("primcountattrib", ParamValue::S("count".into())).label("Primitive Count Attribute"),
        ParamSpec::new("outputnormals", ParamValue::B(false)).label("Output Normals"),
        ParamSpec::new("density", ParamValue::F(10.0)).label("Density").range(0.0, 100.0),
        ParamSpec::new("attribs", ParamValue::S(String::new())).label("Attributes to Transfer"),
        ParamSpec::new("primnumattrib", ParamValue::S(String::new())).label("Source Prim Attribute"),
        ParamSpec::new("emergencylimit", ParamValue::I(0)).label("Emergency Limit").range(0.0, 1_000_000.0),
        ParamSpec::new("overridedensity", ParamValue::B(false)).label("Override Density"),
        ParamSpec::new("overridedensityattrib", ParamValue::S("density".into())).label("Override Density Attribute"),
        ParamSpec::new("densityscale", ParamValue::F(1.0)).label("Density Scale").range(0.0, 10.0),
        ParamSpec::new("outputprimuv", ParamValue::B(false)).label("Output Prim Num and UV"),
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
/// One fan triangle: corner positions, corner weights, corner point ids, owning prim, area.
struct Tri {
    p: [[f64; 3]; 3],
    w: [f64; 3],
    ids: [u32; 3],
    prim: usize,
    area: f64,
    /// index of this triangle in its prim's fan, and the prim's corner count (parametric uv)
    fan: usize,
    nv: usize,
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
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
fn triangles(d: &Detail, dens: &Density, sel: &Option<Vec<bool>>) -> Vec<Tri> {
    let pos = d.p();
    let at = |pt: u32| pos.get(pt as usize).copied().map(widen);
    let mut out = Vec::new();
    for (pi, prim) in d.prims.iter().enumerate() {
        if !matches!(prim.kind, PrimKind::Poly { .. }) {
            continue;
        }
        if sel.as_ref().is_some_and(|s| !s.get(pi).copied().unwrap_or(false)) {
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
        for (fan, pair) in rest.windows(2).enumerate() {
            let (Some(a), Some(b), Some(c)) = (at(first), at(pair[0]), at(pair[1])) else { continue };
            let area = 0.5 * len(cross(sub(b, a), sub(c, a)));
            if area.is_finite() && area > 0.0 {
                out.push(Tri {
                    p: [a, b, c],
                    w: [weight(first), weight(pair[0]), weight(pair[1])],
                    ids: [first, pair[0], pair[1]],
                    prim: pi,
                    area,
                    fan,
                    nv: vs.len(),
                });
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
/// How many points to make and how to place them.
#[derive(Clone, Debug)]
struct Opts {
    npts: usize,
    seed: u64,
    density_attrib: String,
    /// prim group restricting the source (empty = all)
    group: String,
    /// 0 = Count (`npts`), 1 = Density (`density` points per unit weighted area)
    generateby: u32,
    /// prim attr holding the per-prim point count (generateby 2: int or float, rounded, < 0 = 0)
    primcount_attrib: String,
    /// write point attr `N` (prim `N` if present, else geometric normal); Houdini scatter writes none unless transferred
    normals: bool,
    density: f64,
    relax: bool,
    relax_iters: usize,
    /// attribute-name globs (space separated) transferred from the source (point attrs interpolated, prim attrs copied)
    attribs: String,
    /// name of the source-prim int attr ("" = skip)
    primnumattrib: String,
    /// hard maximum point count (0 = off)
    emergency: usize,
    /// per-prim / per-point float attr `override_attrib` replaces `density_attrib` AND (Density mode) the global density
    override_on: bool,
    override_attrib: String,
    /// scales the Density-mode count
    density_scale: f64,
    /// add point attrs `primnum` (I32) + `uv` (V3, z = 0): the point's parametric position in its source prim
    output_primuv: bool,
}
impl Opts {
    fn basic(npts: usize, seed: u64, name: &str) -> Opts {
        Opts {
            npts,
            seed,
            density_attrib: name.to_string(),
            group: String::new(),
            generateby: 0,
            primcount_attrib: "count".into(),
            normals: true,
            density: 10.0,
            relax: false,
            relax_iters: 0,
            attribs: String::new(),
            primnumattrib: "sourceprim".into(),
            emergency: 0,
            override_on: false,
            override_attrib: "density".into(),
            density_scale: 1.0,
            output_primuv: false,
        }
    }
}
/// Parametric prim (u, v) of a point with fan-triangle barycentrics `(u, v)` (weights of corners 1, 2): triangle = the
/// barycentrics; quad = bilinear `P = (1-u)(1-v)p0 + u(1-v)p1 + uv p2 + (1-u)v p3` (exact for parallelograms: fan tri 0
/// `(u+v, v)`, fan tri 1 `(u, u+v)`); n-gon (> 4) = fan-triangle barycentrics.
fn prim_uv(t: &Tri, u: f64, v: f64) -> (f64, f64) {
    match (t.nv, t.fan) {
        (4, 0) => (u + v, v),
        (4, _) => (u, u + v),
        _ => (u, v),
    }
}
/// `*`-glob match (only `*` is special).
fn glob(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !s.starts_with(first) || s.len() < first.len() + last.len() || !s.ends_with(last) {
        return false;
    }
    let mut at = first.len();
    let end = s.len() - last.len();
    for mid in &parts[1..parts.len() - 1] {
        match s.get(at..end).and_then(|t| t.find(mid)) {
            Some(i) => at += i + mid.len(),
            None => return false,
        }
    }
    true
}
/// Clamp barycentric (u, v) (weights of corners 1, 2) into the triangle.
fn clamp_bary(u: f64, v: f64) -> (f64, f64) {
    let (u, v) = (u.max(0.0), v.max(0.0));
    let s = u + v;
    if s > 1.0 { (u / s, v / s) } else { (u, v) }
}
/// Barycentric (u, v) of `q` projected onto triangle `t` (corner 0 = origin of the frame).
fn bary_of(t: &Tri, q: [f64; 3]) -> (f64, f64) {
    let (e1, e2, w) = (sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0]), sub(q, t.p[0]));
    let (d11, d12, d22, dw1, dw2) = (dot3(e1, e1), dot3(e1, e2), dot3(e2, e2), dot3(w, e1), dot3(w, e2));
    let det = d11 * d22 - d12 * d12;
    if det.abs() < 1e-30 {
        return (0.0, 0.0);
    }
    clamp_bary((dw1 * d22 - dw2 * d12) / det, (dw2 * d11 - dw1 * d12) / det)
}
fn point_of(t: &Tri, u: f64, v: f64) -> [f64; 3] {
    let b0 = 1.0 - u - v;
    std::array::from_fn(|k| b0 * t.p[0][k] + u * t.p[1][k] + v * t.p[2][k])
}
/// Relaxation: neighbours closer than `rad` repel; every point is projected back into its own source triangle.
fn relax(tris: &[Tri], tidx: &[usize], bary: &mut [(f64, f64)], rad: f64, iters: usize) {
    use std::collections::HashMap;
    if rad <= 0.0 || !rad.is_finite() {
        return;
    }
    for _ in 0..iters {
        let pts: Vec<[f64; 3]> = tidx
            .iter()
            .zip(bary.iter())
            .map(|(&ti, &(u, v))| tris.get(ti).map_or([0.0; 3], |t| point_of(t, u, v)))
            .collect();
        let cell = |p: [f64; 3]| -> (i64, i64, i64) {
            (((p[0] / rad).floor()) as i64, ((p[1] / rad).floor()) as i64, ((p[2] / rad).floor()) as i64)
        };
        let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
        for (i, p) in pts.iter().enumerate() {
            grid.entry(cell(*p)).or_default().push(i);
        }
        for i in 0..pts.len() {
            let c = cell(pts[i]);
            let mut push = [0.0f64; 3];
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(list) = grid.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) else { continue };
                        for &j in list {
                            if j == i {
                                continue;
                            }
                            let d = sub(pts[i], pts[j]);
                            let l = len(d);
                            if l < rad && l > 1e-12 {
                                let f = 0.5 * (rad - l) / l;
                                push = [push[0] + d[0] * f, push[1] + d[1] * f, push[2] + d[2] * f];
                            }
                        }
                    }
                }
            }
            if let (Some(t), Some(b)) = (tris.get(tidx[i]), bary.get_mut(i)) {
                let q = [pts[i][0] + push[0], pts[i][1] + push[1], pts[i][2] + push[2]];
                *b = bary_of(t, q);
            }
        }
    }
}
/// Core: scatter points over `d`'s polygons. Pure function of (d, opts).
fn scatter_with(d: &Detail, o: &Opts) -> Result<Detail, CookError> {
    let wname = if o.override_on { &o.override_attrib } else { &o.density_attrib };
    if o.override_on && wname.is_empty() {
        return Err(CookError::BadParam("scatter: `overridedensity` needs a non-empty attribute name".into()));
    }
    let dens = density(d, wname)?;
    let sel = selection("scatter", d, &o.group)?;
    let tris = triangles(d, &dens, &sel);
    // cumulative weight table: area · mean corner weight
    let mut cdf = Vec::with_capacity(tris.len());
    let mut total = 0.0f64;
    for t in &tris {
        total += t.area * (t.w[0] + t.w[1] + t.w[2]) / 3.0;
        cdf.push(total);
    }
    // Count per Primitive: slot k belongs to prim group `slots[k]`; each prim places its own count of points on its own triangles
    let mut groups: Vec<(Vec<usize>, Vec<f64>)> = Vec::new();
    let mut slots: Vec<usize> = Vec::new();
    if o.generateby == 2 {
        let counts: Vec<f64> = match d.attr(AttrClass::Prim, &o.primcount_attrib).map(|a| &a.data) {
            Some(AttrData::I32(v)) => v.iter().map(|&x| f64::from(x)).collect(),
            Some(AttrData::F32(v)) => v.iter().map(|&x| f64::from(x).round()).collect(),
            _ => {
                return Err(CookError::BadParam(format!(
                    "scatter: `primcountattrib` `{}` is not an int/float prim attribute",
                    o.primcount_attrib
                )));
            }
        };
        let mut by_prim: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
        for (ti, t) in tris.iter().enumerate() {
            by_prim.entry(t.prim).or_default().push(ti);
        }
        for (prim, tl) in by_prim {
            let n = counts.get(prim).copied().unwrap_or(0.0).max(0.0);
            if !n.is_finite() || slots.len() as f64 + n > f64::from(MAX_POINTS) {
                return Err(CookError::Failed(format!("scatter: per-prim count total exceeds {MAX_POINTS}")));
            }
            let mut acc = 0.0f64;
            let local: Vec<f64> = tl
                .iter()
                .filter_map(|&ti| tris.get(ti))
                .map(|t| {
                    acc += t.area * (t.w[0] + t.w[1] + t.w[2]) / 3.0;
                    acc
                })
                .collect();
            if acc > 0.0 && acc.is_finite() {
                slots.extend(std::iter::repeat_n(groups.len(), n as usize));
                groups.push((tl, local));
            }
        }
    }
    let mut npts = if o.generateby == 2 {
        slots.len()
    } else if o.generateby == 1 {
        // override attr = points per unit area itself; otherwise the weight is relative and `density` is the global rate
        let rate = if o.override_on { 1.0 } else { o.density };
        let mut n = (total * rate * o.density_scale).round();
        if o.emergency > 0 && n.is_finite() {
            n = n.min(o.emergency as f64);
        }
        if !n.is_finite() || n < 0.0 || n > f64::from(MAX_POINTS) {
            return Err(CookError::Failed(format!("scatter: density count {n} outside 0..={MAX_POINTS}")));
        }
        n as usize
    } else {
        o.npts
    };
    if o.emergency > 0 {
        npts = npts.min(o.emergency);
    }
    let mut out = Detail::default();
    out.add_points(0);
    if tris.is_empty() || total.is_nan() || total <= 0.0 || !total.is_finite() || npts == 0 {
        return Ok(out);
    }
    let n_u32 = u32::try_from(npts).map_err(|_| CookError::Failed("scatter: npts overflow".into()))?;
    out.add_points(n_u32);
    let mut rng = Rng(o.seed);
    let mut tidx = vec![0usize; npts];
    let mut bary = vec![(0.0f64, 0.0f64); npts];
    for i in 0..npts {
        let pick = |target: f64| cdf.partition_point(|&c| c <= target).min(tris.len() - 1);
        let ti = if o.generateby == 2 {
            let Some((tl, local)) = slots.get(i).and_then(|&g| groups.get(g)) else { continue };
            let sum = local.last().copied().unwrap_or(0.0);
            let target = rng.unit() * sum;
            let k = local.partition_point(|&c| c <= target).min(tl.len().saturating_sub(1));
            tl.get(k).copied().unwrap_or(0)
        } else {
            pick(rng.unit() * total)
        };
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
        tidx[i] = ti;
        bary[i] = (u, v);
    }
    if o.relax && o.relax_iters > 0 {
        let rad = 1.2 * (tris.iter().map(|t| t.area).sum::<f64>() / npts as f64).sqrt();
        relax(&tris, &tidx, &mut bary, rad, o.relax_iters);
    }
    let mut src = vec![0i32; npts];
    let mut nrm = vec![[0.0f32; 3]; npts];
    let mut pos = vec![[0.0f32; 3]; npts];
    let mut normals: Vec<Option<[f32; 3]>> = vec![None; d.prims.len()];
    for i in 0..npts {
        let (Some(t), Some(&(u, v))) = (tris.get(tidx[i]), bary.get(i)) else { continue };
        let q = point_of(t, u, v);
        pos[i] = [q[0] as f32, q[1] as f32, q[2] as f32];
        src[i] = i32::try_from(t.prim).unwrap_or(i32::MAX);
        nrm[i] = match normals.get_mut(t.prim) {
            Some(slot) => *slot.get_or_insert_with(|| prim_normal(d, t.prim)),
            None => [0.0; 3],
        };
    }
    *out.p_mut() = pos;
    if !o.primnumattrib.is_empty() {
        out.add_attr(Attribute {
            name: o.primnumattrib.clone(),
            class: AttrClass::Point,
            data: AttrData::I32(src.clone()),
            type_info: TypeInfo::None,
        });
    }
    if o.normals {
        out.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::V3(nrm),
            type_info: TypeInfo::Normal,
        });
    }
    // ---- attribute transfer ----
    let primuv = o.output_primuv.then(|| {
        let uv: Vec<[f32; 3]> = (0..npts)
            .map(|i| match (tris.get(tidx[i]), bary.get(i)) {
                (Some(t), Some(&(u, v))) => {
                    let (pu, pv) = prim_uv(t, u, v);
                    [pu as f32, pv as f32, 0.0]
                }
                _ => [0.0; 3],
            })
            .collect();
        uv
    });
    let pats: Vec<&str> = o.attribs.split_whitespace().collect();
    if !pats.is_empty() {
        let prim_idx: Vec<Option<usize>> = src.iter().map(|&s| usize::try_from(s).ok()).collect();
        let nearest: Vec<Option<usize>> = (0..npts)
            .map(|i| {
                let t = tris.get(tidx[i])?;
                let (u, v) = bary[i];
                let b = [1.0 - u - v, u, v];
                let k = (0..3).max_by(|&a, &c| b[a].total_cmp(&b[c])).unwrap_or(0);
                Some(t.ids[k] as usize)
            })
            .collect();
        for a in &d.attrs {
            if a.name == "P" || a.name == o.primnumattrib || !pats.iter().any(|p| glob(p, &a.name)) {
                continue;
            }
            let data = match a.class {
                AttrClass::Prim => gather(&a.data, &prim_idx),
                AttrClass::Point => match &a.data {
                    AttrData::F32(_) | AttrData::V2(_) | AttrData::V3(_) | AttrData::V4(_) | AttrData::FN { .. } => {
                        interp(&a.data, &tris, &tidx, &bary)
                    }
                    other => gather(other, &nearest),
                },
                _ => continue,
            };
            out.add_attr(Attribute { name: a.name.clone(), class: AttrClass::Point, data, type_info: a.type_info });
        }
    }
    if let Some(uv) = primuv {
        out.add_attr(Attribute {
            name: "primnum".into(),
            class: AttrClass::Point,
            data: AttrData::I32(src.clone()),
            type_info: TypeInfo::None,
        });
        out.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Point,
            data: AttrData::V3(uv),
            type_info: TypeInfo::None,
        });
    }
    Ok(out)
}
/// Barycentric interpolation of a float point attribute onto the scattered points.
fn interp(data: &AttrData, tris: &[Tri], tidx: &[usize], bary: &[(f64, f64)]) -> AttrData {
    let (flat, w): (Vec<f32>, usize) = match data {
        AttrData::F32(v) => (v.clone(), 1),
        AttrData::V2(v) => (v.iter().flatten().copied().collect(), 2),
        AttrData::V3(v) => (v.iter().flatten().copied().collect(), 3),
        AttrData::V4(v) => (v.iter().flatten().copied().collect(), 4),
        AttrData::FN { size, data } => (data.clone(), (*size as usize).max(1)),
        other => return other.clone(),
    };
    let mut res = Vec::with_capacity(tidx.len() * w);
    for (&ti, &(u, v)) in tidx.iter().zip(bary) {
        let bw = [1.0 - u - v, u, v];
        for c in 0..w {
            let Some(t) = tris.get(ti) else {
                res.push(0.0);
                continue;
            };
            let s: f64 =
                (0..3).map(|k| bw[k] * f64::from(flat.get(t.ids[k] as usize * w + c).copied().unwrap_or(0.0))).sum();
            res.push(s as f32);
        }
    }
    match data {
        AttrData::F32(_) => AttrData::F32(res),
        AttrData::V2(_) => AttrData::V2(res.as_chunks::<2>().0.to_vec()),
        AttrData::V3(_) => AttrData::V3(res.as_chunks::<3>().0.to_vec()),
        AttrData::V4(_) => AttrData::V4(res.as_chunks::<4>().0.to_vec()),
        AttrData::FN { size, .. } => AttrData::FN { size: *size, data: res },
        other => other.clone(),
    }
}
/// Count scatter (test helper).
#[cfg(test)]
fn scatter(d: &Detail, npts: usize, seed: u64, density_name: &str) -> Result<Detail, CookError> {
    scatter_with(d, &Opts::basic(npts, seed, density_name))
}
fn bad(name: &str) -> CookError {
    CookError::BadParam(format!("scatter: missing or mistyped `{name}`"))
}
fn read(p: &ParamSet) -> Result<Opts, CookError> {
    let npts = p.int("npts").ok_or_else(|| bad("npts"))?;
    if !(0..=MAX_POINTS).contains(&npts) {
        return Err(CookError::BadParam(format!("scatter: `npts` {npts} outside 0..={MAX_POINTS}")));
    }
    let seed = p.int("seed").ok_or_else(|| bad("seed"))?;
    let name = p.string("density_attrib").ok_or_else(|| bad("density_attrib"))?;
    let mut o = Opts::basic(usize::try_from(npts).unwrap_or(0), i64::from(seed) as u64, name);
    // QA params are optional: absent = default
    o.group = p.string("group").unwrap_or("").to_string();
    o.generateby = p.menu("generateby").unwrap_or(0);
    if o.generateby > 2 {
        return Err(CookError::BadParam(format!("scatter: `generateby` {} out of range", o.generateby)));
    }
    o.primcount_attrib = p.string("primcountattrib").unwrap_or("count").to_string();
    o.normals = p.toggle("outputnormals").unwrap_or(false);
    o.density = f64::from(p.float("density").unwrap_or(10.0));
    if !o.density.is_finite() || o.density < 0.0 {
        return Err(CookError::BadParam(format!("scatter: `density` {} must be finite >= 0", o.density)));
    }
    o.relax = p.toggle("relaxpoints").unwrap_or(false);
    let it = p.int("relaxiterations").unwrap_or(5);
    if !(0..=MAX_RELAX_ITERS).contains(&it) {
        return Err(CookError::BadParam(format!("scatter: `relaxiterations` {it} outside 0..={MAX_RELAX_ITERS}")));
    }
    o.relax_iters = usize::try_from(it).unwrap_or(0);
    o.attribs = p.string("attribs").unwrap_or("").to_string();
    if let Some(n) = p.string("primnumattrib") {
        o.primnumattrib = n.to_string();
    }
    let lim = p.int("emergencylimit").unwrap_or(0);
    if !(0..=MAX_POINTS).contains(&lim) {
        return Err(CookError::BadParam(format!("scatter: `emergencylimit` {lim} outside 0..={MAX_POINTS}")));
    }
    o.emergency = usize::try_from(lim).unwrap_or(0);
    o.override_on = p.toggle("overridedensity").unwrap_or(false);
    if let Some(n) = p.string("overridedensityattrib") {
        o.override_attrib = n.to_string();
    }
    o.density_scale = f64::from(p.float("densityscale").unwrap_or(1.0));
    if !o.density_scale.is_finite() || o.density_scale < 0.0 {
        return Err(CookError::BadParam(format!("scatter: `densityscale` {} must be finite >= 0", o.density_scale)));
    }
    o.output_primuv = p.toggle("outputprimuv").unwrap_or(false);
    Ok(o)
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
        let o = read(ctx.params)?;
        let empty = Detail::default();
        let d = inputs.first().map_or(&empty, |a| &**a);
        scatter_with(d, &o).map(Arc::new)
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
    /// generateby 2: every prim gets exactly its own `count` points, inside itself; negative / zero counts give none; missing attr = error.
    #[test]
    fn count_per_primitive_places_exactly_the_prim_attribute_on_each_prim() {
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
            &[[0, 1, 2, 3], [4, 5, 6, 7], [0, 1, 2, 3]],
        );
        let mut o = Opts::basic(1, 4, "");
        o.generateby = 2;
        assert!(scatter_with(&d, &o).is_err(), "no `count` attribute");
        d.add_attr(Attribute {
            name: "count".into(),
            class: AttrClass::Prim,
            data: AttrData::I32(vec![7, 3, -2]),
            type_info: TypeInfo::None,
        });
        let out = scatter_with(&d, &o).expect("cooks");
        assert_eq!(out.point_count, 10);
        let Some(AttrData::I32(src)) = out.attr(AttrClass::Point, "sourceprim").map(|a| &a.data) else {
            panic!("sourceprim")
        };
        assert_eq!((src.iter().filter(|&&s| s == 0).count(), src.iter().filter(|&&s| s == 1).count()), (7, 3));
        assert!(src.iter().zip(out.p()).all(|(&s, p)| if s == 0 { p[0] <= 1.0 } else { p[0] >= 3.0 }));
    }
    /// Houdini scatter writes no `N` and no source-prim attribute unless asked: the node defaults produce `P` only.
    #[test]
    fn node_defaults_write_p_only_and_outputnormals_adds_n() {
        let d = a4_b1();
        let run = |extra: Option<(&str, ParamValue)>| {
            let mut p = super::super::meshutil::testkit::defaults(&*new()).with("npts", ParamValue::I(50));
            if let Some((k, v)) = extra {
                p = p.with(k, v);
            }
            let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
            (*new().cook(&ctx, &[Arc::new(d.clone())]).expect("cooks")).clone()
        };
        let plain = run(None);
        assert_eq!(plain.attrs.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["P"]);
        let n = run(Some(("outputnormals", ParamValue::B(true))));
        assert!(n.attr(AttrClass::Point, "N").is_some());
        let s = run(Some(("primnumattrib", ParamValue::S("sourceprim".into()))));
        assert!(s.attr(AttrClass::Point, "sourceprim").is_some());
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
    fn nn_min(p: &[[f32; 3]]) -> f64 {
        let mut m = f64::MAX;
        for i in 0..p.len() {
            for j in i + 1..p.len() {
                let d: f64 = (0..3).map(|k| f64::from(p[i][k] - p[j][k]).powi(2)).sum::<f64>().sqrt();
                m = m.min(d);
            }
        }
        m
    }
    fn opts(n: usize) -> Opts {
        Opts::basic(n, 11, "")
    }
    /// Group = {prim 1}: all 500 points land on quad B (x in [3,4]).
    #[test]
    fn group_restricts_source_prims() {
        let mut d = a4_b1();
        d.groups.push(hexe_geo::Group { name: "b".into(), class: AttrClass::Prim, mask: vec![false, true] });
        let mut o = opts(500);
        o.group = "b".into();
        let out = scatter_with(&d, &o).expect("cooks");
        assert_eq!(out.point_count, 500);
        assert_eq!(counts(&out), [0, 500]);
        assert!(out.p().iter().all(|q| q[0] >= 3.0 && q[0] <= 4.0));
        o.group = "nope".into();
        assert!(matches!(scatter_with(&d, &o), Err(CookError::Failed(_))));
    }
    /// Density mode: unit quad (area 1) x 10 = 10 pts; A4+B1 (area 5) x 10 = 50; 2.5x -> 125 (round).
    #[test]
    fn generateby_density_count_is_area_times_density() {
        let mut o = opts(7);
        o.generateby = 1;
        assert_eq!(scatter_with(&unit_quad(), &o).expect("cooks").point_count, 10);
        assert_eq!(scatter_with(&a4_b1(), &o).expect("cooks").point_count, 50);
        o.density = 25.0;
        assert_eq!(scatter_with(&a4_b1(), &o).expect("cooks").point_count, 125);
        o.density = 0.0;
        assert_eq!(scatter_with(&a4_b1(), &o).expect("cooks").point_count, 0);
    }
    /// Relax raises the minimum nearest-neighbour distance of 100 points on the unit quad and keeps every point inside it.
    #[test]
    fn relax_increases_min_neighbour_distance_and_stays_inside() {
        let q = unit_quad();
        let base = scatter_with(&q, &opts(100)).expect("cooks");
        let mut o = opts(100);
        o.relax = true;
        o.relax_iters = 20;
        let rel = scatter_with(&q, &o).expect("cooks");
        assert_eq!(rel.point_count, 100);
        let (b, r) = (nn_min(base.p()), nn_min(rel.p()));
        assert!(r > b * 2.0, "min NN {b} -> {r}");
        assert!(
            rel.p()
                .iter()
                .all(|p| (-1e-6..=1.0 + 1e-6).contains(&p[0]) && (-1e-6..=1.0 + 1e-6).contains(&p[1]) && p[2] == 0.0)
        );
    }
    /// Point attr w = x interpolates exactly to P.x (linear field, barycentric = exact); prim attr id copied per sourceprim; ints nearest; unlisted attrs absent.
    #[test]
    fn attribute_transfer_interpolates_points_and_copies_prims() {
        let mut d = a4_b1();
        let xs: Vec<f32> = d.p().iter().map(|p| p[0]).collect();
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: AttrData::F32(xs),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "id".into(),
            class: AttrClass::Prim,
            data: AttrData::I32(vec![10, 20]),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "skip".into(),
            class: AttrClass::Prim,
            data: AttrData::I32(vec![1, 2]),
            type_info: TypeInfo::None,
        });
        let mut o = opts(2000);
        o.attribs = "w id".into();
        let out = scatter_with(&d, &o).expect("cooks");
        let Some(AttrData::F32(w)) = out.attr(AttrClass::Point, "w").map(|a| &a.data) else { panic!("w") };
        for (p, &wv) in out.p().iter().zip(w) {
            assert!((p[0] - wv).abs() < 1e-5, "w {wv} vs x {}", p[0]);
        }
        let (Some(AttrData::I32(id)), Some(AttrData::I32(sp))) = (
            out.attr(AttrClass::Point, "id").map(|a| &a.data),
            out.attr(AttrClass::Point, "sourceprim").map(|a| &a.data),
        ) else {
            panic!("id/sourceprim")
        };
        assert!(id.iter().zip(sp).all(|(&i, &s)| i == 10 * (s + 1)));
        assert!(out.attr(AttrClass::Point, "skip").is_none());
        o.attribs = "*".into();
        assert!(scatter_with(&d, &o).expect("cooks").attr(AttrClass::Point, "skip").is_some());
        o.primnumattrib = String::new();
        o.attribs = String::new();
        assert!(scatter_with(&d, &o).expect("cooks").attr(AttrClass::Point, "sourceprim").is_none());
    }
    /// Emergency limit caps both modes and rescues a density count that would otherwise exceed the hard cap.
    #[test]
    fn emergency_limit_caps_count_and_density_modes() {
        let mut o = opts(500);
        o.emergency = 100;
        assert_eq!(scatter_with(&unit_quad(), &o).expect("cooks").point_count, 100);
        o.emergency = 900;
        assert_eq!(scatter_with(&unit_quad(), &o).expect("cooks").point_count, 500, "limit above count = no-op");
        // 100 x 100 quad (area 1e4) x density 1000 = 1e7 > MAX_POINTS
        let big = quads(&[[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 100.0, 0.0], [0.0, 100.0, 0.0]], &[[0, 1, 2, 3]]);
        let mut o = opts(7);
        o.generateby = 1;
        o.density = 1000.0;
        assert!(matches!(scatter_with(&big, &o), Err(CookError::Failed(_))));
        o.emergency = 1234;
        assert_eq!(scatter_with(&big, &o).expect("cooks").point_count, 1234);
        // unit quad x 25 = 25 < limit 40 -> untouched; limit 10 -> 10
        o.density = 25.0;
        o.emergency = 40;
        assert_eq!(scatter_with(&unit_quad(), &o).expect("cooks").point_count, 25);
        o.emergency = 10;
        assert_eq!(scatter_with(&unit_quad(), &o).expect("cooks").point_count, 10);
    }
    /// overridedensity: the attr is points-per-unit-area per prim; n = round(sum(area * attr) * densityscale) and the
    /// split follows area * attr (A: 4 * 10 = 40, B: 1 * 40 = 40 -> 1:1 although the areas are 4:1).
    #[test]
    fn override_density_attr_and_scale_drive_count_and_split() {
        let mut d = a4_b1();
        d.add_attr(Attribute {
            name: "dens".into(),
            class: AttrClass::Prim,
            data: AttrData::F32(vec![10.0, 40.0]),
            type_info: TypeInfo::None,
        });
        let mut o = opts(7);
        o.generateby = 1;
        o.override_on = true;
        o.override_attrib = "dens".into();
        o.density_scale = 100.0;
        let out = scatter_with(&d, &o).expect("cooks");
        assert_eq!(out.point_count, 8000);
        let [a, b] = counts(&out);
        assert!(((a as f64 / b as f64) - 1.0).abs() < 0.05, "a={a} b={b}");
        o.density_scale = 0.5;
        assert_eq!(scatter_with(&d, &o).expect("cooks").point_count, 40);
        // global `density` is ignored while overriding; off = the old behaviour (area-weighted, density 10 x 5 = 50)
        o.density = 999.0;
        assert_eq!(scatter_with(&d, &o).expect("cooks").point_count, 40);
        o.override_on = false;
        o.density = 10.0;
        o.density_scale = 1.0;
        assert_eq!(scatter_with(&d, &o).expect("cooks").point_count, 50);
        // missing / empty override attribute is a BadParam
        o.override_on = true;
        o.override_attrib = "nope".into();
        assert!(matches!(scatter_with(&d, &o), Err(CookError::BadParam(_))));
        o.override_attrib = String::new();
        assert!(matches!(scatter_with(&d, &o), Err(CookError::BadParam(_))));
    }
    /// outputprimuv: `primnum` == source prim, `uv` = parametric position. Quads A (x 0..2, y 0..2) and B (x 3..4, y 0..1)
    /// are bilinear = (x/2, y/2) and (x-3, y); a triangle (0,0),(4,0),(0,4) has (u,v) = (x/4, y/4).
    #[test]
    fn output_primuv_is_the_parametric_position_in_the_source_prim() {
        let mut o = opts(3000);
        o.output_primuv = true;
        let out = scatter_with(&a4_b1(), &o).expect("cooks");
        let (Some(AttrData::I32(pn)), Some(AttrData::V3(uv))) =
            (out.attr(AttrClass::Point, "primnum").map(|a| &a.data), out.attr(AttrClass::Point, "uv").map(|a| &a.data))
        else {
            panic!("primnum / uv")
        };
        assert_eq!(
            Some(pn),
            match out.attr(AttrClass::Point, "sourceprim").map(|a| &a.data) {
                Some(AttrData::I32(s)) => Some(s),
                _ => None,
            }
        );
        for ((q, &p), t) in out.p().iter().zip(pn).zip(uv) {
            let want = if p == 0 { [q[0] / 2.0, q[1] / 2.0] } else { [q[0] - 3.0, q[1]] };
            assert!(
                (t[0] - want[0]).abs() < 1e-5 && (t[1] - want[1]).abs() < 1e-5 && t[2] == 0.0,
                "{q:?} p={p} uv={t:?}"
            );
        }
        // default off: no primnum / uv attrs
        o.output_primuv = false;
        assert!(scatter_with(&a4_b1(), &o).expect("cooks").attr(AttrClass::Point, "uv").is_none());
        let mut tri = Detail::default();
        tri.add_points(3);
        tri.p_mut().copy_from_slice(&[[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]]);
        tri.add_poly(&[0, 1, 2], true);
        o.output_primuv = true;
        let out = scatter_with(&tri, &o).expect("cooks");
        let Some(AttrData::V3(uv)) = out.attr(AttrClass::Point, "uv").map(|a| &a.data) else { panic!("uv") };
        for (q, t) in out.p().iter().zip(uv) {
            assert!((t[0] - q[0] / 4.0).abs() < 1e-5 && (t[1] - q[1] / 4.0).abs() < 1e-5, "{q:?} uv={t:?}");
        }
    }
    #[test]
    fn glob_matches_star_patterns() {
        assert!(
            glob("*", "abc") && glob("a*", "abc") && glob("*c", "abc") && glob("a*c", "abc") && glob("a*b*c", "aXbYc")
        );
        assert!(!glob("a*d", "abc") && !glob("abc", "abd") && !glob("ab*bc", "abc"));
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
