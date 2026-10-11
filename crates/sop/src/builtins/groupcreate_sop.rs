//! Group Create SOP (token `groupcreate`, Houdini 20.5 `groupcreate`; the legacy `group` SOP stays as-is) + the pattern
//! evaluator shared with `groupcombine`.
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-groupcreate*`)
//! Creates / edits ONE group `groupname` of class `grouptype` (primitive | point; edge + vertex -> `Unsupported`).
//! Name: whitespace and `*` `?` become `_` (`x y` -> `x_y`); an empty name passes the input through.
//! * `groupbase` (on) + `basegroup`: element pattern, empty = every element. Tokens (whitespace separated, UNIONED):
//!   `*`; `N`; `N-M` (reversed range accepted: `2-0` = 0-2); `N-M:step`; group name of the same class (`*`/`?` glob);
//!   `!X` = COMPLEMENT of X (unioned: `0-2 !1` = everything); `^X` = subtract X from the running set. An unknown name or an id
//!   past the end selects nothing (Houdini warns); a `,` token -> `Unsupported`. `groupbase` off with no other criterion = EMPTY group.
//! * `groupbounding`: `boundtype` bbox only (sphere / object / volume / hull -> `Unsupported`), box centre `t`, full extent `size`,
//!   faces INCLUSIVE, a negative extent selects nothing. Point = P inside. Prim = EVERY point inside, or ANY point with
//!   `includenotwhollycontained`. AND-ed with the base set (also when `groupbase` is off).
//! * `groupnormal`: keep elements whose normal is within `angle` degrees of `dir` (inclusive, 1e-6 slack; `oppositenormals` also
//!   accepts `-dir`). Prim normal = Newell, point = normalised sum of its prims' normals. Zero `dir` or no normal -> excluded
//!   (empty group, no error).
//! * `mergeop` vs the existing group of that class + name: replace | union | intersect | subtract. The group is always created.
//!
//! Not modelled: `geotype` filter, `ordered`, edge groups (`groupedges*`), `grouprandom*`, `camerapath`, `nonplanar`, vertex groups
//! (Houdini reads a vertex pattern as `prim:vtx` ranges), bracket globs `[bc]`, `,` increments.
use super::group_sop::{ANGLE_EPS, V3, dot, normals, prim_verts, set_group, token_set, unit, widen};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const GT_PRIM: u32 = 0;
const GT_POINT: u32 = 1;
const MERGE_UNION: u32 = 1;
const MERGE_INTERSECT: u32 = 2;
const MERGE_SUBTRACT: u32 = 3;
const OP: &str = "groupcreate";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("groupname", ParamValue::S("group1".into())).label("Group Name"),
        ParamSpec::new("grouptype", ParamValue::Menu(GT_PRIM)).label("Group Type").menu(&[
            ("primitive", "Primitives"),
            ("point", "Points"),
            ("edge", "Edges"),
            ("vertex", "Vertices"),
        ]),
        ParamSpec::new("mergeop", ParamValue::Menu(0)).label("Merge Operation").menu(&[
            ("replace", "Replace Existing"),
            ("union", "Union with Existing"),
            ("intersect", "Intersect with Existing"),
            ("subtract", "Subtract from Existing"),
        ]),
        ParamSpec::new("groupbase", ParamValue::B(true)).label("Base Group"),
        ParamSpec::new("basegroup", ParamValue::S(String::new())).label("Base Group Pattern"),
        ParamSpec::new("ordered", ParamValue::B(false)).label("Ordered"),
        ParamSpec::new("groupbounding", ParamValue::B(false)).label("Keep in Bounding Regions"),
        ParamSpec::new("boundtype", ParamValue::Menu(0)).label("Bounding Type").menu(&[
            ("usebbox", "Bounding Box"),
            ("usebsphere", "Bounding Sphere"),
            ("usebobject", "Bounding Object"),
            ("usebvolume", "Bounding Volume"),
            ("usebconvex", "Bounding Convex Hull"),
        ]),
        ParamSpec::new("size", ParamValue::V3([1.0, 1.0, 1.0])).label("Size").range(0.0, 10.0),
        ParamSpec::new("t", ParamValue::V3([0.0, 0.0, 0.0])).label("Center").range(-10.0, 10.0),
        ParamSpec::new("includenotwhollycontained", ParamValue::B(false)).label("Include by Not Wholly Contained"),
        ParamSpec::new("groupnormal", ParamValue::B(false)).label("Keep by Normals"),
        ParamSpec::new("dir", ParamValue::V3([0.0, 0.0, 1.0])).label("Direction").range(-1.0, 1.0),
        ParamSpec::new("angle", ParamValue::F(180.0)).label("Spread Angle").range(0.0, 180.0),
        ParamSpec::new("oppositenormals", ParamValue::B(false)).label("Opposite Normals"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(GroupCreate { specs: spec() })
}
struct GroupCreate {
    specs: Vec<ParamSpec>,
}

/// Group name as Houdini stores it: whitespace and glob characters become `_`.
pub(super) fn sanitize(name: &str) -> String {
    name.trim().chars().map(|c| if c.is_whitespace() || c == '*' || c == '?' { '_' } else { c }).collect()
}
fn numeric_token(body: &str, n: usize) -> Option<Vec<bool>> {
    let (range, step) = match body.split_once(':') {
        Some((r, s)) => (r, s.parse::<u64>().ok().filter(|s| *s > 0)?),
        None => (body, 1),
    };
    let (lo, hi) = match range.split_once('-') {
        Some((a, b)) => (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?),
        None => {
            let v = range.parse::<u64>().ok()?;
            (v, v)
        }
    };
    let (lo, hi) = if lo > hi { (hi, lo) } else { (lo, hi) };
    Some((0..n as u64).map(|i| i >= lo && i <= hi && (i - lo) % step == 0).collect())
}
fn one_token(d: &Detail, class: AttrClass, body: &str, n: usize, who: &str) -> Result<Vec<bool>, CookError> {
    if body.contains(',') {
        return Err(CookError::Unsupported(format!("{who}: `,` increments in patterns are not modelled")));
    }
    if body.starts_with(|c: char| c.is_ascii_digit())
        && let Some(set) = numeric_token(body, n)
    {
        return Ok(set);
    }
    match token_set(d, class, body, n, who) {
        Err(CookError::Failed(_)) => Ok(vec![false; n]),
        other => other,
    }
}
/// Houdini group pattern over `class` (module docs). Empty pattern -> empty set (callers decide what that means).
pub(super) fn eval_pattern(d: &Detail, class: AttrClass, pat: &str, who: &str) -> Result<Vec<bool>, CookError> {
    let n = d.class_len(class);
    let mut run = vec![false; n];
    for raw in pat.split_whitespace() {
        let (kind, body) = match raw.chars().next() {
            Some('!') => ('!', &raw[1..]),
            Some('^') => ('^', &raw[1..]),
            _ => (' ', raw),
        };
        if body.is_empty() {
            continue;
        }
        let set = one_token(d, class, body, n, who)?;
        for (r, s) in run.iter_mut().zip(&set) {
            match kind {
                '!' => *r |= !*s,
                '^' => *r &= !*s,
                _ => *r |= *s,
            }
        }
    }
    Ok(run)
}
fn bad(m: impl AsRef<str>) -> CookError {
    CookError::BadParam(format!("{OP}: {}", m.as_ref()))
}
/// Point ids of each element of `class` (point / prim).
fn element_points(d: &Detail, class: AttrClass) -> Vec<Vec<usize>> {
    match class {
        AttrClass::Point => (0..d.point_count as usize).map(|i| vec![i]).collect(),
        _ => d
            .prims
            .iter()
            .map(|pr| {
                d.vertex_point
                    .get(prim_verts(d, pr))
                    .map_or_else(Vec::new, |vs| vs.iter().map(|&v| v as usize).collect())
            })
            .collect(),
    }
}

impl Node<Detail> for GroupCreate {
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
        let name = sanitize(p.string("groupname").ok_or_else(|| miss("groupname"))?);
        let class = match p.menu("grouptype").ok_or_else(|| miss("grouptype"))? {
            GT_PRIM => AttrClass::Prim,
            GT_POINT => AttrClass::Point,
            2 | 3 => return Err(CookError::Unsupported(format!("{OP}: edge and vertex groups are not modelled"))),
            m => return Err(bad(format!("grouptype menu index {m} out of range"))),
        };
        let merge = p.menu("mergeop").ok_or_else(|| miss("mergeop"))?;
        if merge > MERGE_SUBTRACT {
            return Err(bad(format!("mergeop menu index {merge} out of range")));
        }
        let base = p.toggle("groupbase").ok_or_else(|| miss("groupbase"))?;
        let basegroup = p.string("basegroup").ok_or_else(|| miss("basegroup"))?;
        let bounding = p.toggle("groupbounding").ok_or_else(|| miss("groupbounding"))?;
        let use_normal = p.toggle("groupnormal").ok_or_else(|| miss("groupnormal"))?;
        if bounding && p.menu("boundtype").ok_or_else(|| miss("boundtype"))? != 0 {
            return Err(CookError::Unsupported(format!("{OP}: only the bounding box region is modelled")));
        }
        let size = p.vec3("size").ok_or_else(|| miss("size"))?;
        let centre = p.vec3("t").ok_or_else(|| miss("t"))?;
        let whole = !p.toggle("includenotwhollycontained").ok_or_else(|| miss("includenotwhollycontained"))?;
        let dir = p.vec3("dir").ok_or_else(|| miss("dir"))?;
        let angle = p.float("angle").ok_or_else(|| miss("angle"))?;
        let opposite = p.toggle("oppositenormals").ok_or_else(|| miss("oppositenormals"))?;
        if size.iter().chain(&centre).chain(&dir).any(|c| !c.is_finite()) || !angle.is_finite() {
            return Err(bad("non-finite vector component"));
        }
        let empty = Arc::new(Detail::default());
        let d: &Arc<Detail> = inputs.first().unwrap_or(&empty);
        if name.is_empty() {
            return Ok(Arc::clone(d));
        }
        let n = d.class_len(class);
        let mut new = match (base, basegroup.trim().is_empty()) {
            (true, false) => eval_pattern(d, class, basegroup, OP)?,
            (false, _) if !bounding && !use_normal => vec![false; n],
            _ => vec![true; n],
        };
        if bounding {
            let (c, h) = (widen(centre), widen(size));
            let pos = d.p();
            let inside = |i: usize| {
                pos.get(i).copied().map(widen).is_some_and(|q| (0..3).all(|k| (q[k] - c[k]).abs() <= h[k] * 0.5))
            };
            for (m, pts) in new.iter_mut().zip(element_points(d, class)) {
                let hit = !pts.is_empty()
                    && if whole { pts.iter().all(|&i| inside(i)) } else { pts.iter().any(|&i| inside(i)) };
                *m = *m && hit;
            }
        }
        if use_normal {
            let dir: Option<V3> = unit(widen(dir));
            let max = f64::from(angle) + ANGLE_EPS;
            for (m, nv) in new.iter_mut().zip(normals(d, class)) {
                let ok = match (nv, dir) {
                    (Some(nv), Some(dv)) => {
                        let a = dot(nv, dv).clamp(-1.0, 1.0).acos().to_degrees();
                        a <= max || (opposite && 180.0 - a <= max)
                    }
                    _ => false,
                };
                *m = *m && ok;
            }
        }
        let old: Vec<bool> = match d.group(class, &name) {
            Some(g) => (0..n).map(|i| g.mask.get(i).copied().unwrap_or(false)).collect(),
            None => vec![false; n],
        };
        let out: Vec<bool> = new
            .iter()
            .zip(&old)
            .map(|(&nw, &ol)| match merge {
                MERGE_UNION => ol || nw,
                MERGE_INTERSECT => ol && nw,
                MERGE_SUBTRACT => ol && !nw,
                _ => nw,
            })
            .collect();
        let mut res = (**d).clone();
        set_group(&mut res, class, &name, out);
        Ok(Arc::new(res))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use super::*;

    fn gc(ov: &[(&str, ParamValue)], d: &Arc<Detail>) -> Result<Arc<Detail>, CookError> {
        run(OP, ov, Some(d))
    }
    fn prim(d: &Detail) -> Vec<usize> {
        mask(d, AttrClass::Prim, "group1")
    }
    fn pt() -> ParamValue {
        ParamValue::Menu(GT_POINT)
    }
    fn on(k: &'static str) -> (&'static str, ParamValue) {
        (k, ParamValue::B(true))
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), OP);
        let names: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert!(names.starts_with(&["groupname", "grouptype", "mergeop", "groupbase", "basegroup"]));
        assert_eq!(
            n.params().iter().find(|s| s.name == "angle").map(|s| s.default.clone()),
            Some(ParamValue::F(180.0))
        );
    }
    /// hython: default / groupbase off / `0-2` / `!0-2` / `0 3-4` / `2-0` on the default box (prims and points).
    #[test]
    fn base_patterns_match_hython() {
        let b = cube();
        assert_eq!(prim(&gc(&[], &b).expect("cooks")), [0, 1, 2, 3, 4, 5]);
        assert!(prim(&gc(&[("groupbase", ParamValue::B(false))], &b).expect("cooks")).is_empty());
        let cases = [("0-2", vec![0, 1, 2]), ("!0-2", vec![3, 4, 5]), ("0 3-4", vec![0, 3, 4]), ("2-0", vec![0, 1, 2])];
        for (pat, want) in cases {
            assert_eq!(prim(&gc(&[("basegroup", s(pat))], &b).expect("cooks")), want, "{pat}");
        }
        let pts = gc(&[("grouptype", pt()), ("basegroup", s("!0-2"))], &b).expect("cooks");
        assert_eq!(mask(&pts, AttrClass::Point, "group1"), [3, 4, 5, 6, 7]);
    }
    /// hython: `!` is a UNIONED complement, `^` subtracts, steps, unknown names / out of range ids select nothing.
    #[test]
    fn pattern_corner_cases_match_hython() {
        let a = gc(&[("groupname", s("a")), ("basegroup", s("0-2"))], &cube()).expect("cooks");
        let sel = |pat: &str| {
            mask(&gc(&[("groupname", s("c")), ("basegroup", s(pat))], &a).expect("cooks"), AttrClass::Prim, "c")
        };
        assert_eq!(sel("0-2 !1"), [0, 1, 2, 3, 4, 5]);
        assert_eq!(sel("a !1"), [0, 1, 2, 3, 4, 5]);
        assert_eq!(sel("!a"), [3, 4, 5]);
        assert_eq!(sel("a 5"), [0, 1, 2, 5]);
        assert_eq!(sel("a ^1"), [0, 2]);
        assert_eq!(sel("0-5:2"), [0, 2, 4]);
        assert_eq!(sel("3-1 a"), [0, 1, 2, 3]);
        assert_eq!(sel("a nope"), [0, 1, 2]);
        assert!(sel("nope").is_empty() && sel("9").is_empty() && sel("-1").is_empty());
        assert_eq!(sel("0-100"), [0, 1, 2, 3, 4, 5]);
        assert_eq!(sel("a*"), [0, 1, 2]);
        assert!(matches!(gc(&[("basegroup", s("1,3"))], &a), Err(CookError::Unsupported(_))));
    }
    /// hython merge ops against an existing group `a` = 0-2 with a new set 1-4.
    #[test]
    fn mergeop_matches_hython() {
        let a = gc(&[("groupname", s("a")), ("basegroup", s("0-2"))], &cube()).expect("cooks");
        for (op, want) in [(0, vec![1, 2, 3, 4]), (1, vec![0, 1, 2, 3, 4]), (2, vec![1, 2]), (3, vec![0])] {
            let ov = [("groupname", s("a")), ("basegroup", s("1-4")), ("mergeop", ParamValue::Menu(op))];
            assert_eq!(mask(&gc(&ov, &a).expect("cooks"), AttrClass::Prim, "a"), want, "mergeop {op}");
        }
    }
    /// hython: box bbox (size 1.2/2/1.2 at x=.5): wholly -> prim 1, not-wholly -> 0 1 2 4 5, points 1 2 5 6; boundary inclusive.
    #[test]
    fn bounding_box_matches_hython() {
        let b = cube();
        let bb = |extra: &[(&'static str, ParamValue)]| -> Vec<(&'static str, ParamValue)> {
            let mut v = vec![
                on("groupbounding"),
                ("size", ParamValue::V3([1.2, 2.0, 1.2])),
                ("t", ParamValue::V3([0.5, 0.0, 0.0])),
            ];
            v.extend(extra.iter().cloned());
            v
        };
        let pts = |ov: &[(&str, ParamValue)]| mask(&gc(ov, &b).expect("cooks"), AttrClass::Point, "group1");
        assert_eq!(prim(&gc(&bb(&[]), &b).expect("cooks")), [1]);
        assert_eq!(prim(&gc(&bb(&[on("includenotwhollycontained")]), &b).expect("cooks")), [0, 1, 2, 4, 5]);
        assert_eq!(pts(&bb(&[("grouptype", pt())])), [1, 2, 5, 6]);
        assert_eq!(pts(&[on("groupbounding"), ("grouptype", pt())]).len(), 8, "boundary is inclusive");
        let shy = [on("groupbounding"), ("size", ParamValue::V3([0.9999, 1.0, 1.0])), ("grouptype", pt())];
        assert!(pts(&shy).is_empty());
        let neg = [on("groupbounding"), ("size", ParamValue::V3([-1.0, 1.0, 1.0])), ("grouptype", pt())];
        assert!(pts(&neg).is_empty());
        assert_eq!(prim(&gc(&bb(&[("groupbase", ParamValue::B(false))]), &b).expect("cooks")), [1]);
        let sphere = bb(&[("boundtype", ParamValue::Menu(1))]);
        assert!(matches!(gc(&sphere, &b), Err(CookError::Unsupported(_))));
    }
    /// hython: normals on the box (dir z default angle 180 = all; y10 -> prim 5; y90 -> 0 1 2 3 5; opposite 45 -> 4 5; dir 0 -> none).
    #[test]
    fn normals_match_hython() {
        let b = cube();
        let y = |angle: f32, extra: &[(&'static str, ParamValue)]| {
            let mut v =
                vec![on("groupnormal"), ("dir", ParamValue::V3([0.0, 1.0, 0.0])), ("angle", ParamValue::F(angle))];
            v.extend(extra.iter().cloned());
            v
        };
        assert_eq!(prim(&gc(&[on("groupnormal")], &b).expect("cooks")), [0, 1, 2, 3, 4, 5]);
        assert_eq!(prim(&gc(&y(10.0, &[]), &b).expect("cooks")), [5]);
        assert_eq!(prim(&gc(&y(90.0, &[]), &b).expect("cooks")), [0, 1, 2, 3, 5]);
        assert_eq!(prim(&gc(&y(45.0, &[on("oppositenormals")]), &b).expect("cooks")), [4, 5]);
        assert_eq!(prim(&gc(&y(180.0, &[]), &b).expect("cooks")), [0, 1, 2, 3, 4, 5]);
        let z = [on("groupnormal"), ("dir", ParamValue::V3([0.0; 3])), ("angle", ParamValue::F(10.0))];
        assert!(prim(&gc(&z, &b).expect("cooks")).is_empty());
        let p90 = gc(&y(90.0, &[("grouptype", pt())]), &b).expect("cooks");
        assert_eq!(mask(&p90, AttrClass::Point, "group1"), [4, 5, 6, 7]);
        let p10 = gc(&y(10.0, &[("grouptype", pt())]), &b).expect("cooks");
        assert!(mask(&p10, AttrClass::Point, "group1").is_empty());
    }
    #[test]
    fn names_and_unsupported_and_hostile() {
        let b = cube();
        let o = gc(&[("groupname", s("x y"))], &b).expect("cooks");
        assert!(o.group(AttrClass::Prim, "x_y").is_some());
        let o = gc(&[("groupname", s("a*"))], &b).expect("cooks");
        assert!(o.group(AttrClass::Prim, "a_").is_some());
        assert!(Arc::ptr_eq(&gc(&[("groupname", s(""))], &b).expect("cooks"), &b));
        for gt in [2, 3] {
            assert!(matches!(gc(&[("grouptype", ParamValue::Menu(gt))], &b), Err(CookError::Unsupported(_))));
        }
        assert!(matches!(gc(&[("grouptype", ParamValue::Menu(9))], &b), Err(CookError::BadParam(_))));
        assert!(matches!(gc(&[("angle", ParamValue::F(f32::NAN))], &b), Err(CookError::BadParam(_))));
        assert!(matches!(gc(&[("groupname", ParamValue::I(1))], &b), Err(CookError::BadParam(_))));
        assert!(run(OP, &[], None).is_ok());
        for pat in ["!", "^", "::", "1:0", "9-1:3", "\u{0}"] {
            assert!(gc(&[("basegroup", s(pat))], &b).is_ok(), "{pat:?}");
        }
        let huge = gc(&[("basegroup", s("0-99999999999999999999"))], &b);
        assert!(matches!(huge, Err(CookError::BadParam(_))), "u64 overflow is an error, not a wrap");
    }
}
