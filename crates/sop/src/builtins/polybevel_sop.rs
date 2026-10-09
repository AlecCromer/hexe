//! PolyBevel SOP (token `polybevel`): edge bevel of closed polygons (chamfer or rounded fillet).
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! A BEVELABLE edge is an undirected edge used by exactly two valid closed polys (>= 3 distinct corners, non-degenerate
//! Newell normal) with opposite directions (consistent winding). Boundary / non-manifold / half-flipped edges are never
//! beveled. Selected = `group` empty (all) or listed (see Params).
//!
//! Geometry: every face boundary along a beveled edge moves INTO the face, in the face plane, by `offset` (perpendicular
//! distance). A face corner lands on the intersection of its two boundary lines (shifted if beveled, original if not):
//! both edges beveled `v + d/(1+c) (in1 + in2)` (`in*` = inward unit normals, `c = in1.in2`; 90-degree corner = `v + d(in1
//! + in2)`), one beveled = slide along the other edge. Each beveled edge becomes a strip of `divisions` quads.
//! Each vertex splits into SECTORS: face corners joined across unbeveled edges share one point (no cracks); a vertex with
//! no beveled edge keeps a single point; a sector point = mean of its corner positions. A vertex whose sectors form a
//! closed loop of beveled edge ends gets a CAP polygon (>= 3 distinct points) closing the hole; fewer sectors (1-2) need
//! none (strip ends meet / taper to a triangle).
//! Cube, all edges, `divisions = 1`: 24 pts = 8 x 3 sectors; 6 shrunk quads + 12 strip quads + 8 corner tris = 26 prims.
//! `divisions = n`: + 12 x 2 x (n-1) profile pts, 12 n strip quads, 8 caps of 3n corners (n = 2: 48 pts / 38 prims).
//!
//! Profile (cross-section from tangent point T1 on face 1 to T2 on face 2, apex = the original edge point shifted along
//! the edge to the mean of T1/T2): `round` = conic arc, weight `w = sin(theta/2)`, `theta` = angle T1-apex-T2 (exact
//! circle tangent to both faces; 90-degree edge: `w = sqrt(1/2)`); `solid` = straight (linear) profile, `divisions` segments.
//!
//! Attributes: sector / profile points copy the point attrs of their source vertex (float attrs average when several
//! sources; here one); shrunk faces keep their vertex attrs; strips copy the prim attrs of their first face (vertex attrs
//! default-fill except at the strip ends); caps default-fill; groups follow the same maps. Existing `N` (V3, any of
//! point / vertex / prim) is RECOMPUTED: point = unit-summed Newell normals of incident closed polys, vertex / prim = flat
//! Newell normal of the owning prim. Prims that are not valid closed polys pass through (their points map to the first
//! point of the source vertex). Not modelled: edge-angle limits, per-edge offsets, uv fix-up, n-gon corner caps shaped
//! for sharp / non-planar corners, concave offset-collision cleanup.
//!
//! Params: `group` S (space-separated tokens; `a-b` = the edge between point numbers a and b, anything else = a POINT
//! group naming both endpoints; empty = all edges; unknown group = `Failed`), `offset` F 0.1 (negative / non-finite =
//! `BadParam`; 0 = input `Arc` shared), `divisions` I 1 (1..=64 else `BadParam`), `filletshape` menu (0 round,
//! 1 solid; else `BadParam`). Element counts above `MAX_ELEMS` -> `Failed` BEFORE allocating.
use super::meshutil::dot;
use super::meshutil::{
    MAX_ELEMS, Remap, V3, add, bad, closed_poly_pts, failed, float, int, point_normals, positions, prim_normal,
    rebuild, scale, sub, to_f32,
};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

const OP: &str = "polybevel";
const MAX_DIVISIONS: i32 = 64;
const SHAPE_ROUND: u32 = 0;
const SHAPE_SOLID: u32 = 1;
const EPS: f64 = 1e-9;
const CLOSED: PrimKind = PrimKind::Poly { closed: true };
const ZERO: V3 = [0.0; 3];
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("offset", ParamValue::F(0.1)).label("Offset").range(0.0, 10.0),
        ParamSpec::new("divisions", ParamValue::I(1)).label("Divisions").range(1.0, 10.0),
        ParamSpec::new("filletshape", ParamValue::Menu(SHAPE_ROUND))
            .label("Fillet Shape")
            .menu(&[("round", "Round"), ("solid", "Solid")]),
ParamSpec::new("ignoreflat", ParamValue::B(false)).label("Ignore Flat Edges"),
ParamSpec::new("flatangle", ParamValue::F(1.0)).label("Flat Angle").range(0.0, 180.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(PolyBevel { specs: spec() })
}
struct PolyBevel {
    specs: Vec<ParamSpec>,
}

fn at<T: Copy>(v: &[T], i: usize, def: T) -> T {
    v.get(i).copied().unwrap_or(def)
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(a: V3) -> Option<V3> {
    let l = dot(a, a).sqrt();
    (l.is_finite() && l > EPS).then(|| scale(a, 1.0 / l))
}

/// A valid closed poly: source prim, first vertex id, corner points, unit normal.
struct Face {
    prim: usize,
    first: usize,
    pts: Vec<usize>,
    nrm: V3,
}
/// A beveled edge `a -> b`: `f1` = (face, corner) of the face traversing `a -> b` (corner at `a`), `f2` = the face
/// traversing `b -> a` (corner at `b`).
struct BEdge {
    a: usize,
    b: usize,
    f1: (usize, usize),
    f2: (usize, usize),
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    let mut root = x;
    while let Some(&p) = parent.get(root) {
        if p == root {
            break;
        }
        root = p;
    }
    while let Some(&p) = parent.get(x) {
        if p == root {
            break;
        }
        if let Some(s) = parent.get_mut(x) {
            *s = root;
        }
        x = p;
    }
    root
}
fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb
        && let Some(s) = parent.get_mut(rb)
    {
        *s = ra;
    }
}

/// Which edges are selected.
struct EdgeSel {
    all: bool,
    edges: HashSet<(usize, usize)>,
    points: Vec<bool>,
}
impl EdgeSel {
    fn parse(src: &Detail, names: &str) -> Result<EdgeSel, CookError> {
        let toks: Vec<&str> = names.split_whitespace().collect();
        let mut sel =
            EdgeSel { all: toks.is_empty(), edges: HashSet::new(), points: vec![false; src.point_count as usize] };
        for t in toks {
            let pair = t.split_once('-').and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)));
            if let Some((a, b)) = pair {
                sel.edges.insert((a.min(b), a.max(b)));
                continue;
            }
            let g = src.group(AttrClass::Point, t).ok_or_else(|| failed(OP, &format!("unknown point group `{t}`")))?;
            for (s, m) in sel.points.iter_mut().zip(&g.mask) {
                *s |= *m;
            }
        }
        Ok(sel)
    }
    fn has(&self, a: usize, b: usize) -> bool {
        self.all
            || self.edges.contains(&(a.min(b), a.max(b)))
            || (at(&self.points, a, false) && at(&self.points, b, false))
    }
}

/// Where the corner `i` of `f` lands, relative to its vertex (see module docs).
fn corner_shift(pos: &[V3], f: &Face, i: usize, prev_b: bool, next_b: bool, d: f64) -> V3 {
    let m = f.pts.len();
    let p = |k: usize| at(pos, at(&f.pts, k % m, 0), ZERO);
    let (pp, v, q) = (p(i + m - 1), p(i), p(i + 1));
    let (Some(e1), Some(e2)) = (unit(sub(v, pp)), unit(sub(q, v))) else { return ZERO };
    let (Some(in1), Some(in2)) = (unit(cross(e1, f.nrm)), unit(cross(e2, f.nrm))) else { return ZERO };
    match (prev_b, next_b) {
        (false, false) => ZERO,
        (true, true) => {
            let c = dot(in1, in2);
            if 1.0 + c < 1e-6 { scale(in1, d) } else { scale(add(in1, in2), d / (1.0 + c)) }
        }
        (true, false) => {
            let k = dot(e2, in1);
            if k.abs() < 1e-6 { scale(in1, d) } else { scale(e2, d / k) }
        }
        (false, true) => {
            let back = scale(e1, -1.0);
            let k = dot(back, in2);
            if k.abs() < 1e-6 { scale(in2, d) } else { scale(back, d / k) }
        }
    }
}

/// Interior profile points `k = 1..div-1` between tangent points `t1`, `t2` around edge point `x` (edge direction `th`).
fn profile(x: V3, th: V3, t1: V3, t2: V3, div: usize, round: bool) -> Vec<V3> {
    let along = (dot(sub(t1, x), th) + dot(sub(t2, x), th)) / 2.0;
    let apex = add(x, scale(th, along));
    let w = match (unit(sub(t1, apex)), unit(sub(t2, apex))) {
        (Some(u1), Some(u2)) => ((1.0 - dot(u1, u2)) / 2.0).max(0.0).sqrt(),
        _ => 1.0,
    };
    (1..div)
        .map(|k| {
            let t = k as f64 / div as f64;
            if !round {
                return add(scale(t1, 1.0 - t), scale(t2, t));
            }
            let (b0, b1, b2) = ((1.0 - t) * (1.0 - t), 2.0 * w * t * (1.0 - t), t * t);
            scale(add(add(scale(t1, b0), scale(apex, b1)), scale(t2, b2)), 1.0 / (b0 + b1 + b2))
        })
        .collect()
}

/// Polygon corners with consecutive (cyclic) duplicate points removed; `None` below 3 corners.
fn dedup(poly: Vec<(usize, Option<usize>)>) -> Option<Vec<(usize, Option<usize>)>> {
    let mut out: Vec<(usize, Option<usize>)> = Vec::with_capacity(poly.len());
    for c in poly {
        if out.last().is_none_or(|l| l.0 != c.0) {
            out.push(c);
        }
    }
    while out.len() > 1 && out.first().map(|f| f.0) == out.last().map(|l| l.0) {
        out.pop();
    }
    (out.len() >= 3).then_some(out)
}

/// One bevel pass; `None` when no edge is bevelable.
fn bevel(src: &Detail, names: &str, d: f64, div: usize, round: bool, flat: Option<f64>) -> Result<Option<Detail>, CookError> {
    let pos = positions(src);
    let np = pos.len();
    let sel = EdgeSel::parse(src, names)?;
    // ---- valid faces, corner ids ----
    let mut faces: Vec<Face> = Vec::new();
    let mut face_of_prim: Vec<Option<usize>> = vec![None; src.prims.len()];
    for (pi, pr) in src.prims.iter().enumerate() {
        let (Some(pts), Ok(first)) = (closed_poly_pts(src, pr), usize::try_from(pr.verts.start)) else { continue };
        let distinct: HashSet<usize> = pts.iter().copied().collect();
        let Some(nrm) = prim_normal(&pos, &pts).filter(|_| distinct.len() == pts.len()) else { continue };
        if let Some(s) = face_of_prim.get_mut(pi) {
            *s = Some(faces.len());
        }
        faces.push(Face { prim: pi, first, pts, nrm });
    }
    let mut off: Vec<usize> = Vec::with_capacity(faces.len());
    let mut ncorners = 0usize;
    for f in &faces {
        off.push(ncorners);
        ncorners += f.pts.len();
    }
    // ---- directed edges, bevelable + selected ----
    let mut dmap: HashMap<(usize, usize), Vec<(usize, usize)>> = HashMap::new();
    for (fi, f) in faces.iter().enumerate() {
        for (i, &a) in f.pts.iter().enumerate() {
            dmap.entry((a, at(&f.pts, (i + 1) % f.pts.len(), a))).or_default().push((fi, i));
        }
    }
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut bedges: Vec<BEdge> = Vec::new();
    for (fi, f) in faces.iter().enumerate() {
        for (i, &a) in f.pts.iter().enumerate() {
            let b = at(&f.pts, (i + 1) % f.pts.len(), a);
            if !seen.insert((a.min(b), a.max(b))) || !sel.has(a, b) {
                continue;
            }
            if let (Some([_]), Some(&[f2])) =
                (dmap.get(&(a, b)).map(Vec::as_slice), dmap.get(&(b, a)).map(Vec::as_slice))
            {
                // `f2` is the lone opposite use: its corner is at `b`
if let Some(lim) = flat {
let n2 = faces.get(f2.0).map_or(ZERO, |g| g.nrm);
let ang = dot(f.nrm, n2).clamp(-1.0, 1.0).acos().to_degrees();
if ang <= lim {
continue;
}
}
                bedges.push(BEdge { a, b, f1: (fi, i), f2 });
            }
        }
    }
    if bedges.is_empty() {
        return Ok(None);
    }
    let beveled: HashSet<(usize, usize)> = bedges.iter().map(|e| (e.a.min(e.b), e.a.max(e.b))).collect();
    let is_b = |a: usize, b: usize| beveled.contains(&(a.min(b), a.max(b)));
    // ---- size cap (u64, before any allocation) ----
    let nb = bedges.len() as u64;
    let dv = div as u64;
    let out_pts = (np as u64).saturating_add(ncorners as u64).saturating_add(nb.saturating_mul(2 * (dv - 1)));
    let out_verts = u64::from(src.vertex_count()).saturating_add(nb.saturating_mul(6 * dv));
    let out_prims = (src.prims.len() as u64).saturating_add(nb.saturating_mul(dv + 2));
    if out_pts > MAX_ELEMS || out_verts > MAX_ELEMS || out_prims > MAX_ELEMS {
        return Err(failed(OP, "output element count exceeds the allocation cap"));
    }
    // ---- sectors: corners joined across unbeveled edges, whole vertices without a bevel ----
    let mut parent: Vec<usize> = (0..ncorners).collect();
    let mut by_vertex: Vec<Vec<usize>> = vec![Vec::new(); np];
    let mut by_edge: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    let mut bev_vertex = vec![false; np];
    for e in &bedges {
        for v in [e.a, e.b] {
            if let Some(s) = bev_vertex.get_mut(v) {
                *s = true;
            }
        }
    }
    for (fi, f) in faces.iter().enumerate() {
        let m = f.pts.len();
        for (i, &v) in f.pts.iter().enumerate() {
            let c = at(&off, fi, 0) + i;
            if let Some(l) = by_vertex.get_mut(v) {
                l.push(c);
            }
            for o in [at(&f.pts, (i + 1) % m, v), at(&f.pts, (i + m - 1) % m, v)] {
                by_edge.entry((v, o)).or_default().push(c);
            }
        }
    }
    for (&(v, o), cs) in &by_edge {
        if !is_b(v, o) {
            for w in cs.windows(2) {
                union(&mut parent, at(w, 0, 0), at(w, 1, 0));
            }
        }
    }
    for (v, cs) in by_vertex.iter().enumerate() {
        if !bev_vertex.get(v).copied().unwrap_or(false) {
            for w in cs.windows(2) {
                union(&mut parent, at(w, 0, 0), at(w, 1, 0));
            }
        }
    }
    // ---- output points: one per sector, one per face-less vertex ----
    let mut corner_pt: Vec<usize> = vec![0; ncorners];
    let mut first_pt: Vec<usize> = vec![0; np];
    let mut root_pt: HashMap<usize, usize> = HashMap::new();
    let mut point_src: Vec<Vec<usize>> = Vec::new();
    for (v, cs) in by_vertex.iter().enumerate() {
        if cs.is_empty() {
            if let Some(s) = first_pt.get_mut(v) {
                *s = point_src.len();
            }
            point_src.push(vec![v]);
        }
        for (k, &c) in cs.iter().enumerate() {
            let r = find(&mut parent, c);
            let id = *root_pt.entry(r).or_insert_with(|| {
                point_src.push(vec![v]);
                point_src.len() - 1
            });
            if k == 0
                && let Some(s) = first_pt.get_mut(v)
            {
                *s = id;
            }
            if let Some(s) = corner_pt.get_mut(c) {
                *s = id;
            }
        }
    }
    let mut acc: Vec<V3> = vec![ZERO; point_src.len()];
    let mut cnt: Vec<f64> = vec![0.0; point_src.len()];
    for (fi, f) in faces.iter().enumerate() {
        let m = f.pts.len();
        for (i, &v) in f.pts.iter().enumerate() {
            let (pv, nv) = (at(&f.pts, (i + m - 1) % m, v), at(&f.pts, (i + 1) % m, v));
            let x = corner_shift(&pos, f, i, is_b(pv, v), is_b(v, nv), d);
            let pt = at(&corner_pt, at(&off, fi, 0) + i, 0);
            if let (Some(a), Some(c)) = (acc.get_mut(pt), cnt.get_mut(pt)) {
                *a = add(*a, add(at(&pos, v, ZERO), x));
                *c += 1.0;
            }
        }
    }
    let mut pt_pos: Vec<V3> = acc
        .iter()
        .zip(&cnt)
        .zip(&point_src)
        .map(|((a, &c), s)| if c > 0.0 { scale(*a, 1.0 / c) } else { at(&pos, at(s, 0, 0), ZERO) })
        .collect();
    // ---- strips: profile points, paths for caps ----
    struct Strip {
        a: Vec<usize>,
        b: Vec<usize>,
        v1: (Option<usize>, Option<usize>, Option<usize>, Option<usize>),
        prim: usize,
    }
    let mut strips: Vec<Strip> = Vec::new();
    let mut paths: BTreeMap<usize, Vec<Vec<usize>>> = BTreeMap::new();
    for e in &bedges {
        let (Some(fa), Some(fb)) = (faces.get(e.f1.0), faces.get(e.f2.0)) else { continue };
        let (ma, mb) = (fa.pts.len(), fb.pts.len());
        let corner = |fi: usize, i: usize| at(&corner_pt, at(&off, fi, 0) + i, 0);
        let s1a = corner(e.f1.0, e.f1.1);
        let s1b = corner(e.f1.0, (e.f1.1 + 1) % ma);
        let s2b = corner(e.f2.0, e.f2.1);
        let s2a = corner(e.f2.0, (e.f2.1 + 1) % mb);
        let (pa, pb) = (at(&pos, e.a, ZERO), at(&pos, e.b, ZERO));
        let th = unit(sub(pb, pa)).unwrap_or(ZERO);
        let mut ends: Vec<Vec<usize>> = Vec::with_capacity(2);
        for (x, v, s1, s2) in [(pa, e.a, s1a, s2a), (pb, e.b, s1b, s2b)] {
            let mut ids = vec![s1];
            if s1 == s2 {
                ids.extend(std::iter::repeat_n(s1, div));
            } else {
                let mids = profile(x, th, at(&pt_pos, s1, ZERO), at(&pt_pos, s2, ZERO), div, round);
                for m in mids {
                    ids.push(pt_pos.len());
                    pt_pos.push(m);
                    point_src.push(vec![v]);
                }
                ids.push(s2);
            }
            ends.push(ids);
        }
        let (Some(a_ids), Some(b_ids)) = (ends.first().cloned(), ends.get(1).cloned()) else { continue };
        paths.entry(e.a).or_default().push(a_ids.iter().rev().copied().collect());
        paths.entry(e.b).or_default().push(b_ids.clone());
        let vi = |fa: &Face, i: usize| Some(fa.first + i % fa.pts.len());
        strips.push(Strip {
            a: a_ids,
            b: b_ids,
            v1: (vi(fa, e.f1.1), vi(fa, e.f1.1 + 1), vi(fb, e.f2.1 + 1), vi(fb, e.f2.1)),
            prim: fa.prim,
        });
    }
    // ---- assemble topology ----
    let mut vertex_point: Vec<u32> = Vec::new();
    let mut vert_src: Vec<Option<usize>> = Vec::new();
    let mut prims: Vec<Prim> = Vec::new();
    let mut prim_src: Vec<Option<usize>> = Vec::new();
    let pid = |p: usize| u32::try_from(p).map_err(|_| failed(OP, "point id overflow"));
    let mut emit = |poly: &[(usize, Option<usize>)], from: Option<usize>, kind: PrimKind| -> Result<(), CookError> {
        let start = u32::try_from(vertex_point.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
        for &(p, vs) in poly {
            vertex_point.push(pid(p)?);
            vert_src.push(vs);
        }
        let end = u32::try_from(vertex_point.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
        prims.push(Prim { kind, verts: start..end });
        prim_src.push(from);
        Ok(())
    };
    for (pi, pr) in src.prims.iter().enumerate() {
        if let Some(fi) = face_of_prim.get(pi).copied().flatten().and_then(|fi| faces.get(fi).map(|f| (fi, f))) {
            let poly: Vec<(usize, Option<usize>)> =
                (0..fi.1.pts.len()).map(|i| (at(&corner_pt, at(&off, fi.0, 0) + i, 0), Some(fi.1.first + i))).collect();
            emit(&poly, Some(pi), CLOSED)?;
        } else {
            let (lo, hi) = (pr.verts.start as usize, pr.verts.end as usize);
            let poly: Vec<(usize, Option<usize>)> =
                (lo..hi).map(|vx| (at(&first_pt, at(&src.vertex_point, vx, 0) as usize, 0), Some(vx))).collect();
            emit(&poly, Some(pi), pr.kind.clone())?;
        }
    }
    for s in &strips {
        for k in 0..div {
            let corner_src = |end: usize, kk: usize| -> Option<usize> {
                let (v1a, v1b, v2a, v2b) = s.v1;
                match (end, kk) {
                    (0, 0) => v1a,
                    (0, x) if x == div => v2a,
                    (1, 0) => v1b,
                    (1, x) if x == div => v2b,
                    _ => None,
                }
            };
            let poly = vec![
                (at(&s.b, k, 0), corner_src(1, k)),
                (at(&s.a, k, 0), corner_src(0, k)),
                (at(&s.a, k + 1, 0), corner_src(0, k + 1)),
                (at(&s.b, k + 1, 0), corner_src(1, k + 1)),
            ];
            if let Some(p) = dedup(poly) {
                emit(&p, Some(s.prim), CLOSED)?;
            }
        }
    }
    for ps in paths.values() {
        let mut used = vec![false; ps.len()];
        let (Some(first), Some(slot)) = (ps.first(), used.first_mut()) else { continue };
        *slot = true;
        let mut ring: Vec<usize> = first.iter().take(first.len().saturating_sub(1)).copied().collect();
        let mut tail = first.last().copied();
        let head = first.first().copied();
        let mut n_used = 1;
        while n_used < ps.len() {
            let next = ps
                .iter()
                .enumerate()
                .find(|(k, p)| !used.get(*k).copied().unwrap_or(true) && p.first().copied() == tail);
            let Some((k, p)) = next else { break };
            if let Some(s) = used.get_mut(k) {
                *s = true;
            }
            ring.extend(p.iter().take(p.len().saturating_sub(1)).copied());
            tail = p.last().copied();
            n_used += 1;
        }
        if n_used == ps.len()
            && tail == head
            && let Some(p) = dedup(ring.into_iter().map(|p| (p, None)).collect())
        {
            emit(&p, None, CLOSED)?;
        }
    }
    let mut out =
        rebuild(OP, src, vertex_point, prims, &Remap { point_src, vert_src, prim_src }, to_f32(OP, &pt_pos)?)?;
    renormal(&mut out);
    Ok(Some(out))
}

/// Recompute any existing V3 `N` (point / vertex / prim) from the new topology.
fn renormal(d: &mut Detail) {
    let pos = positions(d);
    let face_n: Vec<Option<V3>> =
        d.prims.iter().map(|pr| closed_poly_pts(d, pr).and_then(|pts| prim_normal(&pos, &pts))).collect();
    let pn = point_normals(d, &pos);
    let vn: Vec<V3> = {
        let mut v = vec![ZERO; d.vertex_count() as usize];
        for (pr, n) in d.prims.iter().zip(&face_n) {
            for x in pr.verts.start..pr.verts.end {
                if let Some(s) = v.get_mut(x as usize) {
                    *s = n.unwrap_or(ZERO);
                }
            }
        }
        v
    };
    let f32v = |v: V3| [v[0] as f32, v[1] as f32, v[2] as f32];
    for a in &mut d.attrs {
        if a.name != "N" {
            continue;
        }
        if let AttrData::V3(cur) = &mut a.data {
            let new: Vec<[f32; 3]> = match a.class {
                AttrClass::Point => pn.iter().copied().map(f32v).collect(),
                AttrClass::Vertex => vn.iter().copied().map(f32v).collect(),
                AttrClass::Prim => face_n.iter().map(|n| f32v(n.unwrap_or(ZERO))).collect(),
                AttrClass::Detail => continue,
            };
            if new.len() == cur.len() {
                *cur = new;
            }
        }
    }
}

impl Node<Detail> for PolyBevel {
    fn type_name(&self) -> &'static str {
        "polybevel"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let offset = float(p, "offset")?;
        if offset < 0.0 {
            return Err(bad(format!("`offset` must be >= 0, got {offset}")));
        }
        let div = int(p, "divisions")?;
        if !(1..=MAX_DIVISIONS).contains(&div) {
            return Err(bad(format!("`divisions` must be in 1..={MAX_DIVISIONS}, got {div}")));
        }
        let round = match p.menu("filletshape") {
            Some(SHAPE_ROUND) => true,
            Some(SHAPE_SOLID) => false,
            Some(m) => return Err(bad(format!("`filletshape` menu {m} out of range"))),
            None => return Err(bad("`filletshape`: missing or not a menu")),
        };
        let names = p.string("group").unwrap_or("");
let flat = if p.toggle("ignoreflat").unwrap_or(false) {
let a = float(p, "flatangle")?;
if !a.is_finite() || !(0.0..=180.0).contains(&a) {
return Err(bad(format!("`flatangle` must be in 0..=180, got {a}")));
}
Some(a)
} else {
None
};
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if offset == 0.0 {
            return Ok(Arc::clone(src));
        }
        match bevel(src, names, offset, div as usize, round, flat)? {
            Some(d) => Ok(Arc::new(d)),
            None => Ok(Arc::clone(src)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, grid3, prim_pts};
    use super::*;
    use hexe_geo::{Attribute, Group, TypeInfo};
    use hexe_graph::ParamSet;

    fn with(offset: f32, div: i32, shape: u32, group: &str) -> ParamSet {
        defaults(&*new())
            .with("offset", ParamValue::F(offset))
            .with("divisions", ParamValue::I(div))
            .with("filletshape", ParamValue::Menu(shape))
            .with("group", ParamValue::S(group.into()))
    }
    fn count(d: &Detail, n: usize) -> usize {
        d.prims.iter().filter(|p| p.verts.len() == n).count()
    }
    /// Closed 2-manifold: every directed edge exactly once, its reverse exactly once; Euler characteristic = 2.
    fn assert_watertight_sphere(d: &Detail) {
        let mut dir: HashMap<(u32, u32), u32> = HashMap::new();
        for pr in &d.prims {
            let pts: Vec<u32> = d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].to_vec();
            for (i, &a) in pts.iter().enumerate() {
                *dir.entry((a, pts[(i + 1) % pts.len()])).or_default() += 1;
            }
        }
        for (&(a, b), &n) in &dir {
            assert_eq!(n, 1, "directed edge {a}->{b} used {n}x");
            assert_eq!(dir.get(&(b, a)), Some(&1), "edge {a}->{b} has no unique reverse");
        }
        let (v, e, f) = (d.point_count as i64, dir.len() as i64 / 2, d.prim_count() as i64);
        assert_eq!(v - e + f, 2, "V={v} E={e} F={f}");
    }
    /// Every prim wound outward of the origin-centred, convex result (right-handed Newell normal . centroid < 0: Houdini's clockwise-from-outside winding).
    fn assert_outward(d: &Detail) {
        for i in 0..d.prims.len() {
            let p = prim_pts(d, i);
            let (mut n, mut c) = ([0.0f64; 3], [0.0f64; 3]);
            for (k, a) in p.iter().enumerate() {
                let b = p[(k + 1) % p.len()];
                let (a, b) = (a.map(f64::from), b.map(f64::from));
                n = [
                    n[0] + a[1] * b[2] - a[2] * b[1],
                    n[1] + a[2] * b[0] - a[0] * b[2],
                    n[2] + a[0] * b[1] - a[1] * b[0],
                ];
                c = [c[0] + a[0], c[1] + a[1], c[2] + a[2]];
            }
            assert!(n[0] * c[0] + n[1] * c[1] + n[2] * c[2] < 0.0, "prim {i} wound inward: {p:?}");
        }
    }
    fn near3(a: [f32; 3], b: [f64; 3]) -> bool {
        (0..3).all(|k| (f64::from(a[k]) - b[k]).abs() < 1e-6)
    }
    fn has_pt(d: &Detail, b: [f64; 3]) -> bool {
        d.p().iter().any(|&p| near3(p, b))
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "polybevel");
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("group", ParamValue::S(String::new())),
                ("offset", ParamValue::F(0.1)),
                ("divisions", ParamValue::I(1)),
                ("filletshape", ParamValue::Menu(0)),
                ("ignoreflat", ParamValue::B(false)),
                ("flatangle", ParamValue::F(1.0)),
            ]
        );
    }
    /// Cube (8 pts / 6 quads), all 12 edges, chamfer: each vertex -> 3 face-corner points = 24 pts; 6 shrunk quads +
    /// 12 edge quads + 8 corner tris = 26 prims. Euler: 24 - 48 + 26 = 2.
    #[test]
    fn cube_all_edges_chamfer_is_24_points_6_quads_12_edge_quads_8_corner_tris() {
        let out = cook(&*new(), cube(), &with(0.1, 1, 0, "")).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (24, 26));
        assert_eq!((count(&out, 4), count(&out, 3)), (18, 8));
        assert_watertight_sphere(&out);
        assert_outward(&out);
        // every point sits on one face (|c| = 0.5) and 0.1 in from both neighbouring faces (|c| = 0.4)
        for p in out.p() {
            let mut a = p.map(|c| (c.abs() * 10.0).round() / 10.0);
            a.sort_by(f32::total_cmp);
            assert_eq!(a, [0.4, 0.4, 0.5], "{p:?}");
            assert!(p.iter().all(|c| (c.abs() - 0.4).abs() < 1e-6 || (c.abs() - 0.5).abs() < 1e-6), "{p:?}");
        }
    }
    /// Edge (0,1) of the cube runs (-.5,-.5,-.5) -> (-.5,-.5,.5). Its strip quad is bounded by the four points
    /// (-.5,-.4,+-.4) on face x = -.5 and (-.4,-.5,+-.4) on face y = -.5 (offset .1 measured in each face plane).
    #[test]
    fn chamfer_strip_of_one_edge_is_at_exact_offset_positions() {
        let out = cook(&*new(), cube(), &with(0.1, 1, 0, "")).expect("cooks");
        let want = [[-0.5, -0.4, -0.4], [-0.4, -0.5, -0.4], [-0.5, -0.4, 0.4], [-0.4, -0.5, 0.4]];
        let strip = (0..out.prims.len()).find(|&i| {
            let p = prim_pts(&out, i);
            p.len() == 4 && want.iter().all(|w| p.iter().any(|&q| near3(q, *w)))
        });
        assert!(strip.is_some(), "no strip quad over the four expected points");
        // chamfer width = sqrt(2) * offset
        let (a, b) = (want[0], want[1]);
        let w = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        assert!((w - 0.1 * 2f64.sqrt()).abs() < 1e-12);
        // the shrunk face x = -.5 has corners (-.5, +-.4, +-.4)
        let f0 = prim_pts(&out, 0);
        for w in [[-0.5, -0.4, -0.4], [-0.5, -0.4, 0.4], [-0.5, 0.4, 0.4], [-0.5, 0.4, -0.4]] {
            assert!(f0.iter().any(|&q| near3(q, w)), "{f0:?} lacks {w:?}");
        }
    }
    /// divisions = 2: +1 profile point per edge end = 24 more (48), +12 strip quads (24), 8 hexagon caps (3 sectors x 2).
    /// Euler: V=48, E=(6*4 + 24*4 + 8*6)/2 = 84, F=38: 48 - 84 + 38 = 2.
    #[test]
    fn cube_divisions_2_is_48_points_38_prims_with_hexagon_caps() {
        let out = cook(&*new(), cube(), &with(0.1, 2, 0, "")).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (48, 38));
        assert_eq!((count(&out, 4), count(&out, 6), count(&out, 3)), (30, 8, 0));
        assert_watertight_sphere(&out);
        assert_outward(&out);
    }
    /// Round profile of edge end (-.5,-.5,-.5): tangent points T1=(-.5,-.4,-.4), T2=(-.4,-.5,-.4), apex (-.5,-.5,-.4), 90 deg
    /// -> conic weight sqrt(1/2), t = 1/2: apex + (1 - 1/sqrt2)/... = apex + 0.29289.. * (T1 + T2 - 2 apex) = (-.4707107, -.4707107, -.4):
    /// exactly on the circle of radius .1 about (-.4,-.4,-.4). `solid` = straight midpoint (-.45, -.45, -.4).
    #[test]
    fn divisions_2_round_profile_point_is_on_the_circle_solid_is_the_chord_midpoint() {
        let r = cook(&*new(), cube(), &with(0.1, 2, SHAPE_ROUND, "")).expect("cooks");
        let s = 0.1 * (1.0 - 1.0 / 2f64.sqrt());
        let mid = [-0.5 + s, -0.4, -0.5 + s]; // first profile edge = Houdini-wound cube's first edge
        assert!(near3(r.p()[24], mid), "{:?}", r.p()[24]);
        // all 24 profile points: 0.1 from their cross-section circle centre (+-.4 on all three axes)
        for p in &r.p()[24..] {
            let c = p.map(|x| if x < 0.0 { -0.4 } else { 0.4 });
            // the along-edge axis has |coord| = .4 exactly; the other two lie on the circle about the centre
            let off: Vec<f64> = (0..3).map(|k| f64::from(p[k]) - c[k]).collect();
            let rad = off.iter().map(|x| x * x).sum::<f64>().sqrt();
            assert!((rad - 0.1).abs() < 1e-6, "{p:?} radius {rad}");
        }
        let l = cook(&*new(), cube(), &with(0.1, 2, SHAPE_SOLID, "")).expect("cooks");
        assert!(near3(l.p()[24], [-0.45, -0.4, -0.45]), "{:?}", l.p()[24]);
        assert_eq!((l.point_count, l.prim_count()), (48, 38));
        assert!(has_pt(&r, mid) && !has_pt(&l, mid));
    }
    /// Top face loop (4 edges): the 4 top vertices each split in 2 sectors (top face | two sides) = 8 + 4 = 12 pts;
    /// 6 faces + 4 strips = 10 prims, no caps (2 sectors). Euler: 12 - 20 + 10 = 2. Side corners slide down 0.1.
    #[test]
    fn edge_group_top_loop_bevels_only_those_edges() {
        let out = cook(&*new(), cube(), &with(0.1, 1, 0, "2-3 3-7 7-6 6-2")).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (12, 10));
        assert_eq!((count(&out, 4), count(&out, 3)), (10, 0));
        assert_watertight_sphere(&out);
        // top vertex 3 = (-.5,.5,.5): top-face corner insets in x,z -> (-.4,.5,.4); side-face corner slides down -> (-.5,.4,.5)
        assert!(has_pt(&out, [-0.4, 0.5, 0.4]));
        assert!(has_pt(&out, [-0.5, 0.4, 0.5]));
        assert!(!has_pt(&out, [-0.5, 0.5, 0.5]));
        // untouched bottom vertices keep their position
        assert!(has_pt(&out, [-0.5, -0.5, -0.5]) && has_pt(&out, [0.5, -0.5, 0.5]));
    }
    #[test]
    fn point_group_selects_edges_with_both_ends_in_group() {
        let mut d = cube();
        d.groups.push(Group {
            name: "top".into(),
            class: AttrClass::Point,
            mask: (0..8).map(|i| (i >> 1) & 1 == 1).collect(),
        });
        let out = cook(&*new(), d, &with(0.1, 1, 0, "top")).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (12, 10));
        let bad = with(0.1, 1, 0, "nope");
        assert!(matches!(cook(&*new(), cube(), &bad), Err(CookError::Failed(_))));
    }
    /// A lone beveled edge: both end vertices keep ONE sector (documented taper) -> strip degenerates, nothing breaks.
    #[test]
    fn lone_edge_tapers_without_new_topology() {
        let out = cook(&*new(), cube(), &with(0.1, 1, 0, "0-1")).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (8, 6));
        assert_outward(&out);
    }
    /// Open 2x2 grid: the 4 interior spokes are bevelable. Corners (4) keep 1 pt; edge-midpoints (4) split in 2; centre
    /// splits in 4 = 16 pts; 4 faces + 4 strips + 1 square cap at the centre = 9 prims.
    #[test]
    fn open_grid_bevels_interior_edges_only() {
        let out = cook(&*new(), grid3(), &with(0.1, 1, 0, "")).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (16, 9));
        assert_eq!((count(&out, 4), count(&out, 3)), (9, 0));
        assert!(out.p().iter().all(|p| p[2] == 0.0));
        // face 0 (grid corner (0,0)) loses 0.1 along both spokes: its inner corner (1,1) -> (0.9, 0.9)
        assert!(has_pt(&out, [0.9, 0.9, 0.0]));
        assert!(has_pt(&out, [0.0, 0.0, 0.0]));
    }
    #[test]
    fn point_attrs_inherit_normals_recompute_groups_and_prim_attrs_follow() {
        let mut d = cube();
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: AttrData::F32((0..8).map(|i| i as f32).collect()),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[9.0, 9.0, 9.0]; 8]),
            type_info: TypeInfo::Normal,
        });
        d.add_attr(Attribute {
            name: "id".into(),
            class: AttrClass::Prim,
            data: AttrData::I32((0..6).collect()),
            type_info: TypeInfo::None,
        });
        d.groups.push(Group {
            name: "g".into(),
            class: AttrClass::Prim,
            mask: vec![true, false, false, false, false, false],
        });
        let out = cook(&*new(), d, &with(0.1, 2, 0, "")).expect("cooks");
        let Some(AttrData::F32(w)) = out.attr(AttrClass::Point, "w").map(|a| &a.data) else { panic!("w lost") };
        assert_eq!(w.len(), 48);
        // each point carries the weight of the source vertex it grew from: positions coincide with its 3x3 neighbourhood
        for (i, p) in out.p().iter().enumerate() {
            let src = (0..8).min_by(|&a, &b| {
                let dist = |k: usize| {
                    let c = [(k >> 2) & 1, (k >> 1) & 1, k & 1].map(|x| if x == 1 { 0.5f32 } else { -0.5 });
                    (0..3).map(|j| (c[j] - p[j]).powi(2)).sum::<f32>()
                };
                dist(a).total_cmp(&dist(b))
            });
            assert_eq!(w[i], src.unwrap_or(99) as f32, "point {i} {p:?}");
        }
        let Some(AttrData::V3(n)) = out.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N lost") };
        assert_eq!(n.len(), 48);
        for (nn, p) in n.iter().zip(out.p()) {
            let l = nn.iter().map(|c| c * c).sum::<f32>().sqrt();
            assert!((l - 1.0).abs() < 1e-5, "N not unit: {nn:?}");
            // convex origin-centred: normal points away from the origin
            assert!(nn.iter().zip(p).map(|(a, b)| a * b).sum::<f32>() > 0.0);
        }
        let Some(AttrData::I32(id)) = out.attr(AttrClass::Prim, "id").map(|a| &a.data) else { panic!("id lost") };
        assert_eq!(id.len(), 38);
        assert_eq!(id[0], 0);
        assert_eq!(out.group(AttrClass::Prim, "g").map(|g| g.mask.len()), Some(38));
        // face 0 keeps membership; the strips of its edges (first face = face 0 for edges 0-1, 0-2 ...) inherit it
        assert!(
            out.group(AttrClass::Prim, "g").is_some_and(|g| g.mask[0] && g.mask.iter().filter(|m| **m).count() > 1)
        );
    }
    #[test]
    fn zero_offset_no_bevelable_edge_and_empty_input_share_or_pass() {
        let a = Arc::new(cube());
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &with(0.0, 1, 0, "")).expect("cooks"), &a));
        let mut line = Detail::default();
        line.add_points(2);
        line.add_poly(&[0, 1], false);
        let l = Arc::new(line);
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &l, &with(0.1, 1, 0, "")).expect("cooks"), &l));
        assert_eq!(cook(&*new(), Detail::default(), &with(0.1, 1, 0, "")).expect("cooks").point_count, 0);
    }
    #[test]
    fn hostile_params_degenerate_prims_and_caps() {
        for (o, d, s) in
            [(-0.1, 1, 0), (0.1, 0, 0), (0.1, 65, 0), (0.1, -3, 0), (0.1, i32::MAX, 0), (0.1, 1, 2), (0.1, 1, 99)]
        {
            assert!(matches!(cook(&*new(), cube(), &with(o, d, s, "")), Err(CookError::BadParam(_))), "{o} {d} {s}");
        }
        // degenerate / open / out-of-range prims pass through; huge offset does not panic
        let mut d = cube();
        d.add_poly(&[0, 0, 0, 0], true);
        d.add_poly(&[0, 1, 2], false);
        let out = cook(&*new(), d.clone(), &with(0.1, 1, 0, "")).expect("cooks");
        assert_eq!(out.prim_count(), 26 + 2);
        assert!(cook(&*new(), d, &with(1.0e6, 3, 0, "")).is_ok());
        // dense grid at max divisions exceeds MAX_ELEMS up front instead of allocating
        let mut g = Detail::default();
        let n = 400u32;
        g.add_points(n * n);
        *g.p_mut() = (0..n * n).map(|i| [(i % n) as f32, (i / n) as f32, 0.0]).collect();
        for r in 0..n - 1 {
            for c in 0..n - 1 {
                let a = r * n + c;
                g.add_poly(&[a, a + 1, a + n + 1, a + n], true);
            }
        }
        assert!(matches!(cook(&*new(), g, &with(0.01, 64, 0, "")), Err(CookError::Failed(_))));
    }
    /// Cube dihedral = 90 deg: `ignoreflat` with `flatangle` 89 keeps all 12 edges (24/26); 90 ignores them all (8/6 passthrough).
    #[test]
    fn ignoreflat_angle_limit_keeps_90_degree_edges_below_90_and_drops_them_at_90() {
        let on = |a: f32| with(0.1, 1, 0, "").with("ignoreflat", ParamValue::B(true)).with("flatangle", ParamValue::F(a));
        let kept = cook(&*new(), cube(), &on(89.0)).expect("cooks");
        assert_eq!((kept.point_count, kept.prim_count()), (24, 26));
        let dropped = cook(&*new(), cube(), &on(90.0)).expect("cooks");
        assert_eq!((dropped.point_count, dropped.prim_count()), (8, 6));
        // off (default): flatangle ignored
        let off = with(0.1, 1, 0, "").with("flatangle", ParamValue::F(179.0));
        assert_eq!(cook(&*new(), cube(), &off).expect("cooks").prim_count(), 26);
    }
    #[test]
    fn ignoreflat_bad_angle_is_rejected_and_only_checked_when_on() {
        let bad_a = with(0.1, 1, 0, "").with("ignoreflat", ParamValue::B(true)).with("flatangle", ParamValue::F(-1.0));
        assert!(matches!(cook(&*new(), cube(), &bad_a), Err(CookError::BadParam(_))));
        let off = with(0.1, 1, 0, "").with("flatangle", ParamValue::F(-1.0));
        assert!(cook(&*new(), cube(), &off).is_ok());
    }
    #[test]
    fn scene_loader_cooks_example_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/polybevel.json"))
            .expect("example present");
        let (mut g, id) = crate::load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert_eq!((out.point_count, out.prim_count()), (48, 38));
        assert_eq!(crate::registry().create("polybevel").map(|n| n.type_name()), Some("polybevel"));
    }
}
