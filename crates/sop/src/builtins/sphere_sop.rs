//! Sphere SOP: generator. `type` prim = 1 point + `Prim::Sphere` · poly = subdivided icosahedron · polymesh = lat/long quads.
//!
//! Closed forms (all UNVERIFIED vs Houdini; hython blocked, see parity/features.toml `sop-sphere`):
//! - prim: 1 pt (= `t`), 1 vertex, 1 `Sphere` prim, xform = R·diag(rad·scale)·O (row-major 3x3).
//! - poly, `freq`=f: 10f²+2 pts, 20f² tris. Icosahedron with poles on the pole axis; every face split on a barycentric
//!   lattice, shared corner/edge points deduplicated, points projected onto the unit sphere.
//! - polymesh, `rows`=R `cols`=C: the poles FIRST (shared: top 0, bottom 1; `upole`: 2·C) then ring points (R-2)·C; ring angle runs +X toward -Z
//!   (verified vs real Houdini 20.5, fixtures/sop-sphere/houdini.geo). Houdini lists cells clockwise; hexe emits the same first corner
//!   reversed (counter-clockwise, outward). Prim order = Houdini: top fan, ring rows, bottom fan. Prims (R-1)·C =
//!   (R-3)·C quads + 2·C pole faces (tris when `triangularpoles`, else quads that touch the pole). `upole` on: R·C pts.
//! - P = R(rad·scale ∘ O(unit)) + t. O = pole-axis permutation (y identity, x: (y,z,x), z: (z,x,y)), R = Rx then Ry then Rz.
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::Arc;

/// Menu positions (parms-sphere.md `type`).
const TY_PRIM: u32 = 0;
const TY_POLY: u32 = 1;
const TY_POLYMESH: u32 = 2;
/// `surftype` menu position of `quads` (the only polymesh connectivity implemented).
const SURF_QUADS: u32 = 4;
/// Hard caps (AGENTS.md §4: capped allocations).
const MAX_FREQ: i32 = 100;
const MAX_POINTS: usize = 4_000_000;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(TY_PRIM)).label("Primitive Type"),
        ParamSpec::new("surftype", ParamValue::Menu(SURF_QUADS)).label("Connectivity"),
        ParamSpec::new("rad", ParamValue::V3([1.0, 1.0, 1.0])).label("Radius").range(-1.0, 1.0),
        ParamSpec::new("t", ParamValue::V3([0.0; 3])).label("Center").range(-1.0, 1.0),
        ParamSpec::new("r", ParamValue::V3([0.0; 3])).label("Rotate").range(0.0, 360.0),
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Uniform Scale").range(0.0, 10.0),
        ParamSpec::new("orient", ParamValue::Menu(1)).label("Orientation").range(0.0, 1.0),
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

/// Icosahedron with vertex 0 / 11 on the +Y / -Y pole, rings of 5 at y = ±1/√5 (lower ring offset 36°).
fn ico_vertices() -> Vec<[f64; 3]> {
    let (ry, rr) = (1.0 / 5f64.sqrt(), 2.0 / 5f64.sqrt());
    let ring = |k: usize, off: f64, y: f64| {
        let a = 2.0 * PI * k as f64 / 5.0 + off;
        [rr * a.cos(), y, rr * a.sin()]
    };
    let mut v = vec![[0.0, 1.0, 0.0]];
    v.extend((0..5).map(|k| ring(k, 0.0, ry)));
    v.extend((0..5).map(|k| ring(k, PI / 5.0, -ry)));
    v.push([0.0, -1.0, 0.0]);
    v
}
/// 20 outward-CCW faces (vertex indices into `ico_vertices`): per k top, down, up, bottom.
fn ico_faces() -> Vec<[usize; 3]> {
    let mut f = Vec::with_capacity(20);
    for k in 0..5usize {
        let n = (k + 1) % 5;
        f.push([0, 1 + n, 1 + k]);
        f.push([1 + k, 1 + n, 6 + k]);
        f.push([6 + k, 6 + (k + 4) % 5, 1 + k]);
        f.push([6 + k, 6 + n, 11]);
    }
    f
}

fn normalized(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 { [v[0] / l, v[1] / l, v[2] / l] } else { v }
}

/// Dedup key of a lattice point: corner / edge / face interior.
type Key = (u8, usize, usize, usize);

fn poly(freq: usize, xf: &Xf, t: [f32; 3]) -> Detail {
    let base = ico_vertices();
    let mut d = Detail::default();
    let mut ids: BTreeMap<Key, u32> = BTreeMap::new();
    let mut unit: Vec<[f64; 3]> = base.clone();
    for i in 0..base.len() {
        ids.insert((0, i, 0, 0), u32::try_from(i).unwrap_or(0));
    }
    let mut tris: Vec<[u32; 3]> = Vec::with_capacity(20 * freq * freq);
    for (fi, face) in ico_faces().iter().enumerate() {
        let [a, b, c] = *face;
        // lattice point (r, s): weights a = f-r-s, b = s, c = r
        let mut id_of = |r: usize, s: usize| -> u32 {
            let (wa, wb, wc) = (freq - r - s, s, r);
            let key: Key = match (wa == 0, wb == 0, wc == 0) {
                (false, true, true) => (0, a, 0, 0),
                (true, false, true) => (0, b, 0, 0),
                (true, true, false) => (0, c, 0, 0),
                (false, false, true) => edge_key(a, b, wa, wb),
                (false, true, false) => edge_key(a, c, wa, wc),
                (true, false, false) => edge_key(b, c, wb, wc),
                _ => (2, fi, r, s),
            };
            if let Some(&id) = ids.get(&key) {
                return id;
            }
            let w = |wt: usize, v: [f64; 3]| [wt as f64 * v[0], wt as f64 * v[1], wt as f64 * v[2]];
            let (pa, pb, pc) = (w(wa, at(&base, a)), w(wb, at(&base, b)), w(wc, at(&base, c)));
            let sum = [pa[0] + pb[0] + pc[0], pa[1] + pb[1] + pc[1], pa[2] + pb[2] + pc[2]];
            let id = u32::try_from(unit.len()).unwrap_or(u32::MAX);
            unit.push(normalized(sum));
            ids.insert(key, id);
            id
        };
        for r in 0..freq {
            for s in 0..(freq - r) {
                tris.push([id_of(r, s), id_of(r, s + 1), id_of(r + 1, s)]);
                if s + 1 < freq - r {
                    tris.push([id_of(r, s + 1), id_of(r + 1, s + 1), id_of(r + 1, s)]);
                }
            }
        }
    }
    d.add_points(u32::try_from(unit.len()).unwrap_or(u32::MAX));
    for (dst, u) in d.p_mut().iter_mut().zip(&unit) {
        *dst = xf.p(*u, t);
    }
    for tri in &tris {
        d.add_poly(tri, true);
    }
    d
}
/// Edge point shared by both faces: sorted vertex pair + weight on the larger vertex.
fn edge_key(u: usize, v: usize, wu: usize, wv: usize) -> Key {
    if u < v { (1, u, v, wv) } else { (1, v, u, wu) }
}

fn polymesh(rows: usize, cols: usize, upole: bool, tripoles: bool, xf: &Xf, t: [f32; 3]) -> Result<Detail, CookError> {
    let rings = rows - 2;
    let ring_pts = rings.checked_mul(cols);
    let pole_pts = if upole { 2 * cols } else { 2 };
    let total = ring_pts.and_then(|n| n.checked_add(pole_pts)).filter(|&n| n <= MAX_POINTS);
    let (Some(ring_pts), Some(total)) = (ring_pts, total) else {
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
            d.add_poly(&[top(j), ring(1, j), ring(1, j + 1)], true);
        } else {
            d.add_poly(&[top(j), ring(1, j), ring(1, j + 1), top(j + 1)], true);
        }
    }
    for i in 1..rings {
        for j in 0..cols {
            d.add_poly(&[ring(i, j), ring(i + 1, j), ring(i + 1, j + 1), ring(i, j + 1)], true);
        }
    }
    for j in 0..cols {
        if tripoles {
            d.add_poly(&[ring(rings, j), bot(j), ring(rings, j + 1)], true);
        } else {
            d.add_poly(&[ring(rings, j), bot(j), bot(j + 1), ring(rings, j + 1)], true);
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
                if !(1..=MAX_FREQ).contains(&f) {
                    return Err(CookError::BadParam(format!("sphere: freq {f} not in 1..={MAX_FREQ}")));
                }
                poly(usize::try_from(f).map_err(|_| bad("freq"))?, &xf, t)
            }
            TY_POLYMESH => {
                let (rows, cols) = (int(p, "rows")?, int(p, "cols")?);
                if rows < 3 || cols < 3 {
                    return Err(CookError::BadParam(format!("sphere: rows {rows} / cols {cols} must be >= 3")));
                }
                if menu(p, "surftype")? != SURF_QUADS {
                    return Err(CookError::BadParam("sphere: polymesh supports surftype quads only".into()));
                }
                let (r, c) =
                    (usize::try_from(rows).map_err(|_| bad("rows"))?, usize::try_from(cols).map_err(|_| bad("cols"))?);
                polymesh(r, c, toggle(p, "upole")?, toggle(p, "triangularpoles")?, &xf, t)?
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
            // Newell normal
            let mut n = [0.0f32; 3];
            for i in 0..pos.len() {
                let (a, b) = (pos[i], pos[(i + 1) % pos.len()]);
                let x = cross(a, b);
                n = [n[0] + x[0], n[1] + x[1], n[2] + x[2]];
            }
            assert!(dot(n, c) > 0.0, "face not outward: {f:?}");
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
        for f in [1usize, 2, 3, 4, 7, 10, 100] {
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
    #[test]
    fn poly_has_pole_vertices_on_orient_axis() {
        for (o, axis) in [(0, 0usize), (1, 1), (2, 2)] {
            let d = cook(&format!(r#"{{"type":1,"orient":{o}}}"#));
            for s in [1.0f32, -1.0] {
                assert!(d.p().iter().any(|q| (q[axis] - s).abs() < 1e-6
                    && q[(axis + 1) % 3].abs() < 1e-6
                    && q[(axis + 2) % 3].abs() < 1e-6));
            }
            assert_closed_outward_unit_sphere(&d, true); // cyclic permutation keeps handedness
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
            r#"{"type":1,"freq":101}"#,
            r#"{"type":1,"freq":2147483647}"#,
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
    /// Icosahedron by degrees, faces literal, f=2 via edge midpoints (map keyed by sorted vertex pair).
    fn expected_poly_f2() -> Detail {
        let (y, r) = (1.0f64 / 5f64.sqrt(), 2.0f64 / 5f64.sqrt());
        let mut u: Vec<[f64; 3]> = vec![[0.0, 1.0, 0.0]];
        for k in 0..5 {
            let a = (72.0 * k as f64).to_radians();
            u.push([r * a.cos(), y, r * a.sin()]);
        }
        for k in 0..5 {
            let a = (72.0 * k as f64 + 36.0).to_radians();
            u.push([r * a.cos(), -y, r * a.sin()]);
        }
        u.push([0.0, -1.0, 0.0]);
        let faces: [[usize; 3]; 20] = [
            [0, 2, 1],
            [1, 2, 6],
            [6, 10, 1],
            [6, 7, 11],
            [0, 3, 2],
            [2, 3, 7],
            [7, 6, 2],
            [7, 8, 11],
            [0, 4, 3],
            [3, 4, 8],
            [8, 7, 3],
            [8, 9, 11],
            [0, 5, 4],
            [4, 5, 9],
            [9, 8, 4],
            [9, 10, 11],
            [0, 1, 5],
            [5, 1, 10],
            [10, 9, 5],
            [10, 6, 11],
        ];
        let mut mids: HashMap<(usize, usize), usize> = HashMap::new();
        let mut mid = |u: &mut Vec<[f64; 3]>, a: usize, b: usize| -> u32 {
            let key = (a.min(b), a.max(b));
            if let Some(&i) = mids.get(&key) {
                return i as u32;
            }
            let (p, q) = (u[a], u[b]);
            let m = [p[0] + q[0], p[1] + q[1], p[2] + q[2]];
            let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
            u.push([m[0] / l, m[1] / l, m[2] / l]);
            mids.insert(key, u.len() - 1);
            (u.len() - 1) as u32
        };
        let mut polys = Vec::new();
        for [a, b, c] in faces {
            let (mab, mac) = (mid(&mut u, a, b), mid(&mut u, a, c));
            let mbc = mid(&mut u, b, c);
            let (a, b, c) = (a as u32, b as u32, c as u32);
            polys.extend([vec![a, mab, mac], vec![mab, mbc, mac], vec![mab, b, mbc], vec![mac, mbc, c]]);
        }
        let (rad, t) = ([1.0, 2.0, 3.0], [0.5, 0.0, -1.0]);
        let pts: Vec<[f32; 3]> = u
            .iter()
            .map(|q| [(q[0] * rad[0] + t[0]) as f32, (q[1] * rad[1] + t[1]) as f32, (q[2] * rad[2] + t[2]) as f32])
            .collect();
        build(&pts, &polys)
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
            polys.push(vec![top, at(1, c), at(1, c + 1)]);
        }
        for row in 1..rows - 2 {
            for c in 0..cols {
                polys.push(vec![at(row, c), at(row + 1, c), at(row + 1, c + 1), at(row, c + 1)]);
            }
        }
        for c in 0..cols {
            polys.push(vec![at(rows - 2, c), bot, at(rows - 2, c + 1)]);
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
