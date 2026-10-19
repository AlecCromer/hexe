//! Null SOP: passthrough of input 0 (no-op node used as a named output marker). Houdini 20.5 parms (hython):
//! `copyinput` (default on; off = empty geometry, the input still cooks) and `cacheinput` (default off; caching only, no
//! effect on the result).
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("copyinput", ParamValue::B(true))
            .label("Copy Input (Note: Input will be still cooked if disabled)"),
        ParamSpec::new("cacheinput", ParamValue::B(false)).label("Cache Input"),
    ]
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
    /// Input 0 shared as-is (same `Arc`, no copy); empty `Detail` if the slice is empty or Copy Input is off.
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        if ctx.params.toggle("copyinput") == Some(false) {
            return Ok(Arc::default());
        }
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
    fn copy_input_off_gives_empty_geometry_like_houdini() {
        // hython 20.5: box -> null, copyinput 0 -> 0 points / 0 prims (cacheinput changes nothing)
        let mut d = Detail::default();
        d.add_points(8);
        let input = Arc::new(d);
        for cache in [false, true] {
            let p = ParamSet::new().with("copyinput", ParamValue::B(false)).with("cacheinput", ParamValue::B(cache));
            let out = new().cook(&ctx(&p), std::slice::from_ref(&input)).expect("cooks");
            assert_eq!(out.point_count, 0);
        }
        let p = ParamSet::new().with("cacheinput", ParamValue::B(true));
        assert!(Arc::ptr_eq(&new().cook(&ctx(&p), std::slice::from_ref(&input)).expect("cooks"), &input));
    }

    #[test]
    fn no_input_gives_empty_detail() {
        let p = ParamSet::new();
        let out = new().cook(&ctx(&p), &[]).expect("cooks");
        assert_eq!(*out, Detail::default());
    }
}
