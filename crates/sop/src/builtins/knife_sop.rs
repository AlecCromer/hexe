//! Knife SOP (token `knife`): cut by a plane and keep both sides (default), with the seam marked.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Same plane parameters and geometry as `clip` (`clip_sop.rs`, one shared cutting core): `origin`, `dir`, `dist`, `keep` menu
//! 0 above / 1 below / 2 both (DEFAULT 2 here). Cut points are shared between the two halves, so the halves stay
//! welded along the seam. On top of `clip` the knife writes groups:
//! * `seamgroup` S `cut`: POINT group = the cut points plus kept source points lying ON the plane ("" = no group);
//! * `abovegroup` S `above` / `belowgroup` S `below`: PRIM groups of the pieces from the above / below pass ("" = none).
//! An existing group of the same name and class is replaced; names must not contain spaces.
//! Box cut at y=0, both sides: 12 points (4 seam), 10 prims (5 above + 5 below).
//! NOT modelled (DEFERRED): Houdini's `cap`/`remove seam`/`fill` modes, `keepedges`, group filter, `shatter`, the second-input
//! cutting surface, `N` recompute.
use super::clip_sop::{Cut, clip, plane_params, plane_specs};
use super::meshutil::bad;
use crate::SopNode;
use hexe_geo::{AttrClass, Detail, Group};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    let mut v = plane_specs(2);
    v.push(ParamSpec::new("seamgroup", ParamValue::S("cut".into())).label("Seam Points Group"));
    v.push(ParamSpec::new("abovegroup", ParamValue::S("above".into())).label("Above Prims Group"));
    v.push(ParamSpec::new("belowgroup", ParamValue::S("below".into())).label("Below Prims Group"));
    v
}
pub fn new() -> Box<SopNode> {
    Box::new(Knife { specs: spec() })
}
struct Knife {
    specs: Vec<ParamSpec>,
}

fn group_name(p: &hexe_graph::ParamSet, n: &str) -> Result<String, CookError> {
    match p.string(n) {
        Some(s) if !s.contains(char::is_whitespace) => Ok(s.to_owned()),
        _ => Err(bad(format!("knife: `{n}` must be a name without spaces"))),
    }
}
fn set_group(d: &mut Detail, name: &str, class: AttrClass, mask: Vec<bool>) {
    if name.is_empty() {
        return;
    }
    d.groups.retain(|g| !(g.class == class && g.name == name));
    d.groups.push(Group { name: name.to_owned(), class, mask });
}

impl Node<Detail> for Knife {
    fn type_name(&self) -> &'static str {
        "knife"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let (o, n, keep) = plane_params("knife", ctx.params)?;
        let (seam, above, below) = (
            group_name(ctx.params, "seamgroup")?,
            group_name(ctx.params, "abovegroup")?,
            group_name(ctx.params, "belowgroup")?,
        );
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let Cut { mut detail, seam: sm, above: am, below: bm } = clip("knife", src, o, n, keep)?;
        set_group(&mut detail, &seam, AttrClass::Point, sm);
        set_group(&mut detail, &above, AttrClass::Prim, am);
        set_group(&mut detail, &below, AttrClass::Prim, bm);
        Ok(Arc::new(detail))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cube, defaults};
    use super::*;
    use hexe_graph::ParamSet;

    fn with(keep: u32, y: f32) -> ParamSet {
        defaults(&*new()).with("keep", ParamValue::Menu(keep)).with("dist", ParamValue::F(y))
    }
    fn mask(d: &Detail, c: AttrClass, n: &str) -> Vec<bool> {
        d.group(c, n).map(|g| g.mask.clone()).unwrap_or_else(|| panic!("group {n}"))
    }

    /// Box cut at y=0 keeping both: 8 corners + 4 shared cut points = 12 points, 5 + 5 prims; seam = exactly the 4 y=0 points.
    #[test]
    fn box_cut_at_y0_keeps_both_halves_welded_with_marked_seam() {
        let out = cook(&*new(), cube(), &defaults(&*new())).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (12, 10));
        let seam = mask(&out, AttrClass::Point, "cut");
        assert_eq!(seam.len(), 12);
        for (k, p) in out.p().iter().enumerate() {
            assert_eq!(seam[k], p[1] == 0.0, "point {k} {p:?}");
        }
        assert_eq!(seam.iter().filter(|&&s| s).count(), 4);
        let (a, b) = (mask(&out, AttrClass::Prim, "above"), mask(&out, AttrClass::Prim, "below"));
        assert_eq!((a.iter().filter(|&&m| m).count(), b.iter().filter(|&&m| m).count()), (5, 5));
        assert!(a.iter().zip(&b).all(|(x, y)| x != y), "every prim is exactly one side");
        // above prims only touch y >= 0 points, below prims only y <= 0
        for (i, pr) in out.prims.iter().enumerate() {
            let ys = out.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&v| out.p()[v as usize][1]);
            assert!(if a[i] { ys.clone().all(|y| y >= 0.0) } else { ys.clone().all(|y| y <= 0.0) });
        }
    }

    #[test]
    fn seam_includes_on_plane_corners_and_keep_one_side_still_flags() {
        // plane on the top face: the 4 top corners are ON; nothing is split
        let out = cook(&*new(), cube(), &with(2, 0.5)).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (8, 6));
        let seam = mask(&out, AttrClass::Point, "cut");
        assert_eq!(seam.iter().filter(|&&s| s).count(), 4);
        assert!(out.p().iter().zip(&seam).all(|(p, &s)| s == (p[1] == 0.5)));
        // keep above only at y=0: 5 prims, all flagged above
        let up = cook(&*new(), cube(), &with(0, 0.0)).expect("cooks");
        assert_eq!(mask(&up, AttrClass::Prim, "above"), vec![true; 5]);
        assert_eq!(mask(&up, AttrClass::Prim, "below"), vec![false; 5]);
    }

    #[test]
    fn group_names_are_configurable_replaced_and_optional() {
        let p = defaults(&*new())
            .with("seamgroup", ParamValue::S("edge".into()))
            .with("abovegroup", ParamValue::S(String::new()))
            .with("belowgroup", ParamValue::S("lo".into()));
        let out = cook(&*new(), cube(), &p).expect("cooks");
        assert!(out.group(AttrClass::Point, "edge").is_some() && out.group(AttrClass::Point, "cut").is_none());
        assert!(out.group(AttrClass::Prim, "above").is_none());
        assert_eq!(mask(&out, AttrClass::Prim, "lo").iter().filter(|&&m| m).count(), 5);
        // cutting twice replaces rather than duplicates
        let again = cook(&*new(), (*cook(&*new(), cube(), &defaults(&*new())).expect("cooks")).clone(), &defaults(&*new())).expect("cooks");
        assert_eq!(again.groups.iter().filter(|g| g.name == "cut").count(), 1);
    }

    #[test]
    fn hostile_params() {
        for p in [
            defaults(&*new()).with("dir", ParamValue::V3([0.0; 3])),
            defaults(&*new()).with("keep", ParamValue::Menu(7)),
            defaults(&*new()).with("seamgroup", ParamValue::S("a b".into())),
            ParamSet::new(),
        ] {
            assert!(matches!(cook(&*new(), cube(), &p), Err(CookError::BadParam(_))));
        }
        assert_eq!(cook(&*new(), Detail::default(), &defaults(&*new())).expect("cooks").point_count, 0);
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/knife.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (12, 10));
        assert_eq!(crate::registry().create("knife").map(|n| n.type_name()), Some("knife"));
    }
}
