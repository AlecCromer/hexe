//! PolyReduce SOP (token `polyreduce`): reduce polygon count by shortest-edge collapse. PARTIAL: quadric-free.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! * `percentage` F 50 (0..=100): keep about this share of the closed polys (target = `round(polys * pct / 100)`); 100 = input
//!   `Arc` shared. `lockboundary` B off: on, boundary points (an edge used by one poly) never move or merge.
//! * Loop: take the SHORTEST live edge `(a, b)` (ties: lowest ids), collapse `b` into `a` at the edge midpoint, repeat until the
//!   poly count is at the target or no legal edge is left (so the result can stay above the target; quads stay quads or
//!   degrade to triangles, only triangle-pair edges remove two polys). Polys left with < 3 distinct corners vanish.
//! * An edge is collapsible only when (link condition) every point adjacent to both ends lies in a poly that contains the edge,
//!   neither end is used by a non-poly prim, and not (both ends on the boundary while the edge itself is interior). No flip /
//!   normal-deviation test (quadric-free): thin folded regions can fold over.
//! * Attributes: merged points blend their sources (`meshutil::rebuild`); surviving vertices and polys copy theirs; groups follow
//!   (a merged point stays in a group only if all its sources were). Non-poly prims and unused points pass through.
//!   NOT modelled (DEFERRED): quadric error metric, attribute-seam/UV preservation, `target` modes other than percentage, normal/
//!   area weights, symmetry.
use super::meshutil::{Remap, V3, bad, failed, positions, rebuild, to_f32, toggle};
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const OP: &str = "polyreduce";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("percentage", ParamValue::F(50.0)).label("Percentage").range(0.0, 100.0),
        ParamSpec::new("lockboundary", ParamValue::B(false)).label("Lock Boundary"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(PolyReduce { specs: spec() })
}
struct PolyReduce {
    specs: Vec<ParamSpec>,
}

struct Mesh {
    /// per poly: corner point ids + source vertex ids (`None` = dead poly)
    faces: Vec<Option<(Vec<u32>, Vec<usize>)>>,
    of_point: Vec<BTreeSet<usize>>,
}

impl Mesh {
    fn edge_faces(&self, a: u32, b: u32) -> Vec<usize> {
        self.of_point[a as usize]
            .iter()
            .copied()
            .filter(|&f| {
                self.faces[f].as_ref().is_some_and(|(c, _)| {
                    (0..c.len())
                        .any(|k| (c[k] == a && c[(k + 1) % c.len()] == b) || (c[k] == b && c[(k + 1) % c.len()] == a))
                })
            })
            .collect()
    }
    fn neighbours(&self, p: u32) -> BTreeSet<u32> {
        let mut n = BTreeSet::new();
        for &f in &self.of_point[p as usize] {
            if let Some((c, _)) = &self.faces[f] {
                for k in 0..c.len() {
                    if c[k] == p {
                        n.insert(c[(k + 1) % c.len()]);
                        n.insert(c[(k + c.len() - 1) % c.len()]);
                    }
                }
            }
        }
        n
    }
    fn on_boundary(&self, p: u32) -> bool {
        self.neighbours(p).into_iter().any(|q| self.edge_faces(p, q).len() == 1)
    }
}

fn len_key(a: V3, b: V3) -> u64 {
    let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    if d.is_finite() { d.to_bits() } else { u64::MAX }
}

impl Node<Detail> for PolyReduce {
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
        let pct = match p.float("percentage") {
            Some(v) if v.is_finite() && (0.0..=100.0).contains(&v) => f64::from(v),
            _ => return Err(bad("polyreduce: `percentage` must be a finite float in 0..=100")),
        };
        let lock_b = toggle(p, "lockboundary")?;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let np = src.point_count as usize;
        let mut m = Mesh { faces: Vec::new(), of_point: vec![BTreeSet::new(); np] };
        let mut locked = vec![false; np];
        // prim -> face index (closed polys only)
        let mut face_of_prim: Vec<Option<usize>> = Vec::with_capacity(src.prims.len());
        for pr in &src.prims {
            let vs = (usize::try_from(pr.verts.start).ok(), usize::try_from(pr.verts.end).ok());
            let corners = match vs {
                (Some(lo), Some(hi)) => src.vertex_point.get(lo..hi).filter(|c| c.iter().all(|&i| (i as usize) < np)),
                _ => None,
            };
            match (&pr.kind, corners) {
                (PrimKind::Poly { closed: true }, Some(c)) if c.len() >= 3 => {
                    let f = m.faces.len();
                    for &i in c {
                        m.of_point[i as usize].insert(f);
                    }
                    m.faces.push(Some((c.to_vec(), (pr.verts.start as usize..pr.verts.end as usize).collect())));
                    face_of_prim.push(Some(f));
                }
                (_, _) => {
                    let (lo, hi) = (pr.verts.start as usize, (pr.verts.end as usize).min(src.vertex_point.len()));
                    for &i in src.vertex_point.get(lo..hi.max(lo)).unwrap_or(&[]) {
                        if (i as usize) < np {
                            locked[i as usize] = true;
                        }
                    }
                    face_of_prim.push(None);
                }
            }
        }
        let total = m.faces.len();
        let target = (total as f64 * pct / 100.0).round() as usize;
        if total == 0 || target >= total {
            return Ok(Arc::clone(src));
        }
        let mut pos = positions(src);
        let mut alive = vec![true; np];
        let mut ver = vec![0u32; np];
        let mut merged: Vec<Vec<usize>> = (0..np).map(|i| vec![i]).collect();
        let mut heap: BinaryHeap<Reverse<(u64, u32, u32, u32, u32)>> = BinaryHeap::new();
        let push_edges = |heap: &mut BinaryHeap<Reverse<(u64, u32, u32, u32, u32)>>,
                          m: &Mesh,
                          pos: &[V3],
                          ver: &[u32],
                          pt: u32| {
            for q in m.neighbours(pt) {
                let (a, b) = (pt.min(q), pt.max(q));
                heap.push(Reverse((len_key(pos[a as usize], pos[b as usize]), a, b, ver[a as usize], ver[b as usize])));
            }
        };
        for pt in 0..np as u32 {
            for q in m.neighbours(pt).into_iter().filter(|&q| q > pt) {
                heap.push(Reverse((len_key(pos[pt as usize], pos[q as usize]), pt, q, 0, 0)));
            }
        }
        let mut live = total;
        while live > target {
            let Some(Reverse((_, a, b, va, vb))) = heap.pop() else { break };
            let (ai, bi) = (a as usize, b as usize);
            if !alive[ai] || !alive[bi] || ver[ai] != va || ver[bi] != vb || locked[ai] || locked[bi] {
                continue;
            }
            let shared = m.edge_faces(a, b);
            if shared.is_empty() {
                continue;
            }
            let (ba, bb) = (m.on_boundary(a), m.on_boundary(b));
            if (lock_b && (ba || bb)) || (ba && bb && shared.len() > 1) {
                continue;
            }
            let mut allowed: BTreeSet<u32> = BTreeSet::new();
            for &f in &shared {
                if let Some((c, _)) = &m.faces[f] {
                    allowed.extend(c.iter().copied());
                }
            }
            let (na, nb) = (m.neighbours(a), m.neighbours(b));
            if na.intersection(&nb).any(|c| !allowed.contains(c)) {
                continue;
            }
            // collapse b into a at the midpoint
            let mid =
                [(pos[ai][0] + pos[bi][0]) / 2.0, (pos[ai][1] + pos[bi][1]) / 2.0, (pos[ai][2] + pos[bi][2]) / 2.0];
            pos[ai] = mid;
            alive[bi] = false;
            ver[ai] += 1;
            let moved = std::mem::take(&mut merged[bi]);
            merged[ai].extend(moved);
            let touched: Vec<usize> = m.of_point[bi].iter().copied().collect();
            for f in touched {
                let Some((c, v)) = m.faces[f].take() else { continue };
                let (mut nc, mut nv): (Vec<u32>, Vec<usize>) = (Vec::new(), Vec::new());
                for (id, vx) in c.iter().zip(v.iter()) {
                    let id = if *id == b { a } else { *id };
                    if nc.last() != Some(&id) {
                        nc.push(id);
                        nv.push(*vx);
                    }
                }
                while nc.len() > 1 && nc.first() == nc.last() {
                    nc.pop();
                    nv.pop();
                }
                if nc.len() >= 3 {
                    m.of_point[ai].insert(f);
                    m.faces[f] = Some((nc, nv));
                } else {
                    live -= 1;
                    for id in c {
                        m.of_point[id as usize].remove(&f);
                    }
                }
            }
            m.of_point[bi].clear();
            push_edges(&mut heap, &m, &pos, &ver, a);
        }
        // output
        let mut new_id = vec![u32::MAX; np];
        let mut rm = Remap::default();
        let mut p_out: Vec<V3> = Vec::new();
        for i in 0..np {
            if alive[i] {
                new_id[i] = rm.point_src.len() as u32;
                rm.point_src.push(std::mem::take(&mut merged[i]));
                p_out.push(pos[i]);
            }
        }
        let mut vertex_point: Vec<u32> = Vec::new();
        let mut prims: Vec<Prim> = Vec::new();
        for (pi, pr) in src.prims.iter().enumerate() {
            let start = vertex_point.len() as u32;
            match face_of_prim[pi] {
                Some(f) => {
                    let Some((c, v)) = &m.faces[f] else { continue };
                    vertex_point.extend(c.iter().map(|&i| new_id[i as usize]));
                    rm.vert_src.extend(v.iter().map(|&x| Some(x)));
                }
                None => {
                    let (lo, hi) = (pr.verts.start as usize, pr.verts.end as usize);
                    for k in lo..hi.min(src.vertex_point.len()) {
                        let id = src.vertex_point[k];
                        vertex_point.push(new_id.get(id as usize).copied().unwrap_or(id));
                        rm.vert_src.push(Some(k));
                    }
                }
            }
            prims.push(Prim { kind: pr.kind.clone(), verts: start..vertex_point.len() as u32 });
            rm.prim_src.push(Some(pi));
        }
        let p32 = to_f32(OP, &p_out)?;
        if vertex_point.iter().any(|&i| i == u32::MAX) {
            return Err(failed(OP, "internal: dangling point after collapse"));
        }
        Ok(Arc::new(rebuild(OP, src, vertex_point, prims, &rm, p32)?))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::{testkit, tri_volume};
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
    /// `n x n` points on the XY plane, 2 triangles per cell, +Z-facing in the hexe winding.
    fn tri_grid(n: u32) -> Detail {
        let mut d = Detail::default();
        d.add_points(n * n);
        *d.p_mut() = (0..n * n).map(|i| [(i % n) as f32, (i / n) as f32, 0.0]).collect();
        for r in 0..n - 1 {
            for c in 0..n - 1 {
                let a = r * n + c;
                d.add_poly(&[a, a + n, a + n + 1], true);
                d.add_poly(&[a, a + n + 1, a + 1], true);
            }
        }
        d
    }
    fn octahedron() -> Detail {
        let mut d = Detail::default();
        d.add_points(6);
        *d.p_mut() = vec![
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        // outward in the Houdini winding (checked by the volume sign in the test)
        for (a, c, b) in [(0, 2, 4), (2, 1, 4), (1, 3, 4), (3, 0, 4), (2, 0, 5), (1, 2, 5), (3, 1, 5), (0, 3, 5)] {
            d.add_poly(&[a, b, c], true);
        }
        d
    }
    fn volume(d: &Detail) -> f64 {
        let pos = positions(d);
        d.prims
            .iter()
            .map(|pr| {
                let i: Vec<usize> = d.vertex_point[pr.verts.start as usize..pr.verts.end as usize]
                    .iter()
                    .map(|&x| x as usize)
                    .collect();
                tri_volume(pos[i[0]], pos[i[1]], pos[i[2]])
            })
            .sum()
    }
    fn area_xy(d: &Detail) -> f64 {
        let pos = positions(d);
        d.prims
            .iter()
            .map(|pr| {
                let i: Vec<usize> = d.vertex_point[pr.verts.start as usize..pr.verts.end as usize]
                    .iter()
                    .map(|&x| x as usize)
                    .collect();
                let (a, b, c) = (pos[i[0]], pos[i[1]], pos[i[2]]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }

    #[test]
    fn grid_reduces_to_about_half_with_boundary_locked_and_area_preserved() {
        let g = tri_grid(6);
        assert_eq!(g.prim_count(), 50);
        let o = run(g, defaults().with("lockboundary", ParamValue::B(true))).expect("cooks");
        assert!(o.prim_count() <= 25 + 1 && o.prim_count() > 10, "{}", o.prim_count());
        assert!((area_xy(&o) - 25.0).abs() < 1e-4, "{}", area_xy(&o));
        // every prim still a triangle on 3 distinct points; all 20 boundary points kept at their place
        for pr in &o.prims {
            let v = &o.vertex_point[pr.verts.start as usize..pr.verts.end as usize];
            assert!(v.len() == 3 && v[0] != v[1] && v[1] != v[2] && v[0] != v[2]);
        }
        let on_edge = o.p().iter().filter(|q| q[0] == 0.0 || q[0] == 5.0 || q[1] == 0.0 || q[1] == 5.0).count();
        assert_eq!(on_edge, 20);
    }

    #[test]
    fn closed_octahedron_collapses_staying_a_closed_outward_manifold() {
        let oct = octahedron();
        assert!(volume(&oct) > 0.0, "fixture must be outward");
        let o = run(oct, defaults()).expect("cooks");
        assert!(o.prim_count() < 8 && o.prim_count() >= 4, "{}", o.prim_count());
        let mut dir = std::collections::BTreeMap::new();
        for pr in &o.prims {
            let v = &o.vertex_point[pr.verts.start as usize..pr.verts.end as usize];
            for k in 0..v.len() {
                *dir.entry((v[k], v[(k + 1) % v.len()])).or_insert(0) += 1;
            }
        }
        assert!(dir.iter().all(|(&(a, b), &c)| c == 1 && dir.contains_key(&(b, a))), "closed 2-manifold kept");
        assert!(volume(&o) > 0.0);
    }

    #[test]
    fn percentage_extremes_attributes_and_non_poly_prims() {
        let g = tri_grid(4);
        let a = Arc::new(g.clone());
        let n = new();
        let p = defaults().with("percentage", ParamValue::F(100.0));
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        assert!(Arc::ptr_eq(&n.cook(&ctx, std::slice::from_ref(&a)).expect("cooks"), &a));
        // Cd (point) blends, prim attr copies; a polyline pins its points
        let mut d = g;
        d.attrs.push(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3((0..16).map(|i| [i as f32, 0.0, 0.0]).collect()),
            type_info: TypeInfo::Color,
        });
        d.attrs.push(Attribute {
            name: "id".into(),
            class: AttrClass::Prim,
            data: AttrData::I32((0..18).collect()),
            type_info: TypeInfo::None,
        });
        d.add_poly(&[0, 1, 2, 3], false);
        let o = run(d, defaults().with("percentage", ParamValue::F(0.0))).expect("cooks");
        assert!(o.prim_count() < 19);
        assert!(matches!(o.prims.last().map(|p| &p.kind), Some(PrimKind::Poly { closed: false })));
        let cd = match o.attr(AttrClass::Point, "Cd").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            x => panic!("{x:?}"),
        };
        assert_eq!(cd.len(), o.point_count as usize);
        // the polyline's four end points are untouched (locked)
        assert_eq!(&o.p()[..4], &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]]);
        assert!(
            matches!(o.attr(AttrClass::Prim, "id").map(|a| &a.data), Some(AttrData::I32(v)) if v.len() == o.prims.len())
        );
    }

    #[test]
    fn hostile_params_and_geometry() {
        for (k, v) in [
            ("percentage", ParamValue::F(f32::NAN)),
            ("percentage", ParamValue::F(-1.0)),
            ("percentage", ParamValue::F(101.0)),
            ("percentage", ParamValue::I(5)),
            ("lockboundary", ParamValue::I(1)),
        ] {
            assert!(matches!(run(tri_grid(3), defaults().with(k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        assert_eq!(run(Detail::default(), defaults()).expect("cooks").prim_count(), 0);
        let mut d = tri_grid(4);
        d.vertex_point[0] = 99; // dangling: that poly becomes a pass-through prim, never panics
        let r = run(d, defaults());
        assert!(r.is_ok(), "{:?}", r.err());
        let mut deg = tri_grid(3);
        deg.p_mut()[4] = [f32::NAN, 0.0, 0.0];
        let r = run(deg, defaults());
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/polyreduce.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert!(out.prim_count() > 0 && out.prim_count() < 200, "{}", out.prim_count());
        assert_eq!(crate::registry().create("polyreduce").map(|n| n.type_name()), Some("polyreduce"));
    }
}
