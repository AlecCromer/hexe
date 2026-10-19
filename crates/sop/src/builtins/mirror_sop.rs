//! Mirror SOP (token `mirror`): reflect geometry across a plane.
//!
//! # Semantics (DOC-VERIFIED vs Houdini 20.5 help `sop/mirror.txt`, tests/mirror_semantics.rs; no recorded Houdini row)
//! Plane = through `origin + normalize(dir) * dist`, normal `normalize(dir)`; `dir` of zero length = `BadParam`.
//! `group` = primitive pattern (as in the `group` SOP; empty = every prim; a prim-less point cloud mirrors all points).
//! * `keepOriginal` (on): output = the input unchanged + a reflected copy of the selected prims (and the points they use,
//!   fresh points; every attribute and group follows the source element).
//! * `keepOriginal` off: "only the selected geometry will remain": unselected prims (and points only they used) are dropped,
//!   the selected prims' points are reflected IN PLACE; no new elements.
//! * `createoutputgroup` (off) + `outputgroup`: prim group holding exactly the mirrored prims.
//! * `reverse` (on): reflected closed polygons get their vertex order reversed so the winding / normal stays consistent.
//! * `consolidate` (off, keep-original only): a copied point within `consolidatetol` of the plane reuses its original point
//!   (welds the seam).
//! * Point / vertex `V3` attributes typed Normal or Vector (not `P`) are reflected on the reflected elements.
//!
//! Not modelled: sphere / tube transforms (copied as-is), `Mirror` of volumes, output group. No selected prim -> input `Arc`.
use super::group_sop::{parse_pattern, prim_verts};
use super::meshutil::{MAX_ELEMS, Remap, V3, bad, dot, failed, float, positions, rebuild, scale, sub, to_f32, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail, Prim, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "mirror";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("origin", ParamValue::V3([0.0; 3])).label("Origin"),
        ParamSpec::new("dir", ParamValue::V3([0.0, 1.0, 0.0])).label("Direction"),
        ParamSpec::new("dist", ParamValue::F(0.0)).label("Distance").range(-5.0, 5.0),
        ParamSpec::new("keepOriginal", ParamValue::B(true)).label("Keep Original"),
        ParamSpec::new("reverse", ParamValue::B(true)).label("Reverse Polygons"),
        ParamSpec::new("consolidate", ParamValue::B(false)).label("Consolidate Seam Points"),
        ParamSpec::new("consolidatetol", ParamValue::F(0.001)).label("Tolerance").range(0.0, 1.0),
        ParamSpec::new("createoutputgroup", ParamValue::B(false)).label("Create Output Group"),
        ParamSpec::new("outputgroup", ParamValue::S(String::new())).label("Output Group"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Mirror { specs: spec() })
}

struct Mirror {
    specs: Vec<ParamSpec>,
}

fn v3(p: &hexe_graph::ParamSet, n: &str) -> Result<V3, CookError> {
    let v = p.vec3(n).ok_or_else(|| bad(format!("`{n}`: missing or not a vector")))?;
    Ok([f64::from(v[0]), f64::from(v[1]), f64::from(v[2])])
}

impl Node<Detail> for Mirror {
    fn type_name(&self) -> &'static str {
        "mirror"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let pat = p.string("group").ok_or_else(|| bad("`group`: missing or not a string"))?;
        let (origin, dir, dist) = (v3(p, "origin")?, v3(p, "dir")?, float(p, "dist")?);
        let (keep, reverse, consolidate) =
            (toggle(p, "keepOriginal")?, toggle(p, "reverse")?, toggle(p, "consolidate")?);
        let tol = float(p, "consolidatetol")?;
        let (make_grp, grp_name) =
            (toggle(p, "createoutputgroup")?, p.string("outputgroup").unwrap_or_default().trim());
        if make_grp && (grp_name.is_empty() || grp_name.contains(char::is_whitespace)) {
            return Err(bad("`outputgroup` must be one non-empty token when `createoutputgroup` is on".to_string()));
        }
        if tol < 0.0 {
            return Err(bad("`consolidatetol` must be >= 0"));
        }
        let len = dot(dir, dir).sqrt();
        if len.is_nan() || len <= 1e-12 {
            return Err(bad("`dir` must be a non-zero vector"));
        }
        let n = scale(dir, 1.0 / len);
        let plane = [origin[0] + n[0] * dist, origin[1] + n[1] * dist, origin[2] + n[2] * dist];
        let Some(src) = inputs.first() else { return Ok(Arc::default()) };
        let np = src.point_count as usize;
        let pos = positions(src);
        // signed distance to the plane / reflection of a point / reflection of a direction vector
        let sd = |q: V3| dot(sub(q, plane), n);
        let reflect = |q: V3| sub(q, scale(n, 2.0 * sd(q)));
        let reflect_dir = |q: V3| sub(q, scale(n, 2.0 * dot(q, n)));

        let cloud = src.prims.is_empty() && pat.trim().is_empty();
        let sel = if cloud {
            Vec::new()
        } else {
            parse_pattern(src, AttrClass::Prim, pat, OP)?.unwrap_or_else(|| vec![true; src.prims.len()])
        };
        let mut used = vec![cloud; np];
        for (q, pr) in src.prims.iter().enumerate() {
            if sel.get(q).copied().unwrap_or(false) {
                for v in prim_verts(src, pr) {
                    if let Some(u) = src.vertex_point.get(v).and_then(|&pt| used.get_mut(pt as usize)) {
                        *u = true;
                    }
                }
            }
        }
        if !used.iter().any(|u| *u) {
            return Ok(Arc::clone(src));
        }

        // points
        // keep off (Houdini: "only the selected geometry will remain"): drop unselected prims + the points they alone used
        let drop_rest = !keep && !cloud;
        let mut point_src: Vec<Vec<usize>> = (0..np).filter(|&i| !drop_rest || used[i]).map(|i| vec![i]).collect();
        let mut out_pos: Vec<V3> = (0..np).filter(|&i| !drop_rest || used[i]).map(|i| pos[i]).collect();
        let mut newidx: Vec<usize> = if drop_rest {
            let mut r = 0;
            (0..np)
                .map(|i| {
                    let v = r;
                    r += usize::from(used[i]);
                    v
                })
                .collect()
        } else {
            (0..np).collect()
        };
        let mut ref_pts: Vec<usize> = Vec::new();
        for i in (0..np).filter(|&i| used[i]) {
            let q = pos[i];
            if keep {
                if consolidate && sd(q).abs() <= tol {
                    continue;
                }
                newidx[i] = out_pos.len();
                ref_pts.push(out_pos.len());
                out_pos.push(reflect(q));
                point_src.push(vec![i]);
            } else {
                out_pos[newidx[i]] = reflect(q);
                ref_pts.push(newidx[i]);
            }
            if out_pos.len() as u64 > MAX_ELEMS {
                return Err(failed(OP, "too many points"));
            }
        }

        // prims: (source prim, reflected?)
        let mut order: Vec<(usize, bool)> = Vec::new();
        for q in 0..src.prims.len() {
            let selected = sel.get(q).copied().unwrap_or(false);
            if keep || selected || !drop_rest {
                order.push((q, !keep && selected));
            }
        }
        if keep {
            order.extend((0..src.prims.len()).filter(|&q| sel.get(q).copied().unwrap_or(false)).map(|q| (q, true)));
        }
        let (mut vertex_point, mut vert_src, mut prims, mut prim_src) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut ref_verts: Vec<usize> = Vec::new();
        let mut order_mir: Vec<bool> = Vec::new();
        for (q, mir) in order {
            order_mir.push(mir);
            let pr = &src.prims[q];
            let mut vs: Vec<usize> = prim_verts(src, pr).collect();
            if mir && reverse && matches!(pr.kind, PrimKind::Poly { closed: true }) {
                vs.reverse();
            }
            let start = vertex_point.len();
            for v in vs {
                let pt = src.vertex_point.get(v).copied().unwrap_or(0) as usize;
                let mapped = if mir { newidx.get(pt).copied().unwrap_or(pt) } else { pt };
                if mir {
                    ref_verts.push(vertex_point.len());
                }
                vertex_point.push(u32::try_from(mapped).map_err(|_| failed(OP, "point id overflow"))?);
                vert_src.push(Some(v));
            }
            if vertex_point.len() as u64 > MAX_ELEMS || prims.len() as u64 > MAX_ELEMS {
                return Err(failed(OP, "too many vertices / primitives"));
            }
            let end = u32::try_from(vertex_point.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
            prims.push(Prim { kind: pr.kind.clone(), verts: u32::try_from(start).unwrap_or(0)..end });
            prim_src.push(Some(q));
        }
        let mirrored_prims: Vec<bool> = order_mir;
        let rm = Remap { point_src, vert_src, prim_src };
        let mut out = rebuild(OP, src, vertex_point, prims, &rm, to_f32(OP, &out_pos)?)?;
        if make_grp {
            let g = hexe_geo::Group { name: grp_name.to_string(), class: AttrClass::Prim, mask: mirrored_prims };
            out.groups.retain(|x| !(x.class == AttrClass::Prim && x.name == g.name));
            out.groups.push(g);
        }
        for a in out.attrs.iter_mut() {
            let idx = match a.class {
                AttrClass::Point if a.name != "P" => &ref_pts,
                AttrClass::Vertex => &ref_verts,
                _ => continue,
            };
            if !matches!(a.type_info, TypeInfo::Normal | TypeInfo::Vector) {
                continue;
            }
            if let AttrData::V3(v) = &mut a.data {
                for &i in idx {
                    if let Some(slot) = v.get_mut(i) {
                        let r = reflect_dir([f64::from(slot[0]), f64::from(slot[1]), f64::from(slot[2])]);
                        *slot = [r[0] as f32, r[1] as f32, r[2] as f32];
                    }
                }
            }
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::{cube, run, s};
    use super::super::meshutil::testkit;
    use super::*;
    use hexe_geo::Attribute;

    /// 3x3 points, x/y in 0..2, z = 0, +Z winding.
    fn grid3() -> Arc<Detail> {
        Arc::new(testkit::grid3())
    }
    fn ov(items: &[(&str, ParamValue)], d: &Arc<Detail>) -> Arc<Detail> {
        run("mirror", items, Some(d)).expect("cooks")
    }
    fn yaxis() -> [(&'static str, ParamValue); 1] {
        [("dir", ParamValue::V3([0.0, 1.0, 0.0]))]
    }
    /// Newell z of prim `i` (grid3 lives in z=0).
    fn area_z(d: &Detail, i: usize) -> f32 {
        let pr = &d.prims[i];
        let pts: Vec<[f32; 3]> =
            d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&p| d.p()[p as usize]).collect();
        (0..pts.len()).map(|k| pts[k][0] * pts[(k + 1) % pts.len()][1] - pts[(k + 1) % pts.len()][0] * pts[k][1]).sum()
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        let got: Vec<(&str, ParamValue)> = n.params().iter().map(|s| (s.name.as_str(), s.default.clone())).collect();
        assert_eq!(got[0], ("group", ParamValue::S(String::new())));
        assert_eq!(got[2], ("dir", ParamValue::V3([0.0, 1.0, 0.0])));
        assert_eq!(got[4], ("keepOriginal", ParamValue::B(true)));
        assert_eq!(got.len(), 10);
    }

    #[test]
    fn keep_original_doubles_geometry_and_reflects_positions() {
        let g = grid3();
        let o = ov(&yaxis(), &g);
        assert_eq!((o.point_count, o.prims.len(), o.vertex_point.len()), (18, 8, 32));
        assert_eq!(&o.p()[..9], g.p(), "original untouched");
        for i in 0..9 {
            let (a, b) = (g.p()[i], o.p()[9 + i]);
            assert_eq!(b, [a[0], -a[1], a[2]]);
        }
    }

    #[test]
    fn reverse_keeps_winding_and_off_flips_it() {
        let g = grid3();
        let on = ov(&yaxis(), &g);
        for i in 0..4 {
            assert!(area_z(&on, i) * area_z(&on, 4 + i) > 0.0, "prim {i} consistent winding");
        }
        let mut off = yaxis().to_vec();
        off.push(("reverse", ParamValue::B(false)));
        let o = ov(&off, &g);
        assert!(area_z(&o, 0) * area_z(&o, 4) < 0.0, "reflection alone flips the winding");
    }

    #[test]
    fn plane_origin_and_distance_offset_the_copy() {
        let c = cube();
        let o = ov(&[("dir", ParamValue::V3([1.0, 0.0, 0.0])), ("dist", ParamValue::F(2.0))], &c);
        let (lo, hi) = o.bbox();
        assert_eq!((lo[0], hi[0]), (-0.5, 4.5), "x: cube [-.5,.5] reflected over x=2 -> [3.5,4.5]");
        let o2 = ov(
            &[
                ("dir", ParamValue::V3([2.0, 0.0, 0.0])),
                ("origin", ParamValue::V3([1.0, 5.0, 5.0])),
                ("dist", ParamValue::F(1.0)),
            ],
            &c,
        );
        assert_eq!(o2.bbox(), o.bbox(), "dir is normalised; origin + n*dist = x=2 plane");
    }

    #[test]
    fn keep_off_reflects_in_place() {
        let g = grid3();
        let mut p = yaxis().to_vec();
        p.push(("keepOriginal", ParamValue::B(false)));
        let o = ov(&p, &g);
        assert_eq!((o.point_count, o.prims.len()), (9, 4));
        for i in 0..9 {
            assert_eq!(o.p()[i], [g.p()[i][0], -g.p()[i][1], 0.0]);
        }
        assert!(area_z(&o, 0) * area_z(&g, 0) > 0.0, "reverse on: winding preserved");
    }

    #[test]
    fn consolidate_welds_seam_points() {
        let g = grid3();
        let mut p = yaxis().to_vec();
        p.push(("consolidate", ParamValue::B(true)));
        let o = ov(&p, &g);
        assert_eq!(o.point_count, 15, "3 seam points (y=0) reused");
        assert_eq!(o.prims.len(), 8);
        // the mirrored prim next to the seam uses the original seam points
        let seam: Vec<u32> = o.vertex_point[16..32].iter().copied().filter(|&p| p < 9).collect();
        assert!(!seam.is_empty());
    }

    #[test]
    fn group_limits_the_copy_and_carries_attributes() {
        let mut d = (*grid3()).clone();
        d.add_attr(Attribute {
            name: "id".into(),
            class: AttrClass::Prim,
            data: AttrData::I32(vec![10, 11, 12, 13]),
            type_info: TypeInfo::None,
        });
        let d = Arc::new(d);
        let o = ov(&[("group", s("1")), ("dir", ParamValue::V3([0.0, 1.0, 0.0]))], &d);
        assert_eq!((o.point_count, o.prims.len()), (13, 5));
        let Some(AttrData::I32(ids)) = o.attr(AttrClass::Prim, "id").map(|a| &a.data) else { panic!("id") };
        assert_eq!(ids, &[10, 11, 12, 13, 11]);
    }

    #[test]
    fn normals_and_vectors_are_reflected_on_the_copy_only() {
        let mut d = (*grid3()).clone();
        d.add_attr(Attribute {
            name: "N".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[0.0, 1.0, 1.0]; 9]),
            type_info: TypeInfo::Normal,
        });
        d.add_attr(Attribute {
            name: "plain".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[0.0, 1.0, 1.0]; 9]),
            type_info: TypeInfo::None,
        });
        let o = ov(&yaxis(), &Arc::new(d));
        let Some(AttrData::V3(n)) = o.attr(AttrClass::Point, "N").map(|a| &a.data) else { panic!("N") };
        assert_eq!((n[0], n[9]), ([0.0, 1.0, 1.0], [0.0, -1.0, 1.0]));
        let Some(AttrData::V3(q)) = o.attr(AttrClass::Point, "plain").map(|a| &a.data) else { panic!("plain") };
        assert_eq!(q[9], [0.0, 1.0, 1.0], "untyped V3 is not a direction");
    }

    #[test]
    fn point_cloud_mirrors_all_points_and_edge_cases() {
        let mut c = Detail::default();
        c.add_points(2);
        *c.p_mut() = vec![[1.0, 2.0, 3.0], [0.0, -1.0, 0.0]];
        let o = ov(&yaxis(), &Arc::new(c));
        assert_eq!(o.p(), &[[1.0, 2.0, 3.0], [0.0, -1.0, 0.0], [1.0, -2.0, 3.0], [0.0, 1.0, 0.0]]);
        // nothing selected -> shared
        let g = grid3();
        assert!(Arc::ptr_eq(&ov(&[("group", s("!*"))], &g), &g));
        assert_eq!(run("mirror", &[], None).expect("cooks").point_count, 0);
    }

    #[test]
    fn hostile_params() {
        let g = grid3();
        let r = |items: &[(&str, ParamValue)]| run("mirror", items, Some(&g));
        assert!(matches!(r(&[("dir", ParamValue::V3([0.0; 3]))]), Err(CookError::BadParam(_))));
        assert!(matches!(r(&[("consolidatetol", ParamValue::F(-1.0))]), Err(CookError::BadParam(_))));
        assert!(matches!(r(&[("group", ParamValue::I(1))]), Err(CookError::BadParam(_))));
        assert!(matches!(r(&[("group", s("nogroup"))]), Err(CookError::Failed(_))));
    }

    #[test]
    fn scene_loader_and_registry_slot() {
        let ok = r#"{"nodes":[{"name":"b","type":"box"},{"name":"m","type":"mirror","params":{"dir":[1,0,0],"dist":1.5}}],"edges":[["b",0,"m",0]],"display":"m"}"#;
        let (mut g, id) = crate::load_scene(ok).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 16);
        assert_eq!(crate::registry().create("mirror").map(|n| n.type_name()), Some("mirror"));
    }
}
