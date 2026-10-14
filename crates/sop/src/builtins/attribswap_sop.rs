//! Attribute Swap SOP (token `attribswap`, Houdini 20.5 `attribswap`): swap / copy / move whole attributes between two names.
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-attribswap*`)
//! `numswaps` rows (1..=4 modelled, applied in order); row `i`: `enable{i}`, `method{i}` swap | copy | move, `class{i}` detail | primitive | point |
//! vertex, `srcattribs{i}` / `dstattribs{i}` (ONE name or a `*`/`?` glob each: a space does NOT separate names, `h k` matches nothing;
//! globs resolve against the attribute names of the class, paired by index), `typeinfo{i}` copy source | preserve destination.
//! * swap: the two attributes exchange DATA and storage type (a float `h` swapped with an int `k` leaves `h` int and `k` float); names stay.
//!   Exactly one of the two missing: the existing one is COPIED to the missing name (both survive); both missing / same name / empty = no-op.
//! * copy: `dst` becomes a copy of `src` (created when missing); `src` unchanged. move: copy, then `src` removed (`P` is never removed).
//! * `P` takes part (`P` <-> `Cd` swaps positions and colour).
//! * typeinfo: preserve destination (Houdini default) keeps each NAME's type info (Color stays on `Cd`); copy source moves it with the data.
//!
//! UNVERIFIED: copy / move with a missing source (passthrough here), globs matching several names, vertex / detail classes.
use super::group_sop::glob_match;
use crate::SopNode;
use hexe_geo::{AttrClass, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "attribswap";
const MAX_ROWS: i32 = 4;
pub fn spec() -> Vec<ParamSpec> {
    let mut v = vec![ParamSpec::new("numswaps", ParamValue::I(1)).label("Number of Swaps").range(0.0, MAX_ROWS as f32)];
    for i in 1..=MAX_ROWS {
        v.push(ParamSpec::new(&format!("enable{i}"), ParamValue::B(true)).label("Enable"));
        v.push(ParamSpec::new(&format!("method{i}"), ParamValue::Menu(0)).label("Method").menu(&[
            ("swap", "Swap"),
            ("copy", "Copy"),
            ("move", "Move"),
        ]));
        v.push(ParamSpec::new(&format!("class{i}"), ParamValue::Menu(2)).label("Class").menu(&[
            ("detail", "Detail"),
            ("primitive", "Primitive"),
            ("point", "Point"),
            ("vertex", "Vertex"),
        ]));
        v.push(ParamSpec::new(&format!("srcattribs{i}"), ParamValue::S(String::new())).label("Source Attributes"));
        v.push(ParamSpec::new(&format!("dstattribs{i}"), ParamValue::S(String::new())).label("Destination Attributes"));
        v.push(
            ParamSpec::new(&format!("typeinfo{i}"), ParamValue::Menu(1))
                .label("Type Info")
                .menu(&[("source", "Copy Source"), ("dest", "Preserve Destination")]),
        );
    }
    v
}
pub fn new() -> Box<SopNode> {
    Box::new(AttribSwap { specs: spec() })
}
struct AttribSwap {
    specs: Vec<ParamSpec>,
}
fn bad(m: &str) -> CookError {
    CookError::BadParam(format!("{OP}: {m}"))
}
fn is_glob(s: &str) -> bool {
    s.contains(['*', '?'])
}
/// Names of `class` attributes matching the (single) pattern; a literal name resolves to itself even when missing.
fn resolve(d: &Detail, class: AttrClass, pat: &str) -> Vec<String> {
    if !is_glob(pat) {
        return vec![pat.to_string()];
    }
    d.attrs.iter().filter(|a| a.class == class && glob_match(pat, &a.name)).map(|a| a.name.clone()).collect()
}
fn idx(d: &Detail, class: AttrClass, name: &str) -> Option<usize> {
    d.attrs.iter().position(|a| a.class == class && a.name == name)
}
/// One src/dst pair.
fn apply(d: &mut Detail, class: AttrClass, method: u32, keep_dst_info: bool, src: &str, dst: &str) {
    if src == dst || src.is_empty() || dst.is_empty() {
        return;
    }
    let (si, di) = (idx(d, class, src), idx(d, class, dst));
    match (si, di, method) {
        (Some(s), Some(t), 0) => {
            let (a, b) = (d.attrs[s].data.clone(), d.attrs[t].data.clone());
            let (ia, ib) = (d.attrs[s].type_info, d.attrs[t].type_info);
            d.attrs[s].data = b;
            d.attrs[t].data = a;
            if !keep_dst_info {
                d.attrs[s].type_info = ib;
                d.attrs[t].type_info = ia;
            }
        }
        // swap with one side missing = copy of the existing one to the missing name
        (Some(_), None, 0 | 1 | 2) | (None, Some(_), 0) => {
            let from = si.or(di).unwrap_or(0);
            let to_name = if si.is_some() { dst } else { src };
            let mut a = d.attrs[from].clone();
            a.name = to_name.to_string();
            if method == 2 && si.is_some() && src != "P" {
                d.attrs.remove(from);
            }
            d.attrs.push(a);
        }
        (Some(s), Some(t), m) => {
            let data = d.attrs[s].data.clone();
            let info = d.attrs[s].type_info;
            d.attrs[t].data = data;
            if !keep_dst_info {
                d.attrs[t].type_info = info;
            }
            if m == 2 && src != "P" {
                d.attrs.remove(s);
            }
        }
        _ => {}
    }
}
impl Node<Detail> for AttribSwap {
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
        let rows = p.int("numswaps").ok_or_else(|| bad("`numswaps` missing or not an int"))?;
        if !(0..=MAX_ROWS).contains(&rows) {
            return Err(bad(&format!("numswaps {rows} outside 0..={MAX_ROWS}")));
        }
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let mut out = (**d).clone();
        for i in 1..=rows {
            let get = |k: &str, p: &ParamSet| -> Result<String, CookError> {
                p.string(&format!("{k}{i}"))
                    .map(str::to_string)
                    .ok_or_else(|| bad(&format!("`{k}{i}` missing or not a string")))
            };
            if !p.toggle(&format!("enable{i}")).ok_or_else(|| bad(&format!("`enable{i}` missing or not a toggle")))? {
                continue;
            }
            let method =
                p.menu(&format!("method{i}")).ok_or_else(|| bad(&format!("`method{i}` missing or not a menu")))?;
            if method > 2 {
                return Err(bad(&format!("`method{i}` menu index {method} out of range")));
            }
            let class =
                match p.menu(&format!("class{i}")).ok_or_else(|| bad(&format!("`class{i}` missing or not a menu")))? {
                    0 => AttrClass::Detail,
                    1 => AttrClass::Prim,
                    2 => AttrClass::Point,
                    3 => AttrClass::Vertex,
                    m => return Err(bad(&format!("`class{i}` menu index {m} out of range"))),
                };
            let keep_dst = match p
                .menu(&format!("typeinfo{i}"))
                .ok_or_else(|| bad(&format!("`typeinfo{i}` missing or not a menu")))?
            {
                0 => false,
                1 => true,
                m => return Err(bad(&format!("`typeinfo{i}` menu index {m} out of range"))),
            };
            let (sp, dp) = (get("srcattribs", p)?, get("dstattribs", p)?);
            let (sp, dp) = (sp.trim(), dp.trim());
            let (ss, ds) = (resolve(&out, class, sp), resolve(&out, class, dp));
            for (s, t) in ss.iter().zip(&ds) {
                apply(&mut out, class, method, keep_dst, s, t);
            }
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::{cube, s};
    use super::*;
    use hexe_geo::{AttrData, Attribute, TypeInfo};
    fn rich() -> Arc<Detail> {
        let mut d = (*cube()).clone();
        d.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3((0..8).map(|i| [i as f32, 0.5, 0.25]).collect()),
            type_info: TypeInfo::Color,
        });
        d.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[0.0, 1.0, 0.0]; 8]),
            type_info: TypeInfo::Normal,
        });
        d.add_attr(Attribute {
            name: "h".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![2.0; 8]),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "k".into(),
            class: AttrClass::Point,
            data: AttrData::I32(vec![7; 8]),
            type_info: TypeInfo::None,
        });
        Arc::new(d)
    }
    fn run(ov: &[(&str, ParamValue)], d: &Arc<Detail>) -> Arc<Detail> {
        super::super::group_sop::tt::run("attribswap", ov, Some(d)).expect("cooks")
    }
    fn cd(d: &Detail, n: &str) -> Vec<[f32; 3]> {
        match &d.attr(AttrClass::Point, n).expect(n).data {
            AttrData::V3(v) => v.clone(),
            _ => panic!("not V3"),
        }
    }
    #[test]
    fn swap_exchanges_data_and_keeps_type_info_by_name() {
        let d = rich();
        let o = run(&[("srcattribs1", s("Cd")), ("dstattribs1", s("N"))], &d);
        assert_eq!(cd(&o, "N"), cd(&d, "Cd"));
        assert_eq!(cd(&o, "Cd"), cd(&d, "N"));
        assert_eq!(o.attr(AttrClass::Point, "Cd").expect("Cd").type_info, TypeInfo::Color);
        let src = run(&[("srcattribs1", s("Cd")), ("dstattribs1", s("N")), ("typeinfo1", ParamValue::Menu(0))], &d);
        assert_eq!(
            src.attr(AttrClass::Point, "Cd").expect("Cd").type_info,
            TypeInfo::Normal,
            "copy source moves the info"
        );
        let mixed = run(&[("srcattribs1", s("h")), ("dstattribs1", s("k"))], &d);
        assert!(matches!(mixed.attr(AttrClass::Point, "h").expect("h").data, AttrData::I32(_)), "storage swaps too");
        assert!(matches!(mixed.attr(AttrClass::Point, "k").expect("k").data, AttrData::F32(_)));
        let glob = run(&[("srcattribs1", s("h*")), ("dstattribs1", s("k"))], &d);
        assert!(matches!(glob.attr(AttrClass::Point, "h").expect("h").data, AttrData::I32(_)));
        let p = run(&[("srcattribs1", s("Cd")), ("dstattribs1", s("P"))], &d);
        assert_eq!(p.p()[3], [3.0, 0.5, 0.25]);
        assert_eq!(cd(&p, "Cd")[3], d.p()[3]);
    }
    #[test]
    fn copy_move_and_missing_partner() {
        let d = rich();
        let c = run(&[("srcattribs1", s("Cd")), ("dstattribs1", s("N")), ("method1", ParamValue::Menu(1))], &d);
        assert_eq!(cd(&c, "N"), cd(&d, "Cd"));
        assert_eq!(cd(&c, "Cd"), cd(&d, "Cd"));
        let m = run(&[("srcattribs1", s("Cd")), ("dstattribs1", s("N")), ("method1", ParamValue::Menu(2))], &d);
        assert!(m.attr(AttrClass::Point, "Cd").is_none());
        assert_eq!(cd(&m, "N"), cd(&d, "Cd"));
        for method in [0, 1] {
            let n =
                run(&[("srcattribs1", s("Cd")), ("dstattribs1", s("nope")), ("method1", ParamValue::Menu(method))], &d);
            assert_eq!(cd(&n, "nope"), cd(&d, "Cd"), "copy / swap to a missing name copies");
            assert_eq!(cd(&n, "Cd"), cd(&d, "Cd"));
        }
        let miss = run(&[("srcattribs1", s("nope")), ("dstattribs1", s("Cd"))], &d);
        assert_eq!(cd(&miss, "nope"), cd(&d, "Cd"));
    }
    #[test]
    fn noops_and_rows() {
        let d = rich();
        for ov in [
            vec![("srcattribs1", s("Cd")), ("dstattribs1", s("Cd"))],
            vec![],
            vec![("srcattribs1", s("h k")), ("dstattribs1", s("k h"))],
            vec![("srcattribs1", s("Cd")), ("dstattribs1", s("N")), ("enable1", ParamValue::B(false))],
        ] {
            assert_eq!(*run(&ov, &d), *d);
        }
        let two = run(
            &[
                ("numswaps", ParamValue::I(2)),
                ("srcattribs1", s("Cd")),
                ("dstattribs1", s("N")),
                ("srcattribs2", s("N")),
                ("dstattribs2", s("Cd")),
            ],
            &d,
        );
        assert_eq!(cd(&two, "Cd"), cd(&d, "Cd"), "two swaps cancel");
    }
    #[test]
    fn hostile_params() {
        let d = rich();
        let r = |ov: &[(&str, ParamValue)]| super::super::group_sop::tt::run("attribswap", ov, Some(&d));
        assert!(r(&[("numswaps", ParamValue::I(9))]).is_err());
        assert!(r(&[("method1", ParamValue::Menu(7))]).is_err());
        assert!(r(&[("class1", ParamValue::Menu(7))]).is_err());
        assert_eq!(super::super::group_sop::tt::run("attribswap", &[], None).expect("empty").point_count, 0);
    }
    /// Real Houdini 20.5 twin: grid3 -> color -> normal -> attribswap Cd<->N (point) -> values (hython) equal.
    #[test]
    fn attribswap_matches_hython() {
        for (scene, geo) in [
            (
                include_str!("../../../../fixtures/sop-attribswap/scene.json"),
                &include_bytes!("../../../../fixtures/sop-attribswap/houdini.geo")[..],
            ),
            (
                include_str!("../../../../fixtures/sop-attribswap-move/scene.json"),
                &include_bytes!("../../../../fixtures/sop-attribswap-move/houdini.geo")[..],
            ),
        ] {
            super::super::meshutil::testkit::houdini_twin(scene, geo).expect("hexe == Houdini");
            super::super::meshutil::testkit::houdini_values(scene, geo, 1e-5).expect("values == Houdini");
        }
    }
}
