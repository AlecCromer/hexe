//! NURBS / Bezier curve + surface prims (`PrimKind::{BezierCurve, NurbsCurve, BezierSurf, NurbsSurf}`): evaluation + tessellation.
//!
//! * Curves: unit parameter `u in [0,1]` maps linearly onto the native domain (NURBS `[t_{k-1}, t_{n_eff}]`, Bezier `[b_0, b_last]`).
//! * NURBS = de Boor, Bezier = de Casteljau; both are run on identity coefficients so one pass yields the sparse rational-free
//!   basis weights `(cv, N_i)`; rational weights (`Pw` point attr) are folded in afterwards (`R_i = N_i Pw_i / sum`). The same
//!   weights blend every point attribute onto tessellated points.
//! * Surfaces: tensor product (`N_i(u) N_j(v)`), CV `r*cols + c`, u along `c`.
//! * Hostile input: every plan is validated, orders capped at 64, sample counts capped, failures are `None`/fallbacks, never panics.
use crate::types::*;

const MAX_ORDER: usize = 64;
const MAX_LOD: u32 = 1024;
const MAX_SAMPLES: usize = 1 << 22;

impl PrimKind {
    pub fn is_spline_curve(&self) -> bool {
        matches!(self, PrimKind::BezierCurve { .. } | PrimKind::NurbsCurve { .. })
    }
    pub fn is_spline_surf(&self) -> bool {
        matches!(self, PrimKind::BezierSurf { .. } | PrimKind::NurbsSurf { .. })
    }
    /// Any of the four NURBS/Bezier kinds.
    pub fn is_spline(&self) -> bool {
        self.is_spline_curve() || self.is_spline_surf()
    }
}

/// Houdini-stored basis (`Basis k [end|noend|use] knots..`) -> canonical [`Basis`] (see its docs). Only the open+`end` NURBS form
/// (`n-k+2` stored values: end values once + interior) and Bezier `use` breaks are interpreted; every other stored form falls back
/// to the default knots (LOSSY, UNVERIFIED vs Houdini: the closed / `noend` stored layouts are not decoded).
pub(crate) fn basis_from_stored(order: u32, end: bool, bezier: bool, closed: bool, n: usize, stored: &[f32]) -> Basis {
    let k = order as usize;
    let mut b = Basis { order, knots: Vec::new(), end };
    if k < 2 || stored.is_empty() {
        return b;
    }
    if bezier {
        let deg = k - 1;
        let segs = if closed { n / deg } else { n.saturating_sub(1) / deg };
        if closed && stored.len() == segs && segs >= 1 {
            let last = stored[segs - 1];
            let step = if segs >= 2 { last - stored[segs - 2] } else { 1.0 };
            b.knots = stored.to_vec();
            b.knots.push(last + step);
        } else if stored.len() == segs + 1 {
            b.knots = stored.to_vec();
        }
    } else if end && !closed && n >= k && stored.len() == n - k + 2 {
        let mut kn = Vec::with_capacity(n + k);
        kn.extend(std::iter::repeat_n(stored[0], k));
        kn.extend_from_slice(&stored[1..stored.len() - 1]);
        kn.extend(std::iter::repeat_n(stored[stored.len() - 1], k));
        b.knots = kn;
    }
    b
}

enum Kind {
    Nurbs { knots: Vec<f32>, n_eff: usize },
    Bezier { breaks: Vec<f32>, segs: usize },
}

/// A validated spline direction.
struct Plan {
    k: usize,
    n: usize,
    closed: bool,
    kind: Kind,
}

fn finite_nondecreasing(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite()) && v.windows(2).all(|w| w[0] <= w[1])
}

impl Plan {
    fn new(b: &Basis, n: usize, closed: bool, bezier: bool) -> Option<Plan> {
        let k = b.order as usize;
        if !(2..=MAX_ORDER).contains(&k) || n == 0 || n > MAX_SAMPLES {
            return None;
        }
        let kind = if bezier {
            let deg = k - 1;
            let segs = if closed {
                if n < deg || !n.is_multiple_of(deg) {
                    return None;
                }
                n / deg
            } else {
                if n < k || !(n - 1).is_multiple_of(deg) {
                    return None;
                }
                (n - 1) / deg
            };
            let ok =
                b.knots.len() == segs + 1 && finite_nondecreasing(&b.knots) && b.knots.windows(2).all(|w| w[0] < w[1]);
            let breaks = if ok { b.knots.clone() } else { (0..=segs).map(|i| i as f32).collect() };
            Kind::Bezier { breaks, segs }
        } else {
            if closed && n < 2 {
                return None;
            }
            let n_eff = if closed { n + k - 1 } else { n };
            if n_eff < k {
                return None;
            }
            let custom =
                b.knots.len() == n_eff + k && finite_nondecreasing(&b.knots) && b.knots[n_eff] > b.knots[k - 1];
            let knots = if custom {
                b.knots.clone()
            } else if b.end && !closed {
                (0..n + k)
                    .map(|i| {
                        if i < k {
                            0.0
                        } else if i < n {
                            (i - k + 1) as f32
                        } else {
                            (n - k + 1) as f32
                        }
                    })
                    .collect()
            } else {
                (0..n_eff + k).map(|i| i as f32).collect()
            };
            Kind::Nurbs { knots, n_eff }
        };
        Some(Plan { k, n, closed, kind })
    }
    fn domain(&self) -> (f32, f32) {
        match &self.kind {
            Kind::Nurbs { knots, n_eff } => (knots[self.k - 1], knots[*n_eff]),
            Kind::Bezier { breaks, segs } => (breaks[0], breaks[*segs]),
        }
    }
    /// Distinct parameter values of the domain (span boundaries).
    fn breaks(&self) -> Vec<f32> {
        match &self.kind {
            Kind::Nurbs { knots, n_eff } => {
                let mut v: Vec<f32> = Vec::new();
                for &t in &knots[self.k - 1..=*n_eff] {
                    if v.last().is_none_or(|&l| t > l) {
                        v.push(t);
                    }
                }
                v
            }
            Kind::Bezier { breaks, .. } => breaks.clone(),
        }
    }
    /// `lod` samples per span; the closing end value is included only when open.
    fn samples(&self, lod: u32) -> Option<Vec<f32>> {
        let lod = lod.clamp(1, MAX_LOD) as usize;
        let br = self.breaks();
        let spans = br.len().saturating_sub(1);
        if spans.saturating_mul(lod) > MAX_SAMPLES {
            return None;
        }
        let mut out = Vec::with_capacity(spans * lod + 1);
        for a in 0..spans {
            for s in 0..lod {
                out.push(br[a] + (br[a + 1] - br[a]) * (s as f32 / lod as f32));
            }
        }
        if !self.closed {
            out.push(*br.last()?);
        }
        Some(out)
    }
    /// Sparse basis weights `(cv ordinal, N)` at native parameter `t` (clamped to the domain).
    fn weights(&self, t: f32) -> Vec<(usize, f32)> {
        let (k, p) = (self.k, self.k - 1);
        let (t0, t1) = self.domain();
        let t = if t.is_nan() { t0 } else { t.clamp(t0, t1) };
        let ident = || -> Vec<Vec<f32>> {
            (0..k)
                .map(|j| {
                    let mut v = vec![0.0f32; k];
                    v[j] = 1.0;
                    v
                })
                .collect()
        };
        match &self.kind {
            Kind::Nurbs { knots, n_eff } => {
                // span s in [k-1, n_eff-1] with knots[s] <= t < knots[s+1] (de Boor)
                let c = knots[p..*n_eff].partition_point(|&x| x <= t);
                let mut s = p + c.max(1) - 1;
                while s > p && knots[s] >= knots[s + 1] {
                    s -= 1;
                }
                let mut d = ident();
                for r in 1..=p {
                    for j in (r..=p).rev() {
                        let i = j + s - p;
                        let den = knots[j + 1 + s - r] - knots[i];
                        let a = if den > 0.0 { (t - knots[i]) / den } else { 0.0 };
                        let (lo, hi) = d.split_at_mut(j);
                        for m in 0..k {
                            hi[0][m] = (1.0 - a) * lo[j - 1][m] + a * hi[0][m];
                        }
                    }
                }
                let w = &d[p];
                (0..k)
                    .map(|m| {
                        let i = s - p + m;
                        (if self.closed { i % self.n } else { i }, w[m])
                    })
                    .collect()
            }
            Kind::Bezier { breaks, segs } => {
                let c = breaks[..*segs].partition_point(|&x| x <= t);
                let i = c.max(1) - 1;
                let (b0, b1) = (breaks[i], breaks[i + 1]);
                let s = if b1 > b0 { (t - b0) / (b1 - b0) } else { 0.0 };
                let mut d = ident();
                for r in 1..k {
                    for j in 0..k - r {
                        let (lo, hi) = d.split_at_mut(j + 1);
                        for m in 0..k {
                            lo[j][m] = (1.0 - s) * lo[j][m] + s * hi[0][m];
                        }
                    }
                }
                (0..k)
                    .map(|m| {
                        let ci = i * p + m;
                        (if self.closed { ci % self.n } else { ci }, d[0][m])
                    })
                    .collect()
            }
        }
    }
}

/// Directional plans of one prim.
enum Plans {
    Curve(Plan),
    Surf { u: Plan, v: Plan, cols: usize },
}

fn plans(kind: &PrimKind, nverts: usize) -> Option<Plans> {
    match kind {
        PrimKind::BezierCurve { basis, closed } => Some(Plans::Curve(Plan::new(basis, nverts, *closed, true)?)),
        PrimKind::NurbsCurve { basis, closed } => Some(Plans::Curve(Plan::new(basis, nverts, *closed, false)?)),
        PrimKind::BezierSurf { cols, rows, ubasis, vbasis, uwrap, vwrap }
        | PrimKind::NurbsSurf { cols, rows, ubasis, vbasis, uwrap, vwrap } => {
            let bez = matches!(kind, PrimKind::BezierSurf { .. });
            let (c, r) = (*cols as usize, *rows as usize);
            if c.checked_mul(r)? != nverts {
                return None;
            }
            Some(Plans::Surf { u: Plan::new(ubasis, c, *uwrap, bez)?, v: Plan::new(vbasis, r, *vwrap, bez)?, cols: c })
        }
        _ => None,
    }
}

/// One tessellated spline prim: new-point blend weights `(source point, weight)` + polys over new-point ordinals.
struct Tess {
    samples: Vec<Vec<(PointId, f32)>>,
    polys: Vec<(Vec<u32>, bool)>,
}

impl AttrData {
    /// Blend `samples` (each `(index, weight)` over this attribute's elements) into new elements. Floats: weighted sum.
    /// Ints / strings: value of the heaviest source. Out-of-range indices contribute nothing.
    pub(crate) fn blend(&self, samples: &[Vec<(PointId, f32)>]) -> AttrData {
        fn heaviest(s: &[(PointId, f32)]) -> Option<usize> {
            let mut best: Option<(usize, f32)> = None;
            for &(i, w) in s {
                if best.is_none_or(|(_, bw)| w > bw) {
                    best = Some((i as usize, w));
                }
            }
            best.map(|b| b.0)
        }
        fn sum<const N: usize>(src: &[[f32; N]], s: &[(PointId, f32)]) -> [f32; N] {
            let mut o = [0.0f32; N];
            for &(i, w) in s {
                if let Some(x) = src.get(i as usize) {
                    for c in 0..N {
                        o[c] += w * x[c];
                    }
                }
            }
            o
        }
        match self {
            AttrData::F32(v) => AttrData::F32(
                samples
                    .iter()
                    .map(|s| s.iter().map(|&(i, w)| w * v.get(i as usize).copied().unwrap_or(0.0)).sum())
                    .collect(),
            ),
            AttrData::V2(v) => AttrData::V2(samples.iter().map(|s| sum::<2>(v, s)).collect()),
            AttrData::V3(v) => AttrData::V3(samples.iter().map(|s| sum::<3>(v, s)).collect()),
            AttrData::V4(v) => AttrData::V4(samples.iter().map(|s| sum::<4>(v, s)).collect()),
            AttrData::FN { size, data } => {
                let sz = (*size as usize).max(1);
                let mut out = Vec::with_capacity(samples.len() * sz);
                for s in samples {
                    let mut o = vec![0.0f32; sz];
                    for &(i, w) in s {
                        if let Some(x) = data.get(i as usize * sz..(i as usize + 1) * sz) {
                            for c in 0..sz {
                                o[c] += w * x[c];
                            }
                        }
                    }
                    out.extend(o);
                }
                AttrData::FN { size: *size, data: out }
            }
            _ => {
                let idx: Vec<usize> = samples.iter().map(|s| heaviest(s).unwrap_or(usize::MAX)).collect();
                self.gather(&idx)
            }
        }
    }
    /// Elements at `idx` (out of range / `usize::MAX` -> default).
    pub(crate) fn gather(&self, idx: &[usize]) -> AttrData {
        match self {
            AttrData::F32(v) => AttrData::F32(idx.iter().map(|&i| v.get(i).copied().unwrap_or(0.0)).collect()),
            AttrData::I32(v) => AttrData::I32(idx.iter().map(|&i| v.get(i).copied().unwrap_or(0)).collect()),
            AttrData::V2(v) => AttrData::V2(idx.iter().map(|&i| v.get(i).copied().unwrap_or([0.0; 2])).collect()),
            AttrData::V3(v) => AttrData::V3(idx.iter().map(|&i| v.get(i).copied().unwrap_or([0.0; 3])).collect()),
            AttrData::V4(v) => AttrData::V4(idx.iter().map(|&i| v.get(i).copied().unwrap_or([0.0; 4])).collect()),
            AttrData::Str { idx: si, table } => AttrData::Str {
                idx: idx.iter().map(|&i| si.get(i).copied().unwrap_or(-1)).collect(),
                table: table.clone(),
            },
            AttrData::FN { size, data } => {
                let sz = (*size as usize).max(1);
                let mut out = Vec::with_capacity(idx.len() * sz);
                for &i in idx {
                    match data.get(i.saturating_mul(sz)..i.saturating_add(1).saturating_mul(sz)) {
                        Some(x) => out.extend_from_slice(x),
                        None => out.extend(std::iter::repeat_n(0.0, sz)),
                    }
                }
                AttrData::FN { size: *size, data: out }
            }
            AttrData::IN { size, data } => {
                let sz = (*size as usize).max(1);
                let mut out = Vec::with_capacity(idx.len() * sz);
                for &i in idx {
                    match data.get(i.saturating_mul(sz)..i.saturating_add(1).saturating_mul(sz)) {
                        Some(x) => out.extend_from_slice(x),
                        None => out.extend(std::iter::repeat_n(0, sz)),
                    }
                }
                AttrData::IN { size: *size, data: out }
            }
        }
    }
}

impl Detail {
    fn prim_cv_points(&self, prim: usize) -> Option<&[PointId]> {
        let p = self.prims.get(prim)?;
        self.vertex_point.get(p.verts.start as usize..p.verts.end as usize)
    }

    /// `(point, rational basis weight)` for a sample; folds `Pw` in and normalises. `None` = degenerate (zero denominator).
    fn rationalise(&self, cvs: &[PointId], w: Vec<(usize, f32)>) -> Option<Vec<(PointId, f32)>> {
        let pw: Option<&[f32]> = match self.attr(AttrClass::Point, "Pw").map(|a| &a.data) {
            Some(AttrData::F32(v)) => Some(v),
            _ => None,
        };
        let mut out: Vec<(PointId, f32)> = Vec::with_capacity(w.len());
        let mut sum = 0.0f32;
        for (i, n) in w {
            let pt = *cvs.get(i)?;
            let h = pw.map_or(1.0, |v| v.get(pt as usize).copied().unwrap_or(1.0));
            let r = n * h;
            sum += r;
            out.push((pt, r));
        }
        if !sum.is_finite() || sum.abs() < 1e-12 {
            return None;
        }
        for o in &mut out {
            o.1 /= sum;
        }
        Some(out)
    }

    fn blend_position(&self, w: &[(PointId, f32)]) -> [f32; 3] {
        let p = self.p();
        let mut o = [0.0f32; 3];
        for &(i, wt) in w {
            if let Some(x) = p.get(i as usize) {
                for c in 0..3 {
                    o[c] += wt * x[c];
                }
            }
        }
        o
    }

    /// Point on curve prim `prim` at unit parameter `u in [0,1]` (clamped). `None` = not a valid NURBS/Bezier curve prim.
    pub fn evaluate(&self, prim: usize, u: f32) -> Option<[f32; 3]> {
        let q = self.prims.get(prim)?;
        let cvs = self.prim_cv_points(prim)?;
        let Plans::Curve(pl) = plans(&q.kind, cvs.len())? else { return None };
        let (t0, t1) = pl.domain();
        let u = if u.is_nan() { 0.0 } else { u.clamp(0.0, 1.0) };
        let w = self.rationalise(cvs, pl.weights(t0 + u * (t1 - t0)))?;
        Some(self.blend_position(&w))
    }

    /// Point on surface prim `prim` at unit `(u, v)`. `None` = not a valid NURBS/Bezier surface prim.
    pub fn evaluate_uv(&self, prim: usize, u: f32, v: f32) -> Option<[f32; 3]> {
        let q = self.prims.get(prim)?;
        let cvs = self.prim_cv_points(prim)?;
        let Plans::Surf { u: pu, v: pv, cols } = plans(&q.kind, cvs.len())? else { return None };
        let un = |x: f32| if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
        let ((a0, a1), (b0, b1)) = (pu.domain(), pv.domain());
        let w = self.rationalise(cvs, surf_weights(&pu, &pv, cols, a0 + un(u) * (a1 - a0), b0 + un(v) * (b1 - b0)))?;
        Some(self.blend_position(&w))
    }

    fn tess_prim(&self, prim: usize, lod_u: u32, lod_v: u32) -> Option<Tess> {
        let q = self.prims.get(prim)?;
        let cvs = self.prim_cv_points(prim)?;
        match plans(&q.kind, cvs.len())? {
            Plans::Curve(pl) => {
                let ts = pl.samples(lod_u)?;
                let mut samples = Vec::with_capacity(ts.len());
                for t in ts {
                    samples.push(self.rationalise(cvs, pl.weights(t))?);
                }
                let idx: Vec<u32> = (0..samples.len() as u32).collect();
                Some(Tess { samples, polys: vec![(idx, pl.closed)] })
            }
            Plans::Surf { u: pu, v: pv, cols } => {
                let (us, vs) = (pu.samples(lod_u)?, pv.samples(lod_v)?);
                let (nu, nv) = (us.len(), vs.len());
                if nu.checked_mul(nv)? > MAX_SAMPLES {
                    return None;
                }
                let mut samples = Vec::with_capacity(nu * nv);
                for &tv in &vs {
                    for &tu in &us {
                        samples.push(self.rationalise(cvs, surf_weights(&pu, &pv, cols, tu, tv))?);
                    }
                }
                let (qu, qv) = (
                    if pu.closed { nu } else { nu.saturating_sub(1) },
                    if pv.closed { nv } else { nv.saturating_sub(1) },
                );
                let mut polys = Vec::with_capacity(qu * qv);
                for j in 0..qv {
                    for i in 0..qu {
                        let (i1, j1) = ((i + 1) % nu, (j + 1) % nv);
                        let id = |a: usize, b: usize| (b * nu + a) as u32;
                        polys.push((vec![id(i, j), id(i1, j), id(i1, j1), id(i, j1)], true));
                    }
                }
                Some(Tess { samples, polys })
            }
        }
    }

    /// True if any prim is one of the four NURBS/Bezier kinds.
    pub fn has_spline_prims(&self) -> bool {
        self.prims.iter().any(|p| p.kind.is_spline())
    }

    /// Replace every NURBS/Bezier prim by polygons (curves -> one poly per curve, closed iff the curve is; surfaces -> quad grid,
    /// `lod_u`/`lod_v` samples per span, wrapped directions closed without a duplicate seam). New points get every point attribute
    /// blended with the same rational basis weights as `P` (ints/strings: heaviest CV; `Pw` = 1); prim attributes + prim
    /// groups are inherited by every produced poly; CV points used by nothing afterwards are removed. A spline prim that cannot
    /// be evaluated (bad order/knots/counts) degrades to an open poly through its CVs: never an error, never a panic.
    /// Non-spline prims keep their relative order; produced polys take the slot of the prim they replace.
    pub fn tessellate(&self, lod_u: u32, lod_v: u32) -> Detail {
        self.tessellate_mapped(lod_u, lod_v).0
    }

    /// [`Detail::tessellate`] + for every output prim the index of the source prim it came from.
    pub fn tessellate_mapped(&self, lod_u: u32, lod_v: u32) -> (Detail, Vec<usize>) {
        if !self.has_spline_prims() {
            return (self.clone(), (0..self.prims.len()).collect());
        }
        let np0 = self.point_count as usize;
        let mut samples: Vec<Vec<(PointId, f32)>> = Vec::new();
        let mut prims: Vec<Prim> = Vec::new();
        let mut vp: Vec<PointId> = Vec::new();
        let (mut vsrc, mut psrc): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
        let mut cv_flag = vec![false; np0];
        for (qi, q) in self.prims.iter().enumerate() {
            let Some(cvs) = self.prim_cv_points(qi) else { continue };
            if !q.kind.is_spline() {
                let start = vp.len() as u32;
                for (k, &pt) in cvs.iter().enumerate() {
                    vp.push(pt);
                    vsrc.push(q.verts.start as usize + k);
                }
                prims.push(Prim { kind: q.kind.clone(), verts: start..vp.len() as u32 });
                psrc.push(qi);
                continue;
            }
            for &pt in cvs {
                if let Some(f) = cv_flag.get_mut(pt as usize) {
                    *f = true;
                }
            }
            let (polys, base) = match self.tess_prim(qi, lod_u, lod_v) {
                Some(t) => {
                    let base = (np0 + samples.len()) as u32;
                    samples.extend(t.samples);
                    (t.polys, base)
                }
                // degrade: open poly through the CV points (local ordinals are real point ids -> base 0)
                None => (vec![(cvs.to_vec(), false)], 0),
            };
            for (ids, closed) in polys {
                let start = vp.len() as u32;
                for id in ids {
                    vp.push(id + base);
                    vsrc.push(usize::MAX);
                }
                prims.push(Prim { kind: PrimKind::Poly { closed }, verts: start..vp.len() as u32 });
                psrc.push(qi);
            }
        }
        let mut out = self.clone();
        // points: blend the new ones, extend groups
        let new_n = samples.len();
        for a in out.attrs.iter_mut().filter(|a| a.class == AttrClass::Point) {
            let blended = a.data.blend(&samples);
            a.data.extend_from(&blended);
            match &mut a.data {
                AttrData::F32(v) if a.name == "Pw" => v.iter_mut().skip(np0).for_each(|x| *x = 1.0),
                _ => {}
            }
        }
        out.point_count = (np0 + new_n) as u32;
        for g in out.groups.iter_mut() {
            match g.class {
                AttrClass::Point => g.mask.resize(np0 + new_n, false),
                AttrClass::Vertex => g.mask = vsrc.iter().map(|&i| g.mask.get(i).copied().unwrap_or(false)).collect(),
                AttrClass::Prim => g.mask = psrc.iter().map(|&i| g.mask.get(i).copied().unwrap_or(false)).collect(),
                AttrClass::Detail => {}
            }
        }
        for a in out.attrs.iter_mut() {
            match a.class {
                AttrClass::Vertex => a.data = a.data.gather(&vsrc),
                AttrClass::Prim => a.data = a.data.gather(&psrc),
                _ => {}
            }
        }
        out.vertex_point = vp;
        out.prims = prims;
        // drop orphaned CV points
        let mut used = vec![false; np0 + new_n];
        for &pt in &out.vertex_point {
            if let Some(u) = used.get_mut(pt as usize) {
                *u = true;
            }
        }
        let remove: Vec<bool> =
            (0..np0 + new_n).map(|i| cv_flag.get(i).copied().unwrap_or(false) && !used[i]).collect();
        if remove.iter().any(|&r| r) {
            out.remove_points(&remove);
        }
        (out, psrc)
    }

    /// Delete points flagged in `remove` (point attrs + point groups compacted, `vertex_point` renumbered). Vertices that still
    /// reference a removed point are re-pointed at point 0 (callers delete those prims first).
    pub fn remove_points(&mut self, remove: &[bool]) {
        let n = self.point_count as usize;
        let mut map = vec![u32::MAX; n];
        let mut keep: Vec<usize> = Vec::new();
        for (i, m) in map.iter_mut().enumerate() {
            if !remove.get(i).copied().unwrap_or(false) {
                *m = keep.len() as u32;
                keep.push(i);
            }
        }
        for a in self.attrs.iter_mut().filter(|a| a.class == AttrClass::Point) {
            a.data = a.data.gather(&keep);
        }
        for g in self.groups.iter_mut().filter(|g| g.class == AttrClass::Point) {
            g.mask = keep.iter().map(|&i| g.mask.get(i).copied().unwrap_or(false)).collect();
        }
        for v in self.vertex_point.iter_mut() {
            *v = match map.get(*v as usize) {
                Some(&m) if m != u32::MAX => m,
                _ => 0,
            };
        }
        self.point_count = keep.len() as u32;
    }
}

fn surf_weights(pu: &Plan, pv: &Plan, cols: usize, tu: f32, tv: f32) -> Vec<(usize, f32)> {
    let (wu, wv) = (pu.weights(tu), pv.weights(tv));
    let mut out = Vec::with_capacity(wu.len() * wv.len());
    for &(r, b) in &wv {
        for &(c, a) in &wu {
            out.push((r * cols + c, a * b));
        }
    }
    out
}
