//! Attribute Copy SOP (token `attribcopy`): copies attribute values from input 1's elements onto input 0's elements.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! `class` (point | vertex | prim | detail) picks the element class; `attribs` selects the SOURCE attributes of that class
//! (pattern grammar: `attribs.rs` header; default `* ^P`). `matchby`:
//! * `index` (0): destination element `i` receives source element `i`; destinations beyond the source count are untouched.
//! * `P` (1, point class only): destination point receives the value of the source point at the same position (distance
//!   `<= tolerance`, default 1e-5; several candidates -> the closest, ties -> lowest index); unmatched points are untouched.
//!
//! Untouched elements keep their existing value (default 0 / unset string for a new attribute); an existing attribute of a
//! different shape is replaced by the source's shape. Topology and groups are untouched. Absent / empty source -> input 0
//! shared. Pair cost `n*m` in `P` mode is capped at 2^26 (`Failed`).
use super::attribs::{base_for, class_param, copy_elem, selected, set_attr, str_param};
use super::group_sop::{bad, failed};
use crate::SopNode;
use hexe_geo::{AttrClass, Attribute, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const WHO: &str = "attribcopy";
const MAX_PAIRS: usize = 1 << 26;
const BY_INDEX: u32 = 0;
const BY_P: u32 = 1;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("class", ParamValue::Menu(0)).label("Class").menu(&[
            ("point", "Point"),
            ("vertex", "Vertex"),
            ("prim", "Primitive"),
            ("detail", "Detail"),
        ]),
        ParamSpec::new("attribs", ParamValue::S("* ^P".into())).label("Attributes"),
        ParamSpec::new("matchby", ParamValue::Menu(BY_INDEX))
            .label("Match By")
            .menu(&[("index", "Index"), ("P", "Position")]),
        ParamSpec::new("tolerance", ParamValue::F(1e-5)).label("Position Tolerance").range(0.0, 1.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(AttribCopy { specs: spec() })
}
struct AttribCopy {
    specs: Vec<ParamSpec>,
}
/// For each destination point: the matching source point (closest within `tol`, lowest index on ties).
fn match_by_p(dst: &[[f32; 3]], src: &[[f32; 3]], tol: f64) -> Result<Vec<Option<usize>>, CookError> {
    if dst.len().saturating_mul(src.len()) > MAX_PAIRS {
        return Err(failed(WHO, "destination x source point count exceeds the cap"));
    }
    let t2 = tol * tol;
    Ok(dst
        .iter()
        .map(|q| {
            let mut best: Option<(f64, usize)> = None;
            for (j, s) in src.iter().enumerate() {
                let d2: f64 = (0..3).map(|k| (f64::from(q[k]) - f64::from(s[k])).powi(2)).sum();
                if d2 <= t2 && best.is_none_or(|b| d2 < b.0) {
                    best = Some((d2, j));
                }
            }
            best.map(|b| b.1)
        })
        .collect())
}
impl Node<Detail> for AttribCopy {
    fn type_name(&self) -> &'static str {
        "attribcopy"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "target" }, InputSpec { name: "source" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let class = class_param(p, "class", WHO)?;
        let pat = str_param(p, "attribs", WHO)?;
        let by = p.menu("matchby").ok_or_else(|| bad(WHO, "`matchby` missing or not a menu"))?;
        let tol = match p.float("tolerance") {
            Some(v) if v.is_finite() && v >= 0.0 => f64::from(v),
            _ => return Err(bad(WHO, "`tolerance` missing, not a float, or negative")),
        };
        if by > BY_P {
            return Err(bad(WHO, format!("`matchby`: menu index {by} out of range")));
        }
        if by == BY_P && class != AttrClass::Point {
            return Err(bad(WHO, "`matchby` P applies to the point class only"));
        }
        let empty = Arc::new(Detail::default());
        let d = inputs.first().unwrap_or(&empty);
        let src = inputs.get(1).unwrap_or(&empty);
        let n = d.class_len(class);
        let map: Vec<Option<usize>> = if by == BY_P {
            match_by_p(d.p(), src.p(), tol)?
        } else {
            let m = src.class_len(class);
            (0..n).map(|i| (i < m).then_some(i)).collect()
        };
        let mut out = (**d).clone();
        let mut changed = false;
        for a in src.attrs.iter().filter(|a| a.class == class && selected(pat, &a.name)) {
            let mut data = base_for(&out, class, &a.name, &a.data);
            for (i, m) in map.iter().enumerate() {
                if let Some(j) = *m {
                    copy_elem(&mut data, i, &a.data, j);
                }
            }
            set_attr(&mut out, Attribute { name: a.name.clone(), class, data, type_info: a.type_info });
            changed = true;
        }
        Ok(if changed { Arc::new(out) } else { Arc::clone(d) })
    }
}
#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
    use hexe_graph::{CookError, ParamValue};
    use std::sync::Arc;
    fn mk(name: &str, class: AttrClass, data: AttrData) -> Attribute {
        Attribute { name: name.into(), class, data, type_info: TypeInfo::None }
    }
    fn pts(p: &[[f32; 3]]) -> Detail {
        let mut d = Detail::default();
        d.add_points(p.len() as u32);
        d.p_mut().copy_from_slice(p);
        d
    }
    fn copy(t: &Arc<Detail>, src: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        let node = crate::registry().create("attribcopy").expect("registered");
        let mut ps = hexe_graph::ParamSet::new();
        for sp in node.params() {
            ps = ps.with(&sp.name, sp.default.clone());
        }
        for (k, v) in ov {
            ps = ps.with(k, v.clone());
        }
        let ctx = hexe_graph::CookCtx { time: 0.0, fps: 24.0, params: &ps, log: &|_| {} };
        node.cook(&ctx, &[Arc::clone(t), Arc::clone(src)])
    }
    fn f32s(d: &Detail, c: AttrClass, n: &str) -> Vec<f32> {
        match &d.attr(c, n).expect("attr").data {
            AttrData::F32(v) => v.clone(),
            o => panic!("not f32: {o:?}"),
        }
    }
    fn menu(m: u32) -> ParamValue {
        ParamValue::Menu(m)
    }
    #[test]
    fn params_and_defaults() {
        let n = super::new();
        let got: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(got, ["class", "attribs", "matchby", "tolerance"]);
        assert_eq!(n.inputs().len(), 2);
    }
    #[test]
    fn by_index_copies_prefix_and_leaves_rest() {
        let mut s = pts(&[[9.0; 3]; 4]);
        s.add_attr(mk("v", AttrClass::Point, AttrData::F32(vec![10.0, 20.0, 30.0, 40.0])));
        let mut t = (*grid3()).clone();
        t.add_attr(mk("v", AttrClass::Point, AttrData::F32(vec![-1.0; 9])));
        let (t, s) = (Arc::new(t), Arc::new(s));
        let o = copy(&t, &s, &[]).expect("cooks");
        assert_eq!(f32s(&o, AttrClass::Point, "v"), [10.0, 20.0, 30.0, 40.0, -1.0, -1.0, -1.0, -1.0, -1.0]);
        assert_eq!(o.p(), t.p(), "P not copied by the default pattern");
        // new attribute on the target: untouched elements default to 0
        let mut s2 = pts(&[[0.0; 3]; 2]);
        s2.add_attr(mk("w", AttrClass::Point, AttrData::F32(vec![7.0, 8.0])));
        let o = copy(&grid3(), &Arc::new(s2), &[]).expect("cooks");
        assert_eq!(f32s(&o, AttrClass::Point, "w"), [7.0, 8.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }
    #[test]
    fn by_p_matches_positions_within_tolerance() {
        let mut s = pts(&[[2.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0000001, 0.0, 0.0]]);
        s.add_attr(mk("v", AttrClass::Point, AttrData::F32(vec![20.0, 0.5, 10.0, 99.0])));
        // target order differs from the source order and has one point with no partner
        let t = Arc::new(pts(&[[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [0.0, 0.0, 0.0]]));
        let o = copy(&t, &Arc::new(s), &[("matchby", menu(1))]).expect("cooks");
        // x=1 has two candidates within 1e-5; the exact one (idx 2, d=0) wins over idx 3
        assert_eq!(f32s(&o, AttrClass::Point, "v"), [10.0, 20.0, 0.0, 0.5]);
    }
    #[test]
    fn by_p_tolerance_gates() {
        let mut s = pts(&[[0.01, 0.0, 0.0]]);
        s.add_attr(mk("v", AttrClass::Point, AttrData::F32(vec![5.0])));
        let (t, s) = (Arc::new(pts(&[[0.0; 3]])), Arc::new(s));
        let strict = copy(&t, &s, &[("matchby", menu(1))]).expect("cooks");
        assert_eq!(f32s(&strict, AttrClass::Point, "v"), [0.0]);
        let loose = copy(&t, &s, &[("matchby", menu(1)), ("tolerance", ParamValue::F(0.02))]).expect("cooks");
        assert_eq!(f32s(&loose, AttrClass::Point, "v"), [5.0]);
    }
    #[test]
    fn classes_pattern_p_opt_in_and_strings() {
        let g = grid3();
        let mut s = (*g).clone();
        s.add_attr(mk("id", AttrClass::Prim, AttrData::F32(vec![1.0, 2.0, 3.0, 4.0])));
        s.add_attr(mk("info", AttrClass::Detail, AttrData::F32(vec![42.0])));
        s.add_attr(mk("cd", AttrClass::Point, AttrData::F32(vec![0.0; 9])));
        s.add_attr(mk(
            "tag",
            AttrClass::Prim,
            AttrData::Str { idx: vec![0, 1, 0, 1], table: vec!["a".into(), "b".into()] },
        ));
        for q in s.p_mut().iter_mut() {
            q[1] += 5.0;
        }
        let s = Arc::new(s);
        let o = copy(&g, &s, &[("class", menu(2))]).expect("cooks");
        assert_eq!(f32s(&o, AttrClass::Prim, "id"), [1.0, 2.0, 3.0, 4.0]);
        assert!(o.attr(AttrClass::Prim, "tag").is_some() && o.attr(AttrClass::Point, "cd").is_none());
        let o = copy(&g, &s, &[("class", menu(2)), ("attribs", ParamValue::S("id".into()))]).expect("cooks");
        assert!(o.attr(AttrClass::Prim, "tag").is_none());
        let o = copy(&g, &s, &[("class", menu(3))]).expect("cooks");
        assert_eq!(f32s(&o, AttrClass::Detail, "info"), [42.0]);
        // P is opt-in and moves the geometry
        let o = copy(&g, &s, &[("attribs", ParamValue::S("P".into()))]).expect("cooks");
        assert_eq!(o.p(), s.p());
        // nothing selected -> input shared
        let o = copy(&g, &s, &[("attribs", ParamValue::S("zz*".into()))]).expect("cooks");
        assert!(Arc::ptr_eq(&o, &g));
    }
    #[test]
    fn empty_source_and_hostile_params() {
        let g = grid3();
        let empty = Arc::new(Detail::default());
        assert!(Arc::ptr_eq(&copy(&g, &empty, &[]).expect("cooks"), &g));
        assert_eq!(copy(&empty, &g, &[]).expect("cooks").point_count, 0);
        for (k, v) in [
            ("class", menu(5)),
            ("matchby", menu(2)),
            ("tolerance", ParamValue::F(-1.0)),
            ("attribs", ParamValue::I(0)),
        ] {
            assert!(matches!(copy(&g, &g, &[(k, v)]), Err(CookError::BadParam(_))), "{k}");
        }
        // P match is point-only
        assert!(matches!(copy(&g, &g, &[("matchby", menu(1)), ("class", menu(2))]), Err(CookError::BadParam(_))));
    }
}
