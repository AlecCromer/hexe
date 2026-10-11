//! Split SOP (token `split`): one input, two outputs (selected / rest) of Houdini 20.5 `split`. The hexe graph has ONE output per node, so
//! the param `output` (0 = selected, 1 = rest; hexe-only) picks which of the two Houdini outputs this node cooks.
//!
//! # Semantics (hython 20.5 probes on a box, `fixtures/sop-split*`)
//! `group` = pattern over `grouptype` (0 guess, 3 points, 4 prims; guess = a named prim group, else a named point group, else prims).
//! EMPTY pattern = every element (out0 = all, out1 = nothing). `negate` swaps the outputs. Output 0 = the selection, output 1 = its complement.
//! * prim mode: output = those prims in source order; points = the points they use, in source order, renumbered (unused points removed).
//! * point mode: output = those points; every prim keeps only its vertices on a kept point (order kept), and a polygon left with fewer than
//!   3 vertices is dropped (an open one with fewer than 2). E.g. box points 0,1 -> `rest` = 6 points, 4 prims (two quads shortened to tris).
//!
//! Attributes and groups are carried by the same remap as the other re-topologising SOPs.
//!
//! Not modelled: `breakpoints` / `edges` grouptype (-> `Unsupported`), `removegrp`. A group name that does not exist is an error here
//! (Houdini silently selects nothing).
use super::group_sop::{parse_pattern, prim_verts};
use super::meshutil::{bad, failed, int, rebuild_w, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "split";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("grouptype", ParamValue::Menu(0)).label("Group Type").menu(&[
            ("guess", "Guess from Group"),
            ("breakpoints", "Breakpoints"),
            ("edges", "Edges"),
            ("points", "Points"),
            ("prims", "Primitives"),
        ]),
        ParamSpec::new("negate", ParamValue::B(false)).label("Negate"),
        ParamSpec::new("output", ParamValue::I(0)).label("Output (0 selected, 1 rest)").range(0.0, 1.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Split { specs: spec() })
}
struct Split {
    specs: Vec<ParamSpec>,
}
impl Node<Detail> for Split {
    fn type_name(&self) -> &'static str {
        OP
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let pat = p.string("group").ok_or_else(|| bad("`group`: missing or not a string"))?;
        let gt = p.menu("grouptype").ok_or_else(|| bad("`grouptype`: missing or not a menu"))?;
        let negate = toggle(p, "negate")?;
        let output = int(p, "output")?;
        if !(0..=1).contains(&output) {
            return Err(bad(format!("`output`: {output} outside 0..=1")));
        }
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let class = match gt {
            3 => AttrClass::Point,
            4 => AttrClass::Prim,
            0 => {
                let named = |c: AttrClass| d.groups.iter().any(|g| g.class == c && g.name == pat.trim());
                if !named(AttrClass::Prim) && named(AttrClass::Point) { AttrClass::Point } else { AttrClass::Prim }
            }
            _ => return Err(CookError::Unsupported("split: breakpoints / edges group types are not modelled".into())),
        };
        let n = d.class_len(class);
        let mut sel = parse_pattern(d, class, pat, OP)?.unwrap_or_else(|| vec![true; n]);
        // output 0 = selection (swapped by negate), output 1 = complement
        if (output == 1) != negate {
            sel.iter_mut().for_each(|b| *b = !*b);
        }
        let np = d.point_count as usize;
        let verts_of = |pr: &Prim| d.vertex_point.get(prim_verts(d, pr)).unwrap_or(&[]);
        let mut keep_pt = vec![false; np];
        let mut keep_prim = vec![false; d.prims.len()];
        if class == AttrClass::Point {
            keep_pt.clone_from(&sel);
            keep_pt.resize(np, false);
        } else {
            for (i, pr) in d.prims.iter().enumerate() {
                if sel.get(i).copied().unwrap_or(false) {
                    keep_prim[i] = true;
                    for &q in verts_of(pr) {
                        if let Some(k) = keep_pt.get_mut(q as usize) {
                            *k = true;
                        }
                    }
                }
            }
        }
        let mut remap = vec![u32::MAX; np];
        let mut point_w: Vec<Vec<(usize, f64)>> = Vec::new();
        let mut p_out: Vec<[f32; 3]> = Vec::new();
        for (q, _) in keep_pt.iter().enumerate().filter(|(_, k)| **k) {
            remap[q] = u32::try_from(point_w.len()).map_err(|_| failed(OP, "point id overflow"))?;
            point_w.push(vec![(q, 1.0)]);
            p_out.push(d.p()[q]);
        }
        let mut vertex_point: Vec<u32> = Vec::new();
        let mut vert_src: Vec<Option<usize>> = Vec::new();
        let mut prims: Vec<Prim> = Vec::new();
        let mut prim_src: Vec<Option<usize>> = Vec::new();
        for (i, pr) in d.prims.iter().enumerate() {
            if class == AttrClass::Prim && !keep_prim[i] {
                continue;
            }
            let base = prim_verts(d, pr).start;
            let kept: Vec<(usize, u32)> = verts_of(pr)
                .iter()
                .enumerate()
                .filter_map(|(k, &q)| remap.get(q as usize).copied().filter(|&r| r != u32::MAX).map(|r| (base + k, r)))
                .collect();
            let min = match pr.kind {
                PrimKind::Poly { closed: true } => 3,
                PrimKind::Poly { .. } | PrimKind::Polyline => 2,
                _ => 1,
            };
            if kept.len() < min || (class == AttrClass::Prim && kept.len() != verts_of(pr).len()) {
                continue;
            }
            let start = vertex_point.len() as u32;
            for (v, r) in kept {
                vertex_point.push(r);
                vert_src.push(Some(v));
            }
            prims.push(Prim { kind: pr.kind.clone(), verts: start..vertex_point.len() as u32 });
            prim_src.push(Some(i));
        }
        Ok(Arc::new(rebuild_w(OP, d, vertex_point, prims, &point_w, &vert_src, &prim_src, p_out)?))
    }
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cube, defaults, houdini_twin};
    use super::*;
    fn run(pat: &str, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let p =
            ov.iter().fold(defaults(&*n).with("group", ParamValue::S(pat.into())), |p, (k, v)| p.with(k, v.clone()));
        cook(&*n, cube(), &p)
    }
    #[test]
    fn prims_split_into_selected_and_rest() {
        let a = run("5", &[]).expect("cooks");
        let b = run("5", &[("output", ParamValue::I(1))]).expect("cooks");
        assert_eq!((a.point_count, a.prims.len(), b.point_count, b.prims.len()), (4, 1, 8, 5));
        let c = run("5", &[("negate", ParamValue::B(true))]).expect("cooks");
        assert_eq!((c.point_count, c.prims.len()), (8, 5));
    }
    #[test]
    fn point_mode_shortens_prims_and_drops_degenerate() {
        let pt = ParamValue::Menu(3);
        let rest = run("0 1", &[("grouptype", pt.clone()), ("output", ParamValue::I(1))]).expect("cooks");
        assert_eq!((rest.point_count, rest.prims.len()), (6, 4));
        let sel = run("0 1", &[("grouptype", pt)]).expect("cooks");
        assert_eq!((sel.point_count, sel.prims.len()), (2, 0));
    }
    #[test]
    fn empty_pattern_is_everything() {
        let a = run("", &[]).expect("cooks");
        let b = run("", &[("output", ParamValue::I(1))]).expect("cooks");
        assert_eq!((a.prims.len(), b.prims.len(), b.point_count), (6, 0, 0));
    }
    #[test]
    fn hostile_params() {
        assert!(run("0", &[("output", ParamValue::I(2))]).is_err());
        assert!(run("0", &[("grouptype", ParamValue::Menu(1))]).is_err());
        assert!(run("nope", &[]).is_err());
        assert!(run("99", &[]).is_ok());
        assert!(cook(&*new(), Detail::default(), &defaults(&*new())).is_ok());
    }
    macro_rules! twin {
        ($n:literal) => {
            houdini_twin(
                include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
            )
        };
    }
    /// Hexe twin == real Houdini 20.5 `.geo` of both split outputs (prim group, point group shortening, negate).
    #[test]
    fn split_matches_houdini_fixtures() {
        for n in [
            twin!("sop-split"),
            twin!("sop-split-rest"),
            twin!("sop-split-pts"),
            twin!("sop-split-pts-rest"),
            twin!("sop-split-negate"),
        ] {
            assert_eq!(n, Ok(()));
        }
    }
}
