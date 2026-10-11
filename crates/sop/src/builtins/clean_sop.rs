//! Clean SOP (token `clean`): remove degenerate / unused geometry and split non-manifold surfaces (Houdini 20.5 `clean`).
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-clean*`)
//! * `deldegengeo` (default on), tolerance `degentol` (default 0.001): a CLOSED poly is deleted when it has `< 3` vertices or its area
//!   (Newell, half the cross-sum magnitude) is `< degentol^2` (probes: area .00025 survives tol .001; area .004 dies at tol .1, survives .05;
//!   4 collinear points die at the default and SURVIVE tol 0 -> strict `<`). An OPEN poly is deleted when its total length is `<= degentol`
//!   (probe: length .05 dies at tol .05 and .1, a length-1 line dies at tol 1). Other prim kinds are never deleted. Polygon order is kept.
//! * Unused points go when `delunusedpts` OR `deldegengeo` is on (probed: with `deldegengeo` on, `delunusedpts` off still drops every
//!   unused point; both off keeps them). Point attrs / groups compact with the points.
//! * `make_manifold` (default off), `delete_small` (default on), `prim_count` (default 3): runs on the polygons that survived the degenerate pass.
//!   At every point the vertices that use it are partitioned into FANS: two vertices join when their prims share an edge at this point
//!   that exactly two prims (distinct) use. Non-manifold edges (3+ uses) join nothing; a bow-tie point splits too. Walking the vertices
//!   in DESCENDING vertex number, the first fan keeps the point, every further fan gets a NEW point (copy of the original, appended after
//!   all existing points in point order, fan by fan). `delete_small` then deletes every connected piece (prims linked by shared points after
//!   the split) with `<= prim_count` prims, even when nothing was split; it does nothing without `make_manifold`.
//! * `removeunusedgrp` (default on): groups that end up empty are dropped.
//!
//! Not modelled (-> `Unsupported`): `fusepts`, `orientpoly`, `reversewinding`, `fixoverlap`, `dodelattribs`, `dodelgroups`, non-finite `P`
//! (`delnans` would delete them), `make_manifold` on input with open polys / non-poly prims. Repeated-vertex collapse inside a poly is not modelled
//! (area test only).
use super::meshutil::{MAX_ELEMS, V3, bad, cross, failed, float, int, positions, rebuild_w, sub, toggle};
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::BTreeMap;
use std::sync::Arc;
const OP: &str = "clean";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("fusepts", ParamValue::B(false)).label("Fuse Overlapping Points"),
        ParamSpec::new("fusedist", ParamValue::F(0.001)).label("Fuse Distance").range(0.0, 10.0),
        ParamSpec::new("deldegengeo", ParamValue::B(true)).label("Delete Degenerate Geometry"),
        ParamSpec::new("degentol", ParamValue::F(0.001)).label("Degenerate Tolerance").range(0.0, 10.0),
        ParamSpec::new("removeunusedgrp", ParamValue::B(true)).label("Remove Unused Groups"),
        ParamSpec::new("orientpoly", ParamValue::B(false)).label("Orient Polygons"),
        ParamSpec::new("reversewinding", ParamValue::B(false)).label("Reverse Winding"),
        ParamSpec::new("fixoverlap", ParamValue::B(false)).label("Fix Overlaps"),
        ParamSpec::new("deleteoverlap", ParamValue::B(true)).label("Delete Overlapping Polygons"),
        ParamSpec::new("delunusedpts", ParamValue::B(true)).label("Delete Unused Points"),
        ParamSpec::new("dodelattribs", ParamValue::B(false)).label("Delete Attributes"),
        ParamSpec::new("delattribs", ParamValue::S("*".into())).label("Attributes"),
        ParamSpec::new("dodelgroups", ParamValue::B(false)).label("Delete Groups"),
        ParamSpec::new("delgroups", ParamValue::S("*".into())).label("Groups"),
        ParamSpec::new("delnans", ParamValue::B(true)).label("Delete NaNs"),
        ParamSpec::new("make_manifold", ParamValue::B(false)).label("Fix Non-Manifold"),
        ParamSpec::new("delete_small", ParamValue::B(true)).label("Delete Small Pieces"),
        ParamSpec::new("prim_count", ParamValue::I(3)).label("Primitive Count").range(0.0, 1000.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Clean { specs: spec() })
}
/// undirected edge -> uses `(prim, slot of lo endpoint, slot of hi endpoint)`.
type EdgeUses = BTreeMap<(u32, u32), Vec<(usize, usize, usize)>>;
struct Clean {
    specs: Vec<ParamSpec>,
}
fn unsupported(m: &str) -> CookError {
    CookError::Unsupported(format!("{OP}: {m}"))
}
fn find(uf: &mut [usize], mut x: usize) -> usize {
    while uf[x] != x {
        uf[x] = uf[uf[x]];
        x = uf[x];
    }
    x
}
fn union(uf: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(uf, a), find(uf, b));
    if ra != rb {
        uf[ra.max(rb)] = ra.min(rb);
    }
}
/// One surviving prim: kind, its point ids and the SOURCE vertex numbers they came from.
struct Item {
    kind: PrimKind,
    pts: Vec<u32>,
    verts: Vec<usize>,
    src: usize,
}
fn degenerate(it: &Item, pos: &[V3], tol: f64) -> bool {
    match it.kind {
        PrimKind::Poly { closed: true } => {
            if it.pts.len() < 3 {
                return true;
            }
            let o = pos[it.pts[0] as usize];
            let mut n = [0.0; 3];
            for k in 1..it.pts.len() - 1 {
                let c = cross(sub(pos[it.pts[k] as usize], o), sub(pos[it.pts[k + 1] as usize], o));
                for (a, b) in n.iter_mut().zip(c) {
                    *a += b;
                }
            }
            let area = 0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            area < tol * tol
        }
        PrimKind::Poly { closed: false } => {
            let len: f64 = it
                .pts
                .windows(2)
                .map(|w| {
                    let d = sub(pos[w[1] as usize], pos[w[0] as usize]);
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                })
                .sum();
            it.pts.len() < 2 || len <= tol
        }
        _ => false,
    }
}
impl Node<Detail> for Clean {
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
        for n in ["fusepts", "orientpoly", "reversewinding", "fixoverlap", "dodelattribs", "dodelgroups"] {
            if toggle(p, n)? {
                return Err(unsupported(&format!("`{n}` is not modelled")));
            }
        }
        let (degen, unused_pts) = (toggle(p, "deldegengeo")?, toggle(p, "delunusedpts")?);
        let (rm_grp, manifold, small) =
            (toggle(p, "removeunusedgrp")?, toggle(p, "make_manifold")?, toggle(p, "delete_small")?);
        let (tol, prim_count) = (float(p, "degentol")?, int(p, "prim_count")?);
        float(p, "fusedist")?;
        toggle(p, "deleteoverlap")?;
        toggle(p, "delnans")?;
        if tol < 0.0 || prim_count < 0 {
            return Err(bad("`degentol` / `prim_count` must not be negative"));
        }
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if src.p().iter().any(|q| q.iter().any(|c| !c.is_finite())) {
            return Err(unsupported("non-finite P (delnans) is not modelled"));
        }
        let pos = positions(src);
        let mut items: Vec<Item> = Vec::with_capacity(src.prims.len());
        for (i, pr) in src.prims.iter().enumerate() {
            let (lo, hi) = (pr.verts.start as usize, pr.verts.end as usize);
            let Some(vs) = src.vertex_point.get(lo..hi) else { continue };
            if vs.iter().any(|&q| q >= src.point_count) {
                continue;
            }
            items.push(Item { kind: pr.kind.clone(), pts: vs.to_vec(), verts: (lo..hi).collect(), src: i });
        }
        if degen {
            items.retain(|it| !degenerate(it, &pos, tol));
        }
        // point table: (source point, weight) per output point
        let mut srcpt: Vec<usize> = (0..src.point_count as usize).collect();
        if manifold {
            if items.iter().any(|it| !matches!(it.kind, PrimKind::Poly { closed: true })) {
                return Err(unsupported("`make_manifold` with open polys / non-poly prims is not modelled"));
            }
            // global slot numbering = position in the surviving vertex list
            let mut base = Vec::with_capacity(items.len());
            let mut total = 0usize;
            for it in &items {
                base.push(total);
                total += it.pts.len();
            }
            // edge uses: undirected (lo, hi) -> [(prim, slot of first endpoint, slot of second endpoint)]
            let mut uses: EdgeUses = BTreeMap::new();
            for (f, it) in items.iter().enumerate() {
                let n = it.pts.len();
                for k in 0..n {
                    let (a, b) = (it.pts[k], it.pts[(k + 1) % n]);
                    let (sa, sb) = (base[f] + k, base[f] + (k + 1) % n);
                    let e = if a <= b { ((a, b), sa, sb) } else { ((b, a), sb, sa) };
                    uses.entry(e.0).or_default().push((f, e.1, e.2));
                }
            }
            let mut uf: Vec<usize> = (0..total).collect();
            for u in uses.values() {
                if let [x, y] = u.as_slice()
                    && x.0 != y.0
                {
                    union(&mut uf, x.1, y.1);
                    union(&mut uf, x.2, y.2);
                }
            }
            // slots per point
            let mut at: Vec<Vec<usize>> = vec![Vec::new(); src.point_count as usize];
            for (f, it) in items.iter().enumerate() {
                for (k, &q) in it.pts.iter().enumerate() {
                    at[q as usize].push(base[f] + k);
                }
            }
            let mut new_pt: Vec<u32> = vec![0; total];
            // descending vertex number; first fan keeps the point, later fans get fresh points (point order, fan by fan)
            for (q, slots) in at.iter().enumerate() {
                let mut fans: Vec<(usize, u32)> = Vec::new();
                for &s in slots.iter().rev() {
                    let r = find(&mut uf, s);
                    let id = match fans.iter().find(|f| f.0 == r) {
                        Some(f) => f.1,
                        None => {
                            let id = if fans.is_empty() {
                                q as u32
                            } else {
                                srcpt.push(q);
                                (srcpt.len() - 1) as u32
                            };
                            fans.push((r, id));
                            id
                        }
                    };
                    new_pt[s] = id;
                }
            }
            for (f, it) in items.iter_mut().enumerate() {
                for (k, q) in it.pts.iter_mut().enumerate() {
                    *q = new_pt[base[f] + k];
                }
            }
            if small {
                let mut cc: Vec<usize> = (0..items.len()).collect();
                let mut first: Vec<Option<usize>> = vec![None; srcpt.len()];
                for (f, it) in items.iter().enumerate() {
                    for &q in &it.pts {
                        match first[q as usize] {
                            Some(g) => union(&mut cc, f, g),
                            None => first[q as usize] = Some(f),
                        }
                    }
                }
                let mut size = vec![0usize; items.len()];
                for f in 0..items.len() {
                    let r = find(&mut cc, f);
                    size[r] += 1;
                }
                let keep: Vec<bool> = (0..items.len()).map(|f| size[find(&mut cc, f)] > prim_count as usize).collect();
                let mut k = 0;
                items.retain(|_| {
                    k += 1;
                    keep[k - 1]
                });
            }
        }
        // assemble
        let mut vertex_point: Vec<u32> = Vec::new();
        let mut vert_src: Vec<Option<usize>> = Vec::new();
        let mut prims: Vec<Prim> = Vec::new();
        let mut prim_src: Vec<Option<usize>> = Vec::new();
        for it in &items {
            let start = vertex_point.len() as u32;
            vertex_point.extend(&it.pts);
            vert_src.extend(it.verts.iter().map(|&v| Some(v)));
            prims.push(Prim { kind: it.kind.clone(), verts: start..vertex_point.len() as u32 });
            prim_src.push(Some(it.src));
        }
        if vertex_point.len() as u64 > MAX_ELEMS || srcpt.len() as u64 > MAX_ELEMS {
            return Err(failed(OP, "output too large"));
        }
        let drop_unused = unused_pts || degen;
        let mut used = vec![!drop_unused; srcpt.len()];
        for &q in &vertex_point {
            used[q as usize] = true;
        }
        let mut remap = vec![0u32; srcpt.len()];
        let mut point_w: Vec<Vec<(usize, f64)>> = Vec::new();
        let mut np: Vec<[f32; 3]> = Vec::new();
        for (q, u) in used.iter().enumerate() {
            if *u {
                remap[q] = point_w.len() as u32;
                point_w.push(vec![(srcpt[q], 1.0)]);
                np.push(src.p()[srcpt[q]]);
            }
        }
        for q in &mut vertex_point {
            *q = remap[*q as usize];
        }
        let mut out = rebuild_w(OP, src, vertex_point, prims, &point_w, &vert_src, &prim_src, np)?;
        if rm_grp {
            out.groups.retain(|g| g.mask.iter().any(|&b| b));
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, defaults, houdini_twin, houdini_values};
    use super::*;
    use hexe_graph::ParamSet;
    fn run(d: Detail, f: impl FnOnce(ParamSet) -> ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        cook(&*n, d, &f(defaults(&*n)))
    }
    fn mesh(pts: &[[f32; 3]], polys: &[(&[u32], bool)]) -> Detail {
        let mut d = Detail::default();
        d.add_points(pts.len() as u32);
        *d.p_mut() = pts.to_vec();
        for (p, c) in polys {
            d.add_poly(p, *c);
        }
        d
    }
    fn cyc(d: &Detail, i: usize) -> Vec<u32> {
        d.vertex_point[d.prims[i].verts.start as usize..d.prims[i].verts.end as usize].to_vec()
    }
    #[test]
    fn degenerate_rules_follow_the_probes() {
        let pts =
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.5, 0.0005, 0.0], [9.0, 9.0, 9.0]];
        let d = || mesh(&pts, &[(&[0, 1, 2], true), (&[0, 1, 3], true), (&[0, 1, 4], true), (&[0, 1], false)]);
        let o = run(d(), |p| p).expect("cooks");
        // collinear tri dies, thin tri (area .00025 > tol^2) lives, unused point 5 and now-unused point 2 go
        assert_eq!((o.point_count, o.prims.len()), (4, 3));
        assert_eq!(cyc(&o, 0), vec![0, 1, 2]);
        let z = run(d(), |p| p.with("degentol", ParamValue::F(0.0))).expect("cooks");
        assert_eq!(z.prims.len(), 4);
        let big = run(d(), |p| p.with("degentol", ParamValue::F(1.0))).expect("cooks");
        assert_eq!((big.point_count, big.prims.len()), (0, 0));
        // both off keeps everything incl. the unused point; deldegengeo off alone still drops unused points
        let keep = run(d(), |p| p.with("deldegengeo", ParamValue::B(false)).with("delunusedpts", ParamValue::B(false)))
            .expect("cooks");
        assert_eq!((keep.point_count, keep.prims.len()), (6, 4));
        let un = run(d(), |p| p.with("deldegengeo", ParamValue::B(false))).expect("cooks");
        assert_eq!((un.point_count, un.prims.len()), (5, 4));
    }
    #[test]
    fn bowtie_and_nonmanifold_edges_split_points_in_descending_vertex_order() {
        // bowtie: point 1 shared by two quads only
        let pts = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
        ];
        let d = mesh(&pts, &[(&[0, 1, 2, 3], true), (&[1, 4, 5, 6], true)]);
        let o = run(d, |p| p.with("make_manifold", ParamValue::B(true)).with("delete_small", ParamValue::B(false)))
            .expect("cooks");
        assert_eq!((o.point_count, cyc(&o, 0), cyc(&o, 1)), (8, vec![0, 7, 2, 3], vec![1, 4, 5, 6]));
        // three quads on the edge 0-1: the LAST prim keeps the points, earlier ones get new ones, in descending order
        let pts = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [1.0, -1.0, 0.0],
            [0.0, -1.0, 0.5],
        ];
        let d = mesh(&pts, &[(&[0, 1, 2, 3], true), (&[0, 1, 4, 5], true), (&[0, 1, 6, 7], true)]);
        let o = run(d, |p| p.with("make_manifold", ParamValue::B(true)).with("delete_small", ParamValue::B(false)))
            .expect("cooks");
        assert_eq!(o.point_count, 12);
        assert_eq!((cyc(&o, 0), cyc(&o, 1), cyc(&o, 2)), (vec![9, 11, 2, 3], vec![8, 10, 4, 5], vec![0, 1, 6, 7]));
    }
    #[test]
    fn delete_small_needs_make_manifold_and_uses_less_or_equal() {
        let pts =
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0]];
        let d = || mesh(&pts, &[(&[0, 1, 2, 3], true), (&[1, 4, 5, 2], true)]);
        let off = run(d(), |p| p).expect("cooks");
        assert_eq!(off.prims.len(), 2);
        let del =
            |c: i32| run(d(), |p| p.with("make_manifold", ParamValue::B(true)).with("prim_count", ParamValue::I(c)));
        assert_eq!(del(2).expect("cooks").prims.len(), 0);
        assert_eq!(del(1).expect("cooks").prims.len(), 2);
    }
    #[test]
    fn unsupported_and_hostile() {
        let d = || mesh(&[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], &[(&[0, 1, 2], true)]);
        for n in ["fusepts", "orientpoly", "reversewinding", "fixoverlap", "dodelattribs", "dodelgroups"] {
            assert!(matches!(run(d(), |p| p.with(n, ParamValue::B(true))), Err(CookError::Unsupported(_))), "{n}");
        }
        let open = mesh(&[[0.0; 3], [1.0, 0.0, 0.0]], &[(&[0, 1], false)]);
        assert!(matches!(run(open, |p| p.with("make_manifold", ParamValue::B(true))), Err(CookError::Unsupported(_))));
        let nan = mesh(&[[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], &[(&[0, 1, 2], true)]);
        assert!(matches!(run(nan, |p| p), Err(CookError::Unsupported(_))));
        assert!(matches!(run(d(), |p| p.with("degentol", ParamValue::F(-1.0))), Err(CookError::BadParam(_))));
        assert_eq!(run(Detail::default(), |p| p).expect("cooks").prim_count(), 0);
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
    /// Hexe twin == real Houdini 20.5 `.geo`: defaults, non-manifold fans, unused points, delete_small.
    #[test]
    fn clean_matches_houdini_fixtures() {
        for n in [twin!("sop-clean"), twin!("sop-clean-manifold"), twin!("sop-clean-unused"), twin!("sop-clean-small")]
        {
            assert_eq!(n, Ok(()));
        }
    }
}
