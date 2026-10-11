//! Torus SOP (token `torus`): generator (no inputs), polygon torus (Houdini 20.5 `torus`).
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-torus*`)
//! `rows` = points around the tube (minor circle), `cols` = points around the ring (major circle); point `(j, i) = j*cols + i`.
//! Minor angle `b_j = -2*pi*j/rows`, major angle `a_i = 2*pi*i/cols`, `rad = (major, minor)`, `Rc = rad0 + rad1*cos(b_j)`,
//! `m = rad1*sin(b_j)`. Local point by `orient`: y (default) `(Rc cos a, m, -Rc sin a)`; x `(m, Rc cos a, Rc sin a)`;
//! z `(Rc cos a, Rc sin a, m)`. `P = t + R_xyz(r) * (scale * local)`. Prims by `surftype` (default quads, see `surfgen`); both
//! directions wrap. Only `P` exists (no normals / uv).
//!
//! `type` poly only: mesh / nurbs / bezier / polysoup -> `Unsupported` (hexe has no Mesh prim). `rows`/`cols` < 3 -> `BadParam`.
use super::meshutil::{bad, failed, float, int, menu, v3};
use super::surfgen::{SURF_MENU, SURF_QUADS, rot_xyz, surface_prims};
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::f64::consts::TAU;
use std::sync::Arc;

const OP: &str = "torus";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(0)).label("Primitive Type").menu(&[
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
        ParamSpec::new("rad", ParamValue::V3([1.0, 0.5, 0.0])).label("Radius (major, minor)").range(0.0, 5.0),
        ParamSpec::new("t", ParamValue::V3([0.0; 3])).label("Center").range(-5.0, 5.0),
        ParamSpec::new("r", ParamValue::V3([0.0; 3])).label("Rotate").range(0.0, 360.0),
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Uniform Scale").range(0.0, 10.0),
        ParamSpec::new("rows", ParamValue::I(12)).label("Rows").range(3.0, 50.0),
        ParamSpec::new("cols", ParamValue::I(24)).label("Columns").range(3.0, 50.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Torus { specs: spec() })
}
struct Torus {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Torus {
    fn type_name(&self) -> &'static str {
        "torus"
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
        let (rad, t, r) = (v3(p, "rad")?, v3(p, "t")?, v3(p, "r")?);
        let scale = float(p, "scale")?;
        let (rows, cols) = (int(p, "rows")?, int(p, "cols")?);
        if ty != 0 {
            return Err(CookError::Unsupported("torus: only type `poly` is implemented".into()));
        }
        if surf > 6 || orient > 2 {
            return Err(bad("`surftype` / `orient`: menu index out of range"));
        }
        let (rows, cols) = match (u32::try_from(rows), u32::try_from(cols)) {
            (Ok(a), Ok(b)) if a >= 3 && b >= 3 => (a, b),
            _ => return Err(bad("`rows` / `cols` must be >= 3")),
        };
        let prims = surface_prims(OP, rows, cols, surf, true)?;
        let mut d = Detail::default();
        let ids = d.add_points(rows * cols);
        let pos = d.p_mut();
        pos.clear();
        for j in 0..rows {
            let b = -TAU * f64::from(j) / f64::from(rows);
            let rc = rad[0] + rad[1] * b.cos();
            let m = rad[1] * b.sin();
            for i in 0..cols {
                let a = TAU * f64::from(i) / f64::from(cols);
                let local = match orient {
                    0 => [m, rc * a.cos(), rc * a.sin()],
                    2 => [rc * a.cos(), rc * a.sin(), m],
                    _ => [rc * a.cos(), m, -rc * a.sin()],
                };
                let q = rot_xyz(r, local.map(|c| c * scale));
                let out = [q[0] + t[0], q[1] + t[1], q[2] + t[2]];
                if !out.iter().all(|c| c.is_finite() && c.abs() < 1e30) {
                    return Err(failed(OP, "non-finite or overflowing point"));
                }
                pos.push(out.map(|c| c as f32));
            }
        }
        for pr in prims {
            d.add_poly(&pr.iter().map(|k| ids.start + k).collect::<Vec<_>>(), true);
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
    fn default_counts_and_first_ring() {
        let d = cooked(|p| p).expect("cooks");
        assert_eq!((d.point_count, d.prim_count(), d.vertex_count()), (288, 288, 1152));
        assert!(
            near(d.p()[0], [1.5, 0.0, 0.0])
                && near(d.p()[1], [1.5 * (TAU / 24.0).cos() as f32, 0.0, -1.5 * (TAU / 24.0).sin() as f32])
        );
    }

    #[test]
    fn small_torus_orient_transform_and_surftypes() {
        let small = |p: ParamSet| p.with("rows", ParamValue::I(3)).with("cols", ParamValue::I(4));
        let d = cooked(small).expect("cooks");
        assert_eq!(&d.vertex_point[0..4], &[0, 1, 5, 4]);
        assert!(near(d.p()[4], [0.75, -0.433_012_7, 0.0]));
        let x = cooked(|p| small(p).with("orient", ParamValue::Menu(0))).expect("cooks");
        assert!(near(x.p()[1], [0.0, 0.0, 1.5]));
        let z = cooked(|p| small(p).with("orient", ParamValue::Menu(2))).expect("cooks");
        assert!(near(z.p()[1], [0.0, 1.5, 0.0]));
        let xf = cooked(|p| {
            small(p)
                .with("rad", ParamValue::V3([2.0, 0.5, 0.0]))
                .with("scale", ParamValue::F(2.0))
                .with("t", ParamValue::V3([1.0, 2.0, 3.0]))
        })
        .expect("cooks");
        assert!(near(xf.p()[0], [6.0, 2.0, 3.0]) && near(xf.p()[4], [4.5, 1.133_975, 3.0]));
        let rz = cooked(|p| small(p).with("r", ParamValue::V3([0.0, 0.0, 90.0]))).expect("cooks");
        assert!(near(rz.p()[0], [0.0, 1.5, 0.0]));
        for (s, n) in [(0, 3), (1, 4), (2, 7), (3, 24), (4, 12), (5, 24), (6, 24)] {
            let d = cooked(|p| small(p).with("surftype", ParamValue::Menu(s))).expect("cooks");
            assert_eq!(d.prim_count(), n, "surftype {s}");
        }
        let alt = cooked(|p| small(p).with("surftype", ParamValue::Menu(5))).expect("cooks");
        assert_eq!((alt.vertex_point[3..6].to_vec(), alt.vertex_point[6..9].to_vec()), (vec![0, 5, 4], vec![1, 2, 5]));
    }

    #[test]
    fn hostile_params() {
        assert!(matches!(cooked(|p| p.with("type", ParamValue::Menu(2))), Err(CookError::Unsupported(_))));
        assert!(cooked(|p| p.with("rows", ParamValue::I(2))).is_err());
        assert!(cooked(|p| p.with("cols", ParamValue::I(-4))).is_err());
        assert!(cooked(|p| p.with("rows", ParamValue::I(100_000)).with("cols", ParamValue::I(100_000))).is_err());
        assert!(cooked(|p| p.with("scale", ParamValue::F(f32::NAN))).is_err());
        assert!(
            cooked(|p| p.with("rad", ParamValue::V3([f32::MAX, f32::MAX, 0.0])).with("scale", ParamValue::F(f32::MAX)))
                .is_err()
        );
    }

    macro_rules! twin {
        ($n:literal) => {
            houdini_twin(
                include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
            )
        };
    }

    /// Hexe twin == real Houdini 20.5 `.geo` (P in index order, prim cycles) for the default torus, an x-oriented transformed one and
    /// the alternating-triangle connectivity.
    #[test]
    fn torus_matches_houdini_fixtures() {
        assert_eq!(twin!("sop-torus"), Ok(()));
        assert_eq!(twin!("sop-torus-xform"), Ok(()));
        assert_eq!(twin!("sop-torus-alt"), Ok(()));
    }
}
