//! Platonic Solids SOP (token `platonic`): generator (no inputs), the five regular solids (Houdini 20.5 `platonic`).
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-platonic*`)
//! `type` 0 tetrahedron, 1 cube, 2 octahedron, 3 icosahedron, 4 dodecahedron: point / prim tables below are Houdini's own
//! (hard-coded f32 values at `radius` 1, incl. its small rounding noise, e.g. tetra y = -0.3333297; the cube has edge 1 and the
//! dodecahedron circumradius 0.99332). `orient` y = tables as-is; x: `(x,y,z) -> (y,-x,z)`; z: `(x,y,z) -> (x,-z,y)`.
//! `P = t + R_xyz(r) * (radius * local)`. Prims are closed polygons in the table's winding. No attributes.
//!
//! Not modelled: `type` 5 (soccer ball: 60 pts / 32 prims + prim `Cd`) -> `Unsupported`; Houdini clamps a negative type to 0 (hexe
//! menu is range-checked instead).
use super::meshutil::{bad, failed, float, menu, v3};
use super::surfgen::rot_xyz;
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "platonic";

/// tetra: 4 points, 4 prims (hython 20.5, radius 1).
const TETRA_P: [[f32; 3]; 4] = [
    [0.0, 1.0, 0.0],
    [-0.9428103, -0.3333297, 0.0],
    [0.47140515, -0.3333297, 0.8164976],
    [0.47140515, -0.3333297, -0.8164976],
];
const TETRA_F: &[&[u32]] = &[&[0, 3, 2], &[0, 1, 3], &[0, 2, 1], &[1, 2, 3]];
/// cube: 8 points, 6 prims (hython 20.5, radius 1).
const CUBE_P: [[f32; 3]; 8] = [
    [-0.5, -0.5, -0.5],
    [0.5, -0.5, -0.5],
    [0.5, -0.5, 0.5],
    [-0.5, -0.5, 0.5],
    [-0.5, 0.5, -0.5],
    [0.5, 0.5, -0.5],
    [0.5, 0.5, 0.5],
    [-0.5, 0.5, 0.5],
];
const CUBE_F: &[&[u32]] = &[&[1, 5, 4, 0], &[2, 6, 5, 1], &[3, 7, 6, 2], &[0, 4, 7, 3], &[2, 1, 0, 3], &[5, 6, 7, 4]];
/// octa: 6 points, 8 prims (hython 20.5, radius 1).
const OCTA_P: [[f32; 3]; 6] =
    [[0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 1.0, 0.0]];
const OCTA_F: &[&[u32]] =
    &[&[0, 3, 4], &[0, 4, 1], &[1, 4, 2], &[2, 4, 3], &[0, 5, 3], &[0, 1, 5], &[1, 2, 5], &[2, 3, 5]];
/// icosa: 12 points, 20 prims (hython 20.5, radius 1).
const ICOSA_P: [[f32; 3]; 12] = [
    [0.0, 0.0, -1.0],
    [0.0, 0.89442736, -0.44721314],
    [0.85065085, 0.27639368, -0.4472132],
    [0.5257312, -0.72360694, -0.44721326],
    [-0.5257312, -0.72360694, -0.44721326],
    [-0.85065085, 0.27639368, -0.4472132],
    [-0.5257312, 0.72360694, 0.44721326],
    [-0.85065085, -0.27639368, 0.4472132],
    [0.0, -0.89442736, 0.44721314],
    [0.85065085, -0.27639368, 0.4472132],
    [0.5257312, 0.72360694, 0.44721326],
    [0.0, 0.0, 1.0],
];
const ICOSA_F: &[&[u32]] = &[
    &[2, 1, 0],
    &[3, 2, 0],
    &[4, 3, 0],
    &[5, 4, 0],
    &[1, 5, 0],
    &[6, 5, 1],
    &[7, 4, 5],
    &[8, 3, 4],
    &[9, 2, 3],
    &[10, 1, 2],
    &[10, 6, 1],
    &[6, 7, 5],
    &[7, 8, 4],
    &[8, 9, 3],
    &[9, 10, 2],
    &[6, 10, 11],
    &[10, 9, 11],
    &[9, 8, 11],
    &[8, 7, 11],
    &[7, 6, 11],
];
/// dodeca: 20 points, 12 prims (hython 20.5, radius 1).
const DODECA_P: [[f32; 3]; 20] = [
    [0.35443786, 0.4878421, -0.7893443],
    [0.5734926, -0.18633887, -0.7893444],
    [0.0, -0.6030058, -0.78934443],
    [-0.5734926, -0.18633887, -0.7893444],
    [-0.35443786, 0.4878421, -0.7893443],
    [-0.5734926, 0.789345, -0.1863388],
    [-0.9279304, -0.3015029, -0.18633887],
    [0.0, -0.9756839, -0.1863389],
    [0.9279304, -0.3015029, -0.18633887],
    [0.5734926, 0.789345, -0.1863388],
    [0.0, 0.97568387, 0.1863389],
    [-0.9279304, 0.3015029, 0.18633887],
    [-0.5734926, -0.789345, 0.1863388],
    [0.5734926, -0.789345, 0.1863388],
    [0.9279304, 0.3015029, 0.18633887],
    [0.0, 0.6030058, 0.78934443],
    [0.5734926, 0.18633887, 0.7893444],
    [0.35443786, -0.4878421, 0.7893443],
    [-0.35443786, -0.4878421, 0.7893443],
    [-0.5734926, 0.18633887, 0.7893444],
];
const DODECA_F: &[&[u32]] = &[
    &[0, 4, 3, 2, 1],
    &[0, 9, 10, 5, 4],
    &[0, 1, 8, 14, 9],
    &[1, 2, 7, 13, 8],
    &[2, 3, 6, 12, 7],
    &[3, 4, 5, 11, 6],
    &[5, 10, 15, 19, 11],
    &[6, 11, 19, 18, 12],
    &[7, 12, 18, 17, 13],
    &[8, 13, 17, 16, 14],
    &[9, 14, 16, 15, 10],
    &[15, 16, 17, 18, 19],
];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("type", ParamValue::Menu(0)).label("Solid Type").menu(&[
            ("tetrahedron", "Tetrahedron"),
            ("cube", "Cube"),
            ("octahedron", "Octahedron"),
            ("icosahedron", "Icosahedron"),
            ("dodecahedron", "Dodecahedron"),
            ("soccerball", "Soccer Ball"),
        ]),
        ParamSpec::new("orient", ParamValue::Menu(1)).label("Orientation").menu(&[
            ("x", "X Axis"),
            ("y", "Y Axis"),
            ("z", "Z Axis"),
        ]),
        ParamSpec::new("radius", ParamValue::F(1.0)).label("Radius").range(0.0, 10.0),
        ParamSpec::new("t", ParamValue::V3([0.0; 3])).label("Center").range(-5.0, 5.0),
        ParamSpec::new("r", ParamValue::V3([0.0; 3])).label("Rotate").range(0.0, 360.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Platonic { specs: spec() })
}
struct Platonic {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Platonic {
    fn type_name(&self) -> &'static str {
        "platonic"
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
        let (t, r) = (v3(p, "t")?, v3(p, "r")?);
        let radius = float(p, "radius")?;
        if orient > 2 {
            return Err(bad("`orient`: menu index out of range 0..=2"));
        }
        let (pts, faces): (&[[f32; 3]], &[&[u32]]) = match ty {
            0 => (&TETRA_P, TETRA_F),
            1 => (&CUBE_P, CUBE_F),
            2 => (&OCTA_P, OCTA_F),
            3 => (&ICOSA_P, ICOSA_F),
            4 => (&DODECA_P, DODECA_F),
            5 => return Err(CookError::Unsupported("platonic: soccer ball is not implemented".into())),
            _ => return Err(bad("`type`: menu index out of range 0..=5")),
        };
        let mut d = Detail::default();
        let ids = d.add_points(pts.len() as u32);
        let pos = d.p_mut();
        pos.clear();
        for q in pts {
            let [x, y, z] = q.map(f64::from);
            let local = match orient {
                0 => [y, -x, z],
                2 => [x, -z, y],
                _ => [x, y, z],
            };
            let o = rot_xyz(r, local.map(|c| c * radius));
            let out = [o[0] + t[0], o[1] + t[1], o[2] + t[2]];
            if !out.iter().all(|c| c.is_finite() && c.abs() < 1e30) {
                return Err(failed(OP, "non-finite or overflowing point"));
            }
            pos.push(out.map(|c| c as f32));
        }
        for f in faces {
            d.add_poly(&f.iter().map(|k| ids.start + k).collect::<Vec<_>>(), true);
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
    fn counts_of_the_five_solids() {
        let c = |t: u32| {
            let d = cooked(|p| p.with("type", ParamValue::Menu(t))).expect("cooks");
            (d.point_count, d.prim_count(), d.vertex_count())
        };
        assert_eq!([c(0), c(1), c(2), c(3), c(4)], [(4, 4, 12), (8, 6, 24), (6, 8, 24), (12, 20, 60), (20, 12, 60)]);
    }

    #[test]
    fn radius_center_orient_and_rotate() {
        let cube = |f: fn(ParamSet) -> ParamSet| cooked(|p| f(p.with("type", ParamValue::Menu(1)))).expect("cooks");
        let d = cube(|p| p.with("radius", ParamValue::F(2.0)).with("t", ParamValue::V3([1.0, 2.0, 3.0])));
        assert!(near(d.p()[0], [0.0, 1.0, 2.0]) && near(d.p()[6], [2.0, 3.0, 4.0]));
        assert_eq!(&d.vertex_point[0..4], &[1, 5, 4, 0]);
        let x = cooked(|p| p.with("radius", ParamValue::F(2.0)).with("orient", ParamValue::Menu(0))).expect("cooks");
        assert!(near(x.p()[0], [2.0, 0.0, 0.0]) && near(x.p()[1], [-0.666_66, 1.885_62, 0.0]));
        let z = cooked(|p| p.with("radius", ParamValue::F(2.0)).with("orient", ParamValue::Menu(2))).expect("cooks");
        assert!(near(z.p()[0], [0.0, 0.0, 2.0]) && near(z.p()[2], [0.942_81, -1.633, -0.666_66]));
        let r = cooked(|p| {
            p.with("type", ParamValue::Menu(3))
                .with("radius", ParamValue::F(2.0))
                .with("r", ParamValue::V3([0.0, 0.0, 90.0]))
        })
        .expect("cooks");
        assert!(near(r.p()[1], [-1.788_85, 0.0, -0.894_43]));
    }

    #[test]
    fn hostile_params() {
        assert!(matches!(cooked(|p| p.with("type", ParamValue::Menu(5))), Err(CookError::Unsupported(_))));
        assert!(cooked(|p| p.with("type", ParamValue::Menu(9))).is_err());
        assert!(cooked(|p| p.with("orient", ParamValue::Menu(3))).is_err());
        assert!(cooked(|p| p.with("radius", ParamValue::F(f32::NAN))).is_err());
        assert!(
            cooked(|p| p.with("radius", ParamValue::F(f32::MAX)).with("t", ParamValue::V3([f32::MAX; 3]))).is_err()
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

    /// Hexe twin == real Houdini 20.5 `.geo` for all five solids (default radius) and a transformed icosahedron.
    #[test]
    fn platonic_matches_houdini_fixtures() {
        for n in [
            twin!("sop-platonic"),
            twin!("sop-platonic-cube"),
            twin!("sop-platonic-octa"),
            twin!("sop-platonic-icosa"),
            twin!("sop-platonic-dodeca"),
            twin!("sop-platonic-xform"),
        ] {
            assert_eq!(n, Ok(()));
        }
    }
}
