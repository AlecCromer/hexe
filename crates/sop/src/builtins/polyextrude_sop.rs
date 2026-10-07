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
        let area2 = dot(nw, nw).sqrt();
        if !area2.is_finite() || area2 <= MIN_NEWELL {
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
                match (n0, n1) {
                    (Some(a), Some(b)) => {
                        let den = 1.0 + dot(a, b);
                        let s = if den > 1e-6 { inset / den } else { inset * 0.5 };
                        [(a[0] + b[0]) * s, (a[1] + b[1]) * s, (a[2] + b[2]) * s]
                    }
                    (Some(a), None) | (None, Some(a)) => [a[0] * inset, a[1] * inset, a[2] * inset],
                    (None, None) => [0.0; 3],
                }
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
            skipped_attribs: Vec::new(),
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
#[cfg(test)]
mod tests {
    use super::*;
    use hexe_geo::{Attribute, TypeInfo};
    fn cook(d: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        new().cook(&ctx, &[Arc::new(d)])
    }
    fn quad() -> Detail {
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2, 3], true);
        d
    }
    /// Unit box (corners +-0.5, 8 shared points), 6 outward-wound quads, closed form.
    fn cube() -> Detail {
        let mut d = Detail::default();
        d.add_points(8);
        let c = |b: u32, i: u32| if (i >> b) & 1 == 1 { 0.5 } else { -0.5 };
        *d.p_mut() = (0..8).map(|i| [c(2, i), c(1, i), c(0, i)]).collect();
        for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
            d.add_poly(&f, true);
        }
        d
    }
    fn f(v: f32) -> ParamValue {
        ParamValue::F(v)
    }
    fn prim_pts(d: &Detail, i: usize) -> Vec<[f32; 3]> {
        let pr = &d.prims[i];
        d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&p| d.p()[p as usize]).collect()
    }
    /// Independent Newell normal (unnormalised).
    fn newell(p: &[[f32; 3]]) -> [f32; 3] {
        let mut n = [0.0f32; 3];
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            n[0] += (a[1] - b[1]) * (a[2] + b[2]);
            n[1] += (a[2] - b[2]) * (a[0] + b[0]);
            n[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        n
    }
    fn centroid(p: &[[f32; 3]]) -> [f32; 3] {
        let k = p.len() as f32;
        [0, 1, 2].map(|j| p.iter().map(|q| q[j]).sum::<f32>() / k)
    }
    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|j| (a[j] - b[j]).abs() < 1e-5)
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "polyextrude");
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("distance", f(0.0)),
                ("inset", f(0.0)),
                ("divisions", ParamValue::I(1)),
                ("output_front", ParamValue::B(true)),
                ("output_side", ParamValue::B(true)),
                ("output_back", ParamValue::B(false)),
                ("group", ParamValue::S(String::new())),
            ]
        );
    }
    fn defaults() -> ParamSet {
        new().params().iter().fold(ParamSet::new(), |p, s| p.with(&s.name, s.default.clone()))
    }
    fn with(p: ParamSet, k: &str, v: ParamValue) -> ParamSet {
        p.with(k, v)
    }
    #[test]
    fn unit_quad_distance_1_is_8_points_5_prims_top_at_z1() {
        let out = cook(quad(), with(defaults(), "distance", f(1.0))).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (8, 5));
        let top = prim_pts(&out, 0);
        assert!(top.iter().all(|p| p[2] == 1.0), "{top:?}");
        assert_eq!(
            top.iter().map(|p| [p[0], p[1]]).collect::<Vec<_>>(),
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        );
        assert_eq!(newell(&top), [0.0, 0.0, 2.0]);
        // side 0 sits on the y=0 edge, outward normal -Y
        let s0 = prim_pts(&out, 1);
        assert_eq!(s0, [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]]);
        assert_eq!(newell(&s0), [0.0, -2.0, 0.0]);
        // every side normal points away from the extrusion axis
        for i in 1..5 {
            let s = prim_pts(&out, i);
            let (n, c) = (newell(&s), centroid(&s));
            assert!(n[0] * (c[0] - 0.5) + n[1] * (c[1] - 0.5) > 0.0, "side {i} inward");
        }
    }
    #[test]
    fn divisions_3_is_16_points_13_prims_layers_evenly_spaced() {
        let out =
            cook(quad(), with(with(defaults(), "distance", f(1.0)), "divisions", ParamValue::I(3))).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (16, 13));
        assert!(prim_pts(&out, 0).iter().all(|p| p[2] == 1.0));
        let z: Vec<f32> = (1..13).step_by(4).map(|i| prim_pts(&out, i)[2][2]).collect();
        assert!(z.iter().zip([1.0 / 3.0, 2.0 / 3.0, 1.0]).all(|(a, b)| (a - b).abs() < 1e-6), "{z:?}");
    }
    #[test]
    fn inset_quarter_distance_0_top_corners_at_quarter_three_quarters() {
        let out = cook(quad(), with(defaults(), "inset", f(0.25))).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (8, 5));
        let top = prim_pts(&out, 0);
        let want = [[0.25, 0.25, 0.0], [0.75, 0.25, 0.0], [0.75, 0.75, 0.0], [0.25, 0.75, 0.0]];
        assert!(top.iter().zip(want).all(|(a, b)| near(*a, b)), "{top:?}");
        // inset before distance: distance 1 lifts the already-inset ring
        let up = cook(quad(), with(with(defaults(), "inset", f(0.25)), "distance", f(1.0))).expect("cooks");
        assert!(prim_pts(&up, 0).iter().zip(want).all(|(a, b)| near(*a, [b[0], b[1], 1.0])));
    }
    #[test]
    fn back_adds_one_prim_with_reversed_winding() {
        let out = cook(quad(), with(with(defaults(), "distance", f(1.0)), "output_back", ParamValue::B(true)))
            .expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (8, 6));
        let (front, back) = (prim_pts(&out, 0), prim_pts(&out, 5));
        assert_eq!(newell(&front), [0.0, 0.0, 2.0]);
        assert_eq!(newell(&back), [0.0, 0.0, -2.0]);
        assert!(back.iter().all(|p| p[2] == 0.0));
    }
    #[test]
    fn output_toggles_select_pieces() {
        let p = |a, b, c| {
            with(
                with(with(defaults(), "output_front", ParamValue::B(a)), "output_side", ParamValue::B(b)),
                "output_back",
                ParamValue::B(c),
            )
        };
        for (t, want) in [
            ((true, false, false), 1),
            ((false, true, false), 4),
            ((false, false, true), 1),
            ((false, false, false), 0),
        ] {
            let out = cook(quad(), with(p(t.0, t.1, t.2), "distance", f(1.0))).expect("cooks");
            assert_eq!(out.prim_count(), want, "{t:?}");
        }
    }
    #[test]
    fn box_distance_half_is_48_points_30_prims_outward() {
        let out = cook(cube(), with(defaults(), "distance", f(0.5))).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (48, 30));
        for face in 0..6 {
            let top = prim_pts(&out, face * 5);
            let (n, c) = (newell(&top), centroid(&top));
            assert!(n[0] * c[0] + n[1] * c[1] + n[2] * c[2] > 0.0, "face {face} top points inward");
            let m = c.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
            assert!((m - 1.0).abs() < 1e-5, "top centroid {c:?} should sit at 0.5 + 0.5");
        }
    }
    #[test]
    fn group_limits_extrusion_others_pass_through() {
        let mut d = cube();
        d.groups.push(Group {
            name: "g".into(),
            class: AttrClass::Prim,
            mask: vec![true, false, false, false, false, true],
        });
        let before: Vec<Vec<[f32; 3]>> = (1..5).map(|i| prim_pts(&d, i)).collect();
        let out =
            cook(d, with(with(defaults(), "distance", f(0.5)), "group", ParamValue::S("g".into()))).expect("cooks");
        assert_eq!(out.prim_count(), 4 + 2 * 5);
        // faces 0 and 5 cover points {0,1,2,3} and {1,3,5,7}; the other four still need all 8, so 8 kept + 16 new
        assert_eq!(out.point_count, 8 + 2 * 8);
        // order: [extruded 0 -> 0..5], passthrough 1..=4 -> 5..9, [extruded 5 -> 9..14]
        let kept: Vec<Vec<[f32; 3]>> = (5..9).map(|i| prim_pts(&out, i)).collect();
        assert_eq!(kept, before);
        let gp = out.group(AttrClass::Prim, "g").expect("group kept");
        assert_eq!(gp.mask.len(), 14);
        assert_eq!(gp.mask.iter().filter(|m| **m).count(), 10);
        let unknown = cook(cube(), with(defaults(), "group", ParamValue::S("nope".into())));
        assert!(matches!(unknown, Err(CookError::Failed(_))));
    }
    #[test]
    fn hostile_non_polygon_and_degenerate_pass_through() {
        let mut d = quad();
        d.add_poly(&[0, 1], false); // open polyline
        d.add_poly(&[0, 1, 2, 3], false); // open poly
        d.add_poly(&[0, 0, 0, 0], true); // zero area
        d.add_poly(&[0, 1, 1], true); // collinear / zero area
        d.add_poly(&[0, 1], true); // < 3 corners
        let p = with(defaults(), "distance", f(1.0));
        let out = cook(d.clone(), p.clone()).expect("cooks");
        assert_eq!(out.prim_count(), 5 + 5);
        // all passthrough -> same Arc
        let mut only_bad = Detail::default();
        only_bad.add_points(3);
        *only_bad.p_mut() = vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        only_bad.add_poly(&[0, 1, 2], true);
        let a = Arc::new(only_bad);
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        let o = new().cook(&ctx, std::slice::from_ref(&a)).expect("cooks");
        assert!(Arc::ptr_eq(&o, &a));
        // empty / missing input
        assert_eq!(*cook(Detail::default(), p.clone()).expect("cooks"), Detail::default());
        let none = new().cook(&ctx, &[]).expect("cooks");
        assert_eq!(none.point_count, 0);
    }
    #[test]
    fn hostile_params_are_rejected_not_panicked() {
        for (k, v) in [
            ("divisions", ParamValue::I(0)),
            ("divisions", ParamValue::I(-3)),
            ("distance", f(f32::NAN)),
            ("inset", f(f32::INFINITY)),
        ] {
            assert!(matches!(cook(quad(), with(defaults(), k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        let huge = cook(quad(), with(defaults(), "divisions", ParamValue::I(i32::MAX)));
        assert!(matches!(huge, Err(CookError::Failed(_))));
        let overflow = cook(quad(), with(defaults(), "distance", f(f32::MAX)));
        assert!(overflow.is_ok() || matches!(overflow, Err(CookError::Failed(_))));
    }
    #[test]
    fn attrs_follow_source_corners_and_prims() {
        let mut d = quad();
        d.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0]]),
            type_info: TypeInfo::Color,
        });
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Prim,
            data: AttrData::F32(vec![7.0]),
            type_info: TypeInfo::None,
        });
        let out = cook(d, with(defaults(), "distance", f(1.0))).expect("cooks");
        let Some(AttrData::V3(cd)) = out.attr(AttrClass::Point, "Cd").map(|a| &a.data) else { panic!("Cd lost") };
        assert_eq!(cd.len(), 8);
        assert_eq!(cd[..4], cd[4..]);
        assert_eq!(cd[2], [0.0, 0.0, 1.0]);
        let Some(AttrData::F32(w)) = out.attr(AttrClass::Prim, "w").map(|a| &a.data) else { panic!("w lost") };
        assert_eq!(w, &[7.0; 5]);
    }
    #[test]
    fn scene_loader_cooks_example_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/polyextrude.json"))
            .expect("example present");
        let (mut g, id) = crate::load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert_eq!((out.point_count, out.prim_count()), (48, 30));
        let bogus = r#"{"nodes":[{"name":"c","type":"polyextrude","params":{"bogus":1}}],"display":"c"}"#;
        assert!(crate::load_scene(bogus).is_err());
        assert_eq!(crate::registry().create("polyextrude").map(|n| n.type_name()), Some("polyextrude"));
    }
}
