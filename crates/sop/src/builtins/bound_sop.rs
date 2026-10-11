//! Bound SOP (token `bound`): axis-aligned bounding box of the input as a polygon box (Houdini 20.5 `bound`).
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-bound*`)
//! Box = the `box` SOP point/prim layout: pts 0..3 = bottom ring (x-z-, x+z-, x+z+, x-z+), 4..7 = top ring; prims
//! `[1,5,4,0] [2,6,5,1] [3,7,6,2] [0,4,7,3] [2,1,0,3] [5,6,7,4]` (closed quads).
//! * bounds = min/max of the selected points, `minpad` subtracted from min, `maxpad` added to max, then `rebar` grows any
//!   axis whose padded extent is below `minsize` symmetrically about its centre. No points -> an all-zero box (as Houdini).
//! * `group` + `grouptype`: `guess` and `prims` = primitive pattern (bounds of the points the prims use); `points` = point pattern.
//!   Empty group = every point. A non-empty selection with no member gives the all-zero box.
//! * `keepOriginal`: input first, box appended; `createboundinggeo` off -> empty output.
//! * `addboundsgroup`: prim group (`boundsgroup`) of the box prims only; `addxformattrib`: detail attrib (`xformattrib`)
//!   = 16 floats, identity + the box centre in the last row (Houdini row-major translate).
//!
//! Not modelled: `boundtype` sphere / rectangle, oriented bbox, `dodivs`, `breakpoints` / `edges` group types, radii attrib
//! -> `Unsupported`.
use super::group_sop::{parse_pattern, prim_verts};
use super::meshutil::{V3, bad, menu, to_f32, toggle, v3};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, Group, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "bound";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const BOX_PRIMS: [[u32; 4]; 6] = [[1, 5, 4, 0], [2, 6, 5, 1], [3, 7, 6, 2], [0, 4, 7, 3], [2, 1, 0, 3], [5, 6, 7, 4]];

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("grouptype", ParamValue::Menu(0)).label("Group Type").menu(&[
            ("guess", "Guess from Group"),
            ("breakpoints", "Breakpoints"),
            ("edges", "Edges"),
            ("points", "Points"),
            ("prims", "Primitives"),
        ]),
        ParamSpec::new("keepOriginal", ParamValue::B(false)).label("Keep Original Geometry"),
        ParamSpec::new("createboundinggeo", ParamValue::B(true)).label("Create Bounding Geometry"),
        ParamSpec::new("boundtype", ParamValue::Menu(0)).label("Bound Type").menu(&[
            ("off", "Box"),
            ("on", "Sphere"),
            ("rectangle", "Rectangle"),
        ]),
        ParamSpec::new("rebar", ParamValue::B(false)).label("Enforce Minimum Size"),
        ParamSpec::new("minsize", ParamValue::V3([0.0; 3])).label("Minimum Size").range(0.0, 10.0),
        ParamSpec::new("minpad", ParamValue::V3([0.0; 3])).label("Min Padding").range(-1.0, 1.0),
        ParamSpec::new("maxpad", ParamValue::V3([0.0; 3])).label("Max Padding").range(-1.0, 1.0),
        ParamSpec::new("addboundsgroup", ParamValue::B(false)).label("Add Bounds Group"),
        ParamSpec::new("boundsgroup", ParamValue::S("bounds".into())).label("Bounds Group"),
        ParamSpec::new("addxformattrib", ParamValue::B(false)).label("Add Transform Attribute"),
        ParamSpec::new("xformattrib", ParamValue::S("xform".into())).label("Transform Attribute"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Bound { specs: spec() })
}
struct Bound {
    specs: Vec<ParamSpec>,
}

impl Node<Detail> for Bound {
    fn type_name(&self) -> &'static str {
        "bound"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let group = p.string("group").ok_or_else(|| bad("`group`: missing or not a string"))?;
        let gtype = menu(p, "grouptype")?;
        let keep = toggle(p, "keepOriginal")?;
        let create = toggle(p, "createboundinggeo")?;
        let btype = menu(p, "boundtype")?;
        let rebar = toggle(p, "rebar")?;
        let (minsize, minpad, maxpad) = (v3(p, "minsize")?, v3(p, "minpad")?, v3(p, "maxpad")?);
        let (add_grp, add_xf) = (toggle(p, "addboundsgroup")?, toggle(p, "addxformattrib")?);
        let grp_name = p.string("boundsgroup").ok_or_else(|| bad("`boundsgroup`: missing or not a string"))?;
        let xf_name = p.string("xformattrib").ok_or_else(|| bad("`xformattrib`: missing or not a string"))?;
        if btype != 0 {
            return Err(CookError::Unsupported("bound: only boundtype `box` is implemented".into()));
        }
        if gtype == 1 || gtype == 2 {
            return Err(CookError::Unsupported("bound: grouptype breakpoints / edges is not implemented".into()));
        }
        if (add_grp && grp_name.trim().is_empty()) || (add_xf && xf_name.trim().is_empty()) {
            return Err(bad("`boundsgroup` / `xformattrib` must be non-empty when enabled"));
        }
        let empty = Detail::default();
        let src = inputs.first().map_or(&empty, |a| a.as_ref());
        if !create {
            return Ok(Arc::new(Detail::default()));
        }
        // points taking part
        let np = src.point_count as usize;
        let mut used = vec![true; np];
        if !group.trim().is_empty() {
            used = vec![false; np];
            if gtype == 3 {
                let sel = parse_pattern(src, AttrClass::Point, group, OP)?.unwrap_or_default();
                for (u, s) in used.iter_mut().zip(sel) {
                    *u = s;
                }
            } else {
                let sel = parse_pattern(src, AttrClass::Prim, group, OP)?.unwrap_or_default();
                for (q, pr) in src.prims.iter().enumerate() {
                    if sel.get(q).copied().unwrap_or(false) {
                        for v in prim_verts(src, pr) {
                            if let Some(u) = src.vertex_point.get(v).and_then(|&pt| used.get_mut(pt as usize)) {
                                *u = true;
                            }
                        }
                    }
                }
            }
        }
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for (q, _) in src.p().iter().zip(&used).filter(|(_, u)| **u) {
            for k in 0..3 {
                lo[k] = lo[k].min(f64::from(q[k]));
                hi[k] = hi[k].max(f64::from(q[k]));
            }
        }
        let (lo, hi) = if lo[0] > hi[0] {
            ([0.0; 3], [0.0; 3])
        } else {
            let (mut l, mut h): (V3, V3) = ([0.0; 3], [0.0; 3]);
            for k in 0..3 {
                l[k] = lo[k] - minpad[k];
                h[k] = hi[k] + maxpad[k];
                if rebar && h[k] - l[k] < minsize[k] {
                    let c = 0.5 * (l[k] + h[k]);
                    l[k] = c - 0.5 * minsize[k];
                    h[k] = c + 0.5 * minsize[k];
                }
            }
            (l, h)
        };
        let corners: Vec<V3> = (0..8u32)
            .map(|i| {
                let top = i >= 4;
                let (x, z) = match i & 3 {
                    0 => (lo[0], lo[2]),
                    1 => (hi[0], lo[2]),
                    2 => (hi[0], hi[2]),
                    _ => (lo[0], hi[2]),
                };
                [x, if top { hi[1] } else { lo[1] }, z]
            })
            .collect();
        let mut out = if keep { src.clone() } else { Detail::default() };
        let (np0, nq0) = (out.point_count, out.prim_count());
        let ids = out.add_points(8);
        let pos = to_f32(OP, &corners)?;
        out.p_mut().truncate(np0 as usize);
        out.p_mut().extend(pos);
        for f in BOX_PRIMS {
            out.add_poly(&f.map(|i| ids.start + i), true);
        }
        if add_grp {
            let mut mask = vec![false; nq0 as usize];
            mask.resize(out.prim_count() as usize, true);
            out.groups.retain(|g| !(g.class == AttrClass::Prim && g.name == grp_name));
            out.groups.push(Group { name: grp_name.to_string(), class: AttrClass::Prim, mask });
        }
        if add_xf {
            let c = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1]), 0.5 * (lo[2] + hi[2])];
            let m = [
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                c[0] as f32,
                c[1] as f32,
                c[2] as f32,
                1.0,
            ];
            out.attrs.retain(|a| !(a.class == AttrClass::Detail && a.name == xf_name));
            out.attrs.push(Attribute {
                name: xf_name.to_string(),
                class: AttrClass::Detail,
                data: AttrData::FN { size: 16, data: m.to_vec() },
                type_info: TypeInfo::None,
            });
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cube, defaults, houdini_twin};
    use super::*;
    use hexe_graph::ParamSet;

    fn cooked(f: impl FnOnce(ParamSet) -> ParamSet, d: Detail) -> Result<Arc<Detail>, CookError> {
        let n = new();
        cook(&*n, d, &f(defaults(&*n)))
    }

    /// Unit cube -> the same cube (Houdini box layout), no extra attributes.
    #[test]
    fn unit_cube_bounds_itself_in_box_layout() {
        let o = cooked(|p| p, cube()).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (8, 6));
        assert_eq!(o.p()[0], [-0.5, -0.5, -0.5]);
        assert_eq!(o.p()[2], [0.5, -0.5, 0.5]);
        assert_eq!(o.p()[4], [-0.5, 0.5, -0.5]);
        assert_eq!(&o.vertex_point[0..4], &[1, 5, 4, 0]);
    }

    #[test]
    fn padding_rebar_keep_group_and_xform() {
        let o = cooked(
            |p| p.with("minpad", ParamValue::V3([0.5, 0.5, 0.5])).with("maxpad", ParamValue::V3([1.0, 2.0, 3.0])),
            cube(),
        )
        .expect("cooks");
        assert_eq!(o.p()[6], [1.5, 2.5, 3.5]);
        assert_eq!(o.p()[0], [-1.0, -1.0, -1.0]);
        // rebar: extent 1 -> 3 about the centre
        let o =
            cooked(|p| p.with("rebar", ParamValue::B(true)).with("minsize", ParamValue::V3([3.0, 0.0, 1.0])), cube())
                .expect("cooks");
        assert_eq!((o.p()[0], o.p()[6]), ([-1.5, -0.5, -0.5], [1.5, 0.5, 0.5]));
        // keep original + group + xform
        let o = cooked(
            |p| {
                p.with("keepOriginal", ParamValue::B(true))
                    .with("addboundsgroup", ParamValue::B(true))
                    .with("addxformattrib", ParamValue::B(true))
            },
            cube(),
        )
        .expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (16, 12));
        let g = o.group(AttrClass::Prim, "bounds").expect("group");
        assert_eq!(g.mask.iter().filter(|b| **b).count(), 6);
        assert!(!g.mask[5] && g.mask[6]);
        let Some(Attribute { data: AttrData::FN { size: 16, data }, .. }) = o.attr(AttrClass::Detail, "xform") else {
            panic!("xform")
        };
        assert_eq!(&data[12..], &[0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn group_point_vs_prim_and_empty_input_and_unsupported() {
        // points 0 and 1 of the testkit cube share y = -0.5
        let o = cooked(|p| p.with("group", ParamValue::S("0 1".into())).with("grouptype", ParamValue::Menu(3)), cube())
            .expect("cooks");
        assert_eq!((o.p()[0][1], o.p()[4][1]), (-0.5, -0.5));
        let o = cooked(|p| p, Detail::default()).expect("cooks");
        assert!(o.p().iter().all(|q| *q == [0.0; 3]));
        let o = cooked(|p| p.with("createboundinggeo", ParamValue::B(false)), cube()).expect("cooks");
        assert_eq!(o.point_count, 0);
        assert!(matches!(cooked(|p| p.with("boundtype", ParamValue::Menu(1)), cube()), Err(CookError::Unsupported(_))));
        assert!(matches!(cooked(|p| p.with("grouptype", ParamValue::Menu(2)), cube()), Err(CookError::Unsupported(_))));
        assert!(cooked(|p| p.with("minpad", ParamValue::V3([f32::NAN, 0.0, 0.0])), cube()).is_err());
    }

    macro_rules! twin {
        ($n:literal) => {
            houdini_twin(
                include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
            )
        };
    }

    /// Hexe twin == real Houdini 20.5 `.geo` (P, prim cycles, attribute / group names) on the recorded oracle cases.
    #[test]
    fn bound_matches_houdini_fixtures() {
        assert_eq!(twin!("sop-bound"), Ok(()));
        assert_eq!(twin!("sop-bound-pad"), Ok(()));
        assert_eq!(twin!("sop-bound-keep"), Ok(()));
    }
}
