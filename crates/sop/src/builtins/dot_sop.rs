//! Network dot (Houdini `__dotN`, `.networkdotinit`): a wire waypoint. One input, its output shared as-is (same `Arc`), so
//! a dot can feed any number of inputs and chain into other dots. `pinned` (off) is Houdini's pin flag, kept so a scene
//! round-trips it; it changes nothing at cook time. The editor draws dots as balls, never gives them the display flag, and
//! deleting one rewires its source straight through to everything it fed (Houdini 20.5 `destroy()` behaviour).
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

/// Registry token of the dot type.
pub const TYPE_NAME: &str = "dot";
/// Name prefix of new dots (`__dot1`, `__dot2`, ...: Houdini's own naming, lowest free number).
pub const NAME_BASE: &str = "__dot";

pub fn spec() -> Vec<ParamSpec> {
    vec![ParamSpec::new("pinned", ParamValue::B(false)).label("Pinned")]
}

pub fn new() -> Box<SopNode> {
    Box::new(Dot { specs: spec() })
}

struct Dot {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Dot {
    fn type_name(&self) -> &'static str {
        TYPE_NAME
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    /// Input 0 shared as-is; empty `Detail` when unwired.
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
    fn dot_passes_input_through_same_arc_pinned_or_not() {
        let mut d = Detail::default();
        d.add_points(4);
        let input = Arc::new(d);
        for pinned in [false, true] {
            let p = ParamSet::new().with("pinned", ParamValue::B(pinned));
            let out = new().cook(&ctx(&p), std::slice::from_ref(&input)).expect("cooks");
            assert!(Arc::ptr_eq(&out, &input));
        }
        assert_eq!(*new().cook(&ctx(&ParamSet::new()), &[]).expect("cooks"), Detail::default());
    }

    #[test]
    fn dot_fans_out_and_chains_in_a_graph() {
        // box -> dot1 -> {xform, dot2 -> merge.0}, xform -> merge.1: both merge inputs see the box
        let json = r#"{"nodes":[{"name":"box1","type":"box"},{"name":"__dot1","type":"dot"},{"name":"__dot2","type":"dot","params":{"pinned":true}},
            {"name":"xform1","type":"xform","params":{"t":[0,2,0]}},{"name":"merge1","type":"merge"}],
            "edges":[["box1",0,"__dot1",0],["__dot1",0,"xform1",0],["__dot1",0,"__dot2",0],["__dot2",0,"merge1",0],["xform1",0,"merge1",1]],
            "display":"merge1"}"#;
        let (mut g, m) = crate::load_scene(json).expect("loads");
        let out = g.cook(m, 0.0).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (16, 12));
    }
}
