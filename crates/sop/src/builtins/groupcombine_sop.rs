//! Group Combine SOP (token `groupcombine`, Houdini 20.5 `groupcombine`): boolean expressions over existing groups.
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-groupcombine*`)
//! `numcombine` rows (1..=4 modelled), applied in order (later rows see earlier results). Row `i` writes group `group{i}` =
//! `A op_ab B op_bc C op_cd D`, folded LEFT TO RIGHT (no precedence: `a or b and c` = `(a or b) and c`). Operands `group_a/b/c/d{i}`
//! are element patterns exactly as in `groupcreate` (names + globs, ids, ranges, `!`, `^`); `negate_*` = `Equals All But` (complement).
//! Ops: none | or | and | xor | sub. `none` SKIPS its operand and the chain continues (`a none b or c` = `a or c`).
//! * `grouptype{i}`: guess (class of A: a prim group when A names one - `*` picks prims although point groups exist -, else a point group, else point) | points |
//!   prims; edges / vertices -> `Unsupported`. An operand naming no group of the class is empty (Houdini warns).
//! * Empty `group_a` or empty `group{i}` = row does nothing. An existing target group of that class is replaced.
//!
//! UNVERIFIED: guess order when A names both a point and a prim group, bracket globs `[bc]`, vertex groups.
use super::group_sop::{glob_match, set_group};
use super::groupcreate_sop::{eval_pattern, sanitize};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "groupcombine";
const MAX_ROWS: i32 = 4;
const OPS: [(&str, &str); 5] = [
    ("none", "None"),
    ("or", "Union (Or)"),
    ("and", "Intersect (And)"),
    ("xor", "Exclusive Or"),
    ("sub", "Subtraction"),
];

pub fn spec() -> Vec<ParamSpec> {
    let mut v =
        vec![ParamSpec::new("numcombine", ParamValue::I(1)).label("Number of Combines").range(1.0, MAX_ROWS as f32)];
    for i in 1..=MAX_ROWS {
        v.push(ParamSpec::new(&format!("grouptype{i}"), ParamValue::Menu(0)).label("Group Type").menu(&[
            ("guess", "Guess from Group"),
            ("points", "Points"),
            ("prims", "Primitives"),
            ("edges", "Edges"),
            ("vertices", "Vertices"),
        ]));
        v.push(ParamSpec::new(&format!("group{i}"), ParamValue::S(String::new())).label("Group"));
        for (k, letter) in ["a", "b", "c", "d"].into_iter().enumerate() {
            let (eq, with) = if k == 0 { ("Equals", "Equals All But") } else { ("With", "With All But") };
            v.push(
                ParamSpec::new(&format!("negate_{letter}{i}"), ParamValue::Menu(0))
                    .label("Negate")
                    .menu(&[("off", eq), ("on", with)]),
            );
            v.push(ParamSpec::new(&format!("group_{letter}{i}"), ParamValue::S(String::new())).label("Group"));
            if let Some(next) = ["b", "c", "d"].get(k) {
                v.push(
                    ParamSpec::new(&format!("op_{letter}{next}{i}"), ParamValue::Menu(0)).label("Operation").menu(&OPS),
                );
            }
        }
    }
    v
}
pub fn new() -> Box<SopNode> {
    Box::new(GroupCombine { specs: spec() })
}
struct GroupCombine {
    specs: Vec<ParamSpec>,
}
fn bad(m: impl AsRef<str>) -> CookError {
    CookError::BadParam(format!("{OP}: {}", m.as_ref()))
}
/// Does any non-numeric token of `pat` glob-match a group of `class`?
fn names_group(d: &Detail, class: AttrClass, pat: &str) -> bool {
    pat.split_whitespace().map(|t| t.trim_start_matches(['!', '^'])).any(|t| {
        !t.starts_with(|c: char| c.is_ascii_digit())
            && d.groups.iter().any(|g| g.class == class && glob_match(t, &g.name))
    })
}
fn operand(d: &Detail, class: AttrClass, p: &ParamSet, letter: &str, row: i32) -> Result<Vec<bool>, CookError> {
    let key = format!("group_{letter}{row}");
    let pat = p.string(&key).ok_or_else(|| bad(format!("`{key}` missing or not a string")))?;
    let neg_key = format!("negate_{letter}{row}");
    let neg = p.menu(&neg_key).ok_or_else(|| bad(format!("`{neg_key}` missing or not a menu")))?;
    if neg > 1 {
        return Err(bad(format!("`{neg_key}` menu index {neg} out of range")));
    }
    let mut set = eval_pattern(d, class, pat, OP)?;
    if neg == 1 {
        set.iter_mut().for_each(|s| *s = !*s);
    }
    Ok(set)
}

impl Node<Detail> for GroupCombine {
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
        let rows = p.int("numcombine").ok_or_else(|| bad("`numcombine` missing or not an int"))?;
        if !(0..=MAX_ROWS).contains(&rows) {
            return Err(bad(format!("numcombine {rows} outside 0..={MAX_ROWS}")));
        }
        let empty = Arc::new(Detail::default());
        let d: &Arc<Detail> = inputs.first().unwrap_or(&empty);
        let mut res: Option<Detail> = None;
        for row in 1..=rows {
            let cur: &Detail = res.as_ref().unwrap_or(d);
            let name = sanitize(
                p.string(&format!("group{row}")).ok_or_else(|| bad(format!("`group{row}` missing or not a string")))?,
            );
            let a_pat = p
                .string(&format!("group_a{row}"))
                .ok_or_else(|| bad(format!("`group_a{row}` missing or not a string")))?;
            if name.is_empty() || a_pat.trim().is_empty() {
                continue;
            }
            let class = match p
                .menu(&format!("grouptype{row}"))
                .ok_or_else(|| bad(format!("`grouptype{row}` missing or not a menu")))?
            {
                0 if names_group(cur, AttrClass::Prim, a_pat) => AttrClass::Prim,
                0 if names_group(cur, AttrClass::Point, a_pat) => AttrClass::Point,
                0 | 1 => AttrClass::Point,
                2 => AttrClass::Prim,
                3 | 4 => return Err(CookError::Unsupported(format!("{OP}: edge and vertex groups are not modelled"))),
                m => return Err(bad(format!("grouptype{row} menu index {m} out of range"))),
            };
            let mut set = operand(cur, class, p, "a", row)?;
            for (op_key, letter) in [("op_ab", "b"), ("op_bc", "c"), ("op_cd", "d")] {
                let k = format!("{op_key}{row}");
                let op = p.menu(&k).ok_or_else(|| bad(format!("`{k}` missing or not a menu")))?;
                if op == 0 {
                    continue;
                }
                if op > 4 {
                    return Err(bad(format!("`{k}` menu index {op} out of range")));
                }
                let other = operand(cur, class, p, letter, row)?;
                for (s, o) in set.iter_mut().zip(&other) {
                    *s = match op {
                        1 => *s || *o,
                        2 => *s && *o,
                        3 => *s != *o,
                        _ => *s && !*o,
                    };
                }
            }
            let mut next = cur.clone();
            set_group(&mut next, class, &name, set);
            res = Some(next);
        }
        Ok(res.map_or_else(|| Arc::clone(d), Arc::new))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use super::*;

    /// Box with prim groups a=0-2 b=2-4 c=4-5 and point groups pa=0-3 pb=2-5 (the hython probe scene).
    fn scene() -> Arc<Detail> {
        let mut d = cube();
        for (name, pat, ty) in [("a", "0-2", 0), ("b", "2-4", 0), ("c", "4-5", 0), ("pa", "0-3", 1), ("pb", "2-5", 1)] {
            let ov = [("groupname", s(name)), ("basegroup", s(pat)), ("grouptype", ParamValue::Menu(ty))];
            d = run("groupcreate", &ov, Some(&d)).expect("cooks");
        }
        d
    }
    fn comb(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        let mut all = vec![("group1", s("r"))];
        all.extend(ov.iter().cloned());
        run(OP, &all, Some(d))
    }
    fn r(d: &Detail) -> Vec<usize> {
        mask(d, AttrClass::Prim, "r")
    }
    fn m(i: u32) -> ParamValue {
        ParamValue::Menu(i)
    }

    #[test]
    fn params_follow_the_houdini_multiparm_names() {
        let n = new();
        let names: Vec<&str> = n.params().iter().map(|p| p.name.as_str()).collect();
        for want in [
            "numcombine",
            "grouptype1",
            "group1",
            "negate_a1",
            "group_a1",
            "op_ab1",
            "group_b1",
            "op_bc1",
            "op_cd1",
            "group_d4",
        ] {
            assert!(names.contains(&want), "{want}");
        }
        assert_eq!(names.len(), 1 + 4 * 13);
    }
    /// hython: a/b ops on a=0-2, b=2-4.
    #[test]
    fn binary_ops_match_hython() {
        let d = scene();
        for (op, want) in
            [(1, vec![0, 1, 2, 3, 4]), (2, vec![2]), (3, vec![0, 1, 3, 4]), (4, vec![0, 1]), (0, vec![0, 1, 2])]
        {
            let ov = [("group_a1", s("a")), ("op_ab1", m(op)), ("group_b1", s("b"))];
            assert_eq!(r(&comb(&d, &ov).expect("cooks")), want, "op {op}");
        }
        assert_eq!(r(&comb(&d, &[("group_a1", s("a"))]).expect("cooks")), [0, 1, 2]);
    }
    #[test]
    fn negate_and_chain_match_hython() {
        let d = scene();
        let go = |ov: &[(&str, ParamValue)]| r(&comb(&d, ov).expect("cooks"));
        assert_eq!(go(&[("negate_a1", m(1)), ("group_a1", s("a"))]), [3, 4, 5]);
        assert_eq!(
            go(&[("negate_a1", m(1)), ("group_a1", s("a")), ("op_ab1", m(1)), ("group_b1", s("b"))]),
            [2, 3, 4, 5]
        );
        assert_eq!(
            go(&[("group_a1", s("a")), ("op_ab1", m(1)), ("negate_b1", m(1)), ("group_b1", s("b"))]),
            [0, 1, 2, 5]
        );
        // left to right, no precedence
        let abc = |o1, o2| {
            go(&[
                ("group_a1", s("a")),
                ("op_ab1", m(o1)),
                ("group_b1", s("b")),
                ("op_bc1", m(o2)),
                ("group_c1", s("c")),
            ])
        };
        assert_eq!(abc(1, 2), [4]);
        assert_eq!(abc(2, 1), [2, 4, 5]);
        assert_eq!(abc(4, 1), [0, 1, 4, 5]);
        assert_eq!(abc(1, 4), [0, 1, 2, 3]);
        assert_eq!(abc(3, 3), [0, 1, 3, 5]);
        assert_eq!(abc(1, 0), [0, 1, 2, 3, 4]);
        assert_eq!(abc(0, 1), [0, 1, 2, 4, 5], "`none` skips its operand, the chain goes on");
        let four = [
            ("group_a1", s("a")),
            ("op_ab1", m(1)),
            ("group_b1", s("b")),
            ("op_bc1", m(1)),
            ("group_c1", s("c")),
            ("op_cd1", m(1)),
            ("group_d1", s("b c")),
        ];
        assert_eq!(go(&four), [0, 1, 2, 3, 4, 5]);
        assert_eq!(go(&[("group_a1", s("*")), ("op_ab1", m(4)), ("group_b1", s("b"))]), [0, 1, 5]);
        assert_eq!(go(&[("group_a1", s("a")), ("op_ab1", m(1)), ("group_b1", s("5"))]), [0, 1, 2, 5]);
    }
    /// hython: point groups via guess and via the explicit type; wrong class operands are empty; empty A / target do nothing.
    #[test]
    fn classes_and_empties_match_hython() {
        let d = scene();
        let pr = |o: &Detail| mask(o, AttrClass::Point, "r");
        let pts = [("group_a1", s("pa")), ("op_ab1", m(2)), ("group_b1", s("pb"))];
        let o = comb(&d, &pts).expect("cooks");
        assert_eq!(pr(&o), [2, 3]);
        assert!(o.group(AttrClass::Prim, "r").is_none());
        let typed = comb(&d, &[("grouptype1", m(1)), ("group_a1", s("pa")), ("op_ab1", m(2)), ("group_b1", s("pb"))])
            .expect("cooks");
        assert_eq!(pr(&typed), [2, 3]);
        let prims = comb(&d, &[("grouptype1", m(2)), ("group_a1", s("pa")), ("op_ab1", m(1)), ("group_b1", s("b"))])
            .expect("cooks");
        assert_eq!(r(&prims), [2, 3, 4]);
        let mixed = comb(&d, &[("group_a1", s("a")), ("op_ab1", m(1)), ("group_b1", s("pa"))]).expect("cooks");
        assert_eq!(r(&mixed), [0, 1, 2]);
        let missing = comb(&d, &[("group_a1", s("zzz")), ("op_ab1", m(1)), ("group_b1", s("b"))]).expect("cooks");
        assert!(mask(&missing, AttrClass::Point, "r").is_empty() && missing.group(AttrClass::Prim, "r").is_none());
        for ov in [
            vec![("group_a1", s("")), ("op_ab1", m(1)), ("group_b1", s("b"))],
            vec![("group1", s("")), ("group_a1", s("a"))],
        ] {
            let o = comb(&d, &ov).expect("cooks");
            assert_eq!(o.groups.len(), d.groups.len());
        }
    }
    /// hython: two rows, row 2 reads row 1's result.
    #[test]
    fn rows_chain_and_overwrite() {
        let d = scene();
        let two = [
            ("numcombine", ParamValue::I(2)),
            ("group_a1", s("a")),
            ("op_ab1", m(1)),
            ("group_b1", s("b")),
            ("group2", s("s")),
            ("group_a2", s("r")),
            ("op_ab2", m(4)),
            ("group_b2", s("c")),
        ];
        let o = comb(&d, &two).expect("cooks");
        assert_eq!(r(&o), [0, 1, 2, 3, 4]);
        assert_eq!(mask(&o, AttrClass::Prim, "s"), [0, 1, 2, 3]);
        let over =
            run(OP, &[("group1", s("a")), ("group_a1", s("b")), ("op_ab1", m(1)), ("group_b1", s("c"))], Some(&d))
                .expect("cooks");
        assert_eq!(mask(&over, AttrClass::Prim, "a"), [2, 3, 4, 5]);
        assert_eq!(over.groups.len(), d.groups.len());
    }
    #[test]
    fn unsupported_and_hostile() {
        let d = scene();
        let base = [("group_a1", s("a"))];
        for ty in [3, 4] {
            let mut ov = base.to_vec();
            ov.push(("grouptype1", m(ty)));
            assert!(matches!(comb(&d, &ov), Err(CookError::Unsupported(_))));
        }
        for bad_ov in [
            ("numcombine", ParamValue::I(99)),
            ("numcombine", ParamValue::I(-1)),
            ("op_ab1", m(9)),
            ("grouptype1", m(9)),
            ("negate_a1", m(5)),
            ("group_a1", ParamValue::I(1)),
        ] {
            let mut ov = base.to_vec();
            ov.push(("op_ab1", m(1)));
            ov.push(bad_ov);
            assert!(matches!(comb(&d, &ov), Err(CookError::BadParam(_))), "{:?}", ov.last());
        }
        assert!(Arc::ptr_eq(&run(OP, &[("numcombine", ParamValue::I(0))], Some(&d)).expect("cooks"), &d));
        assert!(run(OP, &[], None).is_ok());
    }
}
