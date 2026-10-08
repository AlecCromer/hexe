//! Delete SOP (token `delete`): removes points or primitives chosen by group pattern, element range and/or expression.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `entity` = `0 point | 1 prim`. Selection = AND of every ACTIVE criterion (none active = every element, like `blast`):
//! * `group` non-empty: pattern exactly as in the `group` SOP (ranges, `*`, `!`, group names).
//! * `userange`: element `i` with `start <= i <= end` and `(i - start) % step == 0`; `end < 0` = last element; `start < 0`
//!   or `step < 1` -> `BadParam`.
//! * `useexpr`: VEX-subset expression (`crates/vex`) per element, non-zero = selected (`@P.y > 0`, `@ptnum % 2 == 0`; prim
//!   `@P` = centroid). Read-only: any write / addpoint / ... -> `Failed`; empty or unparsable -> `BadParam`.
//!
//! `negate` deletes everything NOT selected (keep the selection instead).
//! * prim entity: selected prims go, ALL points stay (also ones left unused) unless `delete_unused_points`.
//! * point entity: selected points go. A prim touching one is removed whole; with `keep_unused` it survives instead with
//!   just the deleted vertices dropped (removed anyway if that leaves < 3 corners closed / < 2 open). The name is the
//!   spec's; this reading is a GUESS.
//! * `delete_unused_points`: also removes points orphaned BY this deletion (points unused before stay).
//!
//! Order of survivors preserved; attributes and groups compact along. Nothing deleted -> input `Arc` shared.
use super::group_sop::{GT_POINT, GT_PRIM, bad, parse_pattern, prim_verts};
use super::selutil::{eval_expr, rebuild};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const WHO: &str = "delete";
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("entity", ParamValue::Menu(GT_POINT))
            .label("Entity")
            .menu(&[("point", "Points"), ("prim", "Primitives")]),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("userange", ParamValue::B(false)).label("Delete by Range"),
        ParamSpec::new("start", ParamValue::I(0)).label("Start"),
        ParamSpec::new("end", ParamValue::I(-1)).label("End"),
        ParamSpec::new("step", ParamValue::I(1)).label("Step"),
        ParamSpec::new("useexpr", ParamValue::B(false)).label("Delete by Expression"),
        ParamSpec::new("expr", ParamValue::S(String::new())).label("Expression"),
        ParamSpec::new("negate", ParamValue::B(false)).label("Delete Non Selected"),
        ParamSpec::new("keep_unused", ParamValue::B(false)).label("Keep Prims With Deleted Points"),
        ParamSpec::new("delete_unused_points", ParamValue::B(false)).label("Delete Unused Points"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Delete { specs: spec() })
}
struct Delete {
    specs: Vec<ParamSpec>,
}
fn toggle(p: &hexe_graph::ParamSet, n: &str) -> Result<bool, CookError> {
    p.toggle(n).ok_or_else(|| bad(WHO, format!("`{n}` missing or not a toggle")))
}
fn int(p: &hexe_graph::ParamSet, n: &str) -> Result<i32, CookError> {
    p.int(n).ok_or_else(|| bad(WHO, format!("`{n}` missing or not an integer")))
}
/// Fewest corners a prim of this kind may keep.
fn min_corners(k: &PrimKind) -> usize {
    match k {
        PrimKind::Poly { closed: true } | PrimKind::Tube { closed: true, .. } => 3,
        PrimKind::Poly { closed: false } | PrimKind::Polyline | PrimKind::Tube { .. } => 2,
        _ => 1,
    }
}
impl Node<Detail> for Delete {
    fn type_name(&self) -> &'static str {
        "delete"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let class = match p.menu("entity").ok_or_else(|| bad(WHO, "`entity` missing or not a menu"))? {
            GT_POINT => AttrClass::Point,
            GT_PRIM => AttrClass::Prim,
            m => return Err(bad(WHO, format!("`entity`: menu index {m} out of range (point | prim)"))),
        };
        let pat = p.string("group").ok_or_else(|| bad(WHO, "`group` missing or not a string"))?;
        let (use_range, use_expr) = (toggle(p, "userange")?, toggle(p, "useexpr")?);
        let (negate, keep_unused, del_unused) =
            (toggle(p, "negate")?, toggle(p, "keep_unused")?, toggle(p, "delete_unused_points")?);
        let (start, end, step) = (int(p, "start")?, int(p, "end")?, int(p, "step")?);
        let expr = p.string("expr").ok_or_else(|| bad(WHO, "`expr` missing or not a string"))?;
        if use_range && (start < 0 || step < 1) {
            return Err(bad(WHO, "range needs start >= 0 and step >= 1"));
        }
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let n = d.class_len(class);
        let mut sel = parse_pattern(d, class, pat, WHO)?.unwrap_or_else(|| vec![true; n]);
        if use_range {
            let (s, st) = (start as usize, step as usize);
            let e = if end < 0 { usize::MAX } else { end as usize };
            for (i, b) in sel.iter_mut().enumerate() {
                *b &= i >= s && i <= e && (i - s) % st == 0;
            }
        }
        if use_expr {
            for (b, v) in sel.iter_mut().zip(eval_expr(d, class, expr, WHO)?) {
                *b &= v != 0.0;
            }
        }
        let del: Vec<bool> = sel.iter().map(|b| *b != negate).collect();
        if !del.iter().any(|b| *b) {
            return Ok(Arc::clone(d));
        }
        let npt = d.point_count as usize;
        let (mut del_pt, mut del_prim) = (vec![false; npt], vec![false; d.prims.len()]);
        // kept vertex ids per surviving prim
        let mut kept: Vec<Option<Vec<usize>>> = Vec::with_capacity(d.prims.len());
        if class == AttrClass::Point {
            del_pt.clone_from(&del);
        } else {
            del_prim.clone_from(&del);
        }
        for (qi, pr) in d.prims.iter().enumerate() {
            let verts: Vec<usize> = prim_verts(d, pr).collect();
            if class == AttrClass::Prim {
                kept.push((!del_prim.get(qi).copied().unwrap_or(false)).then_some(verts));
                continue;
            }
            let gone =
                |v: &usize| d.vertex_point.get(*v).is_some_and(|q| del_pt.get(*q as usize).copied().unwrap_or(false));
            if !verts.iter().any(gone) {
                kept.push(Some(verts));
            } else if keep_unused {
                let rest: Vec<usize> = verts.iter().copied().filter(|v| !gone(v)).collect();
                kept.push((rest.len() >= min_corners(&pr.kind)).then_some(rest));
            } else {
                kept.push(None);
            }
        }
        if del_unused {
            let mut before = vec![false; npt];
            let mut after = vec![false; npt];
            for &q in &d.vertex_point {
                if let Some(b) = before.get_mut(q as usize) {
                    *b = true;
                }
            }
            for verts in kept.iter().flatten() {
                for &q in verts.iter().filter_map(|v| d.vertex_point.get(*v)) {
                    if let Some(a) = after.get_mut(q as usize) {
                        *a = true;
                    }
                }
            }
            for ((dp, b), a) in del_pt.iter_mut().zip(&before).zip(&after) {
                *dp |= *b && !*a;
            }
        }
        let pidx: Vec<usize> = (0..npt).filter(|&i| !del_pt.get(i).copied().unwrap_or(false)).collect();
        let prims: Vec<(usize, Vec<usize>)> =
            kept.into_iter().enumerate().filter_map(|(q, v)| v.map(|v| (q, v))).collect();
        Ok(Arc::new(rebuild(d, &pidx, &prims, WHO)?))
    }
}
#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use super::*;
    use hexe_geo::{AttrData, Attribute, TypeInfo};
    fn del(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        run("delete", ov, Some(d))
    }
    fn prim() -> ParamValue {
        ParamValue::Menu(GT_PRIM)
    }
    fn on() -> ParamValue {
        ParamValue::B(true)
    }
    fn expr(e: &str) -> [(&str, ParamValue); 2] {
        [("useexpr", on()), ("expr", s(e))]
    }
    /// 3x3 lattice, x = c-1, y = r-1, point id = r*3+c; 4 quads in row-major order; point attr `id`, prim attr `w`.
    fn lattice() -> Arc<Detail> {
        let mut d = Detail::default();
        d.add_points(9);
        *d.p_mut() = (0..9).map(|i| [(i % 3) as f32 - 1.0, (i / 3) as f32 - 1.0, 0.0]).collect();
        for r in 0..2u32 {
            for c in 0..2u32 {
                let b = r * 3 + c;
                d.add_poly(&[b, b + 1, b + 4, b + 3], true);
            }
        }
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
        Arc::new(d)
    }
    fn ids(d: &Detail) -> Vec<i32> {
        match d.attr(AttrClass::Point, "id").map(|a| &a.data) {
            Some(AttrData::I32(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
    fn w(d: &Detail) -> Vec<f32> {
        match d.attr(AttrClass::Prim, "w").map(|a| &a.data) {
            Some(AttrData::F32(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
    /// A point cloud (no prims) with x = 0..5.
    fn cloud() -> Arc<Detail> {
        let mut d = Detail::default();
        d.add_points(5);
        *d.p_mut() = (0..5).map(|i| [i as f32, (i % 2) as f32, 0.0]).collect();
        Arc::new(d)
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("entity", ParamValue::Menu(GT_POINT)),
                ("group", s("")),
                ("userange", ParamValue::B(false)),
                ("start", ParamValue::I(0)),
                ("end", ParamValue::I(-1)),
                ("step", ParamValue::I(1)),
                ("useexpr", ParamValue::B(false)),
                ("expr", s("")),
                ("negate", ParamValue::B(false)),
                ("keep_unused", ParamValue::B(false)),
                ("delete_unused_points", ParamValue::B(false)),
            ]
        );
    }
    #[test]
    fn grid_delete_points_p_x_gt_0_counts_by_hand() {
        // points with x>0 are c=2: ids 2,5,8 -> 3 gone, 6 left; quads (r,1) touch them -> 2 gone, 2 left
        let g = lattice();
        let o = del(&g, &expr("@P.x > 0")).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (6, 2));
        assert_eq!(ids(&o), [0, 1, 3, 4, 6, 7]);
        assert_eq!(w(&o), [10.0, 12.0], "quads (0,0) and (1,0) survive");
        assert!(o.p().iter().all(|q| q[0] <= 0.0));
        assert!(o.vertex_point.iter().all(|&q| q < 6), "vertex refs remapped into the compacted point range");
        assert_eq!(
            o.vertex_point,
            [0, 1, 3, 2, 2, 3, 5, 4],
            "quad (0,0): 0,1,4,3 -> 0,1,3,2; quad (1,0): 3,4,7,6 -> 2,3,5,4"
        );
        // negate keeps the x>0 column only: 3 points, every quad touches an x<=0 point -> 0 prims
        let k = del(&g, &[("useexpr", on()), ("expr", s("@P.x > 0")), ("negate", on())]).expect("cooks");
        assert_eq!((k.point_count, k.prim_count()), (3, 0));
        assert_eq!(ids(&k), [2, 5, 8]);
    }
    #[test]
    fn prim_delete_keeps_points_unless_delete_unused_points() {
        let g = lattice();
        // quad centroids: x = -.5 (c=0), +.5 (c=1) -> delete the c=1 quads (ids 1, 3)
        let o = del(&g, &[("entity", prim()), ("useexpr", on()), ("expr", s("@P.x > 0"))]).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (9, 2), "points kept");
        assert_eq!(w(&o), [10.0, 12.0]);
        let o =
            del(&g, &[("entity", prim()), ("useexpr", on()), ("expr", s("@P.x > 0")), ("delete_unused_points", on())])
                .expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (6, 2), "the x=1 column (3 points) only fed deleted quads");
        assert_eq!(ids(&o), [0, 1, 3, 4, 6, 7]);
        // delete_unused never touches points that were unused BEFORE (orphan cloud point survives)
        let mut d = (*g).clone();
        d.add_points(1);
        let d = Arc::new(d);
        let o = del(&d, &[("entity", prim()), ("delete_unused_points", on())]).expect("cooks");
        assert_eq!(
            (o.point_count, o.prim_count()),
            (1, 0),
            "all prims gone, 9 orphaned points go, the pre-existing orphan stays"
        );
    }
    #[test]
    fn range_pattern_expression_and_negate() {
        let c = cloud();
        let xs = |d: &Detail| d.p().iter().map(|q| q[0]).collect::<Vec<_>>();
        let rg = |a: i32, b: i32, st: i32| {
            [("userange", on()), ("start", ParamValue::I(a)), ("end", ParamValue::I(b)), ("step", ParamValue::I(st))]
        };
        assert_eq!(xs(&del(&c, &rg(1, 3, 2)).expect("cooks")), [0.0, 2.0, 4.0], "deletes 1 and 3");
        assert_eq!(xs(&del(&c, &rg(0, -1, 2)).expect("cooks")), [1.0, 3.0], "0,2,4 gone, end<0 = last");
        assert_eq!(xs(&del(&c, &rg(1, 100, 1)).expect("cooks")), [0.0], "end past the last element is fine");
        let mut neg = rg(1, 3, 2).to_vec();
        neg.push(("negate", on()));
        assert_eq!(xs(&del(&c, &neg).expect("cooks")), [1.0, 3.0], "negate keeps instead");
        assert_eq!(xs(&del(&c, &expr("@ptnum % 2 == 0")).expect("cooks")), [1.0, 3.0]);
        let mut e = expr("@ptnum % 2 == 0").to_vec();
        e.push(("negate", on()));
        assert_eq!(xs(&del(&c, &e).expect("cooks")), [0.0, 2.0, 4.0]);
        // pattern AND expression AND range: pattern 0-3, even ptnum, range 2..: only 2 qualifies
        let mut all = expr("@ptnum % 2 == 0").to_vec();
        all.extend(rg(2, -1, 1));
        all.push(("group", s("0-3")));
        assert_eq!(xs(&del(&c, &all).expect("cooks")), [0.0, 1.0, 3.0, 4.0]);
        // pattern alone, with `!`
        assert_eq!(xs(&del(&c, &[("group", s("!1-2"))]).expect("cooks")), [1.0, 2.0]);
        // attribute expression
        assert_eq!(xs(&del(&c, &expr("@P.y > 0.5")).expect("cooks")), [0.0, 2.0, 4.0], "odd points have y = 1");
        // nothing selected -> shared
        assert!(Arc::ptr_eq(&del(&c, &expr("@ptnum > 99")).expect("cooks"), &c));
    }
    #[test]
    fn point_delete_keep_unused_shortens_prims() {
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2, 3], true);
        let d = Arc::new(d);
        let sel = [("group", s("3"))];
        let o = del(&d, &sel).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (3, 0), "dependent prim removed; its other points stay orphaned");
        let o = del(&d, &[("group", s("3")), ("delete_unused_points", on())]).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (0, 0));
        let o = del(&d, &[("group", s("3")), ("keep_unused", on())]).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (3, 1));
        assert_eq!(o.vertex_point, [0, 1, 2], "quad -> triangle over the surviving points");
        let o = del(&d, &[("group", s("2-3")), ("keep_unused", on())]).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (2, 0), "a 2-corner closed polygon is dropped");
    }
    #[test]
    fn groups_and_vertex_attrs_follow() {
        let mut d = (*lattice()).clone();
        d.groups.push(hexe_geo::Group {
            name: "top".into(),
            class: AttrClass::Prim,
            mask: vec![false, false, true, true],
        });
        d.add_attr(Attribute {
            name: "vid".into(),
            class: AttrClass::Vertex,
            data: AttrData::I32((0..16).collect()),
            type_info: TypeInfo::None,
        });
        let d = Arc::new(d);
        let o = del(&d, &[("entity", prim()), ("group", s("0 3"))]).expect("cooks");
        assert_eq!(w(&o), [11.0, 12.0]);
        assert_eq!(o.group(AttrClass::Prim, "top").map(|g| g.mask.clone()), Some(vec![false, true]));
        let Some(AttrData::I32(v)) = o.attr(AttrClass::Vertex, "vid").map(|a| &a.data) else { panic!("vid lost") };
        assert_eq!(v, &[4, 5, 6, 7, 8, 9, 10, 11]);
        // group by name
        let o = del(&d, &[("entity", prim()), ("group", s("top")), ("negate", on())]).expect("cooks");
        assert_eq!(w(&o), [12.0, 13.0]);
    }
    #[test]
    fn hostile() {
        let g = lattice();
        assert!(matches!(del(&g, &[("entity", ParamValue::Menu(2))]), Err(CookError::BadParam(_))));
        assert!(matches!(del(&g, &[("group", s("nope"))]), Err(CookError::Failed(_))));
        assert!(matches!(del(&g, &[("group", s("3-1"))]), Err(CookError::BadParam(_))));
        assert!(matches!(del(&g, &[("useexpr", on())]), Err(CookError::BadParam(_))), "empty expression");
        assert!(matches!(del(&g, &expr("@P.x >")), Err(CookError::BadParam(_))), "syntax error");
        assert!(matches!(del(&g, &expr("@P")), Err(CookError::Failed(_))), "vector result");
        assert!(matches!(del(&g, &expr("removepoint(0, 1)")), Err(CookError::Failed(_))), "writes refused");
        assert!(matches!(del(&g, &[("userange", on()), ("step", ParamValue::I(0))]), Err(CookError::BadParam(_))));
        assert!(matches!(del(&g, &[("userange", on()), ("start", ParamValue::I(-1))]), Err(CookError::BadParam(_))));
        // an inactive range is not validated
        assert!(del(&g, &[("step", ParamValue::I(0)), ("group", s("99"))]).is_ok());
        assert_eq!(run("delete", &[], None).expect("cooks").point_count, 0);
        let e = del(&Arc::new(Detail::default()), &[("group", s("0-9"))]).expect("cooks");
        assert_eq!((e.point_count, e.prim_count()), (0, 0));
    }
    #[test]
    fn scene_loader_cooks_line_delete_and_registry_has_slot() {
        let s = r#"{"nodes":[
            {"name":"l","type":"line","params":{"points":5,"dist":4.0,"dir":[1,0,0]}},
            {"name":"d","type":"delete","params":{"entity":0,"useexpr":true,"expr":"@ptnum % 2 == 0"}}],
            "edges":[["l",0,"d",0]],"display":"d"}"#;
        let (mut g, id) = crate::load_scene(s).expect("loads");
        let out = g.cook(id, 0.0).expect("cooks");
        // the polyline touches every deleted point -> removed; the 2 odd points stay as a cloud
        assert_eq!((out.point_count, out.prim_count()), (2, 0));
        assert_eq!(out.p().iter().map(|q| q[0]).collect::<Vec<_>>(), [1.0, 3.0]);
        assert_eq!(crate::registry().create("delete").map(|n| n.type_name()), Some("delete"));
    }
}
