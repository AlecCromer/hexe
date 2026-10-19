//! Box SOP: generator. Box of `size` (× `scale`) rotated by `r` (degrees, X then Y then Z) and centred on `t`:
//! `P = t + Rz·Ry·Rx·(scale · size · frac)`, `frac ∈ [-½, ½]` per axis (verified against real Houdini 20.5,
//! `fixtures/sop-box-polymesh/`, recorded by `oracle/houdini/box/sop_box_types.py`).
//!
//! `type` menu = Houdini 20.5's item order: poly, polymesh, mesh, nurbs, bezier, points, prim.
//! * **poly** (`dodivs` off): 8 shared corners (bottom ring `(-x,-z) → (+x,-z) → (+x,+z) → (-x,+z)`, then the top ring),
//!   6 quads in face order -z,+x,+z,-x,-y,+y, each Houdini's clockwise cycle verbatim (prim 0 = `[1,5,4,0]`,
//!   `fixtures/sop-box/houdini.geo`); `consolidatepts` off → each face owns its own 4 points.
//! * **poly** + `dodivs`: Houdini's divisions box is a LATTICE, not a surface: `divs` = SEGMENTS per axis, all
//!   `(dx+1)(dy+1)(dz+1)` points (x outer, y, z inner), then OPEN polylines along x (for each y, z), along y (for each x,
//!   z), along z (for each x, y). `consolidatepts` has no effect; `rebar` (extra diagonals) is unsupported.
//! * **polymesh**: `divrate` = POINTS per axis (1 = that axis collapsed onto the centre, still 2 rows); `dodivs` is
//!   ignored. Surface points: +z face (rows y↑, x↓), -z face (rows y↑, x↑), +y interior rows (z↑, x↑), -y interior rows
//!   (z↑, x↓), +x interior (y↑, z↑), -x interior (y↑, z↓). Quads: the +z grid, the -z → +y → +z-top band, the -z-bottom
//!   → -y → +z-bottom band (x↓), the +x face (rows y↑, z↑), the -x face (rows y↑, z↓). Only `surftype` quads and
//!   `consolidatepts` on are supported.
//! * **points**: the polymesh point set (or, with `dodivs`, the lattice points) and no prims.
//! * mesh / nurbs / bezier / prim (Mesh, NURBS/Bezier surface, Hexahedron prims): no hexe prim type → `BadParam`.
//!
//! Attributes: `P` only (`vertexnormals` = vertex N is unsupported). `orderrate` / `orientedbbox` / the bounding input
//! are not modelled (spline types and the input are out of scope).
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

/// Largest `nx*ny*nz` accepted (also bounds `divrate`); keeps every allocation far below memory limits.
const MAX_LATTICE: u64 = 1 << 26;
/// Largest point / prim / vertex count the cook will allocate.
const MAX_ELEMS: u64 = 1 << 22;
/// Per-axis count above which a value is rejected before any arithmetic.
const MAX_AXIS: f32 = 1.0e7;

/// Houdini 20.5 `box` `type` menu, item order verbatim (`.hip` menu indexes map 1:1).
const TYPES: [(&str, &str); 7] = [
    ("poly", "Polygon"),
    ("polymesh", "Polygon Mesh"),
    ("mesh", "Mesh"),
    ("nurbs", "NURBS"),
    ("bezier", "Bezier"),
    ("points", "Points"),
    ("prim", "Primitive"),
];
/// Houdini 20.5 `box` `surftype` menu, item order verbatim (default 4 = quads).
const SURFTYPES: [(&str, &str); 7] = [
    ("rows", "Rows"),
    ("cols", "Columns"),
    ("rowcol", "Rows and Columns"),
    ("triangles", "Triangles"),
    ("quads", "Quadrilaterals"),
    ("alttriangles", "Alternating Triangles"),
    ("revtriangles", "Reverse Triangles"),
];
const QUADS: u32 = 4;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(0)).label("Primitive Type").menu(&TYPES),
        ParamSpec::new("surftype", ParamValue::Menu(QUADS)).label("Connectivity").menu(&SURFTYPES),
        ParamSpec::new("consolidatepts", ParamValue::B(true)).label("Consolidate Corner Points"),
        ParamSpec::new("size", ParamValue::V3([1.0, 1.0, 1.0])).label("Size").range(-1.0, 1.0),
        ParamSpec::new("t", ParamValue::V3([0.0, 0.0, 0.0])).label("Center").range(-1.0, 1.0),
        ParamSpec::new("r", ParamValue::V3([0.0, 0.0, 0.0])).label("Rotate").range(0.0, 360.0),
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Uniform Scale").range(0.0, 10.0),
        // integer[3] → V3 (no I3 variant): rounded on read.
        ParamSpec::new("divrate", ParamValue::V3([4.0, 4.0, 4.0])).label("Axis Divisions").range(1.0, 50.0),
        ParamSpec::new("dodivs", ParamValue::B(false)).label("Use Divisions"),
        ParamSpec::new("divs", ParamValue::V3([3.0, 3.0, 3.0])).label("Divisions").range(1.0, 50.0),
        ParamSpec::new("rebar", ParamValue::B(false)).label("Enforce Bracing"),
        ParamSpec::new("vertexnormals", ParamValue::B(false)).label("Add Vertex Normals"),
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
    /// Generator. (Houdini's optional bounding-source input is out of scope.)
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

/// Integer[3] read from a `V3`: rounded, each component ≥ 1 (Houdini's strict hard min), product capped.
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
fn finite3(p: &ParamSet, name: &str) -> Result<[f32; 3], CookError> {
    let v = p.vec3(name).ok_or_else(|| bad(format!("`{name}` missing or not a vector")))?;
    if v.iter().all(|c| c.is_finite()) { Ok(v) } else { Err(bad(format!("`{name}` must be finite, got {v:?}"))) }
}
fn toggle(p: &ParamSet, name: &str) -> Result<bool, CookError> {
    p.toggle(name).ok_or_else(|| bad(format!("`{name}` missing or not a toggle")))
}
fn check_elems(what: &str, n: [u64; 3], counts: &[u64]) -> Result<(), CookError> {
    if counts.iter().any(|&c| c > MAX_ELEMS) {
        return Err(failed(format!("{what} {n:?} needs {counts:?} points / prims / vertices, cap is {MAX_ELEMS}")));
    }
    Ok(())
}

/// Box transform: `P = t + Rz·Ry·Rx·(scale · size · frac)` (Houdini: srt, rotate xyz, degrees), evaluated in f64.
struct Xform {
    half: [f64; 3],
    t: [f64; 3],
    rot: [[f64; 3]; 3],
}
impl Xform {
    fn new(size: [f32; 3], t: [f32; 3], r: [f32; 3], scale: f32) -> Xform {
        let (sx, cx) = f64::from(r[0]).to_radians().sin_cos();
        let (sy, cy) = f64::from(r[1]).to_radians().sin_cos();
        let (sz, cz) = f64::from(r[2]).to_radians().sin_cos();
        // Rz·Ry·Rx (column vectors): rotate about X first, then Y, then Z.
        let rot = [
            [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
            [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
            [-sy, cy * sx, cy * cx],
        ];
        Xform { half: size.map(|s| f64::from(scale) * f64::from(s)), t: t.map(f64::from), rot }
    }
    /// Lattice coordinate `c` on an `n`-point-per-axis lattice (`n ≤ 1` = flat axis at the centre).
    fn at(&self, c: [u64; 3], n: [u64; 3]) -> [f32; 3] {
        let mut l = [0.0f64; 3];
        for (a, slot) in l.iter_mut().enumerate() {
            let (ci, ni, h) = (c[a], n[a], self.half[a]);
            let frac = if ni <= 1 { 0.0 } else { ci as f64 / (ni - 1) as f64 - 0.5 };
            *slot = h * frac;
        }
        let mut o = [0.0f32; 3];
        for (a, slot) in o.iter_mut().enumerate() {
            let row = self.rot[a];
            *slot = (self.t[a] + row[0] * l[0] + row[1] * l[1] + row[2] * l[2]) as f32;
        }
        o
    }
}

/// One `poly` box face in Houdini's face order (-Z, +X, +Z, -X, -Y, +Y): fixed axis `f` at its low / high end, varying
/// axes `(u, v)`, and the four corners `(du, dv)` of the face in HOUDINI's clockwise order (prim 0 = `[1,5,4,0]`).
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
/// Consolidated corner id of `(i, j, k) ∈ {0,1}³`: bottom ring then top ring, ring `(0,0) (1,0) (1,1) (0,1)` in `(x,z)`.
fn corner_id(c: [u64; 3]) -> u64 {
    let ring = match (c[0], c[2]) {
        (0, 0) => 0,
        (_, 0) => 1,
        (0, _) => 3,
        _ => 2,
    };
    4 * c[1] + ring
}

/// `type = poly`, `dodivs` off: 8 corners (or 24 face-owned points) + 6 quads.
fn poly_box(xf: &Xform, consolidate: bool) -> Result<Detail, CookError> {
    const N: [u64; 3] = [2, 2, 2];
    let mut d = Detail::default();
    let mut ps = Vec::with_capacity(24);
    if consolidate {
        for j in 0..2 {
            for (i, k) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
                ps.push(xf.at([i, j, k], N));
            }
        }
    } else {
        for fc in &FACES {
            for iu in 0..2 {
                for iv in 0..2 {
                    let mut co = [0u64; 3];
                    co[fc.f] = u64::from(fc.high);
                    co[fc.u] = iu;
                    co[fc.v] = iv;
                    ps.push(xf.at(co, N));
                }
            }
        }
    }
    d.add_points(id(ps.len() as u64)?);
    *d.p_mut() = ps;
    for (face, fc) in FACES.iter().enumerate() {
        let mut ids = [0u32; 4];
        for (slot, [du, dv]) in ids.iter_mut().zip(fc.h) {
            *slot = if consolidate {
                let mut co = [0u64; 3];
                co[fc.f] = u64::from(fc.high);
                co[fc.u] = du;
                co[fc.v] = dv;
                id(corner_id(co))?
            } else {
                id(4 * face as u64 + du * 2 + dv)?
            };
        }
        d.add_poly(&ids, true);
    }
    Ok(d)
}

/// `dodivs` lattice: `m` points per axis, ids `i·m1·m2 + j·m2 + k`.
fn lattice_points(xf: &Xform, m: [u64; 3]) -> Result<Detail, CookError> {
    let mut d = Detail::default();
    let mut ps = Vec::with_capacity(us(m[0] * m[1] * m[2]));
    for i in 0..m[0] {
        for j in 0..m[1] {
            for k in 0..m[2] {
                ps.push(xf.at([i, j, k], m));
            }
        }
    }
    d.add_points(id(ps.len() as u64)?);
    *d.p_mut() = ps;
    Ok(d)
}
/// `type = poly` + `dodivs`: the lattice points, then open polylines along x, y, z.
fn lattice_box(xf: &Xform, m: [u64; 3]) -> Result<Detail, CookError> {
    let [m0, m1, m2] = m;
    let pid = |i: u64, j: u64, k: u64| id(i * m1 * m2 + j * m2 + k);
    let mut d = lattice_points(xf, m)?;
    let mut line = Vec::new();
    for j in 0..m1 {
        for k in 0..m2 {
            line.clear();
            for i in 0..m0 {
                line.push(pid(i, j, k)?);
            }
            d.add_poly(&line, false);
        }
    }
    for i in 0..m0 {
        for k in 0..m2 {
            line.clear();
            for j in 0..m1 {
                line.push(pid(i, j, k)?);
            }
            d.add_poly(&line, false);
        }
    }
    for i in 0..m0 {
        for j in 0..m1 {
            line.clear();
            for k in 0..m2 {
                line.push(pid(i, j, k)?);
            }
            d.add_poly(&line, false);
        }
    }
    Ok(d)
}

/// Polygon-mesh surface lattice, `n` = points per axis (each ≥ 2), point ids in Houdini's order (module doc).
struct Mesh {
    n: [u64; 3],
}
impl Mesh {
    fn points(&self) -> u64 {
        let [x, y, z] = self.n;
        2 * x * y + 2 * x * (z - 2) + 2 * (y - 2) * (z - 2)
    }
    fn prims(&self) -> u64 {
        let [x, y, z] = self.n;
        (x - 1) * (y - 1) + (y + z - 2) * (x - 1) + (z - 1) * (x - 1) + 2 * (y - 1) * (z - 1)
    }
    /// Point id of the surface lattice coordinate `(i, j, k)` (caller guarantees it lies on the surface).
    fn id(&self, i: u64, j: u64, k: u64) -> u64 {
        let [x, y, z] = self.n;
        let face = x * y;
        let band = x * (z - 2);
        let side = (y - 2) * (z - 2);
        if k == z - 1 {
            j * x + (x - 1 - i)
        } else if k == 0 {
            face + j * x + i
        } else if j == y - 1 {
            2 * face + (k - 1) * x + i
        } else if j == 0 {
            2 * face + band + (k - 1) * x + (x - 1 - i)
        } else if i == x - 1 {
            2 * face + 2 * band + (j - 1) * (z - 2) + (k - 1)
        } else {
            2 * face + 2 * band + side + (j - 1) * (z - 2) + (z - 2 - k)
        }
    }
    /// Lattice coordinates in point-id order.
    fn coords(&self) -> Vec<[u64; 3]> {
        let [x, y, z] = self.n;
        let mut v = Vec::with_capacity(us(self.points()));
        for j in 0..y {
            v.extend((0..x).rev().map(|i| [i, j, z - 1]));
        }
        for j in 0..y {
            v.extend((0..x).map(|i| [i, j, 0]));
        }
        for k in 1..z - 1 {
            v.extend((0..x).map(|i| [i, y - 1, k]));
        }
        for k in 1..z - 1 {
            v.extend((0..x).rev().map(|i| [i, 0, k]));
        }
        for j in 1..y - 1 {
            v.extend((1..z - 1).map(|k| [x - 1, j, k]));
        }
        for j in 1..y - 1 {
            v.extend((1..z - 1).rev().map(|k| [0, j, k]));
        }
        v
    }
}
/// Quads between consecutive rows of a `rows × cols` grid: `[at(r,c), at(r,c+1), at(r+1,c+1), at(r+1,c)]`.
fn grid(d: &mut Detail, rows: u64, cols: u64, at: impl Fn(u64, u64) -> u64) -> Result<(), CookError> {
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            d.add_poly(&[id(at(r, c))?, id(at(r, c + 1))?, id(at(r + 1, c + 1))?, id(at(r + 1, c))?], true);
        }
    }
    Ok(())
}
/// `type = polymesh` (or `points` without `dodivs` when `with_prims` is false).
fn mesh_box(xf: &Xform, divrate: [u64; 3], with_prims: bool) -> Result<Detail, CookError> {
    let n = divrate.map(|v| v.max(2));
    let m = Mesh { n };
    check_elems("polymesh box", divrate, &[m.points(), m.prims(), 4 * m.prims()])?;
    let mut d = Detail::default();
    let ps: Vec<[f32; 3]> = m.coords().into_iter().map(|c| xf.at(c, divrate)).collect();
    d.add_points(id(ps.len() as u64)?);
    *d.p_mut() = ps;
    if !with_prims {
        return Ok(d);
    }
    let [x, y, z] = n;
    // +z face: rows y↑, x↓
    grid(&mut d, y, x, |r, c| m.id(x - 1 - c, r, z - 1))?;
    // band -z (y↑) → +y (z↑) → +z top row, x↑
    grid(&mut d, y + z - 1, x, |r, c| m.id(c, r.min(y - 1), r.saturating_sub(y - 1)))?;
    // band -z bottom row → -y (z↑) → +z bottom row, x↓
    grid(&mut d, z, x, |s, c| m.id(x - 1 - c, 0, s))?;
    // +x face: rows y↑, z↑; -x face: rows y↑, z↓
    grid(&mut d, y, z, |j, c| m.id(x - 1, j, c))?;
    grid(&mut d, y, z, |j, c| m.id(0, j, z - 1 - c))?;
    Ok(d)
}

fn cook_box(p: &ParamSet) -> Result<Detail, CookError> {
    let ty = p.menu("type").ok_or_else(|| bad("`type` missing or not a menu"))?;
    let surftype = p.menu("surftype").ok_or_else(|| bad("`surftype` missing or not a menu"))?;
    let consolidate = toggle(p, "consolidatepts")?;
    let size = finite3(p, "size")?;
    let t = finite3(p, "t")?;
    let r = finite3(p, "r")?;
    let scale = p.float("scale").ok_or_else(|| bad("`scale` missing or not a float"))?;
    if !scale.is_finite() {
        return Err(bad(format!("`scale` must be finite, got {scale}")));
    }
    let dodivs = toggle(p, "dodivs")?;
    let rebar = toggle(p, "rebar")?;
    if toggle(p, "vertexnormals")? {
        return Err(bad("`vertexnormals` (vertex N): unsupported"));
    }
    // validated whatever the type (hard min 1 + allocation cap): hostile values never pass silently.
    let divrate = counts(p, "divrate")?;
    let divs = counts(p, "divs")?;
    let xf = Xform::new(size, t, r, scale);
    // dodivs lattice: `divs` = segments per axis.
    let lattice = divs.map(|v| v + 1);
    let lattice_ok = || {
        let pts = lattice[0] * lattice[1] * lattice[2];
        let lines = lattice[0] * lattice[1] + lattice[1] * lattice[2] + lattice[0] * lattice[2];
        check_elems("divisions lattice", divs, &[pts, lines, 3 * pts])
    };
    match ty {
        0 if dodivs => {
            if rebar {
                return Err(bad("`rebar` (bracing diagonals of the divisions lattice): unsupported"));
            }
            lattice_ok()?;
            lattice_box(&xf, lattice)
        }
        0 => poly_box(&xf, consolidate),
        1 => {
            if surftype != QUADS {
                let name = SURFTYPES.get(surftype as usize).map_or("?", |s| s.0);
                return Err(bad(format!("polymesh connectivity `surftype` {surftype} ({name}): only quads supported")));
            }
            if !consolidate {
                return Err(bad("polymesh with `consolidatepts` off (face-owned points): unsupported"));
            }
            mesh_box(&xf, divrate, true)
        }
        5 if dodivs => {
            lattice_ok()?;
            lattice_points(&xf, lattice)
        }
        5 if !consolidate => Err(bad("points box with `consolidatepts` off (face-owned points): unsupported")),
        5 => mesh_box(&xf, divrate, false),
        2 => Err(bad("box type 2 (Mesh): hexe-geo has no Mesh primitive")),
        3 => Err(bad("box type 3 (NURBS): NURBS surface box unsupported (orderrate / knots not modelled)")),
        4 => Err(bad("box type 4 (Bezier): Bezier surface box unsupported (orderrate not modelled)")),
        6 => Err(bad("box type 6 (Primitive): hexe-geo has no Hexahedron primitive")),
        m => Err(bad(format!("box type menu {m}: out of range (0..=6)"))),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::{AttrClass, PrimKind, read_geo, write_geo_json};
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
    fn polys(d: &Detail) -> Vec<Vec<u32>> {
        d.prims.iter().map(|p| d.vertex_point[p.verts.start as usize..p.verts.end as usize].to_vec()).collect()
    }
    fn kinds(d: &Detail) -> Vec<PrimKind> {
        d.prims.iter().map(|p| p.kind.clone()).collect()
    }
    fn near(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }
    /// Same point count, P in index order to `tol`, prim kinds and vertex lists exact, attribute names equal.
    fn assert_same(a: &Detail, b: &Detail, tol: f32, what: &str) {
        assert_eq!(
            (a.point_count, a.vertex_count(), a.prim_count()),
            (b.point_count, b.vertex_count(), b.prim_count()),
            "{what}: counts"
        );
        assert_eq!(polys(a), polys(b), "{what}: vertex lists");
        assert_eq!(kinds(a), kinds(b), "{what}: prim kinds");
        for (i, (x, y)) in a.p().iter().zip(b.p()).enumerate() {
            assert!(near(*x, *y, tol), "{what}: P[{i}] {x:?} vs {y:?}");
        }
        let names = |d: &Detail| d.attrs.iter().map(|x| (x.class, x.name.clone())).collect::<Vec<_>>();
        assert_eq!(names(a), names(b), "{what}: attributes");
    }

    /// Houdini 20.5 cases, `oracle/houdini/box/sop_box_types.py` (cases.json = the parms each `<case>_houdini.geo` used).
    const CASES_JSON: &str = include_str!("../../../../fixtures/sop-box-polymesh/cases.json");
    fn houdini_geo(case: &str) -> &'static [u8] {
        macro_rules! geo {
            ($($c:literal),*) => {
                match case {
                    $($c => include_bytes!(concat!("../../../../fixtures/sop-box-polymesh/", $c, "_houdini.geo")),)*
                    other => panic!("no fixture {other}"),
                }
            };
        }
        geo!(
            "pm_default",
            "pm_div111",
            "pm_flat_y",
            "pm_div2",
            "pm_nonuniform",
            "pm_rot_scale",
            "pm_neg_dodivs",
            "pm_dense",
            "pts_divrate",
            "pts_dodivs",
            "pts_dodivs_noconsol",
            "poly_rot_scale",
            "poly_dodivs333",
            "poly_dodivs_mixed"
        )
    }
    /// cases.json parms → a full hexe ParamSet (defaults + overrides; Houdini ints → V3 / Menu / B).
    fn case_params(parms: &serde_json::Value) -> ParamSet {
        let mut p = defaults();
        for (k, v) in parms.as_object().expect("case is an object") {
            let f = |x: &serde_json::Value| x.as_f64().expect("number") as f32;
            let val = match k.as_str() {
                "type" | "surftype" => ParamValue::Menu(v.as_u64().expect("menu index") as u32),
                "consolidatepts" | "dodivs" | "rebar" | "vertexnormals" => ParamValue::B(f(v) != 0.0),
                "scale" => ParamValue::F(f(v)),
                _ => {
                    let a = v.as_array().expect("tuple");
                    ParamValue::V3([f(&a[0]), f(&a[1]), f(&a[2])])
                }
            };
            p = p.with(k, val);
        }
        p
    }

    #[test]
    fn houdini_cases_match_exactly() {
        let cases: serde_json::Value = serde_json::from_str(CASES_JSON).expect("cases.json parses");
        let cases = cases.as_object().expect("object");
        assert_eq!(cases.len(), 14);
        for (name, parms) in cases {
            let want = read_geo(houdini_geo(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let got = run(&case_params(parms)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_same(&got, &want, 1e-6, name);
        }
    }

    /// Spot checks of the recorded fixtures, independent of the cook code: counts and the documented corner cases.
    #[test]
    fn houdini_fixture_facts() {
        let h = |c: &str| read_geo(houdini_geo(c)).expect("parses");
        let d = h("pm_default");
        assert_eq!((d.point_count, d.prim_count()), (56, 54));
        assert_eq!(polys(&d).first(), Some(&vec![0, 1, 5, 4]));
        assert_eq!(d.p().first().copied(), Some([0.5, -0.5, 0.5]));
        // divrate 1: every point at the centre, size ignored on a flat axis
        let d = h("pm_div111");
        assert_eq!((d.point_count, d.prim_count()), (8, 6));
        assert!(d.p().iter().all(|v| *v == [0.0; 3]));
        // dodivs ignored by polymesh
        assert_eq!(h("pm_neg_dodivs").point_count, 24);
        // poly + dodivs: (3+1)^3 lattice, 3·16 open polylines
        let d = h("poly_dodivs333");
        assert_eq!((d.point_count, d.prim_count()), (64, 48));
        assert!(kinds(&d).iter().all(|k| *k == PrimKind::Poly { closed: false }));
        assert_eq!(h("pts_dodivs").prim_count(), 0);
    }

    #[test]
    fn spec_menus_follow_houdini_item_order() {
        let s = spec();
        let menu = |n: &str| {
            s.iter().find(|p| p.name == n).expect("parm").menu.iter().map(|m| m.0.clone()).collect::<Vec<_>>()
        };
        assert_eq!(menu("type"), ["poly", "polymesh", "mesh", "nurbs", "bezier", "points", "prim"]);
        assert_eq!(menu("surftype"), ["rows", "cols", "rowcol", "triangles", "quads", "alttriangles", "revtriangles"]);
        // the recorded .hip menu table agrees, so `.hip` tokens and indexes land on the same item
        for (i, tok) in menu("type").iter().enumerate() {
            assert_eq!(crate::hip_import::menu_position("box", "type", tok), Some(i), "{tok}");
        }
    }

    #[test]
    fn default_box_is_8_points_6_quads_p_only() {
        let d = run(&defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (8, 6, 24));
        assert!(polys(&d).iter().all(|q| q.len() == 4));
        assert_eq!(d.attrs.len(), 1, "only P");
        assert!(d.attr(AttrClass::Point, "P").is_some());
        let mut want = Vec::new();
        for sy in [-0.5f32, 0.5] {
            for (sx, sz) in [(-0.5f32, -0.5f32), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
                want.push([sx, sy, sz]);
            }
        }
        assert_eq!(d.p(), want.as_slice());
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
        assert_eq!(polys(&d), polys(&base), "topology independent of size/center");
        let s = defaults().with("scale", ParamValue::F(3.0)).with("t", ParamValue::V3([1.0, 0.0, 0.0]));
        assert_eq!(run(&s).expect("cooks").p().first().copied(), Some([-0.5, -1.5, -1.5]));
    }

    /// Closed manifold + Houdini (clockwise-from-outside) winding for poly and polymesh boxes.
    #[test]
    fn closed_manifold_outward_winding() {
        let c = [5.0f32, -2.0, 1.0];
        let mut params = vec![defaults(), defaults().with("consolidatepts", ParamValue::B(false))];
        for n in [[2.0, 2.0, 2.0], [3.0, 3.0, 3.0], [2.0, 3.0, 4.0], [5.0, 2.0, 2.0], [4.0, 6.0, 3.0]] {
            params.push(defaults().with("type", ParamValue::Menu(1)).with("divrate", ParamValue::V3(n)));
        }
        for (pi, p) in params.into_iter().enumerate() {
            let p = p.with("size", ParamValue::V3([2.0, 3.0, 4.0])).with("t", ParamValue::V3(c));
            let d = run(&p).expect("cooks");
            let mut edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
            for q in polys(&d) {
                let v: Vec<[f32; 3]> = q.iter().map(|&i| d.p()[i as usize]).collect();
                let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                let (e1, e2) = (sub(v[1], v[0]), sub(v[3], v[0]));
                let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                let mid = [0, 1, 2].map(|a| (v[0][a] + v[1][a] + v[2][a] + v[3][a]) / 4.0 - c[a]);
                // right-handed e1 x e2 points INWARD for Houdini's clockwise-from-outside cells
                assert!(n[0] * mid[0] + n[1] * mid[1] + n[2] * mid[2] < 0.0, "case {pi}: inward quad {q:?}");
                for w in 0..4 {
                    *edges.entry((q[w], q[(w + 1) % 4])).or_default() += 1;
                }
            }
            if pi != 1 {
                for (&(a, b), &n) in &edges {
                    assert_eq!((n, edges.get(&(b, a)).copied()), (1, Some(1)), "case {pi}: edge {a}-{b}");
                }
            }
        }
    }

    /// `Mesh::id` agrees with the point order of `Mesh::coords` and the closed-form counts on many lattices.
    #[test]
    fn polymesh_ids_and_counts_closed_form() {
        for n in [[2u64, 2, 2], [3, 4, 5], [5, 2, 3], [2, 7, 2], [6, 5, 7]] {
            let m = Mesh { n };
            let co = m.coords();
            assert_eq!(co.len() as u64, m.points());
            let inner = n.iter().map(|&v| v - 2).product::<u64>();
            assert_eq!(m.points(), n.iter().product::<u64>() - inner, "{n:?}");
            let s = n.map(|v| v - 1);
            assert_eq!(m.prims(), 2 * (s[0] * s[1] + s[1] * s[2] + s[0] * s[2]), "{n:?}");
            for (pid, c) in co.iter().enumerate() {
                assert_eq!(m.id(c[0], c[1], c[2]), pid as u64, "{n:?} {c:?}");
            }
            let p = defaults().with("type", ParamValue::Menu(1)).with("divrate", ParamValue::V3(n.map(|v| v as f32)));
            assert_eq!(u64::from(run(&p).expect("cooks").prim_count()), m.prims());
        }
    }

    #[test]
    fn divrate_is_ignored_for_poly_and_dodivs_for_polymesh() {
        let base = run(&defaults()).expect("cooks");
        let p = defaults().with("divrate", ParamValue::V3([2.0, 7.0, 2.0]));
        assert_same(&run(&p).expect("cooks"), &base, 0.0, "poly divrate");
        let pm = defaults().with("type", ParamValue::Menu(1));
        let pm_base = run(&pm).expect("cooks");
        let pm2 = pm.with("dodivs", ParamValue::B(true)).with("divs", ParamValue::V3([1.0, 5.0, 2.0]));
        assert_same(&run(&pm2).expect("cooks"), &pm_base, 0.0, "polymesh dodivs");
        // surftype is inert for poly, points type = polymesh points without prims
        let tri = defaults().with("surftype", ParamValue::Menu(3));
        assert_same(&run(&tri).expect("cooks"), &base, 0.0, "poly surftype");
        let pts = run(&defaults().with("type", ParamValue::Menu(5))).expect("cooks");
        assert_eq!((pts.point_count, pts.prim_count()), (56, 0));
        assert_eq!(pts.p(), pm_base.p());
    }

    #[test]
    fn unconsolidated_faces_own_their_points() {
        let p = defaults().with("consolidatepts", ParamValue::B(false));
        let d = run(&p).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (24, 6));
        let uniq: BTreeSet<[i64; 3]> = d.p().iter().map(|v| v.map(|c| (c * 1000.0).round() as i64)).collect();
        assert_eq!(uniq.len(), 8, "still 8 distinct positions");
    }

    /// Hostile parms: errors (never a panic, never a hang / huge allocation).
    #[test]
    fn hostile_params_are_errors() {
        let v3 = |a: f32| ParamValue::V3([a, a, a]);
        let pm = || defaults().with("type", ParamValue::Menu(1));
        for (base, name, val, want_bad) in [
            (pm(), "divrate", v3(0.0), true),
            (pm(), "divrate", ParamValue::V3([4.0, -1.0, 4.0]), true),
            (pm(), "divrate", ParamValue::V3([4.0, f32::NAN, 4.0]), true),
            (pm(), "divrate", v3(10000.0), false),
            (pm(), "divrate", v3(1.0e9), false),
            (pm(), "divrate", ParamValue::V3([3000.0, 3000.0, 2.0]), false),
            (pm(), "divrate", v3(f32::INFINITY), false),
            (defaults(), "divrate", v3(0.0), true),
            (defaults(), "divs", v3(0.0), true),
            (defaults(), "divs", v3(-3.0), true),
            (defaults().with("dodivs", ParamValue::B(true)), "divs", v3(10000.0), false),
            (defaults().with("dodivs", ParamValue::B(true)), "divs", v3(300.0), false),
            (pm(), "size", ParamValue::V3([1.0, f32::NAN, 1.0]), true),
            (pm(), "t", v3(f32::INFINITY), true),
            (defaults(), "r", v3(f32::NAN), true),
            (pm(), "scale", ParamValue::F(f32::NAN), true),
            (pm(), "surftype", ParamValue::Menu(3), true),
            (pm(), "surftype", ParamValue::Menu(99), true),
            (pm(), "consolidatepts", ParamValue::B(false), true),
            (pm(), "vertexnormals", ParamValue::B(true), true),
            (defaults().with("dodivs", ParamValue::B(true)), "rebar", ParamValue::B(true), true),
            (defaults(), "type", ParamValue::Menu(7), true),
            (defaults(), "type", ParamValue::Menu(u32::MAX), true),
        ] {
            match run(&base.with(name, val.clone())) {
                Err(CookError::BadParam(_)) if want_bad => {}
                Err(CookError::Failed(_)) if !want_bad => {}
                other => panic!("{name}={val:?}: unexpected {other:?}"),
            }
        }
        // zero size / zero scale are legal (degenerate) boxes
        assert!(run(&pm().with("size", v3(0.0))).is_ok());
        assert!(run(&pm().with("scale", ParamValue::F(0.0))).is_ok());
        // wrong typed / missing params
        assert!(matches!(run(&ParamSet::new()), Err(CookError::BadParam(_))));
        assert!(matches!(run(&defaults().with("size", ParamValue::F(1.0))), Err(CookError::BadParam(_))));
        assert!(matches!(run(&defaults().with("r", ParamValue::F(1.0))), Err(CookError::BadParam(_))));
    }

    #[test]
    fn unrepresentable_types_name_the_type() {
        for (m, word) in [(2, "Mesh"), (3, "NURBS"), (4, "Bezier"), (6, "Primitive")] {
            match run(&defaults().with("type", ParamValue::Menu(m))) {
                Err(CookError::BadParam(msg)) => assert!(msg.contains(word), "{msg}"),
                other => panic!("type {m}: {other:?}"),
            }
        }
    }

    #[test]
    fn scene_params_by_token_and_generator_shape() {
        assert!(new().inputs().is_empty());
        assert_eq!(new().type_name(), "box");
        let d = cook_scene(
            r#"{"nodes":[{"name":"b","type":"box","params":{"type":1,"size":[2,2,2],"t":[0,1,0],"divrate":[3,2,2]}}],"display":"b"}"#,
        );
        assert_eq!((d.point_count, d.prim_count()), (12, 10));
        assert_eq!(d.bbox(), ([-1.0, 0.0, -1.0], [1.0, 2.0, 1.0]));
        for bad_scene in [
            r#"{"nodes":[{"name":"b","type":"box","params":{"divrate":[0,4,4]}}],"display":"b"}"#,
            r#"{"nodes":[{"name":"b","type":"box","params":{"type":1,"divrate":[10000,10000,10000]}}],"display":"b"}"#,
        ] {
            let (mut g, id) = load_scene(bad_scene).expect("loads");
            assert!(g.cook(id, 0.0).is_err());
        }
    }

    const HOUDINI_BOX: &[u8] = include_bytes!("../../../../fixtures/sop-box/houdini.geo");
    const DEFAULT_SCENE: &str = include_str!("../../../../fixtures/box-default/scene.json");
    const DIVS_SCENE: &str = include_str!("../../../../fixtures/box-divs222/scene.json");
    const DEFAULT_GEO: &[u8] = include_bytes!("../../../../fixtures/box-default/expected.geo");
    const DIVS_GEO: &[u8] = include_bytes!("../../../../fixtures/box-divs222/expected.geo");

    /// Real Houdini 20.5 default box (`fixtures/sop-box/houdini.geo`): P in index order, prims verbatim.
    #[test]
    fn default_box_matches_houdini_fixture_order() {
        let h = read_geo(HOUDINI_BOX).expect("houdini.geo parses");
        let d = run(&defaults()).expect("cooks");
        assert_eq!(d.p(), h.p());
        assert_eq!(polys(&h).first(), Some(&vec![1, 5, 4, 0]));
        assert_eq!(polys(&d), polys(&h));
    }
    #[test]
    fn fixture_box_default_matches_houdini() {
        let want = read_geo(DEFAULT_GEO).expect("expected.geo parses");
        assert_same(&want, &read_geo(HOUDINI_BOX).expect("parses"), 0.0, "box-default vs houdini");
        assert_same(&cook_scene(DEFAULT_SCENE), &want, 1e-6, "box-default scene");
    }

    /// box-divs222 (dodivs, divs 3,3,3, size (2,4,6), centre (1,-1,0.5)) = Houdini's `poly_dodivs333` case.
    #[test]
    fn fixture_box_divs222_matches_houdini() {
        let want = read_geo(DIVS_GEO).expect("expected.geo parses");
        assert_eq!((want.point_count, want.prim_count()), (64, 48));
        assert_same(&want, &read_geo(houdini_geo("poly_dodivs333")).expect("parses"), 1e-6, "box-divs222 vs houdini");
        assert_same(&cook_scene(DIVS_SCENE), &want, 1e-6, "box-divs222 scene");
    }

    /// `cargo test -p hexe-sop -- --ignored write_fixture` (re)writes the committed expected.geo from the cook.
    #[test]
    #[ignore = "writes fixtures/*/expected.geo; run manually"]
    fn write_fixture() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures");
        for (case, scene) in [("box-default", DEFAULT_SCENE), ("box-divs222", DIVS_SCENE)] {
            std::fs::write(format!("{root}/{case}/expected.geo"), write_geo_json(&cook_scene(scene)))
                .expect("write expected.geo");
        }
    }
}
