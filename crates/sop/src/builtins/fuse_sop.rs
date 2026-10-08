//! Fuse SOP (token `fuse`): welds points that lie within `distance` of each other.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; closed-form oracle only)
//! * Clustering (uniform grid hash, cell = `distance`, O(n) expected): points are visited in ascending point number;
//!   an unclaimed point becomes a LEADER and flood-claims every still-unclaimed selected point within `distance` (Euclidean,
//!   `<=`) of ANY point already in its cluster: TRANSITIVE (connected components), so a chain A-B-C with A-C > `distance`
//!   but A-B, B-C <= `distance` yields one cluster {A,B,C}. `distance` 0 merges
//!   exactly coincident points only. Points with non-finite `P` never merge.
//! * Output points = one per cluster, ordered by leader (lowest point number) -> unmerged points keep relative order.
//! * `snap` menu `average | lowest-point` (0 | 1, default average): average = float point attributes (incl. `P`) are the
//!   arithmetic mean over the cluster; lowest-point = every attribute copies the leader. Int / string attributes always
//!   copy the leader. Point groups: a cluster is in a group when any member was.
//! * Vertices are repointed. A prim touching a merged cluster drops consecutive duplicate vertices (cyclically for closed
//!   polys), then is DELETED when it has `< 3` (closed poly) / `< 2` (open poly, polyline) corners left. Other prim kinds
//!   are repointed, never deleted. Prims that touch no merged cluster are copied verbatim. Vertex / prim attributes and
//!   groups follow the surviving vertices / prims. Detail attributes are kept.
//! * `group`: space-separated POINT group names (union), empty = all points. Unknown name -> `CookError::Failed`.
//! * Nothing merges -> the input `Arc` is shared. Hostile: negative / non-finite `distance`, menu out of range ->
//!   `BadParam`.
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail, Group, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::HashMap;
use std::sync::Arc;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
/// `snap` menu indices.
const SNAP_AVERAGE: u32 = 0;
const SNAP_LOWEST: u32 = 1;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("distance", ParamValue::F(0.001)).label("Distance").range(0.0, 1.0),
        ParamSpec::new("snap", ParamValue::Menu(SNAP_AVERAGE)).label("Snap"),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Fuse { specs: spec() })
}
struct Fuse {
    specs: Vec<ParamSpec>,
}
fn bad(m: impl Into<String>) -> CookError {
    CookError::BadParam(format!("fuse: {}", m.into()))
}
fn failed(m: &str) -> CookError {
    CookError::Failed(format!("fuse: {m}"))
}
type Cell = [i64; 3];
fn cell_of(q: [f64; 3], size: f64) -> Cell {
    // `as` saturates for huge ratios; neighbours use saturating_add below
    [(q[0] / size).floor() as i64, (q[1] / size).floor() as i64, (q[2] / size).floor() as i64]
}
/// Clusters as ascending-by-leader lists of ascending point numbers. TRANSITIVE: connected components of the "within
/// `dist`" graph (flood fill over the grid hash; each point is claimed once and leaves its cell list, so a dense cluster
/// costs O(members)).
fn cluster(pos: &[[f32; 3]], selected: &[bool], dist: f64) -> Vec<Vec<u32>> {
    let size = if dist > 0.0 { dist } else { 1.0 };
    let d2 = dist * dist;
    let at = |i: usize| pos.get(i).map(|q| [f64::from(q[0]), f64::from(q[1]), f64::from(q[2])]);
    let usable =
        |i: usize| selected.get(i).copied().unwrap_or(false) && at(i).is_some_and(|q| q.iter().all(|c| c.is_finite()));
    let mut grid: HashMap<Cell, Vec<u32>> = HashMap::new();
    for i in 0..selected.len() {
        if let (true, Some(q), Ok(id)) = (usable(i), at(i), u32::try_from(i)) {
            grid.entry(cell_of(q, size)).or_default().push(id);
        }
    }
    let mut claimed = vec![false; selected.len()];
    let mut out: Vec<Vec<u32>> = Vec::new();
    for i in 0..selected.len() {
        if claimed.get(i).copied().unwrap_or(true) {
            continue;
        }
        claimed[i] = true;
        let Ok(id) = u32::try_from(i) else { continue };
        let mut members = vec![id];
        if usable(i) {
            let mut queue = vec![id];
            while let Some(cur) = queue.pop() {
                let Some(q) = at(cur as usize) else { continue };
                let c = cell_of(q, size);
                for dx in -1i64..=1 {
                    for dy in -1i64..=1 {
                        for dz in -1i64..=1 {
                            let key = [c[0].saturating_add(dx), c[1].saturating_add(dy), c[2].saturating_add(dz)];
                            let Some(list) = grid.get_mut(&key) else { continue };
                            list.retain(|&j| !claimed.get(j as usize).copied().unwrap_or(true));
                            let hits: Vec<u32> = list
                                .iter()
                                .copied()
                                .filter(|&j| {
                                    at(j as usize).is_some_and(|r| {
                                        (r[0] - q[0]).powi(2) + (r[1] - q[1]).powi(2) + (r[2] - q[2]).powi(2) <= d2
                                    })
                                })
                                .collect();
                            for &j in &hits {
                                if let Some(f) = claimed.get_mut(j as usize) {
                                    *f = true;
                                }
                            }
                            members.extend_from_slice(&hits);
                            queue.extend_from_slice(&hits);
                        }
                    }
                }
            }
            members.sort_unstable();
        }
        out.push(members);
    }
    out
}
/// Per-cluster reduction of a point attribute (mean for floats when `average`, else the leader's value).
fn reduce(d: &AttrData, cl: &[Vec<u32>], average: bool) -> AttrData {
    fn lead<T: Copy>(v: &[T], cl: &[Vec<u32>], def: T) -> Vec<T> {
        cl.iter().map(|m| m.first().and_then(|&i| v.get(i as usize).copied()).unwrap_or(def)).collect()
    }
    fn mean<const N: usize>(v: &[[f32; N]], cl: &[Vec<u32>], average: bool) -> Vec<[f32; N]> {
        cl.iter()
            .map(|m| {
                let first = m.first().and_then(|&i| v.get(i as usize).copied()).unwrap_or([0.0; N]);
                if !average || m.len() < 2 {
                    return first;
                }
                let mut s = [0.0f64; N];
                for &i in m {
                    let e = v.get(i as usize).copied().unwrap_or([0.0; N]);
                    for (a, b) in s.iter_mut().zip(e) {
                        *a += f64::from(b);
                    }
                }
                s.map(|a| (a / m.len() as f64) as f32)
            })
            .collect()
    }
    fn mean_flat(v: &[f32], size: usize, cl: &[Vec<u32>], average: bool) -> Vec<f32> {
        let mut o = Vec::with_capacity(cl.len().saturating_mul(size));
        for m in cl {
            for k in 0..size {
                let at = |i: u32| {
                    (i as usize).checked_mul(size).and_then(|b| v.get(b.checked_add(k)?)).copied().unwrap_or(0.0)
                };
                let first = m.first().map_or(0.0, |&i| at(i));
                o.push(if !average || m.len() < 2 {
                    first
                } else {
                    (m.iter().map(|&i| f64::from(at(i))).sum::<f64>() / m.len() as f64) as f32
                });
            }
        }
        o
    }
    match d {
        AttrData::F32(v) => AttrData::F32(
            mean(&v.iter().map(|x| [*x]).collect::<Vec<_>>(), cl, average).into_iter().map(|x| x[0]).collect(),
        ),
        AttrData::V2(v) => AttrData::V2(mean(v, cl, average)),
        AttrData::V3(v) => AttrData::V3(mean(v, cl, average)),
        AttrData::V4(v) => AttrData::V4(mean(v, cl, average)),
        AttrData::FN { size, data } => AttrData::FN { size: *size, data: mean_flat(data, *size as usize, cl, average) },
        AttrData::I32(v) => AttrData::I32(lead(v, cl, 0)),
        AttrData::Str { idx, table } => AttrData::Str { idx: lead(idx, cl, -1), table: table.clone() },
        AttrData::IN { size, data } => {
            let s = *size as usize;
            let mut o = Vec::with_capacity(cl.len().saturating_mul(s));
            for m in cl {
                for k in 0..s {
                    let pick =
                        m.first().and_then(|&i| (i as usize).checked_mul(s)?.checked_add(k)).and_then(|j| data.get(j));
                    o.push(pick.copied().unwrap_or(0));
                }
            }
            AttrData::IN { size: *size, data: o }
        }
    }
}
/// Element `idx[i]` of `d` for every output element.
fn gather(d: &AttrData, idx: &[usize]) -> AttrData {
    fn pick<T: Copy>(v: &[T], idx: &[usize], def: T) -> Vec<T> {
        idx.iter().map(|&i| v.get(i).copied().unwrap_or(def)).collect()
    }
    fn pick_n<T: Copy>(v: &[T], size: usize, idx: &[usize], def: T) -> Vec<T> {
        let mut o = Vec::with_capacity(idx.len().saturating_mul(size));
        for &i in idx {
            let chunk = i.checked_mul(size).and_then(|b| v.get(b..b.checked_add(size)?));
            match chunk {
                Some(c) => o.extend_from_slice(c),
                None => o.extend(std::iter::repeat_n(def, size)),
            }
        }
        o
    }
    match d {
        AttrData::F32(v) => AttrData::F32(pick(v, idx, 0.0)),
        AttrData::I32(v) => AttrData::I32(pick(v, idx, 0)),
        AttrData::V2(v) => AttrData::V2(pick(v, idx, [0.0; 2])),
        AttrData::V3(v) => AttrData::V3(pick(v, idx, [0.0; 3])),
        AttrData::V4(v) => AttrData::V4(pick(v, idx, [0.0; 4])),
        AttrData::Str { idx: si, table } => AttrData::Str { idx: pick(si, idx, -1), table: table.clone() },
        AttrData::FN { size, data } => AttrData::FN { size: *size, data: pick_n(data, *size as usize, idx, 0.0) },
        AttrData::IN { size, data } => AttrData::IN { size: *size, data: pick_n(data, *size as usize, idx, 0) },
    }
}
impl Node<Detail> for Fuse {
    fn type_name(&self) -> &'static str {
        "fuse"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let dist = match p.float("distance") {
            Some(v) if v.is_finite() && v >= 0.0 => f64::from(v),
            _ => return Err(bad("`distance`: missing or not a finite float >= 0")),
        };
        let snap = match p.menu("snap") {
            Some(v) if v <= SNAP_LOWEST => v,
            _ => return Err(bad("`snap`: missing or menu index out of range 0..=1")),
        };
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let np = src.point_count as usize;
        let names: Vec<&str> = p.string("group").unwrap_or("").split_whitespace().collect();
        let mut sel = vec![names.is_empty(); np];
        for n in names {
            let g: &Group =
                src.group(AttrClass::Point, n).ok_or_else(|| failed(&format!("unknown point group `{n}`")))?;
            for (s, m) in sel.iter_mut().zip(&g.mask) {
                *s |= *m;
            }
        }
        let cl = cluster(src.p(), &sel, dist);
        if cl.iter().all(|m| m.len() < 2) {
            return Ok(Arc::clone(src));
        }
        // old point -> new point; merged flag per old point
        let mut new_of = vec![0u32; np];
        let mut merged = vec![false; np];
        for (k, m) in cl.iter().enumerate() {
            let k = u32::try_from(k).map_err(|_| failed("point id overflow"))?;
            for &i in m {
                if let (Some(n), Some(f)) = (new_of.get_mut(i as usize), merged.get_mut(i as usize)) {
                    *n = k;
                    *f = m.len() > 1;
                }
            }
        }
        // ---- topology ----
        let mut vertex_point: Vec<u32> = Vec::new();
        let (mut vert_src, mut prim_src): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
        let mut prims: Vec<Prim> = Vec::new();
        for (qi, pr) in src.prims.iter().enumerate() {
            let range = usize::try_from(pr.verts.start).unwrap_or(0)..usize::try_from(pr.verts.end).unwrap_or(0);
            let old = src.vertex_point.get(range.clone()).unwrap_or(&[]);
            let mapped: Vec<(u32, usize)> = old
                .iter()
                .zip(range.clone())
                .map(|(&pt, v)| (new_of.get(pt as usize).copied().unwrap_or(pt), v))
                .collect();
            let touched = old.iter().any(|&pt| merged.get(pt as usize).copied().unwrap_or(false));
            let (closed, collapsible) = match pr.kind {
                PrimKind::Poly { closed } => (closed, true),
                PrimKind::Polyline => (false, true),
                _ => (false, false),
            };
            let mut keep = mapped;
            if touched && collapsible {
                keep.dedup_by_key(|e| e.0);
                if closed && keep.len() > 1 && keep.first().map(|e| e.0) == keep.last().map(|e| e.0) {
                    keep.pop();
                }
                if keep.len() < if closed { 3 } else { 2 } {
                    continue;
                }
            }
            let start = u32::try_from(vertex_point.len()).map_err(|_| failed("vertex id overflow"))?;
            for (pt, v) in &keep {
                vertex_point.push(*pt);
                vert_src.push(*v);
            }
            let end = u32::try_from(vertex_point.len()).map_err(|_| failed("vertex id overflow"))?;
            prims.push(Prim { kind: pr.kind.clone(), verts: start..end });
            prim_src.push(qi);
        }
        // ---- attributes / groups ----
        let average = snap == SNAP_AVERAGE;
        let mut out = Detail {
            point_count: u32::try_from(cl.len()).map_err(|_| failed("point id overflow"))?,
            vertex_point,
            prims,
            attrs: Vec::new(),
            groups: Vec::new(),
            skipped_attribs: src.skipped_attribs.clone(),
        };
        for a in &src.attrs {
            let mut a = a.clone();
            a.data = match a.class {
                AttrClass::Point => reduce(&a.data, &cl, average),
                AttrClass::Vertex => gather(&a.data, &vert_src),
                AttrClass::Prim => gather(&a.data, &prim_src),
                AttrClass::Detail => a.data,
            };
            out.attrs.push(a);
        }
        for g in &src.groups {
            let mut g = g.clone();
            let at = |m: &[bool], i: usize| m.get(i).copied().unwrap_or(false);
            g.mask = match g.class {
                AttrClass::Point => cl.iter().map(|m| m.iter().any(|&i| at(&g.mask, i as usize))).collect(),
                AttrClass::Vertex => vert_src.iter().map(|&i| at(&g.mask, i)).collect(),
                AttrClass::Prim => prim_src.iter().map(|&i| at(&g.mask, i)).collect(),
                AttrClass::Detail => g.mask,
            };
            out.groups.push(g);
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::{Attribute, TypeInfo};
    use hexe_graph::ParamSet;
    fn cook(d: Arc<Detail>, dist: f32, snap: u32, group: &str) -> Result<Arc<Detail>, CookError> {
        let p = ParamSet::new()
            .with("distance", ParamValue::F(dist))
            .with("snap", ParamValue::Menu(snap))
            .with("group", ParamValue::S(group.into()));
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        new().cook(&ctx, &[d])
    }
    fn fuse(d: Detail, dist: f32) -> Arc<Detail> {
        cook(Arc::new(d), dist, SNAP_AVERAGE, "").expect("cooks")
    }
    /// Unit box as 6 separate quads (24 points), outward, Houdini winding.
    fn soup_box() -> Detail {
        let mut d = Detail::default();
        let c = |b: u32, i: u32| if (i >> b) & 1 == 1 { 0.5f32 } else { -0.5 };
        let corner = |i: u32| [c(2, i), c(1, i), c(0, i)];
        let mut pos = Vec::new();
        for f in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
 pos.extend(f.iter().map(|&i| corner(i)));
        }
        d.add_points(24);
        *d.p_mut() = pos;
        for q in 0..6u32 {
            d.add_poly(&[4 * q, 4 * q + 1, 4 * q + 2, 4 * q + 3], true);
        }
        d
    }
    fn two_quads_8_points() -> Detail {
        let mut d = Detail::default();
        d.add_points(8);
        *d.p_mut() = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        d.add_poly(&[0, 1, 2, 3], true);
        d.add_poly(&[4, 5, 6, 7], true);
        d
    }
    fn grid3(size: f32) -> Detail {
        let json = format!(
            r#"{{"nodes":[{{"name":"g","type":"grid","params":{{"rows":3,"cols":3,"size":[{size},{size},0]}}}}],"display":"g"}}"#
        );
        let (mut g, id) = load_scene(&json).expect("loads");
        (*g.cook(id, 0.0).expect("cooks")).clone()
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "fuse");
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("distance", ParamValue::F(0.001)),
                ("snap", ParamValue::Menu(SNAP_AVERAGE)),
                ("group", ParamValue::S(String::new()))
            ]
        );
    }
    #[test]
    fn two_quads_sharing_an_edge_with_8_points_fuse_to_6_points_2_prims() {
        let out = fuse(two_quads_8_points(), 0.001);
        assert_eq!((out.point_count, out.prim_count()), (6, 2));
        // shared edge = points {1,2} of quad 0 are quad 1's {0,3}
        let ids = |i: usize| -> Vec<u32> {
            let pr = &out.prims[i];
            out.vertex_point[pr.verts.start as usize..pr.verts.end as usize].to_vec()
        };
        let (a, b) = (ids(0), ids(1));
        assert_eq!((a[1], a[2]), (b[0], b[3]));
        // positions preserved exactly, leader order: 0 1 2 3 then 5 6
        assert_eq!(out.p()[4], [2.0, 0.0, 0.0]);
        assert_eq!(out.p()[5], [2.0, 1.0, 0.0]);
    }
    #[test]
    fn box_from_6_separate_quads_24_points_fuses_to_8() {
        let src = soup_box();
        let out = fuse(src.clone(), 0.001);
        assert_eq!((out.point_count, out.prim_count()), (8, 6));
        let mut seen: Vec<[f32; 3]> = out.p().to_vec();
        seen.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        seen.dedup();
        assert_eq!(seen.len(), 8, "8 distinct corners");
        // each prim still reads the same positions as before
        for i in 0..6 {
            let pos = |d: &Detail| -> Vec<[f32; 3]> {
                let pr = &d.prims[i];
                d.vertex_point[pr.verts.start as usize..pr.verts.end as usize]
                    .iter()
                    .map(|&p| d.p()[p as usize])
                    .collect()
            };
            assert_eq!(pos(&out), pos(&src), "prim {i}");
        }
    }
    #[test]
    fn distance_0_on_already_shared_geometry_is_unchanged_and_shares_the_arc() {
        let mut shared = Detail::default();
        shared.add_points(4);
        *shared.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        shared.add_poly(&[0, 1, 2, 3], true);
        let a = Arc::new(shared);
        for snap in [SNAP_AVERAGE, SNAP_LOWEST] {
            let out = cook(Arc::clone(&a), 0.0, snap, "").expect("cooks");
            assert!(Arc::ptr_eq(&out, &a));
        }
        // default distance too (nothing within 0.001)
        assert!(Arc::ptr_eq(&cook(Arc::clone(&a), 0.001, SNAP_AVERAGE, "").expect("cooks"), &a));
        // distance 0 merges exactly coincident points only
        let out = cook(Arc::new(two_quads_8_points()), 0.0, SNAP_AVERAGE, "").expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (6, 2));
        let near = {
            let mut d = two_quads_8_points();
            d.p_mut()[4] = [1.0001, 0.0, 0.0];
            d
        };
        let out = cook(Arc::new(near), 0.0, SNAP_AVERAGE, "").expect("cooks");
        assert_eq!(out.point_count, 7);
    }
    #[test]
    fn grid_3x3_with_distance_10_collapses_to_1_point_0_prims() {
        let g = grid3(2.0);
        assert_eq!((g.point_count, g.prim_count()), (9, 4));
        let out = fuse(g, 10.0);
        assert_eq!((out.point_count, out.prim_count(), out.vertex_point.len()), (1, 0, 0));
        // mean of the symmetric lattice is the centre
        let c = out.p()[0];
        assert!(c.iter().all(|v| v.abs() < 1e-6), "{c:?}");
    }
    #[test]
    fn partial_merge_drops_only_degenerate_prims_and_keeps_triangles() {
        // grid spacing 1: distance 1.01 -> leader-centred claims; result must contain no prim with < 3 unique pts
        let out = fuse(grid3(2.0), 1.01);
        assert!(out.point_count < 9 && out.point_count >= 1);
        for pr in &out.prims {
            let mut v = out.vertex_point[pr.verts.start as usize..pr.verts.end as usize].to_vec();
            assert!(v.len() >= 3);
            v.sort_unstable();
            v.dedup();
            assert_eq!(v.len(), pr.verts.len(), "duplicate vertex survived");
        }
        // a quad with two adjacent corners welded becomes a triangle
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [0.0005, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2, 3], true);
        let out = fuse(d, 0.001);
        assert_eq!((out.point_count, out.prim_count(), out.vertex_point.len()), (3, 1, 3));
    }
    #[test]
    fn snap_average_vs_lowest_point_and_attribute_reduction() {
        let mut d = Detail::default();
        d.add_points(3);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [0.0004, 0.0, 0.0], [5.0, 0.0, 0.0]];
        d.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]]),
            type_info: TypeInfo::Color,
        });
        d.add_attr(Attribute {
            name: "id".into(),
            class: AttrClass::Point,
            data: AttrData::I32(vec![7, 8, 9]),
            type_info: TypeInfo::None,
        });
        let d = Arc::new(d);
        let avg = cook(Arc::clone(&d), 0.001, SNAP_AVERAGE, "").expect("cooks");
        assert_eq!(avg.point_count, 2);
        assert!((avg.p()[0][0] - 0.0002).abs() < 1e-9 && avg.p()[1] == [5.0, 0.0, 0.0]);
        let Some(AttrData::V3(cd)) = avg.attr(AttrClass::Point, "Cd").map(|a| &a.data) else { panic!("Cd") };
        assert_eq!(cd[0], [0.5, 0.0, 0.5]);
        assert_eq!(cd[1], [0.0, 1.0, 0.0]);
        let Some(AttrData::I32(id)) = avg.attr(AttrClass::Point, "id").map(|a| &a.data) else { panic!("id") };
        assert_eq!(id, &[7, 9], "ints copy the lowest point");
        let low = cook(d, 0.001, SNAP_LOWEST, "").expect("cooks");
        assert_eq!(low.p()[0], [0.0, 0.0, 0.0]);
        let Some(AttrData::V3(cd)) = low.attr(AttrClass::Point, "Cd").map(|a| &a.data) else { panic!("Cd") };
        assert_eq!(cd[0], [1.0, 0.0, 0.0]);
    }
    #[test]
    fn clustering_is_transitive_union_of_chained_neighbours() {
        // a<->b and b<->c within distance, a<->c NOT: one point (mean = b), regardless of point order
        for order in [[0usize, 1, 2], [2, 0, 1], [1, 2, 0]] {
            let base = [[0.0, 0.0, 0.0], [0.0008, 0.0, 0.0], [0.0016, 0.0, 0.0]];
            let mut d = Detail::default();
            d.add_points(3);
            *d.p_mut() = order.iter().map(|&i| base[i]).collect();
            let out = fuse(d, 0.001);
            assert_eq!(out.point_count, 1, "order {order:?}");
            assert!((out.p()[0][0] - 0.0008).abs() < 1e-7);
        }
        // a gap > distance still separates: 0, 0.0008, | 0.0030
        let mut d = Detail::default();
        d.add_points(3);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [0.0008, 0.0, 0.0], [0.003, 0.0, 0.0]];
        assert_eq!(fuse(d, 0.001).point_count, 2);
    }
    #[test]
    fn group_restricts_fusing_vertex_prim_attrs_follow() {
        let mut d = two_quads_8_points();
        d.groups.push(Group { name: "a".into(), class: AttrClass::Point, mask: (0..8).map(|i| i < 4).collect() });
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Prim,
            data: AttrData::F32(vec![1.0, 2.0]),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "vi".into(),
            class: AttrClass::Vertex,
            data: AttrData::I32((0..8).collect()),
            type_info: TypeInfo::None,
        });
        let d = Arc::new(d);
        // group a = quad 0's own points: nothing coincides inside it -> shared Arc
        assert!(Arc::ptr_eq(&cook(Arc::clone(&d), 0.001, SNAP_AVERAGE, "a").expect("cooks"), &d));
        let all = cook(Arc::clone(&d), 0.001, SNAP_AVERAGE, "").expect("cooks");
        let Some(AttrData::F32(w)) = all.attr(AttrClass::Prim, "w").map(|a| &a.data) else { panic!("w") };
        assert_eq!(w, &[1.0, 2.0]);
        let Some(AttrData::I32(vi)) = all.attr(AttrClass::Vertex, "vi").map(|a| &a.data) else { panic!("vi") };
        assert_eq!(vi, &(0..8).collect::<Vec<i32>>());
        let ga = all.group(AttrClass::Point, "a").expect("group kept");
        assert_eq!(ga.mask.len(), 6);
        assert_eq!(ga.mask.iter().filter(|b| **b).count(), 4);
        assert!(matches!(cook(d, 0.001, SNAP_AVERAGE, "nope"), Err(CookError::Failed(_))));
    }
    #[test]
    fn hostile_params_nan_positions_open_prims_and_empty() {
        let a = Arc::new(soup_box());
        for (dist, snap) in [(-1.0, 0), (f32::NAN, 0), (f32::INFINITY, 0), (0.1, 2)] {
            assert!(matches!(cook(Arc::clone(&a), dist, snap, ""), Err(CookError::BadParam(_))), "{dist} {snap}");
        }
        // enormous distance + coordinates must not panic
        let mut big = soup_box();
        big.p_mut()[3] = [f32::MAX, f32::MIN, 0.0];
        let _ = cook(Arc::new(big), f32::MAX, SNAP_AVERAGE, "").expect("cooks");
        // NaN points never merge
        let mut d = Detail::default();
        d.add_points(3);
        *d.p_mut() = vec![[f32::NAN; 3], [f32::NAN; 3], [0.0; 3]];
        d.add_poly(&[0, 1], false);
        let d = Arc::new(d);
        let out = cook(Arc::clone(&d), 1.0, SNAP_AVERAGE, "").expect("cooks");
        assert!(Arc::ptr_eq(&out, &d), "NaN points never merge");
        // open polyline collapsing to one point is deleted; empty / missing input
        let mut l = Detail::default();
        l.add_points(2);
        *l.p_mut() = vec![[0.0; 3], [0.0001, 0.0, 0.0]];
        l.add_poly(&[0, 1], false);
        let out = fuse(l, 0.001);
        assert_eq!((out.point_count, out.prim_count()), (1, 0));
        let p = ParamSet::new().with("distance", ParamValue::F(0.1)).with("snap", ParamValue::Menu(0));
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        assert_eq!(new().cook(&ctx, &[]).expect("cooks").point_count, 0);
        assert_eq!(fuse(Detail::default(), 1.0).point_count, 0);
    }
    #[test]
    fn linear_time_on_a_large_lattice() {
        // 200x200 distinct points, each duplicated: 80_000 points -> 40_000, must finish fast (hash, not n^2)
        let mut d = Detail::default();
        d.add_points(80_000);
        *d.p_mut() = (0..80_000u32).map(|i| [((i / 2) % 200) as f32, ((i / 2) / 200) as f32, 0.0]).collect();
        let t = std::time::Instant::now();
        let out = fuse(d, 0.01);
        assert_eq!(out.point_count, 40_000);
        assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
    }
    #[test]
    fn example_scene_welds_the_extruded_box_soup() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/fuse.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
                // box -> polyextrude 0.5 (connected components = Houdini default: a closed box has no boundary, 8 pts / 6 prims,
        // identical to Houdini) -> fuse 0.001: nothing left to weld
        assert_eq!((out.point_count, out.prim_count(), out.vertex_count()), (8, 6, 24));
        // splittype 1 (individual faces) = the old soup: 48 pts / 30 prims; fuse welds shared corners
        let soup = ex.replace("\"distance\": 0.5", "\"distance\": 0.5, \"splittype\": 1");
        let (mut g, id) = load_scene(&soup).expect("soup loads");
        let out = g.cook(id, 0.0).expect("soup cooks");
        assert_eq!(out.prim_count(), 30);
        assert_eq!(out.point_count, 8 + 24, "24 coincident base corners weld to 8; 24 lifted corners stay distinct");

        assert_eq!(crate::registry().create("fuse").map(|n| n.type_name()), Some("fuse"));
    }
}
