//! Shared helpers for the `*_semantics.rs` DOC-VERIFIED tests (public API only; no Houdini).
#![allow(dead_code)]
use hexe_geo::{AttrClass, AttrData, Attribute, Detail};
use hexe_graph::{CookCtx, CookError, ParamSet, ParamValue};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Cook a registered SOP at `time` with defaults + overrides.
pub fn cook_at(
    ty: &str,
    ov: &[(&str, ParamValue)],
    input: Option<&Arc<Detail>>,
    time: f32,
) -> Result<Arc<Detail>, CookError> {
    let node = hexe_sop::registry().create(ty).expect("registered");
    let mut ps = node.params().iter().fold(ParamSet::new(), |p, s| p.with(&s.name, s.default.clone()));
    for (k, v) in ov {
        ps = ps.with(k, v.clone());
    }
    let ctx = CookCtx { time, fps: 24.0, params: &ps, log: &|_| {} };
    node.cook(&ctx, input.map(std::slice::from_ref).unwrap_or(&[]))
}
pub fn cook(ty: &str, ov: &[(&str, ParamValue)], input: Option<&Arc<Detail>>) -> Result<Arc<Detail>, CookError> {
    cook_at(ty, ov, input, 0.0)
}
pub fn s(v: &str) -> ParamValue {
    ParamValue::S(v.into())
}
/// Unit box, 8 shared points, 6 quads.
pub fn cube() -> Arc<Detail> {
    cook("box", &[], None).expect("box")
}
/// 3x3 points (2x2 quads) z=0, x/y in 0..2, +Z winding.
pub fn grid3() -> Arc<Detail> {
    let mut d = Detail::default();
    d.add_points(9);
    *d.p_mut() = (0..9).map(|i| [(i % 3) as f32, (i / 3) as f32, 0.0]).collect();
    for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        let a = r * 3 + c;
        d.add_poly(&[a, a + 3, a + 4, a + 1], true);
    }
    Arc::new(d)
}
/// n x n quad grid in z=0 (n+1)^2 points, +Z winding.
pub fn quad_grid(n: u32) -> Arc<Detail> {
    let w = n + 1;
    let mut d = Detail::default();
    d.add_points(w * w);
    *d.p_mut() = (0..w * w).map(|i| [(i % w) as f32, (i / w) as f32, 0.0]).collect();
    for r in 0..n {
        for c in 0..n {
            let a = r * w + c;
            d.add_poly(&[a, a + w, a + w + 1, a + 1], true);
        }
    }
    Arc::new(d)
}
/// Single triangle (odd point count 3) in z=0.
pub fn tri() -> Arc<Detail> {
    let mut d = Detail::default();
    d.add_points(3);
    *d.p_mut() = vec![[0.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 3.0, 0.0]];
    d.add_poly(&[0, 1, 2], true);
    Arc::new(d)
}
/// Vertex point ids of prim `i`.
pub fn prim_pts(d: &Detail, i: usize) -> Vec<u32> {
    let r = d.prims[i].verts.clone();
    d.vertex_point[r.start as usize..r.end as usize].to_vec()
}
/// Newell normal (unnormalised) of closed poly `i`.
pub fn newell(d: &Detail, i: usize) -> [f32; 3] {
    let pts = prim_pts(d, i);
    let p = d.p();
    let mut n = [0.0f32; 3];
    for k in 0..pts.len() {
        let (a, b) = (p[pts[k] as usize], p[pts[(k + 1) % pts.len()] as usize]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}
/// Undirected edge -> number of polygon prims using it.
pub fn edge_use(d: &Detail) -> BTreeMap<(u32, u32), u32> {
    let mut m = BTreeMap::new();
    for i in 0..d.prims.len() {
        let pts = prim_pts(d, i);
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            *m.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    m
}
pub fn attr<'a>(d: &'a Detail, c: AttrClass, n: &str) -> Option<&'a Attribute> {
    d.attr(c, n)
}
pub fn v3(d: &Detail, c: AttrClass, n: &str) -> Vec<[f32; 3]> {
    match &d.attr(c, n).unwrap_or_else(|| panic!("attr {n}")).data {
        AttrData::V3(v) => v.clone(),
        o => panic!("{n}: not V3: {o:?}"),
    }
}
pub fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}
