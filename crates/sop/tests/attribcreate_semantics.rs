//! attribcreate: DOC-VERIFIED against Houdini 20.5 help `sop/attribcreate.txt` (text only; no Houdini runtime).
//! Doc facts: Class point/vertex/prim/detail; Type float/int/string (+ vector compat); type qualifier "Guess from name":
//! N -> Normal, Cd -> Color, v/up -> Vector, else None; Size = tuple size; Write Values off = attribute exists, values
//! untouched/default; Group = only members are written; later slots read the INPUT, not earlier slots.
mod common;
use common::*;
use hexe_geo::{AttrClass, AttrData, Attribute, TypeInfo};
use hexe_graph::ParamValue as PV;

fn m(i: u32) -> PV {
    PV::Menu(i)
}
const PRIM: u32 = 2;
const DETAIL: u32 = 3;
const FLOAT: u32 = 0;
const INT: u32 = 1;
const VECTOR: u32 = 2;
const STRING: u32 = 3;

#[test]
fn default_class_type_size_is_point_float_1_value_zero() {
    let g = grid3();
    let o = cook("attribcreate", &[("name1", s("foo"))], Some(&g)).unwrap();
    let a = o.attr(AttrClass::Point, "foo").expect("default class = point");
    assert_eq!(a.data, AttrData::F32(vec![0.0; 9]), "float, size 1, default 0 on every point");
    assert_eq!(a.type_info, TypeInfo::None);
}

#[test]
fn classes_types_and_sizes_with_values() {
    let g = grid3();
    // prim int (value rounds), size 1
    let o = cook(
        "attribcreate",
        &[("name1", s("id")), ("class1", m(PRIM)), ("type1", m(INT)), ("value1v1", PV::F(7.0))],
        Some(&g),
    )
    .unwrap();
    assert_eq!(o.attr(AttrClass::Prim, "id").unwrap().data, AttrData::I32(vec![7; 4]));
    // detail float size 2 => exactly one element
    let o = cook(
        "attribcreate",
        &[("name1", s("d")), ("class1", m(DETAIL)), ("size1", PV::I(2)), ("value1v1", PV::F(1.5)), ("value1v2", PV::F(2.5))],
        Some(&g),
    )
    .unwrap();
    assert_eq!(o.attr(AttrClass::Detail, "d").unwrap().data, AttrData::V2(vec![[1.5, 2.5]]));
    // vertex string
    let o = cook(
        "attribcreate",
        &[("name1", s("tag")), ("class1", m(1)), ("type1", m(STRING)), ("string1", s("hi"))],
        Some(&g),
    )
    .unwrap();
    match &o.attr(AttrClass::Vertex, "tag").unwrap().data {
        AttrData::Str { idx, table } => {
            assert_eq!(idx.len(), 16);
            assert!(idx.iter().all(|&i| table[i as usize] == "hi"));
        }
        d => panic!("{d:?}"),
    }
}

#[test]
fn type_qualifier_guess_from_name() {
    let g = grid3();
    let ti = |name: &str, ty: u32| {
        let o = cook(
            "attribcreate",
            &[("name1", s(name)), ("type1", m(ty)), ("size1", PV::I(3)), ("value1v1", PV::F(1.0))],
            Some(&g),
        )
        .unwrap();
        o.attr(AttrClass::Point, name).unwrap().type_info
    };
    assert_eq!(ti("N", FLOAT), TypeInfo::Normal);
    assert_eq!(ti("Cd", FLOAT), TypeInfo::Color);
    assert_eq!(ti("v", FLOAT), TypeInfo::Vector);
    assert_eq!(ti("up", FLOAT), TypeInfo::Vector);
    assert_eq!(ti("foo", FLOAT), TypeInfo::None);
    // Vector type = float + Vector qualifier (compat); N / Cd still guessed
    assert_eq!(ti("foo", VECTOR), TypeInfo::Vector);
    assert_eq!(ti("N", VECTOR), TypeInfo::Normal);
    assert_eq!(ti("Cd", VECTOR), TypeInfo::Color);
}

#[test]
fn write_values_off_keeps_defaults_and_group_limits_writes() {
    let g = grid3();
    let o = cook(
        "attribcreate",
        &[("name1", s("f")), ("value1v1", PV::F(5.0)), ("writevalues1", PV::B(false))],
        Some(&g),
    )
    .unwrap();
    assert_eq!(o.attr(AttrClass::Point, "f").unwrap().data, AttrData::F32(vec![0.0; 9]), "exists, not written");
    // group: only members written
    let o = cook("attribcreate", &[("name1", s("f")), ("value1v1", PV::F(5.0)), ("group", s("0-2"))], Some(&g)).unwrap();
    let AttrData::F32(v) = &o.attr(AttrClass::Point, "f").unwrap().data else { panic!() };
    assert_eq!(v, &[5.0, 5.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn existing_attribute_outside_group_keeps_its_values_and_two_slots_both_apply() {
    let mut d = (*grid3()).clone();
    d.add_attr(Attribute {
        name: "f".into(),
        class: AttrClass::Point,
        data: AttrData::F32((0..9).map(|i| i as f32).collect()),
        type_info: TypeInfo::None,
    });
    let g = std::sync::Arc::new(d);
    let o = cook(
        "attribcreate",
        &[
            ("numattr", PV::I(2)),
            ("name1", s("f")),
            ("value1v1", PV::F(-1.0)),
            ("group", s("4")),
            ("name2", s("g")),
            ("class2", m(DETAIL)),
            ("value2v1", PV::F(3.0)),
        ],
        Some(&g),
    )
    .unwrap();
    let AttrData::F32(v) = &o.attr(AttrClass::Point, "f").unwrap().data else { panic!() };
    assert_eq!(v, &[0.0, 1.0, 2.0, 3.0, -1.0, 5.0, 6.0, 7.0, 8.0]);
    assert_eq!(o.attr(AttrClass::Detail, "g").unwrap().data, AttrData::F32(vec![3.0]));
}

#[test]
fn empty_input_and_invalid_name() {
    let e = std::sync::Arc::new(hexe_geo::Detail::default());
    let o = cook("attribcreate", &[("name1", s("foo"))], Some(&e)).unwrap();
    assert_eq!(o.point_count, 0);
    assert_eq!(o.attr(AttrClass::Point, "foo").map(|a| a.data.len()).unwrap_or(0), 0);
    // detail attribute on empty geometry still gets its single element
    let o = cook("attribcreate", &[("name1", s("d")), ("class1", m(DETAIL)), ("value1v1", PV::F(1.0))], Some(&e)).unwrap();
    assert_eq!(o.attr(AttrClass::Detail, "d").unwrap().data, AttrData::F32(vec![1.0]));
}
