//! color: DOC-VERIFIED against Houdini 20.5 help `sop/color.txt` (text only; no Houdini runtime).
//! Doc facts: writes `Cd` as point/prim/vertex/detail colour; Color Type Constant = same colour on the group, Bounding Box =
//! gradient by position in the bbox, Random = per-element colour (seeded); Delete All Existing Color Attributes removes Cd
//! of every class; Set Color Attribute off + delete = clear colour. Related recorded row: color == attribcreate Cd.
mod common;
use common::*;
use hexe_geo::{AttrClass, AttrData, Detail, TypeInfo};
use hexe_graph::ParamValue as PV;
use std::sync::Arc;

const CONSTANT: u32 = 0;
const RANDOM: u32 = 1;
const BBOX: u32 = 2;

#[test]
fn defaults_write_white_point_cd_with_color_typeinfo() {
    let c = cube();
    let o = cook("color", &[], Some(&c)).unwrap();
    let a = o.attr(AttrClass::Point, "Cd").expect("default class = point");
    assert_eq!(a.type_info, TypeInfo::Color);
    assert_eq!(v3(&o, AttrClass::Point, "Cd"), vec![[1.0; 3]; 8]);
    assert!(o.attr(AttrClass::Prim, "Cd").is_none());
    assert_eq!(o.prims, c.prims, "geometry untouched");
}

#[test]
fn constant_matches_attribcreate_cd_the_related_recorded_row() {
    let g = grid3();
    let col = cook("color", &[("color", PV::V3([0.2, 0.4, 0.6]))], Some(&g)).unwrap();
    let ac = cook(
        "attribcreate",
        &[
            ("name1", s("Cd")),
            ("type1", PV::Menu(2)),
            ("value1v1", PV::F(0.2)),
            ("value1v2", PV::F(0.4)),
            ("value1v3", PV::F(0.6)),
        ],
        Some(&g),
    )
    .unwrap();
    assert_eq!(col.attr(AttrClass::Point, "Cd"), ac.attr(AttrClass::Point, "Cd"), "same data + Color type info");
}

#[test]
fn classes_group_and_delete_all() {
    let g = grid3();
    let red = PV::V3([1.0, 0.0, 0.0]);
    // prim class, group limits to prim 1; others stay white
    let o = cook("color", &[("class", PV::Menu(2)), ("color", red.clone()), ("group", s("1"))], Some(&g)).unwrap();
    assert_eq!(v3(&o, AttrClass::Prim, "Cd"), [[1.0; 3], [1.0, 0.0, 0.0], [1.0; 3], [1.0; 3]]);
    // detail: exactly one colour; vertex: one per vertex
    let d = cook("color", &[("class", PV::Menu(3)), ("color", red.clone())], Some(&g)).unwrap();
    assert_eq!(v3(&d, AttrClass::Detail, "Cd"), [[1.0, 0.0, 0.0]]);
    let v = cook("color", &[("class", PV::Menu(1)), ("color", red.clone())], Some(&g)).unwrap();
    assert_eq!(v3(&v, AttrClass::Vertex, "Cd").len(), 16);
    // existing point Cd + write prim Cd: they coexist ("primitive and point attributes allow the same name")
    let both = cook("color", &[("class", PV::Menu(2)), ("color", red.clone())], Some(&o.clone())).unwrap();
    assert!(both.attr(AttrClass::Prim, "Cd").is_some());
    // delete-all removes every Cd class first
    let pt = cook("color", &[], Some(&g)).unwrap();
    let mixed = cook("color", &[("class", PV::Menu(2)), ("deleteallcolorattribs", PV::B(true))], Some(&pt)).unwrap();
    assert!(mixed.attr(AttrClass::Point, "Cd").is_none(), "point Cd deleted");
    assert!(mixed.attr(AttrClass::Prim, "Cd").is_some());
    // Set Color Attribute off + delete = clean slate
    let clean = cook("color", &[("deleteallcolorattribs", PV::B(true)), ("enable", PV::B(false))], Some(&pt)).unwrap();
    assert!(clean.attrs.iter().all(|a| a.name != "Cd"));
}

#[test]
fn bounding_box_gradient_normalises_position() {
    let g = grid3(); // x,y in 0..2, z flat
    let o = cook("color", &[("colortype", PV::Menu(BBOX))], Some(&g)).unwrap();
    let cd = v3(&o, AttrClass::Point, "Cd");
    for (i, c) in cd.iter().enumerate() {
        let p = g.p()[i];
        assert!(close(c[0], p[0] / 2.0) && close(c[1], p[1] / 2.0), "pt {i}: {c:?} for {p:?}");
        assert!((0.0..=1.0).contains(&c[2]));
    }
    assert_eq!(cd[0][..2], [0.0, 0.0]);
    assert_eq!(cd[8][..2], [1.0, 1.0]);
}

#[test]
fn random_is_seeded_per_element_and_in_range() {
    let c = cube();
    let r = |seed: f32| {
        v3(
            &cook("color", &[("colortype", PV::Menu(RANDOM)), ("seed", PV::F(seed))], Some(&c)).unwrap(),
            AttrClass::Point,
            "Cd",
        )
    };
    let (a, b, other) = (r(3.0), r(3.0), r(4.0));
    assert_eq!(a, b, "same seed -> same colours");
    assert_ne!(a, other, "seed changes colours");
    assert!(a.iter().flatten().all(|c| (0.0..=1.0).contains(c)));
    assert!(a.windows(2).any(|w| w[0] != w[1]), "each element gets its own colour");
    let _ = CONSTANT;
}

#[test]
fn empty_input_is_not_an_error() {
    let e = Arc::new(Detail::default());
    let o = cook("color", &[], Some(&e)).unwrap();
    assert_eq!(o.point_count, 0);
    assert!(matches!(o.attr(AttrClass::Point, "Cd").map(|a| &a.data), None | Some(AttrData::V3(_))));
    let d = cook("color", &[("class", PV::Menu(3))], Some(&e)).unwrap();
    assert_eq!(v3(&d, AttrClass::Detail, "Cd"), [[1.0; 3]], "detail colour exists even with no geometry");
}
