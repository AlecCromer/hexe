//! Reverse SOP (token `reverse`): reverses the vertex order of every selected polygon / polyline.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; closed-form oracle only)
//! The vertex list of each selected `Poly` / `Polyline` prim is reversed IN PLACE (full reversal, first vertex too), so a
//! closed polygon's Newell normal is negated. Vertex attributes and vertex-group bits travel with their vertex.
//! Points, prim order, prim attributes and all other classes are untouched. Reversing twice is the identity.
//! `group`: space-separated PRIM group names (union); empty = all prims. Unknown name -> `CookError::Failed`.
//! Other prim kinds (spheres, volumes, unknown) and prims with `< 2` vertices pass through. Nothing reversed -> the input
//! `Arc` is shared.
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::ops::Range;
use std::sync::Arc;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
pub fn spec() -> Vec<ParamSpec> {
    vec![ParamSpec::new("group", ParamValue::S(String::new())).label("Group")]
}
pub fn new() -> Box<SopNode> {
    Box::new(Reverse { specs: spec() })
}
struct Reverse {
    specs: Vec<ParamSpec>,
}
fn failed(m: &str) -> CookError {
    CookError::Failed(format!("reverse: {m}"))
}
/// Reverse elements `r` of every per-vertex array of `d` (attrs + groups).
fn reverse_range(d: &mut Detail, r: Range<usize>) {
    fn rev<T>(v: &mut [T], r: &Range<usize>) {
        if let Some(s) = v.get_mut(r.clone()) {
            s.reverse();
        }
    }
    fn rev_n<T>(v: &mut [T], size: usize, r: &Range<usize>) {
        let (Some(lo), Some(hi)) = (r.start.checked_mul(size), r.end.checked_mul(size)) else { return };
        if let Some(s) = v.get_mut(lo..hi) {
            let n = r.len();
            for i in 0..n / 2 {
                for k in 0..size {
                    s.swap(i * size + k, (n - 1 - i) * size + k);
                }
            }
        }
    }
    rev(&mut d.vertex_point, &r);
    for a in d.attrs.iter_mut().filter(|a| a.class == AttrClass::Vertex) {
        match &mut a.data {
            AttrData::F32(v) => rev(v, &r),
            AttrData::I32(v) => rev(v, &r),
            AttrData::V2(v) => rev(v, &r),
            AttrData::V3(v) => rev(v, &r),
            AttrData::V4(v) => rev(v, &r),
            AttrData::Str { idx, .. } => rev(idx, &r),
            AttrData::FN { size, data } => rev_n(data, *size as usize, &r),
            AttrData::IN { size, data } => rev_n(data, *size as usize, &r),
        }
    }
    for g in d.groups.iter_mut().filter(|g| g.class == AttrClass::Vertex) {
        rev(&mut g.mask, &r);
    }
}
impl Node<Detail> for Reverse {
    fn type_name(&self) -> &'static str {
        "reverse"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let names: Vec<&str> = ctx.params.string("group").unwrap_or("").split_whitespace().collect();
        let mut sel = vec![names.is_empty(); src.prims.len()];
        for n in names {
            let g = src.group(AttrClass::Prim, n).ok_or_else(|| failed(&format!("unknown primitive group `{n}`")))?;
            for (s, m) in sel.iter_mut().zip(&g.mask) {
                *s |= *m;
            }
        }
        let nverts = src.vertex_point.len();
        let ranges: Vec<Range<usize>> = src
            .prims
            .iter()
            .zip(&sel)
            .filter(|(pr, s)| **s && matches!(pr.kind, PrimKind::Poly { .. } | PrimKind::Polyline))
            .filter_map(|(pr, _)| Some(usize::try_from(pr.verts.start).ok()?..usize::try_from(pr.verts.end).ok()?))
            .filter(|r| r.len() >= 2 && r.end <= nverts)
            .collect();
        if ranges.is_empty() {
            return Ok(Arc::clone(src));
        }
        let mut out = (**src).clone();
        for r in ranges {
            reverse_range(&mut out, r);
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_scene;
    use hexe_geo::{Attribute, Group, TypeInfo};
    use hexe_graph::ParamSet;
    fn cook(d: Arc<Detail>, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        new().cook(&ctx, &[d])
    }
    fn cube() -> Detail {
        let mut d = Detail::default();
        d.add_points(8);
        let c = |b: u32, i: u32| if (i >> b) & 1 == 1 { 0.5 } else { -0.5 };
        *d.p_mut() = (0..8).map(|i| [c(2, i), c(1, i), c(0, i)]).collect();
        for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
            d.add_poly(&f, true);
        }
        d
    }
    /// Independent unnormalised Newell vector of prim `i` (textbook form, f64).
    fn newell(d: &Detail, i: usize) -> [f64; 3] {
        let pr = &d.prims[i];
        let q: Vec<[f64; 3]> = d.vertex_point[pr.verts.start as usize..pr.verts.end as usize]
            .iter()
            .map(|&p| d.p()[p as usize].map(f64::from))
            .collect();
        let mut n = [0.0f64; 3];
        for k in 0..q.len() {
            let (a, b) = (q[k], q[(k + 1) % q.len()]);
            n[0] += (a[1] - b[1]) * (a[2] + b[2]);
            n[1] += (a[2] - b[2]) * (a[0] + b[0]);
            n[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        n
    }
    fn none() -> ParamSet {
        ParamSet::new().with("group", ParamValue::S(String::new()))
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "reverse");
        assert_eq!(n.params().len(), 1);
        assert_eq!(n.params()[0].name, "group");
        assert_eq!(n.params()[0].default, ParamValue::S(String::new()));
    }
    #[test]
    fn box_reversed_negates_every_prim_normal_exactly() {
        let src = Arc::new(cube());
        let out = cook(Arc::clone(&src), none()).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (8, 6));
        for i in 0..6 {
            let (a, b) = (newell(&src, i), newell(&out, i));
            assert_eq!(b, a.map(|c| -c), "prim {i}");
            assert!(a.iter().any(|c| *c != 0.0));
        }
        assert_eq!(out.p(), src.p(), "points untouched");
    }
    #[test]
    fn reversed_twice_is_byte_equal_to_input() {
        let mut d = cube();
        d.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::V2((0..24).map(|i| [i as f32, -(i as f32)]).collect()),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "k".into(),
            class: AttrClass::Vertex,
            data: AttrData::IN { size: 2, data: (0..48).collect() },
            type_info: TypeInfo::None,
        });
        d.groups.push(Group {
            name: "v".into(),
            class: AttrClass::Vertex,
            mask: (0..24).map(|i| i % 3 == 0).collect(),
        });
        let src = Arc::new(d);
        let once = cook(Arc::clone(&src), none()).expect("cooks");
        assert_ne!(*once, *src);
        let twice = cook(once, none()).expect("cooks");
        assert_eq!(*twice, *src);
    }
    #[test]
    fn vertex_attrs_and_groups_travel_with_their_vertex() {
        let mut d = cube();
        d.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::F32((0..24).map(|i| i as f32).collect()),
            type_info: TypeInfo::None,
        });
        d.groups.push(Group {
            name: "first".into(),
            class: AttrClass::Vertex,
            mask: (0..24).map(|i| i % 4 == 0).collect(),
        });
        let src = Arc::new(d);
        let out = cook(Arc::clone(&src), none()).expect("cooks");
        let (Some(AttrData::F32(a)), Some(AttrData::F32(b))) =
            (src.attr(AttrClass::Vertex, "uv").map(|x| &x.data), out.attr(AttrClass::Vertex, "uv").map(|x| &x.data))
        else {
            panic!("uv lost")
        };
        let gm = &out.group(AttrClass::Vertex, "first").expect("group").mask;
        for (vi, vp) in out.vertex_point.iter().enumerate() {
            // find the same (point, uv-tag) pairing in the source: attr value names the ORIGINAL vertex index
            let orig = b[vi] as usize;
            assert_eq!(src.vertex_point[orig], *vp, "vertex {vi}");
            assert_eq!(a[orig], b[vi]);
            assert_eq!(gm[vi], orig % 4 == 0);
        }
    }
    #[test]
    fn group_limits_reversal_and_shares_arc_when_nothing_selected() {
        let mut d = cube();
        d.groups.push(Group {
            name: "g".into(),
            class: AttrClass::Prim,
            mask: vec![true, false, false, false, false, true],
        });
        d.groups.push(Group { name: "none".into(), class: AttrClass::Prim, mask: vec![false; 6] });
        let src = Arc::new(d);
        let out = cook(Arc::clone(&src), ParamSet::new().with("group", ParamValue::S("g".into()))).expect("cooks");
        for i in 0..6 {
            let (a, b) = (newell(&src, i), newell(&out, i));
            let flipped = i == 0 || i == 5;
            assert_eq!(b, if flipped { a.map(|c| -c) } else { a }, "prim {i}");
        }
        let same = cook(Arc::clone(&src), ParamSet::new().with("group", ParamValue::S("none".into()))).expect("cooks");
        assert!(Arc::ptr_eq(&same, &src));
        let unknown = cook(src, ParamSet::new().with("group", ParamValue::S("nope".into())));
        assert!(matches!(unknown, Err(CookError::Failed(_))));
    }
    #[test]
    fn hostile_short_and_foreign_prims_pass_through() {
        let mut d = cube();
        d.add_poly(&[0], true); // 1 vertex
        d.add_poly(&[], false); // empty
        d.prims.push(hexe_geo::Prim { kind: PrimKind::Volume, verts: 0..0 });
        d.prims.push(hexe_geo::Prim { kind: PrimKind::Polyline, verts: 5..9999 }); // out-of-range
        let src = Arc::new(d);
        let out = cook(Arc::clone(&src), none()).expect("cooks");
        assert_eq!(out.prims, src.prims);
        assert_eq!(&out.vertex_point[25..], &src.vertex_point[25..]);
        // only unreversible prims -> same Arc
        let mut only = Detail::default();
        only.add_points(1);
        only.add_poly(&[0], false);
        let only = Arc::new(only);
        assert!(Arc::ptr_eq(&cook(Arc::clone(&only), none()).expect("cooks"), &only));
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &none(), log: &|_| {} };
        assert_eq!(new().cook(&ctx, &[]).expect("cooks").point_count, 0);
    }
    #[test]
    fn scene_loader_and_registry() {
        let json = r#"{"nodes":[{"name":"b","type":"box"},{"name":"r","type":"reverse"}],"edges":[["b",0,"r",0]],"display":"r"}"#;
        let (mut g, id) = load_scene(json).expect("loads");
        let out = g.cook(id, 0.0).expect("cooks");
        assert_eq!(out.prim_count(), 6);
        assert_eq!(crate::registry().create("reverse").map(|n| n.type_name()), Some("reverse"));
    }
}
