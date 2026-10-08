//! Grid SOP: planar `rows x cols` POINT lattice. Generator (no inputs). All 12 harvested parms are declared (`parms-grid.md`).
//! `type`: poly | polysoup | mesh | points implemented; nurbs | bezier -> `CookError::Unsupported` (never a fallback).
//! `surftype` (poly/polysoup/mesh; ignored for points): rows cols rowcol triangles quads alttriangles revtriangles.
//! `orderu/orderv/interpu/interpv` are read (declared, typed) but IGNORED: they only matter for nurbs/bezier.
//! Lattice (order verified against real Houdini 20.5, `fixtures/sop-grid/houdini.geo` + xy/yz/zx cases in docs/HOUDINI-ORACLE.md):
//! point id = `row*cols + col`; `cols` span the FIRST in-plane axis over `size[0]`, ascending from `-size[0]/2`; `rows` span
//! the SECOND over `size[1]`, ascending from `-size[1]/2` (row 0 at the LOW end, also for `zx`: row 0 at -Z). Axes: xy: (X, Y);
//! yz: (Y, Z); zx: (X, Z). Houdini's cell is `[a, a+1, a+cols+1, a+cols]` (clockwise, its normal is the left-hand rule); hexe adopts
//! that winding globally (H32) and emits it verbatim: `meshutil::prim_normal` xy -Z, yz +X, zx +Y = Houdini's own prim normals. `r` rotates about the local origin (degrees,
//! Rx then Ry then Rz), then `t` translates. A 1-point axis sits at the center of that axis (no division by zero).
//! No `N` attribute (existence [?]). `size` is declared `V3` (no V2 variant): `size[2]` is ignored.
//! `mesh` = ONE `Unknown{ty:"Mesh"}` prim over all `rows*cols` vertices (row-major) with the connectivity token in its
//! json body (entry layout UNVERIFIED). `polysoup` = same polygons as `poly` (data model has no POLYSOUP prim kind).
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
/// Allocation cap: `rows * cols` above this is `CookError::Failed` (hostile input, AGENTS.md §4).
const MAX_POINTS: u64 = 1 << 20;
/// `type` menu indices (table order: poly mesh nurbs bezier points polysoup).
const TYPE_POLY: u32 = 0;
const TYPE_MESH: u32 = 1;
const TYPE_NURBS: u32 = 2;
const TYPE_BEZIER: u32 = 3;
const TYPE_POINTS: u32 = 4;
const TYPE_POLYSOUP: u32 = 5;
/// `surftype` menu indices and tokens (table order: rows cols rowcol triangles quads alttriangles revtriangles).
const SURF_TOKENS: [&str; 7] = ["rows", "cols", "rowcol", "triangles", "quads", "alttriangles", "revtriangles"];
const SURF_ROWS: u32 = 0;
const SURF_COLS: u32 = 1;
const SURF_ROWCOL: u32 = 2;
const SURF_TRIANGLES: u32 = 3;
const SURF_QUADS: u32 = 4;
const SURF_ALT: u32 = 5;
const SURF_REV: u32 = 6;
/// `orient` menu indices (xy yz zx).
const ORIENT_XY: u32 = 0;
const ORIENT_YZ: u32 = 1;
const ORIENT_ZX: u32 = 2;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(TYPE_POLY)).label("Primitive Type"),
        ParamSpec::new("surftype", ParamValue::Menu(SURF_QUADS)).label("Connectivity"),
        ParamSpec::new("orient", ParamValue::Menu(ORIENT_ZX)).label("Orientation"),
        ParamSpec::new("size", ParamValue::V3([10.0, 10.0, 0.0])).label("Size").range(-1.0, 1.0),
        ParamSpec::new("t", ParamValue::V3([0.0, 0.0, 0.0])).label("Center").range(-1.0, 1.0),
        ParamSpec::new("r", ParamValue::V3([0.0, 0.0, 0.0])).label("Rotate").range(0.0, 360.0),
        ParamSpec::new("rows", ParamValue::I(10)).label("Rows").range(1.0, 50.0),
        ParamSpec::new("cols", ParamValue::I(10)).label("Columns").range(1.0, 50.0),
        ParamSpec::new("orderu", ParamValue::I(4)).label("U Order").range(2.0, 11.0),
        ParamSpec::new("orderv", ParamValue::I(4)).label("V Order").range(2.0, 11.0),
        ParamSpec::new("interpu", ParamValue::B(true)).label("End-point interpolate in U"),
        ParamSpec::new("interpv", ParamValue::B(true)).label("End-point interpolate in V"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Grid { specs: spec() })
}
struct Grid {
    specs: Vec<ParamSpec>,
}
fn bad(name: &str, what: &str) -> CookError {
    CookError::BadParam(format!("grid.{name}: {what}"))
}
fn to_u32(v: usize) -> Result<u32, CookError> {
    u32::try_from(v).map_err(|_| CookError::Failed("grid: index exceeds u32".into()))
}
fn finite3(name: &str, v: [f32; 3]) -> Result<[f64; 3], CookError> {
    if v.iter().all(|c| c.is_finite()) {
        Ok([f64::from(v[0]), f64::from(v[1]), f64::from(v[2])])
    } else {
        Err(bad(name, "non-finite component"))
    }
}
/// Rotate `p` by `deg` (x, y, z degrees): Rx first, then Ry, then Rz. Identity (no float residue) when all zero.
fn rotate(p: [f64; 3], deg: [f64; 3]) -> [f64; 3] {
    if deg == [0.0; 3] {
        return p;
    }
    let (sx, cx) = deg[0].to_radians().sin_cos();
    let (sy, cy) = deg[1].to_radians().sin_cos();
    let (sz, cz) = deg[2].to_radians().sin_cos();
    let [x, y, z] = p;
    let (y, z) = (y * cx - z * sx, y * sx + z * cx);
    let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
    let (x, y) = (x * cz - y * sz, x * sz + y * cz);
    [x, y, z]
}
/// Coordinate of lattice index `i` of `n` over extent `s`, ascending from `-s/2`; a single sample sits at the center.
fn axis(i: usize, n: usize, s: f64) -> f64 {
    if n < 2 { 0.0 } else { -s / 2.0 + s * (i as f64) / ((n - 1) as f64) }
}
/// Mesh prim json body (entry layout UNVERIFIED): vertices row-major, extents, connectivity token.
fn mesh_json(rows: usize, cols: usize, surf: u32) -> String {
    let verts: Vec<String> = (0..rows * cols).map(|i| i.to_string()).collect();
    let token = SURF_TOKENS.get(surf as usize).copied().unwrap_or("quads");
    format!(
        r#"[["type","Mesh"],["vertex",[{}],"ucount",{cols},"vcount",{rows},"uwrap",false,"vwrap",false,"connectivity","{token}"]]"#,
        verts.join(",")
    )
}
impl Node<Detail> for Grid {
    fn type_name(&self) -> &'static str {
        "grid"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, _: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let ty = p.menu("type").ok_or_else(|| bad("type", "missing or not a menu"))?;
        let surf = p.menu("surftype").ok_or_else(|| bad("surftype", "missing or not a menu"))?;
        let orient = p.menu("orient").ok_or_else(|| bad("orient", "missing or not a menu"))?;
        let size = p.vec3("size").ok_or_else(|| bad("size", "missing or not a vector"))?;
        let t = p.vec3("t").ok_or_else(|| bad("t", "missing or not a vector"))?;
        let r = p.vec3("r").ok_or_else(|| bad("r", "missing or not a vector"))?;
        let rows_i = p.int("rows").ok_or_else(|| bad("rows", "missing or not an integer"))?;
        let cols_i = p.int("cols").ok_or_else(|| bad("cols", "missing or not an integer"))?;
        // NURBS/Bezier-only parms: must be well-typed, otherwise ignored.
        p.int("orderu").ok_or_else(|| bad("orderu", "missing or not an integer"))?;
        p.int("orderv").ok_or_else(|| bad("orderv", "missing or not an integer"))?;
        p.toggle("interpu").ok_or_else(|| bad("interpu", "missing or not a toggle"))?;
        p.toggle("interpv").ok_or_else(|| bad("interpv", "missing or not a toggle"))?;
        if ty > TYPE_POLYSOUP {
            return Err(bad("type", "menu index out of range 0..=5"));
        }
        if surf as usize >= SURF_TOKENS.len() {
            return Err(bad("surftype", "menu index out of range 0..=6"));
        }
        if orient > ORIENT_ZX {
            return Err(bad("orient", "menu index out of range 0..=2"));
        }
        if rows_i < 1 || cols_i < 1 {
            return Err(bad("rows/cols", "need at least 1 point row and 1 point column"));
        }
        let size = finite3("size", size)?;
        let c = finite3("t", t)?;
        let rot = finite3("r", r)?;
        if matches!(ty, TYPE_NURBS | TYPE_BEZIER) {
            let name = if ty == TYPE_NURBS { "nurbs" } else { "bezier" };
            return Err(CookError::Unsupported(format!("grid type `{name}` is not implemented")));
        }
        let (rows, cols) = (rows_i as u32 as usize, cols_i as u32 as usize);
        if (rows as u64) * (cols as u64) > MAX_POINTS {
            return Err(CookError::Failed(format!("grid: {rows}x{cols} points exceeds the {MAX_POINTS} point cap")));
        }
        let mut d = Detail::default();
        let ids = d.add_points(to_u32(rows * cols)?);
        let base = ids.start;
        let pos = d.p_mut();
        pos.clear();
        for row in 0..rows {
            let v = axis(row, rows, size[1]);
            for col in 0..cols {
                let u = axis(col, cols, size[0]);
                let local = match orient {
                    ORIENT_XY => [u, v, 0.0],
                    ORIENT_YZ => [0.0, v, u],
                    _ => [u, 0.0, v],
                };
                let q = rotate(local, rot);
                pos.push([(c[0] + q[0]) as f32, (c[1] + q[1]) as f32, (c[2] + q[2]) as f32]);
            }
        }
        if ty == TYPE_POINTS {
            return Ok(Arc::new(d));
        }
        let id = |row: usize, col: usize| -> Result<u32, CookError> { Ok(base + to_u32(row * cols + col)?) };
        if ty == TYPE_MESH {
            for row in 0..rows {
                for col in 0..cols {
                    d.vertex_point.push(id(row, col)?);
                }
            }
            let n = d.vertex_count();
            d.prims.push(Prim {
                kind: PrimKind::Unknown { ty: "Mesh".into(), json: mesh_json(rows, cols, surf) },
                verts: 0..n,
            });
            return Ok(Arc::new(d));
        }
        // poly / polysoup: polygons or polylines over the lattice.
        let want_rows = matches!(surf, SURF_ROWS | SURF_ROWCOL);
        let want_cols = matches!(surf, SURF_COLS | SURF_ROWCOL);
        if want_rows && cols >= 2 {
            for row in 0..rows {
                let line = (0..cols).map(|col| id(row, col)).collect::<Result<Vec<_>, _>>()?;
                d.add_poly(&line, false);
            }
        }
        if want_cols && rows >= 2 {
            for col in 0..cols {
                let line = (0..rows).map(|row| id(row, col)).collect::<Result<Vec<_>, _>>()?;
                d.add_poly(&line, false);
            }
        }
        if matches!(surf, SURF_TRIANGLES | SURF_QUADS | SURF_ALT | SURF_REV) {
            for row in 0..rows.saturating_sub(1) {
                for col in 0..cols.saturating_sub(1) {
                    // cell corners in winding order: a(row,col) b(row+1,col) cc(row+1,col+1) dd(row,col+1). Houdini lists the cell // clockwise as `[a, dd, cc, b]`, emitted verbatim (hexe adopts Houdini's winding globally).
                    let (a, b, cc, dd) = (id(row, col)?, id(row + 1, col)?, id(row + 1, col + 1)?, id(row, col + 1)?);
                    // triangles: diagonal a-cc; revtriangles: diagonal b-dd; alttriangles: checkerboard of the two.
                    let rev = surf == SURF_REV || (surf == SURF_ALT && (row + col) % 2 == 1);
                    match (surf, rev) {
                        (SURF_QUADS, _) => {
                            d.add_poly(&[a, dd, cc, b], true);
                        }
                        (_, false) => {
                            d.add_poly(&[a, cc, b], true);
                            d.add_poly(&[a, dd, cc], true);
                        }
                        (_, true) => {
                            d.add_poly(&[a, dd, b], true);
                            d.add_poly(&[b, dd, cc], true);
                        }
                    }
                }
            }
        }
        Ok(Arc::new(d))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::{AttrClass, read_geo, read_geo_json, write_geo_json};
    fn scene(params: &str) -> String {
        format!(r#"{{"nodes":[{{"name":"g","type":"grid","params":{params}}}],"display":"g"}}"#)
    }
    fn cook(params: &str) -> Result<Arc<Detail>, String> {
        let (mut g, id) = load_scene(&scene(params)).map_err(|e| e.to_string())?;
        g.cook(id, 0.0).map_err(|e| e.to_string())
    }
    fn ok(params: &str) -> Arc<Detail> {
        cook(params).expect("cooks")
    }
    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-5)
    }
    fn prim_pts(d: &Detail, i: usize) -> Vec<u32> {
        let pr = &d.prims[i];
        d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].to_vec()
    }
    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }
    /// Houdini-rule (left-hand) geometric normal of polygon `i` from its first three vertices (test-local oracle).
    fn normal(d: &Detail, i: usize) -> [f32; 3] {
        let v = prim_pts(d, i);
        let q: Vec<[f32; 3]> = v.iter().map(|&k| d.p()[k as usize]).collect();
        cross(sub(q[2], q[0]), sub(q[1], q[0]))
    }
    #[test]
    fn default_is_100_points_81_quads_on_zx_plane() {
        let d = ok("{}");
        assert_eq!((d.point_count, d.vertex_count(), d.prim_count()), (100, 324, 81));
        assert!(d.prims.iter().all(|p| matches!(p.kind, PrimKind::Poly { closed: true }) && p.verts.len() == 4));
        assert!(d.p().iter().all(|p| p[1] == 0.0));
        // zx: x ascends over cols, z ASCENDS over rows (Houdini: row 0 at -Z): point 0 = (-5,0,-5).
        assert!(near(d.p()[0], [-5.0, 0.0, -5.0]));
        assert!(near(d.p()[9], [5.0, 0.0, -5.0]));
        assert!(near(d.p()[90], [-5.0, 0.0, 5.0]));
        assert!(near(d.p()[99], [5.0, 0.0, 5.0]));
        assert!(d.attr(AttrClass::Point, "P").is_some() && d.attrs.len() == 1);
    }
    #[test]
    fn closed_form_counts_for_every_surftype() {
        for (rows, cols) in [(2usize, 2usize), (3, 5), (7, 4), (1, 6), (6, 1), (1, 1)] {
            let p = |s: u32| format!(r#"{{"surftype":{s},"rows":{rows},"cols":{cols}}}"#);
            let cells = rows.saturating_sub(1) * cols.saturating_sub(1);
            let quads = ok(&p(4));
            assert_eq!(quads.point_count as usize, rows * cols);
            assert_eq!(quads.prim_count() as usize, (rows - 1) * (cols - 1));
            for s in [3, 5, 6] {
                let tris = ok(&p(s));
                assert_eq!(tris.point_count as usize, rows * cols);
                assert_eq!(tris.prim_count() as usize, 2 * cells, "surftype {s}");
                assert!(tris.prims.iter().all(|p| p.verts.len() == 3));
            }
            let r = ok(&p(0));
            let c = ok(&p(1));
            let rc = ok(&p(2));
            let (nr, nc) = (if cols >= 2 { rows } else { 0 }, if rows >= 2 { cols } else { 0 });
            assert_eq!(r.prim_count() as usize, nr);
            assert_eq!(c.prim_count() as usize, nc);
            assert_eq!(rc.prim_count() as usize, nr + nc);
            assert!(
                r.prims.iter().all(|p| matches!(p.kind, PrimKind::Poly { closed: false }) && p.verts.len() == cols)
            );
            assert!(
                c.prims.iter().all(|p| matches!(p.kind, PrimKind::Poly { closed: false }) && p.verts.len() == rows)
            );
        }
    }
    #[test]
    fn first_quad_winding_is_0_1_cols_plus_1_cols_for_every_orient() {
        for orient in 0..3 {
            for (rows, cols) in [(2, 2), (3, 4), (5, 3)] {
                let d = ok(&format!(r#"{{"orient":{orient},"rows":{rows},"cols":{cols}}}"#));
                let c = cols as u32;
                assert_eq!(prim_pts(&d, 0), vec![0, 1, c + 1, c], "orient {orient} {rows}x{cols}");
                // second quad of the first row is shifted by one column.
                assert_eq!(
                    prim_pts(&d, 1.min(d.prims.len() - 1)),
                    if cols > 2 { vec![1, 2, c + 2, c + 1] } else { vec![0, 1, c + 1, c] }
                );
            }
        }
    }
    #[test]
    fn quad_normals_follow_the_orient_plane_normal() {
        // Houdini prim normals (left-hand rule on its clockwise cells, measured): xy -Z, yz +X, zx +Y; ours = the same cells // verbatim, so the same vectors. Every quad.
        for (o, axis, sign) in [(0, 2, -1.0f32), (1, 0, 1.0), (2, 1, 1.0)] {
            let d = ok(&format!(r#"{{"orient":{o},"rows":3,"cols":4}}"#));
            for i in 0..d.prims.len() {
                let n = normal(&d, i);
                assert!(n[axis] * sign > 0.0, "orient {o} prim {i}: {n:?}");
                assert!(n.iter().enumerate().all(|(k, c)| k == axis || c.abs() < 1e-4));
            }
        }
    }
    #[test]
    fn corner_positions_for_each_orient_size_and_center() {
        // 2x3 lattice, size (4,2), center (10,20,30): u in {-2,0,2} over cols, v in {-1,1} over rows; cols = X (xy, zx) or Z (yz), rows = Y (xy, yz) or Z (zx).
        let base = |o: u32| ok(&format!(r#"{{"orient":{o},"rows":2,"cols":3,"size":[4,2,0],"t":[10,20,30]}}"#));
        let xy = base(0);
        assert!(near(xy.p()[0], [8.0, 19.0, 30.0]) && near(xy.p()[2], [12.0, 19.0, 30.0]));
        assert!(near(xy.p()[3], [8.0, 21.0, 30.0]) && near(xy.p()[5], [12.0, 21.0, 30.0]));
        let yz = base(1);
        assert!(near(yz.p()[0], [10.0, 19.0, 28.0]) && near(yz.p()[2], [10.0, 19.0, 32.0]));
        assert!(near(yz.p()[3], [10.0, 21.0, 28.0]) && near(yz.p()[5], [10.0, 21.0, 32.0]));
        let zx = base(2);
        assert!(near(zx.p()[0], [8.0, 20.0, 29.0]) && near(zx.p()[2], [12.0, 20.0, 29.0]));
        assert!(near(zx.p()[3], [8.0, 20.0, 31.0]) && near(zx.p()[5], [12.0, 20.0, 31.0]));
    }
    #[test]
    fn interior_points_are_evenly_spaced() {
        let d = ok(r#"{"orient":0,"rows":4,"cols":5,"size":[8,6,0]}"#);
        for row in 0..4usize {
            for col in 0..5usize {
                let want = [-4.0 + 2.0 * col as f32, -3.0 + 2.0 * row as f32, 0.0];
                assert!(near(d.p()[row * 5 + col], want), "({row},{col})");
            }
        }
    }
    #[test]
    fn rotate_param_turns_the_plane_about_the_center() {
        // zx 2x2 size 10 center (1,2,3): point 0 local (-5,0,-5). Ry(90): (x,z) -> (z,-x) => (-5,0,5).
        let d = ok(r#"{"rows":2,"cols":2,"r":[0,90,0],"t":[1,2,3]}"#);
        assert!(near(d.p()[0], [-4.0, 2.0, 8.0]));
        assert!(near(d.p()[1], [-4.0, 2.0, -2.0]));
        // Rx(90) on xy: local (x,y,0) -> (x,0,y): the XY grid lies flat in the XZ plane.
        let d = ok(r#"{"orient":0,"rows":2,"cols":2,"size":[2,2,0],"r":[90,0,0]}"#);
        assert!(d.p().iter().all(|p| p[1].abs() < 1e-5));
        assert!(near(d.p()[3], [1.0, 0.0, 1.0]));
        // rotation preserves winding handedness: xy -Z normal (Houdini) -> Rx(90) -> +Y.
        let n = normal(&d, 0);
        assert!(n[1] > 0.0 && n[0].abs() < 1e-5 && n[2].abs() < 1e-5);
    }
    #[test]
    fn triangle_surftypes_split_cells_by_closed_form() {
        // 3x3 cells row0/col0 = a=0 (b=3, cc=4, dd=1).
        let tri = ok(r#"{"surftype":3,"rows":3,"cols":3}"#);
        assert_eq!((prim_pts(&tri, 0), prim_pts(&tri, 1)), (vec![0, 4, 3], vec![0, 1, 4]));
 let rev = ok(r#"{"surftype":6,"rows":3,"cols":3}"#);
        assert_eq!((prim_pts(&rev, 0), prim_pts(&rev, 1)), (vec![0, 1, 3], vec![3, 1, 4]));
        let alt = ok(r#"{"surftype":5,"rows":3,"cols":3}"#);
        // cell (0,0) even = triangles; cell (0,1) odd = reverse (a=1,b=4,cc=5,dd=2).
        assert_eq!((prim_pts(&alt, 0), prim_pts(&alt, 1)), (vec![0, 4, 3], vec![0, 1, 4]));
 assert_eq!((prim_pts(&alt, 2), prim_pts(&alt, 3)), (vec![1, 2, 4], vec![4, 2, 5]));
        // every triangle of every flavour keeps the +Y normal on zx.
        for d in [&tri, &rev, &alt] {
            assert!((0..d.prims.len()).all(|i| normal(d, i)[1] > 0.0));
        }
    }
    #[test]
    fn rows_cols_rowcol_connectivity_follow_point_lattice() {
        // 3 rows x 4 cols: point id = row*4 + col.
        let r = ok(r#"{"surftype":0,"rows":3,"cols":4}"#);
        assert_eq!(prim_pts(&r, 1), vec![4, 5, 6, 7]);
        let c = ok(r#"{"surftype":1,"rows":3,"cols":4}"#);
        assert_eq!(prim_pts(&c, 2), vec![2, 6, 10]);
        let rc = ok(r#"{"surftype":2,"rows":3,"cols":4}"#);
        assert_eq!((rc.prim_count(), rc.vertex_count()), (3 + 4, 3 * 4 + 4 * 3));
        assert_eq!((prim_pts(&rc, 0), prim_pts(&rc, 3)), (vec![0, 1, 2, 3], vec![0, 4, 8]));
    }
    #[test]
    fn points_type_has_rows_times_cols_points_and_no_prims() {
        for s in 0..7 {
            let d = ok(&format!(r#"{{"type":4,"surftype":{s},"rows":5,"cols":3}}"#));
            assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (15, 0, 0));
        }
    }
    #[test]
    fn polysoup_has_poly_topology() {
        let soup = ok(r#"{"type":5,"rows":4,"cols":3}"#);
        let poly = ok(r#"{"type":0,"rows":4,"cols":3}"#);
        assert_eq!(*soup, *poly);
        assert_eq!(soup.prim_count(), 6);
    }
    #[test]
    fn mesh_is_one_prim_over_every_point_and_roundtrips() {
        let d = ok(r#"{"type":1,"rows":4,"cols":3,"surftype":0}"#);
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (12, 1, 12));
        assert!(
            matches!(&d.prims[0].kind, PrimKind::Unknown { ty, json } if ty == "Mesh" && json.contains("\"ucount\",3,\"vcount\",4") && json.contains("\"rows\""))
        );
        assert_eq!(prim_pts(&d, 0), (0..12).collect::<Vec<u32>>());
        let back = read_geo_json(&write_geo_json(&d)).expect("mesh roundtrips");
        assert_eq!((back.point_count, back.prim_count()), (12, 1));
        assert!(matches!(&back.prims[0].kind, PrimKind::Unknown { ty, .. } if ty == "Mesh"));
    }
    #[test]
    fn nurbs_and_bezier_are_unsupported_never_a_fallback() {
        for ty in [2, 3] {
            let (mut g, id) = load_scene(&scene(&format!(r#"{{"type":{ty}}}"#))).expect("loads");
            let e = g.cook(id, 0.0).expect_err("unsupported");
            assert!(e.to_string().contains("unsupported"), "{e}");
            assert!(e.to_string().contains(if ty == 2 { "nurbs" } else { "bezier" }));
        }
        // orders/interp are ignored for the implemented types, even at extremes.
        let a = ok(r#"{"orderu":11,"orderv":2,"interpu":false,"interpv":false}"#);
        assert_eq!(*a, *ok("{}"));
    }
    #[test]
    fn all_twelve_parms_are_declared() {
        let names: Vec<String> = spec().into_iter().map(|s| s.name).collect();
        let want =
            ["type", "surftype", "orient", "size", "t", "r", "rows", "cols", "orderu", "orderv", "interpu", "interpv"];
        assert_eq!(names, want);
    }
    #[test]
    fn params_by_token_override_defaults() {
        let d = ok(r#"{"rows":4,"cols":3}"#);
        assert_eq!((d.point_count, d.prim_count()), (12, 6));
        let d = ok(r#"{"size":[2,6,0]}"#);
        assert!(near(d.p()[0], [-1.0, 0.0, -3.0]) && near(d.p()[99], [1.0, 0.0, 3.0]));
        let d = ok(r#"{"size":[-2,2,0],"rows":2,"cols":2}"#);
        assert!(near(d.p()[0], [1.0, 0.0, -1.0]));
    }
    #[test]
    fn hostile_params_are_errors() {
        for bad in [
            r#"{"type":6}"#,
            r#"{"surftype":7}"#,
            r#"{"orient":3}"#,
            r#"{"rows":0}"#,
            r#"{"cols":-3}"#,
            r#"{"rows":-2147483648}"#,
        ] {
            let e = cook(bad).expect_err(bad);
            assert!(e.contains("bad parameter: grid."), "{bad}: {e}");
        }
        for huge in [
            r#"{"rows":100000,"cols":100000}"#,
            r#"{"rows":2147483647,"cols":2147483647}"#,
            r#"{"rows":1025,"cols":1025}"#,
        ] {
            let e = cook(huge).expect_err(huge);
            assert!(e.contains("point cap"), "{huge}: {e}");
        }
        // exactly at the cap still cooks.
        assert_eq!(ok(r#"{"rows":1024,"cols":1024,"surftype":0}"#).prim_count(), 1024);
        // 1 row / 1 col is the DS hard minimum: valid, degenerate, no division by zero.
        let d = ok(r#"{"rows":1,"cols":1}"#);
        assert_eq!((d.point_count, d.prim_count()), (1, 0));
        assert!(near(d.p()[0], [0.0, 0.0, 0.0]));
    }
    #[test]
    fn wrong_typed_params_are_errors() {
        assert!(cook(r#"{"rows":2.5}"#).is_err());
        assert!(cook(r#"{"size":[1,2]}"#).is_err());
        assert!(cook(r#"{"r":[0,0]}"#).is_err());
        assert!(cook(r#"{"interpu":3}"#).is_err());
        assert!(cook(r#"{"orderu":1.5}"#).is_err());
    }
    #[test]
    fn ignores_connected_inputs_gracefully() {
        let mut p = hexe_graph::ParamSet::new();
        for s in spec() {
            p = p.with(&s.name, s.default.clone());
        }
        let c = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        let out = new().cook(&c, &[Arc::new(Detail::default())]).expect("cooks");
        assert_eq!(out.point_count, 100);
    }
    // ---- analytical oracle fixtures: expected built by INDEPENDENT maths, never by the SOP ----
    /// `grid-default`: 10x10 lattice on ZX, size 10: x = -5 + 10*i/9 over cols, z = -5 + 10*j/9 over rows (Houdini row 0 at -Z), quads (a,a+1,a+11,a+10) = Houdini's clockwise cell.
    fn expected_default() -> Detail {
        let mut d = Detail::default();
        d.add_points(100);
        let coord = |i: usize| (-5.0 + 10.0 * (i as f64) / 9.0) as f32;
        let p = d.p_mut();
        for (k, slot) in p.iter_mut().enumerate() {
            *slot = [coord(k % 10), 0.0, coord(k / 10)];
        }
        for k in 0..81u32 {
            let (r, c) = (k / 9, k % 9);
            let a = r * 10 + c;
            d.add_poly(&[a, a + 1, a + 11, a + 10], true);
        }
        d
    }
    /// `grid-tri-3x3`: surftype triangles, 3x3, default size 10 on ZX. Points spelled out by hand.
    fn expected_tri_3x3() -> Detail {
        let mut d = Detail::default();
        d.add_points(9);
        let xs = [-5.0f32, 0.0, 5.0];
        let zs = [-5.0f32, 0.0, 5.0];
        let p = d.p_mut();
        for (k, slot) in p.iter_mut().enumerate() {
            *slot = [xs[k % 3], 0.0, zs[k / 3]];
        }
        // cell (r,c): a=3r+c; tris (a, a+4, a+3) and (a, a+1, a+4): +Y under Houdini's left-hand rule.
        for tri in [[0, 4, 3], [0, 1, 4], [1, 5, 4], [1, 2, 5], [3, 7, 6], [3, 4, 7], [4, 8, 7], [4, 5, 8]] {
            d.add_poly(&tri, true);
        }
        d
    }
    fn fixture_path(case: &str, file: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(case).join(file)
    }
    #[test]
    #[ignore = "writes fixtures/grid-*/expected.geo (run: cargo test -p hexe-sop -- --ignored write_fixture)"]
    fn write_fixture() {
        for (case, d) in [("grid-default", expected_default()), ("grid-tri-3x3", expected_tri_3x3())] {
            std::fs::write(fixture_path(case, "expected.geo"), write_geo_json(&d)).expect("write expected.geo");
        }
    }
    #[test]
    fn fixtures_match_sop_output() {
        for (case, expected, counts) in
            [("grid-default", expected_default(), (100, 81)), ("grid-tri-3x3", expected_tri_3x3(), (9, 8))]
        {
            let scene = std::fs::read_to_string(fixture_path(case, "scene.json")).expect("scene.json");
            let (mut g, id) = load_scene(&scene).expect("fixture scene loads");
            let out = g.cook(id, 0.0).expect("cooks");
            assert_eq!((out.point_count, out.prim_count()), counts, "{case}");
            let file = std::fs::read(fixture_path(case, "expected.geo")).expect("expected.geo");
            assert!(write_geo_json(&expected) == file, "{case}: committed expected.geo != independent maths");
            let got = read_geo(&file).expect("reads");
            assert_eq!(got.point_count, out.point_count);
            assert_eq!(got.prims.len(), out.prims.len());
            assert_eq!(got.vertex_point, out.vertex_point);
            assert!(got.p().iter().zip(out.p()).all(|(a, b)| near(*a, *b)), "{case}: P mismatch");
        }
    }
}
