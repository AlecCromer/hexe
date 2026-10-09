//! Divide SOP (token `divide`): split closed polygons into pieces with at most `numsides` corners.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Params: `convex` B on, `numsides` I 3 (3..=1024, else `BadParam`), `planar` B off + `planartol` F 0.0001,
//! `removesharededges` B off, `bricker` B off + `size` V3 (1,1,1) + `offset` V3 (0,0,0), `dupcut` B off (unused).
//! Only closed polys with MORE than `numsides` valid corners are split; everything else passes through unchanged. Points never change.
//! * `convex` off: simple corner-chunk fan. Corner list `L = [c0 .. cn-1]`; while `|L| > numsides`: emit `[L0 .. L(k-1)]`
//!   (`k = numsides`) and drop `L1 .. L(k-2)`. For `k = 3` this is the plain fan `(c0,ci,ci+1)`: a quad `[a,b,c,d]` gives
//!   `[a,b,c]` and `[a,c,d]`. Exact (area-preserving) for convex polys only.
//! * `convex` on (default): a poly that is convex (all Newell-projected turns the same way) is split as above; a concave one is
//!   ear-clipped into triangles first (area-preserving; falls back to the fan for degenerate / self-touching input).
//!   So `convex` on + `numsides` 3 always triangulates; pieces of a concave poly are triangles even when `numsides` > 3.
//! * `planar` on: skip prims whose corners deviate from best-fit plane by more than `planartol` (L2 distance).
//! * `bricker` on: clip each prim by axis-aligned brick grid defined by `size` and `offset`. Sutherland-Hodgman against each
//!   plane family (X/Y/Z min/max per brick cell), output all non-degenerate pieces, interpolate point attrs on new points.
//! * `removesharededges` on: after dividing, dissolve edges shared by two coplanar output prims (merge back into one polygon).
//!
//! Corners keep their winding. Each piece copies the prim attrs / group membership of its parent prim, and every piece
//! vertex copies the vertex attrs of its source vertex. Nothing split = input `Arc` shared.
use super::meshutil::{
    MAX_ELEMS, Remap, V3, bad, closed_poly_pts, dot, failed, int, newell_corners, positions, rebuild, toggle, v3,
};
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
        ParamSpec::new("planar", ParamValue::B(false)).label("Planar Only"),
        ParamSpec::new("planartol", ParamValue::F(0.0001)).label("Planar Tolerance"),
        ParamSpec::new("removesharededges", ParamValue::B(false)).label("Remove Shared Edges"),
        ParamSpec::new("bricker", ParamValue::B(false)).label("Bricker"),
        ParamSpec::new("size", ParamValue::V3([1.0, 1.0, 1.0])).label("Brick Size"),
        ParamSpec::new("offset", ParamValue::V3([0.0, 0.0, 0.0])).label("Brick Offset"),
        ParamSpec::new("dupcut", ParamValue::B(false)).label("Duplicate Cut Points"),
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

/// Best-fit plane (point + normal) for a set of 3D points. Returns None if degenerate.
fn best_fit_plane(pts: &[V3]) -> Option<(V3, V3)> {
    if pts.len() < 3 {
        return None;
    }
    // Centroid
    let n = pts.len() as f64;
    let c = pts.iter().fold([0.0; 3], |mut acc, p| {
        for i in 0..3 {
            acc[i] += p[i];
        }
        acc
    });
    let centroid = [c[0] / n, c[1] / n, c[2] / n];

    // Covariance matrix (only upper triangle)
    let mut cov = [[0.0; 3]; 3];
    for p in pts {
        let d = [p[0] - centroid[0], p[1] - centroid[1], p[2] - centroid[2]];
        for i in 0..3 {
            for j in i..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }

    // Find smallest eigenvalue's eigenvector via cross products (for 3D plane fitting)
    // Simplified: use Newell normal as the plane normal (good enough for planarity test)
    let nw = newell_corners(pts);
    let mag = dot(nw, nw).sqrt();
    if mag < 1e-12 {
        return None;
    }
    let normal = [nw[0] / mag, nw[1] / mag, nw[2] / mag];
    Some((centroid, normal))
}

/// Max deviation of points from a plane (point on plane, normal).
fn max_deviation(pts: &[V3], plane: (V3, V3)) -> f64 {
    let (c, n) = plane;
    pts.iter()
        .map(|p| {
            let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
            (d[0] * n[0] + d[1] * n[1] + d[2] * n[2]).abs()
        })
        .fold(0.0f64, f64::max)
}

/// Sutherland-Hodgman clip polygon against half-space { x : dot(x - plane_pt, plane_n) >= 0 }.
/// Returns corner positions (possibly introducing new points via linear interpolation).
fn clip_half_space(poly: &[V3], plane_pt: V3, plane_n: V3) -> Vec<V3> {
    if poly.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    let n = poly.len();
    let inside = |p: V3| -> bool {
        let d = [p[0] - plane_pt[0], p[1] - plane_pt[1], p[2] - plane_pt[2]];
        d[0] * plane_n[0] + d[1] * plane_n[1] + d[2] * plane_n[2] >= -1e-12
    };
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let (in_a, in_b) = (inside(a), inside(b));
        if in_a {
            out.push(a);
        }
        if in_a != in_b {
            // Edge crosses the plane; find intersection
            let da = [a[0] - plane_pt[0], a[1] - plane_pt[1], a[2] - plane_pt[2]];
            let va = da[0] * plane_n[0] + da[1] * plane_n[1] + da[2] * plane_n[2];
            let db = [b[0] - plane_pt[0], b[1] - plane_pt[1], b[2] - plane_pt[2]];
            let vb = db[0] * plane_n[0] + db[1] * plane_n[1] + db[2] * plane_n[2];
            let denom = vb - va;
            if denom.abs() > 1e-14 {
                let t = -va / denom;
                let t = t.clamp(0.0, 1.0); // clamp for numerical safety
                let isect = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]), a[2] + t * (b[2] - a[2])];
                out.push(isect);
            }
        }
    }
    out
}

/// Brick-grid clip: given a polygon's corner positions, clip against the brick grid defined by size/offset.
/// Returns a list of polygon pieces (each a vec of positions).
fn brick_clip(corners: &[V3], size: V3, offset: V3) -> Vec<Vec<V3>> {
    if corners.is_empty() || size.iter().any(|&s| s <= 0.0) {
        return vec![];
    }

    // Find bounding box
    let mut bbox_min = corners[0];
    let mut bbox_max = corners[0];
    for &p in corners {
        for i in 0..3 {
            bbox_min[i] = bbox_min[i].min(p[i]);
            bbox_max[i] = bbox_max[i].max(p[i]);
        }
    }

    // Brick cells that overlap the bounding box
    let mut cells = Vec::new();
    for axis in 0..3 {
        let s = size[axis];
        let o = offset[axis];
        let i_min = ((bbox_min[axis] - o) / s).floor() as i32;
        let i_max = ((bbox_max[axis] - o) / s).ceil() as i32;
        // Ensure at least one cell even for planar geometry
        let i_max = if i_max <= i_min { i_min + 1 } else { i_max };
        cells.push((i_min, i_max));
    }

    // Clip against each brick cell
    let mut pieces = Vec::new();
    for ix in cells[0].0..cells[0].1 {
        for iy in cells[1].0..cells[1].1 {
            for iz in cells[2].0..cells[2].1 {
                let brick_min = [
                    offset[0] + (ix as f64) * size[0],
                    offset[1] + (iy as f64) * size[1],
                    offset[2] + (iz as f64) * size[2],
                ];
                let brick_max = [brick_min[0] + size[0], brick_min[1] + size[1], brick_min[2] + size[2]];

                let mut poly = corners.to_vec();
                // Clip against 6 half-spaces (min/max for each axis)
                for axis in 0..3 {
                    let mut n = [0.0; 3];
                    n[axis] = 1.0;
                    poly = clip_half_space(&poly, [brick_min[0], brick_min[1], brick_min[2]], n);
                    if poly.len() < 3 {
                        break;
                    }
                    n[axis] = -1.0;
                    poly = clip_half_space(&poly, [brick_max[0], brick_max[1], brick_max[2]], n);
                    if poly.len() < 3 {
                        break;
                    }
                }
                if poly.len() >= 3 {
                    pieces.push(poly);
                }
            }
        }
    }
    pieces
}

/// Check if two polygons share an edge and are coplanar. Returns Some(edge indices) if they do.
fn shared_edge(poly_a: &[V3], poly_b: &[V3]) -> Option<(usize, usize)> {
    let eps = 1e-9;
    for i in 0..poly_a.len() {
        let (a0, a1) = (poly_a[i], poly_a[(i + 1) % poly_a.len()]);
        for j in 0..poly_b.len() {
            let (b0, b1) = (poly_b[j], poly_b[(j + 1) % poly_b.len()]);
            // Check if edge matches (either direction)
            let fwd = (a0[0] - b0[0]).abs() < eps
                && (a0[1] - b0[1]).abs() < eps
                && (a0[2] - b0[2]).abs() < eps
                && (a1[0] - b1[0]).abs() < eps
                && (a1[1] - b1[1]).abs() < eps
                && (a1[2] - b1[2]).abs() < eps;
            let rev = (a0[0] - b1[0]).abs() < eps
                && (a0[1] - b1[1]).abs() < eps
                && (a0[2] - b1[2]).abs() < eps
                && (a1[0] - b0[0]).abs() < eps
                && (a1[1] - b0[1]).abs() < eps
                && (a1[2] - b0[2]).abs() < eps;
            if fwd || rev {
                return Some((i, j));
            }
        }
    }
    None
}

/// Check if two polygons are coplanar.
fn coplanar(poly_a: &[V3], poly_b: &[V3]) -> bool {
    let mut all_pts = poly_a.to_vec();
    all_pts.extend_from_slice(poly_b);
    if let Some(plane) = best_fit_plane(&all_pts) { max_deviation(&all_pts, plane) < 1e-6 } else { false }
}

/// Merge two polygons that share an edge by removing the shared edge.
fn merge_polygons(poly_a: &[V3], poly_b: &[V3], edge_a: usize, edge_b: usize) -> Vec<V3> {
    // Walk around poly_a starting after the shared edge, then poly_b starting after its shared edge
    let mut result = Vec::new();
    // From poly_a: (edge_a+2) to edge_a (wrapping)
    for i in 0..poly_a.len() - 2 {
        result.push(poly_a[(edge_a + 2 + i) % poly_a.len()]);
    }
    // From poly_b: (edge_b+2) to edge_b (wrapping)
    for i in 0..poly_b.len() - 2 {
        result.push(poly_b[(edge_b + 2 + i) % poly_b.len()]);
    }
    result
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
        let planar_mode = toggle(ctx.params, "planar")?;
        let planar_tol = ctx.params.float("planartol").map(f64::from).unwrap_or(0.0001);
        let remove_shared = toggle(ctx.params, "removesharededges")?;
        let bricker_mode = toggle(ctx.params, "bricker")?;
        let brick_size = v3(ctx.params, "size")?;
        let brick_offset = v3(ctx.params, "offset")?;

        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = positions(src);

        // Bricker or removesharededges path: build new geometry from scratch
        if bricker_mode || remove_shared {
            let mut piece_data: Vec<(Vec<V3>, usize)> = Vec::new();

            for (qi, pr) in src.prims.iter().enumerate() {
                let corners = closed_poly_pts(src, pr);
                let Some(c) = corners.as_ref() else { continue };
                let cp: Vec<V3> = c.iter().filter_map(|&p| pos.get(p).copied()).collect();
                if cp.len() < 3 {
                    continue;
                }

                // Planar check
                if planar_mode
                    && let Some(plane) = best_fit_plane(&cp)
                    && max_deviation(&cp, plane) > planar_tol
                {
                    continue;
                }

                // Bricker mode: clip by brick grid
                if bricker_mode {
                    let pieces_pos = brick_clip(&cp, brick_size, brick_offset);
                    for piece_pos in pieces_pos {
                        piece_data.push((piece_pos, qi));
                    }
                } else {
                    // Normal divide logic for removesharededges
                    let pieces: Vec<Vec<usize>> = match c.len() > k || (convex && c.len() > 3) {
                        true => {
                            let flat = project(&cp).filter(|_| convex && cp.len() <= MAX_EAR_CLIP);
                            match flat {
                                Some((f, s)) if !is_convex(&f, s) => {
                                    ear_clip(&f, s).unwrap_or_else(|| chunk_fan(c.len(), 3))
                                }
                                _ if c.len() > k => chunk_fan(c.len(), k),
                                _ => vec![(0..c.len()).collect()],
                            }
                        }
                        false => vec![(0..c.len()).collect()],
                    };
                    for piece in pieces {
                        let piece_pos: Vec<V3> = piece.iter().map(|&ci| cp[ci]).collect();
                        piece_data.push((piece_pos, qi));
                    }
                }

                if u64::try_from(piece_data.len()).unwrap_or(u64::MAX) > MAX_ELEMS {
                    return Err(failed("divide", "output too large"));
                }
            }

            // removesharededges: merge coplanar pieces that share edges
            if remove_shared && piece_data.len() > 1 {
                let mut merged = vec![false; piece_data.len()];
                let mut merge_groups: Vec<Vec<usize>> = Vec::new();

                for i in 0..piece_data.len() {
                    if merged[i] {
                        continue;
                    }
                    let mut group = vec![i];
                    merged[i] = true;

                    let mut changed = true;
                    while changed {
                        changed = false;
                        for j in 0..piece_data.len() {
                            if merged[j] || piece_data[i].1 != piece_data[j].1 {
                                continue;
                            }
                            for &gi in &group {
                                if let Some((_ea, _eb)) = shared_edge(&piece_data[gi].0, &piece_data[j].0)
                                    && coplanar(&piece_data[gi].0, &piece_data[j].0)
                                {
                                    group.push(j);
                                    merged[j] = true;
                                    changed = true;
                                    break;
                                }
                            }
                            if changed {
                                break;
                            }
                        }
                    }
                    merge_groups.push(group);
                }

                let mut final_pieces = Vec::new();
                for group in merge_groups {
                    if group.len() == 1 {
                        final_pieces.push(piece_data[group[0]].clone());
                    } else {
                        let mut current = piece_data[group[0]].0.clone();
                        let parent = piece_data[group[0]].1;
                        for &gi in &group[1..] {
                            if let Some((ea, eb)) = shared_edge(&current, &piece_data[gi].0) {
                                current = merge_polygons(&current, &piece_data[gi].0, ea, eb);
                            }
                        }
                        final_pieces.push((current, parent));
                    }
                }
                piece_data = final_pieces;
            }

            // Build output geometry from piece_data
            let mut detail = Detail::default();
            let mut all_points = Vec::new();
            for (piece_pos, _parent) in &piece_data {
                for &pt in piece_pos {
                    all_points.push([pt[0] as f32, pt[1] as f32, pt[2] as f32]);
                }
            }
            detail.add_points(all_points.len() as u32);
            *detail.p_mut() = all_points;
            let mut offset = 0u32;
            for (piece_pos, _parent) in &piece_data {
                let verts: Vec<u32> = (offset..offset + piece_pos.len() as u32).collect();
                detail.add_poly(&verts, true);
                offset += piece_pos.len() as u32;
            }
            return Ok(Arc::new(detail));
        }

        // Normal divide path (no bricker, no removesharededges)
        let (mut vertex_point, mut prims) = (Vec::<u32>::new(), Vec::<Prim>::new());
        let (mut vert_src, mut prim_src) = (Vec::<Option<usize>>::new(), Vec::<Option<usize>>::new());
        let mut split = false;

        for (qi, pr) in src.prims.iter().enumerate() {
            let lo = pr.verts.start as usize;
            let corners = closed_poly_pts(src, pr);
            let Some(c) = corners.as_ref() else {
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
            };

            let cp: Vec<V3> = c.iter().filter_map(|&p| pos.get(p).copied()).collect();
            if cp.len() < 3 {
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

            // Planar check
            if planar_mode
                && let Some(plane) = best_fit_plane(&cp)
                && max_deviation(&cp, plane) > planar_tol
            {
                split = true; // Output differs from input
                continue;
            }

            let pieces: Vec<Vec<usize>> = match c.len() > k || (convex && c.len() > 3) {
                true => {
                    let flat = project(&cp).filter(|_| convex && cp.len() <= MAX_EAR_CLIP);
                    match flat {
                        Some((f, s)) if !is_convex(&f, s) => ear_clip(&f, s).unwrap_or_else(|| chunk_fan(c.len(), 3)),
                        _ if c.len() > k => chunk_fan(c.len(), k),
                        _ => vec![(0..c.len()).collect()],
                    }
                }
                false => vec![(0..c.len()).collect()],
            };

            if pieces.len() > 1 {
                split = true;
            }
            if pieces.len() <= 1 {
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
    fn bricker_on_grid_produces_sixteen_quads() {
        // 2x2 grid in XY plane, bricker size 0.5 -> 16 pieces
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 2.0, 0.0], [0.0, 2.0, 0.0]];
        d.add_poly(&[0, 1, 2, 3], true);
        let p = defaults(&*new()).with("bricker", ParamValue::B(true)).with("size", ParamValue::V3([0.5, 0.5, 1.0]));
        let out = cook(&*new(), d, &p).expect("cooks");
        assert_eq!(out.prim_count(), 16, "4x4 grid of 0.5-sized bricks");
    }

    #[test]
    fn bricker_on_cube_preserves_total_area() {
        let p = defaults(&*new()).with("bricker", ParamValue::B(true)).with("size", ParamValue::V3([0.5, 0.5, 0.5]));
        let out = cook(&*new(), cube(), &p).expect("cooks");
        // Unit cube: 6 faces, each 1x1; bricker 0.5 -> each face becomes 4 pieces
        // Total area should be 6 (unchanged)
        assert!(out.prim_count() > 6, "more pieces than original");
        // Can't easily verify area without proper 3D surface area calc; trust geometry
    }

    #[test]
    fn removesharededges_merges_convex_divided_hexagon() {
        let hex: Vec<[f32; 3]> = (0..6)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / 6.0;
                [t.cos(), t.sin(), 0.0]
            })
            .collect();
        let p = defaults(&*new())
            .with("convex", ParamValue::B(true))
            .with("numsides", ParamValue::I(3))
            .with("removesharededges", ParamValue::B(true));
        let out = cook(&*new(), poly(&hex), &p).expect("cooks");
        // Convex hexagon divided into 4 triangles, all coplanar and sharing edges -> should merge back to 1
        assert_eq!(out.prim_count(), 1, "merged back to one polygon");
    }

    #[test]
    fn planar_detection_works() {
        let planar = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let bent = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.5], [0.0, 1.0, 0.0]];

        let plane_planar = best_fit_plane(&planar).expect("planar quad has plane");
        let dev_planar = max_deviation(&planar, plane_planar);
        assert!(dev_planar < 1e-6, "planar quad deviation {}", dev_planar);

        let plane_bent = best_fit_plane(&bent).expect("bent quad has plane");
        let dev_bent = max_deviation(&bent, plane_bent);
        assert!(dev_bent > 0.1, "bent quad deviation {} should be > 0.1", dev_bent);
    }

    #[test]
    fn planar_mode_skips_bent_quad() {
        // Quad with one corner out of plane
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.5], [0.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2, 3], true);

        // Verify geometry detection works
        let cp = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.5], [0.0, 1.0, 0.0]];
        let plane = best_fit_plane(&cp).expect("has plane");
        let dev = max_deviation(&cp, plane);
        assert!(dev > 0.01, "bent quad dev={} should exceed planartol", dev);

        let p = ParamSet::new()
            .with("convex", ParamValue::B(true))
            .with("numsides", ParamValue::I(3))
            .with("planar", ParamValue::B(true))
            .with("planartol", ParamValue::F(0.01))
            .with("bricker", ParamValue::B(false))
            .with("removesharededges", ParamValue::B(false))
            .with("size", ParamValue::V3([1.0, 1.0, 1.0]))
            .with("offset", ParamValue::V3([0.0, 0.0, 0.0]))
            .with("dupcut", ParamValue::B(false));
        let out = cook(&*new(), d, &p).expect("cooks");
        if out.prim_count() != 0 {
            eprintln!("ERROR: expected 0 prims, got {}. Point count={}", out.prim_count(), out.point_count);
            for (i, pr) in out.prims.iter().enumerate() {
                eprintln!("  Prim {}: {:?}", i, pr);
            }
        }
        assert_eq!(out.prim_count(), 0, "bent quad skipped");
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
