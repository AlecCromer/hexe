//! Curve SOP (token `curve`): ONE polyline through the typed `coords`. Generator (no inputs).
//!
//! `coords` (S): whitespace-separated `x,y,z` triples, e.g. `"0,0,0 1,0,0 1,1,0"`; a component list is exactly 3 finite
//! floats. Empty string → empty geometry. `close` (B, default off) closes the polyline.
//!
//! # Point order (generator defines it; UNVERIFIED vs Houdini, hython blocked)
//! Point `i` = the `i`-th triple, ids ascending in text order; one `Poly{closed:close}` prim over ids `0..n` in order
//! (no repeated closing point). Bad triple / non-finite / more than `MAX_POINTS` points → `BadParam` / `Failed`.
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
        let close =
            p.toggle("close").ok_or_else(|| CookError::BadParam("curve.close: missing or not a toggle".into()))?;
        let curve_type = p.menu("type").unwrap_or(0);
        let divisions = p.int("divisions").unwrap_or(10).max(1).min(1000) as usize;

        let control_pts = parse_coords("curve.coords", coords)?;
        let mut g = Detail::default();
        if control_pts.is_empty() {
            return Ok(Arc::new(g));
        }

        let pts = match curve_type {
            0 => control_pts,                                      // poly: use control points directly
            1 => subdivide_bezier(&control_pts, divisions, close), // bezier
            2 => control_pts,                                      // nurbs: TODO, use control points for now
            _ => return Err(CookError::BadParam("curve.type: invalid type".into())),
        };

        if pts.is_empty() {
            return Ok(Arc::new(g));
        }

        let n = u32::try_from(pts.len()).map_err(|_| CookError::Failed("curve: too many points".into()))?;
        let ids = g.add_points(n);
        *g.p_mut() = pts;
        let line: Vec<u32> = ids.collect();
        g.add_poly(&line, close);
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
