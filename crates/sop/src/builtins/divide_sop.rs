//! Divide SOP (token `divide`): split closed polygons into pieces with at most `numsides` corners.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Params: `convex` B on, `numsides` I 3 (3..=1024, else `BadParam`). Only closed polys with MORE than `numsides` valid corners
//! are split; everything else (small polys, open polys, other kinds) passes through unchanged. Points never change.
//! * `convex` off: simple corner-chunk fan. Corner list `L = [c0 .. cn-1]`; while `|L| > numsides`: emit `[L0 .. L(k-1)]`
//!   (`k = numsides`) and drop `L1 .. L(k-2)`. For `k = 3` this is the plain fan `(c0,ci,ci+1)`: a quad `[a,b,c,d]` gives
//!   `[a,b,c]` and `[a,c,d]`. Exact (area-preserving) for convex polys only.
//! * `convex` on (default): a poly that is convex (all Newell-projected turns the same way) is split as above; a concave one is
//!   ear-clipped into triangles first (area-preserving; falls back to the fan for degenerate / self-touching input).
//!   So `convex` on + `numsides` 3 always triangulates; pieces of a concave poly are triangles even when `numsides` > 3.
//! Corners keep their winding. Each piece copies the prim attrs / group membership of its parent prim, and every piece
//! vertex copies the vertex attrs of its source vertex. Nothing split = input `Arc` shared.
//! NOT modelled (DEFERRED): `bricker` (brick/rect decomposition), `removeshared`, `avoidsmallangles`, `usemaxsides`
//! variants, polys with holes, non-planar fitting.
use super::meshutil::{MAX_ELEMS, Remap, V3, bad, closed_poly_pts, cross, dot, failed, int, positions, rebuild, sub, toggle};
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const MAX_SIDES: i32 = 1024;
/// Ear clipping is O(n^2) per poly; longer polys use the fan.
const MAX_EAR_CLIP: usize = 2048;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("convex", ParamValue::B(true)).label("Convex Polygons"),
        ParamSpec::new("numsides", ParamValue::I(3)).label("Maximum Sides").range(3.0, 12.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Divide { specs: spec() })
}
struct Divide {
    specs: Vec<ParamSpec>,
}

/// Corner-chunk fan over positions in `idx` (indices into the poly's corner list).
fn chunk_fan(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut l: Vec<usize> = (0..n).collect();
    let mut out = Vec::new();
    while l.len() > k {
        out.push(l.iter().take(k).copied().collect());
        let keep: Vec<usize> = l.iter().enumerate().filter(|&(i, _)| i == 0 || i >= k - 1).map(|(_, &c)| c).collect();
        l = keep;
    }
    out.push(l);
    out
}

/// Dominant-axis projection of the corner positions plus the sign that makes the poly counter-clockwise.
fn project(pts: &[V3]) -> Option<(Vec<[f64; 2]>, f64)> {
    let o = *pts.first()?;
    let mut nw = [0.0; 3];
    for w in pts.windows(2) {
        let c = cross(sub(w[0], o), sub(w[1], o));
        nw = [nw[0] + c[0], nw[1] + c[1], nw[2] + c[2]];
    }
    if dot(nw, nw) < 1e-24 {
        return None;
    }
    let ax = (0..3).max_by(|&a, &b| nw[a].abs().total_cmp(&nw[b].abs()))?;
    let (u, v) = ((ax + 1) % 3, (ax + 2) % 3); // right-handed pair about `ax`
    let flat = pts.iter().map(|p| [p[u], p[v]]).collect();
    Some((flat, if nw[ax] >= 0.0 { 1.0 } else { -1.0 }))
}
fn turn(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn is_convex(flat: &[[f64; 2]], sign: f64) -> bool {
    let n = flat.len();
    (0..n).all(|i| turn(flat[i], flat[(i + 1) % n], flat[(i + 2) % n]) * sign >= -1e-12)
}

/// Ear-clip into triangles (corner indices); `None` when no ear is found (degenerate input).
fn ear_clip(flat: &[[f64; 2]], sign: f64) -> Option<Vec<Vec<usize>>> {
    let mut l: Vec<usize> = (0..flat.len()).collect();
    let mut out = Vec::new();
    while l.len() > 3 {
        let m = l.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (l[(i + m - 1) % m], l[i], l[(i + 1) % m]);
            if turn(flat[a], flat[b], flat[c]) * sign <= 1e-12 {
                return false;
            }
            !l.iter().any(|&q| {
                q != a && q != b && q != c && {
                    let (s1, s2, s3) = (
                        turn(flat[a], flat[b], flat[q]) * sign,
                        turn(flat[b], flat[c], flat[q]) * sign,
                        turn(flat[c], flat[a], flat[q]) * sign,
                    );
                    s1 >= -1e-12 && s2 >= -1e-12 && s3 >= -1e-12
                }
            })
        })?;
        out.push(vec![l[(ear + m - 1) % m], l[ear], l[(ear + 1) % m]]);
        l.remove(ear);
    }
    out.push(l);
    Some(out)
}

impl Node<Detail> for Divide {
    fn type_name(&self) -> &'static str {
        "divide"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let convex = toggle(ctx.params, "convex")?;
        let k = match int(ctx.params, "numsides")? {
            k @ 3..=MAX_SIDES => k as usize,
            k => return Err(bad(format!("divide: `numsides` {k} out of range 3..={MAX_SIDES}"))),
        };
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = positions(src);
        let (mut vertex_point, mut prims) = (Vec::<u32>::new(), Vec::<Prim>::new());
        let (mut vert_src, mut prim_src) = (Vec::<Option<usize>>::new(), Vec::<Option<usize>>::new());
        let mut split = false;
        for (qi, pr) in src.prims.iter().enumerate() {
            let lo = pr.verts.start as usize;
            let corners = closed_poly_pts(src, pr);
            let pieces: Vec<Vec<usize>> = match corners.as_ref().filter(|c| c.len() > k || (convex && c.len() > 3)) {
                Some(c) => {
                    let cp: Vec<V3> = c.iter().filter_map(|&p| pos.get(p).copied()).collect();
                    let flat = project(&cp).filter(|_| convex && cp.len() <= MAX_EAR_CLIP);
                    match flat {
                        Some((f, s)) if !is_convex(&f, s) => ear_clip(&f, s).unwrap_or_else(|| chunk_fan(c.len(), 3)),
                        _ if c.len() > k => chunk_fan(c.len(), k),
                        _ => vec![(0..c.len()).collect()],
                    }
                }
                None => vec![],
            };
            if pieces.len() > 1 {
                split = true;
            }
            if pieces.len() <= 1 {
                // pass through verbatim (invalid prims keep their vertex ids as they were)
                let start = u32::try_from(vertex_point.len()).map_err(|_| failed("divide", "vertex id overflow"))?;
                let range = pr.verts.start as usize..pr.verts.end as usize;
                for v in range {
                    vertex_point.push(src.vertex_point.get(v).copied().unwrap_or(0));
                    vert_src.push(Some(v));
                }
                let end = u32::try_from(vertex_point.len()).map_err(|_| failed("divide", "vertex id overflow"))?;
                prims.push(Prim { kind: pr.kind.clone(), verts: start..end });
                prim_src.push(Some(qi));
                continue;
            }
            let Some(c) = corners else { continue };
            for piece in pieces {
                let start = u32::try_from(vertex_point.len()).map_err(|_| failed("divide", "vertex id overflow"))?;
                for ci in piece {
                    vertex_point.push(c.get(ci).and_then(|&p| u32::try_from(p).ok()).unwrap_or(0));
                    vert_src.push(Some(lo + ci));
                }
                let end = u32::try_from(vertex_point.len()).map_err(|_| failed("divide", "vertex id overflow"))?;
                prims.push(Prim { kind: PrimKind::Poly { closed: true }, verts: start..end });
                prim_src.push(Some(qi));
            }
            if u64::try_from(vertex_point.len()).unwrap_or(u64::MAX) > MAX_ELEMS {
                return Err(failed("divide", "output too large"));
            }
        }
        if !split {
            return Ok(Arc::clone(src));
        }
        let rm = Remap { point_src: (0..pos.len()).map(|i| vec![i]).collect(), vert_src, prim_src };
        let p = src.p().to_vec();
        Ok(Arc::new(rebuild("divide", src, vertex_point, prims, &rm, p)?))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, prim_pts};
    use super::*;
    use hexe_geo::{AttrClass, AttrData, Attribute, TypeInfo};
    use hexe_graph::ParamSet;

    fn with(convex: bool, k: i32) -> ParamSet {
        defaults(&*new()).with("convex", ParamValue::B(convex)).with("numsides", ParamValue::I(k))
    }
    fn poly(pts: &[[f32; 3]]) -> Detail {
        let mut d = Detail::default();
        d.add_points(pts.len() as u32);
        *d.p_mut() = pts.to_vec();
        let ids: Vec<u32> = (0..pts.len() as u32).collect();
        d.add_poly(&ids, true);
        d
    }
    /// Signed area of prim `i` in the XY plane (shoelace).
    fn area_xy(d: &Detail, i: usize) -> f32 {
        let q = prim_pts(d, i);
        (0..q.len()).map(|k| q[k][0] * q[(k + 1) % q.len()][1] - q[(k + 1) % q.len()][0] * q[k][1]).sum::<f32>() / 2.0
    }
    fn l_shape() -> Detail {
        poly(&[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 2.0, 0.0]])
    }

    /// Box 6 quads -> 12 triangles; points untouched; the quad [a,b,c,d] becomes [a,b,c] and [a,c,d].
    #[test]
    fn box_six_quads_become_twelve_triangles() {
        let src = cube();
        let out = cook(&*new(), src.clone(), &defaults(&*new())).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (8, 12));
        assert_eq!(out.p(), src.p());
        assert!(out.prims.iter().all(|p| p.verts.len() == 3 && p.kind == PrimKind::Poly { closed: true }));
        for q in 0..6 {
            let (a, b) = (prim_pts(&src, q), (prim_pts(&out, 2 * q), prim_pts(&out, 2 * q + 1)));
            assert_eq!(b.0, vec![a[0], a[1], a[2]]);
            assert_eq!(b.1, vec![a[0], a[2], a[3]], "winding kept");
        }
    }

    #[test]
    fn attrs_follow_their_source_and_small_polys_pass() {
        let mut d = cube();
        d.add_attr(Attribute {
            name: "pid".into(),
            class: AttrClass::Prim,
            data: AttrData::I32((0..6).collect()),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "vv".into(),
            class: AttrClass::Vertex,
            data: AttrData::F32((0..24).map(|i| i as f32).collect()),
            type_info: TypeInfo::None,
        });
        let out = cook(&*new(), d.clone(), &defaults(&*new())).expect("cooks");
        let Some(AttrData::I32(pid)) = out.attr(AttrClass::Prim, "pid").map(|a| &a.data) else { panic!("pid") };
        assert_eq!(pid, &vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5]);
        let Some(AttrData::F32(vv)) = out.attr(AttrClass::Vertex, "vv").map(|a| &a.data) else { panic!("vv") };
        assert_eq!(&vv[..6], &[0.0, 1.0, 2.0, 0.0, 2.0, 3.0], "quad 0 corners (0,1,2),(0,2,3)");
        // quads fit numsides 4 and are convex: nothing to do, Arc shared
        let a = Arc::new(d);
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &with(true, 4)).expect("cooks"), &a));
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &with(false, 4)).expect("cooks"), &a));
    }

    #[test]
    fn hexagon_chunks_and_open_polys_pass_through() {
        let hex: Vec<[f32; 3]> = (0..6)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / 6.0;
                [t.cos(), t.sin(), 0.0]
            })
            .collect();
        let tri = cook(&*new(), poly(&hex), &with(true, 3)).expect("cooks");
        assert_eq!(tri.prim_count(), 4, "n-2 triangles");
        let total: f32 = (0..4).map(|i| area_xy(&tri, i)).sum();
        assert!((total - 1.5 * 3.0f32.sqrt()).abs() < 1e-5, "regular hexagon area 3 sqrt3 / 2");
        let quads = cook(&*new(), poly(&hex), &with(true, 4)).expect("cooks");
        assert_eq!(quads.prims.iter().map(|p| p.verts.len()).collect::<Vec<_>>(), vec![4, 4]);
        assert_eq!(prim_pts(&quads, 0), hex[0..4].to_vec());
        assert_eq!(prim_pts(&quads, 1), vec![hex[0], hex[3], hex[4], hex[5]]);
        // open polyline with 6 points is not a polygon
        let mut line = poly(&hex);
        line.prims[0].kind = PrimKind::Poly { closed: false };
        let a = Arc::new(line);
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &with(true, 3)).expect("cooks"), &a));
    }

    /// L-shape (area 3): convex on ear-clips into 4 triangles of total area exactly 3, all positively wound.
    #[test]
    fn concave_l_shape_is_ear_clipped_exactly() {
        for k in [3, 6] {
            let out = cook(&*new(), l_shape(), &with(true, k)).expect("cooks");
            assert_eq!(out.prim_count(), 4, "k={k}: 4 triangles even though 6 sides are allowed");
            let areas: Vec<f32> = (0..4).map(|i| area_xy(&out, i)).collect();
            assert!(areas.iter().all(|&a| a > 0.0), "{areas:?}");
            assert_eq!(areas.iter().sum::<f32>(), 3.0);
        }
        // convex off with numsides 6 leaves the 6-gon alone
        let a = Arc::new(l_shape());
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &with(false, 6)).expect("cooks"), &a));
    }

    #[test]
    fn hostile_params_and_degenerate_polys() {
        for p in [with(true, 2), with(true, 1025), ParamSet::new()] {
            assert!(matches!(cook(&*new(), cube(), &p), Err(CookError::BadParam(_))));
        }
        assert_eq!(cook(&*new(), Detail::default(), &defaults(&*new())).expect("cooks").prim_count(), 0);
        // collinear 5-gon: no ear exists / zero normal -> fan fallback, no panic
        let line: Vec<[f32; 3]> = (0..5).map(|i| [i as f32, 0.0, 0.0]).collect();
        assert_eq!(cook(&*new(), poly(&line), &with(true, 3)).expect("cooks").prim_count(), 3);
        // dangling vertex ids are passed through, not crashed on
        let mut d = cube();
        d.vertex_point.push(99);
        d.prims.push(Prim { kind: PrimKind::Poly { closed: true }, verts: 24..25 });
        assert_eq!(cook(&*new(), d, &defaults(&*new())).expect("cooks").prim_count(), 13);
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/divide.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (8, 12));
        assert_eq!(crate::registry().create("divide").map(|n| n.type_name()), Some("divide"));
    }
}
