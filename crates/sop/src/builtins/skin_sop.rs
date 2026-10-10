//! Skin SOP (token `skin`): loft a surface through the curves of input 0 (one curve prim = one section, in prim order).
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! * Sections = every `Poly`/`Polyline` prim with >= 2 corners; fewer than 2 sections = empty output. The surface is closed
//!   around `u` (the ring direction) iff EVERY section is closed; otherwise it is open and the sections' point lists are used
//!   as they are.
//! * `surftype` menu `quads` (0, default) | `triangles` (1: each quad split along its first diagonal).
//! * `closev` B off: also loft the LAST section back to the FIRST (needs >= 3 sections).
//! * `resample` B on: sections with differing point counts are resampled by arclength to the LARGEST count (closed: evenly
//!   around the loop starting at the first point; open: end points kept); off + differing counts = `Failed`. Equal counts
//!   use the input positions unchanged (no resampling).
//! * Quad `(s, j)` = `[(s,j), (s+1,j), (s+1,j+1), (s,j+1)]` reversed to `A,D,C,B`: the Houdini normal is `u x v` (ring direction x
//!   loft direction), outward for CCW rings lofted along their axis.
//! * `order` menu `prim`(0) | `reverse`(1) | `x`/`y`/`z`(2..=4: sections sorted ascending by their mean coordinate).
//! * `divs` I 1 (1..=256): stations per section pair (the last section closes an open loft). `keepshape` B on = straight
//!   ruled loft (lerp between sections); off = Catmull-Rom (Barry-Goldman) interpolation through the neighbouring sections
//!   (end sections extrapolate linearly, `closev` wraps) - exact for quadratic profiles at uniform spacing. `uniform` B on
//!   = uniform knots; off = chord-length knots (mean point-to-point distance between sections).
//! * `carryattrs` B on: every input point attr (not `P`) is inherited by the output points through the same linear weights
//!   as the position (resampled points blend their two neighbours, intermediate stations blend the two sections; floats =
//!   weighted mean, int/str = heaviest source); prim attrs: quads between sections `s` and `s+1` copy section `s`'s prim.
//! * `surftype` 2 = `nurbs`: ONE `NurbsSurf` / `BezierSurf` instead of a mesh: CV rows = the loft stations (sections + `divs` stations, so
//!   the v direction is a ruled order-2 basis through them, wrapped with `closev`), u = the sections' own basis. Spline curve sections
//!   (`NurbsCurve` / `BezierCurve`) keep their basis (all sections must then be splines of equal order / knots / closedness / CV
//!   count, else `Failed`; their weights `Pw` blend with the station weights); Poly/Polyline sections use linear order 2 (wrapped when
//!   closed). With `surftype` quads (0) and any spline section in the input the result is ALSO a NURBS surface (auto); triangles (1)
//!   tessellates the splines (8 samples per span) and lofts the polygons. NURBS output carries no `uv` / attributes / `tris`.
//!   UNVERIFIED vs Houdini (analytical oracle only).
//! * Output (mesh) = new points + vertex `uv` V3 (`u` = column / columns, `v` = station / stations).
use super::meshutil::{V3, add, bad, failed, positions, scale, sub, toggle};
use super::splinesurf::{TESS_LOD, point_weight, spline_curves, spline_detail};
use super::surfutil::{Curve, Mesh, curves, dist};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const OP: &str = "skin";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("surftype", ParamValue::Menu(0)).label("Surface Type").menu(&[
            ("quads", "Quadrilaterals"),
            ("triangles", "Triangles"),
            ("nurbs", "NURBS"),
        ]),
        ParamSpec::new("closev", ParamValue::B(false)).label("Close V"),
        ParamSpec::new("resample", ParamValue::B(true)).label("Resample Mismatched Sections"),
        ParamSpec::new("order", ParamValue::Menu(0)).label("Section Order").menu(&[
            ("prim", "Primitive Order"),
            ("reverse", "Reverse"),
            ("x", "Sort By X"),
            ("y", "Sort By Y"),
            ("z", "Sort By Z"),
        ]),
        ParamSpec::new("divs", ParamValue::I(1)).label("Divisions Between Sections").range(1.0, 16.0),
        ParamSpec::new("keepshape", ParamValue::B(true)).label("Keep Shape (Straight Loft)"),
        ParamSpec::new("uniform", ParamValue::B(true)).label("Uniform Knots"),
        ParamSpec::new("carryattrs", ParamValue::B(true)).label("Inherit Attributes"),
        ParamSpec::new("computeuv", ParamValue::B(true)).label("Compute UVs"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Skin { specs: spec() })
}
struct Skin {
    specs: Vec<ParamSpec>,
}

/// Houdini 20.5 (oracle sop-skin, sop-skin-2ring): a section short of points keeps ALL its points and gains EDGE MIDPOINTS. Each pass
/// splits the `need` LONGEST edges (length^2 summed in f32, ties -> lower edge index first; octagon 8 -> 12 splits edges 6,4,1,3 in
/// that order); `need` >= edge count splits every edge (edge order) and goes on (3 -> 8 = quarter points; pass order beyond one pass
/// UNVERIFIED). Returns the ring (position + source-point weights) and the ring indices of the new points in CREATION order.
type Blend = Vec<(usize, f64)>;
type Resampled = (Vec<(V3, Blend)>, Vec<usize>);
fn resample(pos: &[V3], closed: bool, n: usize) -> Resampled {
    if pos.len() < 2 || n <= pos.len() {
        return (resample_even(pos, closed, n), Vec::new());
    }
    // node = (id, position, weights); id < pos.len() = original, else creation rank + pos.len()
    let mut ring: Vec<(usize, V3, Blend)> = pos.iter().enumerate().map(|(i, q)| (i, *q, vec![(i, 1.0)])).collect();
    let mut next_id = pos.len();
    while ring.len() < n {
        let m = ring.len();
        let edges = if closed { m } else { m - 1 };
        let need = n - m;
        let f = |q: V3| [q[0] as f32, q[1] as f32, q[2] as f32];
        let l2 = |e: usize| {
            let (a, b) = (f(ring[e].1), f(ring[(e + 1) % m].1));
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
        };
        let mut pick: Vec<usize> = (0..edges).collect();
        if need < edges {
            pick.sort_by(|&x, &y| l2(y).total_cmp(&l2(x)).then(x.cmp(&y)));
            pick.truncate(need);
        }
        let mut ids = std::collections::HashMap::new();
        for &e in &pick {
            ids.insert(e, next_id);
            next_id += 1;
        }
        let mut out = Vec::with_capacity(m + pick.len());
        for e in 0..m {
            out.push(ring[e].clone());
            if let Some(&id) = ids.get(&e) {
                let (a, b) = (&ring[e], &ring[(e + 1) % m]);
                let mut w: Vec<(usize, f64)> = Vec::new();
                for &(i, v) in a.2.iter().chain(&b.2) {
                    match w.iter_mut().find(|c| c.0 == i) {
                        Some(c) => c.1 += v * 0.5,
                        None => w.push((i, v * 0.5)),
                    }
                }
                out.push((id, scale(add(a.1, b.1), 0.5), w));
            }
        }
        ring = out;
    }
    let mut order: Vec<usize> = (0..ring.len()).filter(|&k| ring[k].0 >= pos.len()).collect();
    order.sort_by_key(|&k| ring[k].0);
    (ring.into_iter().map(|(_, q, w)| (q, w)).collect(), order)
}
/// `n` points evenly spaced by arclength along `pos` (closed: loop, last point NOT repeated; open: end points kept). Each
/// result carries its position and the two source indices (into `pos`) with linear weights.
fn resample_even(pos: &[V3], closed: bool, n: usize) -> Vec<(V3, Vec<(usize, f64)>)> {
    let mut path = pos.to_vec();
    if closed {
        path.push(pos[0]);
    }
    let mut acc = vec![0.0f64];
    for k in 1..path.len() {
        acc.push(acc[k - 1] + dist(path[k - 1], path[k]));
    }
    let total = acc[acc.len() - 1];
    if total <= 0.0 {
        return (0..n).map(|_| (pos[0], vec![(0, 1.0)])).collect();
    }
    let mut seg = 0;
    (0..n)
        .map(|k| {
            let t = total * k as f64 / if closed { n as f64 } else { (n - 1) as f64 };
            while seg + 2 < path.len() && acc[seg + 1] < t {
                seg += 1;
            }
            let len = acc[seg + 1] - acc[seg];
            let f = if len > 0.0 { ((t - acc[seg]) / len).clamp(0.0, 1.0) } else { 0.0 };
            let (a, b) = (seg % pos.len(), (seg + 1) % pos.len());
            (add(path[seg], scale(sub(path[seg + 1], path[seg]), f)), vec![(a, 1.0 - f), (b, f)])
        })
        .collect()
}

/// One section as loft stations: positions + attribute blend weights (over SOURCE point ids).
#[derive(Clone)]
struct Row {
    p: Vec<V3>,
    w: Vec<Vec<(usize, f64)>>,
}

fn lerp_row(a: &Row, b: &Row, t: f64) -> Row {
    let p = a.p.iter().zip(&b.p).map(|(x, y)| add(scale(*x, 1.0 - t), scale(*y, t))).collect();
    let w =
        a.w.iter()
            .zip(&b.w)
            .map(|(x, y)| {
                x.iter()
                    .map(|&(i, v)| (i, v * (1.0 - t)))
                    .chain(y.iter().map(|&(i, v)| (i, v * t)))
                    .filter(|c| c.1 > 0.0)
                    .collect()
            })
            .collect();
    Row { p, w }
}

/// Barry-Goldman Catmull-Rom through `p[1]..p[2]` with knots `k`, at parameter `t` in `[k1, k2]`.
fn barry_goldman(p: [V3; 4], k: [f64; 4], t: f64) -> V3 {
    let mix = |a: V3, b: V3, ka: f64, kb: f64| {
        let d = kb - ka;
        add(scale(a, (kb - t) / d), scale(b, (t - ka) / d))
    };
    let a1 = mix(p[0], p[1], k[0], k[1]);
    let a2 = mix(p[1], p[2], k[1], k[2]);
    let a3 = mix(p[2], p[3], k[2], k[3]);
    let b1 = mix(a1, a2, k[0], k[2]);
    let b2 = mix(a2, a3, k[1], k[3]);
    mix(b1, b2, k[1], k[2])
}

/// Mean point-to-point distance of two rows (chord length of the loft direction).
fn row_dist(a: &Row, b: &Row) -> f64 {
    a.p.iter().zip(&b.p).map(|(x, y)| dist(*x, *y)).sum::<f64>() / a.p.len().max(1) as f64
}

/// Station between sections `rows[s]` and `rows[s2]` at `t` in `[0, 1)`; `smooth` = Catmull-Rom through the neighbouring
/// sections (uniform or chord-length knots), else a straight lerp. Attribute weights always lerp.
fn station(rows: &[Row], s: usize, s2: usize, t: f64, wrap: bool, smooth: bool, uniform: bool) -> Row {
    let (a, b) = (&rows[s], &rows[s2]);
    let mut r = lerp_row(a, b, t);
    if !smooth || t == 0.0 || rows.len() < 3 {
        return r;
    }
    let m = rows.len();
    let ghost = |edge: usize, inner: usize| Row {
        p: rows[edge].p.iter().zip(&rows[inner].p).map(|(e, i)| sub(scale(*e, 2.0), *i)).collect(),
        w: rows[edge].w.clone(),
    };
    let before = if s > 0 {
        rows[s - 1].clone()
    } else if wrap {
        rows[m - 1].clone()
    } else {
        ghost(s, s2)
    };
    let after = if s2 + 1 < m {
        rows[s2 + 1].clone()
    } else if wrap {
        rows[(s2 + 1) % m].clone()
    } else {
        ghost(s2, s)
    };
    let k = if uniform {
        [0.0, 1.0, 2.0, 3.0]
    } else {
        let (d01, d12, d23) = (row_dist(&before, a), row_dist(a, b), row_dist(b, &after));
        [0.0, d01, d01 + d12, d01 + d12 + d23]
    };
    if k[1] - k[0] < 1e-12 || k[2] - k[1] < 1e-12 || k[3] - k[2] < 1e-12 {
        return r;
    }
    let tt = k[1] + (k[2] - k[1]) * t;
    for j in 0..r.p.len() {
        r.p[j] = barry_goldman([before.p[j], a.p[j], b.p[j], after.p[j]], k, tt);
    }
    r
}

const ORDER_PRIM: u32 = 0;
const ORDER_REVERSE: u32 = 1;
const MAX_DIVS: i32 = 256;

impl Node<Detail> for Skin {
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
        let surftype = match p.menu("surftype") {
            Some(v @ 0..=2) => v,
            Some(v) => return Err(bad(format!("skin: `surftype`: menu index {v} out of range 0..=2"))),
            None => return Err(bad("skin: `surftype`: missing or not a menu")),
        };
        let tris = surftype == 1;
        let order = match p.menu("order") {
            Some(v @ 0..=4) => v,
            Some(v) => return Err(bad(format!("skin: `order`: menu index {v} out of range 0..=4"))),
            None => return Err(bad("skin: `order`: missing or not a menu")),
        };
        let divs = match p.int("divs") {
            Some(d) if (1..=MAX_DIVS).contains(&d) => d as usize,
            _ => return Err(bad(format!("skin: `divs` must be an int in 1..={MAX_DIVS}"))),
        };
        let (closev, rs) = (toggle(p, "closev")?, toggle(p, "resample")?);
        let (keepshape, uniform, carry) = (toggle(p, "keepshape")?, toggle(p, "uniform")?, toggle(p, "carryattrs")?);
        let Some(src0) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let nurbs_mode = surftype == 2 || (surftype == 0 && src0.has_spline_prims());
        let tessed = if !nurbs_mode && src0.has_spline_prims() {
            Some(Arc::new(src0.tessellate(TESS_LOD, TESS_LOD)))
        } else {
            None
        };
        let src: &Arc<Detail> = tessed.as_ref().unwrap_or(src0);
        let pos = positions(src);
        let sc = if nurbs_mode { spline_curves(src0) } else { Vec::new() };
        let mut secs = curves(src);
        if nurbs_mode {
            secs.extend(sc.iter().map(|s| Curve { pts: s.pts.clone(), closed: s.closed, prim: s.prim }));
            secs.sort_by_key(|c| c.prim);
        }
        if secs.len() < 2 {
            return Ok(Arc::new(Detail::default()));
        }
        match order {
            ORDER_PRIM => {}
            ORDER_REVERSE => secs.reverse(),
            axis => {
                let ax = (axis - 2) as usize;
                let key = |c: &Curve| c.pts.iter().map(|&i| pos[i as usize][ax]).sum::<f64>() / c.pts.len() as f64;
                secs.sort_by(|a, b| key(a).total_cmp(&key(b)));
            }
        }
        let closed_u = secs.iter().all(|c| c.closed);
        let n = secs.iter().map(|c| c.pts.len()).max().unwrap_or(0);
        if !rs && secs.iter().any(|c| c.pts.len() != n) {
            return Err(failed(OP, "sections have different point counts and `resample` is off"));
        }
        let mut rows: Vec<Row> = Vec::with_capacity(secs.len());
        // ring indices of the points a resampled section ADDED, in creation order (Houdini appends them after every original point)
        let mut news: Vec<Vec<usize>> = vec![Vec::new(); secs.len()];
        for c in &secs {
            let q: Vec<V3> = c.pts.iter().map(|&i| pos[i as usize]).collect();
            let row = if q.len() == n {
                Row { p: q, w: c.pts.iter().map(|&i| vec![(i as usize, 1.0)]).collect() }
            } else {
                let (r, order) = resample(&q, c.closed, n);
                news[rows.len()] = order;
                Row {
                    p: r.iter().map(|x| x.0).collect(),
                    w: r.iter().map(|x| x.1.iter().map(|&(l, f)| (c.pts[l] as usize, f)).collect()).collect(),
                }
            };
            rows.push(row);
        }
        let s_count = rows.len();
        let wrap_v = closev && s_count >= 3;
        let v_segs = if wrap_v { s_count } else { s_count - 1 };
        // stations: `divs` per section pair (+ the last section when open)
        let mut stations: Vec<Row> = Vec::with_capacity(v_segs * divs + 1);
        let mut station_sec: Vec<usize> = Vec::new();
        for s in 0..v_segs {
            let s2 = (s + 1) % s_count;
            for d in 0..divs {
                stations.push(station(&rows, s, s2, d as f64 / divs as f64, wrap_v, !keepshape, uniform));
                station_sec.push(s);
            }
        }
        if !wrap_v {
            stations.push(rows[s_count - 1].clone());
        }
        if nurbs_mode {
            let no = |m: &str| failed(OP, m);
            let spline_of = |c: &Curve| sc.iter().find(|s| s.prim == c.prim);
            let any_spline = secs.iter().any(|c| spline_of(c).is_some());
            let (ubasis, bezier, uwrap) = if any_spline {
                let first = spline_of(&secs[0]).ok_or_else(|| {
                    no("NURBS sections cannot be mixed with polygon sections (use surftype triangles)")
                })?;
                for c in &secs {
                    match spline_of(c) {
                        Some(s)
                            if s.basis == first.basis
                                && s.bezier == first.bezier
                                && s.closed == first.closed
                                && s.pts.len() == first.pts.len() => {}
                        _ => return Err(no("NURBS sections must share order, knots, closedness and CV count")),
                    }
                }
                (first.basis.clone(), first.bezier, first.closed)
            } else {
                (hexe_geo::Basis::new(2), false, closed_u)
            };
            if (stations.len() as u64).saturating_mul(n as u64) > super::meshutil::MAX_ELEMS {
                return Err(no("output exceeds the point cap"));
            }
            let mut cv: Vec<[f32; 3]> = Vec::with_capacity(stations.len() * n);
            let mut pw: Vec<f32> = Vec::with_capacity(stations.len() * n);
            for st in &stations {
                for (pt, w) in st.p.iter().zip(&st.w) {
                    let r = [pt[0] as f32, pt[1] as f32, pt[2] as f32];
                    if !r.iter().all(|x| x.is_finite()) {
                        return Err(no("non-finite or f32-overflowing position"));
                    }
                    cv.push(r);
                    pw.push(w.iter().map(|&(i, f)| f as f32 * point_weight(src0, i as u32)).sum());
                }
            }
            let (cols, rows) = (n as u32, stations.len() as u32);
            let vb = hexe_geo::Basis::new(2);
            let kind = if bezier {
                PrimKind::BezierSurf { cols, rows, ubasis, vbasis: vb, uwrap, vwrap: wrap_v }
            } else {
                PrimKind::NurbsSurf { cols, rows, ubasis, vbasis: vb, uwrap, vwrap: wrap_v }
            };
            return Ok(Arc::new(spline_detail(cv, pw, kind)));
        }
        let mut m = Mesh::default();
        let mut ids: Vec<Vec<u32>> = Vec::with_capacity(stations.len());
        let mut wts: Vec<Vec<(usize, f64)>> = Vec::new();
        // oracle (Houdini 20.5, divs 1): original points first (section-major), then each resampled section's added points in creation order
        let defer = divs == 1;
        let mut ids_sparse: Vec<Vec<Option<u32>>> = Vec::with_capacity(stations.len());
        for (k, st) in stations.iter().enumerate() {
            let late: &[usize] = if defer { news.get(k).map_or(&[][..], |v| v.as_slice()) } else { &[] };
            let mut row = vec![None; n];
            for (j, (pt, w)) in st.p.iter().zip(&st.w).enumerate() {
                if !late.contains(&j) {
                    row[j] = Some(m.point(OP, *pt)?);
                    wts.push(w.clone());
                }
            }
            ids_sparse.push(row);
        }
        for (k, st) in stations.iter().enumerate() {
            let late: &[usize] = if defer { news.get(k).map_or(&[][..], |v| v.as_slice()) } else { &[] };
            for &j in late {
                if let (Some(pt), Some(w), Some(slot)) =
                    (st.p.get(j), st.w.get(j), ids_sparse.get_mut(k).and_then(|r| r.get_mut(j)))
                {
                    *slot = Some(m.point(OP, *pt)?);
                    wts.push(w.clone());
                }
            }
        }
        for row in ids_sparse {
            ids.push(row.into_iter().map(|x| x.unwrap_or(0)).collect());
        }
        let r_count = ids.len();
        let quads = if wrap_v { r_count } else { r_count - 1 };
        let u_segs = if closed_u { n } else { n - 1 };
        for k in 0..quads {
            let k2 = (k + 1) % r_count;
            m.cur_prim = Some(secs[station_sec[k]].prim);
            let (v0, v1) = (k as f32 / quads as f32, (k + 1) as f32 / quads as f32);
            for j in 0..u_segs {
                let j2 = (j + 1) % n;
                let (u0, u1) = (j as f32 / u_segs as f32, (j + 1) as f32 / u_segs as f32);
                m.quad(
                    OP,
                    // oracle (Houdini 20.5): cell wound (k,j) (k,j+1) (k+1,j+1) (k+1,j)
                    [ids[k][j], ids[k][j2], ids[k2][j2], ids[k2][j]],
                    [[u0, v0, 0.0], [u1, v0, 0.0], [u1, v1, 0.0], [u0, v1, 0.0]],
                    tris,
                )?;
            }
        }
        let mut out = if carry { m.finish_blended(src, &wts) } else { m.finish() };
        if !toggle(p, "computeuv")? {
            // oracle: Houdini's skin writes no uv
            out.attrs.retain(|a| !(a.name == "uv" && a.class == AttrClass::Vertex));
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::{positions, prim_normal, testkit};
    use super::super::surfutil::testkit::curve;
    use super::*;
    use crate::load_scene;
    use hexe_geo::{AttrClass, AttrData};
    use hexe_graph::ParamSet;

    fn run(d: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        n.cook(&ctx, &[Arc::new(d)])
    }
    fn defaults() -> ParamSet {
        testkit::defaults(&*new())
    }
    fn sq(z: f32, s: f32) -> Detail {
        curve(&[[-s, -s, z], [s, -s, z], [s, s, z], [-s, s, z]], true)
    }
    fn stack(zs: &[(f32, f32)]) -> Detail {
        let mut d = sq(zs[0].0, zs[0].1);
        for &(z, s) in &zs[1..] {
            d.merge(&sq(z, s));
        }
        d
    }

    fn with_attr(mut d: Detail, name: &str, class: AttrClass, data: AttrData) -> Detail {
        d.add_attr(hexe_geo::Attribute { name: name.into(), class, data, type_info: hexe_geo::TypeInfo::None });
        d
    }

    #[test]
    fn divs_insert_linear_stations_between_sections() {
        let src = stack(&[(0.0, 0.5), (1.0, 0.5), (2.0, 0.5)]);
        let d = run(src, defaults().with("divs", ParamValue::I(3))).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (7 * 4, 6 * 4));
        for k in 0..7 {
            assert!((d.p()[4 * k][2] - k as f32 / 3.0).abs() < 1e-6, "{k} {:?}", d.p()[4 * k]);
        }
        // closev wraps: 3 sections, divs 2 -> 6 stations, 6 quad rings
        let c = run(
            stack(&[(0.0, 0.5), (1.0, 0.5), (2.0, 0.5)]),
            defaults().with("divs", ParamValue::I(2)).with("closev", ParamValue::B(true)),
        )
        .expect("cooks");
        assert_eq!((c.point_count, c.prim_count()), (6 * 4, 6 * 4));
    }

    #[test]
    fn smooth_keepshape_off_is_catmull_rom_exact_for_quadratic_profiles() {
        // ring half-size s = 1 + z^2 at z = 0..3; divs 2 -> station 3 is z = 1.5 -> s = 3.25 (linear would give 3.5)
        let src = stack(&[(0.0, 1.0), (1.0, 2.0), (2.0, 5.0), (3.0, 10.0)]);
        let lin = run(src.clone(), defaults().with("divs", ParamValue::I(2))).expect("cooks");
        let cr =
            run(src, defaults().with("divs", ParamValue::I(2)).with("keepshape", ParamValue::B(false))).expect("cooks");
        assert_eq!(cr.point_count, 7 * 4);
        assert!((lin.p()[12][0] + 3.5).abs() < 1e-6 && (lin.p()[12][2] - 1.5).abs() < 1e-6);
        assert!((cr.p()[12][0] + 3.25).abs() < 1e-6 && (cr.p()[12][2] - 1.5).abs() < 1e-6, "{:?}", cr.p()[12]);
        // the outer segments extrapolate linearly: station 1 (z=0.5) = 1 + 0.25 + ghost correction, finite and between 1 and 2
        assert!(cr.p()[4][0] < -1.0 && cr.p()[4][0] > -2.0 && (cr.p()[4][2] - 0.5).abs() < 1e-6);
        // sections themselves are untouched by smoothing
        assert_eq!(cr.p()[8][0], -2.0);
        // chord-length knots on equally spaced linear sections give the same exact midpoint (2.5)
        let eq = stack(&[(0.0, 1.0), (1.0, 2.0), (2.0, 3.0), (3.0, 4.0)]);
        let ch = run(
            eq,
            defaults()
                .with("divs", ParamValue::I(2))
                .with("keepshape", ParamValue::B(false))
                .with("uniform", ParamValue::B(false)),
        )
        .expect("cooks");
        assert!((ch.p()[12][0] + 2.5).abs() < 1e-6, "{:?}", ch.p()[12]);
    }

    #[test]
    fn order_menu_reorders_the_sections() {
        let src = stack(&[(2.0, 0.5), (0.0, 0.5), (1.0, 0.5)]);
        let z0 = |o: u32| run(src.clone(), defaults().with("order", ParamValue::Menu(o))).expect("cooks").p()[0][2];
        assert_eq!((z0(0), z0(1), z0(4)), (2.0, 1.0, 0.0));
        // by x: all sections share x -> stable sort keeps prim order
        assert_eq!(z0(2), 2.0);
    }

    #[test]
    fn point_and_prim_attrs_are_inherited_with_the_loft_weights() {
        let src = with_attr(
            stack(&[(0.0, 0.5), (1.0, 0.5)]),
            "t",
            AttrClass::Point,
            AttrData::F32(vec![0.0, 1.0, 2.0, 3.0, 10.0, 11.0, 12.0, 13.0]),
        );
        let src = with_attr(src, "cls", AttrClass::Prim, AttrData::F32(vec![1.0, 2.0]));
        let d = run(src, defaults().with("divs", ParamValue::I(2))).expect("cooks");
        assert_eq!(
            d.attr(AttrClass::Point, "t").map(|a| a.data.clone()),
            Some(AttrData::F32(vec![0.0, 1.0, 2.0, 3.0, 5.0, 6.0, 7.0, 8.0, 10.0, 11.0, 12.0, 13.0]))
        );
        assert_eq!(d.attr(AttrClass::Prim, "cls").map(|a| a.data.clone()), Some(AttrData::F32(vec![1.0; 8])));
        // resampled section: 2-point line gets 3 points; the middle one blends both ends
        let mut b = curve(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], false);
        b.merge(&curve(&[[0.0, 0.0, 1.0], [2.0, 0.0, 1.0]], false));
        let b = with_attr(b, "t", AttrClass::Point, AttrData::F32(vec![0.0, 10.0, 20.0, 0.0, 20.0]));
        let r = run(b.clone(), defaults()).expect("cooks");
        assert_eq!(r.point_count, 6);
        assert_eq!(
            r.attr(AttrClass::Point, "t").map(|a| a.data.clone()),
            Some(AttrData::F32(vec![0.0, 10.0, 20.0, 0.0, 20.0, 10.0]))
        );
        // Houdini order: originals first, the added midpoint last
        assert!((r.p()[5][0] - 1.0).abs() < 1e-6);
        // carry off
        let n = run(b, defaults().with("carryattrs", ParamValue::B(false))).expect("cooks");
        assert!(n.attr(AttrClass::Point, "t").is_none());
    }

    #[test]
    fn new_params_reject_bad_values() {
        for (k, v) in [
            ("divs", ParamValue::I(0)),
            ("divs", ParamValue::I(257)),
            ("order", ParamValue::Menu(5)),
            ("keepshape", ParamValue::I(1)),
        ] {
            assert!(
                matches!(run(stack(&[(0.0, 0.5), (1.0, 0.5)]), defaults().with(k, v)), Err(CookError::BadParam(_))),
                "{k}"
            );
        }
    }

    #[test]
    fn two_squares_loft_into_four_outward_quads() {
        // sections run top -> bottom: Houdini winds skin cells (k,j)(k,j+1)(k+1,j+1)(k+1,j), outward (clockwise-front) only that way
        let d = run(stack(&[(1.0, 0.5), (0.0, 0.5)]), defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (8, 4));
        let pos = positions(&d);
        for pr in &d.prims {
            let ids: Vec<usize> =
                d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&i| i as usize).collect();
            let nrm = prim_normal(&pos, &ids).expect("normal");
            let c: V3 = ids.iter().fold([0.0; 3], |a, &i| add(a, scale(pos[i], 0.25)));
            assert!(nrm[2].abs() < 1e-9 && nrm[0] * c[0] + nrm[1] * c[1] > 0.0);
        }
        let t = run(stack(&[(0.0, 0.5), (1.0, 0.5)]), defaults().with("surftype", ParamValue::Menu(1))).expect("cooks");
        assert_eq!(t.prim_count(), 8);
        assert!(t.prims.iter().all(|p| p.verts.len() == 3));
    }

    #[test]
    fn closev_wraps_three_sections_and_uv_spans_the_unit_square() {
        let s = stack(&[(0.0, 0.5), (1.0, 1.0), (2.0, 0.5)]);
        assert_eq!(run(s.clone(), defaults()).expect("cooks").prim_count(), 8);
        let d = run(s, defaults().with("closev", ParamValue::B(true))).expect("cooks");
        assert_eq!(d.prim_count(), 12);
        let uv = match d.attr(AttrClass::Vertex, "uv").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            o => panic!("{o:?}"),
        };
        assert_eq!(uv.len(), d.vertex_point.len());
        assert!(uv.iter().any(|c| c[0] == 1.0) && uv.iter().any(|c| c[1] == 1.0));
        // closev with only 2 sections is ignored (would double the same strip)
        assert_eq!(
            run(stack(&[(0.0, 1.0), (1.0, 1.0)]), defaults().with("closev", ParamValue::B(true)))
                .expect("cooks")
                .prim_count(),
            4
        );
    }

    #[test]
    fn mismatched_counts_resample_to_the_largest_or_fail() {
        let mut d = sq(0.0, 1.0);
        d.merge(&curve(&[[1.0, 0.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, -1.0, 1.0]], true));
        let o = run(d.clone(), defaults()).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (8, 4));
        // the resampled triangle keeps its first point and walks the loop
        assert_eq!(o.p()[4], [1.0, 0.0, 1.0]);
        assert!(matches!(run(d, defaults().with("resample", ParamValue::B(false))), Err(CookError::Failed(_))));
    }

    #[test]
    fn open_curves_make_an_open_sheet_and_mixed_closure_is_open() {
        let mut d = curve(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], false);
        d.merge(&curve(&[[0.0, 1.0, 0.0], [1.0, 1.0, 1.0], [2.0, 1.0, 0.0]], false));
        let o = run(d.clone(), defaults()).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (6, 2));
        d.merge(&sq(5.0, 1.0));
        let mixed = run(d, defaults()).expect("cooks");
        assert_eq!(mixed.prim_count(), 2 * 3, "mixed closure -> open loft over the largest count");
    }

    #[test]
    fn hostile_params_and_geometry() {
        for (k, v) in
            [("surftype", ParamValue::Menu(3)), ("closev", ParamValue::I(1)), ("resample", ParamValue::F(1.0))]
        {
            assert!(matches!(run(sq(0.0, 1.0), defaults().with(k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        assert_eq!(run(Detail::default(), defaults()).expect("cooks").prim_count(), 0);
        assert_eq!(run(sq(0.0, 1.0), defaults()).expect("one section").prim_count(), 0);
        let mut d = stack(&[(0.0, 1.0), (1.0, 1.0)]);
        d.vertex_point[0] = 99;
        assert_eq!(run(d, defaults()).expect("dangling section dropped").prim_count(), 0);
        let mut z = sq(0.0, 0.0);
        z.merge(&curve(&[[0.0; 3], [0.0; 3]], false));
        assert!(run(z, defaults()).is_ok());
    }

    use super::super::splinesurf::testkit::{area, nurbs, spline_curve};
    use hexe_geo::{Basis, PrimKind};

    /// Polygon sections + surftype nurbs: ONE ruled NurbsSurf (4 wrapped columns x 2 rows) with the exact box-wall area (4 sides of
    /// 1 x 1) and the v midpoint at z = 1/2.
    #[test]
    fn nurbs_surftype_lofts_polygon_sections_into_one_ruled_surface() {
        let d = run(stack(&[(0.0, 0.5), (1.0, 0.5)]), defaults().with("surftype", ParamValue::Menu(2))).expect("cooks");
        assert_eq!((d.prim_count(), d.point_count), (1, 8));
        assert!(matches!(&d.prims[0].kind, PrimKind::NurbsSurf { cols: 4, rows: 2, uwrap: true, vwrap: false, .. }));
        let t = d.tessellate(1, 1);
        assert!((area(&t) - 4.0).abs() < 1e-5, "area {}", area(&t));
        let m = d.evaluate_uv(0, 0.0, 0.5).expect("evaluates");
        assert!((m[2] - 0.5).abs() < 1e-6 && (m[0].abs() - 0.5).abs() < 1e-6, "{m:?}");
        // divs adds ruled stations: 3 divs over 1 pair = 4 rows
        let e = run(
            stack(&[(0.0, 0.5), (1.0, 0.5)]),
            defaults().with("surftype", ParamValue::Menu(2)).with("divs", ParamValue::I(3)),
        )
        .expect("cooks");
        assert!(matches!(&e.prims[0].kind, PrimKind::NurbsSurf { rows: 4, .. }));
    }

    /// NURBS sections keep their basis (auto with surftype quads): CV-row lerp = curve-wise lerp, so the surface at (u, 1/2) is the
    /// midpoint of the two section curves at u; mismatched bases are refused; triangles tessellates instead.
    #[test]
    fn nurbs_curve_sections_loft_into_a_nurbs_surface_sharing_their_basis() {
        let a = [[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [3.0, 2.0, 0.0], [4.0, 0.0, 0.0]];
        let b = a.map(|p| [p[0], p[1] * 2.0, 2.0]);
        let (ca, cb) = (spline_curve(&a, nurbs(3)), spline_curve(&b, nurbs(3)));
        let mut both = ca.clone();
        both.merge(&cb);
        let d = run(both.clone(), defaults()).expect("cooks");
        assert!(matches!(&d.prims[0].kind, PrimKind::NurbsSurf { cols: 4, rows: 2, ubasis, .. } if ubasis.order == 3));
        for u in [0.0f32, 0.3, 0.77, 1.0] {
            let (p, q) = (ca.evaluate(0, u).expect("a"), cb.evaluate(0, u).expect("b"));
            let s = d.evaluate_uv(0, u, 0.5).expect("surf");
            for k in 0..3 {
                assert!((s[k] - 0.5 * (p[k] + q[k])).abs() < 1e-5, "u {u} axis {k}: {s:?} vs {p:?} {q:?}");
            }
            // rows are the sections themselves
            assert!(d.evaluate_uv(0, u, 0.0).expect("v0").iter().zip(p).all(|(x, y)| (x - y).abs() < 1e-5));
            assert!(d.evaluate_uv(0, u, 1.0).expect("v1").iter().zip(q).all(|(x, y)| (x - y).abs() < 1e-5));
        }
        // mismatched orders are refused, never panic
        let mut mixed = ca.clone();
        mixed.merge(&spline_curve(&b, nurbs(4)));
        assert!(run(mixed, defaults()).is_err());
        // triangles = tessellate the splines, loft the polygons (open sections, 2 spans x 8 + 1 = 17 points each, 16 quads split in 2)
        let t = run(both, defaults().with("surftype", ParamValue::Menu(1))).expect("cooks");
        assert!(!t.has_spline_prims() && t.prim_count() == 16 * 2, "{}", t.prim_count());
        // Bezier sections give a BezierSurf
        let bz = |p: &[[f32; 3]; 4]| spline_curve(p, PrimKind::BezierCurve { basis: Basis::new(4), closed: false });
        let mut two = bz(&a);
        two.merge(&bz(&b));
        let r = run(two, defaults()).expect("cooks");
        assert!(matches!(&r.prims[0].kind, PrimKind::BezierSurf { cols: 4, rows: 2, .. }));
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/skin.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert!(out.prim_count() >= 12);
        assert_eq!(crate::registry().create("skin").map(|n| n.type_name()), Some("skin"));
    }

    /// promote-1: skin equals real Houdini 20.5 on equal-count sections (quads, triangles, closev); point/prim order + winding exact (computeuv off: Houdini writes no uv).
    #[test]
    fn skin_matches_houdini_on_recorded_oracle_cases() {
        use super::super::meshutil::testkit::houdini_twin;
        macro_rules! case {
            ($n:literal) => {
                (
                    $n,
                    houdini_twin(
                        include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                        include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
                    ),
                )
            };
        }
        for (n, r) in [case!("sop-skin-eq"), case!("sop-skin-tris"), case!("sop-skin-closev")] {
            assert_eq!(r, Ok(()), "{n}");
        }
    }

    /// Houdini 20.5 resample rule on Houdini's OWN f32 ring (raw points 12..19 of fixtures/sop-skin-2ring/houdini.geo, noise included): the 4 added
    /// midpoints are created on edges 6,4,1,3 in that order (longest f32 length^2 first, ties -> lower edge), appended after the originals.
    #[test]
    fn resample_splits_longest_f32_edges_first_on_houdinis_own_ring() {
        let ring: [V3; 8] = [
            [0.5, 1.0, 0.0],
            [0.353553385, 1.0, 0.353553385],
            [-2.18556941e-08, 1.0, 0.5],
            [-0.353553385, 1.0, 0.353553385],
            [-0.5, 1.0, -4.37113883e-08],
            [-0.353553325, 1.0, -0.353553444],
            [5.96244032e-09, 1.0, -0.5],
            [0.353553504, 1.0, -0.353553265],
        ];
        let ring: Vec<V3> = ring.iter().map(|q| q.map(|c| f64::from(c as f32))).collect();
        let (out, order) = resample(&ring, true, 12);
        assert_eq!(out.len(), 12);
        let made: Vec<V3> = order.iter().map(|&k| out[k].0).collect();
        let mid = |e: usize| scale(add(ring[e], ring[(e + 1) % 8]), 0.5);
        for (got, e) in made.iter().zip([6usize, 4, 1, 3]) {
            assert!(dist(*got, mid(e)) < 1e-9, "edge {e}: {got:?}");
        }
        // originals keep their relative ring order; added points sit between their edge ends
        assert_eq!(out[0].1, vec![(0, 1.0)]);
        assert_eq!(out[1].1, vec![(1, 1.0)]);
        assert_eq!(out[2].1.len(), 2);
    }

    /// KNOWN GAP (pinned): resampling 8 -> 12 points. RULE implemented (edge midpoints, longest f32 edge first, originals kept, added points appended)
    /// and proven on Houdini's own ring (test above); the residual P-DIFF is ONLY the tie order: Houdini's circle SOP points carry f32 noise from its
    /// rotation arithmetic (e.g. x = -2.1855694e-8 where hexe has 3.06e-17), hexe's circle is f64-exact, so the 8 equal octagon edges tie differently.
    /// 2-ring case: prims + 21 of 24 points equal.
    #[test]
    fn resample_of_mismatched_rings_is_a_known_gap_vs_houdini() {
        let r = super::super::meshutil::testkit::houdini_twin(
            include_str!("../../../../fixtures/sop-skin/scene.json"),
            include_bytes!("../../../../fixtures/sop-skin/houdini.geo"),
        );
        assert!(r.as_ref().is_err_and(|e| e.starts_with("P[")), "{r:?}");
    }
}
