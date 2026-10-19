//! Control SOP (token `control`): the Null object's viewport indicator. A cross of three axis lines through the origin,
//! each running from `-scale` to `+scale` (six open 2-point polylines from the centre point outwards), point `Cd` green.
//!
//! Params: `scale` (F, "Uniform Scale", 1, range 0..10: how far each line reaches from the centre). Generator (no input).
//! Point order: centre, then +X, -X, +Y, -Y, +Z, -Z tips; prim i = centre -> tip i.
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

/// Indicator colour (point `Cd`).
pub const GREEN: [f32; 3] = [0.0, 0.8, 0.0];

pub fn spec() -> Vec<ParamSpec> {
    vec![ParamSpec::new("scale", ParamValue::F(1.0)).label("Uniform Scale").range(0.0, 10.0)]
}

pub fn new() -> Box<SopNode> {
    Box::new(Control { specs: spec() })
}

struct Control {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Control {
    fn type_name(&self) -> &'static str {
        "control"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, _: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let s = ctx.params.float("scale").ok_or_else(|| CookError::BadParam("control.scale: missing".into()))?;
        if !s.is_finite() {
            return Err(CookError::BadParam("control.scale: not finite".into()));
        }
        let mut g = Detail::default();
        g.add_points(7);
        let tips = [[s, 0.0, 0.0], [-s, 0.0, 0.0], [0.0, s, 0.0], [0.0, -s, 0.0], [0.0, 0.0, s], [0.0, 0.0, -s]];
        for (slot, q) in g.p_mut().iter_mut().skip(1).zip(tips) {
            *slot = q;
        }
        for tip in 1..7u32 {
            g.add_poly(&[0, tip], false);
        }
        g.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![GREEN; 7]),
            type_info: TypeInfo::Color,
        });
        Ok(Arc::new(g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hexe_graph::ParamSet;

    fn cook(scale: f32) -> Result<Arc<Detail>, CookError> {
        let p = ParamSet::new().with("scale", ParamValue::F(scale));
        new().cook(&CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} }, &[])
    }

    #[test]
    fn six_axis_lines_reach_scale_from_the_centre() {
        let d = cook(2.5).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (7, 6));
        assert_eq!(d.p()[0], [0.0, 0.0, 0.0]);
        assert_eq!(d.p()[1], [2.5, 0.0, 0.0]);
        assert_eq!(d.p()[6], [0.0, 0.0, -2.5]);
        assert!(d.attr(AttrClass::Point, "Cd").is_some());
    }

    #[test]
    fn hostile_scale_is_an_error() {
        assert!(cook(f32::NAN).is_err());
        assert!(cook(f32::INFINITY).is_err());
        assert_eq!(cook(0.0).expect("zero is fine").point_count, 7);
    }
}
