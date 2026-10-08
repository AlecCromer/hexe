//! Connectivity SOP (token `connectivity`): labels every connected piece of the input with an integer `class` attribute.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Union-find over POINTS: every vertex of a prim is unioned with the first vertex of that prim (any prim kind; dangling
//! vertex ids are skipped). `connecttype` menu: 0 `prim` (default) writes the PRIM attribute `attribname`, 1 `point` writes
//! the POINT attribute. `attribname` S `class`. Values are I32, numbered from 0 in order of first appearance walking the
//! output elements by ascending id (so two disjoint boxes: points 0..8 -> 0, 8..16 -> 1; prims 0..6 -> 0, 6..12 -> 1).
//! A point in no prim is its own piece in `point` mode (and gets no prim in `prim` mode); a prim without any valid vertex
//! is its own piece. An existing same-name attribute of that class is replaced. Everything else is untouched.
//! NOT modelled (DEFERRED): `pointconn`-style edge-only / `attrib`-split modes, group-restricted labelling, `class` as a
//! prim-group split, polysoup unpack.
use super::meshutil::{bad, failed};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const CONN_PRIM: u32 = 0;
const CONN_POINT: u32 = 1;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("connecttype", ParamValue::Menu(CONN_PRIM))
            .label("Connectivity Type")
            .menu(&[("prim", "Primitives"), ("point", "Points")]),
        ParamSpec::new("attribname", ParamValue::S("class".into())).label("Attribute Name"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Connectivity { specs: spec() })
}
struct Connectivity {
    specs: Vec<ParamSpec>,
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while let Some(&p) = parent.get(x) {
        if p == x {
            break;
        }
        let gp = parent.get(p).copied().unwrap_or(p);
        if let Some(s) = parent.get_mut(x) {
            *s = gp; // path halving
        }
        x = gp;
    }
    x
}

/// Valid point ids of each prim (out-of-range vertex / point ids skipped).
fn prim_points(d: &Detail) -> Vec<Vec<usize>> {
    let np = d.point_count as usize;
    d.prims
        .iter()
        .map(|pr| {
            let (lo, hi) = (pr.verts.start as usize, pr.verts.end as usize);
            d.vertex_point
                .get(lo..hi)
                .unwrap_or(&[])
                .iter()
                .filter_map(|&p| usize::try_from(p).ok().filter(|&p| p < np))
                .collect()
        })
        .collect()
}

/// `(point root, prim piece lists)`: union-find roots per point.
fn roots(d: &Detail, prims: &[Vec<usize>]) -> Vec<usize> {
    let np = d.point_count as usize;
    let mut parent: Vec<usize> = (0..np).collect();
    for pts in prims {
        let Some(&first) = pts.first() else { continue };
        for &p in pts {
            let (a, b) = (find(&mut parent, first), find(&mut parent, p));
            if a != b {
                // lower root wins: deterministic
                let (lo, hi) = (a.min(b), a.max(b));
                if let Some(s) = parent.get_mut(hi) {
                    *s = lo;
                }
            }
        }
    }
    (0..np).map(|i| find(&mut parent, i)).collect()
}

impl Node<Detail> for Connectivity {
    fn type_name(&self) -> &'static str {
        "connectivity"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let ty = match ctx.params.menu("connecttype") {
            Some(m @ (CONN_PRIM | CONN_POINT)) => m,
            Some(m) => return Err(bad(format!("connectivity: `connecttype` menu {m} out of range 0..=1"))),
            None => return Err(bad("connectivity: `connecttype`: missing or not a menu")),
        };
        let name = match ctx.params.string("attribname") {
            Some(s) if !s.trim().is_empty() && !s.contains(char::is_whitespace) => s.to_owned(),
            _ => return Err(bad("connectivity: `attribname` must be a non-empty name without spaces")),
        };
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let prims = prim_points(src);
        let root = roots(src, &prims);
        let mut class_of_root = std::collections::HashMap::<usize, i32>::new();
        let mut next = 0i32;
        let mut label = |key: usize| -> Result<i32, CookError> {
            if let Some(&c) = class_of_root.get(&key) {
                return Ok(c);
            }
            let c = next;
            next = next.checked_add(1).ok_or_else(|| failed("connectivity", "class overflow"))?;
            class_of_root.insert(key, c);
            Ok(c)
        };
        let mut out = (**src).clone();
        let (class, values) = if ty == CONN_POINT {
            let v: Vec<i32> = root.iter().map(|&r| label(r)).collect::<Result<_, _>>()?;
            (AttrClass::Point, v)
        } else {
            let mut lone = usize::MAX; // synthetic keys for prims with no valid vertex (never a point root)
            let v: Vec<i32> = prims
                .iter()
                .map(|pts| match pts.first().and_then(|&p| root.get(p)) {
                    Some(&r) => label(r),
                    None => {
                        lone -= 1;
                        label(lone)
                    }
                })
                .collect::<Result<_, _>>()?;
            (AttrClass::Prim, v)
        };
        out.add_attr(Attribute { name, class, data: AttrData::I32(values), type_info: TypeInfo::None });
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cube, defaults};
    use super::*;
    use hexe_graph::ParamSet;

    fn two_boxes() -> Detail {
        let mut b = cube();
        for p in b.p_mut().iter_mut() {
            p[0] += 3.0;
        }
        let mut a = cube();
        a.merge(&b);
        a
    }
    fn i32s(d: &Detail, c: AttrClass, n: &str) -> Vec<i32> {
        match d.attr(c, n).map(|a| &a.data) {
            Some(AttrData::I32(v)) => v.clone(),
            other => panic!("{n}: {other:?}"),
        }
    }
    fn point_mode() -> ParamSet {
        defaults(&*new()).with("connecttype", ParamValue::Menu(1))
    }

    #[test]
    fn two_disjoint_boxes_are_classes_0_and_1_on_points_and_prims() {
        let d = two_boxes();
        assert_eq!((d.point_count, d.prim_count()), (16, 12));
        let pt = cook(&*new(), d.clone(), &point_mode()).expect("cooks");
        let want: Vec<i32> = (0..16).map(|i| i / 8).collect();
        assert_eq!(i32s(&pt, AttrClass::Point, "class"), want);
        assert!(pt.attr(AttrClass::Prim, "class").is_none());
        let pr = cook(&*new(), d, &defaults(&*new())).expect("cooks");
        let want: Vec<i32> = (0..12).map(|i| i / 6).collect();
        assert_eq!(i32s(&pr, AttrClass::Prim, "class"), want);
        assert!(pr.attr(AttrClass::Point, "class").is_none());
    }

    /// Chain q0-q1 share point 1,4; q2 is far away; a loose point is its own piece (point mode only).
    #[test]
    fn chain_pieces_numbered_by_first_appearance_and_loose_point_is_alone() {
        let mut d = Detail::default();
        d.add_points(11);
        d.add_poly(&[0, 1, 4, 3], true); // 0
        d.add_poly(&[1, 2, 5, 4], true); // 1 (shares 1,4 with prim 0)
        d.add_poly(&[6, 7, 8, 9], true); // 2 separate
        // point 10 loose
        let pt = cook(&*new(), d.clone(), &point_mode()).expect("cooks");
        assert_eq!(i32s(&pt, AttrClass::Point, "class"), vec![0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2]);
        let pr = cook(&*new(), d, &defaults(&*new())).expect("cooks");
        assert_eq!(i32s(&pr, AttrClass::Prim, "class"), vec![0, 0, 1]);
    }

    #[test]
    fn a_late_bridge_prim_merges_earlier_pieces_and_name_is_configurable() {
        let mut d = Detail::default();
        d.add_points(4);
        d.add_poly(&[0, 1], false);
        d.add_poly(&[2, 3], false);
        let bridge = d.add_poly(&[1, 2], false);
        let p = defaults(&*new()).with("attribname", ParamValue::S("piece".into()));
        let out = cook(&*new(), d.clone(), &p).expect("cooks");
        assert_eq!(bridge, 2);
        assert_eq!(i32s(&out, AttrClass::Prim, "piece"), vec![0, 0, 0]);
        // existing attribute of that name+class is replaced, not duplicated
        let again = cook(&*new(), (*out).clone(), &p).expect("cooks");
        assert_eq!(again.attrs.iter().filter(|a| a.name == "piece").count(), 1);
    }

    #[test]
    fn hostile_inputs_do_not_panic() {
        for bad in [
            defaults(&*new()).with("connecttype", ParamValue::Menu(2)),
            defaults(&*new()).with("attribname", ParamValue::S(String::new())),
            defaults(&*new()).with("attribname", ParamValue::S("a b".into())),
            ParamSet::new(),
        ] {
            assert!(matches!(cook(&*new(), cube(), &bad), Err(CookError::BadParam(_))));
        }
        assert_eq!(cook(&*new(), Detail::default(), &defaults(&*new())).expect("cooks").prim_count(), 0);
        let mut d = cube();
        d.vertex_point.push(77); // dangling
        d.prims.push(hexe_geo::Prim { kind: hexe_geo::PrimKind::Poly { closed: true }, verts: 24..25 });
        d.prims.push(hexe_geo::Prim { kind: hexe_geo::PrimKind::Poly { closed: true }, verts: 90..99 }); // out of range
        let out = cook(&*new(), d, &defaults(&*new())).expect("cooks");
        assert_eq!(i32s(&out, AttrClass::Prim, "class"), vec![0, 0, 0, 0, 0, 0, 1, 2], "vertex-less prims are alone");
    }

    #[test]
    fn example_scene_labels_two_boxes() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/connectivity.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (16, 12));
        assert_eq!(i32s(&d, AttrClass::Prim, "class"), (0..12).map(|i| i / 6).collect::<Vec<i32>>());
        assert_eq!(crate::registry().create("connectivity").map(|n| n.type_name()), Some("connectivity"));
    }
}
