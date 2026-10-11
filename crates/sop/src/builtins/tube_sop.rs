//! Tube SOP (token `tube`): generator (no inputs), polygon tube / cone / cylinder (Houdini 20.5 `tube`).
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-tube*`)
//! `rows` rings (>= 2) of `cols` points (>= 3), ring 0 on TOP: height `h_k = height/2 - height*k/(rows-1)`, radius
//! `(rad0 + (rad1 - rad0)*k/(rows-1)) * radscale` (`rad0` top, `rad1` bottom). Ring point `a_i = 2*pi*i/cols`, local
//! `(r cos a, h, -r sin a)` for `orient` y (default); x -> `(h, -r cos a, -r sin a)`; z -> `(r cos a, r sin a, h)`.
//! `P = t + R_xyz(r) * local` (no uniform scale parm). Prims by `surftype` (default quads, see `surfgen`; columns wrap, rows do not).
//! * `cap`: top cap `[c-1 .. 0]` then bottom cap `[last ring 0..c-1]` come BEFORE the side prims; `consolidatepts` off gives the
//!   caps their own points after the ring points (top cap ring reversed, bottom cap in ring order).
//!
//! DEVIATION: Houdini's default `type` is `prim` (one Tube primitive); hexe has no verified Tube-prim transform model, so
//! `prim` -> `Unsupported` and the hexe default is `poly`. mesh / nurbs / bezier / polysoup -> `Unsupported`. `vertexnormals`
//! not modelled. Houdini clamps a bottom radius of 0 to 1e-5 when capped (NOT modelled).
use super::meshutil::{bad, failed, float, int, menu, toggle, v3};
use super::surfgen::{SURF_MENU, SURF_QUADS, rot_xyz, surface_prims};
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::f64::consts::TAU;
use std::sync::Arc;

const OP: &str = "tube";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(1)).label("Primitive Type").menu(&[
            ("prim", "Primitive"),
            ("poly", "Polygon"),
            ("mesh", "Mesh"),
            ("nurbs", "NURBS"),
            ("bezier", "Bezier"),
            ("polysoup", "Polygon Soup"),
        ]),
        ParamSpec::new("surftype", ParamValue::Menu(SURF_QUADS)).label("Connectivity").menu(&SURF_MENU),
        ParamSpec::new("orient", ParamValue::Menu(1)).label("Orientation").menu(&[
            ("x", "X Axis"),
            ("y", "Y Axis"),
            ("z", "Z Axis"),
        ]),
        ParamSpec::new("cap", ParamValue::B(false)).label("End Caps"),
        ParamSpec::new("consolidatepts", ParamValue::B(true)).label("Consolidate Corner Points"),
        ParamSpec::new("t", ParamValue::V3([0.0; 3])).label("Center").range(-5.0, 5.0),
        ParamSpec::new("r", ParamValue::V3([0.0; 3])).label("Rotate").range(0.0, 360.0),
        ParamSpec::new("rad", ParamValue::V3([1.0, 1.0, 0.0])).label("Radius (top, bottom)").range(0.0, 5.0),
        ParamSpec::new("radscale", ParamValue::F(1.0)).label("Radius Scale").range(0.0, 10.0),
        ParamSpec::new("height", ParamValue::F(1.0)).label("Height").range(0.0, 10.0),
        ParamSpec::new("rows", ParamValue::I(2)).label("Rows").range(2.0, 50.0),
        ParamSpec::new("cols", ParamValue::I(12)).label("Columns").range(3.0, 50.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Tube { specs: spec() })
}
struct Tube {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Tube {
    fn type_name(&self) -> &'static str {
        "tube"
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
        let surf = menu(p, "surftype")?;
        let orient = menu(p, "orient")?;
        let (cap, consolidate) = (toggle(p, "cap")?, toggle(p, "consolidatepts")?);
        let (t, r, rad) = (v3(p, "t")?, v3(p, "r")?, v3(p, "rad")?);
        let (radscale, height) = (float(p, "radscale")?, float(p, "height")?);
        let (rows, cols) = (int(p, "rows")?, int(p, "cols")?);
        if ty != 1 {
            return Err(CookError::Unsupported("tube: only type `poly` is implemented".into()));
        }
        if surf > 6 || orient > 2 {
            return Err(bad("`surftype` / `orient`: menu index out of range"));
        }
        let (rows, cols) = match (u32::try_from(rows), u32::try_from(cols)) {
            (Ok(a), Ok(b)) if a >= 2 && b >= 3 => (a, b),
            _ => return Err(bad("`rows` must be >= 2 and `cols` >= 3")),
        };
        let sides = surface_prims(OP, rows, cols, surf, false)?;
        let place = |local: [f64; 3]| -> Result<[f32; 3], CookError> {
            let q = rot_xyz(r, local);
            let out = [q[0] + t[0], q[1] + t[1], q[2] + t[2]];
            if out.iter().all(|c| c.is_finite() && c.abs() < 1e30) {
                Ok(out.map(|c| c as f32))
            } else {
                Err(failed(OP, "non-finite or overflowing point"))
            }
        };
        // ring point (row k, column i)
        let ring = |k: u32, i: u32| -> [f64; 3] {
            let f = f64::from(k) / f64::from(rows - 1);
            let h = 0.5 * height - height * f;
            let rr = (rad[0] + (rad[1] - rad[0]) * f) * radscale;
            let a = TAU * f64::from(i) / f64::from(cols);
            match orient {
                0 => [h, -rr * a.cos(), -rr * a.sin()],
                2 => [rr * a.cos(), rr * a.sin(), h],
                _ => [rr * a.cos(), h, -rr * a.sin()],
            }
        };
        let extra = if cap && !consolidate { 2 * cols } else { 0 };
        let mut d = Detail::default();
        let ids = d.add_points(rows * cols + extra);
        let pos = d.p_mut();
        pos.clear();
        for k in 0..rows {
            for i in 0..cols {
                pos.push(place(ring(k, i))?);
            }
        }
        let base = ids.start;
        let (top, bottom): (Vec<u32>, Vec<u32>) = if cap && !consolidate {
            for i in (0..cols).rev() {
                pos.push(place(ring(0, i))?);
            }
            for i in 0..cols {
                pos.push(place(ring(rows - 1, i))?);
            }
            let c0 = base + rows * cols;
            ((c0..c0 + cols).collect(), (c0 + cols..c0 + 2 * cols).collect())
        } else {
            ((0..cols).rev().map(|i| base + i).collect(), (0..cols).map(|i| base + (rows - 1) * cols + i).collect())
        };
        if cap {
            d.add_poly(&top, true);
            d.add_poly(&bottom, true);
        }
        for pr in sides {
            d.add_poly(&pr.iter().map(|k| base + k).collect::<Vec<_>>(), true);
        }
        Ok(Arc::new(d))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, defaults, houdini_twin};
    use super::*;
    use hexe_graph::ParamSet;

    fn cooked(f: impl FnOnce(ParamSet) -> ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        cook(&*n, Detail::default(), &f(defaults(&*n)))
    }
    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn default_poly_tube_and_caps() {
        let d = cooked(|p| p.with("cols", ParamValue::I(4))).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (8, 4));
        assert!(
            near(d.p()[0], [1.0, 0.5, 0.0]) && near(d.p()[1], [0.0, 0.5, -1.0]) && near(d.p()[4], [1.0, -0.5, 0.0])
        );
        assert_eq!(&d.vertex_point[0..4], &[0, 1, 5, 4]);
        let c = cooked(|p| p.with("cols", ParamValue::I(4)).with("cap", ParamValue::B(true))).expect("cooks");
        assert_eq!(c.prim_count(), 6);
        assert_eq!(&c.vertex_point[0..8], &[3, 2, 1, 0, 4, 5, 6, 7]);
        let s = cooked(|p| {
            p.with("cols", ParamValue::I(4))
                .with("cap", ParamValue::B(true))
                .with("consolidatepts", ParamValue::B(false))
        })
        .expect("cooks");
        assert_eq!((s.point_count, s.prim_count()), (16, 6));
        assert_eq!(&s.vertex_point[0..8], &[8, 9, 10, 11, 12, 13, 14, 15]);
        assert!(near(s.p()[8], [0.0, 0.5, 1.0]) && near(s.p()[12], [1.0, -0.5, 0.0]));
    }

    #[test]
    fn rad_height_orient_and_rotate() {
        let d = cooked(|p| {
            p.with("cols", ParamValue::I(4))
                .with("rad", ParamValue::V3([2.0, 1.0, 0.0]))
                .with("height", ParamValue::F(3.0))
        })
        .expect("cooks");
        assert!(near(d.p()[0], [2.0, 1.5, 0.0]) && near(d.p()[4], [1.0, -1.5, 0.0]));
        let x = cooked(|p| {
            p.with("rows", ParamValue::I(3))
                .with("cols", ParamValue::I(3))
                .with("orient", ParamValue::Menu(0))
                .with("radscale", ParamValue::F(2.0))
        })
        .expect("cooks");
        assert!(
            near(x.p()[0], [0.5, -2.0, 0.0])
                && near(x.p()[1], [0.5, 1.0, -1.732_050_8])
                && near(x.p()[3], [0.0, -2.0, 0.0])
        );
        let z = cooked(|p| {
            p.with("rows", ParamValue::I(2))
                .with("cols", ParamValue::I(3))
                .with("orient", ParamValue::Menu(2))
                .with("t", ParamValue::V3([1.0, 2.0, 3.0]))
                .with("r", ParamValue::V3([0.0, 0.0, 90.0]))
        })
        .expect("cooks");
        assert!(near(z.p()[0], [1.0, 3.0, 3.5]) && near(z.p()[1], [0.133_975, 1.5, 3.5]));
    }

    #[test]
    fn surftypes_and_hostile_params() {
        let n = |s: u32| {
            cooked(|p| p.with("cols", ParamValue::I(4)).with("surftype", ParamValue::Menu(s)))
                .expect("cooks")
                .prim_count()
        };
        assert_eq!([n(0), n(1), n(2), n(3), n(4), n(5), n(6)], [2, 4, 6, 8, 4, 8, 8]);
        assert!(matches!(cooked(|p| p.with("type", ParamValue::Menu(0))), Err(CookError::Unsupported(_))));
        assert!(cooked(|p| p.with("rows", ParamValue::I(1))).is_err());
        assert!(cooked(|p| p.with("cols", ParamValue::I(2))).is_err());
        assert!(cooked(|p| p.with("rows", ParamValue::I(100_000)).with("cols", ParamValue::I(100_000))).is_err());
        assert!(cooked(|p| p.with("height", ParamValue::F(f32::INFINITY))).is_err());
    }

    macro_rules! twin {
        ($n:literal) => {
            houdini_twin(
                include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
            )
        };
    }

    /// Hexe twin == real Houdini 20.5 `.geo` (P in index order, prim cycles) for a cone, a capped z-tube and capped triangles.
    #[test]
    fn tube_matches_houdini_fixtures() {
        assert_eq!(twin!("sop-tube"), Ok(()));
        assert_eq!(twin!("sop-tube-xform"), Ok(()));
        assert_eq!(twin!("sop-tube-cap"), Ok(()));
    }
}
