//! Group Delete SOP (token `groupdelete`): removes groups by name, geometry untouched.
//!
//! `group`: whitespace-separated names, `*`/`?` glob (`*` = every group). A name matches groups of EVERY class (point, vertex,
//! prim). Empty `group` = delete nothing (UNVERIFIED: Houdini's empty field may mean all). Nothing matched -> the input
//! `Arc` is shared. Absent input -> empty `Detail`.
use super::group_sop::glob_match;
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

pub fn spec() -> Vec<ParamSpec> {
    vec![ParamSpec::new("group", ParamValue::S(String::new())).label("Group")]
}
pub fn new() -> Box<SopNode> {
    Box::new(GroupDelete { specs: spec() })
}
struct GroupDelete {
    specs: Vec<ParamSpec>,
}
impl Node<Detail> for GroupDelete {
    fn type_name(&self) -> &'static str {
        "groupdelete"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let pat = ctx
            .params
            .string("group")
            .ok_or_else(|| CookError::BadParam("groupdelete: `group` missing or not a string".into()))?;
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let toks: Vec<&str> = pat.split_whitespace().collect();
        if !d.groups.iter().any(|g| toks.iter().any(|t| glob_match(t, &g.name))) {
            return Ok(Arc::clone(d));
        }
        let mut out = (**d).clone();
        out.groups.retain(|g| !toks.iter().any(|t| glob_match(t, &g.name)));
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use hexe_geo::AttrClass;
    use hexe_graph::{CookError, ParamValue};
    use std::sync::Arc;

    fn two_groups() -> Arc<hexe_geo::Detail> {
        let b = cube();
        let a = run("group", &[("groupname", s("top")), ("pattern", s("0"))], Some(&b)).expect("cooks");
        let a =
            run("group", &[("groupname", s("pts")), ("grouptype", ParamValue::Menu(0)), ("pattern", s("0"))], Some(&a))
                .expect("cooks");
        run("group", &[("groupname", s("keep")), ("pattern", s("1"))], Some(&a)).expect("cooks")
    }
    #[test]
    fn deletes_named_and_glob_groups_geometry_untouched() {
        let d = two_groups();
        assert_eq!(d.groups.len(), 3);
        let one = run("groupdelete", &[("group", s("top"))], Some(&d)).expect("cooks");
        let names: Vec<&str> = one.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["pts", "keep"]);
        let star = run("groupdelete", &[("group", s("*"))], Some(&d)).expect("cooks");
        assert!(star.groups.is_empty());
        let globbed = run("groupdelete", &[("group", s("t* p?s"))], Some(&d)).expect("cooks");
        assert_eq!(globbed.groups.len(), 1);
        for o in [&one, &star, &globbed] {
            assert_eq!(
                (o.point_count, &o.vertex_point, &o.prims, &o.attrs),
                (d.point_count, &d.vertex_point, &d.prims, &d.attrs)
            );
        }
        assert!(star.group(AttrClass::Point, "pts").is_none());
    }
    #[test]
    fn nothing_matched_shares_the_arc() {
        let d = two_groups();
        for pat in ["", "nope", "zz*"] {
            let o = run("groupdelete", &[("group", s(pat))], Some(&d)).expect("cooks");
            assert!(Arc::ptr_eq(&o, &d), "{pat:?}");
        }
        assert_eq!(run("groupdelete", &[], None).expect("cooks").point_count, 0);
        let e: Result<_, CookError> = run("groupdelete", &[("group", ParamValue::I(1))], Some(&d));
        assert!(matches!(e, Err(CookError::BadParam(_))));
    }
}
