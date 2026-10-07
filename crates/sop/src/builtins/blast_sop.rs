//! Blast SOP (token `blast`): deletes points or primitives.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `group` = pattern over the `grouptype` class (point | prim): ranges, `*`, `!`, group names exactly as in the `group` SOP.
//! EMPTY pattern = every element (Houdini's empty group field = all). `negate` deletes everything NOT selected (keep only group).
//! * prim mode: selected prims go; points stay unless orphaned by this deletion AND `removeunused` (default on).
//! * point mode: selected points go, and every prim using one of them goes with it (whole prim, never a shortened one);
//!   points orphaned by those collateral prim deletions follow the same `removeunused` rule.
//! Points that were already unused before the blast (point clouds) are never touched by `removeunused`.
//! Order of survivors is preserved; attributes and groups are compacted along. Nothing deleted -> input `Arc` shared.
use super::group_sop::{GT_POINT, GT_PRIM, bad, compact, parse_pattern, prim_verts};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const WHO: &str = "blast";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("grouptype", ParamValue::Menu(GT_PRIM)).label("Group Type"),
        ParamSpec::new("negate", ParamValue::B(false)).label("Delete Non Selected"),
        ParamSpec::new("removeunused", ParamValue::B(true)).label("Delete Unused Points"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Blast { specs: spec() })
}
struct Blast {
    specs: Vec<ParamSpec>,
}
impl Node<Detail> for Blast {
    fn type_name(&self) -> &'static str {
        "blast"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let class = match p.menu("grouptype").ok_or_else(|| bad(WHO, "`grouptype` missing or not a menu"))? {
            GT_POINT => AttrClass::Point,
            GT_PRIM => AttrClass::Prim,
            m => return Err(bad(WHO, format!("`grouptype`: menu index {m} out of range (point | prim)"))),
        };
        let pat = p.string("group").ok_or_else(|| bad(WHO, "`group` missing or not a string"))?;
        let negate = p.toggle("negate").ok_or_else(|| bad(WHO, "`negate` missing or not a toggle"))?;
        let unused = p.toggle("removeunused").ok_or_else(|| bad(WHO, "`removeunused` missing or not a toggle"))?;
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let n = d.class_len(class);
        let mut del = parse_pattern(d, class, pat, WHO)?.unwrap_or_else(|| vec![true; n]);
        if negate {
            del.iter_mut().for_each(|b| *b = !*b);
        }
        if !del.iter().any(|b| *b) {
            return Ok(Arc::clone(d));
        }
        let (mut del_pt, mut del_prim) = (vec![false; d.point_count as usize], vec![false; d.prims.len()]);
        if class == AttrClass::Point {
            del_pt.clone_from(&del);
            for (dp, pr) in del_prim.iter_mut().zip(&d.prims) {
                let r = prim_verts(d, pr);
                let hit = d
                    .vertex_point
                    .get(r)
                    .is_some_and(|v| v.iter().any(|&q| del_pt.get(q as usize).copied().unwrap_or(false)));
                *dp = hit;
            }
        } else {
            del_prim.clone_from(&del);
        }
        if unused {
            let mut before = vec![false; del_pt.len()];
            let mut after = vec![false; del_pt.len()];
            for &q in &d.vertex_point {
                if let Some(b) = before.get_mut(q as usize) {
                    *b = true;
                }
            }
            for (pr, &gone) in d.prims.iter().zip(&del_prim) {
                if gone {
                    continue;
                }
                for &q in d.vertex_point.get(prim_verts(d, pr)).unwrap_or(&[]) {
                    if let Some(a) = after.get_mut(q as usize) {
                        *a = true;
                    }
                }
            }
            for ((dp, b), a) in del_pt.iter_mut().zip(&before).zip(&after) {
                *dp |= *b && !*a;
            }
        }
        let keep_pt: Vec<bool> = del_pt.iter().map(|b| !*b).collect();
        let keep_prim: Vec<bool> = del_prim.iter().map(|b| !*b).collect();
        Ok(Arc::new(compact(d, &keep_pt, &keep_prim, WHO)?))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use super::*;
    use hexe_geo::{AttrData, Attribute, TypeInfo};

    fn blast(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        run("blast", ov, Some(d))
    }
    fn top_box() -> Arc<Detail> {
        run(
            "group",
            &[
                ("groupname", s("top")),
                ("normal", ParamValue::B(true)),
                ("dir", ParamValue::V3([0.0, 1.0, 0.0])),
                ("angle", ParamValue::F(10.0)),
            ],
            Some(&cube()),
        )
        .expect("cooks")
    }
    fn pt() -> ParamValue {
        ParamValue::Menu(GT_POINT)
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("group", s("")),
                ("grouptype", ParamValue::Menu(GT_PRIM)),
                ("negate", ParamValue::B(false)),
                ("removeunused", ParamValue::B(true))
            ]
        );
    }
    #[test]
    fn box_blast_prim_0_is_5_prims_8_points() {
        let o = blast(&cube(), &[("group", s("0"))]).expect("cooks");
        assert_eq!((o.prim_count(), o.point_count), (5, 8));
    }
    #[test]
    fn box_blast_top_group_negated_is_1_prim_4_points_and_unnegated_is_5_prims() {
        let d = top_box();
        let o = blast(&d, &[("group", s("top")), ("negate", ParamValue::B(true))]).expect("cooks");
        assert_eq!((o.prim_count(), o.point_count), (1, 4));
        assert!(o.p().iter().all(|q| q[1] > 0.0));
        assert_eq!(mask(&o, AttrClass::Prim, "top"), [0], "groups compact with the survivors");
        let o = blast(&d, &[("group", s("top"))]).expect("cooks");
        assert_eq!((o.prim_count(), o.point_count), (5, 8));
        assert!(mask(&o, AttrClass::Prim, "top").is_empty());
        // removeunused off: negated blast keeps all 8 points
        let k =
            blast(&d, &[("group", s("top")), ("negate", ParamValue::B(true)), ("removeunused", ParamValue::B(false))])
                .expect("cooks");
        assert_eq!((k.prim_count(), k.point_count), (1, 8));
    }
    #[test]
    fn grid_blast_points_0_2_is_6_points_2_prims() {
        let g = grid3();
        let o = blast(&g, &[("grouptype", pt()), ("group", s("0-2"))]).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (6, 2));
        // the surviving points are exactly the original points 3..=8, in order
        assert_eq!(o.p(), &g.p()[3..]);
        // every survivor vertex references a valid, remapped point
        assert!(o.vertex_point.iter().all(|&q| q < 6));
    }
    #[test]
    fn attributes_and_orphans_follow() {
        let mut d = (*grid3()).clone();
        d.add_attr(Attribute {
            name: "id".into(),
            class: AttrClass::Point,
            data: AttrData::I32((0..9).collect()),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Prim,
            data: AttrData::F32(vec![10.0, 11.0, 12.0, 13.0]),
            type_info: TypeInfo::None,
        });
        let orphan = d.add_points(1); // never used by a prim: must survive any prim blast (point clouds are not "unused")
        assert_eq!(orphan, 9..10);
        let d = Arc::new(d);
        let o = blast(&d, &[("group", s("0"))]).expect("cooks");
        assert_eq!((o.prim_count(), o.point_count), (3, 9), "quad 0 owned point 0 alone -> removed; orphan stays");
        let Some(AttrData::I32(id)) = o.attr(AttrClass::Point, "id").map(|a| &a.data) else { panic!("id lost") };
        assert_eq!(id, &[1, 2, 3, 4, 5, 6, 7, 8, 0], "orphan point's default-filled id rides along");
        let Some(AttrData::F32(w)) = o.attr(AttrClass::Prim, "w").map(|a| &a.data) else { panic!("w lost") };
        assert_eq!(w, &[11.0, 12.0, 13.0]);
        // everything deleted (empty pattern) -> empty geometry, no panic
        let all = blast(&d, &[]).expect("cooks");
        assert_eq!((all.prim_count(), all.point_count), (0, 1), "unused orphan point survives a prim blast");
        let none = blast(&d, &[("group", s("")), ("negate", ParamValue::B(true))]).expect("cooks");
        assert!(Arc::ptr_eq(&none, &d), "nothing deleted -> shared Arc");
    }
    #[test]
    fn hostile() {
        let b = cube();
        assert!(matches!(blast(&b, &[("grouptype", ParamValue::Menu(2))]), Err(CookError::BadParam(_))));
        assert!(matches!(blast(&b, &[("group", s("nope"))]), Err(CookError::Failed(_))));
        assert!(matches!(blast(&b, &[("group", s("3-1"))]), Err(CookError::BadParam(_))));
        assert_eq!(run("blast", &[], None).expect("cooks").point_count, 0);
        let e = blast(&Arc::new(Detail::default()), &[("group", s("0-9"))]).expect("cooks");
        assert_eq!(e.prim_count(), 0);
    }
}

#[cfg(test)]
mod scene_tests {
    use hexe_geo::AttrClass;
    #[test]
    fn scene_loader_cooks_group_blast_example_and_registry_has_slots() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/group-blast.json"))
            .expect("example present");
        let (mut g, id) = crate::load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        // box -> group top -> promote to points -> blast (keep only those 4 points) -> delete the point group
        assert_eq!((out.point_count, out.prim_count()), (4, 1));
        assert_eq!(out.group(AttrClass::Prim, "top").map(|g| g.mask.clone()), Some(vec![true]));
        assert!(out.group(AttrClass::Point, "toppts").is_none());
        let r = crate::registry();
        for t in ["group", "groupdelete", "grouppromote", "blast"] {
            assert_eq!(r.create(t).map(|n| n.type_name()), Some(t));
        }
    }
}
