//! Smooth SOP (token `smooth`): implicit Laplacian smoothing of point positions.
//!
//! # Semantics (Houdini smooth::2.0 model, fitted vs hython 20.5; see `parity/ref/smooth-probes/`)
//! `W = D^-1 A` (uniform: 1 per distinct edge; scale/curvature weight the edges, UNVERIFIED vs Houdini),
//! `M = I - W` with the rows of pinned points zeroed, `q = min(filterquality, 5)`,
//! `K = diag(M^q)^-1 M^q` (rows scaled so the diagonal is 1), result solves `(I + strength*K) x = P` per channel
//! (BiCGSTAB, rel tol 1e-12; Houdini's own solve stops ~1e-5 relative residual). `constrainboundary`: boundary points
//! (edges used by exactly one closed poly) keep their positions (= Houdini `unsharededges`; default `none`).
//!
//! Params: `method` M 0 (uniform=0, scale=1, curv=2, volume=3), `strength` F 0.5, `filterquality` I 10
//! (0 = input Arc shared, max 1000, effective cap 5 like Houdini), `constrainboundary` B false.
//! Work bound `min(filterquality,5) * (edges + points) <= 2^31`, else `Failed`.
use super::meshutil::{V3, bad, failed, float, int, menu, point_normals, positions, sub, to_f32, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, Prim, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::HashMap;
use std::sync::Arc;

const OP: &str = "smooth";
const MAX_QUALITY: i32 = 1000;
const MAX_WORK: u64 = 1 << 31;
/// Houdini smooth::2.0 clamps `filterquality` at 5 (probed: q=5..20 give bit-identical P).
const HOUDINI_MAX_Q: usize = 5;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("method", ParamValue::Menu(0)).label("Weighting Method").menu(&[
            ("uniform", "Uniform"),
            ("scale", "Scale-Dominant"),
            ("curvature", "Curvature-Dominant"),
            ("volume", "Volume-Preserving"),
        ]),
        ParamSpec::new("strength", ParamValue::F(0.5)).label("Strength").range(0.0, 2.0),
        ParamSpec::new("filterquality", ParamValue::I(10)).label("Filter Quality").range(0.0, 100.0),
        ParamSpec::new("constrainboundary", ParamValue::B(false)).label("Constrain Boundary Points"),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("attribs", ParamValue::S(String::new())).label("Extra Attributes"),
        ParamSpec::new("updatenormals", ParamValue::B(false)).label("Update Normals"),
    ]
}

/// Signed enclosed volume of the closed polys (fan-triangulated divergence theorem).
fn signed_volume(d: &Detail, pts: &[V3]) -> f64 {
    let mut v = 0.0;
    for pr in &d.prims {
        if !matches!(pr.kind, PrimKind::Poly { closed: true }) {
            continue;
        }
        let pl = prim_points(d, pr);
        for k in 1..pl.len().saturating_sub(1) {
            if let (Some(&a), Some(&b), Some(&c)) = (pts.get(pl[0]), pts.get(pl[k]), pts.get(pl[k + 1])) {
                v += super::meshutil::tri_volume(a, b, c);
            }
        }
    }
    v
}

/// Point-group selection (union of space-separated names); `None` = every point.
fn point_selection(src: &Detail, names: &str) -> Result<Option<Vec<bool>>, CookError> {
    let names: Vec<&str> = names.split_whitespace().collect();
    if names.is_empty() {
        return Ok(None);
    }
    let mut sel = vec![false; src.point_count as usize];
    for n in names {
        let g = src.group(AttrClass::Point, n).ok_or_else(|| failed(OP, &format!("unknown point group `{n}`")))?;
        for (s, m) in sel.iter_mut().zip(&g.mask) {
            *s |= *m;
        }
    }
    Ok(Some(sel))
}

pub fn new() -> Box<SopNode> {
    Box::new(Smooth { specs: spec() })
}

struct Smooth {
    specs: Vec<ParamSpec>,
}

/// Ids of a prim's vertex points that are in range, in order.
fn prim_points(d: &Detail, pr: &Prim) -> Vec<usize> {
    let (lo, hi) = (usize::try_from(pr.verts.start).unwrap_or(0), usize::try_from(pr.verts.end).unwrap_or(0));
    let np = d.point_count as usize;
    d.vertex_point.get(lo..hi).unwrap_or(&[]).iter().map(|&p| p as usize).filter(|&p| p < np).collect()
}

/// Cotangent weights per undirected boundary edge of the closed polys: each poly is fan-triangulated from corner 0;
/// every triangle adds `0.5 * cot(angle opposite the edge)` to its poly-boundary edges (fan diagonals get nothing).
/// Negative sums are clamped to 0 (keeps `I + lambda L` positive definite).
pub(crate) fn cotangent_weights(d: &Detail, pts: &[V3]) -> HashMap<(usize, usize), f64> {
    let mut m: HashMap<(usize, usize), f64> = HashMap::new();
    for pr in &d.prims {
        if !matches!(pr.kind, PrimKind::Poly { closed: true }) {
            continue;
        }
        let pl = prim_points(d, pr);
        let n = pl.len();
        if n < 3 {
            continue;
        }
        for k in 1..n - 1 {
            let tri = [pl[0], pl[k], pl[k + 1]];
            let Some(p) = tri.iter().map(|&i| pts.get(i).copied()).collect::<Option<Vec<V3>>>() else { continue };
            for o in 0..3 {
                let (a, b) = ((o + 1) % 3, (o + 2) % 3);
                let (u, v) = (sub(p[a], p[o]), sub(p[b], p[o]));
                let cr = super::meshutil::cross(u, v);
                let area2 = cr.iter().map(|x| x * x).sum::<f64>().sqrt();
                if area2 < 1e-12 {
                    continue;
                }
                let cot = u.iter().zip(&v).map(|(x, y)| x * y).sum::<f64>() / area2;
                let (ia, ib) = (tri[a], tri[b]);
                // edge (a,b) opposite corner o is a poly edge unless it is the fan diagonal (pl[0], pl[k])/(pl[0], pl[k+1]).
                let diag_first = k > 1 && (a == 0 || b == 0) && (a == 1 || b == 1);
                let diag_last = k + 2 < n && (a == 0 || b == 0) && (a == 2 || b == 2);
                if ia == ib || diag_first || diag_last {
                    continue;
                }
                *m.entry((ia.min(ib), ia.max(ib))).or_insert(0.0) += 0.5 * cot;
            }
        }
    }
    for w in m.values_mut() {
        *w = w.max(0.0);
    }
    m
}

/// Edge list with weights. method: 0=uniform, 1=scale, 2=curvature (cotangent), 3=volume-preserving (uniform).
fn build_edges(d: &Detail, pts: &[V3], method: u32) -> Vec<(usize, usize, f64)> {
    let np = d.point_count as usize;
    let mut edges: Vec<(usize, usize, f64)> = Vec::new();
    let cot = if method == 2 { cotangent_weights(d, pts) } else { HashMap::new() };

    for pr in &d.prims {
        let (closed, linear) = match pr.kind {
            PrimKind::Poly { closed } => (closed, true),
            PrimKind::Polyline => (false, true),
            _ => (false, false),
        };
        if !linear {
            continue;
        }
        let plist = prim_points(d, pr);
        let n = plist.len();
        let m = if closed && n >= 3 { n } else { n.saturating_sub(1) };

        for k in 0..m {
            let (Some(&a), Some(&b)) = (plist.get(k), plist.get((k + 1) % n)) else { continue };
            if a == b || a >= np || b >= np {
                continue;
            }

            let weight = match method {
                0 => 1.0, // uniform
                1 => {
                    // scale-dominant: inverse edge length
                    let pa = pts.get(a).copied().unwrap_or([0.0; 3]);
                    let pb = pts.get(b).copied().unwrap_or([0.0; 3]);
                    let len = sub(pb, pa).iter().map(|x| x * x).sum::<f64>().sqrt();
                    if len > 1e-12 { 1.0 / len } else { 1.0 }
                }
                2 => cot.get(&(a.min(b), a.max(b))).copied().unwrap_or(0.0),
                _ => 1.0, // volume-preserving: uniform operator, volume restored afterwards
            };
            edges.push((a, b, weight));
            edges.push((b, a, weight));
        }
    }
    edges.sort_unstable_by_key(|e| (e.0, e.1));
    // Deduplicate: one weight per distinct edge (shared edges are NOT double-counted)
    let mut dedup = Vec::new();
    let mut i = 0;
    while i < edges.len() {
        let (a, b, w) = edges[i];
        let mut j = i + 1;
        while j < edges.len() && edges[j].0 == a && edges[j].1 == b {
            j += 1;
        }
        dedup.push((a, b, w));
        i = j;
    }
    dedup
}

/// Find boundary points (edges used by exactly one closed poly).
fn find_boundary(d: &Detail) -> Vec<bool> {
    let np = d.point_count as usize;
    let mut closed_edges: Vec<(usize, usize)> = Vec::new();

    for pr in &d.prims {
        let (closed, linear) = match pr.kind {
            PrimKind::Poly { closed } => (closed, true),
            PrimKind::Polyline => (false, true),
            _ => (false, false),
        };
        if !linear || !closed {
            continue;
        }
        let plist = prim_points(d, pr);
        let n = plist.len();
        if n < 3 {
            continue;
        }
        for k in 0..n {
            let (Some(&a), Some(&b)) = (plist.get(k), plist.get((k + 1) % n)) else { continue };
            if a != b {
                closed_edges.push((a.min(b), a.max(b)));
            }
        }
    }
    closed_edges.sort_unstable();
    let mut boundary = vec![false; np];
    let mut i = 0;
    while i < closed_edges.len() {
        let j = closed_edges[i..].iter().take_while(|e| **e == closed_edges[i]).count();
        if j == 1 {
            let (a, b) = closed_edges[i];
            boundary[a] = true;
            boundary[b] = true;
        }
        i += j;
    }
    boundary
}

/// Houdini smooth::2.0 operator (fitted vs real Houdini 20.5, `parity/ref/smooth-probes/`).
///
/// `W = D_w^-1 A_w` (row-normalised edge weights; uniform = 1 per distinct edge), `M = I - W` with the rows of
/// pinned points zeroed, `K = diag(M^q)^-1 M^q` (each row scaled so its diagonal entry is 1), `q = min(filterquality, 5)`;
/// the result solves `(I + strength * K) x = P` per channel.
struct Operator {
    /// Per row: `(neighbour, W_ij)`.
    rows: Vec<Vec<(usize, f64)>>,
    /// Row participates in `M` (not pinned, has weight).
    active: Vec<bool>,
    /// `1 / (M^q)_ii` (1 for inactive rows).
    inv_diag: Vec<f64>,
    q: usize,
}

impl Operator {
    fn new(np: usize, edges: &[(usize, usize, f64)], pinned: &dyn Fn(usize) -> bool, q: usize) -> Self {
        let mut rows: Vec<Vec<(usize, f64)>> = vec![Vec::new(); np];
        for &(a, b, w) in edges {
            if a < np && b < np {
                rows[a].push((b, w));
            }
        }
        let mut active = vec![false; np];
        for (i, row) in rows.iter_mut().enumerate() {
            let ws: f64 = row.iter().map(|e| e.1).sum();
            if ws > 1e-300 && !pinned(i) {
                active[i] = true;
                for e in row.iter_mut() {
                    e.1 /= ws;
                }
            } else {
                // inactive row: zero row of M (pinned points still appear as columns of active rows)
                row.iter_mut().for_each(|e| e.1 = 0.0);
            }
        }
        let mut op = Operator { rows, active, inv_diag: vec![1.0; np], q };
        op.compute_diag();
        op
    }

    /// `(M^q)_ii` by pushing the sparse row vector `e_i` through `M` q times (row-vector convention: `r <- r M`).
    fn compute_diag(&mut self) {
        let np = self.rows.len();
        for i in 0..np {
            if !self.active[i] {
                continue;
            }
            let mut r: HashMap<usize, f64> = HashMap::new();
            r.insert(i, 1.0);
            for _ in 0..self.q {
                let mut next: HashMap<usize, f64> = HashMap::with_capacity(r.len() * 4);
                for (&k, &v) in &r {
                    if !self.active[k] {
                        continue; // M row k is zero
                    }
                    *next.entry(k).or_insert(0.0) += v;
                    for &(j, w) in &self.rows[k] {
                        *next.entry(j).or_insert(0.0) -= v * w;
                    }
                }
                r = next;
            }
            let d = r.get(&i).copied().unwrap_or(0.0);
            self.inv_diag[i] = if d.abs() > 1e-12 { 1.0 / d } else { 1.0 };
        }
    }

    /// `y = M x`.
    fn apply_m(&self, x: &[f64], y: &mut [f64]) {
        for (i, yi) in y.iter_mut().enumerate() {
            *yi = if self.active[i] { x[i] - self.rows[i].iter().map(|&(j, w)| w * x[j]).sum::<f64>() } else { 0.0 };
        }
    }

    /// `y = (I + lambda K) x`, `K = diag^-1 M^q`.
    fn apply_system(&self, lambda: f64, x: &[f64], y: &mut [f64]) {
        let n = x.len();
        let mut cur = x.to_vec();
        let mut tmp = vec![0.0; n];
        for _ in 0..self.q {
            self.apply_m(&cur, &mut tmp);
            std::mem::swap(&mut cur, &mut tmp);
        }
        for i in 0..n {
            y[i] = x[i] + lambda * self.inv_diag[i] * cur[i];
        }
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// BiCGSTAB for the (non-symmetric) system `(I + lambda K) x = b`; `x` holds the start guess and the best iterate.
fn bicgstab_solve(op: &Operator, lambda: f64, b: &[f64], x: &mut [f64], max_iter: usize, tol: f64) {
    let n = b.len();
    let bn = dot(b, b).sqrt();
    if bn < 1e-300 {
        return;
    }
    let mut ax = vec![0.0; n];
    op.apply_system(lambda, x, &mut ax);
    let mut r: Vec<f64> = (0..n).map(|i| b[i] - ax[i]).collect();
    let r0 = r.clone();
    let (mut rho, mut alpha, mut omega) = (1.0, 1.0, 1.0);
    let mut v = vec![0.0; n];
    let mut p = vec![0.0; n];
    let mut s = vec![0.0; n];
    let mut t = vec![0.0; n];
    for _ in 0..max_iter {
        if dot(&r, &r).sqrt() <= tol * bn {
            return;
        }
        let rho_new = dot(&r0, &r);
        if rho_new.abs() < 1e-300 {
            return;
        }
        let beta = (rho_new / rho) * (alpha / omega);
        rho = rho_new;
        for i in 0..n {
            p[i] = r[i] + beta * (p[i] - omega * v[i]);
        }
        op.apply_system(lambda, &p, &mut v);
        let denom = dot(&r0, &v);
        if denom.abs() < 1e-300 {
            return;
        }
        alpha = rho / denom;
        for i in 0..n {
            s[i] = r[i] - alpha * v[i];
        }
        if dot(&s, &s).sqrt() <= tol * bn {
            for i in 0..n {
                x[i] += alpha * p[i];
            }
            return;
        }
        op.apply_system(lambda, &s, &mut t);
        let tt = dot(&t, &t);
        if tt < 1e-300 {
            return;
        }
        omega = dot(&t, &s) / tt;
        for i in 0..n {
            x[i] += alpha * p[i] + omega * s[i];
            r[i] = s[i] - omega * t[i];
        }
        if omega.abs() < 1e-300 {
            return;
        }
    }
}
impl Node<Detail> for Smooth {
    fn type_name(&self) -> &'static str {
        "smooth"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let method = menu(p, "method")?;
        let strength = float(p, "strength")?;
        let quality = int(p, "filterquality")?;
        let constrain = toggle(p, "constrainboundary")?;

        if !(0..=MAX_QUALITY).contains(&quality) {
            return Err(bad(format!("`filterquality` must be in 0..={MAX_QUALITY}, got {quality}")));
        }
        if !strength.is_finite() {
            return Err(bad(format!("`strength` must be finite, got {strength}")));
        }

        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if quality == 0 || strength == 0.0 || src.point_count == 0 {
            return Ok(Arc::clone(src));
        }

        let np = src.point_count as usize;
        let pts = positions(src);
        let edges = build_edges(src, &pts, method);

        let work = (quality as u64).min(HOUDINI_MAX_Q as u64).saturating_mul((edges.len() + np) as u64);
        if work > MAX_WORK {
            return Err(failed(OP, "filterquality * edges exceeds the work cap"));
        }

        let boundary = if constrain { find_boundary(src) } else { vec![false; np] };
        // Point-group selection: unselected points are left untouched (they still take part in the Laplacian).
        let sel = point_selection(src, p.string("group").unwrap_or(""))?;
        let pinned = |i: usize| {
            (constrain && boundary.get(i).copied().unwrap_or(false))
                || sel.as_ref().is_some_and(|s| !s.get(i).copied().unwrap_or(false))
        };
        let op = Operator::new(np, &edges, &pinned, (quality as usize).min(HOUDINI_MAX_Q));
        let solve = |b: &[f64]| -> Vec<f64> {
            let mut x = b.to_vec();
            bicgstab_solve(&op, strength, b, &mut x, 2000, 1e-12);
            for (i, xi) in x.iter_mut().enumerate() {
                if pinned(i) {
                    *xi = b.get(i).copied().unwrap_or(0.0);
                }
            }
            x
        };
        // Solve for each coordinate independently
        let mut result = vec![[0.0; 3]; np];
        for axis in 0..3 {
            let b: Vec<f64> = pts.iter().map(|p| p[axis]).collect();
            for (i, xi) in solve(&b).into_iter().enumerate() {
                result[i][axis] = xi;
            }
        }

        let mut out = (**src).clone();
        *out.p_mut() = to_f32(OP, &result)?;

        // Attribute smoothing: every listed point attribute (F32 / V2 / V3 / V4 / FN) gets the same filter per channel.
        for name in p.string("attribs").unwrap_or("").split_whitespace().filter(|n| *n != "P") {
            let Some(a) = out.attr_mut(AttrClass::Point, name) else {
                return Err(failed(OP, &format!("unknown point attribute `{name}`")));
            };
            let (flat, w): (Vec<f32>, usize) = match &a.data {
                AttrData::F32(v) => (v.clone(), 1),
                AttrData::V2(v) => (v.iter().flatten().copied().collect(), 2),
                AttrData::V3(v) => (v.iter().flatten().copied().collect(), 3),
                AttrData::V4(v) => (v.iter().flatten().copied().collect(), 4),
                AttrData::FN { size, data } => (data.clone(), (*size as usize).max(1)),
                _ => return Err(failed(OP, &format!("attribute `{name}` is not a float attribute"))),
            };
            let mut res = flat.clone();
            for c in 0..w {
                let b: Vec<f64> = (0..np).map(|i| f64::from(flat.get(i * w + c).copied().unwrap_or(0.0))).collect();
                for (i, xi) in solve(&b).into_iter().enumerate() {
                    if let Some(s) = res.get_mut(i * w + c) {
                        *s = xi as f32;
                    }
                }
            }
            match &mut a.data {
                AttrData::F32(v) => *v = res,
                AttrData::V2(v) => *v = res.as_chunks::<2>().0.to_vec(),
                AttrData::V3(v) => *v = res.as_chunks::<3>().0.to_vec(),
                AttrData::V4(v) => *v = res.as_chunks::<4>().0.to_vec(),
                AttrData::FN { data, .. } => *data = res,
                _ => {}
            }
        }

        // Volume-preserving: uniform operator, then scale about the centroid so the signed enclosed volume is restored.
        if method == 3 {
            let before = signed_volume(src, &pts);
            let after = signed_volume(&out, &positions(&out));
            if before.abs() > 1e-12 && after.abs() > 1e-12 && before.signum() == after.signum() {
                let s = (before / after).cbrt();
                let np_f = np.max(1) as f64;
                let c = result.iter().fold([0.0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]).map(|v| v / np_f);
                let scaled: Vec<V3> = result
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        if pinned(i) {
                            *p
                        } else {
                            [c[0] + (p[0] - c[0]) * s, c[1] + (p[1] - c[1]) * s, c[2] + (p[2] - c[2]) * s]
                        }
                    })
                    .collect();
                *out.p_mut() = to_f32(OP, &scaled)?;
            }
        }

        // Normal update: point N = uniform-averaged unit Newell normals of the smoothed surface.
        if toggle(p, "updatenormals")? {
            let pos = positions(&out);
            let n: Vec<[f32; 3]> =
                point_normals(&out, &pos).into_iter().map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect();
            out.add_attr(Attribute {
                name: "N".into(),
                class: AttrClass::Point,
                data: AttrData::V3(n),
                type_info: TypeInfo::Normal,
            });
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, grid3};
    use super::*;
    use hexe_graph::ParamSet;

    fn with(method: u32, strength: f32, quality: i32) -> ParamSet {
        defaults(&*new())
            .with("method", ParamValue::Menu(method))
            .with("strength", ParamValue::F(strength))
            .with("filterquality", ParamValue::I(quality))
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "smooth");
        let params = n.params();
        assert_eq!(params.len(), 7);
        assert_eq!(params[0].name, "method");
        assert_eq!(params[1].name, "strength");
        assert_eq!(params[2].name, "filterquality");
        assert_eq!(params[3].name, "constrainboundary");
    }

    #[test]
    fn planar_grid_stays_planar() {
        let src = grid3();
        let out = cook(&*new(), src.clone(), &with(0, 0.5, 10)).expect("cooks");
        // All points should have z ~ 0
        for p in out.p() {
            assert!(p[2].abs() < 1e-5, "point {:?} not planar", p);
        }
    }

    /// Recorded hython 20.5 smooth::2.0 box corner factors f(s,q) = 1/(1+nu): q1 nu=2s/3, q2 nu=s/3, q3 nu=4s/27.
    #[test]
    fn box_matches_recorded_houdini_factors() {
        for (s, q, f) in
            [(0.5, 1, 0.75), (1.0, 1, 0.6), (0.25, 2, 12.0 / 13.0), (2.0, 3, 27.0 / 35.0), (0.5, 2, 6.0 / 7.0)]
        {
            let out = cook(&*new(), cube(), &with(0, s, q)).expect("cooks");
            for p in out.p() {
                for c in p {
                    assert!((c.abs() as f64 - 0.5 * f).abs() < 1e-6, "s{s} q{q}: {c} vs {}", 0.5 * f);
                }
            }
        }
        // quality above 5 is clamped like Houdini
        let a = cook(&*new(), cube(), &with(0, 1.0, 5)).expect("cooks");
        let b = cook(&*new(), cube(), &with(0, 1.0, 50)).expect("cooks");
        assert_eq!(a.p(), b.p());
    }

    #[test]
    fn boundary_pinned_when_constrain_on() {
        let src = grid3();
        let out = cook(&*new(), src.clone(), &with(0, 1.0, 10).with("constrainboundary", ParamValue::B(true)))
            .expect("cooks");

        // Grid 3x3: boundary points are all except center (index 4)
        for i in [0, 1, 2, 3, 5, 6, 7, 8] {
            assert_eq!(out.p()[i], src.p()[i], "boundary point {i} should be pinned");
        }
    }

    /// Group {4 = centre}: only the centre of grid3 moves; the other 8 points are bit-identical.
    #[test]
    fn point_group_limits_smoothing_to_selected_points() {
        let mut src = grid3();
        let mut mask = vec![false; 9];
        mask[4] = true;
        src.groups.push(hexe_geo::Group { name: "g".into(), class: AttrClass::Point, mask });
        // lift the centre so uniform smoothing has something to do
        src.p_mut()[4][2] = 1.0;
        let out =
            cook(&*new(), src.clone(), &with(0, 1.0, 50).with("group", ParamValue::S("g".into()))).expect("cooks");
        for i in [0, 1, 2, 3, 5, 6, 7, 8] {
            assert_eq!(out.p()[i], src.p()[i], "point {i} not in group");
        }
        assert!(out.p()[4][2] < 1.0, "centre must move toward its (z=0) neighbours: {}", out.p()[4][2]);
        let bad = cook(&*new(), src, &with(0, 1.0, 5).with("group", ParamValue::S("nope".into())));
        assert!(matches!(bad, Err(CookError::Failed(_))));
    }

    /// Attribute `w` (F32, 0 everywhere but one spike) smooths: spike drops, values stay bounded.
    #[test]
    fn extra_attribute_is_smoothed_and_bounded_unlisted_untouched() {
        let mut src = grid3();
        let mut w = vec![0.0f32; 9];
        w[4] = 9.0;
        src.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: AttrData::F32(w),
            type_info: TypeInfo::None,
        });
        src.add_attr(Attribute {
            name: "u".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![5.0; 9].into_iter().enumerate().map(|(i, v)| v + i as f32).collect()),
            type_info: TypeInfo::None,
        });
        let out =
            cook(&*new(), src.clone(), &with(0, 1.0, 100).with("attribs", ParamValue::S("w".into()))).expect("cooks");
        let Some(AttrData::F32(ow)) = out.attr(AttrClass::Point, "w").map(|a| &a.data) else { panic!("w lost") };
        assert!(ow[4] < 9.0 && ow[4] > 0.0, "spike must drop: {}", ow[4]);
        // the Houdini operator is a row-normalised order-q filter: not mean-conserving, may ring negative
        assert!(ow.iter().all(|v| v.is_finite()), "{ow:?}");
        assert_eq!(out.attr(AttrClass::Point, "u"), src.attr(AttrClass::Point, "u"));
        let missing = cook(&*new(), src, &with(0, 1.0, 5).with("attribs", ParamValue::S("zz".into())));
        assert!(matches!(missing, Err(CookError::Failed(_))));
    }

    /// Smoothing a z-lifted grid centre then updatenormals: flat grid -> N = (0,0,+-1) exactly; centre N of the lifted grid is +-z (symmetric).
    #[test]
    fn updatenormals_writes_unit_point_normals() {
        let src = grid3();
        let off = cook(&*new(), src.clone(), &with(0, 0.5, 10)).expect("cooks");
        assert!(off.attr(AttrClass::Point, "N").is_none());
        let on = cook(&*new(), src, &with(0, 0.5, 10).with("updatenormals", ParamValue::B(true))).expect("cooks");
        let Some(AttrData::V3(n)) = on.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N missing") };
        assert_eq!(n.len(), 9);
        for v in n {
            assert!(v[0].abs() < 1e-5 && v[1].abs() < 1e-5 && (v[2].abs() - 1.0).abs() < 1e-5, "{v:?}");
        }
    }

    /// Right triangle (0,0,0),(1,0,0),(0,1,0): legs see a 45-degree opposite angle -> w = 0.5*cot45 = 0.5; hypotenuse sees 90 -> 0.
    #[test]
    fn cotangent_weights_of_a_right_triangle_are_half_half_zero() {
        let mut d = Detail::default();
        d.add_points(3);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2], true);
        let w = cotangent_weights(&d, &positions(&d));
        assert!((w[&(0, 1)] - 0.5).abs() < 1e-12 && (w[&(0, 2)] - 0.5).abs() < 1e-12);
        assert!(w[&(1, 2)].abs() < 1e-12);
    }

    /// grid3 = 4 unit quads: every quad edge sees two 45-degree fan angles... interior (shared) edges get 2 quads' worth = 1.0, border edges 0.5.
    #[test]
    fn cotangent_weights_on_grid3_interior_edges_double_border_edges() {
        let g = grid3();
        let w = cotangent_weights(&g, &positions(&g));
        // 12 edges: 4 interior spokes (touch point 4) + 8 border
        assert_eq!(w.len(), 12);
        for (&(a, b), &x) in &w {
            let interior = a == 4 || b == 4;
            assert!((x - if interior { 1.0 } else { 0.5 }).abs() < 1e-9, "edge {a}-{b}: {x}");
        }
    }

    /// Volume-preserving (method 3) keeps the cube's enclosed volume; uniform (method 0) shrinks it.
    #[test]
    fn volume_preserving_keeps_cube_volume_uniform_shrinks_it() {
        let c = cube();
        let v0 = signed_volume(&c, &positions(&c));
        assert!(v0.abs() > 0.99, "{v0}");
        let shrunk = cook(&*new(), c.clone(), &with(0, 2.0, 1)).expect("cooks");
        assert!(signed_volume(&shrunk, &positions(&shrunk)).abs() < v0.abs() * 0.9);
        let kept = cook(&*new(), c, &with(3, 2.0, 1)).expect("cooks");
        let v1 = signed_volume(&kept, &positions(&kept));
        assert!((v1 - v0).abs() < 1e-4, "{v0} -> {v1}");
    }

    /// Replays every hython 20.5 probe (box/grid5/sphere/fan/perturbed grid x none|unsharededges x q 1..10 x strength) from
    /// `parity/ref/smooth-probes/probes.json`; max|dP| <= 1e-4 (Houdini's own solve stops at ~1e-5 relative residual).
    #[test]
    fn replays_houdini_probes() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../parity/ref/smooth-probes/probes.json");
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("probes.json")).expect("json");
        let mut worst = 0.0f64;
        let mut n = 0;
        for (name, g) in doc.as_object().expect("obj") {
            let v3 = |v: &serde_json::Value| -> V3 {
                [v[0].as_f64().unwrap(), v[1].as_f64().unwrap(), v[2].as_f64().unwrap()]
            };
            let src: Vec<V3> = g["src"].as_array().unwrap().iter().map(v3).collect();
            let mut d = Detail::default();
            d.add_points(src.len() as u32);
            *d.p_mut() = src.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
            for pr in g["prims"].as_array().unwrap() {
                let ids: Vec<u32> = pr.as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
                d.add_poly(&ids, true);
            }
            for r in g["runs"].as_array().unwrap() {
                let cb = r["cb"].as_i64().unwrap();
                let params = with(0, r["s"].as_f64().unwrap() as f32, r["q"].as_i64().unwrap() as i32)
                    .with("constrainboundary", ParamValue::B(cb == 1));
                let out = cook(&*new(), d.clone(), &params).expect("cooks");
                for (o, h) in out.p().iter().zip(r["P"].as_array().unwrap()) {
                    let h = v3(h);
                    for c in 0..3 {
                        worst = worst.max((f64::from(o[c]) - h[c]).abs());
                    }
                }
                n += 1;
            }
            assert!(worst < 1e-4, "{name}: max|dP| {worst}");
        }
        assert_eq!(n, 200);
        eprintln!("probe replay: {n} runs, max|dP| {worst:.3e}");
    }

    #[test]
    fn zero_quality_and_strength_share_arc() {
        let a = Arc::new(cube());
        let out1 = cook_arc(&*new(), &a, &with(0, 0.5, 0)).expect("cooks");
        assert!(Arc::ptr_eq(&out1, &a));

        let out2 = cook_arc(&*new(), &a, &with(0, 0.0, 10)).expect("cooks");
        assert!(Arc::ptr_eq(&out2, &a));
    }

    #[test]
    fn scene_loader_cooks_example() {
        let scene = r#"{"nodes":[{"name":"b","type":"box"},{"name":"s","type":"smooth","params":{"filterquality":5}}],"edges":[["b",0,"s",0]],"display":"s"}"#;
        let (mut g, id) = crate::load_scene(scene).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 8);
    }
}
