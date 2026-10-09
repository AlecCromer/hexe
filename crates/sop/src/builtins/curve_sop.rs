//! Curve SOP (token `curve`): ONE polyline through the typed `coords`. Generator (no inputs).
//!
//! `coords` (S): whitespace-separated `x,y,z` triples, e.g. `"0,0,0 1,0,0 1,1,0"`; a component list is exactly 3 finite
//! floats. Empty string → empty geometry. `close` (B, default off) closes the polyline.
//!
//! # Point order (generator defines it; UNVERIFIED vs Houdini, hython blocked)
//! Point `i` = the `i`-th triple, ids ascending in text order; one `Poly{closed:close}` prim over ids `0..n` in order
//! (no repeated closing point). Bad triple / non-finite / more than `MAX_POINTS` points → `BadParam` / `Failed`.
//! # Curve types (QA, tessellated to polylines: no NURBS / Bezier prim kind in `hexe-geo`; UNVERIFIED vs Houdini)
//! `type` 0 poly = the CVs; 1 bezier = piecewise degree `order-1` (shared end CVs) when `(n-1) % (order-1) == 0`, else one whole-curve
//! Bezier; 2 nurbs = B-spline (unit weights), clamped uniform knots (or `knots` S, n + order values) open / periodic when `close`.
//! `divisions` = samples per span. `method` 1 breakpoints = the curve interpolates every point (uniform Catmull-Rom, `divisions` per
//! span). `coords` may hold several curves separated by `;` (one prim each). Not done: interactive drawing (UI-only).
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

/// Allocation cap on parsed points (AGENTS.md §4).
pub(super) const MAX_POINTS: usize = 1 << 20;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("coords", ParamValue::S(String::new())).label("Coordinates"),
        ParamSpec::new("type", ParamValue::Menu(0)).label("Type").menu(&[
            ("poly", "Polygon"),
            ("bezier", "Bezier"),
            ("nurbs", "NURBS"),
        ]),
        ParamSpec::new("order", ParamValue::I(4)).label("Order").range(2.0, 11.0),
        ParamSpec::new("close", ParamValue::B(false)).label("Closed"),
        ParamSpec::new("divisions", ParamValue::I(10)).label("Divisions").range(1.0, 100.0),
        ParamSpec::new("method", ParamValue::Menu(0))
            .label("Method")
            .menu(&[("cvs", "Control Vertices"), ("breakpoints", "Breakpoints")]),
        ParamSpec::new("knots", ParamValue::S(String::new())).label("Knots"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Curve { specs: spec() })
}

struct Curve {
    specs: Vec<ParamSpec>,
}

/// De Casteljau algorithm: evaluate Bezier curve at parameter t ∈ [0,1].
fn de_casteljau(control_pts: &[[f32; 3]], t: f32) -> [f32; 3] {
    if control_pts.is_empty() {
        return [0.0; 3];
    }
    if control_pts.len() == 1 {
        return control_pts[0];
    }
    let mut pts = control_pts.to_vec();
    let n = pts.len();
    for r in 1..n {
        for i in 0..(n - r) {
            #[allow(clippy::needless_range_loop)]
            for k in 0..3 {
                pts[i][k] = (1.0 - t) * pts[i][k] + t * pts[i + 1][k];
            }
        }
    }
    pts[0]
}

/// Subdivide a Bezier curve into line segments.
fn subdivide_bezier(control_pts: &[[f32; 3]], divisions: usize, closed: bool) -> Vec<[f32; 3]> {
    if control_pts.is_empty() || divisions == 0 {
        return vec![];
    }
    let mut result = Vec::new();
    let steps = if closed { divisions } else { divisions + 1 };
    for i in 0..steps {
        let t = i as f32 / divisions as f32;
        result.push(de_casteljau(control_pts, t));
    }
    result
}

/// Piecewise Bezier of degree `order - 1` (segments share end CVs) when `(n-1) % (order-1) == 0` and `n > order - 1`;
/// otherwise ONE whole-curve Bezier over all CVs. `divisions` samples per segment, end CV included.
fn bezier_open(ctrl: &[[f32; 3]], order: usize, divisions: usize) -> Vec<[f32; 3]> {
    let n = ctrl.len();
    let deg = order.max(2) - 1;
    if n < 2 || divisions == 0 {
        return ctrl.to_vec();
    }
    if n > deg && (n - 1) % deg == 0 {
        let mut out = Vec::new();
        for s in 0..(n - 1) / deg {
            let seg = &ctrl[s * deg..=(s + 1) * deg];
            out.extend((0..divisions).map(|i| de_casteljau(seg, i as f32 / divisions as f32)));
        }
        out.push(ctrl[n - 1]);
        out
    } else {
        subdivide_bezier(ctrl, divisions, false)
    }
}
/// de Boor evaluation of the degree `p` B-spline (NURBS with unit weights) with control points `c` and `knots` at `t`.
fn de_boor(c: &[[f64; 3]], p: usize, knots: &[f64], t: f64) -> [f64; 3] {
    let n = c.len();
    let mut s = p;
    while s + 1 < n && t >= knots[s + 1] {
        s += 1;
    }
    let mut d: Vec<[f64; 3]> = (0..=p).map(|j| c[s - p + j]).collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = s - p + j;
            let den = knots[i + p - r + 1] - knots[i];
            let a = if den.abs() < 1e-300 { 0.0 } else { (t - knots[i]) / den };
            for k in 0..3 {
                d[j][k] = (1.0 - a) * d[j - 1][k] + a * d[j][k];
            }
        }
    }
    d[p]
}
/// NURBS (unit weights = B-spline) tessellated to `divisions` samples per knot span. Open: clamped uniform knots (or the
/// user `knots`, length n + order), ends hit the first / last CV, `spans * divisions + 1` points. Closed: periodic uniform
/// knots over the CVs wrapped by `order - 1`, `n * divisions` points (no duplicate end point).
fn nurbs_points(
    ctrl: &[[f32; 3]],
    order: usize,
    divisions: usize,
    closed: bool,
    user_knots: &[f64],
) -> Result<Vec<[f32; 3]>, CookError> {
    let n = ctrl.len();
    let k = order.min(n);
    if k < 2 || divisions == 0 {
        return Ok(ctrl.to_vec());
    }
    let p = k - 1;
    let w = |v: &[f32; 3]| [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])];
    if closed {
        let mut c: Vec<[f64; 3]> = ctrl.iter().map(w).collect();
        for i in 0..p {
            c.push(w(&ctrl[i % n]));
        }
        let knots: Vec<f64> = (0..c.len() + k).map(|i| i as f64).collect();
        return Ok((0..n * divisions)
            .map(|i| {
                let q = de_boor(&c, p, &knots, p as f64 + i as f64 / divisions as f64);
                [q[0] as f32, q[1] as f32, q[2] as f32]
            })
            .collect());
    }
    let c: Vec<[f64; 3]> = ctrl.iter().map(w).collect();
    let knots: Vec<f64> = if user_knots.is_empty() {
        let spans = n - p;
        (0..n + k).map(|i| if i < k { 0.0 } else if i >= n { spans as f64 } else { (i - p) as f64 }).collect()
    } else {
        if user_knots.len() != n + k {
            return Err(CookError::BadParam(format!(
                "curve.knots: need n + order = {} values, got {}",
                n + k,
                user_knots.len()
            )));
        }
        if user_knots.windows(2).any(|w| w[1] < w[0]) || user_knots[p] >= user_knots[n] {
            return Err(CookError::BadParam("curve.knots: must be non-decreasing with a non-empty domain".into()));
        }
        user_knots.to_vec()
    };
    let (t0, t1) = (knots[p], knots[n]);
    let total = (n - p) * divisions;
    Ok((0..=total)
        .map(|i| {
            let t = t0 + (t1 - t0) * i as f64 / total as f64;
            let q = de_boor(&c, p, &knots, t);
            [q[0] as f32, q[1] as f32, q[2] as f32]
        })
        .collect())
}
/// Uniform Catmull-Rom through every point (`method = breakpoints`): `divisions` samples per span.
fn through_points(pts: &[[f32; 3]], divisions: usize, closed: bool) -> Vec<[f32; 3]> {
    let n = pts.len();
    if n < 2 || divisions == 0 {
        return pts.to_vec();
    }
    let at = |i: isize| -> [f64; 3] {
        let j = if closed { i.rem_euclid(n as isize) as usize } else { i.clamp(0, n as isize - 1) as usize };
        [f64::from(pts[j][0]), f64::from(pts[j][1]), f64::from(pts[j][2])]
    };
    let spans = if closed { n } else { n - 1 };
    let mut out = Vec::new();
    for s in 0..spans {
        let (p0, p1, p2, p3) = (at(s as isize - 1), at(s as isize), at(s as isize + 1), at(s as isize + 2));
        for i in 0..divisions {
            let t = i as f64 / divisions as f64;
            let (t2, t3) = (t * t, t * t * t);
            out.push(std::array::from_fn(|k| {
                (0.5 * (2.0 * p1[k]
                    + (-p0[k] + p2[k]) * t
                    + (2.0 * p0[k] - 5.0 * p1[k] + 4.0 * p2[k] - p3[k]) * t2
                    + (-p0[k] + 3.0 * p1[k] - 3.0 * p2[k] + p3[k]) * t3)) as f32
            }));
        }
    }
    if !closed {
        out.push(pts[n - 1]);
    }
    out
}
/// Parse `"x,y,z x,y,z ..."` (shared with the Add SOP). `who` prefixes errors (`curve.coords`).
pub(super) fn parse_coords(who: &str, s: &str) -> Result<Vec<[f32; 3]>, CookError> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        if out.len() >= MAX_POINTS {
            return Err(CookError::Failed(format!("{who}: more than {MAX_POINTS} points")));
        }
        let bad = |what: &str| CookError::BadParam(format!("{who}: `{tok}`: {what}"));
        let mut it = tok.split(',');
        let mut v = [0.0f32; 3];
        for c in v.iter_mut() {
            let part = it.next().ok_or_else(|| bad("expected x,y,z"))?;
            *c = part.parse::<f32>().map_err(|_| bad("not a number"))?;
            if !c.is_finite() {
                return Err(bad("non-finite component"));
            }
        }
        if it.next().is_some() {
            return Err(bad("expected exactly x,y,z"));
        }
        out.push(v);
    }
    Ok(out)
}

impl Node<Detail> for Curve {
    fn type_name(&self) -> &'static str {
        "curve"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, _: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let coords =
            p.string("coords").ok_or_else(|| CookError::BadParam("curve.coords: missing or not a string".into()))?;
        let method = p.menu("method").unwrap_or(0);
        if method > 1 {
            return Err(CookError::BadParam("curve.method: invalid method".into()));
        }
        let order = p.int("order").unwrap_or(4).clamp(2, 11) as usize;
        let user_knots: Vec<f64> = p
            .string("knots")
            .unwrap_or("")
            .split_whitespace()
            .map(|t| {
                t.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| CookError::BadParam(format!("curve.knots: `{t}` is not a finite number")))
            })
            .collect::<Result<_, _>>()?;
        let close =
            p.toggle("close").ok_or_else(|| CookError::BadParam("curve.close: missing or not a toggle".into()))?;
        let curve_type = p.menu("type").unwrap_or(0);
        let divisions = p.int("divisions").unwrap_or(10).clamp(1, 1000) as usize;

        if curve_type > 2 {
            return Err(CookError::BadParam("curve.type: invalid type".into()));
        }
        let mut g = Detail::default();
        let mut total = 0usize;
        // `;` separates several curves (one prim each, points concatenated)
        for part in coords.split(';') {
            let control_pts = parse_coords("curve.coords", part)?;
            if control_pts.is_empty() {
                continue;
            }
            let pts = match (curve_type, method) {
                (0, _) => control_pts,
                (1, 0) if close => {
                    let mut c = control_pts.clone();
                    c.push(control_pts[0]);
                    let mut v = bezier_open(&c, order, divisions);
                    v.pop();
                    v
                }
                (1, 0) => bezier_open(&control_pts, order, divisions),
                (2, 0) => nurbs_points(&control_pts, order, divisions, close, &user_knots)?,
                _ => through_points(&control_pts, divisions, close),
            };
            total += pts.len();
            if total > MAX_POINTS {
                return Err(CookError::Failed(format!("curve: more than {MAX_POINTS} points")));
            }
            if pts.is_empty() {
                continue;
            }
            let n = u32::try_from(pts.len()).map_err(|_| CookError::Failed("curve: too many points".into()))?;
            let start = g.point_count as usize;
            let ids = g.add_points(n);
            g.p_mut()[start..start + pts.len()].copy_from_slice(&pts);
            let line: Vec<u32> = ids.collect();
            g.add_poly(&line, close);
        }
        Ok(Arc::new(g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::PrimKind;

    fn cook(params: &str) -> Result<Arc<Detail>, String> {
        let s = format!(r#"{{"nodes":[{{"name":"c","type":"curve","params":{params}}}],"display":"c"}}"#);
        let (mut g, id) = load_scene(&s).map_err(|e| e.to_string())?;
        g.cook(id, 0.0).map_err(|e| e.to_string())
    }

    #[test]
    fn three_coords_is_three_points_one_open_prim_close_closes() {
        let d = cook(r#"{"coords":"0,0,0 1,0,0 1,1,0"}"#).expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (3, 1, 3));
        assert_eq!(d.p(), &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
        assert!(matches!(d.prims[0].kind, PrimKind::Poly { closed: false }));
        let c = cook(r#"{"coords":"0,0,0 1,0,0 1,1,0","close":true}"#).expect("cooks");
        assert_eq!((c.point_count, c.vertex_count()), (3, 3));
        assert!(matches!(c.prims[0].kind, PrimKind::Poly { closed: true }));
    }

    #[test]
    fn empty_and_whitespace_variants() {
        assert_eq!(cook("{}").expect("cooks").point_count, 0);
        let d = cook(r#"{"coords":"  1,2,3\n\t-4.5,5e-1,6  "}"#).expect("cooks");
        assert_eq!(d.p(), &[[1.0, 2.0, 3.0], [-4.5, 0.5, 6.0]]);
    }

    #[test]
    fn hostile_coords_are_errors() {
        for bad in ["1,2", "1,2,3,4", "a,b,c", "1,2,nan", "1,2,inf", "1;2;3", "1,,3", "1,2,1e99"] {
            assert!(cook(&format!(r#"{{"coords":"{bad}"}}"#)).is_err(), "{bad}");
        }
        let many = "0,0,0 ".repeat(MAX_POINTS + 1);
        assert!(matches!(parse_coords("t", &many), Err(CookError::Failed(_))));
        assert_eq!(parse_coords("t", &many[..many.len() - 6]).map(|v| v.len()).ok(), Some(MAX_POINTS));
    }

    fn pts(d: &Detail) -> Vec<[f32; 3]> {
        d.p().to_vec()
    }
    fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-4)
    }
    /// NURBS order 4 on 4 CVs IS the cubic Bezier: sample i == de Casteljau at i/10; ends hit the end CVs; 11 points.
    #[test]
    fn nurbs_order_n_equals_bezier_and_clamped_ends() {
        let c = r#""coords":"0,0,0 1,2,0 3,2,0 4,0,0","divisions":10"#;
        let b = cook(&format!(r#"{{{c},"type":1,"order":4}}"#)).expect("bezier");
        let n = cook(&format!(r#"{{{c},"type":2,"order":4}}"#)).expect("nurbs");
        assert_eq!((b.point_count, n.point_count), (11, 11));
        for (x, y) in pts(&b).into_iter().zip(pts(&n)) {
            assert!(close3(x, y), "{x:?} vs {y:?}");
        }
        assert!(close3(pts(&n)[0], [0.0; 3]) && close3(pts(&n)[10], [4.0, 0.0, 0.0]));
    }
    /// NURBS order 2 = the control polyline; 4 CVs -> 3 spans x 5 divisions + 1 = 16 points, all on the polyline y = 0 segments here.
    #[test]
    fn nurbs_order_2_traces_the_control_polygon_and_counts_are_spans_times_divisions() {
        let n = cook(r#"{"coords":"0,0,0 1,0,0 2,0,0 3,3,0","type":2,"order":2,"divisions":5}"#).expect("nurbs");
        assert_eq!(n.point_count, 16);
        let p = pts(&n);
        for (i, q) in p.iter().enumerate() {
            let t = i as f32 / 5.0; // CV parameter
            let want = if t <= 2.0 { [t, 0.0, 0.0] } else { [2.0 + (t - 2.0), 3.0 * (t - 2.0), 0.0] };
            assert!(close3(*q, want), "i {i}: {q:?} vs {want:?}");
        }
        // cubic over 6 CVs: 3 spans -> 3*4+1 = 13
        let c = cook(r#"{"coords":"0,0,0 1,1,0 2,0,0 3,1,0 4,0,0 5,1,0","type":2,"order":4,"divisions":4}"#).expect("nurbs");
        assert_eq!(c.point_count, 13);
    }
    /// Closed periodic NURBS: n*divisions points, no duplicate end point, stays in the CV hull, one closed prim.
    #[test]
    fn nurbs_closed_is_periodic_n_times_divisions_inside_the_hull() {
        let d = cook(r#"{"coords":"0,0,0 4,0,0 4,4,0 0,4,0","type":2,"order":3,"divisions":8,"close":true}"#).expect("nurbs");
        assert_eq!((d.point_count, d.prim_count()), (32, 1));
        assert!(matches!(d.prims[0].kind, PrimKind::Poly { closed: true }));
        assert!(d.p().iter().all(|p| (0.0..=4.0).contains(&p[0]) && (0.0..=4.0).contains(&p[1]) && p[2] == 0.0));
        // order 3 periodic: first sample = midpoint of CV0, CV1: (2, 0)
        assert!(close3(d.p()[0], [2.0, 0.0, 0.0]), "{:?}", d.p()[0]);
    }
    /// Piecewise Bezier: 7 CVs order 4 = 2 cubic segments -> 2*5+1 = 11 points, the joint CV (index 3) is hit exactly at sample 5.
    #[test]
    fn bezier_piecewise_hits_the_shared_joint_cv() {
        let d = cook(r#"{"coords":"0,0,0 1,1,0 2,1,0 3,0,0 4,-1,0 5,-1,0 6,0,0","type":1,"order":4,"divisions":5}"#).expect("bezier");
        assert_eq!(d.point_count, 11);
        assert!(close3(d.p()[5], [3.0, 0.0, 0.0]) && close3(d.p()[10], [6.0, 0.0, 0.0]));
    }
    /// breakpoints: the curve passes through every given point (sample k*divisions), (n-1)*div+1 points open, n*div closed.
    #[test]
    fn breakpoints_method_interpolates_all_points() {
        let cs = [[0.0f32, 0.0, 0.0], [1.0, 2.0, 0.0], [3.0, 1.0, 0.0], [4.0, 3.0, 0.0]];
        let d = cook(r#"{"coords":"0,0,0 1,2,0 3,1,0 4,3,0","type":2,"method":1,"divisions":6}"#).expect("bp");
        assert_eq!(d.point_count, 19);
        for (k, c) in cs.iter().enumerate() {
            assert!(close3(d.p()[k * 6], *c), "cv {k}");
        }
        let cl = cook(r#"{"coords":"0,0,0 1,2,0 3,1,0 4,3,0","type":1,"method":1,"divisions":6,"close":true}"#).expect("bp");
        assert_eq!(cl.point_count, 24);
    }
    /// Knots: valid user knots == the default clamped ones; wrong length / decreasing is an error.
    #[test]
    fn user_knots_are_validated_and_reproduce_clamped_default() {
        let base = r#""coords":"0,0,0 1,2,0 3,2,0 4,0,0","type":2,"order":3,"divisions":4"#;
        let a = cook(&format!("{{{base}}}")).expect("default");
        let b = cook(&format!(r#"{{{base},"knots":"0 0 0 1 2 2 2"}}"#)).expect("knots");
        for (x, y) in pts(&a).into_iter().zip(pts(&b)) {
            assert!(close3(x, y));
        }
        assert!(cook(&format!(r#"{{{base},"knots":"0 0 1"}}"#)).is_err());
        assert!(cook(&format!(r#"{{{base},"knots":"0 0 0 2 1 2 2"}}"#)).is_err());
        assert!(cook(&format!(r#"{{{base},"knots":"0 0 0 x 2 2 2"}}"#)).is_err());
    }
    /// `;` separates curves: two curves = two prims, 3 + 2 points, second prim ids 3..5.
    #[test]
    fn semicolon_makes_multiple_curves() {
        let d = cook(r#"{"coords":"0,0,0 1,0,0 1,1,0 ; 5,5,5 6,6,6 ;"}"#).expect("multi");
        assert_eq!((d.point_count, d.prim_count()), (5, 2));
        assert_eq!(d.prims[1].verts, 3..5);
        assert_eq!(d.vertex_point[3..5], [3, 4]);
        assert!(close3(d.p()[3], [5.0, 5.0, 5.0]));
        assert!(cook(r#"{"coords":"0,0,0 ; 1,2"}"#).is_err());
    }
    #[test]
    fn bezier_endpoints_hit_control_points() {
        // Bezier curve with 4 control points
        let d = cook(r#"{"coords":"0,0,0 1,2,0 3,2,0 4,0,0","type":1,"divisions":10}"#).expect("bezier");
        assert!(d.point_count > 4, "bezier should subdivide");
        // First and last points should match first and last control points
        let pts = d.p();
        assert_eq!(pts[0], [0.0, 0.0, 0.0], "bezier start should be first control point");
        assert!((pts[pts.len() - 1][0] - 4.0).abs() < 0.01, "bezier end should be near last control point");
        assert!((pts[pts.len() - 1][1] - 0.0).abs() < 0.01, "bezier end y");
    }
}
