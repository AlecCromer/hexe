//! Attribute Create SOP (token `attribcreate`): creates (or overwrites) up to `MAX_SLOTS` attributes with constant values.
//!
//! # Semantics (DOC-VERIFIED vs Houdini 20.5 help `sop/attribcreate.txt`, tests/attribcreate_semantics.rs; no recorded row)
//! `numattr` (1, 0..=4) slots; slot `i` is `name{i}` `class{i}` (point | vertex | prim | detail) `type{i}` (float | int |
//! vector | string) `size{i}` (float / int tuple size 1..=4; vector is always 3) `value{i}v1..4` (numeric; int values round half away
//! from zero) `string{i}` and `writevalues{i}` (on; off creates the attribute with defaults only). A slot with an EMPTY
//! name is skipped (the default node is a no-op, input `Arc` shared). Names: one token, no whitespace.
//! `group` (all slots): pattern over the slot's class (point / vertex / prim; ignored for detail), grammar as in the
//! `group` SOP; empty = every element. Unselected elements keep the existing same-name attribute value when it has the
//! same shape, else the default (0 / unset string). Float size 2/3/4 -> `V2`/`V3`/`V4`, other sizes `FN`; int size 1
//! -> `I32` else `IN`. Type info = doc "Guess from name" for float / vector: size-3 `N` -> Normal, `Cd` (3|4) -> Color, `v` / `up` -> Vector, vector type -> Vector, else None; point `P` keeps Point and must be
//! a size-3 float / vector. Not modelled: `default{i}` multiparm, index (string-indexed) type, per-slot groups.
use super::attribs::{base_for, copy_elem, set_attr};
use super::group_sop::{bad, parse_pattern};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const WHO: &str = "attribcreate";
/// Slots declared in the parameter table (Houdini's multiparm is unbounded; the table here is static).
const MAX_SLOTS: i32 = 4;
const CLASSES: [(&str, &str); 4] =
    [("point", "Point"), ("vertex", "Vertex"), ("prim", "Primitive"), ("detail", "Detail")];
const TYPES: [(&str, &str); 4] = [("float", "Float"), ("int", "Integer"), ("vector", "Vector"), ("string", "String")];

pub fn spec() -> Vec<ParamSpec> {
    let mut v = vec![
        ParamSpec::new("numattr", ParamValue::I(1)).label("Number of Attributes").range(0.0, MAX_SLOTS as f32),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
    ];
    for i in 1..=MAX_SLOTS {
        v.push(ParamSpec::new(&format!("name{i}"), ParamValue::S(String::new())).label("Name"));
        v.push(ParamSpec::new(&format!("class{i}"), ParamValue::Menu(0)).label("Class").menu(&CLASSES));
        v.push(ParamSpec::new(&format!("type{i}"), ParamValue::Menu(0)).label("Type").menu(&TYPES));
        v.push(ParamSpec::new(&format!("size{i}"), ParamValue::I(1)).label("Size").range(1.0, 4.0));
        for c in 1..=4 {
            v.push(ParamSpec::new(&format!("value{i}v{c}"), ParamValue::F(0.0)).label("Value"));
        }
        v.push(ParamSpec::new(&format!("string{i}"), ParamValue::S(String::new())).label("String"));
        v.push(ParamSpec::new(&format!("writevalues{i}"), ParamValue::B(true)).label("Write Values"));
    }
    v
}

pub fn new() -> Box<SopNode> {
    Box::new(AttribCreate { specs: spec() })
}

struct AttribCreate {
    specs: Vec<ParamSpec>,
}

/// Round half away from zero into i32 (saturating).
fn to_int(v: f32) -> i32 {
    v.round() as i32
}

/// One-element attribute data holding the slot value.
fn element(ty: u32, size: usize, v: [f32; 4], s: &str) -> AttrData {
    match ty {
        0 => match size {
            1 => AttrData::F32(vec![v[0]]),
            2 => AttrData::V2(vec![[v[0], v[1]]]),
            3 => AttrData::V3(vec![[v[0], v[1], v[2]]]),
            4 => AttrData::V4(vec![v]),
            n => AttrData::FN { size: n as u32, data: v.iter().copied().take(n).collect() },
        },
        1 => match size {
            1 => AttrData::I32(vec![to_int(v[0])]),
            n => AttrData::IN { size: n as u32, data: v.iter().map(|x| to_int(*x)).take(n).collect() },
        },
        2 => AttrData::V3(vec![[v[0], v[1], v[2]]]),
        _ => AttrData::Str { idx: vec![0], table: vec![s.to_string()] },
    }
}

fn slot_class(m: u32) -> Result<AttrClass, CookError> {
    match m {
        0 => Ok(AttrClass::Point),
        1 => Ok(AttrClass::Vertex),
        2 => Ok(AttrClass::Prim),
        3 => Ok(AttrClass::Detail),
        m => Err(bad(WHO, format!("class menu index {m} out of range"))),
    }
}

impl Node<Detail> for AttribCreate {
    fn type_name(&self) -> &'static str {
        "attribcreate"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p: &ParamSet = ctx.params;
        let num = p.int("numattr").ok_or_else(|| bad(WHO, "`numattr` missing or not an integer"))?;
        if !(0..=MAX_SLOTS).contains(&num) {
            return Err(bad(WHO, format!("`numattr` must be 0..={MAX_SLOTS}")));
        }
        let pat = p.string("group").ok_or_else(|| bad(WHO, "`group` missing or not a string"))?;
        // validate + decode every slot up front (errors even on empty input)
        let mut slots = Vec::new();
        for i in 1..=num {
            let name = p
                .string(&format!("name{i}"))
                .ok_or_else(|| bad(WHO, format!("`name{i}` missing or not a string")))?
                .trim();
            if name.is_empty() {
                continue;
            }
            if name.contains(char::is_whitespace) {
                return Err(bad(WHO, format!("`name{i}` must be one token without whitespace")));
            }
            let menu =
                |k: &str| p.menu(&format!("{k}{i}")).ok_or_else(|| bad(WHO, format!("`{k}{i}` missing or not a menu")));
            let class = slot_class(menu("class")?)?;
            let ty = menu("type")?;
            if ty > 3 {
                return Err(bad(WHO, format!("`type{i}` menu index {ty} out of range")));
            }
            let size =
                p.int(&format!("size{i}")).ok_or_else(|| bad(WHO, format!("`size{i}` missing or not an integer")))?;
            if !(1..=4).contains(&size) {
                return Err(bad(WHO, format!("`size{i}` must be 1..=4")));
            }
            let mut v = [0.0f32; 4];
            for (c, slot) in v.iter_mut().enumerate() {
                *slot = p
                    .float(&format!("value{i}v{}", c + 1))
                    .ok_or_else(|| bad(WHO, format!("`value{i}v{}` missing", c + 1)))?;
            }
            let s = p
                .string(&format!("string{i}"))
                .ok_or_else(|| bad(WHO, format!("`string{i}` missing or not a string")))?;
            let write = p
                .toggle(&format!("writevalues{i}"))
                .ok_or_else(|| bad(WHO, format!("`writevalues{i}` missing or not a toggle")))?;
            let size = if ty == 2 { 3 } else { size as usize };
            if name == "P" && class == AttrClass::Point && !(ty != 1 && ty != 3 && size == 3) {
                return Err(bad(WHO, "point `P` must be a size-3 float / vector"));
            }
            if name == "P" && class != AttrClass::Point {
                return Err(bad(WHO, "`P` only exists on points"));
            }
            slots.push((name, class, ty, element(ty, size, v, s), write));
        }
        let Some(src) = inputs.first() else { return Ok(Arc::default()) };
        if slots.is_empty() {
            return Ok(Arc::clone(src));
        }
        let mut out = (**src).clone();
        for (name, class, ty, value, write) in slots {
            let n = out.class_len(class);
            let sel = if class == AttrClass::Detail { None } else { parse_pattern(&out, class, pat, WHO)? };
            let mut data = base_for(&out, class, name, &value);
            if write {
                for i in 0..n {
                    if sel.as_ref().is_none_or(|m| m.get(i).copied().unwrap_or(false)) {
                        copy_elem(&mut data, i, &value, 0);
                    }
                }
            }
            let type_info = match out.attr(class, name) {
                Some(x) if super::attribs::same_shape(&x.data, &data) => x.type_info,
                // doc "Guess from name": N -> Normal, Cd -> Color, v / up -> Vector, else None (float / vector types only)
                _ => match (name, ty, value.tuple_size()) {
                    ("P", _, _) => TypeInfo::Point,
                    ("N", 0 | 2, 3) => TypeInfo::Normal,
                    ("Cd", 0 | 2, 3 | 4) => TypeInfo::Color,
                    ("v" | "up", 0 | 2, 3) => TypeInfo::Vector,
                    (_, 2, _) => TypeInfo::Vector,
                    _ => TypeInfo::None,
                },
            };
            set_attr(&mut out, Attribute { name: name.to_string(), class, data, type_info });
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::{grid3, run, s};
    use super::*;
    use hexe_graph::ParamValue as PV;

    fn cook(d: &Arc<Detail>, ov: &[(&str, PV)]) -> Result<Arc<Detail>, CookError> {
        run("attribcreate", ov, Some(d))
    }
    fn attr<'a>(d: &'a Detail, c: AttrClass, n: &str) -> &'a Attribute {
        d.attr(c, n).unwrap_or_else(|| panic!("attribute {n}"))
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        let names: Vec<&str> = n.params().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(&names[..6], ["numattr", "group", "name1", "class1", "type1", "size1"]);
        assert!(names.contains(&"value4v4") && names.contains(&"writevalues2") && names.contains(&"string3"));
        assert_eq!(n.params().len(), 2 + 4 * 10);
        assert_eq!(n.params()[0].default, PV::I(1));
    }

    #[test]
    fn default_node_is_a_noop() {
        let g = grid3();
        assert!(Arc::ptr_eq(&cook(&g, &[]).expect("cooks"), &g));
        assert_eq!(run("attribcreate", &[("name1", s("x"))], None).expect("cooks").point_count, 0);
    }

    #[test]
    fn float_vector_int_string_and_detail_values() {
        let g = grid3();
        let o = cook(&g, &[("name1", s("w")), ("value1v1", PV::F(2.5))]).expect("cooks");
        assert_eq!(attr(&o, AttrClass::Point, "w").data, AttrData::F32(vec![2.5; 9]));
        let o = cook(
            &g,
            &[
                ("name1", s("Cd")),
                ("type1", PV::Menu(2)),
                ("value1v1", PV::F(1.0)),
                ("value1v2", PV::F(0.5)),
                ("value1v3", PV::F(0.25)),
            ],
        )
        .expect("cooks");
        let a = attr(&o, AttrClass::Point, "Cd");
        assert_eq!((&a.data, a.type_info), (&AttrData::V3(vec![[1.0, 0.5, 0.25]; 9]), TypeInfo::Color));
        let o =
            cook(&g, &[("name1", s("id")), ("class1", PV::Menu(2)), ("type1", PV::Menu(1)), ("value1v1", PV::F(3.5))])
                .expect("cooks");
        assert_eq!(attr(&o, AttrClass::Prim, "id").data, AttrData::I32(vec![4; 4]), "3.5 rounds away from zero");
        let o = cook(&g, &[("name1", s("tag")), ("class1", PV::Menu(1)), ("type1", PV::Menu(3)), ("string1", s("hi"))])
            .expect("cooks");
        let AttrData::Str { idx, table } = &attr(&o, AttrClass::Vertex, "tag").data else { panic!("str") };
        assert_eq!((idx.len(), idx.iter().all(|i| table[*i as usize] == "hi")), (16, true));
        let o = cook(&g, &[("name1", s("gain")), ("class1", PV::Menu(3)), ("value1v1", PV::F(7.0))]).expect("cooks");
        assert_eq!(attr(&o, AttrClass::Detail, "gain").data, AttrData::F32(vec![7.0]));
    }

    #[test]
    fn sizes_map_to_tuple_types() {
        let g = grid3();
        for (size, want) in [(2, 2u32), (3, 3), (4, 4)] {
            let o = cook(&g, &[("name1", s("f")), ("size1", PV::I(size))]).expect("cooks");
            assert_eq!(attr(&o, AttrClass::Point, "f").data.tuple_size(), want);
        }
        let o = cook(&g, &[("name1", s("k")), ("type1", PV::Menu(1)), ("size1", PV::I(3)), ("value1v3", PV::F(9.0))])
            .expect("cooks");
        assert_eq!(attr(&o, AttrClass::Point, "k").data, AttrData::IN { size: 3, data: [[0, 0, 9]; 9].concat() });
        // vector ignores size
        let o = cook(&g, &[("name1", s("v")), ("type1", PV::Menu(2)), ("size1", PV::I(1))]).expect("cooks");
        assert_eq!(attr(&o, AttrClass::Point, "v").data.tuple_size(), 3);
    }

    #[test]
    fn group_limits_writes_and_keeps_existing_values() {
        let g = grid3();
        let a = cook(&g, &[("name1", s("w")), ("value1v1", PV::F(1.0))]).expect("cooks");
        let b = cook(&a, &[("name1", s("w")), ("value1v1", PV::F(9.0)), ("group", s("0-1"))]).expect("cooks");
        assert_eq!(
            attr(&b, AttrClass::Point, "w").data,
            AttrData::F32(vec![9.0, 9.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0])
        );
        // fresh attribute, group only: outside = default 0
        let c = cook(&g, &[("name1", s("id")), ("class1", PV::Menu(2)), ("value1v1", PV::F(5.0)), ("group", s("3"))])
            .expect("cooks");
        assert_eq!(attr(&c, AttrClass::Prim, "id").data, AttrData::F32(vec![0.0, 0.0, 0.0, 5.0]));
        // writevalues off -> defaults only
        let d =
            cook(&g, &[("name1", s("z")), ("value1v1", PV::F(5.0)), ("writevalues1", PV::B(false))]).expect("cooks");
        assert_eq!(attr(&d, AttrClass::Point, "z").data, AttrData::F32(vec![0.0; 9]));
    }

    #[test]
    fn several_slots_and_overwrite_different_shape() {
        let g = grid3();
        let o = cook(
            &g,
            &[
                ("numattr", PV::I(2)),
                ("name1", s("a")),
                ("value1v1", PV::F(1.0)),
                ("name2", s("b")),
                ("class2", PV::Menu(2)),
                ("value2v1", PV::F(2.0)),
            ],
        )
        .expect("cooks");
        assert_eq!(attr(&o, AttrClass::Point, "a").data, AttrData::F32(vec![1.0; 9]));
        assert_eq!(attr(&o, AttrClass::Prim, "b").data, AttrData::F32(vec![2.0; 4]));
        // numattr 1 ignores slot 2
        let o = cook(&g, &[("name1", s("a")), ("name2", s("b"))]).expect("cooks");
        assert!(o.attr(AttrClass::Point, "b").is_none());
        // same name, new shape replaces
        let v = cook(&o, &[("name1", s("a")), ("size1", PV::I(3)), ("value1v1", PV::F(4.0))]).expect("cooks");
        assert_eq!(attr(&v, AttrClass::Point, "a").data.tuple_size(), 3);
        assert_eq!(v.attrs.iter().filter(|a| a.name == "a").count(), 1);
        // P stays P, keeps Point info
        let p = cook(&g, &[("name1", s("P")), ("type1", PV::Menu(2)), ("value1v3", PV::F(2.0))]).expect("cooks");
        assert!(p.p().iter().all(|q| q[2] == 2.0) && attr(&p, AttrClass::Point, "P").type_info == TypeInfo::Point);
    }

    #[test]
    fn hostile_params() {
        let g = grid3();
        let bad_p = |ov: &[(&str, PV)]| matches!(cook(&g, ov), Err(CookError::BadParam(_)));
        assert!(bad_p(&[("numattr", PV::I(5))]) && bad_p(&[("numattr", PV::I(-1))]));
        assert!(bad_p(&[("name1", s("a b"))]));
        assert!(bad_p(&[("name1", s("a")), ("size1", PV::I(0))]) && bad_p(&[("name1", s("a")), ("size1", PV::I(5))]));
        assert!(
            bad_p(&[("name1", s("a")), ("class1", PV::Menu(9))]) && bad_p(&[("name1", s("a")), ("type1", PV::Menu(9))])
        );
        assert!(
            bad_p(&[("name1", s("P")), ("type1", PV::Menu(1))]) && bad_p(&[("name1", s("P")), ("class1", PV::Menu(2))])
        );
        assert!(bad_p(&[("name1", PV::I(1))]));
        assert!(matches!(cook(&g, &[("name1", s("a")), ("group", s("nogroup"))]), Err(CookError::Failed(_))));
        assert_eq!(cook(&Arc::new(Detail::default()), &[("name1", s("a"))]).expect("cooks").point_count, 0);
    }

    #[test]
    fn scene_loader_and_registry_slot() {
        let ok = r#"{"nodes":[{"name":"b","type":"box"},{"name":"c","type":"attribcreate","params":{"name1":"Cd","type1":2,"value1v1":1}}],"edges":[["b",0,"c",0]],"display":"c"}"#;
        let (mut g, id) = crate::load_scene(ok).expect("loads");
        assert!(g.cook(id, 0.0).expect("cooks").attr(AttrClass::Point, "Cd").is_some());
        assert_eq!(crate::registry().create("attribcreate").map(|n| n.type_name()), Some("attribcreate"));
    }
}
