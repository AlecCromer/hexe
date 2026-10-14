//! Group Invert SOP (token `groupinvert`, Houdini 20.5 `groupinvert`): complement of a group.
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-groupinvert*`)
//! `group` names the group (names + `*`/`?` globs, space separated). `newname` empty = the group is inverted IN PLACE (same name);
//! `newname` set = the source stays and a NEW group holds its complement (name sanitised like `groupcreate`). `grouptype`: auto (any
//! class; first matching group in prim, point, vertex order) | prims | points | vertices; edges -> `Unsupported` (no edge groups).
//! Empty `group` or a group that does not exist = passthrough (Houdini only warns). With several globbed groups each is inverted
//! in place; `newname` takes the FIRST match only.
//!
//! UNVERIFIED: globs matching several groups, same name in several classes under auto.
use super::group_sop::{glob_match, set_group};
use super::groupcreate_sop::sanitize;
use crate::SopNode;
use hexe_geo::{AttrClass, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "groupinvert";
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("grouptype", ParamValue::Menu(0)).label("Group Type").menu(&[
            ("auto", "Auto"),
            ("prims", "Primitives"),
            ("points", "Points"),
            ("edges", "Edges"),
            ("vertices", "Vertices"),
        ]),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("newname", ParamValue::S(String::new())).label("New Group Name"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(GroupInvert { specs: spec() })
}
struct GroupInvert {
    specs: Vec<ParamSpec>,
}
fn bad(m: &str) -> CookError {
    CookError::BadParam(format!("{OP}: {m}"))
}
impl Node<Detail> for GroupInvert {
    fn type_name(&self) -> &'static str {
        OP
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let pat = p.string("group").ok_or_else(|| bad("`group` missing or not a string"))?;
        let newname = sanitize(p.string("newname").ok_or_else(|| bad("`newname` missing or not a string"))?);
        let classes: &[AttrClass] = match p.menu("grouptype").ok_or_else(|| bad("`grouptype` missing or not a menu"))? {
            0 => &[AttrClass::Prim, AttrClass::Point, AttrClass::Vertex],
            1 => &[AttrClass::Prim],
            2 => &[AttrClass::Point],
            3 => return Err(CookError::Unsupported(format!("{OP}: edge groups are not modelled"))),
            4 => &[AttrClass::Vertex],
            m => return Err(bad(&format!("`grouptype` menu index {m} out of range"))),
        };
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let toks: Vec<&str> = pat.split_whitespace().collect();
        // matched (class, name) pairs, first class in `classes` order that has any hit (auto) else the single class
        let mut hits: Vec<(AttrClass, String)> = Vec::new();
        for &c in classes {
            hits.extend(
                d.groups
                    .iter()
                    .filter(|g| g.class == c && toks.iter().any(|t| glob_match(t, &g.name)))
                    .map(|g| (c, g.name.clone())),
            );
            if !hits.is_empty() {
                break;
            }
        }
        if hits.is_empty() {
            return Ok(Arc::clone(d));
        }
        let mut out = (**d).clone();
        for (i, (c, n)) in hits.iter().enumerate() {
            let Some(g) = d.group(*c, n) else { continue };
            let inv: Vec<bool> = g.mask.iter().map(|m| !m).collect();
            if newname.is_empty() {
                set_group(&mut out, *c, n, inv);
            } else if i == 0 {
                set_group(&mut out, *c, &newname, inv);
            }
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::{cube, mask, run, s};
    use super::*;
    fn with_group() -> Arc<Detail> {
        run("groupcreate", &[("groupname", s("a")), ("basegroup", s("0-2"))], Some(&cube())).expect("groupcreate")
    }
    #[test]
    fn in_place_inversion_flips_the_mask() {
        let d = with_group();
        assert_eq!(mask(&d, AttrClass::Prim, "a"), [0, 1, 2]);
        let o = run("groupinvert", &[("group", s("a")), ("grouptype", ParamValue::Menu(1))], Some(&d)).expect("cooks");
        assert_eq!(mask(&o, AttrClass::Prim, "a"), [3, 4, 5]);
        assert_eq!(o.groups.len(), 1);
        let twice = run("groupinvert", &[("group", s("a"))], Some(&o)).expect("cooks");
        assert_eq!(mask(&twice, AttrClass::Prim, "a"), [0, 1, 2], "auto finds the prim group; invert twice = identity");
    }
    #[test]
    fn newname_keeps_the_source() {
        let d = with_group();
        let o = run("groupinvert", &[("group", s("a")), ("newname", s("b"))], Some(&d)).expect("cooks");
        assert_eq!(mask(&o, AttrClass::Prim, "a"), [0, 1, 2]);
        assert_eq!(mask(&o, AttrClass::Prim, "b"), [3, 4, 5]);
    }
    #[test]
    fn wrong_class_empty_and_missing_are_passthrough() {
        let d = with_group();
        for ov in [
            vec![("group", s(""))],
            vec![("group", s("zz"))],
            vec![("group", s("a")), ("grouptype", ParamValue::Menu(2))],
        ] {
            let o = run("groupinvert", &ov, Some(&d)).expect("cooks");
            assert!(Arc::ptr_eq(&o, &d) || *o == *d);
        }
    }
    #[test]
    fn hostile_params() {
        let d = with_group();
        assert!(matches!(
            run("groupinvert", &[("group", s("a")), ("grouptype", ParamValue::Menu(3))], Some(&d)),
            Err(CookError::Unsupported(_))
        ));
        assert!(run("groupinvert", &[("grouptype", ParamValue::Menu(9))], Some(&d)).is_err());
        assert_eq!(run("groupinvert", &[], None).expect("empty").point_count, 0);
    }
    /// Real Houdini 20.5 twin: box prims 0-2 in group `a` -> groupinvert -> blast keeps `a` (prims 3-5).
    #[test]
    fn groupinvert_matches_hython() {
        for (scene, geo) in [
            (
                include_str!("../../../../fixtures/sop-groupinvert/scene.json"),
                &include_bytes!("../../../../fixtures/sop-groupinvert/houdini.geo")[..],
            ),
            (
                include_str!("../../../../fixtures/sop-groupinvert-new/scene.json"),
                &include_bytes!("../../../../fixtures/sop-groupinvert-new/houdini.geo")[..],
            ),
        ] {
            super::super::meshutil::testkit::houdini_twin(scene, geo).expect("hexe == Houdini");
        }
    }
}
