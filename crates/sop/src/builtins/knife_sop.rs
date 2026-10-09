//! Knife SOP (token `knife`): cut by a plane and keep both sides (default), with the seam marked.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Same plane parameters and geometry as `clip` (`clip_sop.rs`, one shared cutting core): `origin`, `dir`, `dist`, `keep` menu
//! 0 above / 1 below / 2 both (DEFAULT 2 here). Cut points are shared between the two halves, so the halves stay
//! welded along the seam. On top of `clip` the knife writes groups:
//! * `seamgroup` S `cut`: POINT group = the cut points plus kept source points lying ON the plane ("" = no group);
//! * `abovegroup` S `above` / `belowgroup` S `below`: PRIM groups of the pieces from the above / below pass ("" = none).
//!
//! An existing group of the same name and class is replaced; names must not contain spaces.
//! Box cut at y=0, both sides: 12 points (4 seam), 10 prims (5 above + 5 below).
//! QA params (OUR names, UNVERIFIED): `group` (prim group: only those prims are cut, the rest pass through un-welded, in neither side group),
//! `cap` (close each side's cut hole with one n-gon per rim loop: rim edges = plane edges used once by that side's prims; cap = the
//! reversed rim so each half is watertight on a closed mesh; cap prims join their side's group), `shatter` (un-weld the halves: the
//! below pass gets its own copies of the seam points, which stay in the seam group), `updatenormals` (point N recomputed).
//! NOT modelled (DEFERRED): `remove seam`, `keepedges`, the second-input cutting surface.
use super::clip_sop::{Cut, clip, plane_params, plane_specs};
use super::copytopoints_sop::subset;
use super::meshutil::{bad, gather, point_normals, positions, selection};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, Group, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    let mut v = plane_specs(2);
    v.push(ParamSpec::new("seamgroup", ParamValue::S("cut".into())).label("Seam Points Group"));
    v.push(ParamSpec::new("abovegroup", ParamValue::S("above".into())).label("Above Prims Group"));
    v.push(ParamSpec::new("belowgroup", ParamValue::S("below".into())).label("Below Prims Group"));
    v.push(ParamSpec::new("group", ParamValue::S(String::new())).label("Group"));
    v.push(ParamSpec::new("cap", ParamValue::B(false)).label("Cap Cut"));
    v.push(ParamSpec::new("shatter", ParamValue::B(false)).label("Shatter (un-weld halves)"));
    v.push(ParamSpec::new("updatenormals", ParamValue::B(false)).label("Update Normals"));
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
/// Rim loops of one side: directed plane edges (both ends in `seam`) of that side's closed polys with no reverse partner within
/// the side, chained into cycles and REVERSED (cap winding).
fn rim_loops(d: &Detail, seam: &[bool], side: &[bool]) -> Vec<Vec<u32>> {
    let on = |p: u32| seam.get(p as usize).copied().unwrap_or(false);
    let mut edges: HashSet<(u32, u32)> = HashSet::new();
    for (i, pr) in d.prims.iter().enumerate() {
        if !side.get(i).copied().unwrap_or(false) || !matches!(pr.kind, PrimKind::Poly { closed: true }) {
            continue;
        }
        let v: Vec<u32> = d.vertex_point.get(pr.verts.start as usize..pr.verts.end as usize).unwrap_or(&[]).to_vec();
        for k in 0..v.len() {
            let (a, b) = (v[k], v[(k + 1) % v.len()]);
            if a != b && on(a) && on(b) {
                edges.insert((a, b));
            }
        }
    }
    let mut next: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in &edges {
        if !edges.contains(&(b, a)) {
            next.entry(a).or_insert(b);
        }
    }
    let mut starts: Vec<u32> = next.keys().copied().collect();
    starts.sort_unstable();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut out = Vec::new();
    for s in starts {
        if seen.contains(&s) {
            continue;
        }
        let (mut cyc, mut cur) = (Vec::new(), s);
        let closed = loop {
            if !seen.insert(cur) {
                break cur == s;
            }
            cyc.push(cur);
            match next.get(&cur) {
                Some(&n) => cur = n,
                None => break false,
            }
        };
        if closed && cyc.len() >= 3 {
            cyc.reverse();
            out.push(cyc);
        }
    }
    out
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
        let p = ctx.params;
        let (cap, shatter, upd) = (
            p.toggle("cap").unwrap_or(false),
            p.toggle("shatter").unwrap_or(false),
            p.toggle("updatenormals").unwrap_or(false),
        );
        let sel = selection("knife", src, p.string("group").unwrap_or("").trim())?;
        let (work, rest): (Detail, Option<Detail>) = match &sel {
            Some(m) => (subset(src, m)?, Some(subset(src, &m.iter().map(|b| !b).collect::<Vec<_>>())?)),
            None => ((**src).clone(), None),
        };
        let Cut { mut detail, seam: mut sm, above: mut am, below: mut bm } = clip("knife", &work, o, n, keep)?;
        if cap {
            let (la, lb) = (rim_loops(&detail, &sm, &am), rim_loops(&detail, &sm, &bm));
            for (loops, above_side) in [(la, true), (lb, false)] {
                for l in loops {
                    detail.add_poly(&l, true);
                    am.push(above_side);
                    bm.push(!above_side);
                }
            }
        }
        if let Some(r) = &rest {
            detail.merge(r);
            sm.resize(detail.point_count as usize, false);
            am.resize(detail.prims.len(), false);
            bm.resize(detail.prims.len(), false);
        }
        if shatter {
            let ids: Vec<usize> = sm.iter().enumerate().filter(|(_, s)| **s).map(|(i, _)| i).collect();
            let idx: Vec<Option<usize>> = ids.iter().map(|&i| Some(i)).collect();
            let first = detail.point_count;
            for a in detail.attrs.iter_mut().filter(|a| a.class == AttrClass::Point) {
                let tail = gather(&a.data, &idx);
                a.data.extend_from(&tail);
            }
            for g in detail.groups.iter_mut().filter(|g| g.class == AttrClass::Point) {
                let tail: Vec<bool> = ids.iter().map(|&i| g.mask.get(i).copied().unwrap_or(false)).collect();
                g.mask.extend(tail);
            }
            detail.point_count += u32::try_from(ids.len()).map_err(|_| bad("knife: too many seam points"))?;
            let remap: HashMap<u32, u32> = ids.iter().enumerate().map(|(k, &i)| (i as u32, first + k as u32)).collect();
            for (i, pr) in detail.prims.iter().enumerate() {
                if bm.get(i).copied().unwrap_or(false) {
                    for v in
                        detail.vertex_point.get_mut(pr.verts.start as usize..pr.verts.end as usize).unwrap_or(&mut [])
                    {
                        if let Some(&n) = remap.get(v) {
                            *v = n;
                        }
                    }
                }
            }
            sm.extend(std::iter::repeat_n(true, ids.len()));
        }
        if upd {
            let pp = positions(&detail);
            let nn: Vec<[f32; 3]> =
                point_normals(&detail, &pp).into_iter().map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect();
            detail.add_attr(Attribute {
                name: "N".into(),
                class: AttrClass::Point,
                data: AttrData::V3(nn),
                type_info: TypeInfo::Normal,
            });
        }
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
            let ys = out.vertex_point[pr.verts.start as usize..pr.verts.end as usize]
                .iter()
                .map(|&v| out.p()[v as usize][1]);
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
        let again =
            cook(&*new(), (*cook(&*new(), cube(), &defaults(&*new())).expect("cooks")).clone(), &defaults(&*new()))
                .expect("cooks");
        assert_eq!(again.groups.iter().filter(|g| g.name == "cut").count(), 1);
    }

    fn params_on(over: &[(&str, ParamValue)]) -> ParamSet {
        let mut p = defaults(&*new());
        for (k, v) in over {
            p = p.with(k, v.clone());
        }
        p
    }
    fn volume(d: &Detail) -> f64 {
        let p = |i: u32| d.p()[i as usize].map(f64::from);
        let mut v = 0.0;
        for pr in &d.prims {
            let ids = &d.vertex_point[pr.verts.start as usize..pr.verts.end as usize];
            for k in 1..ids.len() - 1 {
                v += super::super::meshutil::tri_volume(p(ids[0]), p(ids[k]), p(ids[k + 1]));
            }
        }
        v
    }
    fn watertight(d: &Detail) -> bool {
        let mut e: HashMap<(u32, u32), u32> = HashMap::new();
        for pr in &d.prims {
            let ids = &d.vertex_point[pr.verts.start as usize..pr.verts.end as usize];
            for i in 0..ids.len() {
                *e.entry((ids[i], ids[(i + 1) % ids.len()])).or_default() += 1;
            }
        }
        e.iter().all(|(&(a, b), &c)| c == 1 && e.get(&(b, a)) == Some(&1))
    }
    /// Cube cut at y=0, keep above + cap: 5 + 1 prims, watertight, volume 1/2, the cap is the unit square facing -Y inside the `above` group.
    #[test]
    fn cap_closes_the_cut_with_one_outward_ngon_per_rim_loop() {
        let off = cook(&*new(), cube(), &with(0, 0.0)).expect("cooks");
        assert!(!watertight(&off), "uncapped half is open");
        let out = cook(&*new(), cube(), &params_on(&[("keep", ParamValue::Menu(0)), ("cap", ParamValue::B(true))]))
            .expect("cooks");
        assert_eq!(out.prim_count(), 6);
        assert!(watertight(&out));
        assert!((volume(&out).abs() - 0.5).abs() < 1e-6, "{}", volume(&out));
        let a = mask(&out, AttrClass::Prim, "above");
        assert_eq!(a, vec![true; 6]);
        let cap = out.prims.last().expect("cap");
        let ids = &out.vertex_point[cap.verts.start as usize..cap.verts.end as usize];
        assert_eq!(ids.len(), 4);
        assert!(ids.iter().all(|&i| out.p()[i as usize][1] == 0.0));
        // winding: Newell normal of the cap has y component of the same sign as the Houdini-outward -Y... check |ny| = 1 and area 1
        let q: Vec<[f64; 3]> = ids.iter().map(|&i| out.p()[i as usize].map(f64::from)).collect();
        let n = super::super::meshutil::newell_corners(&q);
        assert!((n[1].abs() * 0.5 - 1.0).abs() < 1e-6 && n[0].abs() < 1e-9 && n[2].abs() < 1e-9, "{n:?}");
        // keep both: each half gets its own cap -> 5 + 5 + 2
        let both = cook(&*new(), cube(), &params_on(&[("cap", ParamValue::B(true))])).expect("cooks");
        assert_eq!(both.prim_count(), 12);
        let (a, b) = (mask(&both, AttrClass::Prim, "above"), mask(&both, AttrClass::Prim, "below"));
        assert_eq!((a.iter().filter(|m| **m).count(), b.iter().filter(|m| **m).count()), (6, 6));
    }
    /// Shatter un-welds: 4 extra seam points (16), halves share no point, seam group has all 8 y=0 points, prims unchanged (10).
    #[test]
    fn shatter_unwelds_the_halves_along_the_seam() {
        let out = cook(&*new(), cube(), &params_on(&[("shatter", ParamValue::B(true))])).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (16, 10));
        let (a, b) = (mask(&out, AttrClass::Prim, "above"), mask(&out, AttrClass::Prim, "below"));
        let used = |side: &[bool], want: bool| -> HashSet<u32> {
            out.prims
                .iter()
                .enumerate()
                .filter(|(i, _)| side[*i] == want)
                .flat_map(|(_, p)| out.vertex_point[p.verts.start as usize..p.verts.end as usize].to_vec())
                .collect()
        };
        assert!(used(&a, true).is_disjoint(&used(&b, true)), "halves share points");
        let seam = mask(&out, AttrClass::Point, "cut");
        assert_eq!(seam.iter().filter(|s| **s).count(), 8);
        assert!(out.p().iter().zip(&seam).all(|(p, &s)| s == (p[1] == 0.0)));
    }
    /// Group filter: cut only prim 0..2 of the cube (the first 2 faces); unselected 4 faces pass through unchanged.
    #[test]
    fn group_filter_cuts_only_selected_prims() {
        let full = cook(&*new(), cube(), &defaults(&*new())).expect("cooks");
        let mut c = cube();
        c.groups.push(Group {
            name: "g".into(),
            class: AttrClass::Prim,
            mask: vec![true, true, false, false, false, false],
        });
        let out = cook(&*new(), c.clone(), &params_on(&[("group", ParamValue::S("g".into()))])).expect("cooks");
        // prims 0,1 are the x = -.5 / +.5 style faces or top/bottom depending on the cube; either way total = cut(selected) + 4 untouched
        let sel_cut = {
            let mut only = c.clone();
            only.prims.truncate(2);
            only.vertex_point.truncate(8);
            only.groups.clear();
            cook(&*new(), only, &defaults(&*new())).expect("cooks").prim_count()
        };
        assert_eq!(out.prim_count(), sel_cut + 4);
        assert!(out.prim_count() < full.prim_count() + 1 && sel_cut >= 2);
        let (a, b) = (mask(&out, AttrClass::Prim, "above"), mask(&out, AttrClass::Prim, "below"));
        assert_eq!(
            a.iter().chain(&b).filter(|m| **m).count(),
            sel_cut as usize,
            "pass-through prims are in neither group"
        );
        assert!(matches!(
            cook(&*new(), c, &params_on(&[("group", ParamValue::S("nope".into()))])),
            Err(CookError::Failed(_))
        ));
    }
    /// updatenormals: unit point N for every point of the capped half; off = none.
    #[test]
    fn updatenormals_writes_unit_point_normals() {
        assert!(cook(&*new(), cube(), &defaults(&*new())).expect("cooks").attr(AttrClass::Point, "N").is_none());
        let out = cook(&*new(), cube(), &params_on(&[("updatenormals", ParamValue::B(true))])).expect("cooks");
        let Some(AttrData::V3(n)) = out.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N") };
        assert_eq!(n.len(), 12);
        assert!(n.iter().all(|v| ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() - 1.0).abs() < 1e-4));
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
