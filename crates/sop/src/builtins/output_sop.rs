//! Output SOP (token `output`): passthrough of input 0, marking the node as a network output.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `outputidx` I 0 (0..=9) is the output slot Houdini uses when the network is a subnet / HDA; here it is stored and
//! validated (negative or > 9 -> `BadParam`) but has no effect on the cook: the result is input 0 shared as-is (same `Arc`).
//! Empty `Detail` when input 0 is unconnected.
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const MAX_IDX: i32 = 9;

pub fn spec() -> Vec<ParamSpec> {
    vec![ParamSpec::new("outputidx", ParamValue::I(0)).label("Output Index").range(0.0, MAX_IDX as f32)]
}
pub fn new() -> Box<SopNode> {
    Box::new(Output { specs: spec() })
}
struct Output {
    specs: Vec<ParamSpec>,
}
impl Node<Detail> for Output {
    fn type_name(&self) -> &'static str {
        "output"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        match ctx.params.int("outputidx") {
            Some(i) if (0..=MAX_IDX).contains(&i) => Ok(inputs.first().cloned().unwrap_or_default()),
            Some(i) => Err(CookError::BadParam(format!("`outputidx` {i} out of range 0..={MAX_IDX}"))),
            None => Err(CookError::BadParam("`outputidx`: missing or not an int".into())),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook_arc, cube, defaults};
    use super::*;
    #[test]
    fn passes_input_through_same_arc() {
        let a = Arc::new(cube());
        let out = cook_arc(&*new(), &a, &defaults(&*new())).expect("cooks");
        assert!(Arc::ptr_eq(&out, &a));
        assert_eq!(out.point_count, 8);
    }
    #[test]
    fn empty_when_unconnected_and_idx_validated() {
        let p = defaults(&*new());
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        assert_eq!(*new().cook(&ctx, &[]).expect("cooks"), Detail::default());
        for bad in [-1, 10] {
            let q = p.clone().with("outputidx", ParamValue::I(bad));
            let ctx = CookCtx { time: 0.0, fps: 24.0, params: &q, log: &|_| {} };
            assert!(matches!(new().cook(&ctx, &[]), Err(CookError::BadParam(_))));
        }
    }
}
