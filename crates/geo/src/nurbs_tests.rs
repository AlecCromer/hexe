//! NURBS / Bezier prim kinds: analytical evaluation tests, tessellation, IO round trips, shipped Houdini fixtures, hostile input.
use crate::*;
use std::path::PathBuf;

fn curve_detail(pts: &[[f32; 3]], kind: PrimKind) -> Detail {
    let mut d = Detail::default();
    d.add_points(pts.len() as u32);
    d.p_mut().copy_from_slice(pts);
    d.vertex_point = (0..pts.len() as u32).collect();
    d.prims.push(Prim { kind, verts: 0..pts.len() as u32 });
    d
}

fn set_pw(d: &mut Detail, w: &[f32]) {
    d.add_attr(Attribute {
        name: "Pw".into(),
        class: AttrClass::Point,
        data: AttrData::F32(w.to_vec()),
        type_info: TypeInfo::None,
    });
}

fn near(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= eps)
}

fn nurbs_basis(order: u32, end: bool) -> Basis {
    Basis { order, knots: Vec::new(), end }
}

/// Standard 9-CV rational quadratic unit circle in the XY plane.
fn circle_detail() -> Detail {
    let pts = [
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let knots = vec![0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0];
    let mut d = curve_detail(&pts, PrimKind::NurbsCurve { basis: Basis { order: 3, knots, end: true }, closed: false });
    let s = std::f32::consts::FRAC_1_SQRT_2;
    set_pw(&mut d, &[1.0, s, 1.0, s, 1.0, s, 1.0, s, 1.0]);
    d
}

#[test]
fn deboor_endpoints_equal_end_control_points() {
    let pts = [[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [3.0, 3.0, 1.0], [5.0, 1.0, 2.0], [6.0, 0.0, 0.5]];
    for order in [2u32, 3, 4, 5] {
        let d = curve_detail(&pts, PrimKind::NurbsCurve { basis: nurbs_basis(order, true), closed: false });
        assert!(near(d.evaluate(0, 0.0).unwrap(), pts[0], 1e-5), "order {order} start");
        assert!(near(d.evaluate(0, 1.0).unwrap(), pts[4], 1e-5), "order {order} end");
    }
    // out-of-range / NaN u clamp instead of failing
    let d = curve_detail(&pts, PrimKind::NurbsCurve { basis: nurbs_basis(3, true), closed: false });
    assert!(near(d.evaluate(0, -4.0).unwrap(), pts[0], 1e-5));
    assert!(near(d.evaluate(0, 9.0).unwrap(), pts[4], 1e-5));
    assert!(d.evaluate(0, f32::NAN).is_some());
}

#[test]
fn nurbs_order2_is_the_control_polyline_and_partition_of_unity() {
    let pts = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 4.0, 0.0]];
    let d = curve_detail(&pts, PrimKind::NurbsCurve { basis: nurbs_basis(2, true), closed: false });
    assert!(near(d.evaluate(0, 0.25).unwrap(), [1.0, 0.0, 0.0], 1e-5));
    assert!(near(d.evaluate(0, 0.5).unwrap(), [2.0, 0.0, 0.0], 1e-5));
    assert!(near(d.evaluate(0, 0.75).unwrap(), [2.0, 2.0, 0.0], 1e-5));
    // constant control points -> constant curve (basis weights sum to 1 everywhere), order 4 unclamped too
    let same = [[1.0, 2.0, 3.0]; 7];
    let d = curve_detail(&same, PrimKind::NurbsCurve { basis: nurbs_basis(4, false), closed: false });
    for i in 0..=20 {
        assert!(near(d.evaluate(0, i as f32 / 20.0).unwrap(), [1.0, 2.0, 3.0], 1e-5));
    }
}

#[test]
fn rational_nurbs_circle_is_exact_and_arc_length_is_two_pi() {
    let d = circle_detail();
    for i in 0..=64 {
        let q = d.evaluate(0, i as f32 / 64.0).unwrap();
        let r = (q[0] * q[0] + q[1] * q[1]).sqrt();
        assert!((r - 1.0).abs() < 1e-5, "u {i}/64 radius {r}");
        assert!(q[2].abs() < 1e-6);
    }
    assert!(near(d.evaluate(0, 0.0).unwrap(), [1.0, 0.0, 0.0], 1e-5));
    assert!(near(d.evaluate(0, 0.25).unwrap(), [0.0, 1.0, 0.0], 1e-5));
    assert!(near(d.evaluate(0, 0.5).unwrap(), [-1.0, 0.0, 0.0], 1e-5));
    let t = d.tessellate(64, 1);
    assert_eq!(t.prims.len(), 1);
    assert!(matches!(t.prims[0].kind, PrimKind::Poly { closed: false }));
    let p = t.p();
    let vs = &t.vertex_point[t.prims[0].verts.start as usize..t.prims[0].verts.end as usize];
    assert_eq!(vs.len(), 4 * 64 + 1);
    let len: f32 = vs
        .windows(2)
        .map(|w| {
            let (a, b) = (p[w[0] as usize], p[w[1] as usize]);
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        })
        .sum();
    assert!((len - std::f32::consts::TAU).abs() < 1e-3, "arc length {len}");
    // CV points consumed (no orphans), tessellated points only
    assert_eq!(t.point_count, 4 * 64 + 1);
}

#[test]
fn bezier_cubic_matches_de_casteljau_closed_form() {
    let c = [[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [3.0, 2.0, 1.0], [4.0, 0.0, 2.0]];
    let d = curve_detail(&c, PrimKind::BezierCurve { basis: Basis::new(4), closed: false });
    assert!(near(d.evaluate(0, 0.0).unwrap(), c[0], 1e-6));
    assert!(near(d.evaluate(0, 1.0).unwrap(), c[3], 1e-6));
    // B(1/2) = (P0 + 3 P1 + 3 P2 + P3) / 8
    let mid = [0, 1, 2].map(|k| (c[0][k] + 3.0 * c[1][k] + 3.0 * c[2][k] + c[3][k]) / 8.0);
    assert!(near(d.evaluate(0, 0.5).unwrap(), mid, 1e-6));
    // B(1/4) = (27 P0 + 27 P1 + 9 P2 + P3) / 64
    let q = [0, 1, 2].map(|k| (27.0 * c[0][k] + 27.0 * c[1][k] + 9.0 * c[2][k] + c[3][k]) / 64.0);
    assert!(near(d.evaluate(0, 0.25).unwrap(), q, 1e-6));
}

#[test]
fn bezier_piecewise_hits_shared_joint_and_closed_wraps() {
    let pts: Vec<[f32; 3]> = (0..7).map(|i| [i as f32, (i % 2) as f32, 0.0]).collect();
    let d = curve_detail(&pts, PrimKind::BezierCurve { basis: Basis::new(4), closed: false });
    assert!(near(d.evaluate(0, 0.5).unwrap(), pts[3], 1e-6), "2 segments: u=1/2 is the joint CV");
    // closed: 6 CVs order 4 = 2 segments, the last ends on CV 0
    let d = curve_detail(&pts[..6], PrimKind::BezierCurve { basis: Basis::new(4), closed: true });
    assert!(near(d.evaluate(0, 0.5).unwrap(), pts[3], 1e-6));
    assert!(near(d.evaluate(0, 1.0).unwrap(), pts[0], 1e-6));
    let t = d.tessellate(8, 1);
    assert!(matches!(t.prims[0].kind, PrimKind::Poly { closed: true }));
    assert_eq!(t.prims[0].verts.end - t.prims[0].verts.start, 16, "closed: no duplicate seam point");
}

#[test]
fn closed_periodic_nurbs_is_a_loop_inside_the_hull() {
    let pts = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 4.0, 0.0], [0.0, 4.0, 0.0]];
    let d = curve_detail(&pts, PrimKind::NurbsCurve { basis: nurbs_basis(3, false), closed: true });
    let t = d.tessellate(10, 1);
    assert!(matches!(t.prims[0].kind, PrimKind::Poly { closed: true }));
    assert_eq!(t.prims[0].verts.end - t.prims[0].verts.start, 40);
    for q in t.p() {
        assert!((0.0..=4.0).contains(&q[0]) && (0.0..=4.0).contains(&q[1]));
    }
    // u=0 and the wrap point agree (periodic)
    assert!(near(d.evaluate(0, 0.0).unwrap(), d.evaluate(0, 1.0).unwrap(), 1e-4));
}

fn plane_patch(order_u: u32, order_v: u32, cols: u32, rows: u32, bezier: bool) -> Detail {
    let mut pts = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            pts.push([c as f32, r as f32, 0.0]);
        }
    }
    let (ub, vb) = (
        Basis { order: order_u, knots: Vec::new(), end: true },
        Basis { order: order_v, knots: Vec::new(), end: true },
    );
    let kind = if bezier {
        PrimKind::BezierSurf { cols, rows, ubasis: ub, vbasis: vb, uwrap: false, vwrap: false }
    } else {
        PrimKind::NurbsSurf { cols, rows, ubasis: ub, vbasis: vb, uwrap: false, vwrap: false }
    };
    curve_detail(&pts, kind)
}

fn area(d: &Detail) -> f32 {
    let p = d.p();
    let mut a = 0.0;
    for q in &d.prims {
        let v: Vec<[f32; 3]> = (q.verts.start..q.verts.end).map(|i| p[d.vertex_point[i as usize] as usize]).collect();
        for k in 1..v.len().saturating_sub(1) {
            let (e1, e2) = (
                [v[k][0] - v[0][0], v[k][1] - v[0][1], v[k][2] - v[0][2]],
                [v[k + 1][0] - v[0][0], v[k + 1][1] - v[0][1], v[k + 1][2] - v[0][2]],
            );
            let c = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            a += 0.5 * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        }
    }
    a
}

#[test]
fn flat_surfaces_evaluate_bilinearly_and_tessellate_to_the_exact_area() {
    // order-2 NURBS 3x2 grid spanning [0,2]x[0,1]
    let d = plane_patch(2, 2, 3, 2, false);
    assert!(near(d.evaluate_uv(0, 0.5, 0.5).unwrap(), [1.0, 0.5, 0.0], 1e-6));
    assert!(near(d.evaluate_uv(0, 0.0, 0.0).unwrap(), [0.0, 0.0, 0.0], 1e-6));
    assert!(near(d.evaluate_uv(0, 1.0, 1.0).unwrap(), [2.0, 1.0, 0.0], 1e-6));
    let t = d.tessellate(3, 3);
    assert!(!t.has_spline_prims());
    assert_eq!(t.prims.len(), 6 * 3, "2 u-spans * 3 + 3 v... = 6 x 3 quads");
    assert!((area(&t) - 2.0).abs() < 1e-4, "area {}", area(&t));
    // normals point +Z (u x v)
    // cubic Bezier 4x4 flat patch over [0,3]^2: area 9, centre (1.5,1.5)
    let b = plane_patch(4, 4, 4, 4, true);
    assert!(near(b.evaluate_uv(0, 0.5, 0.5).unwrap(), [1.5, 1.5, 0.0], 1e-5));
    assert!((area(&b.tessellate(4, 4)) - 9.0).abs() < 1e-4);
}

#[test]
fn surface_winding_faces_u_cross_v() {
    let t = plane_patch(2, 2, 2, 2, false).tessellate(1, 1);
    let p = t.p();
    let v: Vec<[f32; 3]> =
        (t.prims[0].verts.start..t.prims[0].verts.end).map(|i| p[t.vertex_point[i as usize] as usize]).collect();
    let (e1, e2) = ([v[1][0] - v[0][0], v[1][1] - v[0][1]], [v[2][0] - v[1][0], v[2][1] - v[1][1]]);
    assert!(e1[0] * e2[1] - e1[1] * e2[0] > 0.0, "CCW seen from +Z");
}

#[test]
fn wrapped_surface_has_no_duplicate_seam() {
    // 4 cols wrapping in u (periodic order 3), 2 rows open order 2: a ring band
    let mut pts = Vec::new();
    for r in 0..2 {
        for c in 0..4 {
            let a = c as f32 * std::f32::consts::FRAC_PI_2;
            pts.push([a.cos(), r as f32, a.sin()]);
        }
    }
    let kind = PrimKind::NurbsSurf {
        cols: 4,
        rows: 2,
        ubasis: nurbs_basis(3, false),
        vbasis: nurbs_basis(2, true),
        uwrap: true,
        vwrap: false,
    };
    let t = curve_detail(&pts, kind).tessellate(4, 2);
    // u: 4 spans * 4 = 16 columns (wrapped), v: 1 span * 2 + 1 = 3 rows -> 16 * 2 quads
    assert_eq!(t.prims.len(), 32);
    assert_eq!(t.point_count, 16 * 3);
}

#[test]
fn tessellate_inherits_prim_attrs_blends_point_attrs_and_keeps_other_prims() {
    let pts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let mut d = curve_detail(&pts, PrimKind::NurbsCurve { basis: nurbs_basis(2, true), closed: false });
    // + a loose triangle after the curve
    d.add_points(3);
    d.p_mut()[3..].copy_from_slice(&[[0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 2.0, 0.0]]);
    d.add_poly(&[3, 4, 5], true);
    d.add_attr(Attribute {
        name: "Cd".into(),
        class: AttrClass::Point,
        data: AttrData::V3(vec![[0.0; 3], [0.5; 3], [1.0; 3], [0.0; 3], [0.0; 3], [0.0; 3]]),
        type_info: TypeInfo::Color,
    });
    d.add_attr(Attribute {
        name: "id".into(),
        class: AttrClass::Prim,
        data: AttrData::I32(vec![7, 9]),
        type_info: TypeInfo::None,
    });
    d.groups.push(Group { name: "g".into(), class: AttrClass::Prim, mask: vec![true, false] });
    let t = d.tessellate(2, 1);
    // curve: 2 spans * 2 + 1 = 5 poly points, triangle keeps its 3
    assert_eq!(t.prims.len(), 2);
    assert_eq!(t.point_count, 5 + 3);
    let AttrData::I32(ids) = &t.attr(AttrClass::Prim, "id").unwrap().data else { panic!() };
    assert_eq!(ids, &vec![7, 9]);
    assert_eq!(t.group(AttrClass::Prim, "g").unwrap().mask, vec![true, false]);
    // Cd blended exactly like P: x/2 along the curve
    let AttrData::V3(cd) = &t.attr(AttrClass::Point, "Cd").unwrap().data else { panic!() };
    let vs = &t.vertex_point[t.prims[0].verts.start as usize..t.prims[0].verts.end as usize];
    for &v in vs {
        let x = t.p()[v as usize][0];
        assert!((cd[v as usize][0] - x / 2.0).abs() < 1e-6);
    }
    // triangle untouched: same coordinates
    let tv: Vec<[f32; 3]> = t.vertex_point[t.prims[1].verts.start as usize..t.prims[1].verts.end as usize]
        .iter()
        .map(|&i| t.p()[i as usize])
        .collect();
    assert_eq!(tv, vec![[0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 2.0, 0.0]]);
    // no spline prims -> clone
    assert_eq!(t.tessellate(4, 4), t);
}

fn sample_detail() -> Detail {
    let mut d = circle_detail();
    let base = d.point_count;
    d.add_points(4);
    d.p_mut()[base as usize..].copy_from_slice(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]]);
    d.attrs.retain(|a| a.name != "Pw");
    set_pw(&mut d, &[1.0, 0.7, 1.0, 0.7, 1.0, 0.7, 1.0, 0.7, 1.0, 1.0, 1.0, 1.0, 1.0]);
    let v0 = d.vertex_point.len() as u32;
    d.vertex_point.extend(base..base + 4);
    d.prims.push(Prim {
        kind: PrimKind::BezierSurf {
            cols: 2,
            rows: 2,
            ubasis: Basis::new(2),
            vbasis: Basis::new(2),
            uwrap: false,
            vwrap: false,
        },
        verts: v0..v0 + 4,
    });
    let v1 = d.vertex_point.len() as u32;
    d.vertex_point.extend([0, 1, 2, 3, 4, 5]);
    d.prims.push(Prim {
        kind: PrimKind::BezierCurve {
            basis: Basis { order: 3, knots: vec![0.0, 2.0, 5.0], end: false },
            closed: false,
        },
        verts: v1..v1 + 5,
    });
    let v2 = d.vertex_point.len() as u32;
    d.vertex_point.extend(base..base + 4);
    d.prims.push(Prim {
        kind: PrimKind::NurbsSurf {
            cols: 2,
            rows: 2,
            ubasis: Basis { order: 2, knots: vec![0.0, 0.0, 1.0, 1.0], end: true },
            vbasis: nurbs_basis(2, true),
            uwrap: false,
            vwrap: true,
        },
        verts: v2..v2 + 4,
    });
    d.canonicalize();
    d
}

#[test]
fn json_and_bjson_round_trip_equality() {
    let d = sample_detail();
    let mut j = read_geo_json(&write_geo_json(&d)).expect("json round trip");
    j.canonicalize();
    assert_eq!(j, d);
    let b = write_geo_bjson(&d).expect("bjson write");
    let mut b = read_geo_bjson(&b).expect("bjson read");
    b.canonicalize();
    assert_eq!(b, d);
    // evaluation survives the round trip
    assert_eq!(j.evaluate(0, 0.3), d.evaluate(0, 0.3));
}

#[test]
fn foreign_nurbs_json_bodies_stay_unknown_passthrough() {
    // a NURBCurve body without our `basis` field (Houdini's own layout is unverified) is kept verbatim, never dropped
    let src = String::from_utf8(write_geo_json(&sample_detail())).unwrap().replace("\"basis\"", "\"houdini_basis\"");
    let d = read_geo_json(src.as_bytes()).expect("parses");
    assert!(matches!(&d.prims[0].kind, PrimKind::Unknown { ty, .. } if ty == "NURBCurve"));
    assert!(d.prims.iter().any(|p| p.kind.is_spline_surf()), "surfaces use ubasis/vbasis and stay modelled");
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Houdini's shipped `GPD/all.geo` (text) + `all.bgeo` (binary twin) carry 3 BezierCurve, 2 NURBMesh, 1 BezierMesh.
#[test]
fn shipped_houdini_fixtures_decode_to_first_class_kinds_and_evaluate() {
    let text = read_geo(&std::fs::read(fixtures().join("gpd-all/expected.geo")).unwrap()).unwrap();
    let bin = read_geo(&std::fs::read(fixtures().join("houdini-ship/gpd-all-legacy/all.bgeo")).unwrap()).unwrap();
    for d in [&text, &bin] {
        let curves = d.prims.iter().filter(|p| matches!(p.kind, PrimKind::BezierCurve { .. })).count();
        let nurbs = d.prims.iter().filter(|p| matches!(p.kind, PrimKind::NurbsSurf { .. })).count();
        let bez = d.prims.iter().filter(|p| matches!(p.kind, PrimKind::BezierSurf { .. })).count();
        assert_eq!((curves, nurbs, bez), (3, 2, 1));
        let (lo, hi) = d.bbox();
        for (i, p) in d.prims.iter().enumerate() {
            if !p.kind.is_spline() {
                continue;
            }
            let q = if p.kind.is_spline_curve() { d.evaluate(i, 0.37) } else { d.evaluate_uv(i, 0.37, 0.61) };
            let q = q.unwrap_or_else(|| panic!("prim {i} {:?} must evaluate", p.kind));
            for k in 0..3 {
                assert!(
                    q[k] >= lo[k] - 1e-3 && q[k] <= hi[k] + 1e-3,
                    "prim {i} axis {k} {} outside [{}, {}]",
                    q[k],
                    lo[k],
                    hi[k]
                );
            }
        }
        let t = d.tessellate(4, 4);
        assert!(!t.has_spline_prims());
        assert!(t.p().iter().flatten().all(|x| x.is_finite()));
        assert_eq!(t.vertex_point.len(), t.prims.iter().map(|p| (p.verts.end - p.verts.start) as usize).sum::<usize>());
        assert!(t.vertex_point.iter().all(|&v| v < t.point_count));
    }
    // text and binary agree on kinds, orders, wraps
    for (a, b) in text.prims.iter().zip(&bin.prims) {
        match (&a.kind, &b.kind) {
            (PrimKind::BezierCurve { basis: x, closed: c }, PrimKind::BezierCurve { basis: y, closed: e }) => {
                assert_eq!((x.order, c), (y.order, e))
            }
            (
                PrimKind::NurbsSurf { cols: a1, rows: a2, uwrap: a3, vwrap: a4, .. },
                PrimKind::NurbsSurf { cols: b1, rows: b2, uwrap: b3, vwrap: b4, .. },
            ) => assert_eq!((a1, a2, a3, a4), (b1, b2, b3, b4)),
            _ => {}
        }
    }
    // the open BezierMesh 10x10 order 4 in the fixture: a 3x3 patch grid
    let i = text.prims.iter().position(|p| matches!(p.kind, PrimKind::BezierSurf { .. })).unwrap();
    let PrimKind::BezierSurf { cols, rows, ubasis, uwrap, .. } = &text.prims[i].kind else { panic!() };
    assert_eq!((*cols, *rows, ubasis.order, *uwrap), (10, 10, 4, false));
}

#[test]
fn hostile_curves_never_panic() {
    let pts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 1.0, 0.0], [3.0, 0.0, 0.0]];
    let bad = [
        PrimKind::NurbsCurve { basis: nurbs_basis(0, true), closed: false },
        PrimKind::NurbsCurve { basis: nurbs_basis(1, true), closed: false },
        PrimKind::NurbsCurve { basis: nurbs_basis(9, true), closed: false },
        PrimKind::NurbsCurve { basis: nurbs_basis(u32::MAX, true), closed: true },
        PrimKind::NurbsCurve { basis: Basis { order: 3, knots: vec![f32::NAN; 7], end: true }, closed: false },
        PrimKind::NurbsCurve {
            basis: Basis { order: 3, knots: vec![3.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0], end: true },
            closed: false,
        },
        PrimKind::NurbsCurve { basis: Basis { order: 3, knots: vec![1.0; 7], end: true }, closed: false },
        PrimKind::BezierCurve { basis: Basis::new(3), closed: false }, // 4 CVs, (4-1)%2 != 0
        PrimKind::BezierCurve { basis: Basis::new(4), closed: true },  // 4 % 3 != 0
        PrimKind::BezierSurf {
            cols: 3,
            rows: 3,
            ubasis: Basis::new(2),
            vbasis: Basis::new(2),
            uwrap: false,
            vwrap: false,
        },
        PrimKind::NurbsSurf {
            cols: u32::MAX,
            rows: u32::MAX,
            ubasis: Basis::new(2),
            vbasis: Basis::new(2),
            uwrap: true,
            vwrap: true,
        },
    ];
    for k in bad {
        let d = curve_detail(&pts, k.clone());
        let _ = d.evaluate(0, 0.5);
        let _ = d.evaluate_uv(0, 0.5, 0.5);
        let t = d.tessellate(u32::MAX, 0);
        assert!(!t.has_spline_prims(), "{k:?}");
        assert!(t.vertex_point.iter().all(|&v| v < t.point_count), "{k:?}");
    }
    // prim whose vertex range lies outside vertex_point
    let mut d = curve_detail(&pts, PrimKind::NurbsCurve { basis: nurbs_basis(3, true), closed: false });
    d.prims[0].verts = 2..99;
    assert!(d.evaluate(0, 0.5).is_none());
    let _ = d.tessellate(4, 4);
    // zero-weight CVs
    let mut d = curve_detail(&pts, PrimKind::NurbsCurve { basis: nurbs_basis(2, true), closed: false });
    set_pw(&mut d, &[0.0; 4]);
    assert!(d.evaluate(0, 0.5).is_none());
    assert!(!d.tessellate(4, 4).has_spline_prims());
}

#[test]
fn basis_from_stored_decodes_open_end_and_bezier_breaks() {
    // Houdini `Basis 4 end` open, 6 CVs: stored = 0 1 2 3 (n-k+2 = 4) -> full clamped vector
    let b = crate::nurbs::basis_from_stored(4, true, false, false, 6, &[0.0, 1.0, 2.0, 3.0]);
    assert_eq!(b.knots, vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0]);
    // Bezier `use`: 25 CVs order 4 -> 9 breaks
    let br: Vec<f32> = (0..9).map(|i| i as f32).collect();
    assert_eq!(crate::nurbs::basis_from_stored(4, false, true, false, 25, &br).knots, br);
    // uninterpretable forms fall back to default (empty)
    assert!(crate::nurbs::basis_from_stored(4, false, false, true, 10, &[0.0; 12]).knots.is_empty());
}
