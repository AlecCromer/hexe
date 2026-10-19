//! Sphere SOP: generator. `type` prim = 1 point + `Prim::Sphere` · poly = subdivided icosahedron · polymesh = lat/long quads.
//!
//! Closed forms:
//! - prim: 1 pt (= `t`), 1 vertex, 1 `Sphere` prim, xform = R·diag(rad·scale)·O (row-major 3x3).
//! - poly, `freq`=f (any f >= 1 like Houdini; work budget 10f²+2 <= 4M points): 10f²+2 pts, 20f² tris in Houdini 20.5's
//!   order and positions (hython-verified to f = 101, see `poly`): Houdini's icosahedron (vertex 0 at -Z in the Y frame),
//!   faces walked apex-first row by row, sine-weighted (slerp on edges) lattice points; `orient` X / Z remap the unit
//!   points ((y,-z,-x) / (x,-z,y)), then rad·scale, rotation, `t` as below.
//! - polymesh, `rows`=R `cols`=C: the poles FIRST (shared: top 0, bottom 1; `upole`: 2·C) then ring points (R-2)·C; ring angle runs +X toward -Z
//!   (verified vs real Houdini 20.5, fixtures/sop-sphere/houdini.geo). Houdini lists cells clockwise; hexe emits them verbatim (H32:
//!   Houdini winding is global). Prim order = Houdini: top fan, ring rows, bottom fan. Prims (R-1)·C =
//!   (R-3)·C quads + 2·C pole faces (tris when `triangularpoles`, else quads that touch the pole). `upole` on: R·C pts.
//!   `surftype` triangles / alttriangles / revtriangles (verified vs Houdini 20.5 hython): each ring cell [a,b,c,d] split in
//!   place (abc+acd / checkerboard / abd+bcd), caps always triangles. Rows / Columns curve connectivities: not supported.
//! - P = R(rad·scale ∘ O(unit)) + t. O = pole-axis permutation (y identity, x: (y,z,x), z: (z,x,y)), R = Rx then Ry then Rz.
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::Arc;

/// Menu positions (parms-sphere.md `type`).
const TY_PRIM: u32 = 0;
const TY_POLY: u32 = 1;
const TY_POLYMESH: u32 = 2;
/// `surftype` menu positions (Houdini order) of the polymesh connectivities implemented: triangles, quads, alternate /
/// reverse triangles. Rows / Columns / Rows and Columns (open curves) are not.
const SURF_TRIS: u32 = 3;
const SURF_QUADS: u32 = 4;
const SURF_ALT: u32 = 5;
const SURF_REV: u32 = 6;
/// Hard caps (AGENTS.md §4: capped allocations). Houdini's `freq` has no upper bound (min 1, hython 20.5), so the poly
/// sphere is budgeted by its output: 10·f²+2 points <= `MAX_POINTS` (f <= 632), like the polymesh rows x cols.
const MAX_POINTS: usize = 4_000_000;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(TY_PRIM)).label("Primitive Type").menu(&[
            ("prim", "Primitive"),
            ("poly", "Polygon"),
            ("polymesh", "Polygon Mesh"),
        ]),
        ParamSpec::new("surftype", ParamValue::Menu(SURF_QUADS)).label("Connectivity").menu(&[
            ("rows", "Rows"),
            ("cols", "Columns"),
            ("rowcol", "Rows and Columns"),
            ("triangles", "Triangles"),
            ("quads", "Quadrilaterals"),
            ("alttriangles", "Alternating Triangles"),
            ("revtriangles", "Reverse Triangles"),
        ]),
        ParamSpec::new("rad", ParamValue::V3([1.0, 1.0, 1.0])).label("Radius").range(-1.0, 1.0),
        ParamSpec::new("t", ParamValue::V3([0.0; 3])).label("Center").range(-1.0, 1.0),
        ParamSpec::new("r", ParamValue::V3([0.0; 3])).label("Rotate").range(0.0, 360.0),
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Uniform Scale").range(0.0, 10.0),
        ParamSpec::new("orient", ParamValue::Menu(1)).label("Orientation").range(0.0, 1.0).menu(&[
            ("x", "X Axis"),
            ("y", "Y Axis"),
            ("z", "Z Axis"),
        ]),
        ParamSpec::new("freq", ParamValue::I(2)).label("Frequency").range(1.0, 10.0),
        ParamSpec::new("rows", ParamValue::I(13)).label("Rows").range(3.0, 50.0),
        ParamSpec::new("cols", ParamValue::I(24)).label("Columns").range(3.0, 50.0),
        ParamSpec::new("upole", ParamValue::B(false)).label("Unique Points per Pole"),
        ParamSpec::new("triangularpoles", ParamValue::B(true)).label("Triangular Poles"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Sphere { specs: spec() })
}

struct Sphere {
    specs: Vec<ParamSpec>,
}

fn bad(name: &str) -> CookError {
    CookError::BadParam(format!("sphere: missing or mistyped `{name}`"))
}
fn menu(p: &ParamSet, name: &str) -> Result<u32, CookError> {
    p.menu(name).ok_or_else(|| bad(name))
}
fn vec3(p: &ParamSet, name: &str) -> Result<[f32; 3], CookError> {
    p.vec3(name).ok_or_else(|| bad(name))
}
fn int(p: &ParamSet, name: &str) -> Result<i32, CookError> {
    p.int(name).ok_or_else(|| bad(name))
}
fn toggle(p: &ParamSet, name: &str) -> Result<bool, CookError> {
    p.toggle(name).ok_or_else(|| bad(name))
}
fn at(v: &[[f64; 3]], i: usize) -> [f64; 3] {
    v.get(i).copied().unwrap_or_default()
}

/// Unit-sphere → output position (without the centre).
struct Xf {
    rad: [f64; 3],
    rot_deg: [f64; 3],
    orient: u32,
}
impl Xf {
    fn apply(&self, u: [f64; 3]) -> [f64; 3] {
        let [x, y, z] = u;
        let o = match self.orient {
            0 => [y, z, x],
            2 => [z, x, y],
            _ => [x, y, z],
        };
        let s = [o[0] * self.rad[0], o[1] * self.rad[1], o[2] * self.rad[2]];
        let [ax, ay, az] = self.rot_deg.map(f64::to_radians);
        let (sx, cx) = ax.sin_cos();
        let (sy, cy) = ay.sin_cos();
        let (sz, cz) = az.sin_cos();
        let a = [s[0], s[1] * cx - s[2] * sx, s[1] * sx + s[2] * cx];
        let b = [a[0] * cy + a[2] * sy, a[1], -a[0] * sy + a[2] * cy];
        [b[0] * cz - b[1] * sz, b[0] * sz + b[1] * cz, b[2]]
    }
    fn p(&self, u: [f64; 3], t: [f32; 3]) -> [f32; 3] {
        let q = self.apply(u);
        [(q[0] + f64::from(t[0])) as f32, (q[1] + f64::from(t[1])) as f32, (q[2] + f64::from(t[2])) as f32]
    }
}

/// Houdini's icosahedron (orient Y frame, hython 20.5): vertex 0 / 11 = -Z / +Z, ring 1..=5 at z = -1/√5 from 90° in
/// -72° steps, ring 6..=10 at z = +1/√5 from 126° in +72° steps (angles in the XY plane, radius 2/√5).
fn ico_vertices() -> [[f64; 3]; 12] {
    let (rz, rr) = (1.0 / 5f64.sqrt(), 2.0 / 5f64.sqrt());
    let ring = |deg: f64, z: f64| {
        let a = deg.to_radians();
        [rr * a.cos(), rr * a.sin(), z]
    };
    let mut v = [[0.0; 3]; 12];
    v[0] = [0.0, 0.0, -1.0];
    for k in 0..5 {
        v[1 + k] = ring(90.0 - 72.0 * k as f64, -rz);
        v[6 + k] = ring(126.0 + 72.0 * k as f64, rz);
    }
    v[11] = [0.0, 0.0, 1.0];
    v
}
/// Houdini's 20 faces in prim order as (apex A, left L, right R); its `freq` 1 prims are `[R, L, A]` (hython 20.5).
const ICO_FACES: [[usize; 3]; 20] = [
    [0, 1, 2],
    [0, 2, 3],
    [0, 3, 4],
    [0, 4, 5],
    [0, 5, 1],
    [1, 5, 6],
    [5, 4, 7],
    [4, 3, 8],
    [3, 2, 9],
    [2, 1, 10],
    [1, 6, 10],
    [5, 7, 6],
    [4, 8, 7],
    [3, 9, 8],
    [2, 10, 9],
    [11, 10, 6],
    [11, 9, 10],
    [11, 8, 9],
    [11, 7, 8],
    [11, 6, 7],
];
/// Pole-axis map of the poly sphere (hython 20.5): Y is Houdini's native frame (identity), X: (y, -z, -x), Z: (x, -z, y).
fn poly_orient(orient: u32, u: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = u;
    match orient {
        0 => [y, -z, -x],
        2 => [x, -z, y],
        _ => u,
    }
}

/// Dedup key of a face-boundary lattice point: corner `(0, v, 0, 0)` / edge `(1, lo, hi, weight on hi)`.
type Key = (u8, usize, usize, usize);
/// Edge point shared by both faces: sorted vertex pair + weight on the larger vertex.
fn edge_key(u: usize, v: usize, wu: usize, wv: usize) -> Key {
    if u < v { (1, u, v, wv) } else { (1, v, u, wu) }
}
/// Points of a `freq` poly sphere, `None` past the work budget (u64 maths: no overflow for any i32).
fn poly_points(freq: u64) -> Option<u64> {
    freq.checked_mul(freq)?.checked_mul(10)?.checked_add(2).filter(|&n| n <= MAX_POINTS as u64)
}

/// Houdini's geodesic sphere (hython 20.5 point / prim order and positions): per icosahedron face (`ICO_FACES` order) the
/// lattice is walked row by row from the apex (row i = 0..=f, column j = 0..=i; weights A = f-i, L = i-j, R = j) and each
/// point not yet made (corners / edge points are shared) is appended. Position = normalize(Σ sin(θ·w/f)·V) with θ the
/// icosahedron edge angle: slerp on the edges, its sine-weighted extension inside. Prims per face: the upward triangles
/// row by row `[r(i+1,j+1), r(i+1,j), r(i,j)]`, then the inverted ones `[r(i,j+1), r(i+1,j+1), r(i,j)]`.
fn poly(freq: usize, xf: &Xf, orient: u32, t: [f32; 3]) -> Detail {
    let base = ico_vertices();
    let theta = (1.0 / 5f64.sqrt()).acos();
    let f = freq as f64;
    let wsin = |w: usize| (theta * w as f64 / f).sin();
    let mut ids: HashMap<Key, u32> = HashMap::new();
    let mut unit: Vec<[f64; 3]> = Vec::with_capacity(10 * freq * freq + 2);
    let mut tris: Vec<[u32; 3]> = Vec::with_capacity(20 * freq * freq);
    let row_start = |i: usize| i * (i + 1) / 2;
    let mut lattice = vec![0u32; row_start(freq + 1)];
    for &[a, l, r] in &ICO_FACES {
        for i in 0..=freq {
            for j in 0..=i {
                let (wa, wl, wr) = (freq - i, i - j, j);
                let key: Option<Key> = match (wa == 0, wl == 0, wr == 0) {
                    (false, true, true) => Some((0, a, 0, 0)),
                    (true, false, true) => Some((0, l, 0, 0)),
                    (true, true, false) => Some((0, r, 0, 0)),
                    (false, false, true) => Some(edge_key(a, l, wa, wl)),
                    (false, true, false) => Some(edge_key(a, r, wa, wr)),
                    (true, false, false) => Some(edge_key(l, r, wl, wr)),
                    _ => None,
                };
                let known = key.and_then(|k| ids.get(&k).copied());
                let id = match known {
                    Some(id) => id,
                    None => {
                        let (sa, sl, sr) = (wsin(wa), wsin(wl), wsin(wr));
                        let (va, vl, vr) = (at(&base, a), at(&base, l), at(&base, r));
                        let s: [f64; 3] = std::array::from_fn(|k| sa * va[k] + sl * vl[k] + sr * vr[k]);
                        let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
                        let id = u32::try_from(unit.len()).unwrap_or(u32::MAX);
                        unit.push(if len > 0.0 { s.map(|c| c / len) } else { s });
                        if let Some(k) = key {
                            ids.insert(k, id);
                        }
                        id
                    }
                };
                if let Some(slot) = lattice.get_mut(row_start(i) + j) {
                    *slot = id;
                }
            }
        }
        let at_l = |i: usize, j: usize| lattice.get(row_start(i) + j).copied().unwrap_or(0);
        for i in 0..freq {
            for j in 0..=i {
                tris.push([at_l(i + 1, j + 1), at_l(i + 1, j), at_l(i, j)]);
            }
        }
        for i in 1..freq {
            for j in 0..i {
                tris.push([at_l(i, j + 1), at_l(i + 1, j + 1), at_l(i, j)]);
            }
        }
    }
    let mut d = Detail::default();
    d.add_points(u32::try_from(unit.len()).unwrap_or(u32::MAX));
    for (dst, u) in d.p_mut().iter_mut().zip(&unit) {
        *dst = xf.p(poly_orient(orient, *u), t);
    }
    for tri in &tris {
        d.add_poly(tri, true);
    }
    d
}

fn polymesh(
    rows: usize,
    cols: usize,
    upole: bool,
    tripoles: bool,
    surf: u32,
    xf: &Xf,
    t: [f32; 3],
) -> Result<Detail, CookError> {
    // triangle connectivities always cap with triangles (Houdini ignores `triangularpoles` there)
    let tripoles = tripoles || surf != SURF_QUADS;
    let rings = rows - 2;
    let ring_pts = rings.checked_mul(cols);
    let pole_pts = if upole { 2 * cols } else { 2 };
    let total = ring_pts.and_then(|n| n.checked_add(pole_pts)).filter(|&n| n <= MAX_POINTS);
    let (Some(_), Some(total)) = (ring_pts, total) else {
        return Err(CookError::Failed(format!("sphere: rows {rows} x cols {cols} exceeds {MAX_POINTS} points")));
    };
    let id = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let mut d = Detail::default();
    d.add_points(id(total));
    let ring = |i: usize, j: usize| id(pole_pts + (i - 1) * cols + j % cols); // i = 1..=rings; the poles come FIRST (Houdini)
    let top = |j: usize| id(if upole { j % cols } else { 0 });
    let bot = |j: usize| id(if upole { cols + j % cols } else { 1 });
    let mut unit = vec![[0.0f64; 3]; total];
    let mut put = |n: u32, u: [f64; 3]| {
        if let Some(slot) = unit.get_mut(n as usize) {
            *slot = u;
        }
    };
    for i in 1..=rings {
        let th = PI * i as f64 / (rows - 1) as f64;
        for j in 0..cols {
            let ph = 2.0 * PI * j as f64 / cols as f64;
            put(ring(i, j), [th.sin() * ph.cos(), th.cos(), -th.sin() * ph.sin()]);
        }
    }
    for j in 0..(if upole { cols } else { 1 }) {
        put(top(j), [0.0, 1.0, 0.0]);
        put(bot(j), [0.0, -1.0, 0.0]);
    }
    for (dst, u) in d.p_mut().iter_mut().zip(&unit) {
        *dst = xf.p(*u, t);
    }
    for j in 0..cols {
        if tripoles {
            d.add_poly(&[top(j), ring(1, j + 1), ring(1, j)], true);
        } else {
            d.add_poly(&[top(j), top(j + 1), ring(1, j + 1), ring(1, j)], true);
        }
    }
    // ring cells [a, b, c, d]; triangle connectivities split each in place (Houdini 20.5 hython): triangles = abc + acd,
    // reverse = abd + bcd, alternating = reverse where (band + column) is even, triangles where odd
    for i in 1..rings {
        for j in 0..cols {
            let (a, b, c, e) = (ring(i, j), ring(i, j + 1), ring(i + 1, j + 1), ring(i + 1, j));
            let rev = surf == SURF_REV || (surf == SURF_ALT && (i - 1 + j) % 2 == 0);
            if surf == SURF_QUADS {
                d.add_poly(&[a, b, c, e], true);
            } else if rev {
                d.add_poly(&[a, b, e], true);
                d.add_poly(&[b, c, e], true);
            } else {
                d.add_poly(&[a, b, c], true);
                d.add_poly(&[a, c, e], true);
            }
        }
    }
    for j in 0..cols {
        if tripoles {
            d.add_poly(&[ring(rings, j), ring(rings, j + 1), bot(j)], true);
        } else {
            d.add_poly(&[ring(rings, j), ring(rings, j + 1), bot(j + 1), bot(j)], true);
        }
    }
    Ok(d)
}

fn prim(xf: &Xf, t: [f32; 3]) -> Detail {
    let mut d = Detail::default();
    d.add_points(1);
    for p in d.p_mut().iter_mut() {
        *p = t;
    }
    // columns of the linear map = images of the basis vectors; stored row-major
    let (cx, cy, cz) = (xf.apply([1.0, 0.0, 0.0]), xf.apply([0.0, 1.0, 0.0]), xf.apply([0.0, 0.0, 1.0]));
    let m = |r: usize| [cx[r] as f32, cy[r] as f32, cz[r] as f32];
    let (r0, r1, r2) = (m(0), m(1), m(2));
    let xform = [r0[0], r0[1], r0[2], r1[0], r1[1], r1[2], r2[0], r2[1], r2[2]];
    d.vertex_point.push(0);
    d.prims.push(Prim { kind: PrimKind::Sphere { xform }, verts: 0..1 });
    d
}

impl Node<Detail> for Sphere {
    fn type_name(&self) -> &'static str {
        "sphere"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, _: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let ty = menu(p, "type")?;
        let orient = menu(p, "orient")?;
        if orient > 2 {
            return Err(CookError::BadParam(format!("sphere: orient {orient} not in 0..=2")));
        }
        let scale = f64::from(p.float("scale").ok_or_else(|| bad("scale"))?);
        let rad = vec3(p, "rad")?.map(|r| f64::from(r) * scale);
        let rot_deg = vec3(p, "r")?.map(f64::from);
        let t = vec3(p, "t")?;
        let xf = Xf { rad, rot_deg, orient };
        let d = match ty {
            TY_PRIM => prim(&xf, t),
            TY_POLY => {
                let f = int(p, "freq")?;
                if f < 1 {
                    return Err(CookError::BadParam(format!("sphere: freq {f} < 1")));
                }
                let fu = u64::try_from(f).map_err(|_| bad("freq"))?;
                if poly_points(fu).is_none() {
                    return Err(CookError::Failed(format!(
                        "sphere: freq {f} makes more than {MAX_POINTS} points (10 f^2 + 2; work budget)"
                    )));
                }
                let poly_xf = Xf { orient: 1, ..xf };
                poly(usize::try_from(f).map_err(|_| bad("freq"))?, &poly_xf, orient, t)
            }
            TY_POLYMESH => {
                let (rows, cols) = (int(p, "rows")?, int(p, "cols")?);
                if rows < 3 || cols < 3 {
                    return Err(CookError::BadParam(format!("sphere: rows {rows} / cols {cols} must be >= 3")));
                }
                let surf = menu(p, "surftype")?;
                if ![SURF_TRIS, SURF_QUADS, SURF_ALT, SURF_REV].contains(&surf) {
                    return Err(CookError::BadParam(format!(
                        "sphere: polymesh surftype {surf} (rows / columns curves) not supported"
                    )));
                }
                let upole = toggle(p, "upole")?;
                if upole && surf != SURF_QUADS {
                    return Err(CookError::BadParam(
                        "sphere: unique points per pole with triangle connectivity not verified yet".into(),
                    ));
                }
                let (r, c) =
                    (usize::try_from(rows).map_err(|_| bad("rows"))?, usize::try_from(cols).map_err(|_| bad("cols"))?);
                polymesh(r, c, upole, toggle(p, "triangularpoles")?, surf, &xf, t)?
            }
            other => {
                return Err(CookError::BadParam(format!(
                    "sphere: type {other} (mesh/nurbs/bezier/polysoup) not supported"
                )));
            }
        };
        Ok(Arc::new(d))
    }
}
#[cfg(test)]
mod tests {
    use crate::load_scene;
    use hexe_geo::{AttrClass, Detail, Prim, PrimKind, read_geo, write_geo_json};
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;

    fn scene(params: &str) -> String {
        format!(r#"{{"nodes":[{{"name":"s","type":"sphere","params":{params}}}],"display":"s"}}"#)
    }
    fn cook_json(json: &str) -> Result<Detail, String> {
        let (mut g, id) = load_scene(json).map_err(|e| e.to_string())?;
        g.cook(id, 0.0).map(|d| (*d).clone()).map_err(|e| e.to_string())
    }
    fn cook(params: &str) -> Detail {
        cook_json(&scene(params)).expect("cooks")
    }
    fn face_pts(d: &Detail, p: &Prim) -> Vec<u32> {
        p.verts.clone().filter_map(|v| d.vertex_point.get(v as usize).copied()).collect()
    }
    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    /// Unit-sphere topology invariants: every point radius 1, outward winding, closed 2-manifold (each directed edge
    /// once, reverse once; degenerate pole edges skipped) + Euler, both only when `closed` (unique-pole meshes are cut open).
    fn assert_closed_outward_unit_sphere(d: &Detail, closed: bool) {
        let p = d.p();
        for q in p {
            assert!((dot(*q, *q).sqrt() - 1.0).abs() < 1e-5, "off sphere {q:?}");
        }
        let mut dir: HashMap<(u32, u32), u32> = HashMap::new();
        for pr in &d.prims {
            let f = face_pts(d, pr);
            let pos: Vec<[f32; 3]> = f.iter().map(|&i| p[i as usize]).collect();
            let c = pos.iter().fold([0.0; 3], |a, q| [a[0] + q[0], a[1] + q[1], a[2] + q[2]]);
            // right-handed Newell sum about the origin: points INWARD for Houdini's clockwise-from-outside faces
            let mut n = [0.0f32; 3];
            for i in 0..pos.len() {
                let (a, b) = (pos[i], pos[(i + 1) % pos.len()]);
                let x = cross(a, b);
                n = [n[0] + x[0], n[1] + x[1], n[2] + x[2]];
            }
            assert!(dot(n, c) < 0.0, "face not outward: {f:?}");
            for i in 0..f.len() {
                let (a, b) = (f[i], f[(i + 1) % f.len()]);
                if a != b {
                    *dir.entry((a, b)).or_default() += 1;
                }
            }
        }
        for (&(a, b), &n) in &dir {
            assert_eq!(n, 1, "directed edge {a}->{b} used {n}x");
            assert!(!closed || dir.get(&(b, a)) == Some(&1), "edge {a}->{b} has no reverse");
        }
        if closed {
            let (v, e, f) = (d.point_count as i64, dir.len() as i64 / 2, d.prims.len() as i64);
            assert_eq!(v - e + f, 2, "Euler V{v} E{e} F{f}");
        }
    }

    // ---- type = prim -------------------------------------------------------------------------------------------
    #[test]
    fn default_is_one_point_one_sphere_prim() {
        let d = cook("{}");
        assert_eq!((d.point_count, d.vertex_count(), d.prim_count()), (1, 1, 1));
        assert_eq!(d.p(), &[[0.0, 0.0, 0.0]]);
        assert_eq!(d.prims[0].kind, PrimKind::Sphere { xform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] });
    }
    #[test]
    fn prim_xform_is_diag_rad_at_t_by_token() {
        let d = cook(r#"{"rad":[2,3,4],"t":[1,2,3]}"#);
        assert_eq!(d.p(), &[[1.0, 2.0, 3.0]]);
        assert_eq!(d.prims[0].kind, PrimKind::Sphere { xform: [2.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 4.0] });
        let s = cook(r#"{"rad":[1,2,3],"scale":2}"#);
        assert_eq!(s.prims[0].kind, PrimKind::Sphere { xform: [2.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 6.0] });
    }
    #[test]
    fn prim_rotation_z90_moves_axes() {
        let d = cook(r#"{"rad":[2,3,4],"r":[0,0,90]}"#);
        let PrimKind::Sphere { xform } = d.prims[0].kind else { panic!("not a sphere") };
        let want = [0.0, -3.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 4.0];
        assert!(xform.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5), "{xform:?}");
    }

    // ---- type = poly -------------------------------------------------------------------------------------------
    #[test]
    fn poly_counts_are_10f2_plus_2_and_20f2() {
        for f in [1usize, 2, 3, 4, 7, 10, 100, 101] {
            let d = cook(&format!(r#"{{"type":1,"freq":{f}}}"#));
            assert_eq!(d.point_count as usize, 10 * f * f + 2, "points f={f}");
            assert_eq!(d.prims.len(), 20 * f * f, "tris f={f}");
            assert_eq!(d.vertex_count() as usize, 60 * f * f);
            assert!(d.prims.iter().all(|p| matches!(p.kind, PrimKind::Poly { closed: true }) && p.verts.len() == 3));
        }
        let d = cook(r#"{"type":1}"#); // default freq 2
        assert_eq!((d.point_count, d.prims.len()), (42, 80));
        let d = cook(r#"{"type":1,"freq":1}"#);
        assert_eq!((d.point_count, d.prims.len()), (12, 20));
    }
    #[test]
    fn poly_is_closed_outward_unit_sphere() {
        for f in 1..=6 {
            assert_closed_outward_unit_sphere(&cook(&format!(r#"{{"type":1,"freq":{f}}}"#)), true);
        }
    }
    #[test]
    fn poly_points_lie_on_rad_ellipsoid_around_t() {
        let (rad, t) = ([1.5f32, 2.0, 0.5], [3.0f32, -1.0, 2.0]);
        let d = cook(r#"{"type":1,"freq":3,"rad":[1.5,2,0.5],"t":[3,-1,2]}"#);
        for q in d.p() {
            let s =
                ((q[0] - t[0]) / rad[0]).powi(2) + ((q[1] - t[1]) / rad[1]).powi(2) + ((q[2] - t[2]) / rad[2]).powi(2);
            assert!((s - 1.0).abs() < 1e-5, "{q:?} -> {s}");
        }
    }
    fn near(got: [f32; 3], want: [f64; 3], tol: f64, what: &str) {
        assert!(got.iter().zip(want).all(|(g, w)| (f64::from(*g) - w).abs() <= tol), "{what}: {got:?} != {want:?}");
    }
    /// Houdini 20.5 hython (`sphere` type poly, oracle/houdini/parmgaps/sphere_poly_ref.py): orient X / Y / Z at freq 5
    /// put Houdini's icosahedron vertex 0 on -Z (Y) / +Y (X, Z); sampled interior and edge points, same prim lists.
    #[test]
    fn poly_orient_matches_houdini() {
        let rec: [(u32, [(usize, [f64; 3]); 3]); 3] = [
            (
                1,
                [
                    (4, [0.2211194634437561, 0.3043449819087982, -0.9265421628952026]),
                    (100, [-0.9453449845314026, -0.1714264154434204, 0.2773730158805847]),
                    (251, [-0.7270480990409851, -0.0037337145768105984, 0.6865764260292053]),
                ],
            ),
            (
                0,
                [
                    (4, [0.3043449819087982, 0.9265421628952026, -0.2211194634437561]),
                    (100, [-0.1714264154434204, -0.2773730158805847, 0.9453449845314026]),
                    (251, [-0.0037337145768105984, -0.6865764260292053, 0.7270480990409851]),
                ],
            ),
            (
                2,
                [
                    (4, [0.2211194634437561, 0.9265421628952026, 0.3043449819087982]),
                    (100, [-0.9453449845314026, -0.2773730158805847, -0.1714264154434204]),
                    (251, [-0.7270480990409851, -0.6865764260292053, -0.0037337145768105984]),
                ],
            ),
        ];
        for (o, pts) in rec {
            let d = cook(&format!(r#"{{"type":1,"freq":5,"orient":{o}}}"#));
            assert_eq!((d.point_count, d.prims.len()), (252, 500));
            for (i, want) in pts {
                near(d.p()[i], want, 2e-6, &format!("orient {o} P[{i}]"));
            }
            assert_closed_outward_unit_sphere(&d, true);
            let prim = |i: usize| face_pts(&d, &d.prims[i]);
            assert_eq!(
                [prim(0), prim(24), prim(25), prim(100), prim(499)],
                [vec![2, 1, 0], vec![14, 19, 13], vec![21, 2, 0], vec![1, 51, 0], vec![242, 167, 251]]
            );
        }
        // freq 5 face 0 interior rows (sine-weighted lattice, not the normalised flat one)
        let d = cook(r#"{"type":1,"freq":5}"#);
        near(d.p()[7], [0.22503246366977692, 0.5122469067573547, -0.8288325071334839], 2e-6, "P[7]");
        near(d.p()[8], [0.4176367223262787, 0.3723118007183075, -0.8288325071334839], 2e-6, "P[8]");
    }
    /// Houdini 20.5 hython: freq 101 (past hexe's old cap of 100; Houdini has no upper bound) = 102012 points, 204020
    /// triangles; sampled positions and prims.
    #[test]
    fn poly_freq_above_100_matches_houdini() {
        let d = cook(r#"{"type":1,"freq":101}"#);
        assert_eq!((d.point_count, d.prims.len(), d.vertex_count()), (102_012, 204_020, 612_060));
        let rec: [(usize, [f64; 3]); 10] = [
            (0, [0.0, 0.0, -1.0]),
            (1, [0.0, 0.010961574502289295, -0.9999399185180664]),
            (11, [0.010626389645040035, 0.03653772920370102, -0.9992757439613342]),
            (12, [0.021113399416208267, 0.02906011790037155, -0.9993547201156616]),
            (13, [0.03146569803357124, 0.021397091448307037, -0.9992757439613342]),
            (100, [0.09562284499406815, 0.07711715996265411, -0.9924258589744568]),
            (1000, [0.11568845808506012, 0.40928915143013, -0.9050406813621521]),
            (12345, [-0.23620925843715668, -0.5416808724403381, -0.8067137002944946]),
            (50000, [0.5883612632751465, 0.7885125875473022, 0.17910604178905487]),
            (102011, [-0.8476149439811707, -0.264373779296875, 0.46006014943122864]),
        ];
        for (i, want) in rec {
            near(d.p()[i], want, 2e-6, &format!("P[{i}]"));
        }
        let prim = |i: usize| face_pts(&d, &d.prims[i]);
        assert_eq!(prim(0), [2, 1, 0]);
        assert_eq!(prim(1), [4, 3, 1]);
        assert_eq!(prim(2), [5, 4, 2]);
        assert_eq!(prim(5000), [5101, 5100, 5000]);
        assert_eq!(prim(204_019), [96962, 61511, 102_011]);
    }
    /// Houdini 20.5 hython: freq 2, rad (1,2,3), scale 1.5, r (10,20,30), t (0.5,-1,2) for orient Y and X: the orient map
    /// applies to the unit sphere before rad / rotation / translate.
    #[test]
    fn poly_transform_matches_houdini() {
        let base = r#""type":1,"freq":2,"rad":[1,2,3],"scale":1.5,"r":[10,20,30],"t":[0.5,-1,2]"#;
        let rec: [(u32, [(usize, [f64; 3]); 3]); 2] = [
            (
                1,
                [
                    (0, [-1.203350305557251, -1.081127405166626, -2.164374828338623]),
                    (13, [-1.0353213548660278, -2.3088083267211914, 0.09703028202056885]),
                    (41, [1.5535279512405396, -1.7135179042816162, 5.719414710998535]),
                ],
            ),
            (
                0,
                [
                    (0, [-0.8229087591171265, 1.6476922035217285, 2.489527702331543]),
                    (13, [0.8616669178009033, 0.27234721183776855, 5.761268615722656]),
                    (41, [2.2786941528320312, -3.3261947631835938, 3.749116897583008]),
                ],
            ),
        ];
        for (o, pts) in rec {
            let d = cook(&format!("{{{base},\"orient\":{o}}}"));
            for (i, want) in pts {
                near(d.p()[i], want, 1e-5, &format!("orient {o} P[{i}]"));
            }
        }
    }
    #[test]
    fn poly_freq_budget_is_an_error_not_a_hang() {
        assert_eq!(super::poly_points(632), Some(10 * 632 * 632 + 2)); // largest freq inside the budget
        assert_eq!(super::poly_points(633), None);
        assert_eq!(super::poly_points(u64::MAX), None);
        for f in [633, 100_000, i32::MAX] {
            let e = cook_json(&scene(&format!(r#"{{"type":1,"freq":{f}}}"#)));
            assert!(e.as_ref().is_err_and(|m| m.contains("budget")), "freq {f}: {e:?}");
        }
    }

    // ---- type = polymesh ---------------------------------------------------------------------------------------
    #[test]
    fn polymesh_default_closed_form_counts() {
        // rows=13 cols=24: pts (R-2)C+2 = 266 ; prims (R-1)C = 288 = (R-3)C 240 quads + 2C 48 pole tris
        let d = cook(r#"{"type":2}"#);
        assert_eq!((d.point_count, d.prims.len()), (266, 288));
        let (tris, quads) = (
            d.prims.iter().filter(|p| p.verts.len() == 3).count(),
            d.prims.iter().filter(|p| p.verts.len() == 4).count(),
        );
        assert_eq!((tris, quads), (48, 240));
        assert_closed_outward_unit_sphere(&d, true);
    }
    #[test]
    fn polymesh_pole_variants() {
        let up = cook(r#"{"type":2,"upole":true}"#);
        assert_eq!((up.point_count, up.prims.len()), (13 * 24, 288)); // R*C unique pole points
        assert_closed_outward_unit_sphere(&up, false);
        let quad = cook(r#"{"type":2,"triangularpoles":false}"#);
        assert_eq!((quad.point_count, quad.prims.len()), (266, 288));
        assert!(quad.prims.iter().all(|p| p.verts.len() == 4));
        assert_closed_outward_unit_sphere(&quad, true);
        let tiny = cook(r#"{"type":2,"rows":3,"cols":3}"#);
        assert_eq!((tiny.point_count, tiny.prims.len()), (5, 6)); // octahedron-like bipyramid
        assert_closed_outward_unit_sphere(&tiny, true);
    }
    #[test]
    fn polymesh_rows_cols_by_token() {
        let d = cook(r#"{"type":2,"rows":5,"cols":7}"#);
        assert_eq!((d.point_count, d.prims.len()), (3 * 7 + 2, 4 * 7));
    }
    #[test]
    fn polymesh_pole_points_follow_orient_and_rotation() {
        // Houdini: top pole = point 0, bottom pole = point 1, then the ring points
        let d = cook(r#"{"type":2,"rad":[2,3,4]}"#);
        assert_eq!((d.p()[0], d.p()[1]), ([0.0, 3.0, 0.0], [0.0, -3.0, 0.0]));
        let x = cook(r#"{"type":2,"rad":[2,3,4],"orient":0}"#);
        assert_eq!(x.p()[0], [2.0, 0.0, 0.0]);
        let z = cook(r#"{"type":2,"rad":[2,3,4],"orient":2,"t":[1,1,1]}"#);
        assert_eq!(z.p()[0], [1.0, 1.0, 5.0]);
        let r = cook(r#"{"type":2,"rad":[2,3,4],"r":[0,0,90]}"#); // Rz(90): (0,3,0) -> (-3,0,0)
        assert!(r.p()[0].iter().zip([-3.0, 0.0, 0.0]).all(|(a, b)| (a - b).abs() < 1e-5), "{:?}", r.p()[0]);
    }
    #[test]
    fn polymesh_first_ring_point_closed_form() {
        // ring 1, col 0 (point 2 after the two poles): theta = pi/12 from +Y, phi = 0 -> (sin, cos, 0)
        let d = cook(r#"{"type":2}"#);
        let th = std::f32::consts::PI / 12.0;
        let q = d.p()[2];
        assert!((q[0] - th.sin()).abs() < 1e-6 && (q[1] - th.cos()).abs() < 1e-6 && q[2].abs() < 1e-6, "{q:?}");
    }

    // ---- hostile -----------------------------------------------------------------------------------------------
    /// Prim point lists recorded from Houdini 20.5 hython (`sphere` polymesh, rows 5 x cols 4 unless noted): the
    /// triangle connectivities split each ring cell in place, caps stay fans (and are triangles even with
    /// `triangularpoles` off).
    #[test]
    fn polymesh_triangle_connectivities_match_houdini() {
        let caps_top = "[0,3,2],[0,4,3],[0,5,4],[0,2,5]";
        let caps_bot = "[10,11,1],[11,12,1],[12,13,1],[13,10,1]";
        let rec = [
            (
                3,
                format!(
                    "[{caps_top},[2,3,7],[2,7,6],[3,4,8],[3,8,7],[4,5,9],[4,9,8],[5,2,6],[5,6,9],[6,7,11],[6,11,10],[7,8,12],[7,12,11],[8,9,13],[8,13,12],[9,6,10],[9,10,13],{caps_bot}]"
                ),
            ),
            (
                5,
                format!(
                    "[{caps_top},[2,3,6],[3,7,6],[3,4,8],[3,8,7],[4,5,8],[5,9,8],[5,2,6],[5,6,9],[6,7,11],[6,11,10],[7,8,11],[8,12,11],[8,9,13],[8,13,12],[9,6,13],[6,10,13],{caps_bot}]"
                ),
            ),
            (
                6,
                format!(
                    "[{caps_top},[2,3,6],[3,7,6],[3,4,7],[4,8,7],[4,5,8],[5,9,8],[5,2,9],[2,6,9],[6,7,10],[7,11,10],[7,8,11],[8,12,11],[8,9,12],[9,13,12],[9,6,13],[6,10,13],{caps_bot}]"
                ),
            ),
        ];
        for (surf, want) in rec {
            let d = cook(&format!(r#"{{"type":2,"surftype":{surf},"rows":5,"cols":4}}"#));
            let got: Vec<Vec<u32>> = d.prims.iter().map(|p| face_pts(&d, p)).collect();
            let want: Vec<Vec<u32>> = serde_json::from_str(&want).expect("recorded json");
            assert_eq!(got, want, "surftype {surf}");
            assert_closed_outward_unit_sphere(&d, true);
        }
        // rows 4 x cols 3, triangularpoles off: alternating, caps still triangles (odd cols keep the (band + col) parity)
        let d = cook(r#"{"type":2,"surftype":5,"rows":4,"cols":3,"triangularpoles":false}"#);
        let got: Vec<Vec<u32>> = d.prims.iter().map(|p| face_pts(&d, p)).collect();
        assert_eq!(
            got,
            [
                [0, 3, 2],
                [0, 4, 3],
                [0, 2, 4],
                [2, 3, 5],
                [3, 6, 5],
                [3, 4, 7],
                [3, 7, 6],
                [4, 2, 7],
                [2, 5, 7],
                [5, 6, 1],
                [6, 7, 1],
                [7, 5, 1]
            ]
        );
        // your file's sphere: 20 x 20 triangles = 362 points, 720 triangles (Houdini)
        let d = cook(r#"{"type":2,"surftype":3,"rows":20,"cols":20}"#);
        assert_eq!((d.point_count, d.prims.len()), (362, 720));
        assert!(d.prims.iter().all(|p| p.verts.len() == 3));
    }

    #[test]
    fn hostile_params_are_errors_not_panics() {
        for bad in [
            r#"{"type":2,"rows":2}"#,
            r#"{"type":2,"rows":-5}"#,
            r#"{"type":2,"cols":2}"#,
            r#"{"type":2,"rows":2000000000,"cols":2000000000}"#,
            r#"{"type":2,"rows":100000,"cols":100000}"#,
            r#"{"type":2,"surftype":0}"#,
            r#"{"type":1,"freq":0}"#,
            r#"{"type":1,"freq":-3}"#,
            r#"{"type":1,"freq":633}"#,
            r#"{"type":1,"freq":2147483647}"#,
            r#"{"type":1,"freq":-2147483648}"#,
            r#"{"type":3}"#,
            r#"{"type":4}"#,
            r#"{"type":5}"#,
            r#"{"type":6}"#,
            r#"{"type":99}"#,
            r#"{"orient":3}"#,
        ] {
            assert!(cook_json(&scene(bad)).is_err(), "accepted {bad}");
        }
    }
    #[test]
    fn rows_cols_freq_are_ignored_where_disabled() {
        // parms-sphere.md: rows/cols disabled for prim+poly, freq disabled unless poly
        assert!(cook_json(&scene(r#"{"rows":1,"cols":1,"freq":0}"#)).is_ok());
        assert!(cook_json(&scene(r#"{"type":1,"rows":1,"cols":1}"#)).is_ok());
        assert!(cook_json(&scene(r#"{"type":2,"freq":0}"#)).is_ok());
    }
    #[test]
    fn degenerate_radius_still_cooks() {
        let d = cook(r#"{"type":1,"rad":[0,0,0]}"#);
        assert_eq!(d.point_count, 42);
        assert!(d.p().iter().all(|q| *q == [0.0, 0.0, 0.0]));
    }

    // ---- analytical fixtures (independent maths, never produced by the SOP) ---------------------------------------
    fn fx(case: &str, file: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(case).join(file)
    }
    fn build(points: &[[f32; 3]], polys: &[Vec<u32>]) -> Detail {
        let mut d = Detail::default();
        d.add_points(points.len() as u32);
        d.p_mut().copy_from_slice(points);
        for p in polys {
            d.add_poly(p, true);
        }
        d
    }
    fn expected_prim() -> Detail {
        let mut d = Detail::default();
        d.add_points(1);
        d.p_mut()[0] = [1.0, 2.0, 3.0];
        d.vertex_point.push(0);
        d.prims.push(Prim {
            kind: PrimKind::Sphere { xform: [2.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 4.0] },
            verts: 0..1,
        });
        d
    }
    /// Houdini 20.5 `freq` 1 prims (hython): face k = `[R, L, A]` over the 12 icosahedron vertices.
    const H_F1_PRIMS: [[u32; 3]; 20] = [
        [2, 1, 0],
        [3, 2, 0],
        [4, 3, 0],
        [5, 4, 0],
        [1, 5, 0],
        [6, 5, 1],
        [7, 4, 5],
        [8, 3, 4],
        [9, 2, 3],
        [10, 1, 2],
        [10, 6, 1],
        [6, 7, 5],
        [7, 8, 4],
        [8, 9, 3],
        [9, 10, 2],
        [6, 10, 11],
        [10, 9, 11],
        [9, 8, 11],
        [8, 7, 11],
        [7, 6, 11],
    ];
    /// Houdini 20.5 `freq` 2 prims (hython), 4 per face: `[mAR, mAL, A] [mLR, L, mAL] [R, mLR, mAR] [mAR, mLR, mAL]`.
    const H_F2_PRIMS: &str = "[[2,1,0],[4,3,1],[5,4,2],[2,4,1],[6,2,0],[7,5,2],[8,7,6],[6,7,2],[9,6,0],[10,8,6],[11,10,9],\
        [9,10,6],[12,9,0],[13,11,9],[14,13,12],[12,13,9],[1,12,0],[15,14,12],[3,15,1],[1,15,12],[16,15,3],[17,14,15],\
        [18,17,16],[16,17,15],[19,13,14],[20,11,13],[21,20,19],[19,20,13],[22,10,11],[23,8,10],[24,23,22],[22,23,10],\
        [25,7,8],[26,5,7],[27,26,25],[25,26,7],[28,4,5],[29,3,4],[30,29,28],[28,29,4],[29,16,3],[31,18,16],[30,31,29],\
        [29,31,16],[17,19,14],[32,21,19],[18,32,17],[17,32,19],[20,22,11],[33,24,22],[21,33,20],[20,33,22],[23,25,8],\
        [34,27,25],[24,34,23],[23,34,25],[26,28,5],[35,30,28],[27,35,26],[26,35,28],[38,37,36],[31,30,37],[18,31,38],\
        [38,31,37],[37,39,36],[35,27,39],[30,35,37],[37,35,39],[39,40,36],[34,24,40],[27,34,39],[39,34,40],[40,41,36],\
        [33,21,41],[24,33,40],[40,33,41],[41,38,36],[32,18,38],[21,32,41],[41,32,38]]";
    /// Houdini's icosahedron by degrees (Y frame: vertex 0 = -Z, 11 = +Z, rings at z = -+1/sqrt5).
    fn h_ico() -> Vec<[f64; 3]> {
        let (z, r) = (1.0f64 / 5f64.sqrt(), 2.0f64 / 5f64.sqrt());
        let ring = |deg: f64, z: f64| [r * deg.to_radians().cos(), r * deg.to_radians().sin(), z];
        let mut u = vec![[0.0, 0.0, -1.0]];
        u.extend((0..5).map(|k| ring(90.0 - 72.0 * k as f64, -z)));
        u.extend((0..5).map(|k| ring(126.0 + 72.0 * k as f64, z)));
        u.push([0.0, 0.0, 1.0]);
        u
    }
    /// freq 2 = Houdini's recorded prim lists; each point's position read off its role in its face's first prims
    /// (corner = icosahedron vertex of `H_F1_PRIMS`, edge point = normalised midpoint of the two corners).
    fn expected_poly_f2() -> Detail {
        let ico = h_ico();
        let polys: Vec<Vec<u32>> = serde_json::from_str(H_F2_PRIMS).expect("recorded prims");
        let mut u = vec![[f64::NAN; 3]; 42];
        let mid = |a: usize, b: usize| {
            let m: [f64; 3] = std::array::from_fn(|k| ico[a][k] + ico[b][k]);
            let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
            m.map(|c| c / l)
        };
        for (k, f1) in H_F1_PRIMS.iter().enumerate() {
            let (a, l, r) = (f1[2] as usize, f1[1] as usize, f1[0] as usize);
            let (p0, p1, p2) = (&polys[4 * k], &polys[4 * k + 1], &polys[4 * k + 2]);
            u[p0[2] as usize] = ico[a];
            u[p1[1] as usize] = ico[l];
            u[p2[0] as usize] = ico[r];
            u[p0[1] as usize] = mid(a, l);
            u[p0[0] as usize] = mid(a, r);
            u[p1[0] as usize] = mid(l, r);
        }
        assert!(u.iter().flatten().all(|c| c.is_finite()), "every point placed");
        let (rad, t) = ([1.0, 2.0, 3.0], [0.5, 0.0, -1.0]);
        let pts: Vec<[f32; 3]> = u
            .iter()
            .map(|q| [(q[0] * rad[0] + t[0]) as f32, (q[1] * rad[1] + t[1]) as f32, (q[2] * rad[2] + t[2]) as f32])
            .collect();
        build(&pts, &polys)
    }
    #[test]
    fn poly_f1_is_houdinis_icosahedron() {
        let d = cook(r#"{"type":1,"freq":1}"#);
        for (i, want) in h_ico().into_iter().enumerate() {
            near(d.p()[i], want, 1e-6, &format!("P[{i}]"));
        }
        let got: Vec<Vec<u32>> = d.prims.iter().map(|p| face_pts(&d, p)).collect();
        assert_eq!(got, H_F1_PRIMS.map(|p| p.to_vec()));
    }
    /// 13 point rows from latitude +90deg..-90deg; the two polar rows collapse to one point each (rows 0 and 12).
    fn expected_polymesh_default() -> Detail {
        let (rows, cols) = (13usize, 24usize);
        // Houdini order: top pole (0), bottom pole (1), then ring points; ring runs +X toward -Z
        let mut pts: Vec<[f32; 3]> = vec![[0.0, 1.0, 0.0], [0.0, -1.0, 0.0]];
        for row in 1..rows - 1 {
            let lat = (90.0 - 180.0 * row as f64 / (rows - 1) as f64).to_radians();
            for c in 0..cols {
                let lon = (360.0 * c as f64 / cols as f64).to_radians();
                pts.push([(lat.cos() * lon.cos()) as f32, lat.sin() as f32, -(lat.cos() * lon.sin()) as f32]);
            }
        }
        let (top, bot) = (0u32, 1u32);
        let at = |row: usize, c: usize| (2 + (row - 1) * cols + c % cols) as u32;
        let mut polys = Vec::new();
        for c in 0..cols {
            polys.push(vec![top, at(1, c + 1), at(1, c)]);
        }
        for row in 1..rows - 2 {
            for c in 0..cols {
                polys.push(vec![at(row, c), at(row, c + 1), at(row + 1, c + 1), at(row + 1, c)]);
            }
        }
        for c in 0..cols {
            polys.push(vec![at(rows - 2, c), at(rows - 2, c + 1), bot]);
        }
        build(&pts, &polys)
    }
    fn same(a: &Detail, b: &Detail) {
        assert_eq!((a.point_count, &a.vertex_point, &a.prims), (b.point_count, &b.vertex_point, &b.prims));
        assert_eq!(a.p().len(), b.p().len());
        for (i, (x, y)) in a.p().iter().zip(b.p()).enumerate() {
            assert!(x.iter().zip(y).all(|(m, n)| (m - n).abs() <= 1e-5), "P[{i}] {x:?} != {y:?}");
        }
        let names = |d: &Detail| d.attrs.iter().map(|x| (x.class, x.name.clone())).collect::<Vec<_>>();
        assert_eq!(names(a), names(b));
        assert!(a.attrs.iter().all(|x| x.class != AttrClass::Detail));
    }
    fn cases() -> [(&'static str, Detail); 3] {
        [
            ("sphere-prim", expected_prim()),
            ("sphere-poly-f2", expected_poly_f2()),
            ("sphere-polymesh-default", expected_polymesh_default()),
        ]
    }
    /// `cargo test -p hexe-sop -- --ignored write_fixture` writes the analytical expecteds.
    #[test]
    #[ignore]
    fn write_fixture() {
        for (case, d) in cases() {
            std::fs::write(fx(case, "expected.geo"), write_geo_json(&d)).expect("write");
        }
    }
    #[test]
    fn fixtures_match_sop_output_and_independent_expected() {
        let mut counts = BTreeMap::new();
        for (case, want) in cases() {
            let json = std::fs::read_to_string(fx(case, "scene.json")).expect("scene.json");
            let got = cook_json(&json).expect("cooks");
            let file = read_geo(&std::fs::read(fx(case, "expected.geo")).expect("expected.geo")).expect("parses");
            same(&got, &want); // SOP vs closed-form maths in this file
            same(&file, &want); // committed expected.geo vs same maths
            same(&got, &file); // SOP vs committed file (what the oracle sees)
            counts.insert(case, (got.point_count, got.prims.len()));
        }
        assert_eq!(counts.get("sphere-prim"), Some(&(1, 1)));
        assert_eq!(counts.get("sphere-poly-f2"), Some(&(42, 80)));
        assert_eq!(counts.get("sphere-polymesh-default"), Some(&(266, 288)));
    }
}
