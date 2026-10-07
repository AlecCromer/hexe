//! Line SOP (token `line`): ONE open polyline of `points` evenly spaced points. Generator (no inputs).
//!
//! # Point order (generator defines it; UNVERIFIED vs Houdini, hython blocked)
//! Point `i` (i = 0..points) = `origin + normalize(dir) * dist * i / (points - 1)`; point ids ascend from `origin`, the
//! single prim is `Poly{closed:false}` over ids `0..points` in order. `dir` is normalised (its length is ignored),
//! `dist` is the total length (negative flips the direction). Spacing is `dist / (points - 1)`, computed as
//! `dist * i / (points - 1)` in f64 so `points 5, dist 4` gives spacing exactly 1.
//! `points < 2`, zero / non-finite `dir`, or `points` above the allocation cap → `BadParam` / `Failed`.
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

/// Allocation cap (AGENTS.md §4).
const MAX_POINTS: u32 = 1 << 20;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("origin", ParamValue::V3([0.0, 0.0, 0.0])).label("Origin").range(-1.0, 1.0),
        ParamSpec::new("dir", ParamValue::V3([0.0, 1.0, 0.0])).label("Direction").range(-1.0, 1.0),
        ParamSpec::new("dist", ParamValue::F(1.0)).label("Length").range(0.0, 10.0),
        ParamSpec::new("points", ParamValue::I(2)).label("Number of Points").range(2.0, 50.0),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Line { specs: spec() })
}

struct Line {
    specs: Vec<ParamSpec>,
}

fn bad(name: &str, what: &str) -> CookError {
    CookError::BadParam(format!("line.{name}: {what}"))
}

impl Node<Detail> for Line {
    fn type_name(&self) -> &'static str {
        "line"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, _: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let origin = p.vec3("origin").ok_or_else(|| bad("origin", "missing or not a vector"))?;
        let dir = p.vec3("dir").ok_or_else(|| bad("dir", "missing or not a vector"))?;
        let dist = p.float("dist").ok_or_else(|| bad("dist", "missing or not a float"))?;
        let n = p.int("points").ok_or_else(|| bad("points", "missing or not an integer"))?;
        if !origin.iter().chain(dir.iter()).all(|c| c.is_finite()) || !dist.is_finite() {
            return Err(bad("origin/dir/dist", "non-finite value"));
        }
        let n = u32::try_from(n).ok().filter(|&n| n >= 2).ok_or_else(|| bad("points", "need at least 2 points"))?;
        if n > MAX_POINTS {
            return Err(CookError::Failed(format!("line: {n} points exceeds the {MAX_POINTS} point cap")));
        }
        let d = dir.map(f64::from);
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len <= 1e-12 {
            return Err(bad("dir", "zero-length direction"));
        }
        let unit = d.map(|c| c / len);
        let (dist, o, last) = (f64::from(dist), origin.map(f64::from), f64::from(n - 1));
        let mut g = Detail::default();
        let ids = g.add_points(n);
        let pos = g.p_mut();
        pos.clear();
        for i in 0..n {
            let s = dist * f64::from(i) / last;
            pos.push([0, 1, 2].map(|k| (o[k] + unit[k] * s) as f32));
        }
        let line: Vec<u32> = ids.collect();
        g.add_poly(&line, false);
        Ok(Arc::new(g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;

    fn cook(params: &str) -> Result<Arc<Detail>, String> {
        let s = format!(r#"{{"nodes":[{{"name":"l","type":"line","params":{params}}}],"display":"l"}}"#);
        let (mut g, id) = load_scene(&s).map_err(|e| e.to_string())?;
        g.cook(id, 0.0).map_err(|e| e.to_string())
    }

    #[test]
    fn defaults_are_two_points_p1_is_0_1_0_one_open_prim() {
        let d = cook("{}").expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (2, 1, 2));
        assert_eq!(d.p(), &[[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        assert!(matches!(d.prims[0].kind, hexe_geo::PrimKind::Poly { closed: false }));
        assert_eq!(d.vertex_point, vec![0, 1]);
    }

    #[test]
    fn five_points_dist_4_spacing_exactly_1() {
        let d = cook(r#"{"points":5,"dist":4}"#).expect("cooks");
        assert_eq!(d.point_count, 5);
        for (i, p) in d.p().iter().enumerate() {
            assert_eq!(*p, [0.0, i as f32, 0.0]);
        }
    }

    #[test]
    fn origin_and_unnormalised_dir_and_negative_dist() {
        let d = cook(r#"{"origin":[1,2,3],"dir":[0,0,5],"dist":-2,"points":3}"#).expect("cooks");
        assert_eq!(d.p(), &[[1.0, 2.0, 3.0], [1.0, 2.0, 2.0], [1.0, 2.0, 1.0]]);
    }

    #[test]
    fn hostile_params_are_errors() {
        assert!(cook(r#"{"points":1}"#).is_err());
        assert!(cook(r#"{"points":-4}"#).is_err());
        assert!(cook(r#"{"dir":[0,0,0]}"#).is_err());
        assert!(cook(r#"{"points":2000000}"#).is_err());
        assert!(cook(r#"{"dist":1e39}"#).is_err());
    }
}
