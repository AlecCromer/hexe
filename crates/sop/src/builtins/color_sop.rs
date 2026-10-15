//! Color SOP (token `color`): writes the `Cd` colour attribute, constant or random per element.
//!
//! # Semantics (DOC-VERIFIED vs Houdini 20.5 help `sop/color.txt`, tests/color_semantics.rs; no recorded row)
//! * `class` (point | vertex | prim | detail; default point): which `Cd` is written.
//! * `colortype`: `constant` writes `color` (default 1,1,1); `random` writes per element three independent uniform
//!   [0,1) channels from splitmix64(seed, element, channel) - deterministic per `seed` (F, default 0), NOT Houdini's stream.
//! * `group`: pattern over the class (grammar as in the `group` SOP; ignored for detail; empty = every element). Elements
//!   outside it keep the existing same-class `Cd` (V3 of the right length), else white (1,1,1).
//!
//! Output attribute: `V3` `Cd` with type info Color; other classes' `Cd` untouched. Absent input -> empty `Detail`.
//!
//! * `colortype` `bbox` (2): Cd = element position normalised into the input bbox (flat axis -> 0). `deleteallcolorattribs` (off)
//!   removes every-class `Cd` first; `enable` (on) off = no colour written (delete-only use).
//! Not modelled: ramp-from-attribute colour type, alpha, `overridecd`.
use super::attribs::{class_param, set_attr};
use super::group_sop::{bad, parse_pattern};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const WHO: &str = "color";
const CONSTANT: u32 = 0;
const RANDOM: u32 = 1;
const BBOX: u32 = 2;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("class", ParamValue::Menu(0)).label("Class").menu(&[
            ("point", "Point"),
            ("vertex", "Vertex"),
            ("prim", "Primitive"),
            ("detail", "Detail"),
        ]),
        ParamSpec::new("deleteallcolorattribs", ParamValue::B(false)).label("Delete All Existing Color Attributes"),
        ParamSpec::new("enable", ParamValue::B(true)).label("Set Color Attribute"),
        ParamSpec::new("colortype", ParamValue::Menu(CONSTANT))
            .label("Color Type")
            .menu(&[("constant", "Constant"), ("random", "Random"), ("bbox", "Bounding Box")]),
        ParamSpec::new("color", ParamValue::V3([1.0; 3])).label("Color"),
        ParamSpec::new("seed", ParamValue::F(0.0)).label("Seed").range(0.0, 10.0),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Color { specs: spec() })
}

struct Color {
    specs: Vec<ParamSpec>,
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Uniform [0,1) for (seed, element, channel).
fn unit(seed: u32, i: usize, ch: u64) -> f32 {
    let s = splitmix(u64::from(seed));
    let h = splitmix(s ^ splitmix(i as u64).wrapping_add(ch.wrapping_mul(0xD6E8_FEB8_6659_FD93)));
    ((h >> 40) as f32) / (1u64 << 24) as f32
}

impl Node<Detail> for Color {
    fn type_name(&self) -> &'static str {
        "color"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let pat = p.string("group").ok_or_else(|| bad(WHO, "`group` missing or not a string"))?;
        let class = class_param(p, "class", WHO)?;
        let ty = p.menu("colortype").ok_or_else(|| bad(WHO, "`colortype` missing or not a menu"))?;
        if ty > BBOX {
            return Err(bad(WHO, format!("`colortype`: menu index {ty} out of range")));
        }
        let color = p.vec3("color").ok_or_else(|| bad(WHO, "`color` missing or not a vector"))?;
        let seed = p.float("seed").filter(|s| s.is_finite()).ok_or_else(|| bad(WHO, "`seed` missing or not finite"))?;
        let del = p.toggle("deleteallcolorattribs").ok_or_else(|| bad(WHO, "`deleteallcolorattribs` missing or not a toggle"))?;
        let enable = p.toggle("enable").ok_or_else(|| bad(WHO, "`enable` missing or not a toggle"))?;
        let Some(src) = inputs.first() else { return Ok(Arc::default()) };
        if !del && !enable {
            return Ok(Arc::clone(src));
        }
        let mut out = (**src).clone();
        if del {
            out.attrs.retain(|a| a.name != "Cd");
        }
        if !enable {
            return Ok(Arc::new(out));
        }
        let n = out.class_len(class);
        let sel = if class == AttrClass::Detail { None } else { parse_pattern(&out, class, pat, WHO)? };
        let mut cd: Vec<[f32; 3]> = match out.attr(class, "Cd").map(|a| &a.data) {
            Some(AttrData::V3(v)) if v.len() == n => v.clone(),
            _ => vec![[1.0; 3]; n],
        };
        // bounding-box colour: element position normalised into the input bbox (a flat axis maps to 0)
        let (lo, hi) = out.bbox();
        let norm = |q: [f32; 3]| -> [f32; 3] {
            let f = |k: usize| if hi[k] > lo[k] { (q[k] - lo[k]) / (hi[k] - lo[k]) } else { 0.0 };
            [f(0), f(1), f(2)]
        };
        let elem_pos = |i: usize| -> [f32; 3] {
            let pts: Vec<usize> = match class {
                AttrClass::Point => vec![i],
                AttrClass::Vertex => out.vertex_point.get(i).map(|&p| p as usize).into_iter().collect(),
                AttrClass::Prim => out.prims.get(i).map_or_else(Vec::new, |pr| {
                    (pr.verts.start..pr.verts.end)
                        .filter_map(|v| out.vertex_point.get(v as usize))
                        .map(|&p| p as usize)
                        .collect()
                }),
                _ => Vec::new(),
            };
            if pts.is_empty() {
                return [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
            }
            let mut c = [0.0f32; 3];
            for &pt in &pts {
                let q = out.p().get(pt).copied().unwrap_or([0.0; 3]);
                for k in 0..3 {
                    c[k] += q[k] / pts.len() as f32;
                }
            }
            c
        };
        let seed_bits = seed.to_bits();
        let mut fresh: Vec<(usize, [f32; 3])> = Vec::new();
        for i in 0..cd.len() {
            if sel.as_ref().is_some_and(|m| !m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            fresh.push((
                i,
                match ty {
                    RANDOM => [unit(seed_bits, i, 0), unit(seed_bits, i, 1), unit(seed_bits, i, 2)],
                    BBOX => norm(elem_pos(i)),
                    _ => color,
                },
            ));
        }
        for (i, c) in fresh {
            cd[i] = c;
        }
        set_attr(&mut out, Attribute { name: "Cd".into(), class, data: AttrData::V3(cd), type_info: TypeInfo::Color });
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::{cube, grid3, run, s};
    use super::*;
    use hexe_graph::ParamValue as PV;

    fn cd(d: &Detail, c: AttrClass) -> Vec<[f32; 3]> {
        match d.attr(c, "Cd").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            _ => panic!("no V3 Cd on {c:?}"),
        }
    }
    fn col(d: &Arc<Detail>, ov: &[(&str, PV)]) -> Result<Arc<Detail>, CookError> {
        run("color", ov, Some(d))
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        let got: Vec<(&str, PV)> = n.params().iter().map(|p| (p.name.as_str(), p.default.clone())).collect();
        assert_eq!(
            got,
            [
                ("group", PV::S(String::new())),
                ("class", PV::Menu(0)),
                ("deleteallcolorattribs", PV::B(false)),
                ("enable", PV::B(true)),
                ("colortype", PV::Menu(0)),
                ("color", PV::V3([1.0; 3])),
                ("seed", PV::F(0.0))
            ]
        );
    }

    #[test]
    fn constant_color_per_class() {
        let c = cube();
        let red = PV::V3([1.0, 0.0, 0.0]);
        let o = col(&c, &[("color", red.clone())]).expect("cooks");
        let a = o.attr(AttrClass::Point, "Cd").expect("Cd");
        assert_eq!((a.type_info, cd(&o, AttrClass::Point)), (TypeInfo::Color, vec![[1.0, 0.0, 0.0]; 8]));
        assert_eq!(
            cd(&col(&c, &[("class", PV::Menu(2)), ("color", red.clone())]).expect("cooks"), AttrClass::Prim).len(),
            6
        );
        assert_eq!(
            cd(&col(&c, &[("class", PV::Menu(1)), ("color", red.clone())]).expect("cooks"), AttrClass::Vertex).len(),
            24
        );
        assert_eq!(
            cd(&col(&c, &[("class", PV::Menu(3)), ("color", red)]).expect("cooks"), AttrClass::Detail),
            vec![[1.0, 0.0, 0.0]]
        );
        // geometry untouched
        assert_eq!((o.prims.clone(), o.p().to_vec()), (c.prims.clone(), c.p().to_vec()));
    }

    #[test]
    fn random_is_deterministic_in_unit_range_and_seed_dependent() {
        let c = cube();
        let r = |seed: f32| {
            cd(&col(&c, &[("colortype", PV::Menu(1)), ("seed", PV::F(seed))]).expect("cooks"), AttrClass::Point)
        };
        let (a, b, other) = (r(1.0), r(1.0), r(2.0));
        assert_eq!(a, b);
        assert_ne!(a, other);
        assert!(a.iter().flatten().all(|c| (0.0..1.0).contains(c)));
        assert!(a.windows(2).any(|w| w[0] != w[1]), "elements differ");
        assert!(a[0][0] != a[0][1] || a[0][1] != a[0][2], "channels independent");
        let mean = a.iter().flatten().sum::<f32>() / 24.0;
        assert!((0.2..0.8).contains(&mean), "mean {mean}");
    }

    #[test]
    fn group_limits_and_unselected_keep_existing_or_white() {
        let g = grid3();
        let red = PV::V3([1.0, 0.0, 0.0]);
        let o = col(&g, &[("group", s("0-2")), ("color", red.clone())]).expect("cooks");
        let v = cd(&o, AttrClass::Point);
        assert_eq!((v[0], v[2], v[3]), ([1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0; 3]));
        let blue = PV::V3([0.0, 0.0, 1.0]);
        let o2 = col(&o, &[("group", s("8")), ("color", blue)]).expect("cooks");
        let v2 = cd(&o2, AttrClass::Point);
        assert_eq!((v2[0], v2[8]), ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]), "earlier colour survives outside the group");
        // prim class + prim group
        let p = col(&g, &[("class", PV::Menu(2)), ("group", s("1")), ("color", red)]).expect("cooks");
        assert_eq!(cd(&p, AttrClass::Prim), vec![[1.0; 3], [1.0, 0.0, 0.0], [1.0; 3], [1.0; 3]]);
    }

    #[test]
    fn other_class_cd_survives_and_wrong_shape_is_replaced() {
        let c = cube();
        let pt = col(&c, &[("color", PV::V3([0.0, 1.0, 0.0]))]).expect("cooks");
        let both = col(&pt, &[("class", PV::Menu(2))]).expect("cooks");
        assert_eq!(cd(&both, AttrClass::Point), vec![[0.0, 1.0, 0.0]; 8]);
        assert_eq!(cd(&both, AttrClass::Prim).len(), 6);
        assert_eq!(both.attrs.iter().filter(|a| a.name == "Cd").count(), 2);
        let mut d = (*c).clone();
        d.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::F32(vec![0.5; 8]),
            type_info: TypeInfo::None,
        });
        let o = col(&Arc::new(d), &[("group", s("0")), ("color", PV::V3([0.0; 3]))]).expect("cooks");
        assert_eq!(cd(&o, AttrClass::Point)[1], [1.0; 3]);
        assert_eq!(o.attrs.iter().filter(|a| a.name == "Cd").count(), 1);
    }

    #[test]
    fn hostile_params_and_empty_input() {
        let c = cube();
        assert!(matches!(col(&c, &[("class", PV::Menu(9))]), Err(CookError::BadParam(_))));
        assert!(matches!(col(&c, &[("colortype", PV::Menu(9))]), Err(CookError::BadParam(_))));
        assert!(matches!(col(&c, &[("group", PV::I(1))]), Err(CookError::BadParam(_))));
        assert!(matches!(col(&c, &[("group", s("nope"))]), Err(CookError::Failed(_))));
        assert_eq!(run("color", &[], None).expect("cooks").point_count, 0);
        let empty = Arc::new(Detail::default());
        assert_eq!(col(&empty, &[]).expect("cooks").point_count, 0);
    }

    #[test]
    fn scene_loader_and_registry_slot() {
        let ok = r#"{"nodes":[{"name":"b","type":"box"},{"name":"c","type":"color","params":{"colortype":1,"class":2,"seed":3}}],"edges":[["b",0,"c",0]],"display":"c"}"#;
        let (mut g, id) = crate::load_scene(ok).expect("loads");
        assert!(g.cook(id, 0.0).expect("cooks").attr(AttrClass::Prim, "Cd").is_some());
        assert_eq!(crate::registry().create("color").map(|n| n.type_name()), Some("color"));
    }
}
