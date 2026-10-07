//! Peak SOP (token `peak`): push every point along its normal.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `P' = P + N * distance`. `N` = the existing point `N` attribute (V3, one per point) used AS IS (not renormalised);
//! when absent / wrong-typed / wrong-length, `N` is the uniform-averaged Newell normal: per point, the normalised
//! plain sum of the unit Newell normals of the closed polys that use it (box corner -> normalize(+-1,+-1,+-1)). Points
//! without a usable face get a zero normal and stay. Only `P` changes (an absent `N` is NOT added).
//!
//! Params: `distance` F 0 (non-finite = `BadParam`; 0 = input `Arc` shared; f32 overflow = `Failed`).
use super::meshutil::{V3, add, bad, float, point_normals, positions, scale, to_f32};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "peak";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![ParamSpec::new("distance", ParamValue::F(0.0)).label("Distance").range(-1.0, 1.0)]
}
pub fn new() -> Box<SopNode> {
    Box::new(Peak { specs: spec() })
}
struct Peak {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Peak {
    fn type_name(&self) -> &'static str {
        "peak"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let distance = float(ctx.params, "distance").map_err(|_| bad("`distance`: missing or not a finite float"))?;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if distance == 0.0 || src.point_count == 0 {
            return Ok(Arc::clone(src));
        }
        let pos = positions(src);
        let normals: Vec<V3> = match src.attr(AttrClass::Point, "N").map(|a| &a.data) {
            Some(AttrData::V3(n)) if n.len() == pos.len() => {
                n.iter().map(|v| [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]).collect()
            }
            _ => point_normals(src, &pos),
        };
        let moved: Vec<V3> = pos.iter().zip(&normals).map(|(&p, &n)| add(p, scale(n, distance))).collect();
        let mut out = (**src).clone();
        *out.p_mut() = to_f32(OP, &moved)?;
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, grid3};
    use super::*;
    use hexe_geo::{Attribute, TypeInfo};
    use hexe_graph::ParamSet;

    fn with(d: f32) -> ParamSet {
        defaults(&*new()).with("distance", ParamValue::F(d))
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "peak");
        assert_eq!(n.params().len(), 1);
        assert_eq!((n.params()[0].name.as_str(), n.params()[0].default.clone()), ("distance", ParamValue::F(0.0)));
    }
    /// Corner (h,h,h) touches faces +x,+y,+z with unit normals (1,0,0),(0,1,0),(0,0,1): sum (1,1,1),
    /// normalised (1,1,1)/sqrt(3). distance 1 -> corner + that = h + 1/sqrt(3) per axis (sign per corner).
    #[test]
    fn box_distance_one_moves_corners_along_diagonal() {
        let src = cube();
        let out = cook(&*new(), src.clone(), &with(1.0)).expect("cooks");
        let d = (1.0f64 / 3.0f64.sqrt()) as f32;
        for (a, b) in src.p().iter().zip(out.p()) {
            for j in 0..3 {
                let want = a[j] + d * a[j].signum();
                assert!((b[j] - want).abs() < 1e-6, "{a:?} -> {b:?}");
            }
        }
        assert_eq!(out.prims, src.prims);
        assert!(out.attr(AttrClass::Point, "N").is_none(), "N must not be added");
    }
    #[test]
    fn zero_distance_and_empty_share_the_arc() {
        let a = Arc::new(cube());
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &with(0.0)).expect("cooks"), &a));
        assert_eq!(cook(&*new(), Detail::default(), &with(1.0)).expect("cooks").point_count, 0);
    }
    #[test]
    fn existing_n_is_used_as_is_and_negative_distance_reverses() {
        let mut d = grid3();
        d.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[0.0, 0.0, 2.0]; 9]),
            type_info: TypeInfo::Normal,
        });
        let up = cook(&*new(), d.clone(), &with(0.5)).expect("cooks");
        assert!(up.p().iter().all(|p| p[2] == 1.0), "N (0,0,2) * 0.5 = +1 as is");
        let down = cook(&*new(), d, &with(-0.5)).expect("cooks");
        assert!(down.p().iter().all(|p| p[2] == -1.0));
        // computed from the flat +Z grid: unit normals
        let flat = cook(&*new(), grid3(), &with(3.0)).expect("cooks");
        assert!(flat.p().iter().all(|p| p[2] == 3.0));
    }
    #[test]
    fn hostile_inputs_do_not_panic() {
        for v in [f32::NAN, f32::INFINITY] {
            assert!(matches!(cook(&*new(), cube(), &with(v)), Err(CookError::BadParam(_))));
        }
        let r = cook(&*new(), cube(), &with(f32::MAX));
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
        // wrong-length N falls back to computed normals; degenerate faces give zero normals
        let mut d = cube();
        d.add_poly(&[0, 0, 0], true);
        d.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![1.0; 8]),
            type_info: TypeInfo::None,
        });
        assert_eq!(cook(&*new(), d, &with(1.0)).expect("cooks").point_count, 8);
        let mut lone = Detail::default();
        lone.add_points(2);
        *lone.p_mut() = vec![[1.0; 3], [2.0; 3]];
        assert_eq!(cook(&*new(), lone, &with(1.0)).expect("cooks").p(), &[[1.0; 3], [2.0; 3]]);
    }
    #[test]
    fn scene_loader_and_registry_slot() {
        let ok = r#"{"nodes":[{"name":"b","type":"box"},{"name":"p","type":"peak","params":{"distance":0.25}}],"edges":[["b",0,"p",0]],"display":"p"}"#;
        let (mut g, id) = crate::load_scene(ok).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 8);
        assert_eq!(crate::registry().create("peak").map(|n| n.type_name()), Some("peak"));
    }
}
