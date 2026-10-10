//! Facet SOP (token `facet`): split shared points, optionally recompute point normals.
//!
//! # Semantics (unique_points order hython-exact since H33; the rest UNVERIFIED vs Houdini)
//! * `unique_points`: every vertex gets its own point (box 24 pts). First vertex of a point keeps the id, later ones get
//!   appended ids (descending source point, ascending vertex; hython-exact). Unused points keep their ids. Wins over `cusp`.
//! * `cusp` + `cusp_angle` (deg, clamped to 0..=180, default 60): the vertices of one point are grouped by LEADER
//!   clustering of their prim's unit Newell normal: a vertex joins the first cluster whose leader normal is within
//!   `cusp_angle` of its own (`dot >= cos(angle)`), else opens a new cluster (at most 64 clusters per point; later
//!   normals fall into the last one). Vertices of prims without a normal (open / degenerate / non-poly) join the first
//!   cluster. Cluster 0 keeps the original point id, further clusters get new ids appended in point order. Box at
//!   60 deg -> 24 pts (faces are 90 deg apart); flat grid -> unchanged (input `Arc` shared when nothing else changes).
//! * `post_nml`: after the split, point `N` (V3, type Normal) = uniform-averaged Newell normal (see `peak`).
//! * `remove_inline`: NOT modelled (skipped by order).
//!
//! Split points copy the point attrs / groups of their source point; vertices, prims and detail data are unchanged.
//! No option on -> the input `Arc` is shared. Output above `MAX_ELEMS` points -> `Failed`.
use super::meshutil::{
    MAX_ELEMS, Remap, V3, closed_poly_pts, dot, failed, float, point_normals, positions, prim_normal, rebuild, to_f32,
    toggle,
};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "facet";
const MAX_CLUSTERS: usize = 64;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("unique_points", ParamValue::B(false)).label("Unique Points"),
        ParamSpec::new("cusp", ParamValue::B(false)).label("Cusp Polygons"),
        ParamSpec::new("cusp_angle", ParamValue::F(60.0)).label("Cusp Angle").range(0.0, 180.0),
        ParamSpec::new("post_nml", ParamValue::B(false)).label("Post-Compute Normals"),
        ParamSpec::new("consolidate", ParamValue::F(0.0)).label("Consolidate Distance").range(0.0, 1.0),
        ParamSpec::new("orient_polygons", ParamValue::B(false)).label("Orient Polygons"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Facet { specs: spec() })
}
struct Facet {
    specs: Vec<ParamSpec>,
}

/// Unit normal of the prim owning each vertex (`None` = no usable normal).
fn vertex_normals(d: &Detail, pos: &[V3]) -> Vec<Option<V3>> {
    let mut out = vec![None; d.vertex_point.len()];
    for pr in &d.prims {
        let Some(pts) = closed_poly_pts(d, pr) else { continue };
        let Some(n) = prim_normal(pos, &pts) else { continue };
        let (lo, hi) = (usize::try_from(pr.verts.start).unwrap_or(0), usize::try_from(pr.verts.end).unwrap_or(0));
        for v in out.get_mut(lo..hi).unwrap_or(&mut []) {
            *v = Some(n);
        }
    }
    out
}

/// Orient polygons consistently via BFS: ensure adjacent polys have opposite winding on shared edges.
fn orient_polygons(d: &mut Detail) {
    use std::collections::{HashMap, VecDeque};

    // Build edge -> prim map for closed polys
    let mut edge_prims: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    let mut poly_indices = Vec::new();

    for (pi, pr) in d.prims.iter().enumerate() {
        if let Some(pts) = closed_poly_pts(d, pr) {
            poly_indices.push(pi);
            let n = pts.len();
            for k in 0..n {
                let (a, b) = (pts[k] as u32, pts[(k + 1) % n] as u32);
                let key = (a.min(b), a.max(b));
                edge_prims.entry(key).or_default().push(pi);
            }
        }
    }

    if poly_indices.is_empty() {
        return;
    }

    let mut oriented = vec![false; d.prims.len()];
    let mut flipped = vec![false; d.prims.len()];

    for seed_idx in poly_indices.iter().copied() {
        if oriented[seed_idx] {
            continue;
        }

        let mut queue = VecDeque::new();
        queue.push_back(seed_idx);
        oriented[seed_idx] = true;

        while let Some(pi) = queue.pop_front() {
            let Some(pts) = closed_poly_pts(d, &d.prims[pi]) else { continue };
            let n = pts.len();

            for k in 0..n {
                let (a, b) = (pts[k] as u32, pts[(k + 1) % n] as u32);
                let key = (a.min(b), a.max(b));

                if let Some(neighbors) = edge_prims.get(&key) {
                    for &ni in neighbors {
                        if ni == pi || oriented[ni] {
                            continue;
                        }

                        // Check if neighbor has opposite winding on shared edge
                        let Some(npts) = closed_poly_pts(d, &d.prims[ni]) else { continue };
                        let nn = npts.len();

                        let mut has_same_winding = false;
                        for nk in 0..nn {
                            let (na, nb) = (npts[nk] as u32, npts[(nk + 1) % nn] as u32);
                            if (a == na && b == nb) || (a == nb && b == na) {
                                // Found shared edge: if (a,b) in same order, windings are SAME (bad)
                                // We want OPPOSITE winding, so flip if same
                                if a == na && b == nb {
                                    has_same_winding = true;
                                }
                                break;
                            }
                        }

                        oriented[ni] = true;
                        // If this poly has same winding as current, we need to flip it
                        if has_same_winding {
                            flipped[ni] = !flipped[pi];
                        } else {
                            flipped[ni] = flipped[pi];
                        }
                        queue.push_back(ni);
                    }
                }
            }
        }
    }

    // Apply flips
    for (pi, pr) in d.prims.iter_mut().enumerate() {
        if flipped[pi] {
            let (lo, hi) = (pr.verts.start as usize, pr.verts.end as usize);
            if let Some(verts) = d.vertex_point.get_mut(lo..hi) {
                verts.reverse();
            }
        }
    }
}

impl Node<Detail> for Facet {
    fn type_name(&self) -> &'static str {
        "facet"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let (unique, cusp, post) = (toggle(p, "unique_points")?, toggle(p, "cusp")?, toggle(p, "post_nml")?);
        let orient = toggle(p, "orient_polygons")?;
        let consol_dist = float(p, "consolidate")?;
        let consolidate = consol_dist > 0.0;
        let angle = float(p, "cusp_angle")?.clamp(0.0, 180.0);
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if !(unique || cusp || post || orient || consolidate) {
            return Ok(Arc::clone(src));
        }
        let np = src.point_count as usize;
        let vp: Vec<usize> = src.vertex_point.iter().map(|&p| p as usize).collect();
        let pos = positions(src);
        // new point id per vertex + the source point of every new point
        let (mut new_vp, mut origin): (Vec<usize>, Vec<usize>) = (vp.clone(), (0..np).collect());
        if unique {
            if (vp.len() as u64).saturating_add(np as u64) > MAX_ELEMS {
                return Err(failed(OP, "output element count exceeds the allocation cap"));
            }
            // Houdini 20.5 (hython-observed on box + 3x3 grid): the FIRST vertex of a point keeps its id, every further
            // vertex gets a new point appended; new ids go out per original point in DESCENDING point order, vertices
            // of one point ascending. Unused points keep their ids.
            let mut by_point: Vec<Vec<usize>> = vec![Vec::new(); np];
            for (v, &pt) in vp.iter().enumerate() {
                if let Some(list) = by_point.get_mut(pt) {
                    list.push(v);
                }
            }
            for (pt, verts) in by_point.iter().enumerate().rev() {
                for &v in verts.iter().skip(1) {
                    if let Some(slot) = new_vp.get_mut(v) {
                        *slot = origin.len();
                    }
                    origin.push(pt);
                }
            }
        } else if cusp {
            let cos = angle.to_radians().cos();
            let vn = vertex_normals(src, &pos);
            let mut by_point: Vec<Vec<usize>> = vec![Vec::new(); np];
            for (v, &pt) in vp.iter().enumerate() {
                if let Some(list) = by_point.get_mut(pt) {
                    list.push(v);
                }
            }
            for (pt, verts) in by_point.iter().enumerate() {
                let mut leaders: Vec<Option<V3>> = Vec::new();
                let mut ids: Vec<usize> = Vec::new();
                for &v in verts {
                    let n = vn.get(v).copied().flatten();
                    let hit = leaders.iter().position(|l| match (l, n) {
                        (Some(l), Some(n)) => dot(*l, n) >= cos,
                        _ => true,
                    });
                    let c = match hit {
                        Some(c) => c,
                        None if leaders.len() >= MAX_CLUSTERS => leaders.len() - 1,
                        None => {
                            leaders.push(n);
                            if leaders.len() > 1 {
                                if origin.len() as u64 >= MAX_ELEMS {
                                    return Err(failed(OP, "output element count exceeds the allocation cap"));
                                }
                                origin.push(pt);
                                ids.push(origin.len() - 1);
                            } else {
                                ids.push(pt);
                            }
                            leaders.len() - 1
                        }
                    };
                    // a leader-less cluster adopts the first real normal that joins it
                    if let (Some(l), Some(_)) = (leaders.get_mut(c), n)
                        && l.is_none()
                    {
                        *l = n;
                    }
                    if let (Some(nv), Some(&id)) = (new_vp.get_mut(v), ids.get(c)) {
                        *nv = id;
                    }
                }
            }
        }
        let unchanged = origin.len() == np && new_vp == vp;
        let mut out = if unchanged {
            (**src).clone()
        } else {
            let ids: Vec<u32> = new_vp
                .iter()
                .map(|&i| u32::try_from(i).map_err(|_| failed(OP, "point id overflow")))
                .collect::<Result<_, _>>()?;
            let rm = Remap {
                point_src: origin.iter().map(|&i| vec![i]).collect(),
                vert_src: (0..ids.len()).map(Some).collect(),
                prim_src: (0..src.prims.len()).map(Some).collect(),
            };
            let p: Vec<V3> = origin.iter().map(|&i| pos.get(i).copied().unwrap_or([0.0; 3])).collect();
            rebuild(OP, src, ids, src.prims.clone(), &rm, to_f32(OP, &p)?)?
        };
        if orient {
            orient_polygons(&mut out);
        }
        if consolidate {
            // Fuse nearby points within consol_dist
            let out_pos = positions(&out);
            let mut remap = vec![u32::MAX; out_pos.len()];
            for i in 0..out_pos.len() {
                if remap[i] != u32::MAX {
                    continue;
                }
                remap[i] = i as u32;
                for j in (i + 1)..out_pos.len() {
                    if remap[j] == u32::MAX {
                        let d2 = (0..3).map(|k| (out_pos[i][k] - out_pos[j][k]).powi(2)).sum::<f64>().sqrt();
                        if d2 <= consol_dist {
                            remap[j] = i as u32;
                        }
                    }
                }
            }
            // Remap vertex_point
            for vp in &mut out.vertex_point {
                if let Some(&new_id) = remap.get(*vp as usize)
                    && new_id != u32::MAX
                {
                    *vp = new_id;
                }
            }
        }
        if post {
            let n = point_normals(&out, &positions(&out));
            let n: Vec<[f32; 3]> = n.iter().map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect();
            out.add_attr(Attribute {
                name: "N".into(),
                class: AttrClass::Point,
                data: AttrData::V3(n),
                type_info: TypeInfo::Normal,
            });
        } else if unchanged && !orient && !consolidate {
            return Ok(Arc::clone(src));
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, grid3};
    use super::*;
    use hexe_geo::Group;
    use hexe_graph::ParamSet;

    fn opts(unique: bool, cusp: bool, angle: f32, post: bool) -> ParamSet {
        defaults(&*new())
            .with("unique_points", ParamValue::B(unique))
            .with("cusp", ParamValue::B(cusp))
            .with("cusp_angle", ParamValue::F(angle))
            .with("post_nml", ParamValue::B(post))
    }
    fn opts_full(unique: bool, cusp: bool, angle: f32, post: bool, orient: bool, consol: f32) -> ParamSet {
        defaults(&*new())
            .with("unique_points", ParamValue::B(unique))
            .with("cusp", ParamValue::B(cusp))
            .with("cusp_angle", ParamValue::F(angle))
            .with("post_nml", ParamValue::B(post))
            .with("orient_polygons", ParamValue::B(orient))
            .with("consolidate", ParamValue::F(consol))
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "facet");
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("unique_points", ParamValue::B(false)),
                ("cusp", ParamValue::B(false)),
                ("cusp_angle", ParamValue::F(60.0)),
                ("post_nml", ParamValue::B(false)),
                ("consolidate", ParamValue::F(0.0)),
                ("orient_polygons", ParamValue::B(false)),
            ]
        );
    }
    #[test]
    fn box_unique_points_is_24_points_with_same_positions_per_corner() {
        let src = cube();
        let out = cook(&*new(), src.clone(), &opts(true, false, 60.0, false)).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (24, 6));
        // first vertex of a point keeps the id (8 distinct), the other 16 are new; every vertex still sees its corner
        let mut seen = out.vertex_point.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..24).collect::<Vec<u32>>(), "24 distinct points");
        for (v, &pt) in src.vertex_point.iter().enumerate() {
            assert_eq!(out.p()[out.vertex_point[v] as usize], src.p()[pt as usize]);
        }
        // hython-exact: first user keeps its id, new ids descend with the source point, ascend with the vertex
        assert_eq!(&out.vertex_point[..4], &[0, 2, 3, 1].map(|x| x as u32), "prim 0 keeps its 4 ids");
    }
    /// hython 20.5, grid 3x3 + facet unique: prims [[0,1,4,3],[15,2,5,11],[14,12,7,6],[13,10,8,9]], 16 points.
    #[test]
    fn grid_unique_points_matches_houdinis_id_order() {
        let mut src = Detail::default();
        src.add_points(9);
        for q in [[0u32, 1, 4, 3], [1, 2, 5, 4], [3, 4, 7, 6], [4, 5, 8, 7]] {
            src.add_poly(&q, true);
        }
        let ids = |d: &Detail, q: usize| -> Vec<u32> {
            d.vertex_point[d.prims[q].verts.start as usize..d.prims[q].verts.end as usize].to_vec()
        };
        assert_eq!(ids(&src, 1), vec![1, 2, 5, 4], "same input layout as Houdini's grid");
        let out = cook(&*new(), src, &opts(true, false, 60.0, false)).expect("cooks");
        let got: Vec<Vec<u32>> = (0..4).map(|q| ids(&out, q)).collect();
        assert_eq!(got, vec![vec![0, 1, 4, 3], vec![15, 2, 5, 11], vec![14, 12, 7, 6], vec![13, 10, 8, 9]]);
        assert_eq!(out.point_count, 16);
    }
    #[test]
    fn box_cusp_60_is_24_points_and_cusp_100_keeps_8() {
        let out = cook(&*new(), cube(), &opts(false, true, 60.0, false)).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (24, 6));
        assert_eq!(out.p()[..8], cube().p()[..8], "cluster 0 keeps the original point ids");
        let loose = cook(&*new(), cube(), &opts(false, true, 100.0, false)).expect("cooks");
        assert_eq!(loose.point_count, 8, "faces 90 deg apart are within 100 deg");
    }
    #[test]
    fn flat_grid_cusp_60_is_unchanged_and_shares_the_arc() {
        let a = Arc::new(grid3());
        let o = cook_arc(&*new(), &a, &opts(false, true, 60.0, false)).expect("cooks");
        assert_eq!(o.point_count, 9);
        assert!(Arc::ptr_eq(&o, &a));
    }
    #[test]
    fn no_options_share_the_arc() {
        let a = Arc::new(cube());
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &opts(false, false, 60.0, false)).expect("cooks"), &a));
    }
    #[test]
    fn post_nml_unique_gives_face_normals_and_shared_gives_diagonals() {
        let u = cook(&*new(), cube(), &opts(true, false, 60.0, true)).expect("cooks");
        let Some(AttrData::V3(n)) = u.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N missing") };
        assert_eq!(n.len(), 24);
        // face [0,1,3,2] is the x = -0.5 face: outward normal -X for all 4 corners
        assert!(n[..4].iter().all(|v| *v == [-1.0, 0.0, 0.0]), "{:?}", &n[..4]);
        assert_eq!(u.attr(AttrClass::Point, "N").map(|a| a.type_info), Some(TypeInfo::Normal));
        let s = cook(&*new(), cube(), &opts(false, false, 60.0, true)).expect("cooks");
        let Some(AttrData::V3(n)) = s.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N missing") };
        let d = (1.0f64 / 3.0f64.sqrt()) as f32;
        for (c, v) in s.p().iter().zip(n) {
            assert!((0..3).all(|j| (v[j] - d * c[j].signum()).abs() < 1e-6), "{c:?} {v:?}");
        }
        assert_eq!(s.point_count, 8);
    }
    #[test]
    fn point_attrs_groups_orphans_and_hostile_inputs() {
        let mut d = cube();
        d.add_points(1); // orphan
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: AttrData::F32((0..9).map(|i| i as f32).collect()),
            type_info: TypeInfo::None,
        });
        d.groups.push(Group { name: "g".into(), class: AttrClass::Point, mask: vec![true; 9] });
        let out = cook(&*new(), d.clone(), &opts(true, false, 60.0, false)).expect("cooks");
        assert_eq!(out.point_count, 25);
        let Some(AttrData::F32(w)) = out.attr(AttrClass::Point, "w").map(|a| &a.data) else { panic!("w lost") };
        assert_eq!(w[8], 8.0, "orphan keeps its id");
        assert_eq!(w[0], d.vertex_point[0] as f32);
        assert_eq!(out.group(AttrClass::Point, "g").map(|g| g.mask.iter().all(|m| *m)), Some(true));
        // hostile: bad angle, degenerate/open prims, empty
        for a in [f32::NAN, f32::INFINITY] {
            assert!(matches!(cook(&*new(), cube(), &opts(false, true, a, false)), Err(CookError::BadParam(_))));
        }
        let mut h = cube();
        h.add_poly(&[0, 0, 0], true);
        h.add_poly(&[0, 1], false);
        assert_eq!(cook(&*new(), h, &opts(false, true, 720.0, true)).expect("cooks").point_count, 8);
        assert_eq!(cook(&*new(), Detail::default(), &opts(true, true, 60.0, true)).expect("cooks").point_count, 0);
    }
    #[test]
    fn scene_loader_cooks_unique_points_and_registry_slot() {
        let ok = r#"{"nodes":[{"name":"b","type":"box"},{"name":"f","type":"facet","params":{"unique_points":true}}],"edges":[["b",0,"f",0]],"display":"f"}"#;
        let (mut g, id) = crate::load_scene(ok).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 24);
        assert_eq!(crate::registry().create("facet").map(|n| n.type_name()), Some("facet"));
    }
    #[test]
    fn orient_polygons_makes_adjacent_polys_opposite_winding() {
        // Two quads sharing an edge: one with correct winding, one with SAME winding (bad)
        let mut d = Detail::default();
        d.add_points(6);
        *d.p_mut() =
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2, 3], true); // quad 1: counter-clockwise, edge 1->2
        d.add_poly(&[2, 1, 4, 5], true); // quad 2: edge 2->1 (opposite), edge 1->4, should NOT flip
        d.add_poly(&[1, 2, 5, 4], true); // quad 3: edge 1->2 (same as quad 1), SHOULD flip to [4,5,2,1]

        let out = cook(&*new(), d, &opts_full(false, false, 60.0, false, true, 0.0)).expect("cooks");
        assert_eq!(out.prim_count(), 3);

        let p0 = &out.vertex_point[0..4];
        let p1 = &out.vertex_point[4..8];
        let p2 = &out.vertex_point[8..12];

        // First poly should be unchanged: [0,1,2,3]
        assert_eq!(p0, &[0, 1, 2, 3]);

        // Second poly already has opposite winding, should be unchanged: [2,1,4,5]
        assert_eq!(p1, &[2, 1, 4, 5]);

        // Third poly has same winding as first on shared edge, should be flipped: [4,5,2,1]
        assert_eq!(p2, &[4, 5, 2, 1]);
    }
    #[test]
    fn consolidate_fuses_nearby_points() {
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![
            [0.0, 0.0, 0.0],
            [0.0005, 0.0, 0.0], // within 0.001 of point 0
            [1.0, 0.0, 0.0],
            [1.0, 0.0005, 0.0], // within 0.001 of point 2
        ];
        d.add_poly(&[0, 2, 3, 1], true);

        let out = cook(&*new(), d, &opts_full(false, false, 60.0, false, false, 0.001)).expect("cooks");

        // Points should be fused: 0+1 -> 0, 2+3 -> 2
        let vp = &out.vertex_point;
        assert_eq!(vp.len(), 4);
        // Check that nearby points were remapped
        assert_eq!(vp[0], vp[3]); // points 0 and 1 should map to same id
        assert_eq!(vp[1], vp[2]); // points 2 and 3 should map to same id
        assert_ne!(vp[0], vp[1]); // but the two groups should differ
    }
}
