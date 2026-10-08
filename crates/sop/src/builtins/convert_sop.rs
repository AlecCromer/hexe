//! Convert SOP (token `convert`): change primitive representation. Polygon <-> polyline, analytic sphere / tube -> polygons.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! * `fromtype` menu `all` (0, default) | `poly` (1) | `polyline` (2) | `sphere` (3) | `tube` (4): only prims of that kind
//!   are converted, everything else passes through untouched. `totype` menu `polygon` (0, default) | `polyline` (1).
//! * `totype = polygon`: a `Polyline` prim becomes a `Poly`: if its last corner repeats its first (>= 4 corners, the form
//!   written below) it becomes a CLOSED poly without the repeated corner, else an OPEN poly. `Sphere` / `Tube` prims are
//!   tessellated (below). Polys pass through.
//! * `totype = polyline`: a `Poly` becomes a `Polyline` prim over the same points; a CLOSED poly repeats its first corner
//!   at the end so the loop stays visible as a polyline (polyline -> polygon undoes it exactly). Sphere / tube pass through.
//! * Tessellation (`rows` I 8 >= 2 latitude bands, >= 1 for a tube; `cols` I 16 >= 3): the unit shape (sphere radius 1 about
//!   the origin, poles on +-Y; tube radius 1 at y = -0.5, radius `taper` at y = +0.5, axis Y) is mapped by the prim's row-major
//!   3x3 `xform` (column vectors) and offset by the prim's point (its single corner). Faces are wound outward (positive signed
//!   volume, a mirroring `xform` is compensated). Poles are shared points, so polar bands are triangles; a tube with a zero
//!   radius end collapses to a cone apex. `Tube.closed` adds the two cap n-gons. The centre point is dropped when no other
//!   prim uses it; the new points copy the centre point's attributes, the new polys the prim's attributes.
//! * Nothing to convert = the input `Arc` is shared. NOT modelled (DEFERRED): NURBS / Bezier / metaball / volume / packed
//!   sources, polysoup, `polyedges`, `lodu/lodv` style level of detail, caps shared as one pole fan.
use super::meshutil::{MAX_ELEMS, Remap, V3, bad, failed, positions, rebuild};
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const OP: &str = "convert";
const MAX_DIM: i32 = 1 << 12;

/// `fromtype` menu indices.
const FROM_ALL: u32 = 0;
const FROM_POLY: u32 = 1;
const FROM_POLYLINE: u32 = 2;
const FROM_SPHERE: u32 = 3;
const FROM_TUBE: u32 = 4;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("fromtype", ParamValue::Menu(FROM_ALL)).label("Convert From").menu(&[
            ("all", "All"),
            ("poly", "Polygon"),
            ("polyline", "Polyline"),
            ("sphere", "Sphere"),
            ("tube", "Tube"),
        ]),
        ParamSpec::new("totype", ParamValue::Menu(0))
            .label("Convert To")
            .menu(&[("polygon", "Polygon"), ("polyline", "Polyline")]),
        ParamSpec::new("rows", ParamValue::I(8)).label("Rows").range(1.0, 64.0),
        ParamSpec::new("cols", ParamValue::I(16)).label("Columns").range(3.0, 64.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Convert { specs: spec() })
}
struct Convert {
    specs: Vec<ParamSpec>,
}

fn menu(p: &hexe_graph::ParamSet, n: &str, max: u32) -> Result<u32, CookError> {
    match p.menu(n) {
        Some(v) if v <= max => Ok(v),
        Some(v) => Err(bad(format!("convert: `{n}`: menu index {v} out of range 0..={max}"))),
        None => Err(bad(format!("convert: `{n}`: missing or not a menu"))),
    }
}
fn dim(p: &hexe_graph::ParamSet, n: &str, min: i32) -> Result<usize, CookError> {
    match p.int(n) {
        Some(v) if (min..=MAX_DIM).contains(&v) => Ok(v as usize),
        _ => Err(bad(format!("convert: `{n}` must be an int in {min}..={MAX_DIM}"))),
    }
}

/// Faces (local point ids, outward winding) + positions of a sphere / tube, before `xform`/offset.
fn unit_shape(sphere: bool, taper: f64, closed: bool, rows: usize, cols: usize) -> (Vec<V3>, Vec<Vec<usize>>) {
    let mut pts: Vec<V3> = Vec::new();
    let mut rings: Vec<Vec<usize>> = Vec::new();
    let ring_pts = |pts: &mut Vec<V3>, y: f64, r: f64| -> Vec<usize> {
        if r.abs() < 1e-12 {
            pts.push([0.0, y, 0.0]);
            return vec![pts.len() - 1; cols];
        }
        (0..cols)
            .map(|j| {
                let th = std::f64::consts::TAU * j as f64 / cols as f64;
                pts.push([r * th.cos(), y, r * th.sin()]);
                pts.len() - 1
            })
            .collect()
    };
    if sphere {
        for k in 0..=rows {
            let phi = std::f64::consts::PI * k as f64 / rows as f64;
            let r = if k == 0 || k == rows { 0.0 } else { phi.sin() };
            let y = if k == 0 {
                1.0
            } else if k == rows {
                -1.0
            } else {
                phi.cos()
            };
            rings.push(ring_pts(&mut pts, y, r));
        }
    } else {
        for k in 0..=rows {
            // tube: ring 0 = TOP so the band loop below reads like the sphere's
            let t = k as f64 / rows as f64;
            rings.push(ring_pts(&mut pts, 0.5 - t, taper + (1.0 - taper) * t));
        }
    }
    let mut faces: Vec<Vec<usize>> = Vec::new();
    for k in 0..rows {
        for j in 0..cols {
            let j2 = (j + 1) % cols;
            faces.push(vec![rings[k][j2], rings[k][j], rings[k + 1][j], rings[k + 1][j2]]);
        }
    }
    if !sphere && closed {
        faces.push(rings[0].clone());
        faces.push(rings[rows].iter().rev().copied().collect());
    }
    // collapse repeated corners (poles / apex) and drop what is left with < 3 corners
    let faces = faces
        .into_iter()
        .filter_map(|f| {
            let mut o: Vec<usize> = Vec::with_capacity(f.len());
            for id in f {
                if o.last() != Some(&id) {
                    o.push(id);
                }
            }
            while o.len() > 1 && o.first() == o.last() {
                o.pop();
            }
            (o.len() >= 3).then_some(o)
        })
        .collect();
    (pts, faces)
}

impl Node<Detail> for Convert {
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
        let from = menu(p, "fromtype", FROM_TUBE)?;
        let to_line = menu(p, "totype", 1)? == 1;
        let (rows, cols) = (dim(p, "rows", 1)?, dim(p, "cols", 3)?);
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let want = |k: u32| from == FROM_ALL || from == k;
        let corners = |pr: &Prim| -> Option<&[u32]> {
            let (lo, hi) = (usize::try_from(pr.verts.start).ok()?, usize::try_from(pr.verts.end).ok()?);
            src.vertex_point.get(lo..hi)
        };
        let valid = |c: &[u32]| c.iter().all(|&i| i < src.point_count);
        // pass 1: which prims convert, which centre points can go
        #[derive(Clone, Copy, PartialEq)]
        enum Act {
            Keep,
            ToLine,
            ToPoly,
            Tess,
        }
        let acts: Vec<Act> = src
            .prims
            .iter()
            .map(|pr| {
                let Some(c) = corners(pr) else { return Act::Keep };
                if !valid(c) {
                    return Act::Keep;
                }
                match (&pr.kind, to_line) {
                    (PrimKind::Poly { .. }, true) if want(FROM_POLY) && c.len() >= 2 => Act::ToLine,
                    (PrimKind::Polyline, false) if want(FROM_POLYLINE) && c.len() >= 2 => Act::ToPoly,
                    (PrimKind::Sphere { .. }, false) if want(FROM_SPHERE) && c.len() == 1 => Act::Tess,
                    (PrimKind::Tube { .. }, false) if want(FROM_TUBE) && c.len() == 1 => Act::Tess,
                    _ => Act::Keep,
                }
            })
            .collect();
        if acts.iter().all(|&a| a == Act::Keep) {
            return Ok(Arc::clone(src));
        }
        let mut used_by_kept = vec![false; src.point_count as usize];
        for (pr, &a) in src.prims.iter().zip(&acts) {
            if a != Act::Tess {
                for &i in corners(pr).unwrap_or(&[]).iter().filter(|&&i| i < src.point_count) {
                    used_by_kept[i as usize] = true;
                }
            }
        }
        let mut dropped = vec![false; src.point_count as usize];
        for (pr, &a) in src.prims.iter().zip(&acts) {
            if a == Act::Tess {
                let c = corners(pr).unwrap_or(&[])[0] as usize;
                dropped[c] = !used_by_kept[c];
            }
        }
        let mut new_id = vec![u32::MAX; src.point_count as usize];
        let mut rm = Remap::default();
        let pos0 = positions(src);
        let mut pos: Vec<[f32; 3]> = Vec::new();
        for i in 0..src.point_count as usize {
            if !dropped[i] {
                new_id[i] = rm.point_src.len() as u32;
                rm.point_src.push(vec![i]);
                pos.push(src.p().get(i).copied().unwrap_or([0.0; 3]));
            }
        }
        // pass 2: topology
        let mut vertex_point: Vec<u32> = Vec::new();
        let mut prims: Vec<Prim> = Vec::new();
        let push = |vp: &mut Vec<u32>,
                    ps: &mut Vec<Prim>,
                    rm: &mut Remap,
                    kind: PrimKind,
                    ids: &[u32],
                    vsrc: &[Option<usize>],
                    psrc: usize| {
            let start = vp.len() as u32;
            vp.extend_from_slice(ids);
            rm.vert_src.extend_from_slice(vsrc);
            ps.push(Prim { kind, verts: start..vp.len() as u32 });
            rm.prim_src.push(Some(psrc));
        };
        for (pi, (pr, &a)) in src.prims.iter().zip(&acts).enumerate() {
            let vstart = pr.verts.start as usize;
            let c = corners(pr).unwrap_or(&[]);
            let mapped =
                |c: &[u32]| -> Vec<u32> { c.iter().map(|&i| new_id.get(i as usize).copied().unwrap_or(i)).collect() };
            match a {
                Act::Keep => {
                    let ids = mapped(c);
                    let vs: Vec<Option<usize>> = (0..c.len()).map(|k| Some(vstart + k)).collect();
                    push(&mut vertex_point, &mut prims, &mut rm, pr.kind.clone(), &ids, &vs, pi);
                }
                Act::ToLine => {
                    let mut ids = mapped(c);
                    let mut vs: Vec<Option<usize>> = (0..c.len()).map(|k| Some(vstart + k)).collect();
                    if matches!(pr.kind, PrimKind::Poly { closed: true }) {
                        ids.push(ids[0]);
                        vs.push(Some(vstart));
                    }
                    push(&mut vertex_point, &mut prims, &mut rm, PrimKind::Polyline, &ids, &vs, pi);
                }
                Act::ToPoly => {
                    let closed = c.len() >= 4 && c.first() == c.last();
                    let n = if closed { c.len() - 1 } else { c.len() };
                    let ids = mapped(&c[..n]);
                    let vs: Vec<Option<usize>> = (0..n).map(|k| Some(vstart + k)).collect();
                    push(&mut vertex_point, &mut prims, &mut rm, PrimKind::Poly { closed }, &ids, &vs, pi);
                }
                Act::Tess => {
                    let centre = c[0] as usize;
                    let (sphere, xf, taper, closed) = match pr.kind {
                        PrimKind::Sphere { xform } => (true, xform, 1.0, false),
                        PrimKind::Tube { xform, taper, closed } => (false, xform, f64::from(taper), closed),
                        _ => continue,
                    };
                    let m = xf.map(f64::from);
                    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
                        + m[2] * (m[3] * m[7] - m[4] * m[6]);
                    let (upts, faces) = unit_shape(sphere, taper, closed, rows, cols);
                    if (rm.point_src.len() + upts.len()) as u64 > MAX_ELEMS
                        || (prims.len() + faces.len()) as u64 > MAX_ELEMS
                    {
                        return Err(failed(OP, "output exceeds the allocation cap"));
                    }
                    let base = rm.point_src.len() as u32;
                    let o = pos0[centre];
                    for u in &upts {
                        let w = [
                            o[0] + m[0] * u[0] + m[1] * u[1] + m[2] * u[2],
                            o[1] + m[3] * u[0] + m[4] * u[1] + m[5] * u[2],
                            o[2] + m[6] * u[0] + m[7] * u[1] + m[8] * u[2],
                        ];
                        let r = [w[0] as f32, w[1] as f32, w[2] as f32];
                        if !r.iter().all(|x| x.is_finite()) {
                            return Err(failed(OP, "non-finite or f32-overflowing tessellated position"));
                        }
                        pos.push(r);
                        rm.point_src.push(vec![centre]);
                    }
                    for f in faces {
                        let mut ids: Vec<u32> = f.iter().map(|&k| base + k as u32).collect();
                        if det < 0.0 {
                            ids.reverse();
                        }
                        let vs = vec![None; ids.len()];
                        push(&mut vertex_point, &mut prims, &mut rm, PrimKind::Poly { closed: true }, &ids, &vs, pi);
                    }
                }
            }
        }
        Ok(Arc::new(rebuild(OP, src, vertex_point, prims, &rm, pos)?))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit;
    use super::super::surfutil::testkit::signed_volume;
    use super::*;
    use crate::load_scene;
    use hexe_geo::{AttrClass, AttrData, Attribute, TypeInfo};
    use hexe_graph::ParamSet;

    fn run(d: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        n.cook(&ctx, &[Arc::new(d)])
    }
    fn defaults() -> ParamSet {
        testkit::defaults(&*new())
    }
    fn shape(kind: PrimKind, at: [f32; 3]) -> Detail {
        let mut d = Detail::default();
        d.add_points(1);
        *d.p_mut() = vec![at];
        d.vertex_point.push(0);
        d.prims.push(Prim { kind, verts: 0..1 });
        d
    }
    const I3: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    fn scaled(s: [f32; 3]) -> [f32; 9] {
        [s[0], 0.0, 0.0, 0.0, s[1], 0.0, 0.0, 0.0, s[2]]
    }
    fn fine() -> ParamSet {
        defaults().with("rows", ParamValue::I(32)).with("cols", ParamValue::I(64))
    }

    #[test]
    fn sphere_becomes_an_outward_closed_shell_at_its_centre_and_drops_the_centre_point() {
        let d = run(shape(PrimKind::Sphere { xform: scaled([2.0; 3]) }, [1.0, 2.0, 3.0]), fine()).expect("cooks");
        assert_eq!(d.point_count, 2 + 31 * 64);
        assert_eq!(d.prim_count(), 64 + 30 * 64 + 64);
        let ball = 4.0 / 3.0 * std::f64::consts::PI * 8.0;
        let (lo, hi) = d.bbox();
        assert!(
            (lo[1] - 0.0).abs() < 1e-5
                && (hi[1] - 4.0).abs() < 1e-5
                && (lo[0] + 1.0).abs() < 0.01
                && (hi[2] - 5.0).abs() < 0.01
        );
        let c = run(shape(PrimKind::Sphere { xform: scaled([2.0; 3]) }, [0.0; 3]), fine()).expect("cooks");
        assert!((signed_volume(&c) - ball).abs() / ball < 0.01, "{}", signed_volume(&c));
        // a mirroring xform keeps the shell outward
        let m = run(shape(PrimKind::Sphere { xform: scaled([-1.0, 1.0, 1.0]) }, [0.0; 3]), fine()).expect("cooks");
        assert!(signed_volume(&m) > 4.0);
    }

    #[test]
    fn tube_closed_open_and_cone() {
        let tube = |taper: f32, closed: bool| {
            shape(PrimKind::Tube { xform: scaled([1.0, 2.0, 1.0]), taper, closed }, [0.0; 3])
        };
        let cyl_area = 0.5 * 64.0 * (std::f64::consts::TAU / 64.0).sin(); // regular 64-gon, R=1
        let c = run(tube(1.0, true), fine()).expect("cooks");
        assert!((signed_volume(&c) - cyl_area * 2.0).abs() < 1e-4, "{}", signed_volume(&c));
        assert_eq!(c.prim_count(), 32 * 64 + 2);
        let open = run(tube(1.0, false), fine()).expect("cooks");
        assert_eq!(open.prim_count(), 32 * 64);
        // taper 0 = cone: apex shared point, top band triangles, cap only at the bottom
        let cone = run(tube(0.0, true), defaults().with("rows", ParamValue::I(1)).with("cols", ParamValue::I(64)))
            .expect("cooks");
        assert_eq!((cone.point_count, cone.prim_count()), (65, 64 + 1));
        let cone_v = cyl_area * 2.0 / 3.0;
        assert!((signed_volume(&cone) - cone_v).abs() < 1e-4);
    }

    #[test]
    fn poly_polyline_round_trip_is_exact() {
        let mut d = testkit::grid3();
        let n0 = d.prim_count();
        d.add_poly(&[0, 1, 2], false);
        let lines = run(d.clone(), defaults().with("totype", ParamValue::Menu(1))).expect("cooks");
        assert_eq!(lines.prim_count(), n0 + 1);
        assert!(lines.prims.iter().all(|p| p.kind == PrimKind::Polyline));
        assert_eq!(lines.prims[0].verts.len(), 5, "closed quad repeats its first corner");
        assert_eq!(lines.prims[n0 as usize].verts.len(), 3);
        let back = run((*lines).clone(), defaults()).expect("cooks");
        assert_eq!(back.prims, d.prims);
        assert_eq!(back.vertex_point, d.vertex_point);
        assert_eq!(back.point_count, d.point_count);
    }

    #[test]
    fn fromtype_filters_and_unmatched_input_is_shared() {
        let mut d = testkit::grid3();
        d.add_poly(&[0, 1, 2], false);
        let q = Arc::new(d.clone());
        let n = new();
        let p = defaults().with("fromtype", ParamValue::Menu(FROM_SPHERE));
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        assert!(Arc::ptr_eq(&n.cook(&ctx, std::slice::from_ref(&q)).expect("cooks"), &q), "nothing to convert");
        // polygon -> polygon is a no-op too
        let same = Arc::new(d.clone());
        let ctx2 = CookCtx { time: 0.0, fps: 24.0, params: &defaults(), log: &|_| {} };
        assert!(Arc::ptr_eq(&n.cook(&ctx2, std::slice::from_ref(&same)).expect("cooks"), &same));
        // only polys convert with fromtype=poly
        let o = run(d, defaults().with("fromtype", ParamValue::Menu(FROM_POLY)).with("totype", ParamValue::Menu(1)))
            .expect("cooks");
        assert_eq!(o.prims.iter().filter(|p| p.kind == PrimKind::Polyline).count(), 5);
    }

    #[test]
    fn attributes_follow_prims_and_the_centre_point() {
        let mut d = shape(PrimKind::Sphere { xform: I3 }, [0.0; 3]);
        d.attrs.push(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[1.0, 0.5, 0.0]]),
            type_info: TypeInfo::Color,
        });
        d.attrs.push(Attribute {
            name: "id".into(),
            class: AttrClass::Prim,
            data: AttrData::I32(vec![7]),
            type_info: TypeInfo::None,
        });
        let o = run(d, defaults()).expect("cooks");
        let cd = match o.attr(AttrClass::Point, "Cd").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            x => panic!("{x:?}"),
        };
        assert_eq!(cd.len(), o.point_count as usize);
        assert!(cd.iter().all(|c| *c == [1.0, 0.5, 0.0]));
        assert!(
            matches!(o.attr(AttrClass::Prim, "id").map(|a| &a.data), Some(AttrData::I32(v)) if v.len() == o.prims.len() && v.iter().all(|&x| x == 7))
        );
    }

    #[test]
    fn centre_point_shared_with_a_kept_prim_survives() {
        let mut d = shape(PrimKind::Sphere { xform: I3 }, [0.0; 3]);
        d.add_points(2);
        d.p_mut()[1] = [1.0, 0.0, 0.0];
        d.p_mut()[2] = [2.0, 0.0, 0.0];
        d.add_poly(&[0, 1, 2], false);
        let o = run(d, defaults()).expect("cooks");
        assert_eq!(o.p()[0], [0.0; 3]);
        assert_eq!(o.point_count, 3 + 2 + 7 * 16, "centre kept (shared), tessellation added");
        assert!(matches!(o.prims.last().map(|p| &p.kind), Some(PrimKind::Poly { closed: false })));
    }

    #[test]
    fn hostile_params_and_geometry() {
        for (k, v) in [
            ("fromtype", ParamValue::Menu(5)),
            ("totype", ParamValue::Menu(2)),
            ("rows", ParamValue::I(0)),
            ("cols", ParamValue::I(2)),
            ("cols", ParamValue::I(1 << 20)),
            ("rows", ParamValue::F(1.0)),
        ] {
            assert!(matches!(run(testkit::grid3(), defaults().with(k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        assert_eq!(run(Detail::default(), defaults()).expect("cooks").prim_count(), 0);
        let nan = shape(PrimKind::Sphere { xform: scaled([f32::NAN; 3]) }, [0.0; 3]);
        assert!(matches!(run(nan, defaults()), Err(CookError::Failed(_))));
        let mut dangling = shape(PrimKind::Sphere { xform: I3 }, [0.0; 3]);
        dangling.vertex_point[0] = 9;
        assert_eq!(run(dangling, defaults()).expect("passes through").prim_count(), 1);
        let mut empty = shape(PrimKind::Tube { xform: I3, taper: 1.0, closed: true }, [0.0; 3]);
        empty.prims[0].verts = 0..0;
        assert_eq!(run(empty, defaults()).expect("passes through").prim_count(), 1);
        let huge = shape(PrimKind::Sphere { xform: scaled([f32::MAX; 3]) }, [f32::MAX; 3]);
        let r = run(huge, defaults());
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
        let cap = run(
            shape(PrimKind::Sphere { xform: I3 }, [0.0; 3]),
            defaults().with("rows", ParamValue::I(4096)).with("cols", ParamValue::I(4096)),
        );
        assert!(matches!(cap, Err(CookError::Failed(_))));
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/convert.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert!(out.prims.iter().all(|p| matches!(p.kind, PrimKind::Poly { .. })) && out.prim_count() > 10);
        assert_eq!(crate::registry().create("convert").map(|n| n.type_name()), Some("convert"));
    }
}
