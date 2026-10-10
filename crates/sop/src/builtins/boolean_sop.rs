//! Boolean SOP (token `boolean`): union / subtract (A-B) / intersect of two closed polygon meshes.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; closed-form oracle only)
//! * Inputs `input0` = A, `input1` = B. Only CLOSED polys with >= 3 corners take part (open / curve / other prims ignored).
//!   Concave polys are ear-clipped, convex ones kept as n-gons. Non-finite `P` -> `Failed`.
//! * Algorithm: BSP of each operand (iterative, no recursion; plane = first polygon of the node). Every polygon of A is split
//!   by B's BSP into fragments tagged `Out` (outside B) / `In` (inside B) / `CoSame` / `CoOpp` (lying on a B face with the
//!   same / opposite normal); B symmetrically against A. Coplanar parts are decided by clipping against the node's own faces,
//!   so they are exact (no ordering artefacts).
//! * Solid mode (`treat_as_solid` on, default): union = A.Out + B.Out + A.CoSame; subtract = A.Out + A.CoOpp + flipped B.In;
//!   intersect = A.In + B.In + A.CoSame. Face-to-face contact (CoOpp / B.CoSame) is removed where it is interior.
//!   Off (surface mode, no inside/outside bookkeeping): union = every fragment of A and B (coplanar B dropped); subtract =
//!   A.Out + A.CoOpp (A trimmed, B contributes nothing); intersect = A.In + A.CoSame (A trimmed to B).
//! * Cleanup: fragment vertices are welded (1e-5 grid), T-vertices are inserted along edges, fragments of the same source
//!   face and tag are re-fused when their union is ONE simple loop (so an L-shaped remainder is one hexagon, not 3 quads), then
//!   vertices collinear in every polygon that uses them are removed. A face with a hole stays as its un-fused fragments.
//!   Fragments with different tags (e.g. the coplanar overlap and the rest of the same face) are NOT fused.
//! * QA: `operation` also 3 = B - A (swapped operands, group names swapped back) and 4 = Shatter (A cut by B, all pieces kept, B dropped;
//!   prim groups `inside` = A.In|CoSame, `outside`). Attribute transfer: prim attrs follow the source prim (other operand = default),
//!   point attrs follow the operand point at the same position (fresh intersection points = default; no interpolation); a point group
//!   `seampts` = points used by prims of >= 2 (operand, inside/outside) classes. NOT done: vertex attrs, group transfer, resolve modes.
//! * Output: `P` only; fresh points (only those used). Prim groups: `a_only` (from A, away from the cut), `b_only` (from B),
//!   `seam` (from a source face that crosses or lies on the other operand's surface). Disjoint, cover every prim.
//!   Other attributes / groups of the inputs are NOT carried.
//! * Empty / no-closed-poly operand: union -> the other input (shared `Arc`), subtract -> A, intersect -> empty.
//! * NURBS / Bezier prims: tessellated first (8 samples per span, `Detail::tessellate`); surfaces then take part as polygons (a closed
//!   tessellated surface is a solid), curves tessellate to polylines and are ignored like any open prim. No spline prim survives.
//! * Hostile: `operation` > 4 -> `BadParam`; > 16384 polys per side or > 2^20 fragments -> `Failed`.
use super::meshutil::{blend_weighted, cross, gather, newell_corners};
use crate::SopNode;
use hexe_geo::{AttrClass, Attribute, Detail, Group, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const INPUTS: [InputSpec; 2] = [InputSpec { name: "input0" }, InputSpec { name: "input1" }];
/// `operation` menu indices.
const OP_UNION: u32 = 0;
const OP_SUBTRACT: u32 = 1;
const OP_INTERSECT: u32 = 2;
/// B - A (our menu order: appended after the task order).
const OP_SUBTRACT_BA: u32 = 3;
/// Cut A by B's surface and keep every piece of A (B discarded); prim groups `inside` / `outside`.
const OP_SHATTER: u32 = 4;
/// Plane-distance tolerance.
const EPS: f64 = 1e-6;
const MAX_PRIMS: usize = 1 << 14;
const MAX_FRAGS: usize = 1 << 20;
/// Weld grid (1 / cell size).
const WELD: f64 = 1e5;
/// Product cap for the T-vertex scan.
const MAX_TSCAN: u64 = 400_000_000;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("operation", ParamValue::Menu(OP_UNION)).label("Operation").menu(&[
            ("union", "Union"),
            ("subtract", "Subtract"),
            ("intersect", "Intersect"),
            ("subtract_ba", "B Subtract A"),
            ("shatter", "Shatter"),
        ]),
        ParamSpec::new("treat_as_solid", ParamValue::B(true)).label("Treat As Solid"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Boolean { specs: spec() })
}
struct Boolean {
    specs: Vec<ParamSpec>,
}
fn bad(m: &str) -> CookError {
    CookError::BadParam(format!("boolean: {m}"))
}
fn failed(m: &str) -> CookError {
    CookError::Failed(format!("boolean: {m}"))
}

type V3 = [f64; 3];
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn lerp(a: V3, b: V3, t: f64) -> V3 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Convex planar polygon with its supporting plane `n . x = w` (n unit, outward = Houdini corner order (clockwise seen from outside)).
#[derive(Clone)]
struct Poly {
    v: Vec<V3>,
    n: V3,
    w: f64,
    src: u32,
}

fn mk_poly(v: Vec<V3>, n: V3, w: f64, src: u32) -> Option<Poly> {
    (v.len() >= 3 && len(newell_corners(&v)) > 1e-12).then_some(Poly { v, n, w, src })
}

/// Ear-clip a (possibly concave) polygon into triangles (corner order kept). `n` = the RIGHT-handed (counter-clockwise)
/// unit normal, i.e. the negated Houdini normal: the 2D projection below needs the polygon counter-clockwise.
fn ear_clip(v: &[V3], n: V3) -> Vec<Vec<V3>> {
    let k = if n[0].abs() >= n[1].abs() && n[0].abs() >= n[2].abs() {
        0
    } else if n[1].abs() >= n[2].abs() {
        1
    } else {
        2
    };
    let sg = if n[k] < 0.0 { -1.0 } else { 1.0 };
    let (a, b) = ((k + 1) % 3, (k + 2) % 3);
    let p2 = |q: V3| [q[a] * sg, q[b]];
    let cr = |o: [f64; 2], p: [f64; 2], q: [f64; 2]| (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0]);
    let mut idx: Vec<usize> = (0..v.len()).collect();
    let mut out = Vec::new();
    while idx.len() > 3 {
        let m = idx.len();
        let ear = (0..m).find(|&i| {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (pa, pb, pc) = (p2(v[ia]), p2(v[ib]), p2(v[ic]));
            if cr(pa, pb, pc) <= 1e-12 {
                return false;
            }
            !idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let q = p2(v[j]);
                cr(pa, pb, q) >= -1e-12 && cr(pb, pc, q) >= -1e-12 && cr(pc, pa, q) >= -1e-12
            })
        });
        // degenerate input without an ear: clip the first corner anyway (terminates)
        let i = ear.unwrap_or(0);
        out.push(vec![v[idx[(i + m - 1) % m]], v[idx[i]], v[idx[(i + 1) % m]]]);
        idx.remove(i);
    }
    if let [x, y, z] = idx[..] {
        out.push(vec![v[x], v[y], v[z]]);
    }
    out
}

/// Closed polys of `d` as convex `Poly`s (`src` = prim number).
fn polys_of(d: &Detail) -> Result<Vec<Poly>, CookError> {
    let pos = super::meshutil::positions(d);
    if pos.iter().any(|q| q.iter().any(|c| !c.is_finite())) {
        return Err(failed("non-finite P"));
    }
    let mut out = Vec::new();
    for (qi, pr) in d.prims.iter().enumerate() {
        let Some(ids) = super::meshutil::closed_poly_pts(d, pr) else { continue };
        let v: Vec<V3> = ids.iter().filter_map(|&i| pos.get(i).copied()).collect();
        let nw = newell_corners(&v);
        let l = len(nw);
        if l.is_nan() || l <= 1e-12 {
            continue;
        }
        let n = [nw[0] / l, nw[1] / l, nw[2] / l];
        let w = v.iter().map(|&q| dot(n, q)).sum::<f64>() / v.len() as f64;
        let src = u32::try_from(qi).map_err(|_| failed("prim id overflow"))?;
        let convex = (0..v.len()).all(|i| {
            let (a, b, c) = (v[(i + v.len() - 1) % v.len()], v[i], v[(i + 1) % v.len()]);
            dot(cross(sub(b, a), sub(c, b)), n) <= 1e-9
        });
        if convex {
            out.extend(mk_poly(v, n, w, src));
        } else if v.len() <= 512 {
            out.extend(ear_clip(&v, [-n[0], -n[1], -n[2]]).into_iter().filter_map(|t| mk_poly(t, n, w, src)));
        } else {
            return Err(failed("concave poly with more than 512 corners"));
        }
        if out.len() > MAX_PRIMS {
            return Err(failed("too many polygons (max 16384 per operand)"));
        }
    }
    Ok(out)
}

enum Side {
    Front,
    Back,
    Coplanar,
    Span(Poly, Poly),
}
/// Split `p` by the plane `pn . x = pw`.
fn split(p: &Poly, pn: V3, pw: f64) -> Side {
    let t: Vec<f64> = p.v.iter().map(|&q| dot(pn, q) - pw).collect();
    let (nf, nb) = (t.iter().filter(|&&x| x > EPS).count(), t.iter().filter(|&&x| x < -EPS).count());
    match (nf, nb) {
        (0, 0) => Side::Coplanar,
        (_, 0) => Side::Front,
        (0, _) => Side::Back,
        _ => {
            let (mut f, mut b) = (Vec::new(), Vec::new());
            let m = p.v.len();
            for i in 0..m {
                let j = (i + 1) % m;
                let (vi, vj, ti, tj) = (p.v[i], p.v[j], t[i], t[j]);
                let (fi, bi) = (ti > EPS, ti < -EPS);
                let (fj, bj) = (tj > EPS, tj < -EPS);
                if !bi {
                    f.push(vi);
                }
                if !fi {
                    b.push(vi);
                }
                if (fi && bj) || (bi && fj) {
                    let x = lerp(vi, vj, ti / (ti - tj));
                    f.push(x);
                    b.push(x);
                }
            }
            match (mk_poly(f, p.n, p.w, p.src), mk_poly(b, p.n, p.w, p.src)) {
                (Some(a), Some(c)) => Side::Span(a, c),
                (Some(_), None) => Side::Front,
                (None, Some(_)) => Side::Back,
                (None, None) => Side::Coplanar,
            }
        }
    }
}

struct Node2 {
    n: V3,
    w: f64,
    coplanar: Vec<Poly>,
    front: Option<usize>,
    back: Option<usize>,
}
/// BSP build work item: (parent node, front?) + polygons.
type Work = (Option<(usize, bool)>, Vec<Poly>);
/// Iterative BSP build (arena; depth is unbounded but stack-free).
fn build(polys: Vec<Poly>) -> Result<Vec<Node2>, CookError> {
    let mut nodes: Vec<Node2> = Vec::new();
    let mut stack: Vec<Work> = vec![(None, polys)];
    let mut made = 0usize;
    while let Some((parent, list)) = stack.pop() {
        let Some(first) = list.first() else { continue };
        let (n, w) = (first.n, first.w);
        let (mut cop, mut fr, mut bk) = (Vec::new(), Vec::new(), Vec::new());
        for p in list {
            match split(&p, n, w) {
                Side::Coplanar => cop.push(p),
                Side::Front => fr.push(p),
                Side::Back => bk.push(p),
                Side::Span(a, b) => {
                    fr.push(a);
                    bk.push(b);
                }
            }
            made += 1;
            if made > MAX_FRAGS {
                return Err(failed("fragment limit exceeded"));
            }
        }
        let me = nodes.len();
        nodes.push(Node2 { n, w, coplanar: cop, front: None, back: None });
        if let Some((pn, is_front)) = parent.and_then(|(pi, f)| nodes.get_mut(pi).map(|n| (n, f))) {
            if is_front {
                pn.front = Some(me);
            } else {
                pn.back = Some(me);
            }
        }
        stack.push((Some((me, true)), fr));
        stack.push((Some((me, false)), bk));
    }
    Ok(nodes)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tag {
    Out,
    In,
    CoSame,
    CoOpp,
}
/// `r` clipped to the convex face `q` (same plane): (inside part, outside pieces).
fn clip_to_face(r: Poly, q: &Poly) -> (Option<Poly>, Vec<Poly>) {
    let mut outs = Vec::new();
    let mut cur = Some(r);
    let m = q.v.len();
    for i in 0..m {
        let Some(c) = cur.take() else { break };
        let (a, b) = (q.v[i], q.v[(i + 1) % m]);
        let e = cross(sub(b, a), q.n);
        let l = len(e);
        if l < 1e-12 {
            cur = Some(c);
            continue;
        }
        let en = [e[0] / l, e[1] / l, e[2] / l];
        match split(&c, en, dot(en, a)) {
            Side::Front | Side::Coplanar => cur = Some(c),
            Side::Back => outs.push(c),
            Side::Span(f, bk) => {
                cur = Some(f);
                outs.push(bk);
            }
        }
    }
    (cur, outs)
}
/// Classify every polygon of `polys` against the solid bounded by `bsp`.
fn classify(polys: &[Poly], bsp: &[Node2]) -> Result<Vec<(Poly, Tag)>, CookError> {
    let mut out: Vec<(Poly, Tag)> = Vec::new();
    for p in polys {
        let mut stack: Vec<(Option<usize>, Poly)> = vec![(if bsp.is_empty() { None } else { Some(0) }, p.clone())];
        while let Some((ni, p)) = stack.pop() {
            if out.len() > MAX_FRAGS {
                return Err(failed("fragment limit exceeded"));
            }
            let Some(node) = ni.and_then(|i| bsp.get(i)) else {
                out.push((p, Tag::Out));
                continue;
            };
            let front = |p: Poly, stack: &mut Vec<(Option<usize>, Poly)>, out: &mut Vec<(Poly, Tag)>| match node.front {
                Some(c) => stack.push((Some(c), p)),
                None => out.push((p, Tag::Out)),
            };
            let back = |p: Poly, stack: &mut Vec<(Option<usize>, Poly)>, out: &mut Vec<(Poly, Tag)>| match node.back {
                Some(c) => stack.push((Some(c), p)),
                None => out.push((p, Tag::In)),
            };
            match split(&p, node.n, node.w) {
                Side::Front => front(p, &mut stack, &mut out),
                Side::Back => back(p, &mut stack, &mut out),
                Side::Span(f, b) => {
                    front(f, &mut stack, &mut out);
                    back(b, &mut stack, &mut out);
                }
                Side::Coplanar => {
                    let mut rem = vec![p.clone()];
                    for q in &node.coplanar {
                        let tag = if dot(p.n, q.n) > 0.0 { Tag::CoSame } else { Tag::CoOpp };
                        let mut next = Vec::new();
                        for r in rem {
                            let (inside, outs) = clip_to_face(r, q);
                            out.extend(inside.map(|i| (i, tag)));
                            next.extend(outs);
                        }
                        rem = next;
                    }
                    for r in rem {
                        front(r, &mut stack, &mut out);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Weld table (1e-5 grid).
#[derive(Default)]
struct Welder {
    map: HashMap<[i64; 3], u32>,
    pos: Vec<V3>,
}
impl Welder {
    fn id(&mut self, q: V3) -> Result<u32, CookError> {
        let k = [(q[0] * WELD).round() as i64, (q[1] * WELD).round() as i64, (q[2] * WELD).round() as i64];
        if let Some(&i) = self.map.get(&k) {
            return Ok(i);
        }
        let i = u32::try_from(self.pos.len()).map_err(|_| failed("point id overflow"))?;
        self.map.insert(k, i);
        self.pos.push(q);
        Ok(i)
    }
}

/// Insert, into every loop edge, every point that lies strictly inside it (sorted along the edge).
fn insert_t_vertices(loops: &mut [Vec<u32>], pos: &[V3]) {
    let edges: u64 = loops.iter().map(|l| l.len() as u64).sum();
    if edges.saturating_mul(pos.len() as u64) > MAX_TSCAN {
        return;
    }
    let mut by_x: Vec<u32> = (0..pos.len() as u32).collect();
    by_x.sort_by(|&a, &b| pos[a as usize][0].total_cmp(&pos[b as usize][0]));
    let xs: Vec<f64> = by_x.iter().map(|&i| pos[i as usize][0]).collect();
    for l in loops.iter_mut() {
        let mut nl = Vec::with_capacity(l.len());
        for i in 0..l.len() {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            nl.push(a);
            let (pa, pb) = (pos[a as usize], pos[b as usize]);
            let d = sub(pb, pa);
            let dl = len(d);
            if dl < 1e-9 {
                continue;
            }
            let (lo, hi) = (pa[0].min(pb[0]) - 2e-5, pa[0].max(pb[0]) + 2e-5);
            let s = xs.partition_point(|&x| x < lo);
            let mut hits: Vec<(f64, u32)> = Vec::new();
            for &c in by_x.iter().skip(s) {
                let pc = pos[c as usize];
                if pc[0] > hi {
                    break;
                }
                if c == a || c == b {
                    continue;
                }
                let t = dot(sub(pc, pa), d) / (dl * dl);
                if t <= 1e-9 || t >= 1.0 - 1e-9 {
                    continue;
                }
                if len(cross(sub(pc, pa), d)) / dl <= 2e-5 {
                    hits.push((t, c));
                }
            }
            hits.sort_by(|x, y| x.0.total_cmp(&y.0));
            nl.extend(hits.into_iter().map(|h| h.1));
        }
        *l = nl;
    }
}

/// Boundary of the union of loops as 1..=2 closed loops (outer + at most one hole), each starting at its lowest id.
fn boundary_loops(group: &[Vec<u32>]) -> Option<Vec<Vec<u32>>> {
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    for l in group {
        for i in 0..l.len() {
            *edges.entry((l[i], l[(i + 1) % l.len()])).or_default() += 1;
        }
    }
    let keep: Vec<(u32, u32)> =
        edges.iter().filter(|&(&(a, b), &c)| c == 1 && !edges.contains_key(&(b, a))).map(|(&e, _)| e).collect();
    if edges.values().any(|&c| c > 1) {
        return None;
    }
    // every cancelled edge must cancel exactly one-to-one
    let cancelled = edges.keys().filter(|&&(a, b)| edges.contains_key(&(b, a))).count();
    let total = edges.len();
    if keep.len() + cancelled != total {
        return None;
    }
    let mut next: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in &keep {
        if next.insert(a, b).is_some() {
            return None;
        }
    }
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut starts: Vec<u32> = keep.iter().map(|e| e.0).collect();
    starts.sort_unstable();
    let mut out: Vec<Vec<u32>> = Vec::new();
    for st in starts {
        if seen.contains(&st) {
            continue;
        }
        let mut lp = vec![st];
        seen.insert(st);
        let mut cur = *next.get(&st)?;
        while cur != st {
            lp.push(cur);
            seen.insert(cur);
            if lp.len() > keep.len() {
                return None;
            }
            cur = *next.get(&cur)?;
        }
        if lp.len() < 3 {
            return None;
        }
        out.push(lp);
        if out.len() > 2 {
            return None;
        }
    }
    (!out.is_empty() && out.iter().map(Vec::len).sum::<usize>() == keep.len()).then_some(out)
}

/// Houdini bridges a hole into ONE polygon: closest (outer, hole) vertex pair; cycle h0, o0, o1.., o0, h0, h1..h(m-1).
fn bridge_hole(ls: &[Vec<u32>], pos: &[V3]) -> Option<Vec<u32>> {
    let newell = |l: &Vec<u32>| -> Option<V3> {
        let mut n = [0.0f64; 3];
        for i in 0..l.len() {
            let (a, b) = (*pos.get(l[i] as usize)?, *pos.get(l[(i + 1) % l.len()] as usize)?);
            let c = cross(a, b);
            n = [n[0] + c[0], n[1] + c[1], n[2] + c[2]];
        }
        Some(n)
    };
    let (n0, n1) = (newell(ls.first()?)?, newell(ls.get(1)?)?);
    // a hole winds opposite to its outer loop; two same-wound loops are disjoint pieces, not a hole
    if dot(n0, n1) >= 0.0 {
        return None;
    }
    let (outer, hole) = if len(n0) >= len(n1) { (ls.first()?, ls.get(1)?) } else { (ls.get(1)?, ls.first()?) };
    let corner = |l: &Vec<u32>, i: usize| -> Option<bool> {
        let m = l.len();
        let (a, b, c) = (*pos.get(l[(i + m - 1) % m] as usize)?, *pos.get(l[i] as usize)?, *pos.get(l[(i + 1) % m] as usize)?);
        let (u, v) = (sub(b, a), sub(c, b));
        Some(len(cross(u, v)) > 1e-7 * len(u) * len(v))
    };
    let mut best: Option<(f64, usize, usize)> = None;
    for (i, &o) in outer.iter().enumerate() {
        if !corner(outer, i)? {
            continue;
        }
        for (j, &h) in hole.iter().enumerate() {
            let d = len(sub(*pos.get(o as usize)?, *pos.get(h as usize)?));
            if best.map_or(true, |b| d < b.0 - 1e-12) {
                best = Some((d, i, j));
            }
        }
    }
    let (_, i, j) = best?;
    let (n, m) = (outer.len(), hole.len());
    let mut out: Vec<u32> = vec![hole[j]];
    for k in 0..=n {
        out.push(outer[(i + k) % n]);
    }
    for k in 0..m {
        out.push(hole[(j + k) % m]);
    }
    Some(out)
}

/// Drop vertices that are collinear in EVERY loop that uses them; loops left with < 3 corners vanish.
fn drop_collinear(loops: &mut Vec<(Vec<u32>, usize)>, pos: &[V3]) {
    for _ in 0..4 {
        let mut ok: HashMap<u32, bool> = HashMap::new();
        for (l, _) in loops.iter() {
            let m = l.len();
            for i in 0..m {
                let (a, b, c) = (pos[l[(i + m - 1) % m] as usize], pos[l[i] as usize], pos[l[(i + 1) % m] as usize]);
                let (u, v) = (sub(b, a), sub(c, b));
                let flat = len(cross(u, v)) <= 1e-7 * len(u) * len(v) && dot(u, v) > 0.0;
                let e = ok.entry(l[i]).or_insert(true);
                *e &= flat;
            }
        }
        let mut changed = false;
        for (l, _) in loops.iter_mut() {
            let before = l.len();
            l.retain(|v| !ok.get(v).copied().unwrap_or(false));
            changed |= l.len() != before;
        }
        loops.retain(|(l, _)| l.len() >= 3);
        if !changed {
            break;
        }
    }
}

fn run(a: &Detail, b: &Detail, op: u32, solid: bool) -> Result<Detail, CookError> {
    let (pa, pb) = (polys_of(a)?, polys_of(b)?);
    let (bsp_a, bsp_b) = (build(pa.clone())?, build(pb.clone())?);
    let ca = classify(&pa, &bsp_b)?;
    let cb = classify(&pb, &bsp_a)?;
    // seam: a source face that crosses (Out+In) or lies on (Co*) the other surface
    let seam_of = |c: &[(Poly, Tag)]| -> HashSet<u32> {
        let mut tags: HashMap<u32, (bool, bool, bool)> = HashMap::new();
        for (p, t) in c {
            let e = tags.entry(p.src).or_default();
            match t {
                Tag::Out => e.0 = true,
                Tag::In => e.1 = true,
                Tag::CoSame | Tag::CoOpp => e.2 = true,
            }
        }
        tags.into_iter().filter(|(_, (o, i, c))| *c || (*o && *i)).map(|(k, _)| k).collect()
    };
    let (seam_a, seam_b) = (seam_of(&ca), seam_of(&cb));
    // (poly, side 0=A/1=B, tag, flip)
    let mut kept: Vec<(Poly, u8, Tag, bool)> = Vec::new();
    for (p, t) in ca {
        let take = matches!(
            (op, solid, t),
            (OP_UNION, true, Tag::Out | Tag::CoSame)
                | (OP_UNION, false, _)
                | (OP_SUBTRACT, _, Tag::Out | Tag::CoOpp)
                | (OP_INTERSECT, _, Tag::In | Tag::CoSame)
                | (OP_SHATTER, _, _)
        );
        if take {
            kept.push((p, 0, t, false));
        }
    }
    for (p, t) in cb {
        let (take, flip) = match (op, solid, t) {
            (OP_UNION, true, Tag::Out) => (true, false),
            (OP_UNION, false, Tag::Out | Tag::In | Tag::CoOpp) => (true, false),
            (OP_SUBTRACT, true, Tag::In) => (true, true),
            (OP_INTERSECT, true, Tag::In) => (true, false),
            _ => (false, false),
        };
        if take {
            kept.push((p, 1, t, flip));
        }
    }
    // weld
    let mut w = Welder::default();
    // (loop, key) where key indexes `meta`
    let mut meta: Vec<(u8, u32, Tag, bool)> = Vec::new();
    let mut meta_ix: HashMap<(u8, u32, u8, bool), usize> = HashMap::new();
    let mut loops: Vec<(Vec<u32>, usize)> = Vec::new();
    for (p, side, t, flip) in kept {
        let mut ids: Vec<u32> = Vec::with_capacity(p.v.len());
        for q in &p.v {
            let i = w.id(*q)?;
            if ids.last() != Some(&i) {
                ids.push(i);
            }
        }
        while ids.len() > 1 && ids.first() == ids.last() {
            ids.pop();
        }
        if ids.len() < 3 {
            continue;
        }
        if flip {
            ids.reverse();
        }
        let key = (side, p.src, t as u8, flip);
        let m = *meta_ix.entry(key).or_insert_with(|| {
            meta.push((side, p.src, t, flip));
            meta.len() - 1
        });
        loops.push((ids, m));
    }
    // T-vertices, fuse per (source, tag), drop collinear
    let mut only: Vec<Vec<u32>> = loops.iter().map(|l| l.0.clone()).collect();
    insert_t_vertices(&mut only, &w.pos);
    for (l, o) in loops.iter_mut().zip(only) {
        l.0 = o;
    }
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); meta.len()];
    for (i, (_, m)) in loops.iter().enumerate() {
        groups[*m].push(i);
    }
    let mut fused: Vec<(Vec<u32>, usize)> = Vec::new();
    for (m, g) in groups.iter().enumerate() {
        let ls: Vec<Vec<u32>> = g.iter().filter_map(|&i| loops.get(i).map(|l| l.0.clone())).collect();
        let one = (ls.len() > 1).then(|| boundary_loops(&ls)).flatten().and_then(|b| match b.len() {
            1 => b.into_iter().next(),
            _ => bridge_hole(&b, &w.pos),
        });
        match one {
            Some(l) => fused.push((l, m)),
            None => fused.extend(ls.into_iter().map(|l| (l, m))),
        }
    }
    drop_collinear(&mut fused, &w.pos);
    // emit
    // Houdini numbers points by first appearance over the prim order
    let mut used: Vec<u32> = Vec::new();
    {
        let mut seen_pt: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for (l, _) in &fused {
            for &v in l {
                if seen_pt.insert(v) {
                    used.push(v);
                }
            }
        }
    }
    let remap: HashMap<u32, u32> = used.iter().enumerate().map(|(k, &o)| (o, k as u32)).collect();
    let mut d = Detail::default();
    d.add_points(u32::try_from(used.len()).map_err(|_| failed("point id overflow"))?);
    *d.p_mut() = used
        .iter()
        .map(|&o| {
            let q = w.pos.get(o as usize).copied().unwrap_or([0.0; 3]);
            [q[0] as f32, q[1] as f32, q[2] as f32]
        })
        .collect();
    let (mut ga, mut gb, mut gs) = (Vec::new(), Vec::new(), Vec::new());
    let (mut gin, mut gout) = (Vec::new(), Vec::new());
    let mut psrc: Vec<(u8, Option<usize>)> = Vec::new();
    // first output prim of each operand using each point (interpolation source)
    let mut users: Vec<[Option<usize>; 2]> = vec![[None; 2]; used.len()];
    // per output point: which (side, inside?) classes of prims use it (bit = side * 2 + inside)
    let mut pclass = vec![0u8; used.len()];
    for (l, m) in &fused {
        let pts: Vec<u32> = l.iter().filter_map(|v| remap.get(v).copied()).collect();
        d.add_poly(&pts, true);
        let (side, src, tag, _) = meta[*m];
        let seam = if side == 0 { seam_a.contains(&src) } else { seam_b.contains(&src) };
        ga.push(side == 0 && !seam);
        gb.push(side == 1 && !seam);
        gs.push(seam);
        let inside = matches!(tag, Tag::In | Tag::CoSame);
        gin.push(inside);
        gout.push(!inside);
        psrc.push((side, Some(src as usize)));
        for &p in &pts {
            if let Some(u) = users.get_mut(p as usize) {
                u[side as usize].get_or_insert(psrc.len() - 1);
            }
            if let Some(c) = pclass.get_mut(p as usize) {
                *c |= 1 << (side * 2 + u8::from(inside));
            }
        }
    }
    let mut named = vec![("a_only", ga), ("b_only", gb), ("seam", gs)];
    if op == OP_SHATTER {
        named.push(("inside", gin));
        named.push(("outside", gout));
    }
    for (name, mask) in named {
        d.groups.push(Group { name: name.into(), class: AttrClass::Prim, mask });
    }
    // intersection points: used by prims of at least two different (operand, inside/outside) classes
    let seam_pts: Vec<bool> = pclass.iter().map(|c| c.count_ones() >= 2).collect();
    d.groups.push(Group { name: "seampts".into(), class: AttrClass::Point, mask: seam_pts });
    // attribute transfer: prim attrs follow the source prim; vertex attrs the source vertex at the same position; point attrs the
    // coincident operand point or, for fresh intersection points, a linear interpolation across the source polygon
    transfer(&mut d, a, b, AttrClass::Prim, &psrc);
    let mut vsrc: Vec<(u8, Option<usize>)> = Vec::new();
    for (j, pr) in d.prims.iter().enumerate() {
        let (side, src) = psrc[j];
        let det = if side == 0 { a } else { b };
        let sp = src.and_then(|s| det.prims.get(s));
        for vi in pr.verts.start..pr.verts.end {
            let q = d.vertex_point.get(vi as usize).and_then(|&pt| d.p().get(pt as usize)).copied();
            let found = sp.and_then(|sp| {
                (sp.verts.start..sp.verts.end).find(|&sv| {
                    det.vertex_point.get(sv as usize).and_then(|&pt| det.p().get(pt as usize)).copied() == q
                })
            });
            vsrc.push((side, found.map(|v| v as usize)));
        }
    }
    transfer(&mut d, a, b, AttrClass::Vertex, &vsrc);
    let key = |q: &[f32; 3]| q.map(|c| (c + 0.0).to_bits());
    let at: [HashMap<[u32; 3], usize>; 2] = [a, b].map(|det| {
        let mut m = HashMap::new();
        for (i, q) in det.p().iter().enumerate() {
            m.entry(key(q)).or_insert(i);
        }
        m
    });
    let cand: Vec<[Option<Cand>; 2]> = (0..d.p().len())
        .map(|o| {
            let q = d.p()[o];
            let one = |side: usize| -> Option<Cand> {
                let det = if side == 0 { a } else { b };
                if let Some(&i) = at[side].get(&key(&q)) {
                    return Some(Cand { w: vec![(i, 1.0)], exact: true });
                }
                let src = psrc.get(users[o][side]?)?.1?;
                let w = poly_weights(det, src, q.map(f64::from))?;
                Some(Cand { w, exact: false })
            };
            [one(0), one(1)]
        })
        .collect();
    transfer_points(&mut d, a, b, &cand);
    transfer_groups(&mut d, a, b, &psrc, &cand);
    Ok(d)
}

/// Source of one output point on one operand: weighted source points (exact coincident = one point, weight 1).
struct Cand {
    w: Vec<(usize, f64)>,
    exact: bool,
}
/// Linear weights of `q` over the corners of closed poly `src` of `det` (fan triangle containing `q`; the least-violating one
/// with clamped, renormalised weights otherwise).
fn poly_weights(det: &Detail, src: usize, q: V3) -> Option<Vec<(usize, f64)>> {
    let ids = super::meshutil::closed_poly_pts(det, det.prims.get(src)?)?;
    let p: Vec<V3> = ids.iter().map(|&i| det.p().get(i).map(|v| v.map(f64::from))).collect::<Option<_>>()?;
    let mut best: Option<(f64, [f64; 3], [usize; 3])> = None;
    for k in 1..p.len().saturating_sub(1) {
        let (u, v, w) = (sub(p[k], p[0]), sub(p[k + 1], p[0]), sub(q, p[0]));
        let (d00, d01, d11, d20, d21) = (dot(u, u), dot(u, v), dot(v, v), dot(w, u), dot(w, v));
        let den = d00 * d11 - d01 * d01;
        if den.abs() < 1e-18 {
            continue;
        }
        let (l1, l2) = ((d11 * d20 - d01 * d21) / den, (d00 * d21 - d01 * d20) / den);
        let l = [1.0 - l1 - l2, l1, l2];
        let worst = l.iter().copied().fold(f64::MAX, f64::min);
        if best.as_ref().is_none_or(|b| worst > b.0) {
            best = Some((worst, l, [0, k, k + 1]));
        }
    }
    let (_, l, ix) = best?;
    let l = l.map(|c| c.max(0.0));
    let s: f64 = l.iter().sum();
    (s > 0.0).then(|| ix.iter().zip(l).map(|(&k, c)| (ids[k], c / s)).collect())
}
/// Point attributes: per element the first operand (A, then B) that has the attribute and a source for the point.
fn transfer_points(d: &mut Detail, a: &Detail, b: &Detail, cand: &[[Option<Cand>; 2]]) {
    let n = cand.len();
    let mut seen: HashSet<&str> = HashSet::new();
    for at in a.attrs.iter().chain(b.attrs.iter()).filter(|x| x.class == AttrClass::Point && x.name != "P") {
        if !seen.insert(at.name.as_str()) {
            continue;
        }
        let das = [a.attr(AttrClass::Point, &at.name), b.attr(AttrClass::Point, &at.name)];
        let pick: Vec<Option<usize>> =
            (0..n).map(|o| (0..2).find(|&s| das[s].is_some() && cand[o][s].is_some())).collect();
        let srcs = |s: usize| -> Vec<Vec<(usize, f64)>> {
            (0..n)
                .map(|o| {
                    if pick[o] == Some(s) {
                        cand[o][s].as_ref().map_or(Vec::new(), |c| c.w.clone())
                    } else {
                        Vec::new()
                    }
                })
                .collect()
        };
        let data = match (das[0], das[1]) {
            (Some(x), Some(y)) => {
                let mut c = blend_weighted(&x.data, &srcs(0));
                if !c.extend_from(&blend_weighted(&y.data, &srcs(1))) {
                    continue;
                }
                let idx: Vec<Option<usize>> = pick.iter().enumerate().map(|(o, s)| s.map(|s| o + s * n)).collect();
                gather(&c, &idx)
            }
            (Some(x), None) => blend_weighted(&x.data, &srcs(0)),
            (None, Some(y)) => blend_weighted(&y.data, &srcs(1)),
            (None, None) => continue,
        };
        d.add_attr(Attribute { name: at.name.clone(), class: AttrClass::Point, data, type_info: at.type_info });
    }
}
/// Prim groups follow the source prim; point groups the exactly coincident operand point (never interpolated points).
/// Operand groups named like one of ours (`a_only`, `b_only`, `seam`, `seampts`, `inside`, `outside`) are dropped.
fn transfer_groups(d: &mut Detail, a: &Detail, b: &Detail, psrc: &[(u8, Option<usize>)], cand: &[[Option<Cand>; 2]]) {
    let mut done: HashSet<(AttrClass, String)> = d.groups.iter().map(|g| (g.class, g.name.clone())).collect();
    for g in a.groups.iter().chain(b.groups.iter()) {
        if !matches!(g.class, AttrClass::Prim | AttrClass::Point) || !done.insert((g.class, g.name.clone())) {
            continue;
        }
        let dets = [a, b];
        let member = |side: usize, i: usize| {
            dets[side].group(g.class, &g.name).and_then(|x| x.mask.get(i).copied()).unwrap_or(false)
        };
        let mask: Vec<bool> = if g.class == AttrClass::Prim {
            psrc.iter().map(|&(side, src)| src.is_some_and(|s| member(side as usize, s))).collect()
        } else {
            cand.iter()
                .map(|c| {
                    (0..2).any(|s| {
                        c[s].as_ref().is_some_and(|c| c.exact && c.w.first().is_some_and(|&(i, _)| member(s, i)))
                    })
                })
                .collect()
        };
        d.groups.push(Group { name: g.name.clone(), class: g.class, mask });
    }
}
/// Copy every `class` attribute of A / B (except P) onto `d`; `src[i]` = (operand, element) feeding output element `i`
/// (`None` / missing side = default value). Names on both sides need the same type, else skipped.
fn transfer(d: &mut Detail, a: &Detail, b: &Detail, class: AttrClass, src: &[(u8, Option<usize>)]) {
    let (na, _nb) = (a.class_len(class), b.class_len(class));
    let mut seen: HashSet<&str> = HashSet::new();
    for at in a.attrs.iter().chain(b.attrs.iter()).filter(|x| x.class == class && x.name != "P") {
        if !seen.insert(at.name.as_str()) {
            continue;
        }
        let (da, db) = (a.attr(class, &at.name), b.attr(class, &at.name));
        let data = match (da, db) {
            (Some(x), Some(y)) => {
                let mut c = x.data.clone();
                if c.len() != na || !c.extend_from(&y.data) {
                    continue;
                }
                c
            }
            (Some(x), None) => x.data.clone(),
            (None, Some(y)) => y.data.clone(),
            (None, None) => continue,
        };
        let idx: Vec<Option<usize>> = src
            .iter()
            .map(|&(side, i)| match (side, da.is_some(), db.is_some()) {
                (0, true, _) | (1, false, true) => i,
                (1, true, true) => i.map(|k| k + na),
                _ => None,
            })
            .collect();
        d.add_attr(Attribute { name: at.name.clone(), class, data: gather(&data, &idx), type_info: at.type_info });
    }
}
fn has_solid(d: &Detail) -> bool {
    d.prims.iter().any(|p| matches!(p.kind, PrimKind::Poly { closed: true }) && p.verts.len() >= 3)
}

impl Node<Detail> for Boolean {
    fn type_name(&self) -> &'static str {
        "boolean"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        // NURBS/Bezier prims are tessellated (8 samples per span) and take part as polygons / are ignored as curves
        let tess = super::splinesurf::tess_inputs(inputs);
        let inputs = &tess[..];
        let p = ctx.params;
        let op = match p.menu("operation") {
            Some(v) if v <= OP_SHATTER => v,
            _ => return Err(bad("`operation`: missing or menu index out of range 0..=4")),
        };
        let solid = p.toggle("treat_as_solid").ok_or_else(|| bad("`treat_as_solid`: missing or not a toggle"))?;
        let empty = Arc::new(Detail::default());
        let (mut a, mut b) = (inputs.first().unwrap_or(&empty), inputs.get(1).unwrap_or(&empty));
        // B - A = A' - B' with the operands swapped (the a_only / b_only group names are swapped back below)
        let swapped = op == OP_SUBTRACT_BA;
        let op = if swapped { OP_SUBTRACT } else { op };
        if swapped {
            std::mem::swap(&mut a, &mut b);
        }
        let fix = |mut d: Detail| {
            if swapped {
                for g in d.groups.iter_mut().filter(|g| g.class == AttrClass::Prim) {
                    match g.name.as_str() {
                        "a_only" => g.name = "b_only".into(),
                        "b_only" => g.name = "a_only".into(),
                        _ => {}
                    }
                }
            }
            d
        };
        match (has_solid(a), has_solid(b), op) {
            (false, false, _) => Ok(empty),
            (false, true, OP_UNION) => Ok(Arc::clone(b)),
            (false, _, _) => Ok(empty),
            (true, false, OP_INTERSECT) => Ok(empty),
            (true, false, _) => Ok(Arc::clone(a)),
            (true, true, _) => Ok(Arc::new(fix(run(a, b, op, solid)?))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_graph::ParamSet;

    /// Cube corner `i` (bit2 -> x, bit1 -> y, bit0 -> z), each coordinate centre +- size/2.
    const FACES: [[u32; 4]; 6] = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    fn cube(c: [f32; 3], s: [f32; 3]) -> Detail {
        let mut d = Detail::default();
        d.add_points(8);
        for i in 0..8u32 {
            let k = |b: u32, a: usize| c[a] + if (i >> b) & 1 == 1 { s[a] / 2.0 } else { -s[a] / 2.0 };
            d.p_mut()[i as usize] = [k(2, 0), k(1, 1), k(0, 2)];
        }
        for f in FACES {
            d.add_poly(&f, true);
        }
        d
    }
    fn unit(c: [f32; 3]) -> Detail {
        cube(c, [1.0; 3])
    }
    /// L-shaped prism: L (0,0)(2,0)(2,1)(1,1)(1,2)(0,2) extruded z 0..1. Area 3 -> volume 3. Concave caps.
    fn l_prism() -> Detail {
        let ring = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        let mut d = Detail::default();
        d.add_points(12);
        for (i, q) in ring.iter().enumerate() {
            d.p_mut()[i] = [q[0], q[1], 0.0];
            d.p_mut()[6 + i] = [q[0], q[1], 1.0];
        }
        d.add_poly(&[0, 1, 2, 3, 4, 5], true);
        d.add_poly(&[11, 10, 9, 8, 7, 6], true);
        for i in 0..6u32 {
            let j = (i + 1) % 6;
            d.add_poly(&[i, 6 + i, 6 + j, j], true);
        }
        d
    }
    fn run_with(a: &Detail, b: &Detail, op: u32, solid: bool) -> Result<Arc<Detail>, CookError> {
        let p = ParamSet::new().with("operation", ParamValue::Menu(op)).with("treat_as_solid", ParamValue::B(solid));
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        new().cook(&ctx, &[Arc::new(a.clone()), Arc::new(b.clone())])
    }
    fn bool_(a: &Detail, b: &Detail, op: u32) -> Arc<Detail> {
        run_with(a, b, op, true).expect("cooks")
    }
    fn corners(d: &Detail) -> Vec<usize> {
        let mut v: Vec<usize> = d.prims.iter().map(|p| p.verts.len()).collect();
        v.sort_unstable();
        v
    }
    /// Signed volume by the divergence theorem (fan triangles), independent of the SOP.
    fn volume(d: &Detail) -> f64 {
        let p = |i: u32| d.p()[i as usize].map(f64::from);
        let mut v = 0.0;
        for pr in &d.prims {
            let ids = &d.vertex_point[pr.verts.start as usize..pr.verts.end as usize];
            for k in 1..ids.len() - 1 {
                let (a, b, c) = (p(ids[0]), p(ids[k]), p(ids[k + 1]));
                v += super::super::meshutil::tri_volume(a, b, c);
            }
        }
        v
    }
    /// Every directed edge occurs once and its reverse occurs once (closed 2-manifold, no T-junctions).
    fn assert_watertight(d: &Detail) {
        let mut e: HashMap<(u32, u32), u32> = HashMap::new();
        for pr in &d.prims {
            let ids = &d.vertex_point[pr.verts.start as usize..pr.verts.end as usize];
            for i in 0..ids.len() {
                *e.entry((ids[i], ids[(i + 1) % ids.len()])).or_default() += 1;
            }
        }
        for (&(a, b), &c) in &e {
            assert_eq!(c, 1, "edge {a}->{b} repeated");
            assert_eq!(e.get(&(b, a)), Some(&1), "edge {a}->{b} has no unique partner (T-junction / open)");
        }
    }
    fn euler(d: &Detail) -> i64 {
        let sides: usize = d.prims.iter().map(|p| p.verts.len()).sum();
        i64::from(d.point_count) - (sides / 2) as i64 + d.prims.len() as i64
    }
    fn pts(d: &Detail) -> Vec<[i64; 3]> {
        let mut v: Vec<[i64; 3]> = d.p().iter().map(|q| q.map(|c| (f64::from(c) * 1000.0).round() as i64)).collect();
        v.sort_unstable();
        v
    }
    fn group_count(d: &Detail, n: &str) -> usize {
        d.group(AttrClass::Prim, n).map_or(usize::MAX, |g| g.mask.iter().filter(|b| **b).count())
    }
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "boolean");
        let names: Vec<&str> = n.inputs().iter().map(|i| i.name).collect();
        assert_eq!(names, ["input0", "input1"]);
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(got, [("operation", ParamValue::Menu(OP_UNION)), ("treat_as_solid", ParamValue::B(true))]);
    }

    #[test]
    fn union_of_cubes_offset_along_x_by_half_is_14_quads_16_points() {
        // A=[-.5,.5]^3, B=[0,1]x[-.5,.5]^2. Hand count: -x cap (A) + +x cap (B) + 4 sides x 3 strips
        // (A only x[-.5,0], overlap x[0,.5] once, B only x[.5,1]) = 14; points = 4 x-slices x 4 = 16; chi = 2.
        let out = bool_(&unit([0.0; 3]), &unit([0.5, 0.0, 0.0]), OP_UNION);
        assert_eq!((out.point_count, out.prim_count()), (16, 14));
        assert_eq!(corners(&out), [4; 14]);
        let mut want = Vec::new();
        for x in [-500, 0, 500, 1000] {
            for y in [-500, 500] {
                for z in [-500, 500] {
                    want.push([x, y, z]);
                }
            }
        }
        want.sort_unstable();
        assert_eq!(pts(&out), want);
        assert!(close(volume(&out), 1.5), "{}", volume(&out));
        assert_eq!(euler(&out), 2);
        assert_watertight(&out);
        // groups: A cap alone, B cap alone, the 12 side strips lie on the shared surface
        assert_eq!((group_count(&out, "a_only"), group_count(&out, "b_only"), group_count(&out, "seam")), (1, 1, 12));
    }

    #[test]
    fn union_of_cubes_offset_diagonally_is_6_quads_6_hexagons_20_points() {
        // A=[-.5,.5]^3, B=[0,1]^3. A: 3 faces whole + 3 L-shaped (face minus a .5 square) ; same for B.
        // Points: 7 + 7 outside corners + 6 cut-curve corners = 20; sides 6*4+6*6 = 60 -> E=30; V-E+F = 20-30+12 = 2.
        let out = bool_(&unit([0.0; 3]), &unit([0.5; 3]), OP_UNION);
        assert_eq!((out.point_count, out.prim_count()), (20, 12));
        assert_eq!(corners(&out), [4, 4, 4, 4, 4, 4, 6, 6, 6, 6, 6, 6]);
        assert_eq!(euler(&out), 2);
        assert!(close(volume(&out), 2.0 - 0.125), "{}", volume(&out));
        assert_watertight(&out);
        let p = pts(&out);
        for must in [[500, 0, 0], [0, 500, 500], [500, 500, 0], [0, 0, 500], [1000, 1000, 1000], [-500, -500, -500]] {
            assert!(p.contains(&must), "missing {must:?}");
        }
        for gone in [[500, 500, 500], [0, 0, 0]] {
            assert!(!p.contains(&gone), "interior corner {gone:?} survived");
        }
        assert_eq!(group_count(&out, "a_only") + group_count(&out, "b_only") + group_count(&out, "seam"), 12);
        assert_eq!((group_count(&out, "a_only"), group_count(&out, "b_only")), (3, 3));
    }

    #[test]
    fn subtract_cube_minus_cube_diagonal_is_9_prims_14_points() {
        // A - B : A's 3 whole faces + 3 hexagons + B's 3 inner squares (flipped). Points 7+3+3+1 = 14; sides 6*4+3*6 = 42.
        let out = bool_(&unit([0.0; 3]), &unit([0.5; 3]), OP_SUBTRACT);
        assert_eq!((out.point_count, out.prim_count()), (14, 9));
        assert_eq!(corners(&out), [4, 4, 4, 4, 4, 4, 6, 6, 6]);
        assert_eq!(euler(&out), 2);
        assert!(close(volume(&out), 1.0 - 0.125), "{}", volume(&out));
        assert_watertight(&out);
        assert!(pts(&out).contains(&[0, 0, 0]), "inner corner of the notch");
    }

    fn int_attr(d: &mut Detail, class: AttrClass, name: &str, v: Vec<i32>) {
        d.add_attr(Attribute {
            name: name.into(),
            class,
            data: hexe_geo::AttrData::I32(v),
            type_info: hexe_geo::TypeInfo::None,
        });
    }
    fn i32s(d: &Detail, class: AttrClass, name: &str) -> Vec<i32> {
        match d.attr(class, name).map(|a| &a.data) {
            Some(hexe_geo::AttrData::I32(v)) => v.clone(),
            _ => panic!("no i32 `{name}`"),
        }
    }
    /// Prim attr on A only: disjoint union keeps A's 6 ids (10..16), B's 6 prims default to 0; attr on both: ids 1 / 2 follow the operand.
    #[test]
    fn prim_attributes_follow_the_source_prim_and_default_on_the_other_operand() {
        let (mut a, mut b) = (unit([0.0; 3]), unit([5.0, 0.0, 0.0]));
        int_attr(&mut a, AttrClass::Prim, "id", (10..16).collect());
        let out = bool_(&a, &b, OP_UNION);
        assert_eq!(out.prim_count(), 12);
        let mut ids = i32s(&out, AttrClass::Prim, "id");
        ids.sort_unstable();
        assert_eq!(ids, [0, 0, 0, 0, 0, 0, 10, 11, 12, 13, 14, 15]);
        int_attr(&mut a, AttrClass::Prim, "side", vec![1; 6]);
        int_attr(&mut b, AttrClass::Prim, "side", vec![2; 6]);
        let out = bool_(&a, &b, OP_UNION);
        let mut s = i32s(&out, AttrClass::Prim, "side");
        s.sort_unstable();
        assert_eq!(s, [1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2]);
        // the ids line up with the geometry: prims with side 2 sit at x >= 4.5
        for (i, &sd) in i32s(&out, AttrClass::Prim, "side").iter().enumerate() {
            let p = &out.vertex_point[out.prims[i].verts.start as usize..out.prims[i].verts.end as usize];
            assert!(p.iter().all(|&q| (out.p()[q as usize][0] >= 4.5) == (sd == 2)));
        }
    }
    /// Point attr w = index on A: points that coincide with an A point keep w, B points / new points default 0.
    #[test]
    fn point_attributes_follow_coincident_operand_points() {
        let (mut a, b) = (unit([0.0; 3]), unit([5.0, 0.0, 0.0]));
        a.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: hexe_geo::AttrData::F32((0..8).map(|i| 1.0 + i as f32).collect()),
            type_info: hexe_geo::TypeInfo::None,
        });
        let out = bool_(&a, &b, OP_UNION);
        let Some(hexe_geo::AttrData::F32(w)) = out.attr(AttrClass::Point, "w").map(|x| &x.data) else { panic!("w") };
        assert_eq!(w.len(), 16);
        for (i, q) in out.p().iter().enumerate() {
            let src = a.p().iter().position(|p| p == q);
            assert_eq!(w[i], src.map_or(0.0, |k| 1.0 + k as f32), "point {i} {q:?}");
        }
        assert_eq!(w.iter().filter(|v| **v > 0.0).count(), 8);
    }
    /// seampts = the intersection points: union of cubes offset (.5,.5,.5) has exactly the 6 hexagon corners on both surfaces.
    #[test]
    fn seampts_group_is_the_intersection_points() {
        let out = bool_(&unit([0.0; 3]), &unit([0.5; 3]), OP_UNION);
        assert_eq!(out.point_count, 20);
        let g = out.group(AttrClass::Point, "seampts").expect("seampts");
        assert_eq!(g.mask.iter().filter(|b| **b).count(), 6);
        // each seam point lies on a face of A (a coordinate = .5 with the others in [0,.5]) and on a face of B (a coordinate = 0)
        for (i, _) in g.mask.iter().enumerate().filter(|(_, b)| **b) {
            let q = out.p()[i].map(f64::from);
            assert!(q.iter().any(|c| close(*c, 0.5)) && q.iter().any(|c| close(*c, 0.0)), "{q:?}");
        }
    }
    /// B - A: same shape as A - B with the roles swapped (volume 1 - 1/8, 9 prims, 14 points); groups name B's own faces `b_only`.
    #[test]
    fn subtract_ba_is_b_minus_a_with_swapped_group_names() {
        let (a, b) = (unit([0.0; 3]), unit([0.5; 3]));
        let out = bool_(&a, &b, OP_SUBTRACT_BA);
        assert_eq!((out.point_count, out.prim_count()), (14, 9));
        assert!(close(volume(&out), 0.875), "{}", volume(&out));
        assert_watertight(&out);
        // B's far corner (1,1,1) survives, A's far corner (-.5,-.5,-.5) does not
        assert!(
            pts(&out).iter().any(|p| p[0] > 0 && p[1] > 0 && p[2] > 0 && out.p().iter().any(|q| q == &[1.0, 1.0, 1.0]))
        );
        assert!(!out.p().iter().any(|q| q == &[-0.5, -0.5, -0.5]));
        // the 3 whole faces + 3 hexagons from B are `b_only`, the 3 flipped squares of A inside B are `a_only`... named by operand
        assert_eq!(group_count(&out, "b_only") + group_count(&out, "a_only") + group_count(&out, "seam"), 9);
        let ab = bool_(&a, &b, OP_SUBTRACT);
        assert_eq!(ab.prim_count(), out.prim_count());
    }
    /// Shatter: A cut by B's surface, every piece kept (B dropped): 9 prims (3 inside quarter-squares 0.25, 3 whole + 3 L-hexagons outside),
    /// total area 6 (the cube surface is unchanged), inside area 0.75.
    #[test]
    fn shatter_keeps_all_pieces_of_a_split_into_inside_and_outside_groups() {
        let out = bool_(&unit([0.0; 3]), &unit([0.5; 3]), OP_SHATTER);
        assert_eq!(out.prim_count(), 9);
        assert_eq!((group_count(&out, "inside"), group_count(&out, "outside")), (3, 6));
        let area = |i: usize| -> f64 {
            let ids = &out.vertex_point[out.prims[i].verts.start as usize..out.prims[i].verts.end as usize];
            let p: Vec<[f64; 3]> = ids.iter().map(|&q| out.p()[q as usize].map(f64::from)).collect();
            let n = newell_corners(&p);
            0.5 * len(n)
        };
        let inside = out.group(AttrClass::Prim, "inside").expect("inside");
        let (mut tot, mut ins) = (0.0, 0.0);
        for i in 0..out.prims.len() {
            tot += area(i);
            if inside.mask[i] {
                ins += area(i);
            }
        }
        assert!(close(tot, 6.0) && close(ins, 0.75), "total {tot} inside {ins}");
        // B contributes nothing: all points lie on the surface of A's cube [-.5,.5]^3
        assert!(
            out.p()
                .iter()
                .all(|q| q.iter().all(|c| c.abs() <= 0.5 + 1e-6) && q.iter().any(|c| (c.abs() - 0.5).abs() < 1e-6))
        );
    }
    /// Point attr w = x on A only, union with B offset (.5,.5,.5): every output point on A's surface (incl. the 6 FRESH intersection
    /// points, interpolated across A's faces) has w == x exactly (linear field); B-only points default to 0.
    #[test]
    fn point_attributes_are_interpolated_at_intersection_points() {
        let (mut a, b) = (unit([0.0; 3]), unit([0.5; 3]));
        let xs: Vec<f32> = a.p().iter().map(|p| p[0]).collect();
        a.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: hexe_geo::AttrData::F32(xs),
            type_info: hexe_geo::TypeInfo::None,
        });
        let out = bool_(&a, &b, OP_UNION);
        let Some(hexe_geo::AttrData::F32(w)) = out.attr(AttrClass::Point, "w").map(|x| &x.data) else { panic!("w") };
        assert_eq!(w.len(), 20);
        let (mut on_a, mut fresh) = (0, 0);
        let seam = out.group(AttrClass::Point, "seampts").expect("seampts");
        for (i, q) in out.p().iter().enumerate() {
            let inside_a = q.iter().all(|c| c.abs() <= 0.5 + 1e-6);
            if inside_a {
                on_a += 1;
                assert!((w[i] - q[0]).abs() < 1e-5, "point {i} {q:?}: w {}", w[i]);
                fresh += usize::from(seam.mask[i] && !a.p().contains(q));
            } else {
                assert_eq!(w[i], 0.0, "B-only point {i} {q:?}");
            }
        }
        assert_eq!((on_a, fresh), (13, 6), "7 A corners + 6 intersection points, all 6 interpolated");
    }
    /// Vertex attr: every output vertex of an A prim carries the attr of the source vertex at the same position; B's default to 0.
    #[test]
    fn vertex_attributes_follow_the_source_vertex_at_the_same_position() {
        let (mut a, b) = (unit([0.0; 3]), unit([5.0, 0.0, 0.0]));
        int_attr(&mut a, AttrClass::Vertex, "vw", (100..124).collect());
        let out = bool_(&a, &b, OP_UNION);
        let vw = i32s(&out, AttrClass::Vertex, "vw");
        assert_eq!(vw.len(), 48);
        for (vi, &v) in vw.iter().enumerate() {
            let q = out.p()[out.vertex_point[vi] as usize];
            if q[0] < 2.0 {
                let sv = (v - 100) as usize;
                assert!((0..24).contains(&sv), "{v}");
                assert_eq!(a.p()[a.vertex_point[sv] as usize], q, "vertex {vi}");
            } else {
                assert_eq!(v, 0, "B vertex {vi}");
            }
        }
    }
    /// Prim group `top` (prim 1 of A) and point group `pg` (A points 0,1) survive: 1 prim, 2 points; unrelated B points are not members.
    #[test]
    fn operand_groups_are_transferred_by_source_prim_and_coincident_point() {
        let (mut a, b) = (unit([0.0; 3]), unit([5.0, 0.0, 0.0]));
        a.groups.push(Group {
            name: "top".into(),
            class: AttrClass::Prim,
            mask: vec![false, true, false, false, false, false],
        });
        let mut pm = vec![false; 8];
        pm[0] = true;
        pm[1] = true;
        a.groups.push(Group { name: "pg".into(), class: AttrClass::Point, mask: pm });
        let out = bool_(&a, &b, OP_UNION);
        assert_eq!(group_count(&out, "top"), 1);
        let t = out.group(AttrClass::Prim, "top").expect("top");
        let i = t.mask.iter().position(|m| *m).expect("member");
        let ids = &out.vertex_point[out.prims[i].verts.start as usize..out.prims[i].verts.end as usize];
        let src = &a.vertex_point[a.prims[1].verts.start as usize..a.prims[1].verts.end as usize];
        let mut got: Vec<[f32; 3]> = ids.iter().map(|&p| out.p()[p as usize]).collect();
        let mut want: Vec<[f32; 3]> = src.iter().map(|&p| a.p()[p as usize]).collect();
        got.sort_by(|x, y| x.partial_cmp(y).unwrap());
        want.sort_by(|x, y| x.partial_cmp(y).unwrap());
        assert_eq!(got, want);
        let pg = out.group(AttrClass::Point, "pg").expect("pg");
        assert_eq!(pg.mask.iter().filter(|m| **m).count(), 2);
        for (k, _) in pg.mask.iter().enumerate().filter(|(_, m)| **m) {
            assert!(out.p()[k] == a.p()[0] || out.p()[k] == a.p()[1]);
        }
    }
    #[test]
    fn intersect_cube_cube_diagonal_is_the_half_cube() {
        let out = bool_(&unit([0.0; 3]), &unit([0.5; 3]), OP_INTERSECT);
        assert_eq!((out.point_count, out.prim_count()), (8, 6));
        let mut want = Vec::new();
        for x in [0, 500] {
            for y in [0, 500] {
                for z in [0, 500] {
                    want.push([x, y, z]);
                }
            }
        }
        assert_eq!(pts(&out), want);
        assert!(close(volume(&out), 0.125), "{}", volume(&out));
        assert_watertight(&out);
        // A's three faces and B's three faces contribute 3 each
        assert_eq!(group_count(&out, "seam"), 6);
    }

    #[test]
    fn cube_minus_nested_cube_proxy_for_a_low_res_sphere_is_a_hollow_12_prims_16_points() {
        let out = bool_(&unit([0.0; 3]), &cube([0.0; 3], [0.5; 3]), OP_SUBTRACT);
        assert_eq!((out.point_count, out.prim_count()), (16, 12));
        assert!(close(volume(&out), 1.0 - 0.125), "{}", volume(&out));
        assert_watertight(&out);
        assert_eq!((group_count(&out, "a_only"), group_count(&out, "b_only"), group_count(&out, "seam")), (6, 6, 0));
        // outer shell outward (+1), inner shell inward (-.125 of volume) : union & intersect with a nested cube
        let u = bool_(&unit([0.0; 3]), &cube([0.0; 3], [0.5; 3]), OP_UNION);
        assert_eq!((u.point_count, u.prim_count()), (8, 6));
        let i = bool_(&unit([0.0; 3]), &cube([0.0; 3], [0.5; 3]), OP_INTERSECT);
        assert_eq!((i.point_count, i.prim_count()), (8, 6));
        assert!(close(volume(&i), 0.125));
    }

    #[test]
    fn cube_minus_cube_poking_out_of_a_face_makes_a_notch_without_t_junctions() {
        // B = [.25,.75]x[-.25,.25]^2 pokes through A's +x face (a hole in the face: not fusable into one polygon)
        let b = cube([0.5, 0.0, 0.0], [0.5; 3]);
        let s = bool_(&unit([0.0; 3]), &b, OP_SUBTRACT);
        assert!(close(volume(&s), 1.0 - 0.0625), "{}", volume(&s));
        assert_watertight(&s);
        let u = bool_(&unit([0.0; 3]), &b, OP_UNION);
        assert!(close(volume(&u), 1.0 + 0.125 - 0.0625), "{}", volume(&u));
        assert_watertight(&u);
        let i = bool_(&unit([0.0; 3]), &b, OP_INTERSECT);
        assert!(close(volume(&i), 0.0625), "{}", volume(&i));
        assert_watertight(&i);
    }

    #[test]
    fn identical_cubes_union_and_intersect_are_the_cube_and_subtract_is_empty() {
        let a = unit([0.0; 3]);
        for op in [OP_UNION, OP_INTERSECT] {
            let o = bool_(&a, &a, op);
            assert_eq!((o.point_count, o.prim_count()), (8, 6), "op {op}");
            assert!(close(volume(&o), 1.0));
        }
        let s = bool_(&a, &a, OP_SUBTRACT);
        assert_eq!((s.point_count, s.prim_count()), (0, 0));
    }

    #[test]
    fn face_touching_cubes_union_drops_the_shared_face() {
        let (a, b) = (unit([0.0; 3]), unit([1.0, 0.0, 0.0]));
        let u = bool_(&a, &b, OP_UNION);
        // 12 side quads + 2 outer caps; the contact faces cancel. Points: 3 x-slices x 4.
        assert_eq!((u.point_count, u.prim_count()), (12, 10));
        assert!(close(volume(&u), 2.0));
        assert_watertight(&u);
        let i = bool_(&a, &b, OP_INTERSECT);
        assert_eq!((i.point_count, i.prim_count()), (0, 0));
        let s = bool_(&a, &b, OP_SUBTRACT);
        assert_eq!((s.point_count, s.prim_count()), (8, 6));
        assert!(close(volume(&s), 1.0));
    }

    #[test]
    fn disjoint_cubes() {
        let (a, b) = (unit([0.0; 3]), unit([5.0, 0.0, 0.0]));
        let u = bool_(&a, &b, OP_UNION);
        assert_eq!((u.point_count, u.prim_count()), (16, 12));
        assert_eq!((group_count(&u, "a_only"), group_count(&u, "b_only"), group_count(&u, "seam")), (6, 6, 0));
        assert_eq!(bool_(&a, &b, OP_INTERSECT).prim_count(), 0);
        let s = bool_(&a, &b, OP_SUBTRACT);
        assert_eq!((s.point_count, s.prim_count()), (8, 6));
    }

    #[test]
    fn concave_operand_l_prism_keeps_hexagon_caps() {
        // L prism (volume 3) beside a distant cube: 6 side quads + 2 hexagon caps (ear-clipped, then re-fused) + 6 = 14 prims, 12 + 8 points
        let out = bool_(&l_prism(), &unit([9.0, 9.0, 0.5]), OP_UNION);
        assert_eq!((out.point_count, out.prim_count()), (20, 14));
        assert_eq!(corners(&out).iter().filter(|&&c| c == 6).count(), 2);
        assert!(close(volume(&out), 4.0), "{}", volume(&out));
        assert_watertight(&out);
    }

    #[test]
    fn concave_operand_notch_filled_by_a_cube_gives_a_2x2_slab() {
        let fill = cube([1.5, 1.5, 0.5], [1.0; 3]);
        let u = bool_(&l_prism(), &fill, OP_UNION);
        assert!(close(volume(&u), 4.0), "{}", volume(&u));
        assert_watertight(&u);
        // a slab x[-.5,1.5] y[-.5,.5] z[0,1] (caps coplanar with the prism's) overlaps the L in x[0,1.5] y[0,.5]: .75
        let cut = cube([0.5, 0.0, 0.5], [2.0, 1.0, 1.0]);
        let s = bool_(&l_prism(), &cut, OP_SUBTRACT);
        assert!(close(volume(&s), 2.25), "{}", volume(&s));
        assert_watertight(&s);
        let i = bool_(&l_prism(), &cut, OP_INTERSECT);
        assert!(close(volume(&i), 0.75), "{}", volume(&i));
        assert_watertight(&i);
    }

    #[test]
    fn surface_mode_cuts_without_inside_removal() {
        let (a, b) = (unit([0.0; 3]), unit([0.5; 3]));
        // A.Out : 3 whole faces + 3 hexagons
        let s = run_with(&a, &b, OP_SUBTRACT, false).expect("cooks");
        assert_eq!(corners(&s), [4, 4, 4, 6, 6, 6]);
        // A.In : 3 squares
        let i = run_with(&a, &b, OP_INTERSECT, false).expect("cooks");
        assert_eq!(corners(&i), [4, 4, 4]);
        // every fragment of both: A 3 + 3*2, B 3 + 3*2
        let u = run_with(&a, &b, OP_UNION, false).expect("cooks");
        assert_eq!(u.prim_count(), 18);
    }

    #[test]
    fn empty_and_open_operands() {
        let a = Arc::new(unit([0.0; 3]));
        let e = Arc::new(Detail::default());
        let cook = |op, i: &[Arc<Detail>]| {
            let p = ParamSet::new().with("operation", ParamValue::Menu(op)).with("treat_as_solid", ParamValue::B(true));
            new().cook(&CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} }, i).expect("cooks")
        };
        assert!(Arc::ptr_eq(&cook(OP_UNION, &[Arc::clone(&a), Arc::clone(&e)]), &a));
        assert!(Arc::ptr_eq(&cook(OP_UNION, &[Arc::clone(&e), Arc::clone(&a)]), &a));
        assert!(Arc::ptr_eq(&cook(OP_SUBTRACT, &[Arc::clone(&a), Arc::clone(&e)]), &a));
        assert_eq!(cook(OP_SUBTRACT, &[Arc::clone(&e), Arc::clone(&a)]).prim_count(), 0);
        assert_eq!(cook(OP_INTERSECT, &[Arc::clone(&a), Arc::clone(&e)]).prim_count(), 0);
        assert_eq!(cook(OP_UNION, &[]).prim_count(), 0);
        // an open poly is not a solid
        let mut open = Detail::default();
        open.add_points(3);
        *open.p_mut() = vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        open.add_poly(&[0, 1, 2], false);
        assert!(Arc::ptr_eq(&cook(OP_UNION, &[Arc::clone(&a), Arc::new(open)]), &a));
    }

    #[test]
    fn hostile_params_nan_and_reversed_winding() {
        let a = unit([0.0; 3]);
        for op in [5u32, 99] {
            assert!(matches!(run_with(&a, &a, op, true), Err(CookError::BadParam(_))));
        }
        let missing = ParamSet::new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &missing, log: &|_| {} };
        assert!(matches!(new().cook(&ctx, &[Arc::new(a.clone())]), Err(CookError::BadParam(_))));
        let mut nan = unit([0.0; 3]);
        nan.p_mut()[3] = [f32::NAN, 0.0, 0.0];
        assert!(matches!(run_with(&a, &nan, OP_UNION, true), Err(CookError::Failed(_))));
        // degenerate prims (zero area) are ignored, huge coordinates do not panic
        let mut big = unit([0.0; 3]);
        big.p_mut()[0] = [f32::MAX, f32::MIN, 0.0];
        let _ = run_with(&a, &big, OP_UNION, true);
        let mut flat = Detail::default();
        flat.add_points(4);
        *flat.p_mut() = vec![[0.0; 3]; 4];
        flat.add_poly(&[0, 1, 2, 3], true);
        assert_eq!(
            run_with(&a, &flat, OP_UNION, true).expect("cooks").prim_count(),
            6,
            "zero-area poly is not a solid"
        );
    }

    #[test]
    fn many_small_cubes_finish_fast() {
        // 31 cubes (one overlapping the cutter) : BSP stays fast
        let mut a = unit([0.0; 3]);
        for k in 0..30 {
            a.merge(&cube([k as f32 * 0.01 + 3.0, 0.0, 0.0], [0.1; 3]));
        }
        let t = std::time::Instant::now();
        let out = bool_(&a, &unit([0.3, 0.0, 0.0]), OP_UNION);
        assert!(out.prim_count() > 0);
        assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
    }

    /// Spline curve / surface operands never fail or panic: they are tessellated; curves are open prims (ignored), a flat NURBS
    /// patch is an open surface, not a solid. Operation results stay polygon-only.
    #[test]
    fn nurbs_operands_are_tessellated_without_error() {
        use super::super::splinesurf::testkit::{nurbs, nurbs_circle, spline_curve};
        use hexe_geo::{Basis, PrimKind};
        let a = unit([0.0; 3]);
        let mut b = nurbs_circle();
        let mut cv = Vec::new();
        for r in 0..3 {
            for c in 0..3 {
                cv.push([c as f32 - 1.0, r as f32 - 1.0, 0.0]);
            }
        }
        b.merge(&spline_curve(
            &cv,
            PrimKind::NurbsSurf {
                cols: 3,
                rows: 3,
                ubasis: Basis::new(3),
                vbasis: Basis::new(3),
                uwrap: false,
                vwrap: false,
            },
        ));
        b.merge(&spline_curve(&[[0.0; 3], [1.0, 1.0, 0.0], [2.0, 0.0, 0.0]], nurbs(3)));
        for op in 0..=4 {
            for solid in [true, false] {
                let r = run_with(&a, &b, op, solid).unwrap_or_else(|e| panic!("op {op} solid {solid}: {e:?}"));
                assert!(!r.has_spline_prims(), "op {op}");
                assert!(r.vertex_point.iter().all(|&v| v < r.point_count));
            }
        }
        // curve-only B behaves exactly like an empty B: union = A
        let u = run_with(&a, &nurbs_circle(), 0, true).expect("union");
        assert_eq!((u.point_count, u.prim_count()), (8, 6));
        // spline operand A + cube B: union keeps B
        let v = run_with(&nurbs_circle(), &a, 0, true).expect("union");
        assert_eq!(v.prim_count(), 6);
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/boolean.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert_eq!((out.point_count, out.prim_count()), (14, 9), "box - offset box (diagonal)");
        assert_eq!(crate::registry().create("boolean").map(|n| n.type_name()), Some("boolean"));
    }

    /// stat-oracle vs `parity/ref/boolean/*.topo` (hython boolean::2.0 solid/solid): prim/point counts, prim valence histogram, bbox,
    /// surface area and the P multiset equal (1e-4). Prim ORDER / polygon start vertex are NOT compared (see ledger sop-boolean).
    #[test]
    fn houdini_stat_oracle_box_sphere_and_offset_boxes() {
        let refs: [(&str, &str); 9] = [
            ("sph_union", include_str!("../../../../parity/ref/boolean/sph_union.topo")),
            ("sph_sub", include_str!("../../../../parity/ref/boolean/sph_sub.topo")),
            ("sph_int", include_str!("../../../../parity/ref/boolean/sph_int.topo")),
            ("off_union", include_str!("../../../../parity/ref/boolean/off_union.topo")),
            ("off_sub", include_str!("../../../../parity/ref/boolean/off_sub.topo")),
            ("off_int", include_str!("../../../../parity/ref/boolean/off_int.topo")),
            ("offx_union", include_str!("../../../../parity/ref/boolean/offx_union.topo")),
            ("offx_sub", include_str!("../../../../parity/ref/boolean/offx_sub.topo")),
            ("offx_int", include_str!("../../../../parity/ref/boolean/offx_int.topo")),
        ];
        for (name, topo) in refs {
            let (b, op) = name.split_once('_').expect("name");
            let bsrc = match b {
                "sph" => r#"{"name":"b","type":"sphere","params":{"type":2,"rad":[0.65,0.65,0.65],"rows":9,"cols":12}},{"name":"xf","type":"xform"}"#,
                "off" => r#"{"name":"b","type":"box"},{"name":"xf","type":"xform","params":{"t":[0.5,0.5,0.5]}}"#,
                _ => r#"{"name":"b","type":"box"},{"name":"xf","type":"xform","params":{"t":[0.5,0,0]}}"#,
            };
            let opn = match op {
                "union" => 0,
                "sub" => 1,
                _ => 2,
            };
            let scene = format!(
                r#"{{"nodes":[{{"name":"a","type":"box"}},{bsrc},{{"name":"bool1","type":"boolean","params":{{"operation":{opn}}}}}],"edges":[["b",0,"xf",0],["a",0,"bool1",0],["xf",0,"bool1",1]],"display":"bool1"}}"#
            );
            let (mut g, id) = load_scene(&scene).expect("scene loads");
            let out = g.cook(id, 0.0).expect("cooks");
            // reference: P lines + F lines
            let mut rp: Vec<[f64; 3]> = Vec::new();
            let mut rf: Vec<Vec<usize>> = Vec::new();
            for l in topo.lines() {
                let t: Vec<&str> = l.split_whitespace().collect();
                match t.first() {
                    Some(&"P") => rp.push([t[2].parse().expect("x"), t[3].parse().expect("y"), t[4].parse().expect("z")]),
                    Some(&"F") => rf.push(t[2..].iter().map(|s| s.parse().expect("i")).collect()),
                    _ => {}
                }
            }
            assert_eq!((out.point_count as usize, out.prim_count() as usize), (rp.len(), rf.len()), "{name} counts");
            let xp: Vec<[f64; 3]> = out.p().iter().map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]).collect();
            let key = |p: &[f64; 3]| [(p[0] * 1e4).round() as i64, (p[1] * 1e4).round() as i64, (p[2] * 1e4).round() as i64];
            let (mut ka, mut kb): (Vec<_>, Vec<_>) = (rp.iter().map(key).collect(), xp.iter().map(key).collect());
            ka.sort_unstable();
            kb.sort_unstable();
            assert_eq!(ka, kb, "{name} P multiset");
            let hist = |v: Vec<usize>| {
                let mut h = std::collections::BTreeMap::new();
                for n in v {
                    *h.entry(n).or_insert(0usize) += 1;
                }
                h
            };
            let xf: Vec<Vec<usize>> = out.prims.iter().map(|pr| out.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&p| p as usize).collect()).collect();
            assert_eq!(hist(rf.iter().map(Vec::len).collect()), hist(xf.iter().map(Vec::len).collect()), "{name} valence");
            let area = |pts: &[[f64; 3]], f: &[Vec<usize>]| -> f64 {
                f.iter()
                    .map(|q| {
                        let mut n = [0.0f64; 3];
                        for i in 0..q.len() {
                            let (a, b) = (pts[q[i]], pts[q[(i + 1) % q.len()]]);
                            let c = cross(a, b);
                            n = [n[0] + c[0], n[1] + c[1], n[2] + c[2]];
                        }
                        len(n) / 2.0
                    })
                    .sum()
            };
            let (a0, a1) = (area(&rp, &rf), area(&xp, &xf));
            assert!((a0 - a1).abs() < 1e-3 * a0.max(1.0), "{name} area {a0} vs {a1}");
        }
    }
}
