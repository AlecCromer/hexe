//! Mountain SOP (token `mountain`): displace points along a direction by Perlin noise.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `P' = P + dir * height * noise((P + offset) / elementsize)`. `noise` = the classic gradient (Perlin) noise of
//! `attribnoise` (`attribnoise_sop::perlin`, channel 0, hashed with `seed`): value in `[-1, 1]`, EXACTLY 0 at every
//! integer lattice point of the scaled space, deterministic for a given (`seed`, position). `dir` = `direction` param
//! if nonzero (normalized), else point `N` attribute (V3, one per point) used as is; absent / wrong-typed / wrong-length
//! = uniform-averaged Newell point normals (as `peak`; points without a usable face stay). Only `P` changes.
//! Params: `height` F 1 (amplitude; 0 = input `Arc` shared), `elementsize` F 1 (> 0), `seed` I 0, `offset` V3 0,
//! `direction` V3 (0,1,0) (zero = use N).
//! NOT modelled (DEFERRED): Houdini's `roughness`/`fractal` octaves, `turbulence`/`attenuation`/`slope` knobs, `animate`
//! (time scroll), the other noise types (simplex, sparse...), point group, `N` recompute.
use super::attribnoise_sop::perlin;
use super::meshutil::{V3, add, bad, float, int, point_normals, positions, scale, to_f32};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("height", ParamValue::F(1.0)).label("Height").range(-10.0, 10.0),
        ParamSpec::new("elementsize", ParamValue::F(1.0)).label("Element Size").range(0.01, 10.0),
        ParamSpec::new("seed", ParamValue::I(0)).label("Seed").range(0.0, 100.0),
        ParamSpec::new("offset", ParamValue::V3([0.0; 3])).label("Offset"),
        ParamSpec::new("direction", ParamValue::V3([0.0, 0.0, 0.0])).label("Direction"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Mountain { specs: spec() })
}
struct Mountain {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Mountain {
    fn type_name(&self) -> &'static str {
        "mountain"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let height = float(ctx.params, "height")?;
        let size = float(ctx.params, "elementsize")?;
        if size <= 0.0 {
            return Err(bad("`elementsize` must be > 0"));
        }
        let seed = u64::from(int(ctx.params, "seed")? as u32);
        let off = match ctx.params.vec3("offset") {
            Some(v) if v.iter().all(|c| c.is_finite()) => v.map(f64::from),
            _ => return Err(bad("`offset`: missing or not a finite vector")),
        };
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if height == 0.0 || src.point_count == 0 {
            return Ok(Arc::clone(src));
        }
        let pos = positions(src);
        let dir = match ctx.params.vec3("direction") {
            Some(v) if v.iter().all(|c| c.is_finite()) => v.map(f64::from),
            _ => return Err(bad("`direction`: missing or not a finite vector")),
        };
        let use_uniform_dir = dir[0] != 0.0 || dir[1] != 0.0 || dir[2] != 0.0;
        let normals: Vec<V3> = if use_uniform_dir {
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            if len < 1e-12 {
                return Err(bad("`direction` length too small"));
            }
            vec![[dir[0] / len, dir[1] / len, dir[2] / len]; pos.len()]
        } else {
            match src.attr(AttrClass::Point, "N").map(|a| &a.data) {
                Some(AttrData::V3(n)) if n.len() == pos.len() => {
                    n.iter().map(|v| [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]).collect()
                }
                _ => point_normals(src, &pos),
            }
        };
        let moved: Vec<V3> = pos
            .iter()
            .zip(&normals)
            .map(|(&p, &n)| {
                let q = add(p, off);
                let q = [q[0] / size, q[1] / size, q[2] / size];
                add(p, scale(n, height * perlin(q, seed, 0)))
            })
            .collect();
        let mut out = (**src).clone();
        *out.p_mut() = to_f32("mountain", &moved)?;
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, grid3};
    use super::*;
    use hexe_geo::{Attribute, TypeInfo};
    use hexe_graph::ParamSet;

    fn with(h: f32) -> ParamSet {
        defaults(&*new()).with("height", ParamValue::F(h))
    }
    /// 20x20 flat +Z grid at non-lattice coordinates.
    fn field() -> Detail {
        let mut d = Detail::default();
        d.add_points(400);
        *d.p_mut() = (0..400).map(|i| [(i % 20) as f32 * 0.37 + 0.11, (i / 20) as f32 * 0.41 + 0.23, 0.0]).collect();
        for r in 0..19u32 {
            for c in 0..19u32 {
                let a = r * 20 + c;
                d.add_poly(&[a, a + 1, a + 21, a + 20], true);
            }
        }
        d
    }

    #[test]
    fn amplitude_zero_is_identity_sharing_the_arc() {
        let a = Arc::new(field());
        assert!(Arc::ptr_eq(&cook_arc(&*new(), &a, &with(0.0)).expect("cooks"), &a));
    }

    #[test]
    fn deterministic_bounded_along_n_and_seed_dependent() {
        let one = cook(&*new(), field(), &with(0.5)).expect("cooks");
        let two = cook(&*new(), field(), &with(0.5)).expect("cooks");
        assert_eq!(one, two, "same inputs -> bit-identical output");
        let src = field();
        let mut moved = 0;
        for (a, b) in src.p().iter().zip(one.p()) {
            assert_eq!((a[0], a[1]), (b[0], b[1]), "flat +Z grid moves along Z only");
            assert!(b[2].abs() <= 0.5 + 1e-6, "|noise| <= 1 so |dz| <= height");
            moved += usize::from(b[2] != 0.0);
        }
        assert!(moved > 300, "non-lattice points are displaced ({moved})");
        let other = cook(&*new(), field(), &with(0.5).with("seed", ParamValue::I(7))).expect("cooks");
        assert_ne!(one.p(), other.p(), "seed changes the field");
        assert_eq!(one.prims, src.prims);
    }

    #[test]
    fn height_is_linear_and_negative_mirrors() {
        let h1 = cook(&*new(), field(), &with(1.0)).expect("cooks");
        let h2 = cook(&*new(), field(), &with(2.0)).expect("cooks");
        let hm = cook(&*new(), field(), &with(-1.0)).expect("cooks");
        for ((a, b), c) in h1.p().iter().zip(h2.p()).zip(hm.p()) {
            assert!((b[2] - 2.0 * a[2]).abs() < 1e-6 && (c[2] + a[2]).abs() < 1e-6);
        }
    }

    /// Perlin noise is exactly 0 at integer lattice points of P/elementsize, so those points do not move at all.
    #[test]
    fn lattice_points_are_fixed() {
        let g = grid3(); // integer coords 0..2
        let out = cook(&*new(), g.clone(), &with(3.0)).expect("cooks");
        assert_eq!(out.p(), g.p());
        // elementsize 2: lattice = even coordinates; all of (0|2, 0|2) stay, (1,*) generally do not
        let p = with(3.0).with("elementsize", ParamValue::F(2.0));
        let out = cook(&*new(), g.clone(), &p).expect("cooks");
        for i in [0usize, 2, 6, 8] {
            assert_eq!(out.p()[i], g.p()[i], "corner {i}");
        }
        assert!(out.p().iter().zip(g.p()).any(|(a, b)| a != b));
        // offset shifts the sample point: P=(0,0,0) with offset (.3,.2,.1) is no longer a lattice point
        let q = with(3.0).with("offset", ParamValue::V3([0.3, 0.2, 0.1]));
        assert_ne!(cook(&*new(), g.clone(), &q).expect("cooks").p(), g.p());
    }

    #[test]
    fn existing_n_attribute_is_the_direction() {
        let mut d = field();
        d.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[1.0, 0.0, 0.0]; 400]),
            type_info: TypeInfo::Normal,
        });
        let src = d.clone();
        let out = cook(&*new(), d, &with(0.5)).expect("cooks");
        for (a, b) in src.p().iter().zip(out.p()) {
            assert_eq!((a[1], a[2]), (b[1], b[2]), "displaced along +X only");
        }
        assert!(src.p().iter().zip(out.p()).any(|(a, b)| a[0] != b[0]));
    }

    #[test]
    fn hostile_params_and_inputs() {
        for p in [
            with(f32::NAN),
            with(1.0).with("elementsize", ParamValue::F(0.0)),
            with(1.0).with("elementsize", ParamValue::F(-1.0)),
            with(1.0).with("offset", ParamValue::V3([f32::INFINITY, 0.0, 0.0])),
            ParamSet::new(),
        ] {
            assert!(matches!(cook(&*new(), cube(), &p), Err(CookError::BadParam(_))));
        }
        assert_eq!(cook(&*new(), Detail::default(), &with(1.0)).expect("cooks").point_count, 0);
        let r = cook(&*new(), cube(), &with(f32::MAX).with("offset", ParamValue::V3([0.37; 3])));
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
        let r =
            cook(&*new(), cube(), &with(1.0).with("seed", ParamValue::I(-5)).with("elementsize", ParamValue::F(1e-30)));
        assert!(r.is_ok() || matches!(r, Err(CookError::Failed(_))));
    }

    #[test]
    fn direction_param_overrides_n_attribute() {
        let mut d = field();
        d.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[1.0, 0.0, 0.0]; 400]),
            type_info: TypeInfo::Normal,
        });
        // direction = (1,0,0): displaces along X regardless of N attr
        let p1 = with(0.5).with("direction", ParamValue::V3([1.0, 0.0, 0.0]));
        let out1 = cook(&*new(), d.clone(), &p1).expect("cooks");
        for (a, b) in d.p().iter().zip(out1.p()) {
            assert_eq!((a[1], a[2]), (b[1], b[2]), "displaced along +X only");
        }
        // direction = (0,0,1): displaces along Z instead
        let p2 = with(0.5).with("direction", ParamValue::V3([0.0, 0.0, 1.0]));
        let out2 = cook(&*new(), d.clone(), &p2).expect("cooks");
        for (a, b) in d.p().iter().zip(out2.p()) {
            assert_eq!((a[0], a[1]), (b[0], b[1]), "displaced along +Z only");
        }
        assert_ne!(out1.p(), out2.p(), "different directions give different results");
        // direction = 0: falls back to N attr = (1,0,0)
        let p3 = with(0.5).with("direction", ParamValue::V3([0.0, 0.0, 0.0]));
        let out3 = cook(&*new(), d.clone(), &p3).expect("cooks");
        assert_eq!(out3.p(), out1.p(), "zero direction uses N attr");
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/mountain.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        assert_eq!(d.point_count, 121);
        let (lo, hi) = d.bbox();
        assert!(hi[1] - lo[1] > 0.01 && hi[1] - lo[1] <= 1.0 + 1e-5, "relief within +-height/2: {lo:?} {hi:?}");
        assert_eq!(crate::registry().create("mountain").map(|n| n.type_name()), Some("mountain"));
    }
}
