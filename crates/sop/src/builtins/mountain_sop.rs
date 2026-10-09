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
//! QA params (OUR names, UNVERIFIED): `noisetype` perlin|simplex|value|worley; `octaves` I 1 (1..=16) with `roughness` F 0.5 (amplitude
//! ratio) and `lacunarity` F 2 (frequency ratio): `n = sum(a_k n_k) / sum(a_k)` (keeps |n| <= 1, octave k seed = seed + 7919 k);
//! `turbulence` B (each octave |n_k|, so n in [0,1]); `attenuation` F 1 (`n -> sign(n) |n|^att`); `animate` B + `animspeed` V3: noise
//! coordinate += time * animspeed (scrolling); `group` (point group, non-members stay); `updatenormals` B (point N recomputed).
//! NOT modelled (DEFERRED): `slope`, sparse-convolution / alligator noise types, the exact Houdini amplitude / attenuation curves.
use super::attribnoise_sop::noise_named;
use super::meshutil::{V3, add, bad, float, int, point_normals, positions, scale, to_f32};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
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
        ParamSpec::new("noisetype", ParamValue::Menu(0)).label("Noise Type").menu(&[
            ("perlin", "Perlin"),
            ("simplex", "Simplex"),
            ("value", "Value"),
            ("worley", "Worley"),
        ]),
        ParamSpec::new("octaves", ParamValue::I(1)).label("Fractal Depth").range(1.0, 16.0),
        ParamSpec::new("roughness", ParamValue::F(0.5)).label("Roughness").range(0.0, 1.0),
        ParamSpec::new("lacunarity", ParamValue::F(2.0)).label("Lacunarity").range(1.0, 4.0),
        ParamSpec::new("turbulence", ParamValue::B(false)).label("Turbulence"),
        ParamSpec::new("attenuation", ParamValue::F(1.0)).label("Attenuation").range(0.1, 4.0),
        ParamSpec::new("animate", ParamValue::B(false)).label("Animate"),
        ParamSpec::new("animspeed", ParamValue::V3([0.0, 0.0, 1.0])).label("Animation Speed"),
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("updatenormals", ParamValue::B(false)).label("Update Normals"),
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
        let p = ctx.params;
        let kind = ["perlin", "simplex", "value", "worley"]
            .get(p.menu("noisetype").unwrap_or(0) as usize)
            .copied()
            .ok_or_else(|| bad("`noisetype`: menu index out of range"))?;
        let octaves = p.int("octaves").unwrap_or(1).clamp(1, 16) as u32;
        let rough = p.float("roughness").map_or(0.5, f64::from);
        let lac = p.float("lacunarity").map_or(2.0, f64::from);
        let att = p.float("attenuation").map_or(1.0, f64::from);
        if ![rough, lac, att].iter().all(|v| v.is_finite()) || !(0.0..=1.0).contains(&rough) || lac <= 0.0 || att <= 0.0 {
            return Err(bad("`roughness` in 0..=1, `lacunarity` > 0, `attenuation` > 0 (all finite)"));
        }
        let turb = p.toggle("turbulence").unwrap_or(false);
        let scroll: V3 = if p.toggle("animate").unwrap_or(false) {
            let s = p.vec3("animspeed").unwrap_or([0.0; 3]);
            if !s.iter().all(|c| c.is_finite()) {
                return Err(bad("`animspeed`: not finite"));
            }
            s.map(|c| f64::from(c) * f64::from(ctx.time))
        } else {
            [0.0; 3]
        };
        let mask: Option<Vec<bool>> = match p.string("group").unwrap_or("").trim() {
            "" => None,
            names => {
                let mut m = vec![false; pos.len()];
                for n in names.split_whitespace() {
                    let g = src
                        .group(AttrClass::Point, n)
                        .ok_or_else(|| CookError::Failed(format!("mountain: unknown point group `{n}`")))?;
                    for (s, b) in m.iter_mut().zip(&g.mask) {
                        *s |= *b;
                    }
                }
                Some(m)
            }
        };
        let field = |q: V3| -> f64 {
            let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, 1.0);
            for o in 0..octaves {
                let n = noise_named(kind, q.map(|c| c * f), seed.wrapping_add(u64::from(o) * 7919), 0).unwrap_or(0.0);
                sum += amp * if turb { n.abs() } else { n };
                norm += amp;
                amp *= rough;
                f *= lac;
            }
            let n = if norm > 0.0 { sum / norm } else { 0.0 };
            if att == 1.0 { n } else { n.signum() * n.abs().powf(att) }
        };
        let moved: Vec<V3> = pos
            .iter()
            .zip(&normals)
            .enumerate()
            .map(|(i, (&p, &n))| {
                if mask.as_ref().is_some_and(|m| !m.get(i).copied().unwrap_or(false)) {
                    return p;
                }
                let q = add(p, off);
                let q = [q[0] / size + scroll[0], q[1] / size + scroll[1], q[2] / size + scroll[2]];
                add(p, scale(n, height * field(q)))
            })
            .collect();
        let mut out = (**src).clone();
        *out.p_mut() = to_f32("mountain", &moved)?;
        if p.toggle("updatenormals").unwrap_or(false) {
            let pp = positions(&out);
            let n: Vec<[f32; 3]> = point_normals(&out, &pp).into_iter().map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect();
            out.add_attr(Attribute { name: "N".into(), class: AttrClass::Point, data: AttrData::V3(n), type_info: TypeInfo::Normal });
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cook_arc, cube, defaults, grid3};
    use super::super::attribnoise_sop::perlin;
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

    /// `field()` with explicit +Z point normals (the Newell normal of this winding is -Z).
    fn field_up() -> Detail {
        let mut d = field();
        d.add_attr(Attribute { name: "N".into(), class: AttrClass::Point, data: AttrData::V3(vec![[0.0, 0.0, 1.0]; 400]), type_info: TypeInfo::Normal });
        d
    }
    fn dz(d: &Detail) -> Vec<f64> {
        d.p().iter().map(|p| f64::from(p[2])).collect()
    }
    fn qof(p: [f32; 3], size: f64) -> [f64; 3] {
        p.map(|c| f64::from(c) / size)
    }
    fn cook_at(d: Detail, p: &ParamSet, time: f32) -> Detail {
        let ctx = CookCtx { time, fps: 24.0, params: p, log: &|_| {} };
        (*new().cook(&ctx, &[Arc::new(d)]).expect("cooks")).clone()
    }
    /// Two octaves: dz = (n0 + r n1) / (1 + r), n0 = perlin(q), n1 = perlin(lac q, seed + 7919) (exact, hand-evaluated).
    #[test]
    fn fractal_octaves_combine_normalised_by_the_amplitude_sum() {
        let p = with(1.0).with("octaves", ParamValue::I(2)).with("roughness", ParamValue::F(0.5)).with("lacunarity", ParamValue::F(3.0));
        let out = cook(&*new(), field_up(), &p).expect("cooks");
        for (src, z) in field_up().p().iter().zip(dz(&out)) {
            let q = qof(*src, 1.0);
            let want = (perlin(q, 0, 0) + 0.5 * perlin(q.map(|c| c * 3.0), 7919, 0)) / 1.5;
            assert!((z - want).abs() < 1e-6, "{z} vs {want}");
            assert!(z.abs() <= 1.0 + 1e-6);
        }
        let one = cook(&*new(), field_up(), &with(1.0).with("octaves", ParamValue::I(1))).expect("cooks");
        assert_eq!(one, cook(&*new(), field_up(), &with(1.0)).expect("cooks"), "octaves 1 = plain noise");
    }
    /// Turbulence = |n| per octave (>= 0); attenuation 2 squares the magnitude keeping the sign.
    #[test]
    fn turbulence_is_abs_noise_and_attenuation_is_a_signed_power() {
        let plain = dz(&cook(&*new(), field_up(), &with(1.0)).expect("cooks"));
        let turb = dz(&cook(&*new(), field_up(), &with(1.0).with("turbulence", ParamValue::B(true))).expect("cooks"));
        for (a, t) in plain.iter().zip(&turb) {
            assert!((t - a.abs()).abs() < 1e-6 && *t >= 0.0);
        }
        let att = dz(&cook(&*new(), field_up(), &with(1.0).with("attenuation", ParamValue::F(2.0))).expect("cooks"));
        for (a, t) in plain.iter().zip(&att) {
            assert!((t - a.signum() * a * a).abs() < 1e-6, "{a} -> {t}");
        }
    }
    /// Noise types: dz == noise_named(type, q) exactly; distinct fields, all bounded.
    #[test]
    fn noise_types_match_the_shared_noise_functions() {
        let mut fields = Vec::new();
        for (menu, name) in [(0u32, "perlin"), (1, "simplex"), (2, "value"), (3, "worley")] {
            let out = cook(&*new(), field_up(), &with(1.0).with("noisetype", ParamValue::Menu(menu))).expect("cooks");
            for (src, z) in field().p().iter().zip(dz(&out)) {
                let want = noise_named(name, qof(*src, 1.0), 0, 0).expect("known noise");
                assert!((z - want).abs() < 1e-6 && z.abs() <= 1.0 + 1e-6, "{name}: {z} vs {want}");
            }
            fields.push(dz(&out));
        }
        for i in 0..4 {
            for j in i + 1..4 {
                assert_ne!(fields[i], fields[j], "{i} vs {j}");
            }
        }
        assert!(matches!(cook(&*new(), cube(), &with(1.0).with("noisetype", ParamValue::Menu(9))), Err(CookError::BadParam(_))));
    }
    /// Animate scrolls the noise coordinate by time * animspeed: z(p, t) == z(p + t * speed * size, 0). Off = time-independent.
    #[test]
    fn animate_scrolls_the_noise_with_time() {
        let base = with(1.0).with("elementsize", ParamValue::F(2.0));
        let anim = base.clone().with("animate", ParamValue::B(true)).with("animspeed", ParamValue::V3([0.5, 0.0, 0.25]));
        let at_t = cook_at(field(), &anim, 1.5);
        // shift = t * speed * size = (1.5, 0, 0.75)
        let mut shifted = field();
        for p in shifted.p_mut().iter_mut() {
            p[0] += 1.5;
            p[2] += 0.75;
        }
        let at_0 = cook_at(shifted.clone(), &base, 0.0);
        for (a, b) in at_t.p().iter().zip(at_0.p()) {
            // flat +Z normals: both displace along Z; the shifted copy carries the 0.75 base offset in z
            assert!((f64::from(a[2]) - (f64::from(b[2]) - 0.75)).abs() < 1e-5, "{a:?} vs {b:?}");
        }
        let still = cook_at(field(), &base.clone().with("animspeed", ParamValue::V3([9.0, 9.0, 9.0])), 7.0);
        assert_eq!(still, cook_at(field(), &base, 0.0), "animate off ignores time");
    }
    /// Group = first 100 points: only those move; non-members bit-identical; unknown group = Failed.
    #[test]
    fn group_limits_displacement() {
        let mut d = field();
        d.groups.push(hexe_geo::Group { name: "g".into(), class: AttrClass::Point, mask: (0..400).map(|i| i < 100).collect() });
        let out = cook(&*new(), d.clone(), &with(1.0).with("group", ParamValue::S("g".into()))).expect("cooks");
        let full = cook(&*new(), d.clone(), &with(1.0)).expect("cooks");
        for i in 0..400 {
            if i < 100 {
                assert_eq!(out.p()[i], full.p()[i]);
            } else {
                assert_eq!(out.p()[i], d.p()[i]);
            }
        }
        let nope = cook(&*new(), d, &with(1.0).with("group", ParamValue::S("zz".into())));
        assert!(matches!(nope, Err(CookError::Failed(_))));
    }
    /// updatenormals: unit point N written after displacement; the undisplaced flat grid (height tiny) has N ~ +-Z; off = none.
    #[test]
    fn updatenormals_writes_unit_normals() {
        assert!(cook(&*new(), field(), &with(0.001)).expect("cooks").attr(AttrClass::Point, "N").is_none());
        let out = cook(&*new(), field(), &with(0.001).with("updatenormals", ParamValue::B(true))).expect("cooks");
        let Some(AttrData::V3(n)) = out.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N") };
        assert_eq!(n.len(), 400);
        for v in n {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-4 && v[2].abs() > 0.99, "{v:?}");
        }
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
