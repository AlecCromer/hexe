//! Subdivide SOP (token `subdivide`): Catmull-Clark and linear subdivision of closed polygons.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! One iteration splits every selected closed poly (>= 3 corners, valid ids) with `n` corners into `n` quads
//! `(v_k, e(v_k,v_k+1), f, e(v_k-1,v_k))`; winding is preserved. New points, in order: one FACE point per selected
//! face (prim order), one EDGE point per distinct undirected edge (first-encounter order), after all input points.
//! Box (8 pts / 6 quads): 8+6+12 = 26 pts / 24 quads; iteration 2: 98 / 96.
//! * face point `F` = mean of the face's corners (both algorithms);
//! * linear: edge point = edge midpoint, input points unchanged (split only);
//! * Catmull-Clark edge point: interior edge (exactly 2 selected faces) `(A + B + F1 + F2) / 4`; boundary or
//!   non-manifold edge (1 or >= 3 faces) midpoint `(A + B) / 2`;
//! * Catmull-Clark vertex point, interior vertex (every incident edge has exactly 2 faces, valence `n >= 3`, faces
//!   incident = valence): `(F + 2R + (n - 3)P) / n`, `F` = mean of the incident face points, `R` = mean of the
//!   midpoints of the `n` incident ORIGINAL edges, `P` = old position;
//! * boundary vertex (exactly two boundary edges to `A`, `B`): `(6P + A + B) / 8`; any other vertex (corner of a
//!   non-manifold fan, bowtie, valence < 3) keeps `P`.
//!
//! Topology among UNselected prims is not smoothed against: only selected faces form the subdivided mesh; points
//! shared with passthrough prims still move (no crack protection). Attributes: input points keep their values, new
//! points average the float point attrs of their source points (int / string: first source); corner vertices keep
//! their vertex attrs, new vertices default-fill; quads copy the prim attrs of their face; groups follow the same maps
//! (a new point is grouped only when all its sources are).
//!
//! Params: `iterations` I 1 (0 = input `Arc` shared; negative or > 6 = `BadParam`), `algorithm` menu
//! (0 catmull-clark, 1 linear; >= 2 OpenSubdiv variants = `Unsupported`), `group` S (prim groups, union; empty = all;
//! unknown name = `Failed`). Element counts above `MAX_ELEMS` -> `Failed` BEFORE allocating each iteration.
use super::meshutil::{
    MAX_ELEMS, Remap, V3, add, bad, closed_poly_pts, failed, int, positions, rebuild, scale, selection, to_f32,
};
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::HashMap;
use std::sync::Arc;

const OP: &str = "subdivide";
/// `iterations` cap (4^6 growth is already the allocation cap for most inputs).
const MAX_ITERATIONS: i32 = 6;
const ALGO_CATMULL: u32 = 0;
const ALGO_LINEAR: u32 = 1;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("iterations", ParamValue::I(1)).label("Subdivision Levels").range(0.0, 6.0),
        ParamSpec::new("algorithm", ParamValue::Menu(ALGO_CATMULL)).label("Algorithm"),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Subdivide { specs: spec() })
}
struct Subdivide {
    specs: Vec<ParamSpec>,
}

fn mean(pos: &[V3], ids: &[usize]) -> V3 {
    let s = ids.iter().filter_map(|&i| pos.get(i)).fold([0.0; 3], |a, &p| add(a, p));
    scale(s, 1.0 / ids.len().max(1) as f64)
}

/// One subdivision pass over the selected faces; `None` when nothing is selectable.
fn step(src: &Detail, sel: Option<&[bool]>, catmull: bool) -> Result<Option<Detail>, CookError> {
    let pos = positions(src);
    let np = pos.len();
    // ---- selected faces: (prim index, first vertex, corner points) ----
    let mut faces: Vec<(usize, usize, Vec<usize>)> = Vec::new();
    for (pi, pr) in src.prims.iter().enumerate() {
        if sel.is_some_and(|s| !s.get(pi).copied().unwrap_or(false)) {
            continue;
        }
        if let (Some(pts), Ok(lo)) = (closed_poly_pts(src, pr), usize::try_from(pr.verts.start)) {
            faces.push((pi, lo, pts));
        }
    }
    if faces.is_empty() {
        return Ok(None);
    }
    // ---- size cap (u64, before any allocation) ----
    let corners: u64 = faces.iter().map(|f| f.2.len() as u64).sum();
    let nf = faces.len() as u64;
    let kept_v = u64::from(src.vertex_count()).saturating_sub(faces.iter().map(|f| f.2.len() as u64).sum::<u64>());
    let out_pts = (np as u64).saturating_add(nf).saturating_add(corners);
    let out_verts = kept_v.saturating_add(corners.saturating_mul(4));
    let out_prims = (src.prims.len() as u64).saturating_sub(nf).saturating_add(corners);
    if out_pts > MAX_ELEMS || out_verts > MAX_ELEMS || out_prims > MAX_ELEMS {
        return Err(failed(OP, "output element count exceeds the allocation cap"));
    }
    // ---- face points, edge table ----
    let fpos: Vec<V3> = faces.iter().map(|f| mean(&pos, &f.2)).collect();
    let mut edge_id: HashMap<(usize, usize), usize> = HashMap::new();
    let mut ends: Vec<(usize, usize)> = Vec::new();
    let mut ecount: Vec<u32> = Vec::new();
    let mut efsum: Vec<V3> = Vec::new();
    let mut face_edges: Vec<Vec<usize>> = Vec::with_capacity(faces.len());
    for (fi, (_, _, pts)) in faces.iter().enumerate() {
        let mut fe = Vec::with_capacity(pts.len());
        for (k, &a) in pts.iter().enumerate() {
            let b = pts.get((k + 1) % pts.len()).copied().unwrap_or(a);
            let key = (a.min(b), a.max(b));
            let id = *edge_id.entry(key).or_insert_with(|| {
                ends.push(key);
                ecount.push(0);
                efsum.push([0.0; 3]);
                ends.len() - 1
            });
            if let (Some(c), Some(s), Some(f)) = (ecount.get_mut(id), efsum.get_mut(id), fpos.get(fi)) {
                *c += 1;
                *s = add(*s, *f);
            }
            fe.push(id);
        }
        face_edges.push(fe);
    }
    let at = |i: usize| pos.get(i).copied().unwrap_or([0.0; 3]);
    // ---- edge points ----
    let epos: Vec<V3> = ends
        .iter()
        .enumerate()
        .map(|(e, &(a, b))| {
            let mid = scale(add(at(a), at(b)), 0.5);
            match (catmull, ecount.get(e), efsum.get(e)) {
                (true, Some(2), Some(&fs)) => scale(add(add(at(a), at(b)), fs), 0.25),
                _ => mid,
            }
        })
        .collect();
    // ---- vertex points (Catmull-Clark only) ----
    let mut new_p = pos.clone();
    if catmull {
        let (mut fcnt, mut fsum) = (vec![0u32; np], vec![[0.0f64; 3]; np]);
        for ((_, _, pts), fp) in faces.iter().zip(&fpos) {
            for &p in pts {
                if let (Some(c), Some(s)) = (fcnt.get_mut(p), fsum.get_mut(p)) {
                    *c += 1;
                    *s = add(*s, *fp);
                }
            }
        }
        let (mut ecnt, mut msum) = (vec![0u32; np], vec![[0.0f64; 3]; np]);
        let (mut bcnt, mut bsum) = (vec![0u32; np], vec![[0.0f64; 3]; np]);
        for (e, &(a, b)) in ends.iter().enumerate() {
            let boundary = ecount.get(e).copied() != Some(2);
            let mid = scale(add(at(a), at(b)), 0.5);
            for (v, o) in [(a, b), (b, a)] {
                if let (Some(c), Some(s)) = (ecnt.get_mut(v), msum.get_mut(v)) {
                    *c += 1;
                    *s = add(*s, mid);
                }
                if let (true, Some(c), Some(s)) = (boundary, bcnt.get_mut(v), bsum.get_mut(v)) {
                    *c += 1;
                    *s = add(*s, at(o));
                }
            }
        }
        for i in 0..np {
            let (fc, ec, bc) = (fcnt[i], ecnt[i], bcnt[i]);
            if fc == 0 {
                continue;
            }
            let p = at(i);
            if bc == 0 {
                // interior: valence = incident edges = incident faces
                if ec == fc && ec >= 3 {
                    let n = f64::from(ec);
                    let f = scale(fsum[i], 1.0 / f64::from(fc));
                    let r = scale(msum[i], 1.0 / n);
                    new_p[i] = scale(add(add(f, scale(r, 2.0)), scale(p, n - 3.0)), 1.0 / n);
                }
            } else if bc == 2 {
                new_p[i] = scale(add(scale(p, 6.0), bsum[i]), 0.125);
            }
        }
    }
    new_p.extend(fpos.iter().copied());
    new_p.extend(epos.iter().copied());
    // ---- topology ----
    let (fbase, ebase) = (np, np + faces.len());
    let id = |i: usize| u32::try_from(i).map_err(|_| failed(OP, "point id overflow"));
    let mut rm = Remap::default();
    rm.point_src.extend((0..np).map(|i| vec![i]));
    rm.point_src.extend(faces.iter().map(|f| f.2.clone()));
    rm.point_src.extend(ends.iter().map(|&(a, b)| vec![a, b]));
    let mut vertex_point: Vec<u32> = Vec::new();
    let mut prims: Vec<Prim> = Vec::new();
    let mut next_face = faces.iter().enumerate().peekable();
    for (pi, pr) in src.prims.iter().enumerate() {
        let start = u32::try_from(vertex_point.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
        if let Some((fi, (_, lo, pts))) = next_face.next_if(|(_, f)| f.0 == pi) {
            let n = pts.len();
            let fe = face_edges.get(fi).map(Vec::as_slice).unwrap_or(&[]);
            for k in 0..n {
                let (nx, pv) = (fe.get(k).copied().unwrap_or(0), fe.get((k + n - 1) % n).copied().unwrap_or(0));
                let s = u32::try_from(vertex_point.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
                vertex_point.extend([
                    id(pts.get(k).copied().unwrap_or(0))?,
                    id(ebase + nx)?,
                    id(fbase + fi)?,
                    id(ebase + pv)?,
                ]);
                rm.vert_src.extend([Some(lo + k), None, None, None]);
                prims.push(Prim { kind: PrimKind::Poly { closed: true }, verts: s..s + 4 });
                rm.prim_src.push(Some(pi));
            }
        } else {
            let (lo, hi) = (usize::try_from(pr.verts.start).unwrap_or(0), usize::try_from(pr.verts.end).unwrap_or(0));
            vertex_point.extend(src.vertex_point.get(lo..hi).unwrap_or(&[]));
            rm.vert_src.extend((lo..hi).map(Some));
            let end = u32::try_from(vertex_point.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
            prims.push(Prim { kind: pr.kind.clone(), verts: start..end });
            rm.prim_src.push(Some(pi));
        }
    }
    let p = to_f32(OP, &new_p)?;
    rebuild(OP, src, vertex_point, prims, &rm, p).map(Some)
}

impl Node<Detail> for Subdivide {
    fn type_name(&self) -> &'static str {
        "subdivide"
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
        let catmull = match p.menu("algorithm") {
            Some(ALGO_CATMULL) => true,
            Some(ALGO_LINEAR) => false,
            Some(m) => {
                return Err(CookError::Unsupported(format!("subdivide algorithm menu {m} (OpenSubdiv variants)")));
            }
            None => return Err(bad("`algorithm`: missing or not a menu")),
        };
        let names = p.string("group").unwrap_or("");
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let mut cur = Arc::clone(src);
        for _ in 0..iters {
            let sel = selection(OP, &cur, names)?;
            match step(&cur, sel.as_deref(), catmull)? {
                Some(d) => cur = Arc::new(d),
                None => break,
            }
        }
        Ok(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, grid3, prim_pts};
    use super::*;
    use hexe_geo::{AttrClass, AttrData, Attribute, Group, TypeInfo};
    use hexe_graph::ParamSet;

    fn with(iters: i32, algo: u32) -> ParamSet {
        defaults(&*new()).with("iterations", ParamValue::I(iters)).with("algorithm", ParamValue::Menu(algo))
    }
    fn all_closed_quads(d: &Detail) -> bool {
        d.prims.iter().all(|p| matches!(p.kind, PrimKind::Poly { closed: true }) && p.verts.len() == 4)
    }
    fn near(a: f32, b: f64) -> bool {
        (f64::from(a) - b).abs() < 1e-7
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "subdivide");
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("iterations", ParamValue::I(1)),
                ("algorithm", ParamValue::Menu(0)),
                ("group", ParamValue::S(String::new()))
            ]
        );
    }
    #[test]
    fn box_catmull_clark_iter1_is_26_points_24_closed_quads() {
        let out = cook(&*new(), cube(), &with(1, 0)).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (26, 24));
        assert!(all_closed_quads(&out));
    }
    #[test]
    fn box_catmull_clark_iter2_is_98_points_96_closed_quads() {
        let out = cook(&*new(), cube(), &with(2, 0)).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (98, 96));
        assert!(all_closed_quads(&out));
    }
    /// Corner of the +-0.5 box, rule `(F + 2R + (n-3)P) / n` with n = 3 (the P term vanishes):
    /// corner P = (h,h,h), h = 1/2. The 3 incident faces have centres (h,0,0),(0,h,0),(0,0,h)
    ///   -> F = ((h,0,0)+(0,h,0)+(0,0,h)) / 3 = (h/3, h/3, h/3) = (1/6,1/6,1/6).
    /// The 3 incident edges go to (-h,h,h),(h,-h,h),(h,h,-h) -> midpoints (0,h,h),(h,0,h),(h,h,0)
    ///   -> R = (2h/3, 2h/3, 2h/3) = (1/3,1/3,1/3).
    /// New corner = (F + 2R) / 3 = (1/6 + 2/3) / 3 = (5/6) / 3 = 5/18 per axis (= original corner x 5/9, NOT 5/12).
    #[test]
    fn box_catmull_clark_corner_is_five_eighteenths_exact() {
        let out = cook(&*new(), cube(), &with(1, 0)).expect("cooks");
        let exact = 5.0 / 18.0;
        for (i, c) in out.p()[..8].iter().enumerate() {
            let sgn = |bit: u32| if (i as u32 >> bit) & 1 == 1 { 1.0 } else { -1.0 };
            let want = [exact * sgn(2), exact * sgn(1), exact * sgn(0)];
            assert!((0..3).all(|j| near(c[j], want[j])), "corner {i}: {c:?} vs {want:?}");
        }
        // edge point 14 = edge (0,2) (first edge of Houdini-wound face [0,2,3,1]) = (-h,-h,-h)-(-h,h,-h), faces x=-h (centre
        // (-h,0,0)) and z=-h (centre (0,0,-h)): (A + B + F1 + F2) / 4 = ((-1,0,-1) + (-1/2,0,0) + (0,0,-1/2)) / 4 = (-3/8, 0, -3/8)
        assert_eq!(out.p()[14], [-0.375, 0.0, -0.375]);
        assert_eq!(out.p()[8], [-0.5, 0.0, 0.0]);
    }
    #[test]
    fn box_linear_iter1_keeps_corners_exactly_and_splits_26_24() {
        let src = cube();
        let out = cook(&*new(), src.clone(), &with(1, 1)).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (26, 24));
        assert_eq!(out.p()[..8], src.p()[..8]);
        assert!(all_closed_quads(&out));
        // linear edge points are midpoints, face points centroids: first face point = centre of face [0,1,3,2] = (-h,0,0)
        assert_eq!(out.p()[8], [-0.5, 0.0, 0.0]);
    }
    #[test]
    fn single_triangle_catmull_clark_is_7_points_3_quads_boundary_rules() {
        let mut d = Detail::default();
        d.add_points(3);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [0.0, 6.0, 0.0]];
        d.add_poly(&[0, 1, 2], true);
        let out = cook(&*new(), d, &with(1, 0)).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (7, 3));
        assert!(all_closed_quads(&out));
        // boundary vertex (6P + A + B) / 8: corner 0 -> (6*0 + (6,0,0) + (0,6,0)) / 8 = (0.75, 0.75, 0)
        assert_eq!(out.p()[0], [0.75, 0.75, 0.0]);
        // face point = centroid (2,2,0); edge points = midpoints (boundary)
        assert_eq!(out.p()[3], [2.0, 2.0, 0.0]);
        assert_eq!(out.p()[4], [3.0, 0.0, 0.0]);
    }
    #[test]
    fn open_grid_3x3_is_25_points_16_quads_boundary_corners_on_boundary() {
        let out = cook(&*new(), grid3(), &with(1, 0)).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (25, 16));
        assert!(all_closed_quads(&out));
        // boundary vertex (6P+A+B)/8 along a straight boundary line stays on it; flat grid stays z = 0
        assert!(out.p().iter().all(|p| p[2] == 0.0));
        let on_edge = |p: [f32; 3]| p[0] == 0.0 || p[1] == 0.0 || p[0] == 2.0 || p[1] == 2.0;
        // corner (0,0): (6*(0,0)+(1,0)+(0,1))/8 = (1/8,1/8): stays inside the grid (corners round in), side mid (1,0) stays y=0
        assert_eq!(out.p()[0], [0.125, 0.125, 0.0]);
        assert_eq!(out.p()[1], [1.0, 0.0, 0.0]);
        assert!(on_edge(out.p()[1]) && on_edge(out.p()[3]));
        // all boundary non-corner originals (1,3,5,7) remain exactly on their boundary line
        for (i, line) in [(1usize, 1usize), (3, 0), (5, 0), (7, 1)] {
            assert!(out.p()[i][line] == 0.0 || out.p()[i][line] == 2.0, "point {i}: {:?}", out.p()[i]);
        }
        // interior vertex of the 4-valent centre stays at (1,1)
        assert_eq!(out.p()[4], [1.0, 1.0, 0.0]);
    }
    #[test]
    fn zero_iterations_and_no_faces_share_the_arc() {
        let a = Arc::new(cube());
        let o = cook_arc(&*new(), &a, &with(0, 0)).expect("cooks");
        assert!(Arc::ptr_eq(&o, &a));
        let mut lines = Detail::default();
        lines.add_points(2);
        lines.add_poly(&[0, 1], false);
        let l = Arc::new(lines);
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &l, &with(2, 0)).expect("cooks"), &l));
        assert_eq!(cook(&*new(), Detail::default(), &with(1, 0)).expect("cooks").point_count, 0);
    }
    #[test]
    fn hostile_params_and_caps() {
        for it in [-1, 7, i32::MAX] {
            assert!(matches!(cook(&*new(), cube(), &with(it, 0)), Err(CookError::BadParam(_))), "{it}");
        }
        for m in [2, 3, 99] {
            assert!(matches!(cook(&*new(), cube(), &with(1, m)), Err(CookError::Unsupported(_))), "{m}");
        }
        // a dense grid whose first iteration exceeds MAX_ELEMS must fail up front, not OOM
        let mut g = Detail::default();
        let n = 1100u32;
        g.add_points(n * n);
        *g.p_mut() = (0..n * n).map(|i| [(i % n) as f32, (i / n) as f32, 0.0]).collect();
        for r in 0..n - 1 {
            for c in 0..n - 1 {
                let a = r * n + c;
                g.add_poly(&[a, a + 1, a + n + 1, a + n], true);
            }
        }
        assert!(matches!(cook(&*new(), g, &with(6, 0)), Err(CookError::Failed(_))));
        // degenerate / open / bad prims pass through; nothing panics
        let mut d = cube();
        d.add_poly(&[0, 0, 0, 0], true);
        d.add_poly(&[0, 1, 2], false);
        let out = cook(&*new(), d, &with(2, 0)).expect("cooks");
        assert!(out.prim_count() > 96);
    }
    #[test]
    fn group_limits_and_unknown_group_fails() {
        let mut d = cube();
        d.groups.push(Group {
            name: "g".into(),
            class: AttrClass::Prim,
            mask: vec![true, false, false, false, false, false],
        });
        let p = with(1, 1).with("group", ParamValue::S("g".into()));
        let out = cook(&*new(), d, &p).expect("cooks");
        assert_eq!(out.prim_count(), 4 + 5);
        assert_eq!(out.group(AttrClass::Prim, "g").map(|g| g.mask.iter().filter(|m| **m).count()), Some(4));
        let bad = with(1, 0).with("group", ParamValue::S("nope".into()));
        assert!(matches!(cook(&*new(), cube(), &bad), Err(CookError::Failed(_))));
    }
    #[test]
    fn attrs_blend_on_new_points_and_prim_attrs_follow() {
        let mut d = cube();
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: AttrData::F32((0..8).map(|i| i as f32).collect()),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "id".into(),
            class: AttrClass::Prim,
            data: AttrData::I32((0..6).collect()),
            type_info: TypeInfo::None,
        });
        let out = cook(&*new(), d, &with(1, 1)).expect("cooks");
        let Some(AttrData::F32(w)) = out.attr(AttrClass::Point, "w").map(|a| &a.data) else { panic!("w lost") };
        assert_eq!(w.len(), 26);
        assert_eq!(w[..8], [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(w[8], 1.5); // face [0,2,3,1] mean
        assert_eq!(w[14], 1.0); // edge (0,2) midpoint of the weights... first edge of face 0
        let Some(AttrData::I32(id)) = out.attr(AttrClass::Prim, "id").map(|a| &a.data) else { panic!("id lost") };
        assert_eq!(id[..4], [0, 0, 0, 0]);
        assert_eq!(id.len(), 24);
        assert_eq!(prim_pts(&out, 0).len(), 4);
    }
    #[test]
    fn scene_loader_cooks_example_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/subdivide.json"))
            .expect("example present");
        let (mut g, id) = crate::load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert_eq!((out.point_count, out.prim_count()), (26, 24));
        assert_eq!(crate::registry().create("subdivide").map(|n| n.type_name()), Some("subdivide"));
    }
}
