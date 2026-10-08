//! Attribute Promote SOP (token `attribpromote`): moves an attribute between classes (point / vertex / prim / detail).
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `inname` (one name) of class `inclass` -> new attribute `outname` (blank or `useoutname` off = same name) of class
//! `outclass`. Missing source -> `Failed`. Element sets (see `attribs::promote_lists`): point->prim = the prim's vertices'
//! points (a point used twice counts twice); vertex->point = the point's vertices; prim->point = the DISTINCT prims touching
//! the point; point->vertex / prim->vertex copy the owner's value; X->detail aggregates all of X; detail->X broadcasts;
//! same class copies/renames. `method` aggregates multi-element sets per component: `max | min | average | first | last |
//! sum`. Int attributes aggregate in f64 and round (average of ints rounds half away from zero); strings support only
//! `first`/`last` where a set has >1 element (else `BadParam`). Elements with an empty set (e.g. a point on no prim) get
//! the default (0 / unset string). `deloriginal` (on) removes the source afterwards (point `P` is never deleted; nothing is
//! deleted when source and result are the same class+name). Not modelled: `mode`/`median`/`sumsq`/`rms` methods.
use super::attribs::{Method, class_param, default_like, promote_lists, reduce_into, set_attr, str_param};
use super::group_sop::{bad, failed};
use crate::SopNode;
use hexe_geo::{AttrClass, Attribute, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const WHO: &str = "attribpromote";
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("inname", ParamValue::S(String::new())).label("Original Name"),
        ParamSpec::new("inclass", ParamValue::Menu(0)).label("Original Class"),
        ParamSpec::new("outclass", ParamValue::Menu(2)).label("New Class"),
        ParamSpec::new("useoutname", ParamValue::B(false)).label("Change New Name"),
        ParamSpec::new("outname", ParamValue::S(String::new())).label("New Name"),
        ParamSpec::new("method", ParamValue::Menu(0)).label("Promotion Method"),
        ParamSpec::new("deloriginal", ParamValue::B(true)).label("Delete Original"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(AttribPromote { specs: spec() })
}
struct AttribPromote {
    specs: Vec<ParamSpec>,
}
fn one_name<'a>(s: &'a str, what: &str) -> Result<&'a str, CookError> {
    let s = s.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return Err(bad(WHO, format!("`{what}` must be one non-empty name without whitespace")));
    }
    Ok(s)
}
impl Node<Detail> for AttribPromote {
    fn type_name(&self) -> &'static str {
        "attribpromote"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let (from, to) = (class_param(p, "inclass", WHO)?, class_param(p, "outclass", WHO)?);
        let m =
            p.menu("method").and_then(Method::from_menu).ok_or_else(|| bad(WHO, "`method` missing or out of range"))?;
        let inname = one_name(str_param(p, "inname", WHO)?, "inname")?;
        let use_out = p.toggle("useoutname").ok_or_else(|| bad(WHO, "`useoutname` missing or not a toggle"))?;
        let outraw = str_param(p, "outname", WHO)?;
        let outname = if use_out && !outraw.trim().is_empty() { one_name(outraw, "outname")? } else { inname };
        let del = p.toggle("deloriginal").ok_or_else(|| bad(WHO, "`deloriginal` missing or not a toggle"))?;
        let empty = Arc::new(Detail::default());
        let d = inputs.first().unwrap_or(&empty);
        let src = d.attr(from, inname).ok_or_else(|| failed(WHO, format!("unknown source attribute `{inname}`")))?;
        let lists = promote_lists(d, from, to);
        if matches!(src.data, hexe_geo::AttrData::Str { .. })
            && !matches!(m, Method::First | Method::Last)
            && lists.iter().any(|l| l.len() > 1)
        {
            return Err(bad(WHO, "string attributes support only first/last for multi-element sets"));
        }
        let mut out = (**d).clone();
        let mut data = default_like(&src.data, out.class_len(to));
        for (i, l) in lists.iter().enumerate() {
            reduce_into(&mut data, i, &src.data, l, m);
        }
        set_attr(&mut out, Attribute { name: outname.into(), class: to, data, type_info: src.type_info });
        if del && !(from == to && inname == outname) && !(from == AttrClass::Point && inname == "P") {
            out.attrs.retain(|a| !(a.class == from && a.name == inname));
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
    use hexe_graph::{CookError, ParamValue};
    use std::sync::Arc;
    const PT: ParamValue = ParamValue::Menu(0);
    const VTX: ParamValue = ParamValue::Menu(1);
    const PRIM: ParamValue = ParamValue::Menu(2);
    const DTL: ParamValue = ParamValue::Menu(3);
    const MAX: u32 = 0;
    const MIN: u32 = 1;
    const AVG: u32 = 2;
    const FIRST: u32 = 3;
    const LAST: u32 = 4;
    const SUM: u32 = 5;
    fn mk(name: &str, class: AttrClass, data: AttrData) -> Attribute {
        Attribute { name: name.into(), class, data, type_info: TypeInfo::None }
    }
    /// grid3 (3x3 points, 2x2 quads) + point `v` = point index.
    fn grid_v() -> Arc<Detail> {
        let mut d = (*grid3()).clone();
        let v: Vec<f32> = (0..9).map(|i| i as f32).collect();
        d.add_attr(mk("v", AttrClass::Point, AttrData::F32(v)));
        Arc::new(d)
    }
    fn f32s(d: &Detail, c: AttrClass, n: &str) -> Vec<f32> {
        match &d.attr(c, n).expect("attr").data {
            AttrData::F32(v) => v.clone(),
            o => panic!("not f32: {o:?}"),
        }
    }
    fn promo(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        run("attribpromote", ov, Some(d))
    }
    fn pv(from: ParamValue, to: ParamValue, m: u32) -> Vec<(&'static str, ParamValue)> {
        vec![
            ("inname", s("v")),
            ("inclass", from),
            ("outclass", to),
            ("method", ParamValue::Menu(m)),
            ("deloriginal", ParamValue::B(false)),
        ]
    }
    /// Corner point ids of each quad, derived from geometry (not from the SOP): quad (r,c) spans points
    /// {3r+c, 3r+c+1, 3r+c+3, 3r+c+4} when point id = 3*row + col.
    fn quad_pts(r: usize, c: usize) -> [usize; 4] {
        let b = 3 * r + c;
        [b, b + 1, b + 3, b + 4]
    }
    #[test]
    fn params_and_defaults() {
        let n = super::new();
        let got: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(got, ["inname", "inclass", "outclass", "useoutname", "outname", "method", "deloriginal"]);
    }
    #[test]
    fn point_index_to_prim_average_is_exact() {
        let g = grid_v();
        // sanity: topology matches the hand layout (each quad has the 4 expected points, any order)
        for (q, pr) in g.prims.iter().enumerate() {
            let mut got: Vec<usize> = pr.verts.clone().map(|v| g.vertex_point[v as usize] as usize).collect();
            got.sort();
            let mut want = quad_pts(q / 2, q % 2).to_vec();
            want.sort();
            assert_eq!(got, want, "quad {q}");
        }
        let want = |f: fn(&[f32]) -> f32| -> Vec<f32> {
            (0..4)
                .map(|q| {
                    let v: Vec<f32> = quad_pts(q / 2, q % 2).iter().map(|&i| i as f32).collect();
                    f(&v)
                })
                .collect()
        };
        let avg = promo(&g, &pv(PT, PRIM, AVG)).expect("cooks");
        assert_eq!(f32s(&avg, AttrClass::Prim, "v"), [2.0, 3.0, 5.0, 6.0], "3r+c+2");
        assert_eq!(f32s(&avg, AttrClass::Prim, "v"), want(|v| v.iter().sum::<f32>() / 4.0));
        assert!(avg.attr(AttrClass::Point, "v").is_some(), "deloriginal off keeps the source");
        let mx = promo(&g, &pv(PT, PRIM, MAX)).expect("cooks");
        assert_eq!(f32s(&mx, AttrClass::Prim, "v"), [4.0, 5.0, 7.0, 8.0]);
        let mn = promo(&g, &pv(PT, PRIM, MIN)).expect("cooks");
        assert_eq!(f32s(&mn, AttrClass::Prim, "v"), [0.0, 1.0, 3.0, 4.0]);
        let sm = promo(&g, &pv(PT, PRIM, SUM)).expect("cooks");
        assert_eq!(f32s(&sm, AttrClass::Prim, "v"), [8.0, 12.0, 20.0, 24.0]);
        // first/last = first/last vertex of the prim
        let fi = promo(&g, &pv(PT, PRIM, FIRST)).expect("cooks");
        let la = promo(&g, &pv(PT, PRIM, LAST)).expect("cooks");
        for (q, pr) in g.prims.iter().enumerate() {
            let (a, b) = (g.vertex_point[pr.verts.start as usize], g.vertex_point[pr.verts.end as usize - 1]);
            assert_eq!(f32s(&fi, AttrClass::Prim, "v")[q], a as f32);
            assert_eq!(f32s(&la, AttrClass::Prim, "v")[q], b as f32);
        }
    }
    #[test]
    fn prim_to_point_uses_distinct_incident_prims() {
        let mut d = (*grid3()).clone();
        d.add_attr(mk("id", AttrClass::Prim, AttrData::F32(vec![10.0, 20.0, 30.0, 40.0])));
        let d = Arc::new(d);
        let ov = |m| vec![("inname", s("id")), ("inclass", PRIM), ("outclass", PT), ("method", ParamValue::Menu(m))];
        let a = promo(&d, &ov(AVG)).expect("cooks");
        let v = f32s(&a, AttrClass::Point, "id");
        // centre point 4 touches all 4 quads; corner point 0 only quad 0; edge point 1 quads 0,1
        assert_eq!((v[4], v[0], v[1]), (25.0, 10.0, 15.0));
        assert_eq!(v[8], 40.0);
        assert!(a.attr(AttrClass::Prim, "id").is_none(), "deloriginal default on");
        assert_eq!(f32s(&promo(&d, &ov(MAX)).expect("cooks"), AttrClass::Point, "id")[4], 40.0);
        assert_eq!(f32s(&promo(&d, &ov(SUM)).expect("cooks"), AttrClass::Point, "id")[4], 100.0);
    }
    #[test]
    fn detail_and_vertex_paths() {
        let g = grid_v();
        let sum = promo(&g, &pv(PT, DTL, SUM)).expect("cooks");
        assert_eq!(f32s(&sum, AttrClass::Detail, "v"), [36.0]);
        let avg = promo(&g, &pv(PT, DTL, AVG)).expect("cooks");
        assert_eq!(f32s(&avg, AttrClass::Detail, "v"), [4.0]);
        // detail -> point broadcast
        let back = promo(&avg, &pv(DTL, PT, MAX)).expect("cooks");
        assert_eq!(f32s(&back, AttrClass::Point, "v"), vec![4.0; 9], "replaces the point attr");
        // point -> vertex copies each vertex's point value
        let vx = promo(&g, &pv(PT, VTX, AVG)).expect("cooks");
        let want: Vec<f32> = g.vertex_point.iter().map(|&p| p as f32).collect();
        assert_eq!(f32s(&vx, AttrClass::Vertex, "v"), want);
        // vertex -> point: centre point has 4 vertices, all of value 4
        let rt = promo(&vx, &pv(VTX, PT, AVG)).expect("cooks");
        assert_eq!(f32s(&rt, AttrClass::Point, "v"), f32s(&g, AttrClass::Point, "v"));
        // vertex -> prim average of the prim's vertex values == point->prim average
        let vp = promo(&vx, &pv(VTX, PRIM, AVG)).expect("cooks");
        assert_eq!(f32s(&vp, AttrClass::Prim, "v"), [2.0, 3.0, 5.0, 6.0]);
    }
    #[test]
    fn rename_delete_ints_vectors_strings() {
        let g = grid_v();
        let o = promo(
            &g,
            &[
                ("inname", s("v")),
                ("useoutname", ParamValue::B(true)),
                ("outname", s("pv")),
                ("method", ParamValue::Menu(AVG)),
            ],
        )
        .expect("cooks");
        assert!(o.attr(AttrClass::Point, "v").is_none() && o.attr(AttrClass::Prim, "pv").is_some());
        assert_eq!(o.point_count, 9);
        // outname ignored while useoutname is off
        let o = promo(&g, &[("inname", s("v")), ("outname", s("zzz"))]).expect("cooks");
        assert!(o.attr(AttrClass::Prim, "v").is_some() && o.attr(AttrClass::Prim, "zzz").is_none());
        // same class + same name: kept (not deleted by deloriginal)
        let o = promo(&g, &[("inname", s("v")), ("outclass", PT)]).expect("cooks");
        assert_eq!(f32s(&o, AttrClass::Point, "v"), f32s(&g, AttrClass::Point, "v"));
        // P can be promoted but never deleted
        let o = promo(&g, &[("inname", s("P")), ("method", ParamValue::Menu(AVG))]).expect("cooks");
        assert_eq!(o.p().len(), 9);
        match &o.attr(AttrClass::Prim, "P").expect("prim P").data {
            AttrData::V3(v) => {
                let p = g.p();
                let c = quad_pts(0, 0).map(|i| p[i]);
                let want: Vec<f32> = (0..3).map(|k| c.iter().map(|q| q[k]).sum::<f32>() / 4.0).collect();
                assert_eq!(v[0].to_vec(), want, "first quad centroid");
            }
            x => panic!("{x:?}"),
        }
        // ints average and round; vectors per component
        let mut d = (*grid3()).clone();
        d.add_attr(mk("i", AttrClass::Point, AttrData::I32(vec![0, 1, 2, 3, 4, 5, 6, 7, 8])));
        d.add_attr(mk("c", AttrClass::Point, AttrData::V3((0..9).map(|i| [i as f32, 2.0 * i as f32, 1.0]).collect())));
        let d = Arc::new(d);
        let o = promo(&d, &[("inname", s("i")), ("method", ParamValue::Menu(AVG))]).expect("cooks");
        // quad 0 {0,1,3,4} avg 2.0 ; quad 1 {1,2,4,5} avg 3.0
        assert_eq!(o.attr(AttrClass::Prim, "i").expect("i").data, AttrData::I32(vec![2, 3, 5, 6]));
        let o = promo(&d, &[("inname", s("c")), ("method", ParamValue::Menu(SUM))]).expect("cooks");
        match &o.attr(AttrClass::Prim, "c").expect("c").data {
            AttrData::V3(v) => assert_eq!(v[0], [8.0, 16.0, 4.0]),
            x => panic!("{x:?}"),
        }
        // strings: first ok, average rejected
        let mut e = (*grid3()).clone();
        let names: Vec<String> = vec!["a".into(), "b".into()];
        e.add_attr(mk("nm", AttrClass::Point, AttrData::Str { idx: (0..9).map(|i| i % 2).collect(), table: names }));
        let e = Arc::new(e);
        let o = promo(&e, &[("inname", s("nm")), ("method", ParamValue::Menu(FIRST))]).expect("cooks");
        assert!(
            matches!(&o.attr(AttrClass::Prim, "nm").expect("nm").data, AttrData::Str { idx, .. } if idx.len() == 4)
        );
        assert!(matches!(
            promo(&e, &[("inname", s("nm")), ("method", ParamValue::Menu(AVG))]),
            Err(CookError::BadParam(_))
        ));
    }
    #[test]
    fn hostile_params_and_empty_input() {
        let g = grid_v();
        assert!(matches!(promo(&g, &[("inname", s("nope"))]), Err(CookError::Failed(_))));
        for (k, v) in [
            ("inclass", ParamValue::Menu(9)),
            ("outclass", ParamValue::Menu(4)),
            ("method", ParamValue::Menu(6)),
            ("inname", s("")),
            ("inname", s("a b")),
        ] {
            let mut ov = vec![("inname", s("v"))];
            ov.push((k, v));
            assert!(matches!(promo(&g, &ov), Err(CookError::BadParam(_))), "{k}");
        }
        let bad_out = promo(&g, &[("inname", s("v")), ("useoutname", ParamValue::B(true)), ("outname", s("a b"))]);
        assert!(matches!(bad_out, Err(CookError::BadParam(_))));
        assert!(matches!(run("attribpromote", &[("inname", s("v"))], None), Err(CookError::Failed(_))));
        // attribute on an element-less class: empty sets give defaults, no panic
        let mut d = Detail::default();
        d.add_attr(mk("v", AttrClass::Point, AttrData::F32(vec![])));
        let o = promo(&Arc::new(d), &[("inname", s("v")), ("outclass", DTL)]).expect("cooks");
        assert_eq!(f32s(&o, AttrClass::Detail, "v"), [0.0]);
    }
}
