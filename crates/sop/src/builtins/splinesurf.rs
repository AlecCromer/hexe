//! Shared NURBS / Bezier helpers of `revolve`, `skin`, `convert`, `clip`, `boolean`, `polyfill`: spline-curve extraction, surface
//! assembly, and the tessellate-then-operate wrapper for ops that only understand polygons. Not a SOP itself.
use super::meshutil::{MAX_ELEMS, V3, add, cross, dot, failed, scale, sub};
use super::surfutil::Curve;
use hexe_geo::{AttrClass, AttrData, Attribute, Basis, Detail, Prim, PrimKind, TypeInfo};
use hexe_graph::CookError;
use std::sync::Arc;

/// Samples per span used when an op tessellates spline prims on its own (clip / boolean / polyfill; `convert` has `lodu/lodv`).
pub(super) const TESS_LOD: u32 = 8;

/// Inputs for polygon-only ops: any input holding NURBS/Bezier prims is replaced by its tessellation (`Detail::tessellate`), the
/// rest is shared. Never fails: curves/surfaces that cannot be evaluated degrade to open polys through their CVs.
pub(super) fn tess_inputs(inputs: &[Arc<Detail>]) -> Vec<Arc<Detail>> {
    inputs
        .iter()
        .map(|d| if d.has_spline_prims() { Arc::new(d.tessellate(TESS_LOD, TESS_LOD)) } else { Arc::clone(d) })
        .collect()
}

/// A spline curve ready to be lofted / revolved: ordered point ids, closedness, basis, kind.
#[derive(Clone, Debug)]
pub(super) struct SplineCurve {
    pub pts: Vec<u32>,
    pub closed: bool,
    pub basis: Basis,
    pub bezier: bool,
    /// Index of the source prim.
    pub prim: usize,
}

/// Every evaluable `BezierCurve` / `NurbsCurve` prim, in prim order.
pub(super) fn spline_curves(d: &Detail) -> Vec<SplineCurve> {
    let mut out = Vec::new();
    for (prim, pr) in d.prims.iter().enumerate() {
        let (basis, closed, bezier) = match &pr.kind {
            PrimKind::BezierCurve { basis, closed } => (basis.clone(), *closed, true),
            PrimKind::NurbsCurve { basis, closed } => (basis.clone(), *closed, false),
            _ => continue,
        };
        let (Ok(lo), Ok(hi)) = (usize::try_from(pr.verts.start), usize::try_from(pr.verts.end)) else { continue };
        let Some(vs) = d.vertex_point.get(lo..hi) else { continue };
        if vs.len() < 2 || vs.iter().any(|&p| p >= d.point_count) || d.evaluate(prim, 0.0).is_none() {
            continue;
        }
        out.push(SplineCurve { pts: vs.to_vec(), closed, basis, bezier, prim });
    }
    out
}

/// A polygon / polyline profile as the exactly-equal linear NURBS curve (order 2).
pub(super) fn linear_curve(c: &Curve) -> SplineCurve {
    SplineCurve { pts: c.pts.clone(), closed: c.closed, basis: Basis::new(2), bezier: false, prim: c.prim }
}

/// Rational weight of point `p` (`Pw`, default 1).
pub(super) fn point_weight(d: &Detail, p: u32) -> f32 {
    match d.attr(AttrClass::Point, "Pw").map(|a| &a.data) {
        Some(AttrData::F32(v)) => v.get(p as usize).copied().unwrap_or(1.0),
        _ => 1.0,
    }
}

/// Exact (rational) surface of revolution of a spline `profile` about the axis through `origin` along the unit `axis`.
/// u = profile (its own basis, wrapped when closed), v = angle: standard rational quadratic arcs, clamped knots `0 0 0 1 1 .. n n n`,
/// `n` = 4 for a full turn else `ceil(|span| / 90deg)` arcs of equal angle (mid CV at radius / cos(half arc), weight cos(half arc)).
/// Surface CV `(profile i, angle k)` = vertex `k * cols + i`; weight = profile weight x arc weight. Winding matches the polygon path
/// of `revolve` (outward for an upward profile at +radius).
pub(super) fn revolve_surface(
    src: &Detail,
    c: &SplineCurve,
    origin: V3,
    axis: V3,
    a0_deg: f64,
    span_deg: f64,
    full: bool,
) -> Result<Detail, CookError> {
    const OP: &str = "revolve";
    let arcs = if full { 4 } else { ((span_deg.abs() / 90.0 - 1e-9).ceil() as usize).clamp(1, 4) };
    let sweep = if full { 360.0f64.copysign(span_deg) } else { span_deg };
    let dth = sweep.to_radians() / arcs as f64;
    let ncv_c = 2 * arcs + 1;
    let cols = c.pts.len();
    if (cols as u64).saturating_mul(ncv_c as u64) > MAX_ELEMS {
        return Err(failed(OP, "output exceeds the point cap"));
    }
    let p = src.p();
    let half = (dth / 2.0).cos();
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(cols * ncv_c);
    let mut pw: Vec<f32> = Vec::with_capacity(cols * ncv_c);
    for k in 0..ncv_c {
        let (arc, odd) = (k / 2, k % 2 == 1);
        let th = a0_deg.to_radians() + dth * (arc as f64 + if odd { 0.5 } else { 0.0 });
        let (s, co) = th.sin_cos();
        let (rs, wc) = if odd { (1.0 / half, half) } else { (1.0, 1.0) };
        for &pt in &c.pts {
            let q = p.get(pt as usize).map_or([0.0; 3], |v| v.map(f64::from));
            let rel = sub(q, origin);
            let h = dot(rel, axis);
            let radial = sub(rel, scale(axis, h));
            let rot = add(add(scale(radial, co * rs), scale(cross(axis, radial), s * rs)), scale(axis, h));
            let w = add(origin, rot);
            let r = [w[0] as f32, w[1] as f32, w[2] as f32];
            if !r.iter().all(|x| x.is_finite()) {
                return Err(failed(OP, "non-finite or f32-overflowing position"));
            }
            pos.push(r);
            pw.push(point_weight(src, pt) * wc as f32);
        }
    }
    let mut kn: Vec<f32> = vec![0.0; 3];
    for a in 1..arcs {
        kn.extend([a as f32; 2]);
    }
    kn.extend([arcs as f32; 3]);
    let (vbasis, kind) =
        if c.bezier { (Basis::new(3), None) } else { (Basis { order: 3, knots: kn, end: true }, Some(())) };
    let (cols32, rows32) = (cols as u32, ncv_c as u32);
    let kind = if kind.is_none() {
        PrimKind::BezierSurf {
            cols: cols32,
            rows: rows32,
            ubasis: c.basis.clone(),
            vbasis,
            uwrap: c.closed,
            vwrap: false,
        }
    } else {
        PrimKind::NurbsSurf {
            cols: cols32,
            rows: rows32,
            ubasis: c.basis.clone(),
            vbasis,
            uwrap: c.closed,
            vwrap: false,
        }
    };
    Ok(spline_detail(pos, pw, kind))
}

/// One-prim detail: `pos` (CV points, one vertex each in order) + `kind`; `pw` becomes the point attr `Pw` when any weight != 1.
pub(super) fn spline_detail(pos: Vec<[f32; 3]>, pw: Vec<f32>, kind: PrimKind) -> Detail {
    let n = pos.len() as u32;
    let mut d = Detail::default();
    d.add_points(n);
    d.p_mut().copy_from_slice(&pos);
    d.vertex_point = (0..n).collect();
    d.prims.push(Prim { kind, verts: 0..n });
    if pw.iter().any(|&w| w != 1.0) {
        d.add_attr(Attribute {
            name: "Pw".into(),
            class: AttrClass::Point,
            data: AttrData::F32(pw),
            type_info: TypeInfo::None,
        });
    }
    d
}

#[cfg(test)]
pub(super) mod testkit {
    use hexe_geo::{Basis, Detail, Prim, PrimKind};
    /// One spline-curve prim over `pts` (one vertex per CV).
    pub fn spline_curve(pts: &[[f32; 3]], kind: PrimKind) -> Detail {
        let mut d = Detail::default();
        d.add_points(pts.len() as u32);
        *d.p_mut() = pts.to_vec();
        d.vertex_point = (0..pts.len() as u32).collect();
        d.prims.push(Prim { kind, verts: 0..pts.len() as u32 });
        d
    }
    pub fn nurbs(order: u32) -> PrimKind {
        PrimKind::NurbsCurve { basis: Basis::new(order), closed: false }
    }
    /// Exact unit circle in XY: 9-CV rational quadratic NURBS (`Pw` = 1 / sqrt2), open, clamped knots, arc length 2 pi.
    pub fn nurbs_circle() -> Detail {
        let pts = [
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [-1.0, -1.0, 0.0],
            [0.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let knots = vec![0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0];
        let mut d =
            spline_curve(&pts, PrimKind::NurbsCurve { basis: Basis { order: 3, knots, end: true }, closed: false });
        let s = std::f32::consts::FRAC_1_SQRT_2;
        d.add_attr(hexe_geo::Attribute {
            name: "Pw".into(),
            class: hexe_geo::AttrClass::Point,
            data: hexe_geo::AttrData::F32(vec![1.0, s, 1.0, s, 1.0, s, 1.0, s, 1.0]),
            type_info: hexe_geo::TypeInfo::None,
        });
        d
    }
    /// Total length of every polyline / open poly edge chain.
    pub fn length(d: &Detail) -> f64 {
        let p = d.p();
        let mut l = 0.0;
        for q in &d.prims {
            let v: Vec<[f32; 3]> =
                (q.verts.start..q.verts.end).map(|i| p[d.vertex_point[i as usize] as usize]).collect();
            for w in v.windows(2) {
                l += (0..3).map(|k| f64::from(w[0][k] - w[1][k]).powi(2)).sum::<f64>().sqrt();
            }
        }
        l
    }
    /// Total area of every closed poly (fan of cross products).
    pub fn area(d: &Detail) -> f64 {
        let p = d.p();
        let mut a = 0.0;
        for q in &d.prims {
            let v: Vec<[f64; 3]> =
                (q.verts.start..q.verts.end).map(|i| p[d.vertex_point[i as usize] as usize].map(f64::from)).collect();
            for k in 1..v.len().saturating_sub(1) {
                let e1 = [v[k][0] - v[0][0], v[k][1] - v[0][1], v[k][2] - v[0][2]];
                let e2 = [v[k + 1][0] - v[0][0], v[k + 1][1] - v[0][1], v[k + 1][2] - v[0][2]];
                let c = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                a += 0.5 * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
            }
        }
        a
    }
}
