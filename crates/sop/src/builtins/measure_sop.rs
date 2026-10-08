//! Measure SOP (token `measure`): per-primitive area / perimeter / volume into a PRIM attribute.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `measure` menu: 0 `area` (default), 1 `perimeter`, 2 `volume`. `attribname` S: empty = the measure's own name
//! (`area` / `perimeter` / `volume`). Result = PRIM attribute, F32, one value per prim, computed in f64.
//! * area: closed poly = half the magnitude of the Newell vector sum (planar polys: exact area; non-planar: area of the
//!   best-fit projection). Open / non-poly prims = 0.
//! * perimeter: sum of edge lengths; closed polys include the closing edge, open polys / polylines do not. Others 0.
//! * volume: SIGNED contribution of a closed poly to the divergence-theorem volume w.r.t. the ORIGIN: `1/6 sum a.(b x c)`
//!   over the fan triangles (v0, vi, vi+1). Outward-wound closed meshes sum to their volume (unit box: each face 1/6,
//!   total 1); a lone prim has no volume of its own. Others 0.
//! `total` B (default off): also write the DETAIL attribute `<attribname>_sum` = sum over prims (f64 sum, then f32).
//! NOT modelled (DEFERRED): point / vertex measure entities, curvature, `smooth` / `dihedral` types, group filter, tetra /
//! volume-prim measures, N-gon holes.
use super::meshutil::{V3, bad, cross, dot, positions, sub};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const NAMES: [&str; 3] = ["area", "perimeter", "volume"];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("measure", ParamValue::Menu(0)).label("Measure"),
        ParamSpec::new("attribname", ParamValue::S(String::new())).label("Attribute Name"),
        ParamSpec::new("total", ParamValue::B(false)).label("Write Total (detail)"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Measure { specs: spec() })
}
struct Measure {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Measure {
    fn type_name(&self) -> &'static str {
        "measure"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let m = match ctx.params.menu("measure") {
            Some(m) if (m as usize) < NAMES.len() => m,
            Some(m) => return Err(bad(format!("measure: `measure` menu {m} out of range 0..=2"))),
            None => return Err(bad("measure: `measure`: missing or not a menu")),
        };
        let name = match ctx.params.string("attribname") {
            Some("") => NAMES.get(m as usize).copied().unwrap_or("area").to_owned(),
            Some(s) if !s.contains(char::is_whitespace) => s.to_owned(),
            _ => return Err(bad("measure: `attribname` must be a name without spaces")),
        };
        let total = ctx.params.toggle("total").ok_or_else(|| bad("measure: `total`: missing or not a toggle"))?;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = positions(src);
        let vals: Vec<f64> = src
            .prims
            .iter()
            .map(|pr| {
                let (lo, hi) = (pr.verts.start as usize, pr.verts.end as usize);
                let pts: Option<Vec<V3>> = src
                    .vertex_point
                    .get(lo..hi)
                    .and_then(|vs| vs.iter().map(|&p| pos.get(p as usize).copied()).collect());
                let Some(pts) = pts else { return 0.0 };
                let closed = matches!(pr.kind, PrimKind::Poly { closed: true });
                let open = matches!(pr.kind, PrimKind::Poly { closed: false } | PrimKind::Polyline);
                match m {
                    0 if closed => area(&pts),
                    1 if closed || open => perimeter(&pts, closed),
                    2 if closed => volume(&pts),
                    _ => 0.0,
                }
            })
            .collect();
        let mut out = (**src).clone();
        let sum: f64 = vals.iter().sum();
        out.add_attr(Attribute {
            name: name.clone(),
            class: AttrClass::Prim,
            data: AttrData::F32(vals.iter().map(|&v| v as f32).collect()),
            type_info: TypeInfo::None,
        });
        if total {
            out.add_attr(Attribute {
                name: format!("{name}_sum"),
                class: AttrClass::Detail,
                data: AttrData::F32(vec![sum as f32]),
                type_info: TypeInfo::None,
            });
        }
        Ok(Arc::new(out))
    }
}

fn area(p: &[V3]) -> f64 {
    let Some(&o) = p.first() else { return 0.0 };
    let mut nw = [0.0; 3];
    for w in p.windows(2) {
        let c = cross(sub(w[0], o), sub(w[1], o));
        nw = [nw[0] + c[0], nw[1] + c[1], nw[2] + c[2]];
    }
    0.5 * dot(nw, nw).sqrt()
}
fn perimeter(p: &[V3], closed: bool) -> f64 {
    let len = |a: V3, b: V3| {
        let d = sub(b, a);
        dot(d, d).sqrt()
    };
    let open: f64 = p.windows(2).map(|w| len(w[0], w[1])).sum();
    match (closed, p.first(), p.last()) {
        (true, Some(&a), Some(&b)) if p.len() > 1 => open + len(b, a),
        _ => open,
    }
}
fn volume(p: &[V3]) -> f64 {
    let Some(&a) = p.first() else { return 0.0 };
    p.windows(2).map(|w| dot(a, cross(w[0], w[1]))).sum::<f64>() / 6.0
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cube, defaults};
    use super::*;
    use hexe_graph::ParamSet;

    fn with(m: u32) -> ParamSet {
        defaults(&*new()).with("measure", ParamValue::Menu(m))
    }
    fn prim_f32(d: &Detail, n: &str) -> Vec<f32> {
        match d.attr(AttrClass::Prim, n).map(|a| &a.data) {
            Some(AttrData::F32(v)) => v.clone(),
            other => panic!("{n}: {other:?}"),
        }
    }
    fn scaled_cube(s: [f32; 3], t: f32) -> Detail {
        let mut d = cube();
        for p in d.p_mut().iter_mut() {
            *p = [p[0] * s[0] + t, p[1] * s[1] + t, p[2] * s[2] + t];
        }
        d
    }

    #[test]
    fn unit_box_every_face_has_area_1_and_perimeter_4() {
        let a = cook(&*new(), cube(), &with(0)).expect("cooks");
        assert_eq!(prim_f32(&a, "area"), vec![1.0; 6]);
        let p = cook(&*new(), cube(), &with(1)).expect("cooks");
        assert_eq!(prim_f32(&p, "perimeter"), vec![4.0; 6]);
    }

    #[test]
    fn box_volume_contributions_sum_to_the_volume_wherever_the_box_sits() {
        for (s, t, vol) in [([1.0, 1.0, 1.0], 0.0, 1.0), ([2.0, 3.0, 4.0], 0.0, 24.0), ([2.0, 3.0, 4.0], 10.0, 24.0)] {
            let p = with(2).with("total", ParamValue::B(true));
            let out = cook(&*new(), scaled_cube(s, t), &p).expect("cooks");
            let sum: f64 = prim_f32(&out, "volume").iter().map(|&v| f64::from(v)).sum();
            assert!((sum - vol).abs() < 1e-4, "{s:?}+{t}: {sum}");
            let Some(AttrData::F32(tot)) = out.attr(AttrClass::Detail, "volume_sum").map(|a| &a.data) else {
                panic!("volume_sum missing")
            };
            assert!((f64::from(tot[0]) - vol).abs() < 1e-4);
        }
        // origin-centred unit box: exactly 1/6 per face
        let out = cook(&*new(), cube(), &with(2)).expect("cooks");
        assert!(prim_f32(&out, "volume").iter().all(|&v| (v - 1.0 / 6.0).abs() < 1e-7));
        // inward-wound (flipped) box is negative
        let mut d = cube();
        for pr in d.prims.clone() {
            d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].reverse();
        }
        let out = cook(&*new(), d, &with(2)).expect("cooks");
        assert!(prim_f32(&out, "volume").iter().all(|&v| v < 0.0));
    }

    #[test]
    fn scaled_box_faces_and_right_triangle() {
        let a = cook(&*new(), scaled_cube([2.0, 3.0, 4.0], 0.0), &with(0)).expect("cooks");
        // faces in cube(): -x/+x? order [x-, x+...]: 2 faces of each pair; sorted areas = 2*(yz=12), 2*(xz=8), 2*(xy=6)
        let mut v = prim_f32(&a, "area");
        v.sort_by(|x, y| x.partial_cmp(y).expect("finite"));
        assert_eq!(v, vec![6.0, 6.0, 8.0, 8.0, 12.0, 12.0]);
        let mut t = Detail::default();
        t.add_points(3);
        *t.p_mut() = vec![[0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [0.0, 4.0, 0.0]];
        t.add_poly(&[0, 1, 2], true);
        assert_eq!(prim_f32(&cook(&*new(), t.clone(), &with(0)).expect("cooks"), "area"), vec![6.0]);
        assert_eq!(prim_f32(&cook(&*new(), t, &with(1)).expect("cooks"), "perimeter"), vec![12.0], "3+5+4");
    }

    #[test]
    fn open_polylines_have_length_but_no_area_and_names_are_configurable() {
        let mut d = Detail::default();
        d.add_points(3);
        *d.p_mut() = vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 2.0, 0.0]];
        d.add_poly(&[0, 1, 2], false);
        assert_eq!(prim_f32(&cook(&*new(), d.clone(), &with(1)).expect("cooks"), "perimeter"), vec![3.0]);
        assert_eq!(prim_f32(&cook(&*new(), d.clone(), &with(0)).expect("cooks"), "area"), vec![0.0]);
        let p = with(0).with("attribname", ParamValue::S("a2".into()));
        assert_eq!(prim_f32(&cook(&*new(), d, &p).expect("cooks"), "a2"), vec![0.0]);
    }

    #[test]
    fn hostile_inputs_do_not_panic() {
        for bad in [with(3), with(0).with("attribname", ParamValue::S("a b".into())), ParamSet::new()] {
            assert!(matches!(cook(&*new(), cube(), &bad), Err(CookError::BadParam(_))));
        }
        assert_eq!(cook(&*new(), Detail::default(), &with(0)).expect("cooks").prim_count(), 0);
        let mut d = cube();
        d.vertex_point.push(55);
        d.prims.push(hexe_geo::Prim { kind: PrimKind::Poly { closed: true }, verts: 24..25 });
        d.prims.push(hexe_geo::Prim { kind: PrimKind::Poly { closed: true }, verts: 90..99 });
        let out = cook(&*new(), d, &with(0)).expect("cooks");
        assert_eq!(prim_f32(&out, "area").len(), 8);
        assert_eq!(&prim_f32(&out, "area")[6..], &[0.0, 0.0]);
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/measure.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        assert_eq!(prim_f32(&d, "area"), vec![1.0; 6]);
        assert_eq!(crate::registry().create("measure").map(|n| n.type_name()), Some("measure"));
    }
}
