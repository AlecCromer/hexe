//! Group Promote SOP (token `grouppromote`): converts a group between point / prim / vertex classes.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `group` (required, one name) of class `fromtype` -> new group `newname` (empty = same name; classes never collide) of class
//! `totype`. Unknown source -> `Failed`. `removesource` (off) deletes the source group afterwards.
//! Everything routes through vertices: point -> its vertices, prim -> its vertices, vertex -> itself; then
//! * to vertex: as is.
//! * to prim: `any` = some vertex selected, `all` = every vertex selected (a prim with no vertices never qualifies).
//! * to point: `any` = some vertex on the point selected, `all` = EVERY vertex on the point selected (point with no
//!   vertices never qualifies). So prim -> point `any` = all points of the group's prims; `all` = only points whose every
//!   prim is in the group.
//! `fromtype == totype` copies the mask. Menus `fromtype`/`totype`: point | prim | vertex; `method`: any | all.
use super::group_sop::{GT_POINT, GT_PRIM, GT_VERTEX, bad, failed, prim_verts, set_group};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const ANY: u32 = 0;
const ALL: u32 = 1;
const WHO: &str = "grouppromote";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Source Group"),
        ParamSpec::new("fromtype", ParamValue::Menu(GT_PRIM)).label("From Type"),
        ParamSpec::new("totype", ParamValue::Menu(GT_POINT)).label("To Type"),
        ParamSpec::new("method", ParamValue::Menu(ANY)).label("Method"),
        ParamSpec::new("newname", ParamValue::S(String::new())).label("New Group Name"),
        ParamSpec::new("removesource", ParamValue::B(false)).label("Delete Original"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(GroupPromote { specs: spec() })
}
struct GroupPromote {
    specs: Vec<ParamSpec>,
}
fn class_of(p: &ParamSet, name: &str) -> Result<AttrClass, CookError> {
    match p.menu(name).ok_or_else(|| bad(WHO, format!("`{name}` missing or not a menu")))? {
        GT_POINT => Ok(AttrClass::Point),
        GT_PRIM => Ok(AttrClass::Prim),
        GT_VERTEX => Ok(AttrClass::Vertex),
        m => Err(bad(WHO, format!("`{name}`: menu index {m} out of range"))),
    }
}
/// `any`/`all` reduction of `sel` over `total` counts (all requires total > 0).
fn reduce(all: bool, sel: usize, total: usize) -> bool {
    if all { total > 0 && sel == total } else { sel > 0 }
}
fn promote(d: &Detail, src: &[bool], from: AttrClass, to: AttrClass, all: bool) -> Vec<bool> {
    let n_to = d.class_len(to);
    if from == to {
        return (0..n_to).map(|i| src.get(i).copied().unwrap_or(false)).collect();
    }
    let at = |i: usize| src.get(i).copied().unwrap_or(false);
    let mut v = vec![false; d.vertex_point.len()];
    match from {
        AttrClass::Vertex => v.iter_mut().enumerate().for_each(|(i, s)| *s = at(i)),
        AttrClass::Point => {
            for (s, &p) in v.iter_mut().zip(&d.vertex_point) {
                *s = at(p as usize);
            }
        }
        _ => {
            for (i, pr) in d.prims.iter().enumerate() {
                for k in prim_verts(d, pr) {
                    if let Some(s) = v.get_mut(k) {
                        *s = at(i);
                    }
                }
            }
        }
    }
    match to {
        AttrClass::Vertex => v,
        AttrClass::Prim => d
            .prims
            .iter()
            .map(|pr| {
                let r = prim_verts(d, pr);
                let total = r.len();
                let sel = v.get(r).map_or(0, |s| s.iter().filter(|x| **x).count());
                reduce(all, sel, total)
            })
            .collect(),
        _ => {
            let n = d.point_count as usize;
            let (mut sel, mut tot) = (vec![0usize; n], vec![0usize; n]);
            for (&p, &on) in d.vertex_point.iter().zip(&v) {
                if let (Some(t), Some(s)) = (tot.get_mut(p as usize), sel.get_mut(p as usize)) {
                    *t += 1;
                    *s += usize::from(on);
                }
            }
            sel.iter().zip(&tot).map(|(&s, &t)| reduce(all, s, t)).collect()
        }
    }
}
impl Node<Detail> for GroupPromote {
    fn type_name(&self) -> &'static str {
        "grouppromote"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let (from, to) = (class_of(p, "fromtype")?, class_of(p, "totype")?);
        let all = match p.menu("method").ok_or_else(|| bad(WHO, "`method` missing or not a menu"))? {
            ANY => false,
            ALL => true,
            m => return Err(bad(WHO, format!("`method`: menu index {m} out of range"))),
        };
        let name = p.string("group").ok_or_else(|| bad(WHO, "`group` missing or not a string"))?.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(bad(WHO, "`group` must be one non-empty name without whitespace"));
        }
        let newname = match p.string("newname").ok_or_else(|| bad(WHO, "`newname` missing or not a string"))?.trim() {
            "" => name,
            n if n.contains(char::is_whitespace) => return Err(bad(WHO, "`newname` must not contain whitespace")),
            n => n,
        };
        let remove = p.toggle("removesource").ok_or_else(|| bad(WHO, "`removesource` missing or not a toggle"))?;
        let empty = Arc::new(Detail::default());
        let d = inputs.first().unwrap_or(&empty);
        let g = d.group(from, name).ok_or_else(|| failed(WHO, format!("unknown source group `{name}`")))?;
        let mask = promote(d, &g.mask, from, to, all);
        let mut out = (**d).clone();
        if remove {
            out.groups.retain(|x| !(x.class == from && x.name == name));
        }
        set_group(&mut out, to, newname, mask);
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use super::*;

    fn menu(m: u32) -> ParamValue {
        ParamValue::Menu(m)
    }
    fn top_box() -> Arc<Detail> {
        let b = cube();
        run(
            "group",
            &[
                ("groupname", s("top")),
                ("normal", ParamValue::B(true)),
                ("dir", ParamValue::V3([0.0, 1.0, 0.0])),
                ("angle", ParamValue::F(10.0)),
            ],
            Some(&b),
        )
        .expect("cooks")
    }
    fn promo(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        run("grouppromote", ov, Some(d))
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        let got: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(got, ["group", "fromtype", "totype", "method", "newname", "removesource"]);
    }
    #[test]
    fn box_top_prim_to_points_is_4_and_all_method_is_0() {
        let d = top_box();
        assert_eq!(mask(&d, AttrClass::Prim, "top").len(), 1);
        let o =
            promo(&d, &[("group", s("top")), ("fromtype", menu(GT_PRIM)), ("totype", menu(GT_POINT))]).expect("cooks");
        let pts = mask(&o, AttrClass::Point, "top");
        assert_eq!(pts.len(), 4);
        assert!(pts.iter().all(|&i| o.p()[i][1] > 0.0), "the 4 points of the +Y face");
        assert!(o.group(AttrClass::Prim, "top").is_some(), "source kept");
        // `all`: every box corner touches 3 prims, only 1 is grouped
        let none = promo(&d, &[("group", s("top")), ("method", menu(ALL))]).expect("cooks");
        assert!(mask(&none, AttrClass::Point, "top").is_empty());
        // prim -> vertex: the face's 4 vertices
        let v = promo(&d, &[("group", s("top")), ("totype", menu(GT_VERTEX)), ("newname", s("tv"))]).expect("cooks");
        assert_eq!(mask(&v, AttrClass::Vertex, "tv").len(), 4);
    }
    #[test]
    fn box_one_corner_point_to_prims_any_3_all_0() {
        let b = cube();
        let d = run("group", &[("groupname", s("c")), ("grouptype", menu(GT_POINT)), ("pattern", s("0"))], Some(&b))
            .expect("cooks");
        let ov =
            |m: u32| [("group", s("c")), ("fromtype", menu(GT_POINT)), ("totype", menu(GT_PRIM)), ("method", menu(m))];
        assert_eq!(mask(&promo(&d, &ov(ANY)).expect("cooks"), AttrClass::Prim, "c").len(), 3);
        assert_eq!(mask(&promo(&d, &ov(ALL)).expect("cooks"), AttrClass::Prim, "c").len(), 0);
        // all 4 points of one face -> that prim qualifies for `all`
        let top = top_box();
        let pts = promo(&top, &[("group", s("top"))]).expect("cooks");
        let back = promo(
            &pts,
            &[
                ("group", s("top")),
                ("fromtype", menu(GT_POINT)),
                ("totype", menu(GT_PRIM)),
                ("method", menu(ALL)),
                ("newname", s("back")),
            ],
        )
        .expect("cooks");
        assert_eq!(mask(&back, AttrClass::Prim, "back"), mask(&top, AttrClass::Prim, "top"));
    }
    #[test]
    fn vertex_roundtrip_same_type_rename_remove_and_hostile() {
        let d = top_box();
        let c = promo(
            &d,
            &[
                ("group", s("top")),
                ("totype", menu(GT_PRIM)),
                ("newname", s("t2")),
                ("removesource", ParamValue::B(true)),
            ],
        )
        .expect("cooks");
        assert!(c.group(AttrClass::Prim, "top").is_none());
        assert_eq!(mask(&c, AttrClass::Prim, "t2"), mask(&d, AttrClass::Prim, "top"));
        let vtx = promo(&d, &[("group", s("top")), ("totype", menu(GT_VERTEX))]).expect("cooks");
        let pt = promo(&vtx, &[("group", s("top")), ("fromtype", menu(GT_VERTEX)), ("totype", menu(GT_POINT))])
            .expect("cooks");
        assert_eq!(mask(&pt, AttrClass::Point, "top").len(), 4);
        for (k, v) in
            [("fromtype", menu(7)), ("totype", menu(3)), ("method", menu(2)), ("group", s("")), ("newname", s("a b"))]
        {
            assert!(matches!(promo(&d, &[("group", s("top")), (k, v)]), Err(CookError::BadParam(_))), "{k}");
        }
        assert!(matches!(promo(&d, &[("group", s("nope"))]), Err(CookError::Failed(_))));
    }
}
