//! Box SOP (wave 2): generator, `type = poly` only. Axis-aligned box of `size` (× `scale`) centred on `t`.
//!
//! Topology (Houdini poly box; DEFAULT box order verified against real Houdini 20.5, `fixtures/sop-box/houdini.geo`):
//! * lattice of `n = (nx, ny, nz)` points per axis, `n = 2` unless `dodivs` (then `divs`, values < 2 treated as 2);
//! * consolidated (`consolidatepts` on): only the lattice SURFACE points, one shared point each, y-layer by y-layer (low →
//!   high), per layer the ring `(-x,-z) → (+x,-z) → (+x,+z) → (-x,+z)` (default box = Houdini's bottom ring then top ring);
//! * `consolidatepts` off: every face owns its own `nu × nv` points (default box = 24 points; order UNVERIFIED);
//! * prims = quads, faces in Houdini's order -z,+x,+z,-x,-y,+y, each cell starts at Houdini's first corner (prim 0 = ids
//!   `[1,5,4,0]`, verbatim); the cycle runs clockwise seen from outside = Houdini's winding (outward normals under
//!   `meshutil::prim_normal`, the left-hand rule);
//! * attributes: `P` only.
//!
//! `divrate` (mesh types only in Houdini) is validated (hard min 1, allocation cap) but has no effect for `poly`.
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

/// Largest `nx*ny*nz` accepted (also bounds `divrate`); keeps every allocation far below memory limits.
const MAX_LATTICE: u64 = 1 << 26;
/// Largest point count / prim count the cook will allocate.
const MAX_ELEMS: u64 = 1 << 22;
/// Per-axis count above which a value is rejected before any arithmetic.
const MAX_AXIS: f32 = 1.0e7;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(0)).label("Primitive Type").menu(&[("poly", "Polygon")]),
        ParamSpec::new("consolidatepts", ParamValue::B(true)).label("Consolidate Corner Points"),
        ParamSpec::new("size", ParamValue::V3([1.0, 1.0, 1.0])).label("Size").range(-1.0, 1.0),
        ParamSpec::new("t", ParamValue::V3([0.0, 0.0, 0.0])).label("Center").range(-1.0, 1.0),
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Uniform Scale").range(0.0, 10.0),
        // integer[3] → V3 (no I3 variant): rounded on read.
        ParamSpec::new("divrate", ParamValue::V3([4.0, 4.0, 4.0])).label("Axis Divisions").range(1.0, 50.0),
        ParamSpec::new("dodivs", ParamValue::B(false)).label("Use Divisions"),
        ParamSpec::new("divs", ParamValue::V3([3.0, 3.0, 3.0])).label("Divisions").range(1.0, 50.0),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(BoxSop { specs: spec() })
}

struct BoxSop {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for BoxSop {
    fn type_name(&self) -> &'static str {
        "box"
    }
    /// Generator. (Houdini's optional bounding-source input is out of wave-2 scope.)
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, _: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        cook_box(ctx.params).map(Arc::new)
    }
}

fn bad(msg: impl Into<String>) -> CookError {
    CookError::BadParam(msg.into())
}
fn failed(msg: impl Into<String>) -> CookError {
    CookError::Failed(msg.into())
}
fn us(v: u64) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}
fn id(v: u64) -> Result<u32, CookError> {
    u32::try_from(v).map_err(|_| failed("point id overflow"))
}

/// Integer[3] read from a `V3`: rounded, each component ≥ 1 (hard min), product capped.
fn counts(p: &ParamSet, name: &str) -> Result<[u64; 3], CookError> {
    let v = p.vec3(name).ok_or_else(|| bad(format!("`{name}` missing or not a vector")))?;
    let mut out = [1u64; 3];
    for (o, c) in out.iter_mut().zip(v) {
        let r = c.round();
        if r.is_nan() || r < 1.0 {
            return Err(bad(format!("`{name}` components must be >= 1, got {c}")));
        }
        if r > MAX_AXIS {
            return Err(failed(format!("`{name}` component {c} exceeds the allocation cap")));
        }
        *o = r as u64;
    }
    let lattice = out[0].saturating_mul(out[1]).saturating_mul(out[2]);
    if lattice > MAX_LATTICE {
        return Err(failed(format!("`{name}` {out:?}: {lattice} cells exceeds the cap {MAX_LATTICE}")));
    }
    Ok(out)
}

/// Shared-point lattice in Houdini's layer order: y-layer by y-layer (low → high); per layer the boundary RING first
/// (start `(-x,-z)`, +x along the `-z` edge, +z along the `+x` edge, -x along the `+z` edge, -z back along the `-x` edge),
/// and on the two end layers (`y` low / high) the interior grid points after the ring (x-major, z fastest).
/// Derived from `fixtures/sop-box/houdini.geo` (default box); the divisions extension is UNVERIFIED (Houdini's own
/// `divsx/y/z` are segment counts with a different point set, see docs/HOUDINI-ORACLE.md).
struct Lattice {
    n: [u64; 3],
}

impl Lattice {
    /// Ring points per layer `2(nx-1) + 2(nz-1)`.
    fn ring(&self) -> u64 {
        let [x, _, z] = self.n;
        2 * (x - 1) + 2 * (z - 1)
    }
    /// Points of an end layer (ring + interior grid).
    fn end_layer(&self) -> u64 {
        let [x, _, z] = self.n;
        self.ring() + (x - 2) * (z - 2)
    }
    /// Surface point count `nx·ny·nz − (nx−2)(ny−2)(nz−2)` (all `n ≥ 2`).
    fn points(&self) -> u64 {
        let [_, y, _] = self.n;
        2 * self.end_layer() + (y - 2) * self.ring()
    }
    /// Point id of surface lattice coordinate `(i,j,k)` (caller guarantees it lies on the surface).
    fn index(&self, i: u64, j: u64, k: u64) -> u64 {
        let [x, y, z] = self.n;
        let base = if j == 0 {
            0
        } else if j == y - 1 {
            self.end_layer() + (y - 2) * self.ring()
        } else {
            self.end_layer() + (j - 1) * self.ring()
        };
        let within = if k == 0 {
            i
        } else if i == x - 1 {
            (x - 1) + k
        } else if k == z - 1 {
            (x - 1) + (z - 1) + (x - 1 - i)
        } else if i == 0 {
            2 * (x - 1) + (z - 1) + (z - 1 - k)
        } else {
            self.ring() + (i - 1) * (z - 2) + (k - 1)
        };
        base + within
    }
    /// `(i,k)` of one layer's points in id order (ring, then the interior grid on the two end layers).
    fn layer_order(&self, end: bool) -> Vec<(u64, u64)> {
        let [x, _, z] = self.n;
        let mut v: Vec<(u64, u64)> = (0..x).map(|i| (i, 0)).collect();
        v.extend((1..z).map(|k| (x - 1, k)));
        v.extend((0..x - 1).rev().map(|i| (i, z - 1)));
        v.extend((1..z - 1).rev().map(|k| (0, k)));
        if end {
            for i in 1..x - 1 {
                v.extend((1..z - 1).map(|k| (i, k)));
            }
        }
        v
    }
}

/// Position of lattice coordinate `c` (per-axis index `0..n`): centre + scale·size·(frac − ½).
fn pos(c: [u64; 3], n: [u64; 3], size: [f32; 3], t: [f32; 3], scale: f32) -> [f32; 3] {
    let mut o = [0.0f32; 3];
    for a in 0..3 {
        let (ci, ni, sz, tc) = (c.get(a).copied().unwrap_or(0), n.get(a).copied().unwrap_or(2), size.get(a), t.get(a));
        let frac = ci as f32 / (ni.max(2) - 1) as f32 - 0.5;
        if let (Some(slot), Some(sz), Some(tc)) = (o.get_mut(a), sz, tc) {
            *slot = tc + scale * sz * frac;
        }
    }
    o
}
/// One box face in Houdini's face order (-Z, +X, +Z, -X, -Y, +Y): fixed axis `f` at its low / high end, varying axes
/// `(u, v)`, and the four corners `(du, dv)` of a cell in HOUDINI's order (default box prim 0 = `[1,5,4,0]`).
struct Face {
    f: usize,
    high: bool,
    u: usize,
    v: usize,
    h: [[u64; 2]; 4],
}
const FACES: [Face; 6] = [
    Face { f: 2, high: false, u: 0, v: 1, h: [[1, 0], [1, 1], [0, 1], [0, 0]] },
    Face { f: 0, high: true, u: 2, v: 1, h: [[1, 0], [1, 1], [0, 1], [0, 0]] },
    Face { f: 2, high: true, u: 0, v: 1, h: [[0, 0], [0, 1], [1, 1], [1, 0]] },
    Face { f: 0, high: false, u: 2, v: 1, h: [[0, 0], [0, 1], [1, 1], [1, 0]] },
    Face { f: 1, high: false, u: 0, v: 2, h: [[1, 1], [1, 0], [0, 0], [0, 1]] },
    Face { f: 1, high: true, u: 0, v: 2, h: [[1, 0], [1, 1], [0, 1], [0, 0]] },
];
/// Houdini lists each cell CLOCKWISE seen from outside (its prim normal is the left-hand rule, `meshutil::prim_normal`);
/// hexe adopts that globally (H32), so the cell is emitted verbatim: `[h0, h1, h2, h3]`.
const CELL: [usize; 4] = [0, 1, 2, 3];

fn cook_box(p: &ParamSet) -> Result<Detail, CookError> {
    match p.menu("type") {
        Some(0) => {}
        Some(m) => return Err(bad(format!("primitive type menu {m}: unsupported in wave 2 (only `poly`)"))),
        None => return Err(bad("`type` missing or not a menu")),
    }
    let consolidate = p.toggle("consolidatepts").ok_or_else(|| bad("`consolidatepts` missing or not a toggle"))?;
    let size = p.vec3("size").ok_or_else(|| bad("`size` missing or not a vector"))?;
    let t = p.vec3("t").ok_or_else(|| bad("`t` missing or not a vector"))?;
    let scale = p.float("scale").ok_or_else(|| bad("`scale` missing or not a float"))?;
    let dodivs = p.toggle("dodivs").ok_or_else(|| bad("`dodivs` missing or not a toggle"))?;
    // validated even when unused (hard min 1 + allocation cap): hostile values never pass silently.
    let _divrate = counts(p, "divrate")?;
    let divs = counts(p, "divs")?;
    // divs = points per axis; hard min 1 allowed by the DS, a 1-point axis has no extent → treated as 2 (UNVERIFIED).
    let n = if dodivs { divs.map(|d| d.max(2)) } else { [2, 2, 2] };
    let [nx, ny, nz] = n;
    let prims = 2 * ((nx - 1) * (ny - 1) + (ny - 1) * (nz - 1) + (nx - 1) * (nz - 1));
    let lat = Lattice { n };
    let npts = if consolidate { lat.points() } else { 2 * (nx * ny + ny * nz + nx * nz) };
    if npts > MAX_ELEMS || prims > MAX_ELEMS {
        return Err(failed(format!("box {n:?} needs {npts} points / {prims} prims, cap is {MAX_ELEMS}")));
    }
    let mut d = Detail::default();
    let range = d.add_points(id(npts)?);
    debug_assert_eq!(u64::from(range.end - range.start), npts);
    let mut ps: Vec<[f32; 3]> = Vec::with_capacity(us(npts));
    if consolidate {
        for j in 0..ny {
            let end = j == 0 || j == ny - 1;
            ps.extend(lat.layer_order(end).into_iter().map(|(i, k)| pos([i, j, k], n, size, t, scale)));
        }
    } else {
        for fc in &FACES {
            let (nu, nv) = (n.get(fc.u).copied().unwrap_or(2), n.get(fc.v).copied().unwrap_or(2));
            let fixed = if fc.high { n.get(fc.f).copied().unwrap_or(2) - 1 } else { 0 };
            for iu in 0..nu {
                for iv in 0..nv {
                    let mut co = [0u64; 3];
                    for (axis, v) in [(fc.f, fixed), (fc.u, iu), (fc.v, iv)] {
                        if let Some(s) = co.get_mut(axis) {
                            *s = v;
                        }
                    }
                    ps.push(pos(co, n, size, t, scale));
                }
            }
        }
    }
    for (slot, v) in d.p_mut().iter_mut().zip(ps) {
        *slot = v;
    }
    let mut face_base = 0u64;
    for fc in &FACES {
        let (nu, nv) = (n.get(fc.u).copied().unwrap_or(2), n.get(fc.v).copied().unwrap_or(2));
        let fixed = if fc.high { n.get(fc.f).copied().unwrap_or(2) - 1 } else { 0 };
        for iu in 0..nu - 1 {
            for iv in 0..nv - 1 {
                let mut ids = [0u32; 4];
                for (slot, &w) in ids.iter_mut().zip(&CELL) {
                    let [du, dv] = fc.h.get(w).copied().unwrap_or([0, 0]);
                    let (cu, cv) = (iu + du, iv + dv);
                    *slot = if consolidate {
                        let mut co = [0u64; 3];
                        for (axis, v) in [(fc.f, fixed), (fc.u, cu), (fc.v, cv)] {
                            if let Some(s) = co.get_mut(axis) {
                                *s = v;
                            }
                        }
                        id(lat.index(co[0], co[1], co[2]))?
                    } else {
                        id(face_base + cu * nv + cv)?
                    };
                }
                d.add_poly(&ids, true);
            }
        }
        face_base += nu * nv;
    }
    Ok(d)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::{AttrClass, read_geo, write_geo_json};
    use std::collections::{BTreeMap, BTreeSet};

    fn ctx(p: &ParamSet) -> CookCtx<'_> {
        CookCtx { time: 0.0, fps: 24.0, params: p, log: &|_| {} }
    }
    fn defaults() -> ParamSet {
        spec().into_iter().fold(ParamSet::new(), |s, p| s.with(&p.name, p.default))
    }
    fn run(p: &ParamSet) -> Result<Arc<Detail>, CookError> {
        new().cook(&ctx(p), &[])
    }
    fn cook_scene(json: &str) -> Detail {
        let (mut g, id) = load_scene(json).expect("scene loads");
        (*g.cook(id, 0.0).expect("cooks")).clone()
    }
    fn quads(d: &Detail) -> Vec<Vec<u32>> {
        d.prims.iter().map(|p| d.vertex_point[p.verts.start as usize..p.verts.end as usize].to_vec()).collect()
    }
    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    /// Closed-form counts (independent of the cook code path): surface points / quads of an n-point lattice.
    fn cf_points(n: [u64; 3]) -> u64 {
        let inner = n.iter().map(|&v| v.saturating_sub(2)).product::<u64>();
        n.iter().product::<u64>() - inner
    }
    fn cf_quads(n: [u64; 3]) -> u64 {
        let s = n.map(|v| v - 1);
        2 * (s[0] * s[1] + s[1] * s[2] + s[0] * s[2])
    }

    /// INDEPENDENT expected builder (BTreeMap lattice, perimeter walk, no `Lattice::index`), consolidated points, Houdini
    /// order: layers low → high, ring from (0,0) walking +x,+z,-x,-z, then the end-layer interior; faces -z,+x,+z,-x,-y,+y.
    fn expected(n: [u64; 3], size: [f64; 3], t: [f64; 3], scale: f64) -> Detail {
        let at = |c: [u64; 3]| -> [f32; 3] {
            let f = |a: usize| (t[a] + scale * size[a] * (c[a] as f64 / (n[a] - 1) as f64 - 0.5)) as f32;
            [f(0), f(1), f(2)]
        };
        let mut ids = BTreeMap::new();
        let mut pts = Vec::new();
        for j in 0..n[1] {
            let (mut i, mut k) = (0u64, 0u64);
            let ring = 2 * (n[0] - 1) + 2 * (n[2] - 1);
            for _ in 0..ring {
                ids.insert([i, j, k], ids.len() as u32);
                pts.push(at([i, j, k]));
                // walk the perimeter: +x on k=0, +z on i=max, -x on k=max, -z on i=0
                if k == 0 && i < n[0] - 1 {
                    i += 1;
                } else if i == n[0] - 1 && k < n[2] - 1 {
                    k += 1;
                } else if k == n[2] - 1 && i > 0 {
                    i -= 1;
                } else {
                    k -= 1;
                }
            }
            if j == 0 || j == n[1] - 1 {
                for i in 1..n[0] - 1 {
                    for k in 1..n[2] - 1 {
                        ids.insert([i, j, k], ids.len() as u32);
                        pts.push(at([i, j, k]));
                    }
                }
            }
        }
        let mut d = Detail::default();
        d.add_points(pts.len() as u32);
        for (s, v) in d.p_mut().iter_mut().zip(pts) {
            *s = v;
        }
        let hi = |a: usize| n[a] - 1;
        // (fixed axis, fixed value, u axis, v axis, Houdini cell corners (du,dv))
        type Fc = (usize, u64, usize, usize, [(u64, u64); 4]);
        let cw_a: [(u64, u64); 4] = [(1, 0), (1, 1), (0, 1), (0, 0)];
        let cw_b: [(u64, u64); 4] = [(0, 0), (0, 1), (1, 1), (1, 0)];
        let faces: [Fc; 6] = [
            (2, 0, 0, 1, cw_a),
            (0, hi(0), 2, 1, cw_a),
            (2, hi(2), 0, 1, cw_b),
            (0, 0, 2, 1, cw_b),
            (1, 0, 0, 2, [(1, 1), (1, 0), (0, 0), (0, 1)]),
            (1, hi(1), 0, 2, cw_a),
        ];
        for (f, fv, u, v, cw) in faces {
            for iu in 0..n[u] - 1 {
                for iv in 0..n[v] - 1 {
                    let corner = |(du, dv): (u64, u64)| {
                        let mut co = [0u64; 3];
                        co[f] = fv;
                        co[u] = iu + du;
                        co[v] = iv + dv;
                        ids[&co]
                    };
                    // Houdini's clockwise cell, verbatim
                    d.add_poly(&[corner(cw[0]), corner(cw[1]), corner(cw[2]), corner(cw[3])], true);
                }
            }
        }
        d
    }
    fn assert_same(a: &Detail, b: &Detail) {
        assert_eq!(
            (a.point_count, a.vertex_count(), a.prim_count()),
            (b.point_count, b.vertex_count(), b.prim_count())
        );
        assert_eq!(quads(a), quads(b));
        assert!(a.p().iter().zip(b.p()).all(|(x, y)| near(*x, *y)), "P differs");
        let names = |d: &Detail| d.attrs.iter().map(|x| (x.class, x.name.clone())).collect::<Vec<_>>();
        assert_eq!(names(a), names(b));
    }

    #[test]
    fn default_box_is_8_points_6_quads_p_only() {
        let d = run(&defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (8, 6, 24));
        assert!(quads(&d).iter().all(|q| q.len() == 4));
        assert_eq!(d.attrs.len(), 1, "only P");
        assert!(d.attr(AttrClass::Point, "P").is_some());
        // closed form: P = ±size/2 + t; Houdini order: bottom ring (-x-z, +x-z, +x+z, -x+z) then top ring
        let mut want = Vec::new();
        for sy in [-0.5f32, 0.5] {
            for (sx, sz) in [(-0.5f32, -0.5f32), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
                want.push([sx, sy, sz]);
            }
        }
        assert_eq!(d.p(), want.as_slice());
        assert_eq!(d.p().first().copied(), Some([-0.5, -0.5, -0.5]));
    }

    #[test]
    fn size_center_scale_move_p_only() {
        let p = defaults().with("size", ParamValue::V3([2.0, 4.0, 6.0])).with("t", ParamValue::V3([1.0, -1.0, 0.5]));
        let d = run(&p).expect("cooks");
        let got: BTreeSet<[i64; 3]> = d.p().iter().map(|v| v.map(|c| (c * 1000.0).round() as i64)).collect();
        let mut want = BTreeSet::new();
        for sx in [-1.0f32, 1.0] {
            for sy in [-2.0f32, 2.0] {
                for sz in [-3.0f32, 3.0] {
                    want.insert(
                        [(1.0 + sx) * 1000.0, (-1.0 + sy) * 1000.0, (0.5 + sz) * 1000.0].map(|c| c.round() as i64),
                    );
                }
            }
        }
        assert_eq!(got, want);
        let base = run(&defaults()).expect("cooks");
        assert_eq!(quads(&d), quads(&base), "topology independent of size/center");
        // uniform scale about the centre; centre not scaled
        let s = defaults().with("scale", ParamValue::F(3.0)).with("t", ParamValue::V3([1.0, 0.0, 0.0]));
        assert_eq!(run(&s).expect("cooks").p().first().copied(), Some([-0.5, -1.5, -1.5]));
    }

    /// Closed manifold + outward winding for several lattices, both consolidated and not.
    #[test]
    fn closed_manifold_outward_winding() {
        for consolidate in [true, false] {
            for (dodivs, divs) in
                [(false, [3.0; 3]), (true, [3.0, 3.0, 3.0]), (true, [2.0, 3.0, 4.0]), (true, [5.0, 2.0, 2.0])]
            {
                let p = defaults()
                    .with("consolidatepts", ParamValue::B(consolidate))
                    .with("dodivs", ParamValue::B(dodivs))
                    .with("divs", ParamValue::V3(divs))
                    .with("size", ParamValue::V3([2.0, 3.0, 4.0]))
                    .with("t", ParamValue::V3([5.0, -2.0, 1.0]));
                let d = run(&p).expect("cooks");
                let c = [5.0f32, -2.0, 1.0];
                let mut edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
                for q in quads(&d) {
                    let v: Vec<[f32; 3]> = q.iter().map(|&i| d.p()[i as usize]).collect();
                    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                    let (e1, e2) = (sub(v[1], v[0]), sub(v[3], v[0]));
                    let n =
                        [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                    let mid = [0, 1, 2].map(|a| (v[0][a] + v[1][a] + v[2][a] + v[3][a]) / 4.0 - c[a]);
                    // right-handed e1 x e2 points INWARD for Houdini's clockwise-from-outside cells
                    assert!(n[0] * mid[0] + n[1] * mid[1] + n[2] * mid[2] < 0.0, "inward quad {q:?}");
                    for w in 0..4 {
                        *edges.entry((q[w], q[(w + 1) % 4])).or_default() += 1;
                    }
                }
                if consolidate {
                    // each directed edge once, its reverse once → every undirected edge shared by exactly 2 prims
                    for (&(a, b), &n) in &edges {
                        assert_eq!((n, edges.get(&(b, a)).copied()), (1, Some(1)), "edge {a}-{b}");
                    }
                }
            }
        }
    }

    #[test]
    fn divisions_match_closed_form_and_independent_builder() {
        for n in [[2u64, 2, 2], [3, 3, 3], [2, 3, 4], [4, 4, 4], [5, 2, 3], [6, 5, 7]] {
            let p = defaults().with("dodivs", ParamValue::B(true)).with("divs", ParamValue::V3(n.map(|v| v as f32)));
            let d = run(&p).expect("cooks");
            assert_eq!(u64::from(d.point_count), cf_points(n), "points {n:?}");
            assert_eq!(u64::from(d.prim_count()), cf_quads(n), "quads {n:?}");
            assert_eq!(u64::from(d.vertex_count()), 4 * cf_quads(n));
            assert_same(&d, &expected(n, [1.0; 3], [0.0; 3], 1.0));
        }
        // 3,3,3 = 26 points (3^3 − the single interior one), 24 quads
        assert_eq!((cf_points([3, 3, 3]), cf_quads([3, 3, 3])), (26, 24));
        // dodivs off → divs ignored
        let off = defaults().with("divs", ParamValue::V3([9.0, 9.0, 9.0]));
        assert_eq!(run(&off).expect("cooks").point_count, 8);
        // divs 1 on an axis behaves like 2 (UNVERIFIED)
        let one = defaults().with("dodivs", ParamValue::B(true)).with("divs", ParamValue::V3([1.0, 1.0, 1.0]));
        assert_eq!(run(&one).expect("cooks").point_count, 8);
    }

    #[test]
    fn divrate_is_ignored_for_poly_but_validated() {
        let base = run(&defaults()).expect("cooks");
        let p = defaults().with("divrate", ParamValue::V3([2.0, 2.0, 2.0]));
        assert_same(&run(&p).expect("cooks"), &base);
    }

    #[test]
    fn unconsolidated_faces_own_their_points() {
        let p = defaults().with("consolidatepts", ParamValue::B(false));
        let d = run(&p).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (24, 6));
        let uniq: BTreeSet<[i64; 3]> = d.p().iter().map(|v| v.map(|c| (c * 1000.0).round() as i64)).collect();
        assert_eq!(uniq.len(), 8, "still 8 distinct positions");
        let p = p.with("dodivs", ParamValue::B(true));
        let d = run(&p).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (54, 24), "6 faces × 3×3 points, 4 quads each");
    }

    #[test]
    fn hostile_params_are_errors() {
        let v3 = |a: f32| ParamValue::V3([a, a, a]);
        for (name, val, want_bad) in [
            ("divrate", v3(0.0), true),
            ("divrate", ParamValue::V3([4.0, -1.0, 4.0]), true),
            ("divrate", ParamValue::V3([4.0, f32::NAN, 4.0]), true),
            ("divrate", v3(10000.0), false),
            ("divrate", v3(1.0e9), false),
            ("divs", v3(0.0), true),
            ("divs", v3(-3.0), true),
            ("divs", v3(10000.0), false),
        ] {
            let r = run(&defaults().with(name, val.clone()));
            match r {
                Err(CookError::BadParam(_)) if want_bad => {}
                Err(CookError::Failed(_)) if !want_bad => {}
                other => panic!("{name}={val:?}: unexpected {other:?}"),
            }
        }
        // surface of a huge-but-in-lattice-cap box would still blow the element cap → Err, no allocation
        let big = defaults().with("dodivs", ParamValue::B(true)).with("divs", ParamValue::V3([1500.0, 1500.0, 2.0]));
        assert!(matches!(run(&big), Err(CookError::Failed(_))));
        // wrong typed / missing params
        assert!(matches!(run(&ParamSet::new()), Err(CookError::BadParam(_))));
        assert!(matches!(run(&defaults().with("size", ParamValue::F(1.0))), Err(CookError::BadParam(_))));
    }

    #[test]
    fn non_poly_types_are_unsupported_in_wave_2() {
        for m in 1..7 {
            match run(&defaults().with("type", ParamValue::Menu(m))) {
                Err(CookError::BadParam(msg)) => assert!(msg.contains("unsupported in wave 2"), "{msg}"),
                other => panic!("type {m}: {other:?}"),
            }
        }
    }

    #[test]
    fn scene_params_by_token_and_generator_shape() {
        assert!(new().inputs().is_empty());
        assert_eq!(new().type_name(), "box");
        let d = cook_scene(
            r#"{"nodes":[{"name":"b","type":"box","params":{"size":[2,2,2],"t":[0,1,0],"dodivs":1,"divs":[3,2,2]}}],"display":"b"}"#,
        );
        assert_eq!(u64::from(d.point_count), cf_points([3, 2, 2]));
        assert_eq!(d.bbox(), ([-1.0, 0.0, -1.0], [1.0, 2.0, 1.0]));
        // scene-level hostile: huge divrate cooks to Err
        let (mut g, id) =
            load_scene(r#"{"nodes":[{"name":"b","type":"box","params":{"divrate":[0,4,4]}}],"display":"b"}"#)
                .expect("loads");
        assert!(g.cook(id, 0.0).is_err());
        let (mut g, id) = load_scene(
            r#"{"nodes":[{"name":"b","type":"box","params":{"divrate":[10000,10000,10000]}}],"display":"b"}"#,
        )
        .expect("loads");
        assert!(g.cook(id, 0.0).is_err());
    }

    const HOUDINI_BOX: &[u8] = include_bytes!("../../../../fixtures/sop-box/houdini.geo");
    const DEFAULT_SCENE: &str = include_str!("../../../../fixtures/box-default/scene.json");
    const DIVS_SCENE: &str = include_str!("../../../../fixtures/box-divs222/scene.json");
    const DEFAULT_GEO: &[u8] = include_bytes!("../../../../fixtures/box-default/expected.geo");
    const DIVS_GEO: &[u8] = include_bytes!("../../../../fixtures/box-divs222/expected.geo");

    fn default_expected() -> Detail {
        expected([2, 2, 2], [1.0; 3], [0.0; 3], 1.0)
    }
    /// box-divs222: 2 segments per axis (divs 3,3,3), size (2,4,6), centre (1,-1,0.5); divrate 2,2,2 set but inert.
    fn divs_expected() -> Detail {
        expected([3, 3, 3], [2.0, 4.0, 6.0], [1.0, -1.0, 0.5], 1.0)
    }

    /// Real Houdini 20.5 default box (`fixtures/sop-box/houdini.geo`): P in index order, prim order and first corner equal;
    /// each cell is Houdini's clockwise cycle verbatim.
    #[test]
    fn default_box_matches_houdini_fixture_order() {
        let h = read_geo(HOUDINI_BOX).expect("houdini.geo parses");
        let d = run(&defaults()).expect("cooks");
        assert_eq!(d.p(), h.p());
        let (hq, dq) = (quads(&h), quads(&d));
        assert_eq!(hq.first(), Some(&vec![1, 5, 4, 0]));
        assert_eq!(hq.len(), dq.len());
        for (a, b) in hq.iter().zip(&dq) {
            assert_eq!(b, a);
        }
    }
    #[test]
    fn fixture_box_default_matches_analytical_expected() {
        let want = read_geo(DEFAULT_GEO).expect("expected.geo parses");
        assert_eq!((want.point_count, want.prim_count()), (8, 6));
        assert_same(&want, &default_expected());
        assert_same(&cook_scene(DEFAULT_SCENE), &want);
    }

    #[test]
    fn fixture_box_divs222_matches_analytical_expected() {
        let want = read_geo(DIVS_GEO).expect("expected.geo parses");
        assert_eq!((want.point_count, want.prim_count()), (26, 24));
        assert_same(&want, &divs_expected());
        assert_same(&cook_scene(DIVS_SCENE), &want);
    }

    /// `cargo test -p hexe-sop -- --ignored write_fixture` (re)writes the committed expected.geo from the closed form.
    #[test]
    #[ignore = "writes fixtures/*/expected.geo; run manually"]
    fn write_fixture() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures");
        for (case, d) in [("box-default", default_expected()), ("box-divs222", divs_expected())] {
            std::fs::write(format!("{root}/{case}/expected.geo"), write_geo_json(&d)).expect("write expected.geo");
        }
    }
}
