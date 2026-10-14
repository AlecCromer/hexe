//! Group Expand SOP (token `groupexpand`, Houdini 20.5 `groupexpand`): grow a prim or point group by connectivity steps.
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-groupexpand*`; 5x5 point grid)
//! `grouptype`: guess (class of the named group: prim group first, else point) | vertices | edges | points | prims; vertices / edges ->
//! `Unsupported`. Each of `numsteps` rounds adds to the selection:
//! * prims: every prim sharing a POINT with a selected prim (`primshareedge` = sharing an EDGE, i.e. consecutive point pair; closed polys wrap);
//! * points: every point joined to a selected point by a prim edge.
//!
//! The result goes to group `outputgroup` (source untouched); `outputgroup` == `group` overwrites in place. `numsteps` 0 = plain copy.
//! EMPTY `group` = every element (the output holds everything). A group that does not exist for the class (missing name, or
//! a point group with `grouptype` prims) = passthrough, no output group (Houdini only warns).
//!
//! Not modelled (-> `Unsupported` when switched on): `floodfill`, `usestepattrib`, `bynormal`, `useconnectivityattrib`, `usecolgroup`.
use super::group_sop::{glob_match, prim_verts, set_group};
use super::groupcreate_sop::{eval_pattern, sanitize};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "groupexpand";
const MAX_STEPS: i32 = 10_000;
const OFF_TOGGLES: [&str; 5] = ["floodfill", "usestepattrib", "bynormal", "useconnectivityattrib", "usecolgroup"];
pub fn spec() -> Vec<ParamSpec> {
    let mut v = vec![
        ParamSpec::new("outputgroup", ParamValue::S("groupexpand1".into())).label("Output Group"),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("grouptype", ParamValue::Menu(0)).label("Group Type").menu(&[
            ("auto", "Guess from Group"),
            ("vertices", "Vertices"),
            ("edges", "Edges"),
            ("points", "Points"),
            ("prims", "Primitives"),
        ]),
        ParamSpec::new("primshareedge", ParamValue::B(false)).label("Prims Must Share Edge"),
        ParamSpec::new("numsteps", ParamValue::I(1)).label("Number of Steps").range(0.0, 100.0),
    ];
    for t in OFF_TOGGLES {
        v.push(ParamSpec::new(t, ParamValue::B(false)).label(t));
    }
    v
}
pub fn new() -> Box<SopNode> {
    Box::new(GroupExpand { specs: spec() })
}
struct GroupExpand {
    specs: Vec<ParamSpec>,
}
fn bad(m: &str) -> CookError {
    CookError::BadParam(format!("{OP}: {m}"))
}
/// Does `pat` name a group of `class` (non-numeric tokens only)?
fn names_group(d: &Detail, class: AttrClass, pat: &str) -> bool {
    pat.split_whitespace().map(|t| t.trim_start_matches(['!', '^'])).any(|t| {
        !t.starts_with(|c: char| c.is_ascii_digit() || c == '*')
            && d.groups.iter().any(|g| g.class == class && glob_match(t, &g.name))
    })
}
/// Undirected prim edges of every prim as `(low, high)`.
fn prim_edges(d: &Detail) -> Vec<Vec<(u32, u32)>> {
    d.prims
        .iter()
        .map(|pr| {
            let pts: Vec<u32> = d.vertex_point.get(prim_verts(d, pr)).map(<[u32]>::to_vec).unwrap_or_default();
            let closed = matches!(pr.kind, PrimKind::Poly { closed: true });
            let n = pts.len();
            let segs = if closed { n } else { n.saturating_sub(1) };
            (0..segs)
                .filter_map(|i| {
                    let (a, b) = (*pts.get(i)?, *pts.get((i + 1) % n)?);
                    (a != b).then_some((a.min(b), a.max(b)))
                })
                .collect()
        })
        .collect()
}
fn expand_prims(d: &Detail, mut sel: Vec<bool>, edge: bool, steps: i32) -> Vec<bool> {
    let pts: Vec<Vec<u32>> = d
        .prims
        .iter()
        .map(|pr| d.vertex_point.get(prim_verts(d, pr)).map(<[u32]>::to_vec).unwrap_or_default())
        .collect();
    let edges = prim_edges(d);
    for _ in 0..steps {
        let mut hot_pt = vec![false; d.point_count as usize];
        let mut hot_edge = std::collections::HashSet::new();
        for (i, on) in sel.iter().enumerate() {
            if !*on {
                continue;
            }
            if edge {
                hot_edge.extend(edges.get(i).into_iter().flatten().copied());
            } else {
                for &p in pts.get(i).into_iter().flatten() {
                    if let Some(h) = hot_pt.get_mut(p as usize) {
                        *h = true;
                    }
                }
            }
        }
        let mut next = sel.clone();
        for (i, n) in next.iter_mut().enumerate() {
            if *n {
                continue;
            }
            *n = if edge {
                edges.get(i).is_some_and(|e| e.iter().any(|x| hot_edge.contains(x)))
            } else {
                pts.get(i).is_some_and(|ps| ps.iter().any(|&p| hot_pt.get(p as usize).copied().unwrap_or(false)))
            };
        }
        if next == sel {
            break;
        }
        sel = next;
    }
    sel
}
fn expand_points(d: &Detail, mut sel: Vec<bool>, steps: i32) -> Vec<bool> {
    let edges: Vec<(u32, u32)> = {
        let mut e: Vec<(u32, u32)> = prim_edges(d).into_iter().flatten().collect();
        e.sort_unstable();
        e.dedup();
        e
    };
    for _ in 0..steps {
        let mut next = sel.clone();
        for &(a, b) in &edges {
            let (sa, sb) =
                (sel.get(a as usize).copied().unwrap_or(false), sel.get(b as usize).copied().unwrap_or(false));
            if sa {
                if let Some(n) = next.get_mut(b as usize) {
                    *n = true;
                }
            }
            if sb {
                if let Some(n) = next.get_mut(a as usize) {
                    *n = true;
                }
            }
        }
        if next == sel {
            break;
        }
        sel = next;
    }
    sel
}
impl Node<Detail> for GroupExpand {
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
        let pat = p.string("group").ok_or_else(|| bad("`group` missing or not a string"))?;
        let outname = sanitize(p.string("outputgroup").ok_or_else(|| bad("`outputgroup` missing or not a string"))?);
        let gt = p.menu("grouptype").ok_or_else(|| bad("`grouptype` missing or not a menu"))?;
        let share_edge = p.toggle("primshareedge").ok_or_else(|| bad("`primshareedge` missing or not a toggle"))?;
        let steps = p.int("numsteps").ok_or_else(|| bad("`numsteps` missing or not an int"))?;
        if !(0..=MAX_STEPS).contains(&steps) {
            return Err(bad(&format!("numsteps {steps} outside 0..={MAX_STEPS}")));
        }
        if outname.is_empty() {
            return Err(bad("`outputgroup` is empty"));
        }
        for t in OFF_TOGGLES {
            if p.toggle(t).unwrap_or(false) {
                return Err(CookError::Unsupported(format!("{OP}: `{t}` is not modelled")));
            }
        }
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let class = match gt {
            0 if names_group(d, AttrClass::Prim, pat) || pat.trim().is_empty() => AttrClass::Prim,
            0 => AttrClass::Point,
            3 => AttrClass::Point,
            4 => AttrClass::Prim,
            1 | 2 => return Err(CookError::Unsupported(format!("{OP}: vertex and edge groups are not modelled"))),
            m => return Err(bad(&format!("`grouptype` menu index {m} out of range"))),
        };
        let has_names = pat
            .split_whitespace()
            .any(|t| !t.trim_start_matches(['!', '^']).starts_with(|c: char| c.is_ascii_digit() || c == '*'));
        if has_names && !names_group(d, class, pat) {
            return Ok(Arc::clone(d));
        }
        let sel = if pat.trim().is_empty() { vec![true; d.class_len(class)] } else { eval_pattern(d, class, pat, OP)? };
        let res = match class {
            AttrClass::Prim => expand_prims(d, sel, share_edge, steps),
            _ => expand_points(d, sel, steps),
        };
        let mut out = (**d).clone();
        set_group(&mut out, class, &outname, res);
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::{mask, run, s};
    use super::*;
    fn grid5() -> Arc<Detail> {
        run("grid", &[("rows", ParamValue::I(5)), ("cols", ParamValue::I(5))], None).expect("grid")
    }
    fn seeded(class_pts: bool, pat: &str) -> Arc<Detail> {
        let ov = [("groupname", s("s")), ("grouptype", ParamValue::Menu(u32::from(class_pts))), ("basegroup", s(pat))];
        run("groupcreate", &ov, Some(&grid5())).expect("groupcreate")
    }
    fn ex(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Arc<Detail> {
        let mut all = vec![("group", s("s")), ("outputgroup", s("o"))];
        all.extend_from_slice(ov);
        run("groupexpand", &all, Some(d)).expect("cooks")
    }
    #[test]
    fn prim_expansion_by_point_and_by_edge() {
        let d = seeded(false, "5");
        let pt = ex(&d, &[("grouptype", ParamValue::Menu(4))]);
        assert_eq!(mask(&pt, AttrClass::Prim, "o"), [0, 1, 2, 4, 5, 6, 8, 9, 10]);
        assert_eq!(mask(&pt, AttrClass::Prim, "s"), [5], "source untouched");
        let edge = ex(&d, &[("grouptype", ParamValue::Menu(4)), ("primshareedge", ParamValue::B(true))]);
        assert_eq!(mask(&edge, AttrClass::Prim, "o"), [1, 4, 5, 6, 9]);
        let e2 = ex(&d, &[("primshareedge", ParamValue::B(true)), ("numsteps", ParamValue::I(2))]);
        assert_eq!(
            mask(&e2, AttrClass::Prim, "o"),
            [0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 13],
            "auto = prim group; 2 edge steps"
        );
        let p2 = ex(&d, &[("numsteps", ParamValue::I(2))]);
        assert_eq!(mask(&p2, AttrClass::Prim, "o").len(), 16);
    }
    #[test]
    fn point_expansion_follows_edges() {
        let d = seeded(true, "7");
        let one = ex(&d, &[("grouptype", ParamValue::Menu(3))]);
        assert_eq!(mask(&one, AttrClass::Point, "o"), [2, 6, 7, 8, 12]);
    }
    #[test]
    fn in_place_zero_steps_empty_group_and_passthrough() {
        let d = seeded(false, "5");
        let inplace = run(
            "groupexpand",
            &[("group", s("s")), ("outputgroup", s("s")), ("grouptype", ParamValue::Menu(4))],
            Some(&d),
        )
        .expect("cooks");
        assert_eq!(mask(&inplace, AttrClass::Prim, "s").len(), 9);
        assert_eq!(inplace.groups.len(), 1);
        assert_eq!(mask(&ex(&d, &[("numsteps", ParamValue::I(0))]), AttrClass::Prim, "o"), [5]);
        let all = run("groupexpand", &[("outputgroup", s("o")), ("grouptype", ParamValue::Menu(4))], Some(&d))
            .expect("cooks");
        assert_eq!(mask(&all, AttrClass::Prim, "o").len(), 16);
        assert!(ex(&d, &[("group", s("zz"))]).group(AttrClass::Prim, "o").is_none());
        let pts = seeded(true, "7");
        assert!(
            ex(&pts, &[("grouptype", ParamValue::Menu(4))]).group(AttrClass::Prim, "o").is_none(),
            "point group is not a prim group"
        );
    }
    #[test]
    fn unsupported_and_hostile() {
        let d = seeded(false, "5");
        for ov in
            [("grouptype", ParamValue::Menu(1)), ("grouptype", ParamValue::Menu(2)), ("floodfill", ParamValue::B(true))]
        {
            assert!(matches!(run("groupexpand", &[("group", s("s")), ov], Some(&d)), Err(CookError::Unsupported(_))));
        }
        assert!(run("groupexpand", &[("numsteps", ParamValue::I(-1))], Some(&d)).is_err());
        assert!(run("groupexpand", &[("outputgroup", s(""))], Some(&d)).is_err());
        assert_eq!(run("groupexpand", &[], None).expect("empty").point_count, 0);
    }
    /// Real Houdini 20.5 twins (5x5 grid, group s = prim 5 / point 7; expansion -> blast keeps `o`).
    #[test]
    fn groupexpand_matches_hython() {
        let cases: [(&str, &[u8]); 3] = [
            (
                include_str!("../../../../fixtures/sop-groupexpand/scene.json"),
                include_bytes!("../../../../fixtures/sop-groupexpand/houdini.geo"),
            ),
            (
                include_str!("../../../../fixtures/sop-groupexpand-edge/scene.json"),
                include_bytes!("../../../../fixtures/sop-groupexpand-edge/houdini.geo"),
            ),
            (
                include_str!("../../../../fixtures/sop-groupexpand-pts/scene.json"),
                include_bytes!("../../../../fixtures/sop-groupexpand-pts/houdini.geo"),
            ),
        ];
        for (scene, geo) in cases {
            super::super::meshutil::testkit::houdini_twin(scene, geo).expect("hexe == Houdini");
        }
    }
}
