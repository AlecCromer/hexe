//! PolyExtrude SOP (token `polyextrude`), minimal "individual" mode: every selected closed polygon is extruded on its own.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Per selected prim with `n >= 3` corners and a non-degenerate Newell normal `N` (unit):
//! * ring `k` (`k = 0..=divisions`) = `p_c + (k/divisions) * (inset_c + N * distance)`; ring 0 is the original corner set;
//! * `inset_c` = mitre offset moving the corner so both adjacent edges shift INWARD (toward the centroid) by `inset`
//!   (unit square, inset 0.25 -> corners at 0.25 / 0.75). Applied to the whole extrusion path, i.e. before `distance`;
//! * `output_front` (on): the top polygon on ring `divisions`, original winding;
//! * `output_side` (on): `n * divisions` quads `(r[k-1][c], r[k-1][c+1], r[k][c+1], r[k][c])`, outward normals;
//! * `output_back` (off): the original face on ring 0, REVERSED winding.
//!
//! Order: output prims keep input order, an extruded prim is replaced IN PLACE by `front, sides (layer-major), back`.
//! No welding: each extruded prim owns `n * (divisions + 1)` fresh points (box 6 quads, 1 division -> 48 points). Input
//! points used ONLY by extruded prims are dropped; every other input point (passthrough prims, orphans) is kept in order.
//! Attributes: new points copy the source corner's point attrs; new prims copy the source prim's prim attrs; front/back
//! vertices copy the source vertex attrs, side vertices are default-filled; groups follow the same maps. Detail attrs kept.
//!
//! `group`: space-separated PRIM group names (union); empty = all prims. Unknown name -> `CookError::Failed`.
//! Not selected / not a closed poly / fewer than 3 corners / zero or non-finite area / bad point ids -> passthrough.
//! No selected prim extrudes -> the input `Arc` is shared. Hostile: `divisions < 1`, non-finite `distance`/`inset` ->
//! `BadParam`; output above `MAX_ELEMS` points / vertices / prims or an f32-overflowing position -> `Failed`.
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail, Group, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;
/// Output allocation cap per element class (AGENTS.md section 4).
const MAX_ELEMS: u64 = 1 << 22;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
/// Areas (|Newell|) at or below this are treated as degenerate.
const MIN_NEWELL: f64 = 1e-12;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("distance", ParamValue::F(0.0)).label("Distance").range(-1.0, 1.0),
        ParamSpec::new("inset", ParamValue::F(0.0)).label("Inset").range(-1.0, 1.0),
        ParamSpec::new("divisions", ParamValue::I(1)).label("Divisions").range(1.0, 10.0),
        ParamSpec::new("output_front", ParamValue::B(true)).label("Output Front"),
        ParamSpec::new("output_side", ParamValue::B(true)).label("Output Side"),
        ParamSpec::new("output_back", ParamValue::B(false)).label("Output Back"),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(PolyExtrude { specs: spec() })
}
struct PolyExtrude {
    specs: Vec<ParamSpec>,
}
type V3 = [f64; 3];
fn bad(m: impl Into<String>) -> CookError {
    CookError::BadParam(m.into())
}
fn failed(m: &str) -> CookError {
    CookError::Failed(format!("polyextrude: {m}"))
}
fn toggle(p: &ParamSet, n: &str) -> Result<bool, CookError> {
    p.toggle(n).ok_or_else(|| bad(format!("`{n}`: missing or not a toggle")))
}
fn float(p: &ParamSet, n: &str) -> Result<f64, CookError> {
    match p.float(n) {
        Some(v) if v.is_finite() => Ok(f64::from(v)),
        _ => Err(bad(format!("`{n}`: missing or not a finite float"))),
    }
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn unit(a: V3) -> Option<V3> {
    let l = dot(a, a).sqrt();
    (l.is_finite() && l > 1e-12).then(|| [a[0] / l, a[1] / l, a[2] / l])
}
/// One extrudable prim: source point ids, source first vertex, ring offsets (per corner) and the unit normal.
struct Face {
    pts: Vec<usize>,
    first_vert: usize,
    pos: Vec<V3>,
    normal: V3,
}
impl Face {
    /// `None` = passthrough (not a closed poly, < 3 corners, bad ids, degenerate or non-finite).
    fn of(src: &Detail, prim: &Prim) -> Option<Face> {
        if !matches!(prim.kind, PrimKind::Poly { closed: true }) {
            return None;
        }
        let (lo, hi) = (usize::try_from(prim.verts.start).ok()?, usize::try_from(prim.verts.end).ok()?);
        let pts: Vec<usize> =
            src.vertex_point.get(lo..hi)?.iter().map(|&p| usize::try_from(p).ok()).collect::<Option<_>>()?;
        if pts.len() < 3 {
            return None;
        }
        let p = src.p();
        let pos: Vec<V3> = pts
            .iter()
            .map(|&i| p.get(i).map(|q| [f64::from(q[0]), f64::from(q[1]), f64::from(q[2])]))
            .collect::<Option<_>>()?;
        if pos.iter().flatten().any(|c| !c.is_finite()) {
            return None;
        }
        // Newell vector (twice the vector area), relative to the first corner for precision.
        let o = *pos.first()?;
        let mut nw = [0.0; 3];
        for (i, a) in pos.iter().enumerate() {
            let b = pos.get((i + 1) % pos.len())?;
            let c = cross(sub(*a, o), sub(*b, o));
            nw = [nw[0] + c[0], nw[1] + c[1], nw[2] + c[2]];
        }
        if !(dot(nw, nw).sqrt() > MIN_NEWELL) {
            return None;
        }
        Some(Face { pts, first_vert: lo, pos, normal: unit(nw)? })
    }
    /// Per-corner inward mitre offset of length-`inset` edge shifts (zero when `inset == 0`).
    fn inset_offsets(&self, inset: f64) -> Vec<V3> {
        let n = self.pos.len();
        let edge_in = |i: usize| -> Option<V3> {
            let (a, b) = (self.pos.get(i % n)?, self.pos.get((i + 1) % n)?);
            unit(cross(self.normal, sub(*b, *a)))
        };
        (0..n)
            .map(|c| {
                if inset == 0.0 {
                    return [0.0; 3];
                }
                let (n0, n1) = (edge_in((c + n - 1) % n), edge_in(c));
                let d = match (n0, n1) {
                    (Some(a), Some(b)) => {
                        let den = 1.0 + dot(a, b);
                        let s = if den > 1e-6 { inset / den } else { inset * 0.5 };
                        [(a[0] + b[0]) * s, (a[1] + b[1]) * s, (a[2] + b[2]) * s]
                    }
                    (Some(a), None) | (None, Some(a)) => [a[0] * inset, a[1] * inset, a[2] * inset],
                    (None, None) => [0.0; 3],
                };
                d
            })
            .collect()
    }
}
/// Prim-group selection: `None` = every prim.
fn selection(src: &Detail, names: &str) -> Result<Option<Vec<bool>>, CookError> {
    let names: Vec<&str> = names.split_whitespace().collect();
    if names.is_empty() {
        return Ok(None);
    }
    let mut sel = vec![false; src.prims.len()];
    for n in names {
        let g: &Group =
            src.group(AttrClass::Prim, n).ok_or_else(|| failed(&format!("unknown primitive group `{n}`")))?;
        for (s, m) in sel.iter_mut().zip(&g.mask) {
            *s |= *m;
        }
    }
    Ok(Some(sel))
}
/// Element `i` of every entry of `idx` (`None` -> default element). Total: short sources default-fill.
fn gather(d: &AttrData, idx: &[Option<usize>]) -> AttrData {
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
/// Output topology plus, per output vertex / prim, the source element it copies attributes from.
#[derive(Default)]
struct Topo {
    vertex_point: Vec<u32>,
    vert_src: Vec<Option<usize>>,
    prims: Vec<Prim>,
    prim_src: Vec<Option<usize>>,
}
impl Topo {
    fn push(&mut self, kind: PrimKind, ids: &[u32], vs: Vec<Option<usize>>, from: usize) -> Result<(), CookError> {
        let len = |v: &Vec<u32>| u32::try_from(v.len()).map_err(|_| failed("vertex id overflow"));
        let start = len(&self.vertex_point)?;
        self.vertex_point.extend_from_slice(ids);
        let end = len(&self.vertex_point)?;
        self.vert_src.extend(vs);
        self.prims.push(Prim { kind, verts: start..end });
        self.prim_src.push(Some(from));
        Ok(())
    }
}
impl Node<Detail> for PolyExtrude {
    fn type_name(&self) -> &'static str {
        "polyextrude"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let (distance, inset) = (float(p, "distance")?, float(p, "inset")?);
        let divs = p.int("divisions").ok_or_else(|| bad("`divisions`: missing or not an int"))?;
        if divs < 1 {
            return Err(bad(format!("`divisions` must be >= 1, got {divs}")));
        }
        let divs = usize::try_from(divs).map_err(|_| bad("`divisions` out of range"))?;
        let (front, side, back) = (toggle(p, "output_front")?, toggle(p, "output_side")?, toggle(p, "output_back")?);
        let names = p.string("group").unwrap_or("");
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let sel = selection(src, names)?;
        let faces: Vec<Option<Face>> =
            src.prims
                .iter()
                .enumerate()
                .map(|(i, pr)| {
                    if sel.as_ref().is_none_or(|s| s.get(i).copied().unwrap_or(false)) {
                        Face::of(src, pr)
                    } else {
                        None
                    }
                })
                .collect();
        if faces.iter().all(Option::is_none) {
            return Ok(Arc::clone(src));
        }
        // ---- size cap (u64, before any allocation) ----
        let d = divs as u64;
        let (mut np, mut nv, mut nq) =
            (u64::from(src.point_count), u64::from(src.vertex_count()), u64::from(src.prim_count()));
        for f in faces.iter().flatten() {
            let n = f.pts.len() as u64;
            let q = u64::from(front) + u64::from(back) + if side { n.saturating_mul(d) } else { 0 };
            let v =
                n * (u64::from(front) + u64::from(back)) + if side { n.saturating_mul(d).saturating_mul(4) } else { 0 };
            np = np.saturating_add(n.saturating_mul(d + 1));
            nv = nv.saturating_add(v);
            nq = nq.saturating_add(q);
        }
        if np > MAX_ELEMS || nv > MAX_ELEMS || nq > MAX_ELEMS {
            return Err(failed("output element count exceeds the allocation cap"));
        }
        // ---- which input points survive ----
        let npts = src.point_count as usize;
        let (mut by_ext, mut by_pass) = (vec![false; npts], vec![false; npts]);
        for (pr, f) in src.prims.iter().zip(&faces) {
            let flags = if f.is_some() { &mut by_ext } else { &mut by_pass };
            let range = usize::try_from(pr.verts.start).unwrap_or(0)..usize::try_from(pr.verts.end).unwrap_or(0);
            for &pt in src.vertex_point.get(range).unwrap_or(&[]) {
                if let Some(s) = flags.get_mut(pt as usize) {
                    *s = true;
                }
            }
        }
        let sp = src.p();
        let mut remap: Vec<Option<u32>> = vec![None; npts];
        let mut point_src: Vec<Option<usize>> = Vec::new();
        let mut out_p: Vec<[f32; 3]> = Vec::new();
        for i in 0..npts {
            if by_ext[i] && !by_pass[i] {
                continue;
            }
            remap[i] = Some(u32::try_from(point_src.len()).map_err(|_| failed("point id overflow"))?);
            point_src.push(Some(i));
            out_p.push(sp.get(i).copied().unwrap_or([0.0; 3]));
        }
        // ---- build topology ----
        let mut o = Topo::default();
        for (qi, (pr, f)) in src.prims.iter().zip(&faces).enumerate() {
            let Some(f) = f else {
                let range = usize::try_from(pr.verts.start).unwrap_or(0)..usize::try_from(pr.verts.end).unwrap_or(0);
                let vs = src.vertex_point.get(range.clone()).unwrap_or(&[]);
                let ids: Vec<u32> = vs
                    .iter()
                    .map(|&pt| remap.get(pt as usize).copied().flatten().ok_or_else(|| failed("dangling point id")))
                    .collect::<Result<_, _>>()?;
                o.push(pr.kind.clone(), &ids, range.map(Some).collect(), qi)?;
                continue;
            };
            let n = f.pts.len();
            let offs = f.inset_offsets(inset);
            let base = point_src.len();
            for k in 0..=divs {
                let t = k as f64 / divs as f64;
                for (c, o) in offs.iter().enumerate() {
                    let (a, pc) = (f.pts.get(c).copied(), f.pos.get(c).copied().unwrap_or([0.0; 3]));
                    let q: [f32; 3] = [0, 1, 2].map(|j| (pc[j] + t * (o[j] + f.normal[j] * distance)) as f32);
                    if q.iter().any(|v| !v.is_finite()) {
                        return Err(failed("extruded position overflows f32"));
                    }
                    point_src.push(a);
                    out_p.push(q);
                }
            }
            let ring = |k: usize, c: usize| -> Result<u32, CookError> {
                u32::try_from(base + k * n + c % n).map_err(|_| failed("point id overflow"))
            };
            let corner = |c: usize| Some(f.first_vert + c);
            if front {
                let ids: Vec<u32> = (0..n).map(|c| ring(divs, c)).collect::<Result<_, _>>()?;
                o.push(PrimKind::Poly { closed: true }, &ids, (0..n).map(corner).collect(), qi)?;
            }
            if side {
                for k in 1..=divs {
                    for c in 0..n {
                        let ids = [ring(k - 1, c)?, ring(k - 1, c + 1)?, ring(k, c + 1)?, ring(k, c)?];
                        o.push(PrimKind::Poly { closed: true }, &ids, vec![None; 4], qi)?;
                    }
                }
            }
            if back {
                let ids: Vec<u32> = (0..n).rev().map(|c| ring(0, c)).collect::<Result<_, _>>()?;
                o.push(PrimKind::Poly { closed: true }, &ids, (0..n).rev().map(corner).collect(), qi)?;
            }
        }
        // ---- assemble: attrs and groups follow the maps ----
        let mut out = Detail {
            point_count: u32::try_from(point_src.len()).map_err(|_| failed("point id overflow"))?,
            vertex_point: o.vertex_point,
            prims: o.prims,
            attrs: Vec::new(),
            groups: Vec::new(),
        };
        let map_for = |c: AttrClass| -> Option<&[Option<usize>]> {
            match c {
                AttrClass::Point => Some(&point_src),
                AttrClass::Vertex => Some(&o.vert_src),
                AttrClass::Prim => Some(&o.prim_src),
                AttrClass::Detail => None,
            }
        };
        for a in &src.attrs {
            let mut a = a.clone();
            if let Some(m) = map_for(a.class) {
                a.data = gather(&a.data, m);
                if a.class == AttrClass::Point && a.name == "P" {
                    a.data = AttrData::V3(out_p.clone());
                }
            }
            out.attrs.push(a);
        }
        for g in &src.groups {
            let mut g = g.clone();
            if let Some(m) = map_for(g.class) {
                g.mask = m.iter().map(|i| i.and_then(|i| g.mask.get(i).copied()).unwrap_or(false)).collect();
            }
            out.groups.push(g);
        }
        Ok(Arc::new(out))
    }
}
