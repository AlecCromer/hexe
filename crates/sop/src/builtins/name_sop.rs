//! Name SOP (token `name`, Houdini 20.5 `name`): writes a string attribute (default `name`) on primitives / points / vertices.
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-name*`)
//! * The attribute `attribname` of `class` is ALWAYS created (string); an existing string attribute keeps the values of elements
//!   no rule touches, unset elements read `""`. An invalid attribute name (not `[A-Za-z_][A-Za-z0-9_]*`) passes the input through
//!   (Houdini warns, makes nothing); an empty one is `BadParam` (Houdini fails the cook).
//! * `donamefromgroup` (first): every group of `class` whose name matches a token of `namefromgroupmask` (`*`/`?` glob, default
//!   `piece*`; empty mask = nothing) names its members with the group name; groups apply in group order, later wins.
//! * Then `numnames` rows (1..=8) in order, later rows overwrite earlier ones: elements of pattern `group{i}` (empty = ALL) get `name{i}`.
//!   Patterns are the `groupcreate` grammar (ids, ranges, names, `!`, `^`, ...).
//! * `class` vertex: only an empty row pattern (= all vertices) is modelled; Houdini reads a vertex pattern as prim ranges -> `Unsupported`.
//!
//! Not modelled: the `numrenames` rename rows. UNVERIFIED: an existing NON-string attribute of that name (-> `Failed`).
use super::group_sop::glob_match;
use super::groupcreate_sop::eval_pattern;
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "name";
const MAX_ROWS: i32 = 8;

pub fn spec() -> Vec<ParamSpec> {
    let mut v = vec![
        ParamSpec::new("attribname", ParamValue::S("name".into())).label("Attribute"),
        ParamSpec::new("class", ParamValue::Menu(0)).label("Class").menu(&[
            ("primitive", "Primitive"),
            ("point", "Point"),
            ("vertex", "Vertex"),
        ]),
        ParamSpec::new("donamefromgroup", ParamValue::B(false)).label("Name from Group"),
        ParamSpec::new("namefromgroupmask", ParamValue::S("piece*".into())).label("Group Mask"),
        ParamSpec::new("numnames", ParamValue::I(1)).label("Number of Names").range(0.0, MAX_ROWS as f32),
    ];
    for i in 1..=MAX_ROWS {
        v.push(ParamSpec::new(&format!("group{i}"), ParamValue::S(String::new())).label("Group"));
        v.push(ParamSpec::new(&format!("name{i}"), ParamValue::S(String::new())).label("Name"));
    }
    v
}
pub fn new() -> Box<SopNode> {
    Box::new(NameSop { specs: spec() })
}
struct NameSop {
    specs: Vec<ParamSpec>,
}
fn bad(m: impl AsRef<str>) -> CookError {
    CookError::BadParam(format!("{OP}: {}", m.as_ref()))
}
fn valid_attr_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}
fn intern(table: &mut Vec<String>, s: &str) -> i32 {
    if s.is_empty() {
        return -1;
    }
    let pos = table.iter().position(|t| t == s).unwrap_or_else(|| {
        table.push(s.to_string());
        table.len() - 1
    });
    i32::try_from(pos).unwrap_or(-1)
}

impl Node<Detail> for NameSop {
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
        let miss = |n: &str| bad(format!("`{n}` missing or wrong type"));
        let attrib = p.string("attribname").ok_or_else(|| miss("attribname"))?.trim();
        let class = match p.menu("class").ok_or_else(|| miss("class"))? {
            0 => AttrClass::Prim,
            1 => AttrClass::Point,
            2 => AttrClass::Vertex,
            m => return Err(bad(format!("class menu index {m} out of range"))),
        };
        let by_group = p.toggle("donamefromgroup").ok_or_else(|| miss("donamefromgroup"))?;
        let mask = p.string("namefromgroupmask").ok_or_else(|| miss("namefromgroupmask"))?;
        let rows = p.int("numnames").ok_or_else(|| miss("numnames"))?;
        if !(0..=MAX_ROWS).contains(&rows) {
            return Err(bad(format!("numnames {rows} outside 0..={MAX_ROWS}")));
        }
        if attrib.is_empty() {
            return Err(bad("`attribname` is empty"));
        }
        let empty = Arc::new(Detail::default());
        let d: &Arc<Detail> = inputs.first().unwrap_or(&empty);
        if !valid_attr_name(attrib) {
            return Ok(Arc::clone(d));
        }
        let n = d.class_len(class);
        let (mut idx, mut table) = match d.attr(class, attrib) {
            Some(Attribute { data: AttrData::Str { idx, table }, .. }) => (idx.clone(), table.clone()),
            Some(_) => return Err(CookError::Failed(format!("{OP}: attribute `{attrib}` exists and is not a string"))),
            None => (Vec::new(), Vec::new()),
        };
        idx.resize(n, -1);
        if by_group {
            let toks: Vec<&str> = mask.split_whitespace().collect();
            for g in d.groups.iter().filter(|g| g.class == class && toks.iter().any(|t| glob_match(t, &g.name))) {
                let id = intern(&mut table, &g.name);
                for (slot, m) in idx.iter_mut().zip(&g.mask) {
                    if *m {
                        *slot = id;
                    }
                }
            }
        }
        for row in 1..=rows {
            let g = p.string(&format!("group{row}")).ok_or_else(|| miss(&format!("group{row}")))?;
            let name = p.string(&format!("name{row}")).ok_or_else(|| miss(&format!("name{row}")))?;
            let sel = if g.trim().is_empty() {
                vec![true; n]
            } else if class == AttrClass::Vertex {
                return Err(CookError::Unsupported(format!(
                    "{OP}: vertex-class group patterns (prim ranges) are not modelled"
                )));
            } else {
                eval_pattern(d, class, g, OP)?
            };
            let id = intern(&mut table, name);
            for (slot, m) in idx.iter_mut().zip(&sel) {
                if *m {
                    *slot = id;
                }
            }
        }
        let mut res = (**d).clone();
        res.add_attr(Attribute {
            name: attrib.to_string(),
            class,
            data: AttrData::Str { idx, table },
            type_info: TypeInfo::None,
        });
        Ok(Arc::new(res))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use super::*;

    fn nm(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        run(OP, ov, Some(d))
    }
    fn vals(d: &Detail, class: AttrClass, attr: &str) -> Vec<String> {
        let a = d.attr(class, attr).expect("attribute exists");
        let AttrData::Str { idx, table } = &a.data else { panic!("string attribute") };
        idx.iter().map(|&i| usize::try_from(i).ok().and_then(|u| table.get(u)).cloned().unwrap_or_default()).collect()
    }
    fn row(g: &str, n: &str) -> Vec<(&'static str, ParamValue)> {
        vec![("group1", s(g)), ("name1", s(n))]
    }
    /// The hython probe scene: box, prim groups piece1 = 0-1, piece2 = 4, point group ptg = 0-2.
    fn scene() -> Arc<Detail> {
        let mut d = cube();
        for (name, pat, ty) in [("piece1", "0-1", 0), ("piece2", "4", 0), ("ptg", "0-2", 1)] {
            let ov = [("groupname", s(name)), ("basegroup", s(pat)), ("grouptype", ParamValue::Menu(ty))];
            d = run("groupcreate", &ov, Some(&d)).expect("cooks");
        }
        d
    }
    fn prims(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Vec<String> {
        vals(&nm(d, ov).expect("cooks"), AttrClass::Prim, "name")
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), OP);
        let names: Vec<&str> = n.params().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(&names[..5], ["attribname", "class", "donamefromgroup", "namefromgroupmask", "numnames"]);
        assert_eq!(names.len(), 5 + 2 * 8);
    }
    /// hython: rows on the box prims.
    #[test]
    fn rows_match_hython() {
        let d = scene();
        let e = |v: &[&str]| v.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
        assert_eq!(prims(&d, &[]), e(&["", "", "", "", "", ""]), "empty row = attribute of empty strings");
        assert_eq!(prims(&d, &row("", "foo")), e(&["foo"; 6]));
        assert_eq!(prims(&d, &row("0-2", "a")), e(&["a", "a", "a", "", "", ""]));
        let two = [
            ("numnames", ParamValue::I(2)),
            ("group1", s("0-2")),
            ("name1", s("a")),
            ("group2", s("3-5")),
            ("name2", s("b")),
        ];
        assert_eq!(prims(&d, &two), e(&["a", "a", "a", "b", "b", "b"]));
        let over = [
            ("numnames", ParamValue::I(2)),
            ("group1", s("0-3")),
            ("name1", s("a")),
            ("group2", s("2-4")),
            ("name2", s("b")),
        ];
        assert_eq!(prims(&d, &over), e(&["a", "a", "b", "b", "b", ""]));
        let three = [
            ("numnames", ParamValue::I(3)),
            ("group1", s("0")),
            ("name1", s("a")),
            ("group2", s("")),
            ("name2", s("b")),
            ("group3", s("1")),
            ("name3", s("c")),
        ];
        assert_eq!(prims(&d, &three), e(&["b", "c", "b", "b", "b", "b"]));
        assert_eq!(prims(&d, &[("numnames", ParamValue::I(0))]), e(&[""; 6]));
        assert_eq!(prims(&d, &row("piece1", "x")), e(&["x", "x", "", "", "", ""]));
        assert_eq!(prims(&d, &row("piece*", "x")), e(&["x", "x", "", "", "x", ""]));
        assert_eq!(prims(&d, &row("!0-2", "x")), e(&["", "", "", "x", "x", "x"]));
        assert_eq!(prims(&d, &row("0", "a b")), e(&["a b", "", "", "", "", ""]));
        let foo = nm(&d, &[("attribname", s("foo")), ("group1", s("0")), ("name1", s("a"))]).expect("cooks");
        assert_eq!(vals(&foo, AttrClass::Prim, "foo")[..2], e(&["a", ""])[..]);
        assert!(foo.attr(AttrClass::Prim, "name").is_none());
    }
    #[test]
    fn classes_match_hython() {
        let d = scene();
        let pt = |extra: &[(&str, ParamValue)]| {
            let mut ov = vec![("class", ParamValue::Menu(1))];
            ov.extend(extra.iter().cloned());
            vals(&nm(&d, &ov).expect("cooks"), AttrClass::Point, "name")
        };
        assert_eq!(pt(&row("0-2", "p")).join(","), "p,p,p,,,,,");
        assert_eq!(pt(&row("ptg", "p")).join(","), "p,p,p,,,,,");
        assert_eq!(pt(&row("", "p")).join(","), "p,p,p,p,p,p,p,p");
        let v = nm(&d, &[("class", ParamValue::Menu(2)), ("group1", s("")), ("name1", s("v"))]).expect("cooks");
        assert_eq!(vals(&v, AttrClass::Vertex, "name"), vec!["v"; 24]);
        let vp = nm(&d, &[("class", ParamValue::Menu(2)), ("group1", s("0-1")), ("name1", s("v"))]);
        assert!(matches!(vp, Err(CookError::Unsupported(_))));
    }
    /// hython: `donamefromgroup` names members with the group name; rows apply after it; the mask filters.
    #[test]
    fn name_from_group_matches_hython() {
        let d = scene();
        let on = ("donamefromgroup", ParamValue::B(true));
        let join = |v: Vec<String>| v.join(",");
        assert_eq!(join(prims(&d, &[on.clone(), ("numnames", ParamValue::I(0))])), "piece1,piece1,,,piece2,");
        assert_eq!(
            join(prims(&d, &[on.clone(), ("namefromgroupmask", s("piece1")), ("numnames", ParamValue::I(0))])),
            "piece1,piece1,,,,"
        );
        let mut ov = vec![on.clone()];
        ov.extend(row("5", "z"));
        assert_eq!(join(prims(&d, &ov)), "piece1,piece1,,,piece2,z");
        assert_eq!(
            join(prims(&d, &[on.clone(), ("namefromgroupmask", s("")), ("numnames", ParamValue::I(0))])),
            ",,,,,"
        );
        let pts = nm(
            &d,
            &[on, ("class", ParamValue::Menu(1)), ("namefromgroupmask", s("ptg")), ("numnames", ParamValue::I(0))],
        )
        .expect("cooks");
        assert_eq!(vals(&pts, AttrClass::Point, "name").join(","), "ptg,ptg,ptg,,,,,");
    }
    /// hython: a second Name keeps the untouched values of the first.
    #[test]
    fn chained_names_keep_old_values() {
        let first = nm(&scene(), &row("0-2", "a")).expect("cooks");
        assert_eq!(prims(&first, &row("2-3", "b")).join(","), "a,a,b,b,,");
        assert_eq!(prims(&first, &[("numnames", ParamValue::I(0))]).join(","), "a,a,a,,,");
        assert_eq!(prims(&first, &row("", "c")).join(","), "c,c,c,c,c,c");
        let other = nm(&first, &[("attribname", s("other")), ("group1", s("0")), ("name1", s("q"))]).expect("cooks");
        assert_eq!(vals(&other, AttrClass::Prim, "other").join(","), "q,,,,,");
        assert_eq!(vals(&other, AttrClass::Prim, "name").join(","), "a,a,a,,,");
    }
    #[test]
    fn hostile_and_passthrough() {
        let d = scene();
        assert!(Arc::ptr_eq(&nm(&d, &[("attribname", s("1x y"))]).expect("cooks"), &d));
        assert!(matches!(nm(&d, &[("attribname", s(""))]), Err(CookError::BadParam(_))));
        assert!(matches!(nm(&d, &[("numnames", ParamValue::I(99))]), Err(CookError::BadParam(_))));
        assert!(matches!(nm(&d, &[("class", ParamValue::Menu(9))]), Err(CookError::BadParam(_))));
        assert!(matches!(nm(&d, &[("group1", ParamValue::I(1))]), Err(CookError::BadParam(_))));
        assert!(run(OP, &[], None).is_ok());
        let multi = nm(&d, &[("numnames", ParamValue::I(1)), ("group1", s("1,3"))]);
        assert!(matches!(multi, Err(CookError::Unsupported(_))));
    }
}
