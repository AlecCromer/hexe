//! polyreduce: DOC-VERIFIED against Houdini 20.5 help `sop/polyreduce.txt` (text only; no Houdini runtime).
//! Doc facts: Target = percentage of input polygons to keep (default node reduces) or fixed polygon count; "final polygon
//! count may be less than the exact target"; boundaries preserved (Stiffen Boundaries); shape retained. hexe invariants:
//! prim count <= target, no non-manifold edge (> 2 prims) introduced, closed meshes stay closed, no degenerate prims,
//! every vertex references a valid point.
mod common;
use common::*;
use hexe_geo::Detail;
use hexe_graph::ParamValue as PV;
use std::sync::Arc;

fn invariants(d: &Detail) {
    assert!(d.vertex_point.iter().all(|&p| p < d.point_count), "valid point refs");
    for i in 0..d.prims.len() {
        let mut p = prim_pts(d, i);
        assert!(p.len() >= 3, "prim {i} has {} corners", p.len());
        p.sort_unstable();
        p.dedup();
        assert_eq!(p.len(), prim_pts(d, i).len(), "prim {i} has a repeated corner");
    }
    assert!(edge_use(d).values().all(|&n| n <= 2), "no non-manifold edge");
}
/// Bumpy 12x12 quad sheet (so QEM has real shape to keep).
fn sheet() -> Arc<Detail> {
    let mut d = (*quad_grid(12)).clone();
    for (i, p) in d.p_mut().iter_mut().enumerate() {
        p[2] = 0.3 * ((i % 13) as f32 * 0.7).sin() * ((i / 13) as f32 * 0.5).cos();
    }
    Arc::new(d)
}
/// Closed icosphere-like mesh from the convert SOP's sphere tessellation.
fn ball() -> Arc<Detail> {
    let mut s = Detail::default();
    s.add_points(1);
    s.vertex_point.push(0);
    s.prims.push(hexe_geo::Prim {
        kind: hexe_geo::PrimKind::Sphere { xform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] },
        verts: 0..1,
    });
    cook("convert", &[("lodu", PV::F(2.0)), ("lodv", PV::F(2.0))], Some(&Arc::new(s))).unwrap()
}

#[test]
fn defaults_reduce_to_at_most_half_the_prims_and_stay_manifold() {
    let s = sheet();
    assert_eq!(s.prims.len(), 144);
    let o = cook("polyreduce", &[], Some(&s)).unwrap();
    assert!(!o.prims.is_empty());
    assert!(o.prims.len() <= 72, "default 50%: got {}", o.prims.len());
    invariants(&o);
    assert!(o.point_count < s.point_count);
}

#[test]
fn percentage_and_count_targets() {
    let s = sheet();
    // hexe preserves boundary edges hard (Houdini: a weight), 48 fixed boundary points floor the sheet; free boundary reaches 25%
    let q = cook("polyreduce", &[("percentage", PV::F(25.0)), ("preserveboundary", PV::B(false))], Some(&s)).unwrap();
    assert!(q.prims.len() <= 36 && !q.prims.is_empty(), "25%: got {}", q.prims.len());
    invariants(&q);
    let hard = cook("polyreduce", &[("percentage", PV::F(25.0))], Some(&s)).unwrap();
    assert!(
        hard.prims.len() < 72 && hard.prims.len() >= q.prims.len(),
        "hard boundary: still reduces, floor above free"
    );
    invariants(&hard);
    let c = cook(
        "polyreduce",
        &[("target", PV::Menu(1)), ("finalcount", PV::I(40)), ("preserveboundary", PV::B(false))],
        Some(&s),
    )
    .unwrap();
    assert!(c.prims.len() <= 40 && !c.prims.is_empty(), "count 40: got {}", c.prims.len());
    invariants(&c);
    // 100% keeps everything (nothing to reduce)
    let all = cook("polyreduce", &[("percentage", PV::F(100.0))], Some(&s)).unwrap();
    assert_eq!(*all, *s);
}

#[test]
fn boundary_is_preserved_and_shape_stays_in_bbox() {
    let s = sheet();
    let o = cook("polyreduce", &[], Some(&s)).unwrap();
    // corners of the sheet are boundary points: still present at their original positions
    for corner in [[0.0, 0.0], [12.0, 0.0], [0.0, 12.0], [12.0, 12.0]] {
        assert!(
            o.p().iter().any(|p| (p[0] - corner[0]).abs() < 1e-4 && (p[1] - corner[1]).abs() < 1e-4),
            "corner {corner:?}"
        );
    }
    let (lo, hi) = (s.bbox(), o.bbox());
    for k in 0..2 {
        assert!(hi.0[k] >= lo.0[k] - 1e-4 && hi.1[k] <= lo.1[k] + 1e-4);
    }
}

#[test]
fn closed_mesh_stays_closed_after_reduction() {
    let b = ball();
    assert!(edge_use(&b).values().all(|&n| n == 2));
    let o = cook("polyreduce", &[("percentage", PV::F(50.0))], Some(&b)).unwrap();
    assert!(o.prims.len() <= b.prims.len() / 2 && !o.prims.is_empty(), "{} -> {}", b.prims.len(), o.prims.len());
    invariants(&o);
    assert!(edge_use(&o).values().all(|&n| n == 2), "closed 2-manifold preserved");
}

#[test]
fn edge_cases_empty_and_single_triangle() {
    let e = cook("polyreduce", &[], Some(&Arc::new(Detail::default()))).unwrap();
    assert_eq!((e.point_count, e.prims.len()), (0, 0));
    // one triangle, all edges are boundary: nothing can legally collapse
    let t = cook("polyreduce", &[], Some(&tri())).unwrap();
    assert!(t.prims.len() <= 1);
    invariants(&t);
}
