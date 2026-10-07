//! Merge SOP: concatenate the geometry of its inputs, in input order.
//!
//! Wave 2: the graph has fixed-arity nodes, so Merge declares 4 inputs (`input0..input3`); unconnected inputs arrive as an
//! empty `Detail` and are skipped. Houdini allows up to 9999 (variadic ports = graph wave 3).
//! Points / vertices / prims are appended in order with later inputs' point indices offset. Attributes union by (class, name):
//! an attribute missing on some input is default-filled (0 / empty string) for that input's elements. Same (class, name) with a
//! different type or tuple size → `CookError::Failed`. Groups union by name, masks concatenated. Detail attributes: first wins.
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Declared input ports (fixed arity until graph wave 3).
const INPUTS: [InputSpec; 4] = [
    InputSpec { name: "input0" },
    InputSpec { name: "input1" },
    InputSpec { name: "input2" },
    InputSpec { name: "input3" },
];

pub fn spec() -> Vec<ParamSpec> {
    Vec::new()
}

pub fn new() -> Box<SopNode> {
    Box::new(Merge { specs: spec() })
}

struct Merge {
    specs: Vec<ParamSpec>,
}

/// No points and no prims: nothing to contribute (Detail attrs of such an input are ignored too).
fn is_empty(d: &Detail) -> bool {
    d.point_count == 0 && d.prims.is_empty() && d.vertex_point.is_empty()
}

/// (variant kind, tuple size) of an attribute payload; two attrs may merge only if these are equal.
fn shape(a: &AttrData) -> (std::mem::Discriminant<AttrData>, u32) {
    (std::mem::discriminant(a), a.tuple_size())
}

fn class_name(c: AttrClass) -> &'static str {
    match c {
        AttrClass::Point => "point",
        AttrClass::Vertex => "vertex",
        AttrClass::Prim => "primitive",
        AttrClass::Detail => "detail",
    }
}

impl Node<Detail> for Merge {
    fn type_name(&self) -> &'static str {
        "merge"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, _: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let live: Vec<&Arc<Detail>> = inputs.iter().filter(|d| !is_empty(d)).collect();
        let Some((first, rest)) = live.split_first() else {
            return Ok(Arc::new(Detail::default()));
        };
        if rest.is_empty() {
            // merge(A) == A: share the Arc.
            return Ok(Arc::clone(first));
        }
        // Guard: element counts must stay addressable (u32 ids).
        let (mut np, mut nv, mut nq) = (0u32, 0u32, 0u32);
        for d in &live {
            np = np.checked_add(d.point_count).ok_or_else(too_big)?;
            nv = nv.checked_add(d.vertex_count()).ok_or_else(too_big)?;
            nq = nq.checked_add(d.prim_count()).ok_or_else(too_big)?;
        }
        // Type/tuple-size conflicts are errors (detail attrs: first wins, never conflict).
        let mut seen: BTreeMap<(u8, &str), (std::mem::Discriminant<AttrData>, u32)> = BTreeMap::new();
        for (k, d) in live.iter().enumerate() {
            for a in d.attrs.iter().filter(|a| a.class != AttrClass::Detail) {
                let key = (a.class as u8, a.name.as_str());
                let sh = shape(&a.data);
                match seen.get(&key) {
                    Some(prev) if *prev != sh => {
                        return Err(CookError::Failed(format!(
                            "merge: {} attribute `{}` has a different type or tuple size on input {} (tuple size {}) than on an earlier input",
                            class_name(a.class),
                            a.name,
                            k,
                            a.data.tuple_size()
                        )));
                    }
                    Some(_) => {}
                    None => {
                        seen.insert(key, sh);
                    }
                }
            }
        }
        let mut out: Detail = (***first).clone();
        for d in rest {
            out.merge(d);
        }
        Ok(Arc::new(out))
    }
}

fn too_big() -> CookError {
    CookError::Failed("merge: combined element count exceeds u32 range".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hexe_geo::{Attribute, Group, PrimKind, TypeInfo, read_geo, write_geo_json};
    use hexe_graph::ParamSet;

    fn ctx(p: &ParamSet) -> CookCtx<'_> {
        CookCtx { time: 0.0, fps: 24.0, params: p, log: &|_| {} }
    }
    fn cook(inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ParamSet::new();
        new().cook(&ctx(&p), inputs)
    }
    /// Cube corner `i` (bit0→x, bit1→y, bit2→z), each coordinate ±0.5 (closed form, independent of any SOP).
    fn corner(i: u32, shift: f32) -> [f32; 3] {
        let c = |b: u32| if (i >> b) & 1 == 1 { 0.5 } else { -0.5 };
        [c(0) + shift, c(1), c(2)]
    }
    /// Six quad faces over corner indices 0..8.
    const FACES: [[u32; 4]; 6] = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
    /// Hand-built box: 8 points, 6 closed quads, no extra attrs.
    fn hand_box(shift: f32) -> Detail {
        let mut d = Detail::default();
        let r = d.add_points(8);
        for (k, i) in r.clone().enumerate() {
            d.p_mut()[i as usize] = corner(k as u32, shift);
        }
        for f in FACES {
            d.add_poly(&f.map(|i| r.start + i), true);
        }
        d
    }
    fn arc(d: Detail) -> Arc<Detail> {
        Arc::new(d)
    }
    fn v3_attr(name: &str, class: AttrClass, v: Vec<[f32; 3]>) -> Attribute {
        Attribute { name: name.into(), class, data: AttrData::V3(v), type_info: TypeInfo::Color }
    }

    #[test]
    fn declares_four_named_inputs_no_params() {
        let n = new();
        let names: Vec<&str> = n.inputs().iter().map(|i| i.name).collect();
        assert_eq!(names, ["input0", "input1", "input2", "input3"]);
        assert!(n.params().is_empty());
        assert_eq!(n.type_name(), "merge");
    }

    #[test]
    fn two_boxes_concatenate_with_shifted_indices() {
        let out = cook(&[arc(hand_box(0.0)), arc(hand_box(10.0))]).expect("cooks");
        assert_eq!((out.point_count, out.prim_count(), out.vertex_count()), (16, 12, 48));
        // points in input order: second box starts at 8, shifted +10 in x
        assert_eq!(out.p()[0], corner(0, 0.0));
        assert_eq!(out.p()[8], corner(0, 10.0));
        assert_eq!(out.p()[15], corner(7, 10.0));
        // vertex→point: first 24 unchanged, next 24 offset by 8
        for (k, f) in FACES.iter().flatten().enumerate() {
            assert_eq!(out.vertex_point[k], *f);
            assert_eq!(out.vertex_point[24 + k], f + 8);
        }
        // prims: second box's prim ranges shifted by 24 vertices, all still closed polys
        for (q, p) in out.prims.iter().enumerate() {
            let (lo, hi) = (4 * (q as u32), 4 * (q as u32) + 4);
            assert_eq!(p.verts, lo..hi);
            assert!(matches!(p.kind, PrimKind::Poly { closed: true }));
        }
    }

    #[test]
    fn point_attr_union_zero_fills_missing_side() {
        let mut a = hand_box(0.0);
        a.add_attr(v3_attr("Cd", AttrClass::Point, vec![[1.0, 0.5, 0.25]; 8]));
        let b = hand_box(10.0); // no Cd
        let out = cook(&[arc(a.clone()), arc(b.clone())]).expect("cooks");
        let Some(AttrData::V3(cd)) = out.attr(AttrClass::Point, "Cd").map(|x| &x.data) else {
            panic!("attribute missing or wrong type")
        };
        assert_eq!(cd.len(), 16);
        assert!(cd[..8].iter().all(|c| *c == [1.0, 0.5, 0.25]));
        assert!(cd[8..].iter().all(|c| *c == [0.0; 3]));
        // reverse order: zeros first
        let rev = cook(&[arc(b), arc(a)]).expect("cooks");
        let Some(AttrData::V3(cd)) = rev.attr(AttrClass::Point, "Cd").map(|x| &x.data) else {
            panic!("attribute missing or wrong type")
        };
        assert!(cd[..8].iter().all(|c| *c == [0.0; 3]));
        assert!(cd[8..].iter().all(|c| *c == [1.0, 0.5, 0.25]));
    }

    #[test]
    fn prim_and_vertex_attrs_union_and_strings_default_empty() {
        let mut a = hand_box(0.0);
        a.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Prim,
            data: AttrData::F32(vec![1.0; 6]),
            type_info: TypeInfo::None,
        });
        let mut b = hand_box(10.0);
        b.add_attr(Attribute {
            name: "name".into(),
            class: AttrClass::Prim,
            data: AttrData::Str { idx: vec![0, 0, 1, 1, 0, 1], table: vec!["x".into(), "y".into()] },
            type_info: TypeInfo::None,
        });
        b.add_attr(Attribute {
            name: "uvw".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(vec![[1.0; 3]; 24]),
            type_info: TypeInfo::None,
        });
        let out = cook(&[arc(a), arc(b)]).expect("cooks");
        let Some(AttrData::F32(w)) = out.attr(AttrClass::Prim, "w").map(|x| &x.data) else {
            panic!("attribute missing or wrong type")
        };
        assert_eq!(w, &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let Some(AttrData::Str { idx, table }) = out.attr(AttrClass::Prim, "name").map(|x| &x.data) else {
            panic!("attribute missing or wrong type")
        };
        assert_eq!(idx.len(), 12);
        assert!(idx[..6].iter().all(|i| *i == -1), "first input has no `name`: empty string");
        let resolved: Vec<&str> = idx[6..].iter().map(|i| table[*i as usize].as_str()).collect();
        assert_eq!(resolved, ["x", "x", "y", "y", "x", "y"]);
        let Some(AttrData::V3(uvw)) = out.attr(AttrClass::Vertex, "uvw").map(|x| &x.data) else {
            panic!("attribute missing or wrong type")
        };
        assert_eq!(uvw.len(), 48);
        assert!(uvw[..24].iter().all(|c| *c == [0.0; 3]) && uvw[24..].iter().all(|c| *c == [1.0; 3]));
    }

    #[test]
    fn string_tables_are_merged_by_value() {
        let mk = |s: &str| {
            let mut d = hand_box(0.0);
            d.add_attr(Attribute {
                name: "name".into(),
                class: AttrClass::Prim,
                data: AttrData::Str { idx: vec![0; 6], table: vec![s.into()] },
                type_info: TypeInfo::None,
            });
            arc(d)
        };
        let out = cook(&[mk("a"), mk("b"), mk("a")]).expect("cooks");
        let Some(AttrData::Str { idx, table }) = out.attr(AttrClass::Prim, "name").map(|x| &x.data) else {
            panic!("attribute missing or wrong type")
        };
        let resolved: Vec<&str> = idx.iter().map(|i| table[*i as usize].as_str()).collect();
        assert_eq!(resolved.iter().filter(|s| **s == "a").count(), 12);
        assert_eq!(resolved.iter().filter(|s| **s == "b").count(), 6);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn mismatched_tuple_size_or_type_is_failed() {
        let mut a = hand_box(0.0);
        a.add_attr(v3_attr("Cd", AttrClass::Point, vec![[1.0; 3]; 8]));
        let mut b = hand_box(10.0);
        b.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V4(vec![[1.0; 4]; 8]),
            type_info: TypeInfo::Color,
        });
        let e = cook(&[arc(a.clone()), arc(b)]).expect_err("tuple size mismatch");
        assert!(matches!(&e, CookError::Failed(m) if m.contains("Cd") && m.contains("point")), "{e}");
        // type mismatch at same tuple size (float vs int)
        let mut c = hand_box(10.0);
        c.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::I32(vec![1; 8]),
            type_info: TypeInfo::None,
        });
        assert!(matches!(cook(&[arc(a.clone()), arc(c)]), Err(CookError::Failed(_))));
        // same name on a DIFFERENT class is not a conflict
        let mut d = hand_box(10.0);
        d.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Prim,
            data: AttrData::F32(vec![2.0; 6]),
            type_info: TypeInfo::None,
        });
        assert!(cook(&[arc(a), arc(d)]).is_ok());
    }

    #[test]
    fn groups_union_by_name_masks_concatenated() {
        let mut a = hand_box(0.0);
        a.groups.push(Group {
            name: "both".into(),
            class: AttrClass::Point,
            mask: vec![true, false, false, false, false, false, false, true],
        });
        a.groups.push(Group { name: "onlya".into(), class: AttrClass::Prim, mask: vec![true; 6] });
        let mut b = hand_box(10.0);
        b.groups.push(Group {
            name: "both".into(),
            class: AttrClass::Point,
            mask: vec![false, true, false, false, false, false, false, false],
        });
        b.groups.push(Group { name: "onlyb".into(), class: AttrClass::Point, mask: vec![true; 8] });
        let out = cook(&[arc(a), arc(b)]).expect("cooks");
        let both = out.group(AttrClass::Point, "both").expect("both");
        assert_eq!(both.mask.len(), 16);
        let set: Vec<usize> = both.mask.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i).collect();
        assert_eq!(set, [0, 7, 9]);
        let onlya = out.group(AttrClass::Prim, "onlya").expect("onlya");
        assert_eq!(onlya.mask.iter().filter(|m| **m).count(), 6);
        assert_eq!(onlya.mask.len(), 12);
        assert!(onlya.mask[6..].iter().all(|m| !*m));
        let onlyb = out.group(AttrClass::Point, "onlyb").expect("onlyb");
        assert_eq!(onlyb.mask.len(), 16);
        assert!(onlyb.mask[..8].iter().all(|m| !*m) && onlyb.mask[8..].iter().all(|m| *m));
    }

    #[test]
    fn detail_attr_first_wins() {
        let mk = |v: f32| {
            let mut d = hand_box(0.0);
            d.add_attr(Attribute {
                name: "tag".into(),
                class: AttrClass::Detail,
                data: AttrData::F32(vec![v]),
                type_info: TypeInfo::None,
            });
            arc(d)
        };
        let out = cook(&[mk(1.0), mk(2.0)]).expect("cooks");
        assert_eq!(out.attr(AttrClass::Detail, "tag").map(|a| a.data.clone()), Some(AttrData::F32(vec![1.0])));
    }

    #[test]
    fn empty_inputs_are_skipped_and_all_empty_is_empty() {
        let (a, b) = (arc(hand_box(0.0)), arc(hand_box(10.0)));
        let e = || arc(Detail::default());
        let plain = cook(&[a.clone(), b.clone()]).expect("cooks");
        let gappy = cook(&[e(), a.clone(), e(), b.clone()]).expect("cooks");
        assert_eq!(*plain, *gappy);
        assert_eq!(*cook(&[e(), e(), e(), e()]).expect("cooks"), Detail::default());
        assert_eq!(*cook(&[]).expect("cooks"), Detail::default());
    }

    #[test]
    fn single_live_input_is_passed_through_same_arc() {
        let a = arc(hand_box(0.0));
        let out = cook(&[arc(Detail::default()), a.clone(), arc(Detail::default())]).expect("cooks");
        assert!(Arc::ptr_eq(&out, &a));
    }

    #[test]
    fn empty_input_with_conflicting_attr_is_ignored() {
        let mut ghost = Detail::default();
        ghost.attrs.push(v3_attr("Cd", AttrClass::Point, vec![]));
        let mut a = hand_box(0.0);
        a.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![0.0; 8]),
            type_info: TypeInfo::None,
        });
        assert!(cook(&[arc(ghost), arc(a.clone()), arc(a)]).is_ok());
    }

    #[test]
    fn scene_loader_feeds_empty_for_unconnected_and_rejects_fifth_input() {
        let lone = r#"{"nodes":[{"name":"m","type":"merge"}],"display":"m"}"#;
        let (mut g, id) = crate::load_scene(lone).expect("loads");
        assert_eq!(*g.cook(id, 0.0).expect("cooks"), Detail::default());
        let wired = r#"{"nodes":[{"name":"a","type":"null"},{"name":"b","type":"null"},{"name":"m","type":"merge"}],
            "edges":[["a",0,"m",0],["b",0,"m",3]],"display":"m"}"#;
        let (mut g, id) = crate::load_scene(wired).expect("loads");
        assert_eq!(*g.cook(id, 0.0).expect("cooks"), Detail::default());
        let five = r#"{"nodes":[{"name":"a","type":"null"},{"name":"m","type":"merge"}],"edges":[["a",0,"m",4]],"display":"m"}"#;
        assert!(crate::load_scene(five).is_err(), "wave 2: max 4 inputs");
    }

    #[test]
    fn hostile_params_do_not_exist() {
        let scene = r#"{"nodes":[{"name":"m","type":"merge","params":{"bogus":1}}],"display":"m"}"#;
        assert!(crate::load_scene(scene).is_err());
    }

    // ---- oracle fixtures (analytical expected, built by closed-form maths, never by the SOP) ----

    const FIXTURE_TWO_BOXES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/merge-two-boxes/expected.geo");
    const FIXTURE_EMPTY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/merge-empty/expected.geo");

    /// Expected for two default (unit, centred) boxes: 16 points = the 8 corners twice, 12 prims, second block offset by 8.
    fn expected_two_boxes() -> Detail {
        let mut d = Detail::default();
        let r = d.add_points(16);
        for k in 0..16u32 {
            d.p_mut()[(r.start + k) as usize] = corner(k % 8, 0.0);
        }
        for blk in 0..2u32 {
            for f in FACES {
                d.add_poly(&f.map(|i| i + 8 * blk), true);
            }
        }
        d
    }

    #[test]
    #[ignore = "writes the committed fixture; run: cargo test -p hexe-sop -- --ignored write_fixture"]
    fn write_fixture() {
        for (path, d) in [(FIXTURE_TWO_BOXES, expected_two_boxes()), (FIXTURE_EMPTY, Detail::default())] {
            std::fs::write(path, write_geo_json(&d)).expect("write expected.geo");
        }
    }

    #[test]
    fn fixture_expected_equals_merge_of_hand_built_boxes() {
        let bytes = std::fs::read(FIXTURE_TWO_BOXES).expect("fixture present");
        let expected = read_geo(&bytes).expect("parses");
        let out = cook(&[arc(hand_box(0.0)), arc(hand_box(0.0))]).expect("cooks");
        assert_eq!((expected.point_count, expected.prim_count()), (16, 12));
        assert_eq!(expected.p(), out.p());
        assert_eq!(expected.vertex_point, out.vertex_point);
        assert_eq!(expected.prims, out.prims);
        let empty = read_geo(&std::fs::read(FIXTURE_EMPTY).expect("fixture present")).expect("parses");
        // the reader materialises an empty `P`; compare the geometry, not that artefact
        assert_eq!((empty.point_count, empty.vertex_count(), empty.prim_count()), (0, 0, 0));
        assert!(cook(&[]).expect("cooks").p().is_empty() && empty.p().is_empty());
    }
}
