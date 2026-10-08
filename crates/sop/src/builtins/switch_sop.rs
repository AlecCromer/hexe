//! Switch SOP (token `switch`): passes ONE of its inputs through, chosen by the `input` parameter.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! 4 fixed input ports (`input0..input3`, graph arity is fixed; Houdini allows many). `input` I 0 selects the port; the
//! index is CLAMPED into `0..=3` (negative -> 0, past the last -> 3), as Houdini clamps. The chosen input's `Arc` is shared
//! as-is (no copy). An unconnected chosen port is an empty `Detail`. Only the selected input matters for the result.
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 4] = [
    InputSpec { name: "input0" },
    InputSpec { name: "input1" },
    InputSpec { name: "input2" },
    InputSpec { name: "input3" },
];

pub fn spec() -> Vec<ParamSpec> {
    vec![ParamSpec::new("input", ParamValue::I(0)).label("Select Input").range(0.0, 3.0)]
}
pub fn new() -> Box<SopNode> {
    Box::new(Switch { specs: spec() })
}
struct Switch {
    specs: Vec<ParamSpec>,
}
impl Node<Detail> for Switch {
    fn type_name(&self) -> &'static str {
        "switch"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let i = ctx.params.int("input").ok_or_else(|| CookError::BadParam("`input`: missing or not an int".into()))?;
        let i = usize::try_from(i.clamp(0, INPUTS.len() as i32 - 1)).unwrap_or(0);
        Ok(inputs.get(i).cloned().unwrap_or_default())
    }
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::defaults;
    use super::*;
    use hexe_graph::ParamSet;
    fn pts(n: u32) -> Arc<Detail> {
        let mut d = Detail::default();
        d.add_points(n);
        Arc::new(d)
    }
    fn run(i: i32, ins: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p: ParamSet = defaults(&*new()).with("input", ParamValue::I(i));
        new().cook(&CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} }, ins)
    }
    #[test]
    fn selects_each_input_by_index_sharing_the_arc() {
        let ins = [pts(1), pts(2), pts(3), pts(4)];
        for (i, a) in ins.iter().enumerate() {
            let out = run(i as i32, &ins).expect("cooks");
            assert!(Arc::ptr_eq(&out, a), "input {i}");
            assert_eq!(out.point_count, i as u32 + 1);
        }
    }
    #[test]
    fn index_clamps_and_unconnected_is_empty() {
        let ins = [pts(1), pts(2), pts(3), pts(4)];
        assert_eq!(run(-5, &ins).expect("cooks").point_count, 1);
        assert_eq!(run(99, &ins).expect("cooks").point_count, 4);
        assert_eq!(run(2, &[pts(7)]).expect("cooks").point_count, 0, "missing port = empty");
        assert_eq!(run(0, &[]).expect("cooks").point_count, 0);
    }
    #[test]
    fn bare_params_are_bad_param() {
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &ParamSet::new(), log: &|_| {} };
        assert!(matches!(new().cook(&ctx, &[]), Err(CookError::BadParam(_))));
    }
    #[test]
    fn example_scene_switches_between_box_and_grid() {
        let src = include_str!("../../../../examples/switch.json");
        let (mut g, id) = crate::load_scene(src).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 8, "switch input 0 = box (8 pts)");
        let flipped = src.replace("\"input\": 0", "\"input\": 1");
        assert_ne!(flipped, src);
        let (mut g, id) = crate::load_scene(&flipped).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 9, "input 1 = 3x3 grid (9 pts)");
    }
}
