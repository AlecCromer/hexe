//! Smooth SOP (token `smooth`): uniform Laplacian smoothing of point positions.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Per iteration, simultaneously (Jacobi, all reads from the previous iteration): `P' = P + strength * (avg(N(P)) - P)`
//! where `N(P)` = the DISTINCT points joined to `P` by an edge. Edges: consecutive corners of every poly / polyline,
//! plus the closing edge of closed polys (a shared edge counts once; box corner -> 3 neighbours). Points without
//! neighbours stay. `pin_boundary` (off): points on an edge used by exactly one closed poly keep their position.
//! Math in f64, one cast to f32 at the end (f32 overflow -> `Failed`).
//!
//! Params: `iterations` I 10 (0 = input `Arc` shared; negative or > 1000 = `BadParam`), `strength` F 0.5
//! (any finite value; 0 = input `Arc` shared; > 1 over-relaxes), `pin_boundary` B false. Only `P` changes (N is NOT
//! recomputed). Work bound `iterations * (edges + points) <= 2^31`, else `Failed`.
use super::meshutil::{V3, add, bad, failed, float, int, positions, scale, sub, to_f32, toggle};
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "smooth";
const MAX_ITERATIONS: i32 = 1000;
const MAX_WORK: u64 = 1 << 31;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("iterations", ParamValue::I(10)).label("Smoothing Iterations").range(0.0, 100.0),
        ParamSpec::new("strength", ParamValue::F(0.5)).label("Strength").range(0.0, 1.0),
        ParamSpec::new("pin_boundary", ParamValue::B(false)).label("Pin Boundary Points"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Smooth { specs: spec() })
}
struct Smooth {
    specs: Vec<ParamSpec>,
}

/// Ids of a prim's vertex points that are in range, in order.
fn prim_points(d: &Detail, pr: &Prim) -> Vec<usize> {
    let (lo, hi) = (usize::try_from(pr.verts.start).unwrap_or(0), usize::try_from(pr.verts.end).unwrap_or(0));
    let np = d.point_count as usize;
    d.vertex_point.get(lo..hi).unwrap_or(&[]).iter().map(|&p| p as usize).filter(|&p| p < np).collect()
}

/// CSR adjacency (sorted, deduplicated) plus the boundary flags of closed-poly edges used exactly once.
fn graph(d: &Detail) -> (Vec<usize>, Vec<usize>, Vec<bool>) {
    let np = d.point_count as usize;
    let mut dir: Vec<(usize, usize)> = Vec::new();
    let mut closed_edges: Vec<(usize, usize)> = Vec::new();
    for pr in &d.prims {
        let (closed, linear) = match pr.kind {
            PrimKind::Poly { closed } => (closed, true),
            PrimKind::Polyline => (false, true),
            _ => (false, false),
        };
        if !linear {
            continue;
        }
        let pts = prim_points(d, pr);
        let n = pts.len();
        let m = if closed && n >= 3 { n } else { n.saturating_sub(1) };
        for k in 0..m {
            let (Some(&a), Some(&b)) = (pts.get(k), pts.get((k + 1) % n)) else { continue };
            if a == b {
                continue;
            }
            dir.push((a, b));
            dir.push((b, a));
            if closed && n >= 3 {
                closed_edges.push((a.min(b), a.max(b)));
            }
        }
    }
    dir.sort_unstable();
    dir.dedup();
    let mut off = vec![0usize; np + 1];
    for &(a, _) in &dir {
        if let Some(o) = off.get_mut(a + 1) {
            *o += 1;
        }
    }
    for i in 0..np {
        off[i + 1] += off[i];
    }
    let adj: Vec<usize> = dir.iter().map(|e| e.1).collect();
    closed_edges.sort_unstable();
    let mut boundary = vec![false; np];
    let mut i = 0;
    while i < closed_edges.len() {
        let j = closed_edges[i..].iter().take_while(|e| **e == closed_edges[i]).count();
        if j == 1 {
            let (a, b) = closed_edges[i];
            boundary[a] = true;
            boundary[b] = true;
        }
        i += j;
    }
    (off, adj, boundary)
}

impl Node<Detail> for Smooth {
    fn type_name(&self) -> &'static str {
        "smooth"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let iters = int(p, "iterations")?;
        if !(0..=MAX_ITERATIONS).contains(&iters) {
            return Err(bad(format!("`iterations` must be in 0..={MAX_ITERATIONS}, got {iters}")));
        }
        let strength = float(p, "strength")?;
        let pin = toggle(p, "pin_boundary")?;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if iters == 0 || strength == 0.0 || src.point_count == 0 {
            return Ok(Arc::clone(src));
        }
        let (off, adj, boundary) = graph(src);
        let work = u64::from(iters as u32).saturating_mul((adj.len() + off.len()) as u64);
        if work > MAX_WORK {
            return Err(failed(OP, "iterations * edges exceeds the work cap"));
        }
        let mut cur = positions(src);
        let mut next: Vec<V3> = cur.clone();
        for _ in 0..iters {
            for (i, out) in next.iter_mut().enumerate() {
                let nb = match (off.get(i), off.get(i + 1)) {
                    (Some(&lo), Some(&hi)) => adj.get(lo..hi).unwrap_or(&[]),
                    _ => &[],
                };
                let here = cur.get(i).copied().unwrap_or([0.0; 3]);
                *out = if nb.is_empty() || (pin && boundary.get(i).copied().unwrap_or(false)) {
                    here
                } else {
                    let s = nb.iter().filter_map(|&j| cur.get(j)).fold([0.0; 3], |a, &q| add(a, q));
                    let avg = scale(s, 1.0 / nb.len() as f64);
                    add(here, scale(sub(avg, here), strength))
                };
            }
            std::mem::swap(&mut cur, &mut next);
        }
        let mut out = (**src).clone();
        *out.p_mut() = to_f32(OP, &cur)?;
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, grid3};
    use super::*;
    use hexe_graph::ParamSet;

    fn with(iters: i32, strength: f32) -> ParamSet {
        defaults(&*new()).with("iterations", ParamValue::I(iters)).with("strength", ParamValue::F(strength))
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "smooth");
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("iterations", ParamValue::I(10)),
                ("strength", ParamValue::F(0.5)),
                ("pin_boundary", ParamValue::B(false))
            ]
        );
    }
    /// Box corner c = (h,h,h)*s (h = 1/2, signs s): its 3 neighbours differ in exactly one sign, so
    /// avg = ((-h+h+h), (h-h+h), (h+h-h)) / 3 per sign pattern = (h/3, h/3, h/3) * s = (1/6,1/6,1/6) * s.
    /// strength 1: P' = P + 1*(avg - P) = avg -> each corner -> (+-1/6, +-1/6, +-1/6), i.e. original corner / 3.
    #[test]
    fn box_one_iteration_strength_one_moves_corners_to_one_sixth() {
        let src = cube();
        let out = cook(&*new(), src.clone(), &with(1, 1.0)).expect("cooks");
        for (a, b) in src.p().iter().zip(out.p()) {
            for j in 0..3 {
                assert!((b[j] - a[j] / 3.0).abs() < 1e-7, "{a:?} -> {b:?}");
                assert!((b[j].abs() - 1.0 / 6.0).abs() < 1e-7);
            }
        }
        // strength 0.5: P' = P + 0.5 (P/-3 ... avg = P/3) = P * (1 - 0.5 * (1 - 1/3)) = P * 2/3
        let half = cook(&*new(), src.clone(), &with(1, 0.5)).expect("cooks");
        for (a, b) in src.p().iter().zip(half.p()) {
            assert!((0..3).all(|j| (b[j] - a[j] * (2.0 / 3.0)).abs() < 1e-7), "{a:?} -> {b:?}");
        }
        // topology untouched
        assert_eq!((out.point_count, out.prim_count()), (8, 6));
        assert_eq!(out.prims, src.prims);
    }
    #[test]
    fn zero_strength_zero_iterations_and_empty_share_the_arc() {
        let a = Arc::new(cube());
        for p in [with(5, 0.0), with(0, 1.0)] {
            let o = cook_arc(&*new(), &a, &p).expect("cooks");
            assert!(Arc::ptr_eq(&o, &a));
            assert_eq!(*o, *a);
        }
        assert_eq!(cook(&*new(), Detail::default(), &with(3, 1.0)).expect("cooks").point_count, 0);
    }
    #[test]
    fn pin_boundary_keeps_grid_border_and_moves_only_the_free_centre() {
        let mut d = grid3();
        d.p_mut()[4] = [1.0, 1.0, 2.0]; // lift the interior point
        let free = cook(&*new(), d.clone(), &with(1, 1.0)).expect("cooks");
        assert!(free.p()[0] != d.p()[0], "unpinned corner moves");
        let pinned = cook(&*new(), d.clone(), &with(1, 1.0).with("pin_boundary", ParamValue::B(true))).expect("cooks");
        for i in [0usize, 1, 2, 3, 5, 6, 7, 8] {
            assert_eq!(pinned.p()[i], d.p()[i], "border point {i} pinned");
        }
        // centre neighbours = 4 edge midpoints at z = 0 -> centre -> (1,1,0)
        assert_eq!(pinned.p()[4], [1.0, 1.0, 0.0]);
    }
    #[test]
    fn many_iterations_converge_box_to_its_centroid() {
        let out = cook(&*new(), cube(), &with(200, 0.5)).expect("cooks");
        assert!(out.p().iter().flatten().all(|c| c.abs() < 1e-6), "{:?}", out.p());
    }
    #[test]
    fn hostile_params_and_open_prims() {
        for (it, s) in [(-1, 0.5), (1001, 0.5), (1, f32::NAN), (1, f32::INFINITY)] {
            assert!(matches!(cook(&*new(), cube(), &with(it, s)), Err(CookError::BadParam(_))), "{it} {s}");
        }
        let huge = cook(&*new(), cube(), &with(1, f32::MAX)).map(|d| d.p().len());
        assert!(huge.is_ok() || matches!(huge, Err(CookError::Failed(_))));
        // polyline + degenerate faces + bad ids: no panic, topology kept
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![[0.0; 3], [2.0, 0.0, 0.0], [4.0, 0.0, 0.0], [9.0, 9.0, 9.0]];
        d.add_poly(&[0, 1, 2], false);
        d.add_poly(&[1, 1, 1], true);
        d.add_poly(&[0, 99], true);
        let out = cook(&*new(), d, &with(1, 1.0)).expect("cooks");
        assert_eq!(out.p()[0], [2.0, 0.0, 0.0]); // end of a line -> its only neighbour
        assert_eq!(out.p()[1], [2.0, 0.0, 0.0]); // middle -> avg(0, 4)
        assert_eq!(out.p()[3], [9.0, 9.0, 9.0]); // orphan stays
    }
    #[test]
    fn scene_loader_cooks_example_and_registry_has_slot() {
        let bogus = r#"{"nodes":[{"name":"c","type":"smooth","params":{"bogus":1}}],"display":"c"}"#;
        assert!(crate::load_scene(bogus).is_err());
        let ok = r#"{"nodes":[{"name":"b","type":"box"},{"name":"s","type":"smooth","params":{"iterations":2}}],"edges":[["b",0,"s",0]],"display":"s"}"#;
        let (mut g, id) = crate::load_scene(ok).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 8);
        assert_eq!(crate::registry().create("smooth").map(|n| n.type_name()), Some("smooth"));
    }
}
