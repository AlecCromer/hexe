//! Dissolve SOP (token `dissolve`): remove edges, merging the polygons on both sides (Houdini 20.5 `dissolve::2.0`, edge-group path).
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-dissolve*`)
//! `group` = space separated EDGES `pA-B` (point pair, either order). EMPTY group = every edge: all polygons go (empty output).
//! * An edge shared by two closed polygons merges them; prims joined through chains of dissolved edges form a REGION. Its boundary is walked
//!   from the first non-dissolved edge of the region's lowest prim: after each edge take the next edge of the same face, and while that edge is
//!   dissolved hop to its twin in the neighbour face and continue after it. Output vertices = start point of every walked edge (a point can
//!   repeat, e.g. 3 box faces merge to `[1,5,4,0,3,2,6,5]`). The merged polygon takes the attributes / groups of the region's FIRST prim and its vertex
//!   attributes follow the walked source vertices.
//! * A dissolved edge owned by ONE polygon deletes the whole region it belongs to (boundary edge on a grid deletes that quad).
//! * Order: every untouched prim in input order (incl. open polys / other kinds), then the merged regions ordered by their first prim.
//! * `reminlinepts` (default on): in a merged polygon a vertex is dropped when its point is used by no other polygon slot AND the turn angle
//!   there is `< coltol` degrees (default 45; 0 removes nothing). `remunusedpts` (default on): points no prim uses are deleted (this also drops
//!   the inline points removed above; with it off they stay as unused points). Point attrs / groups compact with the points.
//!
//! Not modelled (-> `Unsupported`): numeric / prim / point group patterns (Houdini turns them into edges in ways not probed), edge groups on
//! non-manifold edges, `invertsel`, `bridge` / `boundarycurves` / `deldegeneratebridges`, regions with holes (more than one boundary loop),
//! `compnorms` (no effect). Polyline / open poly edges are never dissolved.
use super::meshutil::{MAX_ELEMS, V3, bad, dot, failed, float, positions, rebuild_w, sub, toggle};
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
const OP: &str = "dissolve";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Edges"),
        ParamSpec::new("invertsel", ParamValue::B(false)).label("Invert Selection"),
        ParamSpec::new("compnorms", ParamValue::B(false)).label("Recompute Normals"),
        ParamSpec::new("reminlinepts", ParamValue::B(true)).label("Remove Inline Points"),
        ParamSpec::new("coltol", ParamValue::F(45.0)).label("Colinear Tolerance").range(0.0, 180.0),
        ParamSpec::new("remunusedpts", ParamValue::B(true)).label("Remove Unused Points"),
        ParamSpec::new("bridge", ParamValue::B(false)).label("Bridge"),
        ParamSpec::new("deldegeneratebridges", ParamValue::B(false)).label("Delete Degenerate Bridges"),
        ParamSpec::new("boundarycurves", ParamValue::B(false)).label("Boundary Curves"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Dissolve { specs: spec() })
}
struct Dissolve {
    specs: Vec<ParamSpec>,
}
fn unsupported(m: &str) -> CookError {
    CookError::Unsupported(format!("{OP}: {m}"))
}
/// `pA-B` tokens -> undirected edges (low, high).
fn parse_edges(pat: &str) -> Result<Option<BTreeSet<(u32, u32)>>, CookError> {
    let toks: Vec<&str> = pat.split_whitespace().collect();
    if toks.is_empty() {
        return Ok(None);
    }
    let mut out = BTreeSet::new();
    for t in toks {
        let body = t
            .strip_prefix('p')
            .ok_or_else(|| unsupported(&format!("group token `{t}`: only edge tokens `pA-B` are modelled")))?;
        let (a, b) = body.split_once('-').ok_or_else(|| bad(format!("`group`: bad edge `{t}`")))?;
        let num = |s: &str| s.parse::<u32>().map_err(|_| bad(format!("`group`: bad edge `{t}`")));
        let (a, b) = (num(a)?, num(b)?);
        out.insert((a.min(b), a.max(b)));
    }
    Ok(Some(out))
}
fn find(uf: &mut [usize], mut x: usize) -> usize {
    while uf[x] != x {
        uf[x] = uf[uf[x]];
        x = uf[x];
    }
    x
}
struct Face {
    prim: usize,
    pts: Vec<u32>,
    v0: usize,
}
impl Node<Detail> for Dissolve {
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
        let pat = p.string("group").ok_or_else(|| bad("`group`: missing or not a string"))?;
        let (reminline, remunused) = (toggle(p, "reminlinepts")?, toggle(p, "remunusedpts")?);
        let coltol = float(p, "coltol")?;
        for n in ["invertsel", "bridge", "boundarycurves", "deldegeneratebridges"] {
            if toggle(p, n)? {
                return Err(unsupported(&format!("`{n}` is not modelled")));
            }
        }
        toggle(p, "compnorms")?;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let edges = parse_edges(pat)?;
        let pos = positions(src);
        // closed polygon faces
        let mut faces: Vec<Face> = Vec::new();
        let mut face_of = vec![usize::MAX; src.prims.len()];
        for (i, pr) in src.prims.iter().enumerate() {
            if !matches!(pr.kind, PrimKind::Poly { closed: true }) {
                continue;
            }
            let (lo, hi) = (pr.verts.start as usize, pr.verts.end as usize);
            let Some(vs) = src.vertex_point.get(lo..hi) else { continue };
            if vs.len() < 3 || vs.iter().any(|&q| q >= src.point_count) {
                continue;
            }
            face_of[i] = faces.len();
            faces.push(Face { prim: i, pts: vs.to_vec(), v0: lo });
        }
        // directed edge -> (face, index)
        let mut dir: BTreeMap<(u32, u32), Vec<(usize, usize)>> = BTreeMap::new();
        for (f, fc) in faces.iter().enumerate() {
            for k in 0..fc.pts.len() {
                dir.entry((fc.pts[k], fc.pts[(k + 1) % fc.pts.len()])).or_default().push((f, k));
            }
        }
        let mut uf: Vec<usize> = (0..faces.len()).collect();
        let mut doomed = vec![false; faces.len()];
        let mut merged_edges: BTreeSet<(usize, usize)> = BTreeSet::new(); // (face, idx) of dissolved directed edges
        let all = edges.is_none();
        if all {
            doomed.fill(true);
        }
        for &(a, b) in edges.iter().flatten() {
            let fw = dir.get(&(a, b)).cloned().unwrap_or_default();
            let bw = dir.get(&(b, a)).cloned().unwrap_or_default();
            match (fw.as_slice(), bw.as_slice()) {
                ([], []) => {}
                ([x], []) | ([], [x]) => doomed[x.0] = true,
                ([x], [y]) if x.0 != y.0 => {
                    merged_edges.insert(*x);
                    merged_edges.insert(*y);
                    let (ra, rb) = (find(&mut uf, x.0), find(&mut uf, y.0));
                    uf[ra.max(rb)] = ra.min(rb);
                }
                _ => return Err(unsupported(&format!("edge p{a}-{b} is non-manifold or inconsistently wound"))),
            }
        }
        // regions
        let mut regions: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for f in 0..faces.len() {
            let r = find(&mut uf, f);
            regions.entry(r).or_default().push(f);
        }
        let mut touched = vec![false; faces.len()];
        struct Merged {
            first: usize,
            pts: Vec<u32>,
            verts: Vec<usize>,
        }
        let mut merged: Vec<Merged> = Vec::new();
        for members in regions.values() {
            let region_doomed = members.iter().any(|&f| doomed[f]);
            let is_merge = members.len() > 1;
            if !is_merge && !region_doomed {
                continue;
            }
            for &f in members {
                touched[f] = true;
            }
            if region_doomed {
                continue;
            }
            let first = members[0];
            let start = (0..faces[first].pts.len())
                .find(|&k| !merged_edges.contains(&(first, k)))
                .ok_or_else(|| failed(OP, "region has no boundary"))?;
            let twin = |f: usize, k: usize| -> Option<(usize, usize)> {
                let fc = &faces[f];
                let key = (fc.pts[(k + 1) % fc.pts.len()], fc.pts[k]);
                dir.get(&key).and_then(|v| v.iter().copied().find(|&(g, _)| g != f))
            };
            let limit: usize = members.iter().map(|&f| faces[f].pts.len()).sum::<usize>() + 2;
            let (mut pts, mut verts) = (Vec::new(), Vec::new());
            let mut cur = (first, start);
            let mut steps = 0usize;
            let mut consumed = 0usize;
            loop {
                let fc = &faces[cur.0];
                pts.push(fc.pts[cur.1]);
                verts.push(fc.v0 + cur.1);
                consumed += 1;
                let mut nxt = (cur.0, (cur.1 + 1) % fc.pts.len());
                while merged_edges.contains(&nxt) {
                    let Some(t) = twin(nxt.0, nxt.1) else { return Err(failed(OP, "dangling dissolved edge")) };
                    nxt = (t.0, (t.1 + 1) % faces[t.0].pts.len());
                    steps += 1;
                    if steps > limit * 4 {
                        return Err(failed(OP, "boundary walk did not close"));
                    }
                }
                cur = nxt;
                if cur == (first, start) {
                    break;
                }
                if consumed > limit {
                    return Err(failed(OP, "boundary walk did not close"));
                }
            }
            let boundary: usize = members
                .iter()
                .map(|&f| (0..faces[f].pts.len()).filter(|&k| !merged_edges.contains(&(f, k))).count())
                .sum();
            if boundary != consumed {
                return Err(unsupported("merged region has holes (several boundary loops; needs `bridge`)"));
            }
            merged.push(Merged { first, pts, verts });
        }
        merged.sort_by_key(|m| m.first);
        // slot count per point over the surviving topology (for inline-point detection)
        let mut slots = vec![0u32; src.point_count as usize];
        for (f, fc) in faces.iter().enumerate() {
            if !touched[f] {
                for &q in &fc.pts {
                    slots[q as usize] += 1;
                }
            }
        }
        for (i, pr) in src.prims.iter().enumerate() {
            if face_of[i] == usize::MAX {
                for &q in src.vertex_point.get(pr.verts.start as usize..pr.verts.end as usize).unwrap_or(&[]) {
                    if let Some(s) = slots.get_mut(q as usize) {
                        *s += 1;
                    }
                }
            }
        }
        for m in &merged {
            for &q in &m.pts {
                slots[q as usize] += 1;
            }
        }
        if reminline {
            for m in &mut merged {
                let mut k = 0;
                while k < m.pts.len() && m.pts.len() > 3 {
                    let n = m.pts.len();
                    let (a, b, c) = (m.pts[(k + n - 1) % n], m.pts[k], m.pts[(k + 1) % n]);
                    let (d1, d2): (V3, V3) =
                        (sub(pos[b as usize], pos[a as usize]), sub(pos[c as usize], pos[b as usize]));
                    let (l1, l2) = (dot(d1, d1).sqrt(), dot(d2, d2).sqrt());
                    let turn = if l1 > 0.0 && l2 > 0.0 {
                        (dot(d1, d2) / (l1 * l2)).clamp(-1.0, 1.0).acos().to_degrees()
                    } else {
                        180.0
                    };
                    if slots[b as usize] == 1 && turn < coltol {
                        m.pts.remove(k);
                        m.verts.remove(k);
                        slots[b as usize] = 0;
                    } else {
                        k += 1;
                    }
                }
            }
        }
        // assemble: untouched prims, then merged regions
        let mut vertex_point: Vec<u32> = Vec::new();
        let mut vert_src: Vec<Option<usize>> = Vec::new();
        let mut prims: Vec<Prim> = Vec::new();
        let mut prim_src: Vec<Option<usize>> = Vec::new();
        for (i, pr) in src.prims.iter().enumerate() {
            let f = face_of[i];
            if f != usize::MAX && touched[f] {
                continue;
            }
            if f == usize::MAX && all && matches!(pr.kind, PrimKind::Poly { closed: true }) {
                continue;
            }
            let (lo, hi) = (pr.verts.start as usize, pr.verts.end as usize);
            let start = vertex_point.len() as u32;
            for (j, &q) in src.vertex_point.get(lo..hi).unwrap_or(&[]).iter().enumerate() {
                vertex_point.push(q);
                vert_src.push(Some(lo + j));
            }
            prims.push(Prim { kind: pr.kind.clone(), verts: start..vertex_point.len() as u32 });
            prim_src.push(Some(i));
        }
        for m in &merged {
            let start = vertex_point.len() as u32;
            vertex_point.extend(&m.pts);
            vert_src.extend(m.verts.iter().map(|&v| Some(v)));
            prims.push(Prim { kind: PrimKind::Poly { closed: true }, verts: start..vertex_point.len() as u32 });
            prim_src.push(Some(faces[m.first].prim));
        }
        if vertex_point.len() as u64 > MAX_ELEMS {
            return Err(failed(OP, "output too large"));
        }
        // point compaction
        let mut used = vec![!remunused; src.point_count as usize];
        for &q in &vertex_point {
            used[q as usize] = true;
        }
        let mut remap = vec![0u32; used.len()];
        let mut point_w: Vec<Vec<(usize, f64)>> = Vec::new();
        let mut np: Vec<[f32; 3]> = Vec::new();
        for (q, u) in used.iter().enumerate() {
            if *u {
                remap[q] = point_w.len() as u32;
                point_w.push(vec![(q, 1.0)]);
                np.push(src.p()[q]);
            }
        }
        for q in &mut vertex_point {
            *q = remap[*q as usize];
        }
        Ok(Arc::new(rebuild_w(OP, src, vertex_point, prims, &point_w, &vert_src, &prim_src, np)?))
    }
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cube, defaults, houdini_twin, houdini_values};
    use super::*;
    use hexe_geo::{AttrClass, AttrData, Attribute, Group, TypeInfo};
    use hexe_graph::ParamSet;
    /// 4x4 point grid, 3x3 quads, Houdini grid point order, cells `[a, a+1, a+5, a+4]`.
    fn grid() -> Detail {
        let mut d = Detail::default();
        d.add_points(16);
        *d.p_mut() = (0..16).map(|i| [(i % 4) as f32, 0.0, (i / 4) as f32]).collect();
        for r in 0..3u32 {
            for c in 0..3u32 {
                let a = r * 4 + c;
                d.add_poly(&[a, a + 1, a + 5, a + 4], true);
            }
        }
        d
    }
    fn run(d: Detail, g: &str, f: impl FnOnce(ParamSet) -> ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        cook(&*n, d, &f(defaults(&*n)).with("group", ParamValue::S(g.into())))
    }
    fn cyc(d: &Detail, i: usize) -> Vec<u32> {
        d.vertex_point[d.prims[i].verts.start as usize..d.prims[i].verts.end as usize].to_vec()
    }
    #[test]
    fn merge_walk_matches_the_probed_boxes() {
        let d = run(cube(), "p0-1", |p| p).expect("cooks");
        assert_eq!(d.prims.len(), 5);
        // hexe testkit cube winding differs from Houdini's; only the shape (L of two quads) is checked here, the order by the twin
        assert_eq!(cyc(&d, 4).len(), 6);
    }
    #[test]
    fn inline_points_unused_points_and_boundary_edges() {
        let a = run(grid(), "p1-5", |p| p).expect("cooks");
        assert_eq!((a.point_count, a.prims.len()), (15, 8));
        assert_eq!(cyc(&a, 7), vec![0, 1, 5, 4, 3]);
        let b = run(grid(), "p1-5", |p| p.with("remunusedpts", ParamValue::B(false))).expect("cooks");
        assert_eq!((b.point_count, cyc(&b, 7)), (16, vec![0, 2, 6, 5, 4]));
        let c = run(grid(), "p1-5", |p| p.with("coltol", ParamValue::F(0.0))).expect("cooks");
        assert_eq!(cyc(&c, 7), vec![0, 1, 2, 6, 5, 4]);
        let e = run(grid(), "p0-1 p0-4", |p| p).expect("cooks");
        assert_eq!((e.point_count, e.prims.len()), (15, 8));
        assert_eq!(cyc(&e, 0), vec![0, 1, 5, 4]);
    }
    #[test]
    fn regions_follow_first_prim_and_empty_group_deletes_all() {
        let r = run(grid(), "p1-5 p9-10 p10-14", |p| p.with("reminlinepts", ParamValue::B(false))).expect("cooks");
        assert_eq!(r.prims.len(), 6);
        assert_eq!(cyc(&r, 5), vec![5, 6, 10, 11, 15, 14, 13, 9]);
        let all = run(grid(), "", |p| p).expect("cooks");
        assert_eq!((all.point_count, all.prims.len()), (0, 0));
    }
    #[test]
    fn attrs_follow_the_first_prim_and_compact_with_points() {
        let mut d = grid();
        d.add_attr(Attribute {
            name: "pa".into(),
            class: AttrClass::Prim,
            data: AttrData::I32((0..9).collect()),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: AttrData::F32((0..16).map(|i| i as f32).collect()),
            type_info: TypeInfo::None,
        });
        d.groups.push(Group { name: "odd".into(), class: AttrClass::Prim, mask: (0..9).map(|i| i % 2 == 1).collect() });
        let o = run(d, "p1-5", |p| p).expect("cooks");
        assert_eq!(
            o.attr(AttrClass::Prim, "pa").map(|a| a.data.clone()),
            Some(AttrData::I32(vec![2, 3, 4, 5, 6, 7, 8, 0]))
        );
        // point 1 (inline) gone -> w = 0,2,3,4,...
        assert_eq!(o.attr(AttrClass::Point, "w").map(|a| a.data.len()), Some(15));
        assert_eq!(
            o.group(AttrClass::Prim, "odd").map(|g| g.mask.clone()),
            Some(vec![false, true, false, true, false, true, false, false])
        );
    }
    #[test]
    fn unsupported_and_hostile() {
        assert!(matches!(run(grid(), "0", |p| p), Err(CookError::Unsupported(_))));
        assert!(matches!(
            run(grid(), "p1-5", |p| p.with("invertsel", ParamValue::B(true))),
            Err(CookError::Unsupported(_))
        ));
        assert!(matches!(
            run(grid(), "p1-5", |p| p.with("bridge", ParamValue::B(true))),
            Err(CookError::Unsupported(_))
        ));
        assert!(run(grid(), "p1-", |p| p).is_err());
        assert!(run(grid(), "p99-100", |p| p).is_ok());
        assert!(run(grid(), "p1-5", |p| p.with("coltol", ParamValue::F(f32::NAN))).is_err());
        assert_eq!(run(Detail::default(), "", |p| p).expect("empty").point_count, 0);
        // ring around an interior vertex-free hole: 4 cells dissolved around the center cell leaves one loop
        assert!(run(grid(), "p5-6 p6-10 p10-9 p9-5", |p| p).is_ok());
    }
    macro_rules! twin {
        ($n:literal) => {{
            let (scene, geo) = (
                include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
            );
            houdini_twin(scene, geo).and_then(|()| houdini_values(scene, geo, 1e-5))
        }};
    }
    /// Hexe twin == real Houdini 20.5 `.geo`: merge + inline points, two regions, 3-face box merge, boundary deletion, empty group.
    #[test]
    fn dissolve_matches_houdini_fixtures() {
        for n in [
            twin!("sop-dissolve"),
            twin!("sop-dissolve-regions"),
            twin!("sop-dissolve-box"),
            twin!("sop-dissolve-boundary"),
            twin!("sop-dissolve-all"),
        ] {
            assert_eq!(n, Ok(()));
        }
    }
}
