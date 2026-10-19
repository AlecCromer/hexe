//! convert: DOC-VERIFIED against Houdini 20.5 help `sop/convert.txt` (text only; no Houdini runtime).
//! Doc facts: Convert From (subset of types) / Convert To; Group = subset of prims; polygons<->polylines; quadrics
//! tessellate to polygons; Preserve Original keeps the input alongside; LOD scales resolution. Related recorded rows:
//! polygon/curve/resample topology (open vs closed polys).
mod common;
use common::*;
use hexe_geo::{AttrClass, Detail, Prim, PrimKind};
use hexe_graph::ParamValue as PV;
use std::sync::Arc;

const TO_POLYGON: u32 = 0;
const TO_POLYLINE: u32 = 1;
const TO_EDGES: u32 = 2;
const FROM_POLYLINE: u32 = 2;

fn sphere_at(c: [f32; 3], r: f32) -> Arc<Detail> {
    let mut d = Detail::default();
    d.add_points(1);
    *d.p_mut() = vec![c];
    d.vertex_point.push(0);
    d.prims.push(Prim { kind: PrimKind::Sphere { xform: [r, 0.0, 0.0, 0.0, r, 0.0, 0.0, 0.0, r] }, verts: 0..1 });
    Arc::new(d)
}
fn signed_volume(d: &Detail) -> f32 {
    // sum over fan triangles of (a . (b x c)) / 6 about the origin of the polys
    let mut v = 0.0;
    for i in 0..d.prims.len() {
        let pts = prim_pts(d, i);
        let p = |k: usize| d.p()[pts[k] as usize];
        for k in 1..pts.len() - 1 {
            let (a, b, c) = (p(0), p(k), p(k + 1));
            v += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0;
        }
    }
    v
}

#[test]
fn defaults_tessellate_a_sphere_prim_to_a_closed_outward_unit_sphere() {
    let c = [1.0, 2.0, 3.0];
    let o = cook("convert", &[], Some(&sphere_at(c, 2.0))).unwrap();
    assert!(o.prims.iter().all(|p| matches!(p.kind, PrimKind::Poly { closed: true })), "all polygons");
    assert!(o.prims.len() >= 8);
    for p in o.p() {
        let r = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt();
        assert!((r - 2.0).abs() < 1e-3, "on the radius-2 sphere about the centre, got {r}");
    }
    assert!(edge_use(&o).values().all(|&n| n == 2), "closed 2-manifold");
    // outward winding == the box SOP's (Houdini convention): signed volume has the same sign as a box's
    let mut centred = (*o).clone();
    for p in centred.p_mut().iter_mut() {
        *p = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
    }
    let box_vol = signed_volume(&cube());
    assert!(box_vol != 0.0 && signed_volume(&centred) * box_vol > 0.0, "sphere winding matches box winding");
}

#[test]
fn lod_raises_resolution_with_default_mode() {
    let s = sphere_at([0.0; 3], 1.0);
    let a = cook("convert", &[], Some(&s)).unwrap();
    let b = cook("convert", &[("lodu", PV::F(2.0)), ("lodv", PV::F(2.0))], Some(&s)).unwrap();
    assert!(b.prims.len() > a.prims.len(), "{} !> {}", b.prims.len(), a.prims.len());
}

#[test]
fn polygon_to_polyline_and_back_roundtrips() {
    let g = quad_grid(1); // one quad
    let l = cook("convert", &[("totype", PV::Menu(TO_POLYLINE))], Some(&g)).unwrap();
    assert_eq!(l.point_count, 4);
    assert!(matches!(l.prims[0].kind, PrimKind::Polyline));
    assert_eq!(prim_pts(&l, 0), [0, 2, 3, 1, 0], "closed loop stays visible: first corner repeated");
    let back = cook("convert", &[("totype", PV::Menu(TO_POLYGON))], Some(&l)).unwrap();
    assert!(matches!(back.prims[0].kind, PrimKind::Poly { closed: true }));
    assert_eq!(prim_pts(&back, 0), prim_pts(&g, 0));
    // an open polyline (no repeated corner) becomes an OPEN polygon
    let mut o = Detail::default();
    o.add_points(3);
    *o.p_mut() = vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 1.0, 0.0]];
    o.add_poly(&[0, 1, 2], false);
    let ol = cook("convert", &[("totype", PV::Menu(TO_POLYLINE))], Some(&Arc::new(o))).unwrap();
    assert_eq!(prim_pts(&ol, 0), [0, 1, 2]);
    let ob = cook("convert", &[], Some(&ol)).unwrap();
    assert!(matches!(ob.prims[0].kind, PrimKind::Poly { closed: false }));
}

#[test]
fn convert_from_filters_and_preserve_original_keeps_input() {
    let g = grid3();
    // Convert From = polyline: polygons are not a candidate => untouched
    let n =
        cook("convert", &[("fromtype", PV::Menu(FROM_POLYLINE)), ("totype", PV::Menu(TO_POLYLINE))], Some(&g)).unwrap();
    assert_eq!(*n, *g);
    // Preserve Original: input kept, converted copy appended
    let k = cook("convert", &[("totype", PV::Menu(TO_POLYLINE)), ("keeporiginal", PV::B(true))], Some(&g)).unwrap();
    assert_eq!(k.prims.len(), 8);
    assert!(k.prims[..4].iter().all(|p| matches!(p.kind, PrimKind::Poly { .. })));
    assert!(k.prims[4..].iter().all(|p| matches!(p.kind, PrimKind::Polyline)));
    assert_eq!(&k.p()[..g.p().len()], g.p());
}

#[test]
fn polygon_edges_are_unique_and_empty_input_is_fine() {
    let g = grid3(); // 2x2 quads: 12 unique edges
    let e = cook("convert", &[("totype", PV::Menu(TO_EDGES))], Some(&g)).unwrap();
    assert_eq!(e.prims.len(), 12);
    assert!(e.prims.iter().all(|p| matches!(p.kind, PrimKind::Polyline) && p.verts.len() == 2));
    let z = cook("convert", &[], Some(&Arc::new(Detail::default()))).unwrap();
    assert_eq!((z.point_count, z.prims.len()), (0, 0));
    let _ = AttrClass::Point;
}
