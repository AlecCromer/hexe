//! Divide SOP (token `divide`): split closed polygons into pieces with at most `numsides` corners, or clip them to a brick grid.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Params: `convex` B on, `numsides` I 3 (3..=1024, else `BadParam`), `planar` B off + `planartol` F 0.0001,
//! `removesharededges` B off, `bricker` B off + `size` V3 (1,1,1) (all > 0, else `BadParam`) + `offset` V3 (0,0,0).
//! Only closed polys with MORE than `numsides` valid corners are split; everything else passes through unchanged. Points never
//! change in the non-bricker path.
//! * `convex` off: simple corner-chunk fan. Corner list `L = [c0 .. cn-1]`; while `|L| > numsides`: emit `[L0 .. L(k-1)]`
//!   (`k = numsides`) and drop `L1 .. L(k-2)`. For `k = 3` this is the plain fan `(c0,ci,ci+1)`: a quad `[a,b,c,d]` gives
//!   `[a,b,c]` and `[a,c,d]`. Exact (area-preserving) for convex polys only.
//! * `convex` on (default): a poly that is convex (all Newell-projected turns the same way) is split as above; a concave one is
//!   ear-clipped into triangles first (area-preserving; falls back to the fan for degenerate / self-touching input).
//!   So `convex` on + `numsides` 3 always triangulates; pieces of a concave poly are triangles even when `numsides` > 3.
//! * `planar` on: a prim whose corners deviate from their best-fit plane (Newell normal through the centroid) by more than
//!   `planartol` (max point-plane distance) is SKIPPED = passed through undivided / unclipped. Degenerate (no plane) prims are not skipped.
//! * `removesharededges` on: after dividing a prim, pieces of that SAME prim that share exactly one edge (opposite direction)
//!   and are coplanar (1e-5) are merged back (repeat to fixpoint); a prim that merges back to one piece passes through. No-op
//!   with `bricker` on (it would undo the bricks).
//! * `bricker` on: each closed poly is Sutherland-Hodgman sliced per axis into the slabs `offset + [i, i+1] * size`. Pieces =
//!   separate polys (winding kept). Outermost slab of a prim is unbounded outward (no sliver loss). Axes along which the poly is
//!   flat (< 1e-9) are not clipped. Cut points are new points (deduped across pieces by 1e-5 position grid) whose float point
//!   attrs are linearly interpolated (int / string copy the dominant source); original corners keep their point ids; a new vertex
//!   copies the vertex attrs of the source corner it follows.
//!
//! Each piece copies the prim attrs / group membership of its parent prim, and every piece vertex copies the vertex attrs of its
//! source vertex. Nothing split = input `Arc` shared.
//! NOT modelled (DEFERRED): `avoidsmallangles`, `usemaxsides` variants, polys with holes, `dupcut`, vertex-attr interpolation on cuts,
//! removesharededges across different source prims.
use super::meshutil::{
    MAX_ELEMS, Remap, V3, bad, blend_weighted, closed_poly_pts, dot, failed, float, int, newell_corners, positions,
    rebuild, toggle, v3,
};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::HashMap;
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const MAX_SIDES: i32 = 1024;
/// Ear clipping is O(n^2) per poly; longer polys use the fan.
const MAX_EAR_CLIP: usize = 2048;
/// Removing shared edges is O(n^3) per prim; prims split into more pieces keep them.
const MAX_MERGE_PIECES: usize = 256;
/// Half-space inside tolerance for the brick clip.
const CLIP_EPS: f64 = 1e-12;
/// Slab-index snap (in cell units) so f32 noise on a brick plane does not spawn a sliver cell.
const CELL_SNAP: f64 = 1e-6;
/// Merge coplanarity tolerance of `removesharededges`.
const COPLANAR_TOL: f64 = 1e-5;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("convex", ParamValue::B(true)).label("Convex Polygons"),
        ParamSpec::new("numsides", ParamValue::I(3)).label("Maximum Sides").range(3.0, 12.0),
        ParamSpec::new("planar", ParamValue::B(false)).label("Planar Only"),
        ParamSpec::new("planartol", ParamValue::F(0.0001)).label("Planar Tolerance"),
        ParamSpec::new("removesharededges", ParamValue::B(false)).label("Remove Shared Edges"),
        ParamSpec::new("bricker", ParamValue::B(false)).label("Bricker"),
        ParamSpec::new("size", ParamValue::V3([1.0, 1.0, 1.0])).label("Brick Size"),
        ParamSpec::new("offset", ParamValue::V3([0.0, 0.0, 0.0])).label("Brick Offset"),
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
    pts.first()?;
    let nw = newell_corners(pts); // Houdini sign: the projected loop is counter-clockwise where this is negative
    if dot(nw, nw) < 1e-24 {
        return None;
    }
    let ax = (0..3).max_by(|&a, &b| nw[a].abs().total_cmp(&nw[b].abs()))?;
    let (u, v) = ((ax + 1) % 3, (ax + 2) % 3); // right-handed pair about `ax`
    let flat = pts.iter().map(|p| [p[u], p[v]]).collect();
    Some((flat, if nw[ax] >= 0.0 { -1.0 } else { 1.0 }))
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

/// Best-fit plane (centroid, unit Newell normal). `None` for < 3 points or a degenerate (zero-area) loop.
fn best_fit_plane(pts: &[V3]) -> Option<(V3, V3)> {
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    let c = pts.iter().fold([0.0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]);
    let nw = newell_corners(pts);
    let mag = dot(nw, nw).sqrt();
    if mag.is_nan() || mag <= 1e-12 {
        return None;
    }
    Some(([c[0] / n, c[1] / n, c[2] / n], [nw[0] / mag, nw[1] / mag, nw[2] / mag]))
}

/// Max point-plane distance.
fn max_deviation(pts: &[V3], plane: (V3, V3)) -> f64 {
    let (c, n) = plane;
    pts.iter().map(|p| dot([p[0] - c[0], p[1] - c[1], p[2] - c[2]], n).abs()).fold(0.0, f64::max)
}

/// Merge pieces (corner-index lists) of one prim that share exactly one edge and are coplanar, to fixpoint.
fn merge_pieces(mut pieces: Vec<Vec<usize>>, cp: &[V3]) -> Vec<Vec<usize>> {
    if pieces.len() > MAX_MERGE_PIECES {
        return pieces;
    }
    // the one edge (i in a, j in b) with a[i]->a[i+1] == b[j+1]->b[j]; None when 0 or >= 2 shared edges
    let shared = |a: &[usize], b: &[usize]| -> Option<(usize, usize)> {
        let mut hit = None;
        for i in 0..a.len() {
            let (u, v) = (a[i], a[(i + 1) % a.len()]);
            for j in 0..b.len() {
                if b[j] == v && b[(j + 1) % b.len()] == u {
                    if hit.is_some() {
                        return None;
                    }
                    hit = Some((i, j));
                }
            }
        }
        hit
    };
    loop {
        let mut found = None;
        'scan: for x in 0..pieces.len() {
            for y in x + 1..pieces.len() {
                let Some((i, j)) = shared(&pieces[x], &pieces[y]) else { continue };
                let pts: Vec<V3> = pieces[x].iter().chain(&pieces[y]).filter_map(|&c| cp.get(c).copied()).collect();
                if best_fit_plane(&pts).is_some_and(|pl| max_deviation(&pts, pl) < COPLANAR_TOL) {
                    found = Some((x, y, i, j));
                    break 'scan;
                }
            }
        }
        let Some((x, y, i, j)) = found else { return pieces };
        let (a, b) = (&pieces[x], &pieces[y]);
        let mut m = Vec::with_capacity(a.len() + b.len() - 2);
        m.extend((1..=a.len()).map(|s| a[(i + s) % a.len()])); // a[i+1] .. a[i]
        m.extend((2..b.len()).map(|s| b[(j + s) % b.len()])); // b[j+2] .. b[j-1]
        pieces[x] = m;
        pieces.remove(y);
    }
}

/// One polygon corner during brick clipping: position, source-point weights, nearest source vertex, original point id (if uncut).
#[derive(Clone)]
struct BCorner {
    pos: V3,
    w: Vec<(usize, f64)>,
    vsrc: usize,
    pt: Option<u32>,
}

fn lerp_w(a: &[(usize, f64)], b: &[(usize, f64)], t: f64) -> Vec<(usize, f64)> {
    let mut out: Vec<(usize, f64)> = Vec::with_capacity(a.len() + b.len());
    let scaled = a.iter().map(|&(i, w)| (i, w * (1.0 - t))).chain(b.iter().map(|&(i, w)| (i, w * t)));
    for (i, w) in scaled {
        match out.iter_mut().find(|e| e.0 == i) {
            Some(e) => e.1 += w,
            None => out.push((i, w)),
        }
    }
    out
}

/// Drop cyclically consecutive duplicate corners (prefer the one that is an original point).
fn dedup(poly: Vec<BCorner>) -> Vec<BCorner> {
    let same = |a: &BCorner, b: &BCorner| (0..3).all(|k| (a.pos[k] - b.pos[k]).abs() < 1e-9);
    let mut out: Vec<BCorner> = Vec::with_capacity(poly.len());
    for c in poly {
        match out.last_mut() {
            Some(l) if same(l, &c) => {
                if l.pt.is_none() && c.pt.is_some() {
                    *l = c;
                }
            }
            _ => out.push(c),
        }
    }
    while out.len() > 1 && same(&out[0], &out[out.len() - 1]) {
        let last = out.pop();
        if let (Some(l), true) = (last, out[0].pt.is_none())
            && l.pt.is_some()
        {
            out[0] = l;
        }
    }
    out
}

/// Sutherland-Hodgman against one axis-aligned half-space: keep `pos[axis] >= val` (or `<=` when `!keep_ge`).
fn clip_axis(poly: &[BCorner], axis: usize, val: f64, keep_ge: bool) -> Vec<BCorner> {
    let sg = if keep_ge { 1.0 } else { -1.0 };
    let d = |c: &BCorner| (c.pos[axis] - val) * sg;
    let n = poly.len();
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..n {
        let (a, b) = (&poly[i], &poly[(i + 1) % n]);
        let (da, db) = (d(a), d(b));
        let (ia, ib) = (da >= -CLIP_EPS, db >= -CLIP_EPS);
        if ia {
            out.push(a.clone());
        }
        if ia != ib {
            let t = (da / (da - db)).clamp(0.0, 1.0);
            let mut pos: V3 = std::array::from_fn(|k| a.pos[k] + t * (b.pos[k] - a.pos[k]));
            pos[axis] = val;
            out.push(BCorner { pos, w: lerp_w(&a.w, &b.w, t), vsrc: a.vsrc, pt: None });
        }
    }
    dedup(out)
}

/// Slice one poly into the slabs `offset + [i, i+1] * size` along `axis`; non-degenerate pieces, in slab order.
fn slab(poly: Vec<BCorner>, axis: usize, size: f64, offset: f64) -> Result<Vec<Vec<BCorner>>, CookError> {
    let (mn, mx) = poly.iter().fold((f64::MAX, f64::MIN), |(lo, hi), c| (lo.min(c.pos[axis]), hi.max(c.pos[axis])));
    if mx - mn < 1e-9 {
        return Ok(vec![poly]); // flat along this axis: nothing to cut
    }
    let (f0, f1) = ((mn - offset) / size + CELL_SNAP, (mx - offset) / size - CELL_SNAP);
    if (f1 - f0).is_nan() || f1 - f0 >= MAX_ELEMS as f64 {
        return Err(failed("divide", "brick grid too fine (output too large)"));
    }
    let (i0, i1) = (f0.floor() as i64, (f1.ceil() as i64).max(f0.floor() as i64 + 1));
    let mut out = Vec::new();
    for i in i0..i1 {
        let mut piece = poly.clone();
        if i > i0 {
            piece = clip_axis(&piece, axis, offset + i as f64 * size, true);
        }
        if i + 1 < i1 && piece.len() >= 3 {
            piece = clip_axis(&piece, axis, offset + (i + 1) as f64 * size, false);
        }
        if piece.len() >= 3 {
            let ps: Vec<V3> = piece.iter().map(|c| c.pos).collect();
            let nw = newell_corners(&ps);
            if dot(nw, nw) > 1e-24 {
                out.push(piece);
            }
        }
        if out.len() as u64 > MAX_ELEMS {
            return Err(failed("divide", "output too large"));
        }
    }
    Ok(out)
}

fn brick_pieces(corners: Vec<BCorner>, size: V3, offset: V3) -> Result<Vec<Vec<BCorner>>, CookError> {
    let mut cur = vec![corners];
    for axis in 0..3 {
        let mut next = Vec::new();
        for poly in cur {
            next.extend(slab(poly, axis, size[axis], offset[axis])?);
        }
        if next.len() as u64 > MAX_ELEMS {
            return Err(failed("divide", "output too large"));
        }
        cur = next;
    }
    Ok(cur)
}

/// Output topology under construction: original points keep their ids, cut points are appended (deduped by position).
struct Out {
    vertex_point: Vec<u32>,
    prims: Vec<Prim>,
    vert_src: Vec<Option<usize>>,
    prim_src: Vec<Option<usize>>,
    newp: Vec<[f32; 3]>,
    neww: Vec<Vec<(usize, f64)>>,
    map: HashMap<[i64; 3], u32>,
    base: u32,
}
impl Out {
    fn new(base: u32) -> Self {
        Out {
            vertex_point: Vec::new(),
            prims: Vec::new(),
            vert_src: Vec::new(),
            prim_src: Vec::new(),
            newp: Vec::new(),
            neww: Vec::new(),
            map: HashMap::new(),
            base,
        }
    }
    fn len_u32(&self) -> Result<u32, CookError> {
        u32::try_from(self.vertex_point.len()).map_err(|_| failed("divide", "vertex id overflow"))
    }
    /// Original prim copied verbatim (invalid prims keep their vertex ids as they were).
    fn pass(&mut self, src: &Detail, qi: usize, pr: &Prim) -> Result<(), CookError> {
        let start = self.len_u32()?;
        for v in pr.verts.start as usize..pr.verts.end as usize {
            self.vertex_point.push(src.vertex_point.get(v).copied().unwrap_or(0));
            self.vert_src.push(Some(v));
        }
        let end = self.len_u32()?;
        self.prims.push(Prim { kind: pr.kind.clone(), verts: start..end });
        self.prim_src.push(Some(qi));
        Ok(())
    }
    /// New closed poly of `(point id, source vertex)` corners, child of source prim `qi`.
    fn emit(&mut self, qi: usize, corners: &[(u32, usize)]) -> Result<(), CookError> {
        let start = self.len_u32()?;
        for &(p, v) in corners {
            self.vertex_point.push(p);
            self.vert_src.push(Some(v));
        }
        let end = self.len_u32()?;
        self.prims.push(Prim { kind: PrimKind::Poly { closed: true }, verts: start..end });
        self.prim_src.push(Some(qi));
        if self.vertex_point.len() as u64 > MAX_ELEMS {
            return Err(failed("divide", "output too large"));
        }
        Ok(())
    }
    /// Point id of a clipped corner: the original id when uncut, else a (deduped) new point.
    fn point_of(&mut self, c: &BCorner) -> Result<u32, CookError> {
        if let Some(p) = c.pt {
            return Ok(p);
        }
        let key = [(c.pos[0] * 1e5).round() as i64, (c.pos[1] * 1e5).round() as i64, (c.pos[2] * 1e5).round() as i64];
        if let Some(&id) = self.map.get(&key) {
            return Ok(id);
        }
        let id = u32::try_from(self.newp.len())
            .ok()
            .and_then(|n| n.checked_add(self.base))
            .ok_or_else(|| failed("divide", "point id overflow"))?;
        self.newp.push([c.pos[0] as f32, c.pos[1] as f32, c.pos[2] as f32]);
        self.neww.push(c.w.clone());
        self.map.insert(key, id);
        Ok(id)
    }
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
        let planar = toggle(ctx.params, "planar")?;
        let planartol = float(ctx.params, "planartol")?;
        if planartol < 0.0 {
            return Err(bad(format!("divide: `planartol` {planartol} must be >= 0")));
        }
        let remove_shared = toggle(ctx.params, "removesharededges")?;
        let bricker = toggle(ctx.params, "bricker")?;
        let (size, offset) = (v3(ctx.params, "size")?, v3(ctx.params, "offset")?);
        if bricker && size.iter().any(|&s| s.is_nan() || s <= 0.0) {
            return Err(bad(format!("divide: brick `size` {size:?} must be > 0 on every axis")));
        }
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = positions(src);
        let mut o = Out::new(src.point_count);
        let mut split = false;
        for (qi, pr) in src.prims.iter().enumerate() {
            let lo = pr.verts.start as usize;
            let Some(c) = closed_poly_pts(src, pr) else {
                o.pass(src, qi, pr)?;
                continue;
            };
            let cp: Vec<V3> = c.iter().filter_map(|&p| pos.get(p).copied()).collect();
            if cp.len() != c.len() {
                o.pass(src, qi, pr)?;
                continue;
            }
            if planar && best_fit_plane(&cp).is_some_and(|pl| max_deviation(&cp, pl) > planartol) {
                o.pass(src, qi, pr)?;
                continue;
            }
            let corner = |ci: usize| (u32::try_from(c[ci]).unwrap_or(0), lo + ci);
            if bricker {
                let start: Vec<BCorner> = (0..c.len())
                    .map(|ci| BCorner { pos: cp[ci], w: vec![(c[ci], 1.0)], vsrc: lo + ci, pt: Some(corner(ci).0) })
                    .collect();
                let pieces = brick_pieces(start, size, offset)?;
                if pieces.len() <= 1 {
                    o.pass(src, qi, pr)?;
                    continue;
                }
                split = true;
                for piece in &pieces {
                    let mut cs = Vec::with_capacity(piece.len());
                    for bc in piece {
                        cs.push((o.point_of(bc)?, bc.vsrc));
                    }
                    o.emit(qi, &cs)?;
                }
                continue;
            }
            let mut pieces: Vec<Vec<usize>> = if c.len() > k || (convex && c.len() > 3) {
                let flat = project(&cp).filter(|_| convex && cp.len() <= MAX_EAR_CLIP);
                match flat {
                    Some((f, s)) if !is_convex(&f, s) => ear_clip(&f, s).unwrap_or_else(|| chunk_fan(c.len(), 3)),
                    _ if c.len() > k => chunk_fan(c.len(), k),
                    _ => vec![(0..c.len()).collect()],
                }
            } else {
                vec![(0..c.len()).collect()]
            };
            if remove_shared && pieces.len() > 1 {
                pieces = merge_pieces(pieces, &cp);
            }
            if pieces.len() <= 1 {
                o.pass(src, qi, pr)?;
                continue;
            }
            split = true;
            for piece in pieces {
                let cs: Vec<(u32, usize)> = piece.into_iter().map(corner).collect();
                o.emit(qi, &cs)?;
            }
        }
        if !split {
            return Ok(Arc::clone(src));
        }
        let n_src = pos.len();
        let mut p = src.p().to_vec();
        p.extend_from_slice(&o.newp);
        let mut weights: Vec<Vec<(usize, f64)>> = (0..n_src).map(|i| vec![(i, 1.0)]).collect();
        weights.extend(o.neww.iter().cloned());
        let rm = Remap {
            point_src: weights.iter().map(|w| w.iter().map(|x| x.0).collect()).collect(),
            vert_src: o.vert_src,
            prim_src: o.prim_src,
        };
        let mut out = rebuild("divide", src, o.vertex_point, o.prims, &rm, p)?;
        if !o.newp.is_empty() {
            for a in out.attrs.iter_mut().filter(|a| a.class == AttrClass::Point) {
                if let Some(sa) = src.attrs.iter().find(|s| s.name == a.name && s.class == AttrClass::Point) {
                    a.data = blend_weighted(&sa.data, &weights);
                }
            }
        }
        Ok(Arc::new(out))
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

    /// Flat quad (0,0)-(w,h) in XY with a point attr `t` = x coordinate.
    fn rect(w: f32, h: f32) -> Detail {
        let mut d = poly(&[[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, h, 0.0], [0.0, h, 0.0]]);
        d.add_attr(Attribute {
            name: "t".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![0.0, w, w, 0.0]),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "pid".into(),
            class: AttrClass::Prim,
            data: AttrData::I32(vec![7]),
            type_info: TypeInfo::None,
        });
        d
    }
    fn brick(size: [f32; 3]) -> ParamSet {
        defaults(&*new()).with("bricker", ParamValue::B(true)).with("size", ParamValue::V3(size))
    }
    fn area3(d: &Detail) -> f64 {
        let pos = positions(d);
        d.prims
            .iter()
            .map(|pr| {
                let pts: Vec<V3> = closed_poly_pts(d, pr).expect("closed").iter().map(|&p| pos[p]).collect();
                let nw = newell_corners(&pts);
                dot(nw, nw).sqrt() / 2.0
            })
            .sum()
    }

    #[test]
    fn bricker_on_grid_produces_sixteen_quads() {
        let out = cook(&*new(), rect(2.0, 2.0), &brick([0.5, 0.5, 0.5])).expect("cooks");
        assert_eq!(out.prim_count(), 16, "4x4 grid of 0.5 bricks");
        assert!(out.prims.iter().all(|p| p.verts.len() == 4), "all quads");
        assert_eq!(out.point_count, 25, "cut points shared between neighbouring pieces");
        assert!((area3(&out) - 4.0).abs() < 1e-5);
        // prim attr carried to every piece
        let Some(AttrData::I32(pid)) = out.attr(AttrClass::Prim, "pid").map(|a| &a.data) else { panic!("pid") };
        assert_eq!(pid, &vec![7; 16]);
    }

    #[test]
    fn bricker_interpolates_point_attrs_on_cut_points() {
        let out = cook(&*new(), rect(2.0, 2.0), &brick([0.7, 0.7, 1.0])).expect("cooks");
        let Some(AttrData::F32(t)) = out.attr(AttrClass::Point, "t").map(|a| &a.data) else { panic!("t") };
        let p = out.p();
        assert!(out.point_count > 4 && t.len() == p.len());
        for (i, pt) in p.iter().enumerate() {
            assert!((t[i] - pt[0]).abs() < 1e-5, "t[{i}]={} vs x={}", t[i], pt[0]);
        }
    }

    #[test]
    fn bricker_on_cube_preserves_total_area() {
        for size in [[0.4, 0.4, 0.4], [0.3, 0.7, 0.45]] {
            let p = brick(size).with("offset", ParamValue::V3([0.05, -0.1, 0.2]));
            let out = cook(&*new(), cube(), &p).expect("cooks");
            assert!(out.prim_count() > 6, "more pieces than original");
            assert!((area3(&out) - 6.0).abs() < 1e-5, "area {}", area3(&out));
        }
    }

    #[test]
    fn bricker_offset_shifts_the_grid_and_bad_size_errors() {
        let p = brick([1.0, 1.0, 1.0]).with("offset", ParamValue::V3([0.5, 0.5, 0.0]));
        assert_eq!(cook(&*new(), rect(1.0, 1.0), &p).expect("cooks").prim_count(), 4);
        for s in [[0.0, 1.0, 1.0], [1.0, -1.0, 1.0], [1.0, f32::NAN, 1.0]] {
            assert!(matches!(cook(&*new(), rect(1.0, 1.0), &brick(s)), Err(CookError::BadParam(_))), "{s:?}");
        }
        // a poly inside one brick is passed through (Arc shared)
        let a = Arc::new(rect(0.5, 0.5));
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &brick([1.0, 1.0, 1.0])).expect("cooks"), &a));
    }

    fn hexagon() -> Detail {
        let hex: Vec<[f32; 3]> = (0..6)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / 6.0;
                [t.cos(), t.sin(), 0.0]
            })
            .collect();
        poly(&hex)
    }

    #[test]
    fn removesharededges_merges_convex_divided_hexagon() {
        let off = with(true, 3);
        assert_eq!(cook(&*new(), hexagon(), &off).expect("cooks").prim_count(), 4, "divided without the toggle");
        let p = off.with("removesharededges", ParamValue::B(true));
        let out = cook(&*new(), hexagon(), &p).expect("cooks");
        assert_eq!(out.prim_count(), 1, "merged back to one polygon");
        assert_eq!(out.prims[0].verts.len(), 6);
        assert!((area_xy(&out, 0).abs() - 2.598_076).abs() < 1e-4);
        // concave L (ear-clipped into 4 tris) merges back to one 6-gon
        let l = cook(&*new(), l_shape(), &p).expect("cooks");
        assert_eq!((l.prim_count(), l.prims[0].verts.len()), (1, 6));
        // a cube's coplanar triangle pairs merge back to its 6 quads
        assert_eq!(cook(&*new(), cube(), &p).expect("cooks").prim_count(), 6);
    }

    #[test]
    fn removesharededges_keeps_non_coplanar_pieces() {
        let bent = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.5], [0.0, 1.0, 0.0]];
        let p = with(true, 3).with("removesharededges", ParamValue::B(true));
        assert_eq!(cook(&*new(), poly(&bent), &p).expect("cooks").prim_count(), 2);
    }

    #[test]
    fn planar_mode_skips_bent_quad() {
        let bent = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.5], [0.0, 1.0, 0.0]];
        let flat = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let on = with(true, 3).with("planar", ParamValue::B(true));
        // bent quad skipped = left as one quad (Arc shared); flat quad still divided
        let a = Arc::new(poly(&bent));
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &on).expect("cooks"), &a));
        assert_eq!(cook(&*new(), poly(&flat), &on).expect("cooks").prim_count(), 2);
        // without the toggle the bent quad is divided; a loose tolerance divides it too
        assert_eq!(cook(&*new(), poly(&bent), &with(true, 3)).expect("cooks").prim_count(), 2);
        let loose = on.clone().with("planartol", ParamValue::F(1.0));
        assert_eq!(cook(&*new(), poly(&bent), &loose).expect("cooks").prim_count(), 2);
        let neg = on.with("planartol", ParamValue::F(-1.0));
        assert!(matches!(cook(&*new(), poly(&flat), &neg), Err(CookError::BadParam(_))));
        // planar also skips bricker clipping of a bent prim
        let pb = brick([0.5, 0.5, 0.5]).with("planar", ParamValue::B(true));
        let a = Arc::new(poly(&bent));
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &pb).expect("cooks"), &a));
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/divide.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        // Sphere + bricker 0.4 -> many pieces
        assert!(d.point_count > 100 && d.prim_count() > 100, "bricker should produce many pieces");
        assert_eq!(crate::registry().create("divide").map(|n| n.type_name()), Some("divide"));
    }
}
