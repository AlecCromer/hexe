//! Shared helpers of the surface generators (`sweep`, `revolve`, `skin`) and `convert` / `polyfill`: curve extraction from a
//! `Detail` and a small mesh builder that emits closed polys + a vertex `uv` (V3, TypeInfo::None). Not a SOP itself.
//!
//! Orientation rule used by every generator here: a generated quad ring/loft is wound so its Houdini normal (see
//! `meshutil::newell_corners`, left-hand rule) is `u x v` (ring direction x loft direction); closed shells come out
//! outward-facing (positive `tri_volume` sum).
use super::meshutil::{MAX_ELEMS, V3, failed};
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, PrimKind, TypeInfo};
use hexe_graph::CookError;

/// A curve = an ordered point-id list (all ids in range) + closed flag.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Curve {
    pub pts: Vec<u32>,
    pub closed: bool,
    /// Index of the source prim in `d.prims`.
    pub prim: usize,
}

/// Every `Poly` / `Polyline` prim with >= 2 in-range corners, in prim order. A `Polyline` whose last point repeats its first
/// (the convention `convert` writes for a closed loop) is reported closed without the repeated corner.
pub(super) fn curves(d: &Detail) -> Vec<Curve> {
    let mut out = Vec::new();
    for (prim, pr) in d.prims.iter().enumerate() {
        let closed_kind = match pr.kind {
            PrimKind::Poly { closed } => closed,
            PrimKind::Polyline => false,
            _ => continue,
        };
        let (Ok(lo), Ok(hi)) = (usize::try_from(pr.verts.start), usize::try_from(pr.verts.end)) else { continue };
        let Some(vs) = d.vertex_point.get(lo..hi) else { continue };
        if vs.len() < 2 || vs.iter().any(|&p| p >= d.point_count) {
            continue;
        }
        let mut pts = vs.to_vec();
        let mut closed = closed_kind;
        if matches!(pr.kind, PrimKind::Polyline) && pts.len() >= 4 && pts.first() == pts.last() {
            pts.pop();
            closed = true;
        }
        out.push(Curve { pts, closed, prim });
    }
    out
}

/// Output accumulator: f32 points, prims (corner ids, closed), per-corner uv.
#[derive(Default)]
pub(super) struct Mesh {
    pub pts: Vec<[f32; 3]>,
    prims: Vec<(Vec<u32>, bool)>,
    uvs: Vec<[f32; 3]>,
    /// Source point / prim stamped on every NEW point / prim (set by the caller before emitting; `None` = generated).
    pub cur_pt: Option<usize>,
    pub cur_prim: Option<usize>,
    point_src: Vec<Option<usize>>,
    prim_src: Vec<Option<usize>>,
}

impl Mesh {
    /// New point; `Failed` past the allocation cap or on f32 overflow.
    pub fn point(&mut self, op: &str, p: V3) -> Result<u32, CookError> {
        let r = [p[0] as f32, p[1] as f32, p[2] as f32];
        if !r.iter().all(|c| c.is_finite()) {
            return Err(failed(op, "non-finite or f32-overflowing position"));
        }
        if self.pts.len() as u64 >= MAX_ELEMS {
            return Err(failed(op, "output exceeds the point cap"));
        }
        self.pts.push(r);
        self.point_src.push(self.cur_pt);
        Ok((self.pts.len() - 1) as u32)
    }
    /// New closed poly with per-corner uv.
    pub fn poly(&mut self, op: &str, ids: &[u32], uv: &[[f32; 3]]) -> Result<(), CookError> {
        if self.prims.len() as u64 >= MAX_ELEMS || self.uvs.len() as u64 >= MAX_ELEMS {
            return Err(failed(op, "output exceeds the primitive cap"));
        }
        self.prims.push((ids.to_vec(), true));
        self.prim_src.push(self.cur_prim);
        self.uvs.extend_from_slice(uv);
        Ok(())
    }
    /// Quad (or triangle fan split when `tris`) over four corners with their uvs; consecutive duplicate corners collapse
    /// (revolve about an axis point), < 3 distinct corners = nothing.
    pub fn quad(&mut self, op: &str, c: [u32; 4], uv: [[f32; 3]; 4], tris: bool) -> Result<(), CookError> {
        let mut ids: Vec<u32> = Vec::with_capacity(4);
        let mut uvs: Vec<[f32; 3]> = Vec::with_capacity(4);
        for k in 0..4 {
            if ids.last() != Some(&c[k]) {
                ids.push(c[k]);
                uvs.push(uv[k]);
            }
        }
        if ids.len() > 1 && ids.first() == ids.last() {
            ids.pop();
            uvs.pop();
        }
        if ids.len() == 4 && tris {
            self.poly(op, &[ids[0], ids[1], ids[2]], &[uvs[0], uvs[1], uvs[2]])?;
            return self.poly(op, &[ids[0], ids[2], ids[3]], &[uvs[0], uvs[2], uvs[3]]);
        }
        if ids.len() >= 3 {
            self.poly(op, &ids, &uvs)?;
        }
        Ok(())
    }
    /// `finish`, then every point attr (except `P`) and prim attr of `src` is gathered onto the new points / prims through
    /// the recorded `cur_pt` / `cur_prim` stamps (generated elements get the default value). `uv` (vertex) is untouched.
    pub fn finish_with(self, src: &Detail) -> Detail {
        let (psrc, qsrc) = (self.point_src.clone(), self.prim_src.clone());
        let mut g = self.finish();
        for a in &src.attrs {
            let idx = match a.class {
                AttrClass::Point if a.name != "P" => &psrc,
                AttrClass::Prim => &qsrc,
                _ => continue,
            };
            g.add_attr(Attribute {
                name: a.name.clone(),
                class: a.class,
                data: super::meshutil::gather(&a.data, idx),
                type_info: a.type_info,
            });
        }
        g
    }
    /// Final `Detail`: points, closed polys, vertex `uv`.
    pub fn finish(self) -> Detail {
        let mut g = Detail::default();
        g.add_points(self.pts.len() as u32);
        *g.p_mut() = self.pts;
        for (ids, closed) in &self.prims {
            g.add_poly(ids, *closed);
        }
        if g.vertex_point.len() == self.uvs.len() && !self.uvs.is_empty() {
            g.attrs.push(Attribute {
                name: "uv".into(),
                class: AttrClass::Vertex,
                data: AttrData::V3(self.uvs),
                type_info: TypeInfo::None,
            });
        }
        g
    }
}

/// Cumulative arclength fraction per curve point (`0..=1`); a closed curve's last segment is counted in the total only.
pub(super) fn arc_fractions(pos: &[V3], closed: bool) -> Vec<f64> {
    let mut acc = vec![0.0; pos.len()];
    for i in 1..pos.len() {
        acc[i] = acc[i - 1] + dist(pos[i - 1], pos[i]);
    }
    let total = acc.last().copied().unwrap_or(0.0)
        + if closed && pos.len() > 1 { dist(pos[pos.len() - 1], pos[0]) } else { 0.0 };
    if total > 0.0 {
        acc.iter().map(|a| a / total).collect()
    } else {
        (0..pos.len()).map(|i| i as f64 / (pos.len().max(2) - 1) as f64).collect()
    }
}

pub(super) fn dist(a: V3, b: V3) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

pub(super) fn normalize(a: V3) -> Option<V3> {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    (l.is_finite() && l > 1e-12).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

#[cfg(test)]
pub(super) mod testkit {
    use super::super::meshutil::{positions, tri_volume};
    use hexe_geo::Detail;
    /// Sum of `tri_volume` over a fan of every closed poly: > 0 = outward-facing closed shell.
    pub fn signed_volume(d: &Detail) -> f64 {
        let pos = positions(d);
        let mut v = 0.0;
        for pr in &d.prims {
            let ids: Vec<usize> =
                d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&p| p as usize).collect();
            for k in 1..ids.len().saturating_sub(1) {
                v += tri_volume(pos[ids[0]], pos[ids[k]], pos[ids[k + 1]]);
            }
        }
        v
    }
    /// A polyline through `pts` (open) or ring (closed) as a Detail.
    pub fn curve(pts: &[[f32; 3]], closed: bool) -> Detail {
        let mut d = Detail::default();
        d.add_points(pts.len() as u32);
        *d.p_mut() = pts.to_vec();
        let ids: Vec<u32> = (0..pts.len() as u32).collect();
        d.add_poly(&ids, closed);
        d
    }
}
