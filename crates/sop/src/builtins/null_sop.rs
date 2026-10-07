//! Null SOP: passthrough of input 0 (no-op node used as a named output marker).
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec};
use std::sync::Arc;

pub fn spec() -> Vec<ParamSpec> {
    Vec::new()
}

pub fn new() -> Box<SopNode> {
    Box::new(Null { specs: spec() })
}

struct Null {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Null {
    fn type_name(&self) -> &'static str {
        "null"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    /// Input 0 shared as-is (same `Arc`, no copy); empty `Detail` if the slice is empty.
    fn cook(&self, _: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        Ok(inputs.first().cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hexe_graph::ParamSet;

    fn ctx(p: &ParamSet) -> CookCtx<'_> {
        CookCtx { time: 0.0, fps: 24.0, params: p, log: &|_| {} }
    }

    #[test]
    fn passes_input_through_same_arc() {
        let mut d = Detail::default();
        d.add_points(3);
        let input = Arc::new(d);
        let p = ParamSet::new();
        let out = new().cook(&ctx(&p), std::slice::from_ref(&input)).expect("cooks");
        assert!(Arc::ptr_eq(&out, &input));
    }

    #[test]
    fn no_input_gives_empty_detail() {
        let p = ParamSet::new();
        let out = new().cook(&ctx(&p), &[]).expect("cooks");
        assert_eq!(*out, Detail::default());
    }
}
