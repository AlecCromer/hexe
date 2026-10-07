//! Grid SOP: planar `rows x cols` POINT lattice. Generator (no inputs).
//! Scope (wave 2B): `type`=poly; `surftype` rows/cols/triangles/quads; `orient` xy/yz/zx; `size`, `t`, `rows`, `cols`.
//! Out of scope (params NOT declared): `r` (rotate), `orderu/orderv/interpu/interpv` (NURBS/Bezier only).
//! Other `type` menu items and other `surftype` items (rowcol, alttriangles, revtriangles) -> `BadParam`.
//! Mapping (UNVERIFIED vs Houdini, hython blocked): `cols` points span the FIRST in-plane axis (xy: X, yz: Y, zx: X) over
//! `size[0]`; `rows` points span the SECOND (xy: Y, yz: Z, zx: Z) over `size[1]`; ranges `[-s/2, +s/2]` about `t`,
//! ascending. Point id = `row*cols + col`. Winding: normal +Z (xy), +X (yz), +Y (zx, per help "ZX = normal +Y").
//! No `N` attribute (existence [?]). `size` is declared `V3` (no V2 variant): `size[2]` is ignored.
use crate::SopNode;
use hexe_geo::{Detail, PointId};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

/// Allocation cap: `rows * cols` above this is `CookError::Failed` (hostile input, AGENTS.md §4).
const MAX_POINTS: u64 = 1 << 20;

/// `type` menu index of `poly` (the only one implemented).
const TYPE_POLY: u32 = 0;
/// `surftype` menu indices (table order: rows cols rowcol triangles quads alttriangles revtriangles).
const SURF_ROWS: u32 = 0;
const SURF_COLS: u32 = 1;
const SURF_TRIANGLES: u32 = 3;
const SURF_QUADS: u32 = 4;
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
        ParamSpec::new("rows", ParamValue::I(10)).label("Rows").range(1.0, 50.0),
        ParamSpec::new("cols", ParamValue::I(10)).label("Columns").range(1.0, 50.0),
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
        let rows_i = p.int("rows").ok_or_else(|| bad("rows", "missing or not an integer"))?;
        let cols_i = p.int("cols").ok_or_else(|| bad("cols", "missing or not an integer"))?;
        if ty != TYPE_POLY {
            return Err(bad("type", "only `poly` (0) is implemented"));
        }
        if !matches!(surf, SURF_ROWS | SURF_COLS | SURF_TRIANGLES | SURF_QUADS) {
            return Err(bad("surftype", "only rows(0) cols(1) triangles(3) quads(4) are implemented"));
        }
        if orient > ORIENT_ZX {
            return Err(bad("orient", "menu index out of range 0..=2"));
        }
        if rows_i < 2 || cols_i < 2 {
            return Err(bad("rows/cols", "need at least 2 point rows and 2 point columns"));
        }
        let (rows, cols) = (rows_i as u32 as usize, cols_i as u32 as usize);
        if (rows as u64) * (cols as u64) > MAX_POINTS {
            return Err(CookError::Failed(format!("grid: {rows}x{cols} points exceeds the {MAX_POINTS} point cap")));
        }

        let mut d = Detail::default();
        let ids = d.add_points(to_u32(rows * cols)?);
        let base: PointId = ids.start;
        let (sx, sy) = (f64::from(size[0]), f64::from(size[1]));
        let c = [f64::from(t[0]), f64::from(t[1]), f64::from(t[2])];
        let pos = d.p_mut();
        pos.clear();
        for r in 0..rows {
            let v = -sy / 2.0 + sy * (r as f64) / ((rows - 1) as f64);
            for col in 0..cols {
                let u = -sx / 2.0 + sx * (col as f64) / ((cols - 1) as f64);
                let q = match orient {
                    ORIENT_XY => [c[0] + u, c[1] + v, c[2]],
                    ORIENT_YZ => [c[0], c[1] + u, c[2] + v],
                    _ => [c[0] + u, c[1], c[2] + v],
                };
                pos.push([q[0] as f32, q[1] as f32, q[2] as f32]);
            }
        }

        let id = |r: usize, col: usize| -> Result<PointId, CookError> { Ok(base + to_u32(r * cols + col)?) };
        // xy/yz: (col-axis x row-axis) is already +normal; zx would be -Y, so its cells wind the other way.
        let flip = orient == ORIENT_ZX;
        match surf {
            SURF_ROWS => {
                for r in 0..rows {
                    let line = (0..cols).map(|col| id(r, col)).collect::<Result<Vec<_>, _>>()?;
                    d.add_poly(&line, false);
                }
            }
            SURF_COLS => {
                for col in 0..cols {
                    let line = (0..rows).map(|r| id(r, col)).collect::<Result<Vec<_>, _>>()?;
                    d.add_poly(&line, false);
                }
            }
            _ => {
                for r in 0..rows - 1 {
                    for col in 0..cols - 1 {
                        let (a, b, cc, dd) = (id(r, col)?, id(r, col + 1)?, id(r + 1, col + 1)?, id(r + 1, col)?);
                        match (surf, flip) {
                            (SURF_QUADS, false) => {
                                d.add_poly(&[a, b, cc, dd], true);
                            }
                            (SURF_QUADS, true) => {
                                d.add_poly(&[a, dd, cc, b], true);
                            }
                            (_, false) => {
                                d.add_poly(&[a, b, cc], true);
                                d.add_poly(&[a, cc, dd], true);
                            }
                            (_, true) => {
                                d.add_poly(&[a, cc, b], true);
                                d.add_poly(&[a, dd, cc], true);
                            }
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
    use hexe_geo::{AttrClass, PrimKind, read_geo, write_geo_json};

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

    #[test]
    fn default_is_100_points_81_quads_on_zx_plane() {
        let d = ok("{}");
        assert_eq!((d.point_count, d.vertex_count(), d.prim_count()), (100, 324, 81));
        assert!(d.prims.iter().all(|p| matches!(p.kind, PrimKind::Poly { closed: true }) && p.verts.len() == 4));
        assert!(d.p().iter().all(|p| p[1] == 0.0));
        assert!(near(d.p()[0], [-5.0, 0.0, -5.0]));
        assert!(near(d.p()[99], [5.0, 0.0, 5.0]));
        assert!(near(d.p()[9], [5.0, 0.0, -5.0]));
        assert!(d.attr(AttrClass::Point, "P").is_some() && d.attrs.len() == 1);
    }

    #[test]
    fn closed_form_counts_for_every_surftype() {
        for (rows, cols) in [(2, 2), (3, 5), (7, 4)] {
            let p = |s: u32| format!(r#"{{"surftype":{s},"rows":{rows},"cols":{cols}}}"#);
            let quads = ok(&p(4));
            assert_eq!(quads.point_count as usize, rows * cols);
            assert_eq!(quads.prim_count() as usize, (rows - 1) * (cols - 1));
            let tris = ok(&p(3));
            assert_eq!(tris.point_count as usize, rows * cols);
            assert_eq!(tris.prim_count() as usize, 2 * (rows - 1) * (cols - 1));
            assert!(tris.prims.iter().all(|p| p.verts.len() == 3));
            let r = ok(&p(0));
            assert_eq!((r.prim_count() as usize, r.vertex_count() as usize), (rows, rows * cols));
            assert!(r.prims.iter().all(|p| matches!(p.kind, PrimKind::Polyline | PrimKind::Poly { closed: false })));
            assert!(r.prims.iter().all(|p| p.verts.len() == cols));
            let c = ok(&p(1));
            assert_eq!((c.prim_count() as usize, c.vertex_count() as usize), (cols, rows * cols));
            assert!(c.prims.iter().all(|p| p.verts.len() == rows));
        }
    }

    #[test]
    fn rows_and_cols_connectivity_follow_point_lattice() {
        // 3 rows x 4 cols: point id = row*4 + col.
        let r = ok(r#"{"surftype":0,"rows":3,"cols":4}"#);
        assert_eq!(prim_pts(&r, 1), vec![4, 5, 6, 7]);
        let c = ok(r#"{"surftype":1,"rows":3,"cols":4}"#);
        assert_eq!(prim_pts(&c, 2), vec![2, 6, 10]);
    }

    #[test]
    fn orient_size_and_center_map_points_by_closed_form() {
        // 2x3 lattice, size (4,2), center (10,20,30): u in {-2,0,2} over cols, v in {-1,1} over rows.
        let base = |o: u32| ok(&format!(r#"{{"orient":{o},"rows":2,"cols":3,"size":[4,2,0],"t":[10,20,30]}}"#));
        let xy = base(0);
        assert!(near(xy.p()[0], [8.0, 19.0, 30.0]) && near(xy.p()[2], [12.0, 19.0, 30.0]) && near(xy.p()[3], [8.0, 21.0, 30.0]));
        let yz = base(1);
        assert!(near(yz.p()[0], [10.0, 18.0, 29.0]) && near(yz.p()[2], [10.0, 22.0, 29.0]) && near(yz.p()[3], [10.0, 18.0, 31.0]));
        let zx = base(2);
        assert!(near(zx.p()[0], [8.0, 20.0, 29.0]) && near(zx.p()[2], [12.0, 20.0, 29.0]) && near(zx.p()[3], [8.0, 20.0, 31.0]));
        assert!(near(zx.p()[5], [12.0, 20.0, 31.0]));
    }

    #[test]
    fn quad_normals_follow_the_orient_plane_normal() {
        let nrm = |d: &Detail| {
            let v = prim_pts(d, 0);
            let q: Vec<[f32; 3]> = v.iter().map(|&i| d.p()[i as usize]).collect();
            let (a, b) = ([q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]], [q[3][0] - q[0][0], q[3][1] - q[0][1], q[3][2] - q[0][2]]);
            [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
        };
        for (o, axis) in [(0, 2), (1, 0), (2, 1)] {
            let n = nrm(&ok(&format!(r#"{{"orient":{o},"rows":2,"cols":2}}"#)));
            assert!(n[axis] > 0.0, "orient {o}: normal {n:?}");
            assert!(n.iter().enumerate().all(|(i, c)| i == axis || *c == 0.0));
        }
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
    fn unsupported_menu_items_are_bad_param() {
        for bad in [r#"{"type":1}"#, r#"{"type":5}"#, r#"{"surftype":2}"#, r#"{"surftype":5}"#, r#"{"surftype":6}"#, r#"{"surftype":7}"#, r#"{"orient":3}"#] {
            let e = cook(bad).expect_err(bad);
            assert!(e.contains("bad parameter: grid."), "{bad}: {e}");
        }
    }

    #[test]
    fn hostile_rows_cols_are_errors() {
        for bad in [r#"{"rows":1}"#, r#"{"cols":1}"#, r#"{"rows":0}"#, r#"{"cols":-3}"#, r#"{"rows":-2147483648}"#] {
            let e = cook(bad).expect_err(bad);
            assert!(e.contains("bad parameter: grid.rows/cols"), "{bad}: {e}");
        }
        for huge in [r#"{"rows":100000,"cols":100000}"#, r#"{"rows":2147483647,"cols":2147483647}"#, r#"{"rows":1025,"cols":1025}"#] {
            let e = cook(huge).expect_err(huge);
            assert!(e.contains("point cap"), "{huge}: {e}");
        }
        // exactly at the cap still cooks.
        assert_eq!(ok(r#"{"rows":1024,"cols":1024,"surftype":0}"#).prim_count(), 1024);
    }

    #[test]
    fn wrong_typed_or_unknown_params_are_errors() {
        assert!(cook(r#"{"rows":2.5}"#).is_err());
        assert!(cook(r#"{"size":[1,2]}"#).is_err());
        assert!(cook(r#"{"r":[0,0,0]}"#).is_err());
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

    /// `grid-default`: 10x10 lattice on ZX, size 10: x = -5 + 10*i/9 over cols, z likewise over rows, quads wound for +Y.
    fn expected_default() -> Detail {
        let mut d = Detail::default();
        d.add_points(100);
        let coord = |i: usize| (-5.0 + 10.0 * (i as f64) / 9.0) as f32;
        let p = d.p_mut();
        for k in 0..100usize {
            p[k] = [coord(k % 10), 0.0, coord(k / 10)];
        }
        for k in 0..81u32 {
            let (r, c) = (k / 9, k % 9);
            let a = r * 10 + c;
            d.add_poly(&[a, a + 10, a + 11, a + 1], true);
        }
        d
    }

    /// `grid-tri-3x3`: surftype triangles, 3x3, default size 10 on ZX. Points spelled out by hand.
    fn expected_tri_3x3() -> Detail {
        let mut d = Detail::default();
        d.add_points(9);
        let xs = [-5.0f32, 0.0, 5.0];
        let p = d.p_mut();
        for (k, slot) in p.iter_mut().enumerate() {
            *slot = [xs[k % 3], 0.0, xs[k / 3]];
        }
        // cell (r,c): a=3r+c; tris (a, a+4, a+1) and (a, a+3, a+4) for +Y.
        for tri in [[0, 4, 1], [0, 3, 4], [1, 5, 2], [1, 4, 5], [3, 7, 4], [3, 6, 7], [4, 8, 5], [4, 7, 8]] {
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
        for (case, expected, counts) in [("grid-default", expected_default(), (100, 81)), ("grid-tri-3x3", expected_tri_3x3(), (9, 8))] {
            let scene = std::fs::read_to_string(fixture_path(case, "scene.json")).expect("scene.json");
            let (mut g, id) = load_scene(&scene).expect("fixture scene loads");
            let out = g.cook(id, 0.0).expect("cooks");
            assert_eq!((out.point_count, out.prim_count()), counts, "{case}");
            let file = std::fs::read(fixture_path(case, "expected.geo")).expect("expected.geo");
            assert_eq!(write_geo_json(&expected), file, "{case}: committed expected.geo != independent maths");
            let got = read_geo(&file).expect("reads");
            assert_eq!(got.point_count, out.point_count);
            assert_eq!(got.prims.len(), out.prims.len());
            assert_eq!(got.vertex_point, out.vertex_point);
            assert!(got.p().iter().zip(out.p()).all(|(a, b)| near(*a, *b)), "{case}: P mismatch");
        }
    }
}
