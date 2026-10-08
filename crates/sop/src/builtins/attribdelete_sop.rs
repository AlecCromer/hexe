//! Attribute Delete SOP (token `attribdelete`): removes attributes by pattern, per class.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `ptdel` / `vtxdel` / `primdel` / `dtldel`: attribute pattern per class (grammar: `attribs.rs` header: space-separated,
//! `*`/`?` glob, `^` excludes). Empty pattern = class untouched. `negate` (off) deletes the NOT-selected attributes instead
//! (classes with an empty pattern stay untouched). Point `P` is never deleted. Groups and topology untouched. Nothing
//! deleted -> the input `Arc` is shared. Absent input -> empty `Detail`.
use super::attribs::{selected, str_param};
use super::group_sop::bad;
use crate::SopNode;
use hexe_geo::{AttrClass, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const WHO: &str = "attribdelete";
const CLASSES: [(&str, AttrClass); 4] = [
    ("ptdel", AttrClass::Point),
    ("vtxdel", AttrClass::Vertex),
    ("primdel", AttrClass::Prim),
    ("dtldel", AttrClass::Detail),
];
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("ptdel", ParamValue::S(String::new())).label("Point Attributes"),
        ParamSpec::new("vtxdel", ParamValue::S(String::new())).label("Vertex Attributes"),
        ParamSpec::new("primdel", ParamValue::S(String::new())).label("Primitive Attributes"),
        ParamSpec::new("dtldel", ParamValue::S(String::new())).label("Detail Attributes"),
        ParamSpec::new("negate", ParamValue::B(false)).label("Delete Non-Selected"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(AttribDelete { specs: spec() })
}
struct AttribDelete {
    specs: Vec<ParamSpec>,
}
impl Node<Detail> for AttribDelete {
    fn type_name(&self) -> &'static str {
        "attribdelete"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let negate = p.toggle("negate").ok_or_else(|| bad(WHO, "`negate` missing or not a toggle"))?;
        let mut pats = Vec::new();
        for (name, class) in CLASSES {
            pats.push((class, str_param(p, name, WHO)?));
        }
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let doomed = |class: AttrClass, name: &str| {
            let Some((_, pat)) = pats.iter().find(|(c, _)| *c == class) else { return false };
            if pat.split_whitespace().next().is_none() || (class == AttrClass::Point && name == "P") {
                return false;
            }
            selected(pat, name) != negate
        };
        if !d.attrs.iter().any(|a| doomed(a.class, &a.name)) {
            return Ok(Arc::clone(d));
        }
        let mut out = (**d).clone();
        out.attrs.retain(|a| !doomed(a.class, &a.name));
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
    use hexe_graph::{CookError, ParamValue};
    use std::sync::Arc;
    fn attr(name: &str, class: AttrClass, n: usize) -> Attribute {
        Attribute { name: name.into(), class, data: AttrData::F32(vec![1.0; n]), type_info: TypeInfo::None }
    }
    /// grid3 (9 pts, 4 prims, 16 verts) + point N Cd v, vertex uv, prim id, detail info.
    fn rich() -> Arc<Detail> {
        let mut d = (*grid3()).clone();
        for n in ["N", "Cd", "v"] {
            d.add_attr(attr(n, AttrClass::Point, 9));
        }
        d.add_attr(attr("uv", AttrClass::Vertex, 16));
        d.add_attr(attr("id", AttrClass::Prim, 4));
        d.add_attr(attr("info", AttrClass::Detail, 1));
        Arc::new(d)
    }
    fn names(d: &Detail, c: AttrClass) -> Vec<&str> {
        d.attrs.iter().filter(|a| a.class == c).map(|a| a.name.as_str()).collect()
    }
    fn del(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        run("attribdelete", ov, Some(d))
    }
    #[test]
    fn params_and_defaults() {
        let n = super::new();
        let got: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(got, ["ptdel", "vtxdel", "primdel", "dtldel", "negate"]);
    }
    #[test]
    fn star_deletes_all_but_p_and_names_are_exact() {
        let d = rich();
        let o = del(&d, &[("ptdel", s("*"))]).expect("cooks");
        assert_eq!(names(&o, AttrClass::Point), ["P"], "P never deleted");
        assert_eq!(names(&o, AttrClass::Vertex), ["uv"], "other classes untouched");
        let o = del(&d, &[("ptdel", s("N Cd"))]).expect("cooks");
        assert_eq!(names(&o, AttrClass::Point), ["P", "v"]);
        let o =
            del(&d, &[("ptdel", s("*")), ("vtxdel", s("*")), ("primdel", s("*")), ("dtldel", s("*"))]).expect("cooks");
        assert_eq!(o.attrs.len(), 1);
        assert_eq!((o.point_count, &o.vertex_point, &o.prims), (d.point_count, &d.vertex_point, &d.prims));
    }
    #[test]
    fn exclusion_and_negate() {
        let d = rich();
        let o = del(&d, &[("ptdel", s("^N"))]).expect("cooks");
        assert_eq!(names(&o, AttrClass::Point), ["P", "N"], "^N = all but N (P protected)");
        let o = del(&d, &[("ptdel", s("* ^Cd"))]).expect("cooks");
        assert_eq!(names(&o, AttrClass::Point), ["P", "Cd"]);
        let o = del(&d, &[("ptdel", s("Cd")), ("negate", ParamValue::B(true))]).expect("cooks");
        assert_eq!(names(&o, AttrClass::Point), ["P", "Cd"], "negate keeps the selection");
        assert_eq!(names(&o, AttrClass::Vertex), ["uv"], "empty-pattern class untouched under negate");
    }
    #[test]
    fn nothing_matched_shares_the_arc_and_hostile() {
        let d = rich();
        for ov in [vec![], vec![("ptdel", s("nope zz*"))], vec![("ptdel", s("P"))], vec![("dtldel", s("^info"))]] {
            assert!(Arc::ptr_eq(&del(&d, &ov).expect("cooks"), &d));
        }
        assert_eq!(run("attribdelete", &[], None).expect("cooks").point_count, 0);
        assert!(matches!(del(&d, &[("ptdel", ParamValue::I(1))]), Err(CookError::BadParam(_))));
        assert!(matches!(del(&d, &[("negate", s("x"))]), Err(CookError::BadParam(_))));
    }
}
