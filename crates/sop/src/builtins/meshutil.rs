//! Shared helpers of the mesh-editing SOPs (`subdivide`, `smooth`, `peak`, `facet`): param access, f64 geometry,
//! point-normal averaging, and the attribute remap (`rebuild`) that carries every attribute / group onto a re-topologised
//! mesh. Not a SOP itself.
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, Group, Prim, PrimKind};
use hexe_graph::{CookError, ParamSet};

/// Output allocation cap per element class (AGENTS.md section 4).
pub(super) const MAX_ELEMS: u64 = 1 << 22;
/// Newell magnitudes at or below this are degenerate faces.
const MIN_NEWELL: f64 = 1e-12;
pub(super) type V3 = [f64; 3];

pub(super) fn bad(m: impl Into<String>) -> CookError {
    CookError::BadParam(m.into())
}
pub(super) fn failed(op: &str, m: &str) -> CookError {
    CookError::Failed(format!("{op}: {m}"))
}
pub(super) fn toggle(p: &ParamSet, n: &str) -> Result<bool, CookError> {
    p.toggle(n).ok_or_else(|| bad(format!("`{n}`: missing or not a toggle")))
}
pub(super) fn float(p: &ParamSet, n: &str) -> Result<f64, CookError> {
    match p.float(n) {
        Some(v) if v.is_finite() => Ok(f64::from(v)),
        _ => Err(bad(format!("`{n}`: missing or not a finite float"))),
    }
}
pub(super) fn int(p: &ParamSet, n: &str) -> Result<i32, CookError> {
    p.int(n).ok_or_else(|| bad(format!("`{n}`: missing or not an int")))
}
pub(super) fn menu(p: &ParamSet, n: &str) -> Result<u32, CookError> {
    p.menu(n).ok_or_else(|| bad(format!("`{n}`: missing or not a menu")))
}
pub(super) fn v3(p: &ParamSet, n: &str) -> Result<V3, CookError> {
    match p.vec3(n) {
        Some(v) if v.iter().all(|c| c.is_finite()) => Ok([f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]),
        _ => Err(bad(format!("`{n}`: missing or not a finite V3"))),
    }
}
pub(super) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(super) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(super) fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub(super) fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(super) fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
/// `P` as f64, one entry per point (missing entries are the origin).
pub(super) fn positions(d: &Detail) -> Vec<V3> {
    let p = d.p();
    (0..d.point_count as usize)
        .map(|i| p.get(i).map_or([0.0; 3], |q| [f64::from(q[0]), f64::from(q[1]), f64::from(q[2])]))
        .collect()
}
/// f64 -> f32 positions; `Failed` when a finite coordinate overflows f32.
pub(super) fn to_f32(op: &str, p: &[V3]) -> Result<Vec<[f32; 3]>, CookError> {
    let mut out = Vec::with_capacity(p.len());
    for q in p {
        let r = [q[0] as f32, q[1] as f32, q[2] as f32];
        if q.iter().all(|c| c.is_finite()) && r.iter().any(|c| !c.is_finite()) {
            return Err(failed(op, "position overflows f32"));
        }
        out.push(r);
    }
    Ok(out)
}
/// Point ids of a closed poly with >= 3 corners, all ids in range; `None` otherwise.
pub(super) fn closed_poly_pts(d: &Detail, prim: &Prim) -> Option<Vec<usize>> {
    if !matches!(prim.kind, PrimKind::Poly { closed: true }) {
        return None;
    }
    let (lo, hi) = (usize::try_from(prim.verts.start).ok()?, usize::try_from(prim.verts.end).ok()?);
    let np = d.point_count as usize;
    let pts: Vec<usize> = d
        .vertex_point
        .get(lo..hi)?
        .iter()
        .map(|&p| usize::try_from(p).ok().filter(|&p| p < np))
        .collect::<Option<_>>()?;
    (pts.len() >= 3).then_some(pts)
}
/// THE polygon-normal rule of hexe (H32): Houdini's. Houdini lists a front face CLOCKWISE seen from its front, so its prim
/// normal is the LEFT-hand rule of the corner order = the negated textbook (right-handed) Newell sum. Unnormalised
/// (length = 2 x area), computed relative to the first corner for precision. Every SOP that needs a face normal,
/// face orientation or signed volume goes through this module; no local cross-product copies.
pub(super) fn newell_corners(c: &[V3]) -> V3 {
    let Some(&o) = c.first() else { return [0.0; 3] };
    let mut nw = [0.0; 3];
    for (i, a) in c.iter().enumerate() {
        let b = c.get((i + 1) % c.len()).copied().unwrap_or(o);
        nw = sub(nw, cross(sub(*a, o), sub(b, o)));
    }
    nw
}
/// Unit Houdini normal of corner positions; `None` when degenerate / non-finite.
pub(super) fn unit_corners(c: &[V3]) -> Option<V3> {
    let nw = newell_corners(c);
    let l = dot(nw, nw).sqrt();
    (l.is_finite() && l > MIN_NEWELL).then(|| scale(nw, 1.0 / l))
}
/// Unit Houdini normal (`unit_corners`) of the polygon with point ids `pts`; `None` when degenerate / bad id.
pub(super) fn prim_normal(pos: &[V3], pts: &[usize]) -> Option<V3> {
    let c: Vec<V3> = pts.iter().map(|&i| pos.get(i).copied()).collect::<Option<_>>()?;
    unit_corners(&c)
}
/// Signed volume contribution of triangle `(a, b, c)` seen from the origin: POSITIVE when the triangle faces away from
/// the origin in Houdini's winding (outward-facing triangles of a closed mesh sum to its positive volume).
pub(super) fn tri_volume(a: V3, b: V3, c: V3) -> f64 {
    -dot(a, cross(b, c)) / 6.0
}
/// Uniform-averaged point normals: per point, normalise the plain sum of the unit Newell normals of the closed polys
/// that use it (every corner counts once). Points without a usable face get `[0; 3]`.
pub(super) fn point_normals(d: &Detail, pos: &[V3]) -> Vec<V3> {
    let mut acc = vec![[0.0f64; 3]; pos.len()];
    for pr in &d.prims {
        let Some(pts) = closed_poly_pts(d, pr) else { continue };
        let Some(n) = prim_normal(pos, &pts) else { continue };
        for &p in &pts {
            if let Some(a) = acc.get_mut(p) {
                *a = add(*a, n);
            }
        }
    }
    acc.into_iter()
        .map(|a| {
            let l = dot(a, a).sqrt();
            if l.is_finite() && l > MIN_NEWELL { scale(a, 1.0 / l) } else { [0.0; 3] }
        })
        .collect()
}
/// Prim-group selection (union of space-separated names); `None` = every prim.
pub(super) fn selection(op: &str, src: &Detail, names: &str) -> Result<Option<Vec<bool>>, CookError> {
    let names: Vec<&str> = names.split_whitespace().collect();
    if names.is_empty() {
        return Ok(None);
    }
    let mut sel = vec![false; src.prims.len()];
    for n in names {
        let g: &Group =
            src.group(AttrClass::Prim, n).ok_or_else(|| failed(op, &format!("unknown primitive group `{n}`")))?;
        for (s, m) in sel.iter_mut().zip(&g.mask) {
            *s |= *m;
        }
    }
    Ok(Some(sel))
}
/// Element `i` of every entry of `idx` (`None` -> default element). Total: short sources default-fill.
pub(super) fn gather(d: &AttrData, idx: &[Option<usize>]) -> AttrData {
    fn pick<T: Copy>(v: &[T], idx: &[Option<usize>], def: T) -> Vec<T> {
        idx.iter().map(|i| i.and_then(|i| v.get(i).copied()).unwrap_or(def)).collect()
    }
    fn pick_n<T: Copy>(v: &[T], size: u32, idx: &[Option<usize>], def: T) -> Vec<T> {
        let s = size as usize;
        let mut o = Vec::with_capacity(idx.len().saturating_mul(s));
        for i in idx {
            let chunk = i.and_then(|i| v.get(i.checked_mul(s)?..i.checked_add(1)?.checked_mul(s)?));
            match chunk {
                Some(c) => o.extend_from_slice(c),
                None => o.extend(std::iter::repeat_n(def, s)),
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
        AttrData::FN { size, data } => AttrData::FN { size: *size, data: pick_n(data, *size, idx, 0.0) },
        AttrData::IN { size, data } => AttrData::IN { size: *size, data: pick_n(data, *size, idx, 0) },
    }
}
/// Mean of `size`-wide float tuples over each source list (one source = exact copy; no valid source = zeros).
fn avg_flat(v: &[f32], size: usize, srcs: &[Vec<usize>]) -> Vec<f32> {
    let mut out = Vec::with_capacity(srcs.len().saturating_mul(size));
    for s in srcs {
        let rows: Vec<&[f32]> =
            s.iter().filter_map(|&i| v.get(i.checked_mul(size)?..i.checked_add(1)?.checked_mul(size)?)).collect();
        if let ([one], 1) = (rows.as_slice(), s.len()) {
            out.extend_from_slice(one);
            continue;
        }
        for j in 0..size {
            let sum: f64 = rows.iter().map(|r| r.get(j).copied().map_or(0.0, f64::from)).sum();
            out.push(if rows.is_empty() { 0.0 } else { (sum / rows.len() as f64) as f32 });
        }
    }
    out
}
/// New-point attribute values from source points: float types average the sources, int / string types copy the first.
pub(super) fn blend(d: &AttrData, srcs: &[Vec<usize>]) -> AttrData {
    let first: Vec<Option<usize>> = srcs.iter().map(|s| s.first().copied()).collect();
    let chunk = |f: Vec<f32>, n: usize| -> Vec<Vec<f32>> { f.chunks_exact(n.max(1)).map(<[f32]>::to_vec).collect() };
    match d {
        AttrData::F32(v) => AttrData::F32(avg_flat(v, 1, srcs)),
        AttrData::V2(v) => {
            let flat: Vec<f32> = v.iter().flatten().copied().collect();
            AttrData::V2(chunk(avg_flat(&flat, 2, srcs), 2).iter().map(|c| [c[0], c[1]]).collect())
        }
        AttrData::V3(v) => {
            let flat: Vec<f32> = v.iter().flatten().copied().collect();
            AttrData::V3(chunk(avg_flat(&flat, 3, srcs), 3).iter().map(|c| [c[0], c[1], c[2]]).collect())
        }
        AttrData::V4(v) => {
            let flat: Vec<f32> = v.iter().flatten().copied().collect();
            AttrData::V4(chunk(avg_flat(&flat, 4, srcs), 4).iter().map(|c| [c[0], c[1], c[2], c[3]]).collect())
        }
        AttrData::FN { size, data } => AttrData::FN { size: *size, data: avg_flat(data, *size as usize, srcs) },
        AttrData::I32(_) | AttrData::IN { .. } | AttrData::Str { .. } => gather(d, &first),
    }
}
/// Like `blend` but with explicit per-source weights (linear interpolation on cut points): floats = weighted mean,
/// int / string types copy the heaviest source.
pub(super) fn blend_weighted(d: &AttrData, srcs: &[Vec<(usize, f64)>]) -> AttrData {
    fn wavg(v: &[f32], size: usize, srcs: &[Vec<(usize, f64)>]) -> Vec<f32> {
        let size = size.max(1);
        let mut out = Vec::with_capacity(srcs.len().saturating_mul(size));
        for s in srcs {
            for j in 0..size {
                let (mut sum, mut ws) = (0.0f64, 0.0f64);
                for &(i, w) in s {
                    if let Some(x) = i.checked_mul(size).and_then(|b| v.get(b + j)) {
                        sum += w * f64::from(*x);
                        ws += w;
                    }
                }
                out.push(if ws > 0.0 { (sum / ws) as f32 } else { 0.0 });
            }
        }
        out
    }
    let flat2 = |v: &[[f32; 2]]| wavg(&v.iter().flatten().copied().collect::<Vec<f32>>(), 2, srcs);
    let flat3 = |v: &[[f32; 3]]| wavg(&v.iter().flatten().copied().collect::<Vec<f32>>(), 3, srcs);
    let flat4 = |v: &[[f32; 4]]| wavg(&v.iter().flatten().copied().collect::<Vec<f32>>(), 4, srcs);
    match d {
        AttrData::F32(v) => AttrData::F32(wavg(v, 1, srcs)),
        AttrData::V2(v) => AttrData::V2(flat2(v).as_chunks::<2>().0.to_vec()),
        AttrData::V3(v) => AttrData::V3(flat3(v).as_chunks::<3>().0.to_vec()),
        AttrData::V4(v) => AttrData::V4(flat4(v).as_chunks::<4>().0.to_vec()),
        AttrData::FN { size, data } => AttrData::FN { size: *size, data: wavg(data, *size as usize, srcs) },
        AttrData::I32(_) | AttrData::IN { .. } | AttrData::Str { .. } => {
            let heavy: Vec<Option<usize>> =
                srcs.iter().map(|s| s.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0)).collect();
            gather(d, &heavy)
        }
    }
}
/// How an output mesh derives from its source: per output point the source points it blends, per output vertex /
/// prim the source element it copies (`None` = default-filled).
#[derive(Default)]
pub(super) struct Remap {
    pub point_src: Vec<Vec<usize>>,
    pub vert_src: Vec<Option<usize>>,
    pub prim_src: Vec<Option<usize>>,
}
/// Output `Detail` from new topology + positions; every attribute and group of `src` follows `rm`.
pub(super) fn rebuild(
    op: &str,
    src: &Detail,
    vertex_point: Vec<u32>,
    prims: Vec<Prim>,
    rm: &Remap,
    p: Vec<[f32; 3]>,
) -> Result<Detail, CookError> {
    let point_count = u32::try_from(rm.point_src.len()).map_err(|_| failed(op, "point id overflow"))?;
    let mut out = Detail {
        point_count,
        vertex_point,
        prims,
        attrs: Vec::with_capacity(src.attrs.len()),
        groups: Vec::with_capacity(src.groups.len()),
        skipped_attribs: src.skipped_attribs.clone(),
    };
    for a in &src.attrs {
        let data = match a.class {
            AttrClass::Point => blend(&a.data, &rm.point_src),
            AttrClass::Vertex => gather(&a.data, &rm.vert_src),
            AttrClass::Prim => gather(&a.data, &rm.prim_src),
            AttrClass::Detail => a.data.clone(),
        };
        out.attrs.push(Attribute { name: a.name.clone(), class: a.class, data, type_info: a.type_info });
    }
    for g in &src.groups {
        let at = |i: Option<usize>| i.and_then(|i| g.mask.get(i).copied()).unwrap_or(false);
        let mask: Vec<bool> = match g.class {
            // a blended point is in the group only when every source point is
            AttrClass::Point => rm.point_src.iter().map(|s| !s.is_empty() && s.iter().all(|&i| at(Some(i)))).collect(),
            AttrClass::Vertex => rm.vert_src.iter().map(|&i| at(i)).collect(),
            AttrClass::Prim => rm.prim_src.iter().map(|&i| at(i)).collect(),
            AttrClass::Detail => continue,
        };
        out.groups.push(Group { name: g.name.clone(), class: g.class, mask });
    }
    *out.p_mut() = p;
    Ok(out)
}

#[cfg(test)]
pub(super) mod testkit {
    use crate::SopNode;
    use hexe_geo::Detail;
    use hexe_graph::{CookCtx, CookError, ParamSet};
    use std::sync::Arc;
    /// Node defaults as a `ParamSet`.
    pub fn defaults(n: &SopNode) -> ParamSet {
        n.params().iter().fold(ParamSet::new(), |p, s| p.with(&s.name, s.default.clone()))
    }
    pub fn cook_arc(n: &SopNode, d: &Arc<Detail>, p: &ParamSet) -> Result<Arc<Detail>, CookError> {
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: p, log: &|_| {} };
        n.cook(&ctx, std::slice::from_ref(d))
    }
    pub fn cook(n: &SopNode, d: Detail, p: &ParamSet) -> Result<Arc<Detail>, CookError> {
        cook_arc(n, &Arc::new(d), p)
    }
    /// Unit box (corners +-0.5, 8 shared points), 6 outward quads in Houdini winding (clockwise seen from outside).
    pub fn cube() -> Detail {
        let mut d = Detail::default();
        d.add_points(8);
        let c = |b: u32, i: u32| if (i >> b) & 1 == 1 { 0.5 } else { -0.5 };
        *d.p_mut() = (0..8).map(|i| [c(2, i), c(1, i), c(0, i)]).collect();
        for f in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
            d.add_poly(&f, true);
        }
        d
    }
    /// 3x3 POINTS (2x2 quads), z = 0, +Z (Houdini-rule) normals, points row-major.
    pub fn grid3() -> Detail {
        let mut d = Detail::default();
        d.add_points(9);
        *d.p_mut() = (0..9).map(|i| [(i % 3) as f32, (i / 3) as f32, 0.0]).collect();
        for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let a = r * 3 + c;
            d.add_poly(&[a, a + 3, a + 4, a + 1], true);
        }
        d
    }
    /// Noisy sphere for smoothing tests: UV sphere with noise added to radius.
    #[allow(dead_code)]
    pub fn noisy_sphere() -> Detail {
        let mut d = Detail::default();
        let rows = 8;
        let cols = 12;
        let np = rows * cols;
        d.add_points(np as u32);
        let noise = |i: usize| -> f32 { ((i * 7919) % 1000) as f32 / 1000.0 * 0.1 - 0.05 };
        *d.p_mut() = (0..np)
            .map(|i| {
                let r = i / cols;
                let c = i % cols;
                let theta = std::f32::consts::PI * r as f32 / (rows - 1) as f32;
                let phi = 2.0 * std::f32::consts::PI * c as f32 / cols as f32;
                let rad = 1.0 + noise(i);
                [rad * theta.sin() * phi.cos(), rad * theta.sin() * phi.sin(), rad * theta.cos()]
            })
            .collect();
        // Simple quad grid topology
        for r in 0..(rows - 1) {
            for c in 0..cols {
                let p0 = r * cols + c;
                let p1 = r * cols + (c + 1) % cols;
                let p2 = (r + 1) * cols + (c + 1) % cols;
                let p3 = (r + 1) * cols + c;
                d.add_poly(&[p0 as u32, p1 as u32, p2 as u32, p3 as u32], true);
            }
        }
        d
    }
    /// Houdini-twin evidence: cook the committed oracle scene and compare it with the REAL Houdini 20.5 `.geo` of the same case:
    /// point/prim counts, `P` in index order (1e-5), every prim's point-id list verbatim (+ closedness), attribute NAME sets per
    /// class and group names per class. Differences come back as `Err(text)` so a test can also pin a KNOWN gap.
    pub fn houdini_twin(scene: &str, houdini_geo: &[u8]) -> Result<(), String> {
        use hexe_geo::{AttrClass, PrimKind, read_geo};
        let h = read_geo(houdini_geo).map_err(|e| format!("houdini.geo: {e:?}"))?;
        let (mut g, id) = crate::load_scene(scene).map_err(|e| format!("scene: {e:?}"))?;
        let d = g.cook(id, 0.0).map_err(|e| format!("cook: {e:?}"))?;
        if (d.point_count, d.prim_count(), d.vertex_count()) != (h.point_count, h.prim_count(), h.vertex_count()) {
            return Err(format!(
                "counts hexe {}/{}/{} vs houdini {}/{}/{}",
                d.point_count,
                d.prim_count(),
                d.vertex_count(),
                h.point_count,
                h.prim_count(),
                h.vertex_count()
            ));
        }
        for (k, (a, b)) in d.p().iter().zip(h.p()).enumerate() {
            if a.iter().zip(b).any(|(x, y)| (x - y).abs() > 1e-5) {
                return Err(format!("P[{k}] hexe {a:?} vs houdini {b:?}"));
            }
        }
        let ids = |x: &Detail, i: usize| {
            x.vertex_point[x.prims[i].verts.start as usize..x.prims[i].verts.end as usize].to_vec()
        };
        for i in 0..d.prims.len() {
            if ids(&d, i) != ids(&h, i) {
                return Err(format!("prim {i} hexe {:?} vs houdini {:?}", ids(&d, i), ids(&h, i)));
            }
            if matches!(d.prims[i].kind, PrimKind::Poly { closed: true })
                != matches!(h.prims[i].kind, PrimKind::Poly { closed: true })
            {
                return Err(format!("prim {i} closedness differs"));
            }
        }
        for class in [AttrClass::Point, AttrClass::Vertex, AttrClass::Prim, AttrClass::Detail] {
            let names = |x: &Detail| {
                let mut v: Vec<String> = x.attrs.iter().filter(|a| a.class == class).map(|a| a.name.clone()).collect();
                v.sort();
                v
            };
            if names(&d) != names(&h) {
                return Err(format!("{class:?} attrs hexe {:?} vs houdini {:?}", names(&d), names(&h)));
            }
            let groups = |x: &Detail| {
                let mut v: Vec<String> = x.groups.iter().filter(|g| g.class == class).map(|g| g.name.clone()).collect();
                v.sort();
                v
            };
            if groups(&d) != groups(&h) {
                return Err(format!("{class:?} groups hexe {:?} vs houdini {:?}", groups(&d), groups(&h)));
            }
        }
        Ok(())
    }
    pub fn prim_pts(d: &Detail, i: usize) -> Vec<[f32; 3]> {
        let pr = &d.prims[i];
        d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&p| d.p()[p as usize]).collect()
    }
}
