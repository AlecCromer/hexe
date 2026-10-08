//! Attribute Transfer SOP (token `attribtransfer`): nearest-point attribute transfer from input 1 onto input 0's points.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Every point of input 0 finds the nearest point of input 1 (brute force on `P`, ties -> lowest source index). If that
//! distance is `<= distancethreshold` (default 1) the point receives the source point's value of every POINT attribute selected
//! by `pointattribs` (pattern grammar: `attribs.rs` header; default `* ^P`, so positions stay). Points farther than the
//! threshold keep their existing value (default 0 / unset string if the attribute is new). Attribute shape mismatch with an
//! existing target attribute -> replaced by the source's shape.
//!
//! `kernelradius` (default 0 = off, nearest only): when `> 0`, numeric attributes become the weighted average of ALL source
//! points closer than the radius with linear falloff `w = 1 - d/radius`; no source inside the radius -> nearest value.
//! Strings always take the nearest. Pair cost `n*m` is capped (`Failed` beyond 2^26; with blending 2^24 stored weights).
//! Not modelled: vertex / prim / detail transfer, `lod`, `maxsamplecount`, kernel shapes other than linear, prim-surface
//! (non-point) nearest. Absent / empty source or target -> input 0 shared.
use super::attribs::{base_for, blend_into, copy_elem, is_numeric, selected, set_attr, str_param};
use super::group_sop::{bad, failed};
use crate::SopNode;
use hexe_geo::{AttrClass, Attribute, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const WHO: &str = "attribtransfer";
const MAX_PAIRS: usize = 1 << 26;
const MAX_WEIGHTS: usize = 1 << 24;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("pointattribs", ParamValue::S("* ^P".into())).label("Point Attributes"),
        ParamSpec::new("distancethreshold", ParamValue::F(1.0)).label("Distance Threshold").range(0.0, 10.0),
        ParamSpec::new("kernelradius", ParamValue::F(0.0)).label("Kernel Radius").range(0.0, 10.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(AttribTransfer { specs: spec() })
}
struct AttribTransfer {
    specs: Vec<ParamSpec>,
}
fn nonneg(p: &hexe_graph::ParamSet, name: &str) -> Result<f64, CookError> {
    match p.float(name) {
        Some(v) if v.is_finite() && v >= 0.0 => Ok(f64::from(v)),
        _ => Err(bad(WHO, format!("`{name}` missing, not a float, or negative"))),
    }
}
impl Node<Detail> for AttribTransfer {
    fn type_name(&self) -> &'static str {
        "attribtransfer"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "target" }, InputSpec { name: "source" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let pat = str_param(p, "pointattribs", WHO)?;
        let (thr, rad) = (nonneg(p, "distancethreshold")?, nonneg(p, "kernelradius")?);
        let empty = Arc::new(Detail::default());
        let d = inputs.first().unwrap_or(&empty);
        let src = inputs.get(1).unwrap_or(&empty);
        let (tp, sp) = (d.p(), src.p());
        if tp.is_empty() || sp.is_empty() {
            return Ok(Arc::clone(d));
        }
        if tp.len().saturating_mul(sp.len()) > MAX_PAIRS {
            return Err(failed(WHO, "target x source point count exceeds the cap"));
        }
        let (thr2, blend) = (thr * thr, rad > 0.0);
        let mut near: Vec<Option<usize>> = Vec::with_capacity(tp.len());
        let mut weights: Vec<Vec<(usize, f64)>> = Vec::new();
        let mut stored = 0usize;
        for q in tp {
            let mut best: Option<(f64, usize)> = None;
            let mut w: Vec<(usize, f64)> = Vec::new();
            for (j, s) in sp.iter().enumerate() {
                let d2: f64 = (0..3).map(|k| (f64::from(q[k]) - f64::from(s[k])).powi(2)).sum();
                if best.is_none_or(|b| d2 < b.0) {
                    best = Some((d2, j));
                }
                if blend && d2 < rad * rad {
                    w.push((j, 1.0 - d2.sqrt() / rad));
                }
            }
            match best {
                Some((d2, j)) if d2 <= thr2 => {
                    near.push(Some(j));
                    stored += w.len();
                    if stored > MAX_WEIGHTS {
                        return Err(failed(WHO, "kernel weights exceed the cap"));
                    }
                    if blend {
                        weights.push(w);
                    }
                }
                _ => {
                    near.push(None);
                    if blend {
                        weights.push(Vec::new());
                    }
                }
            }
        }
        let mut out = (**d).clone();
        for a in src.attrs.iter().filter(|a| a.class == AttrClass::Point && selected(pat, &a.name)) {
            let mut data = base_for(&out, AttrClass::Point, &a.name, &a.data);
            for (i, n) in near.iter().enumerate() {
                let Some(j) = *n else { continue };
                let blended = match weights.get(i) {
                    Some(w) if !w.is_empty() && is_numeric(&a.data) => blend_into(&mut data, i, &a.data, w),
                    _ => false,
                };
                if !blended {
                    copy_elem(&mut data, i, &a.data, j);
                }
            }
            set_attr(&mut out, Attribute { name: a.name.clone(), class: AttrClass::Point, data, type_info: a.type_info });
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
    fn mk(name: &str, data: AttrData) -> Attribute {
        Attribute { name: name.into(), class: AttrClass::Point, data, type_info: TypeInfo::None }
    }
    /// Points-only detail from positions.
    fn pts(p: &[[f32; 3]]) -> Detail {
        let mut d = Detail::default();
        d.add_points(p.len() as u32);
        d.p_mut().copy_from_slice(p);
        d
    }
    fn xfer(t: &Arc<Detail>, src: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        let node = crate::registry().create("attribtransfer").expect("registered");
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
    fn f32s(d: &Detail, n: &str) -> Vec<f32> {
        match &d.attr(AttrClass::Point, n).expect("attr").data {
            AttrData::F32(v) => v.clone(),
            o => panic!("not f32: {o:?}"),
        }
    }
    fn f(v: f32) -> ParamValue {
        ParamValue::F(v)
    }
    /// Source grid shifted by +0.6 along both in-plane axes; `v = 10 * index`, `tag` = string per index.
    fn shifted_src() -> (Arc<Detail>, Arc<Detail>) {
        let t = grid3();
        let (lo, hi) = t.bbox();
        let axes: Vec<usize> = (0..3).filter(|&k| hi[k] > lo[k]).collect();
        assert_eq!(axes.len(), 2, "flat grid");
        let mut s = (*t).clone();
        for q in s.p_mut().iter_mut() {
            for &k in &axes {
                q[k] += 0.6;
            }
        }
        s.add_attr(mk("v", AttrData::F32((0..9).map(|i| 10.0 * i as f32).collect())));
        s.add_attr(mk("n", AttrData::I32((0..9).collect())));
        let tags: Vec<String> = (0..9).map(|i| format!("t{i}")).collect();
        s.add_attr(mk("tag", AttrData::Str { idx: (0..9).collect(), table: tags }));
        (t, Arc::new(s))
    }
    #[test]
    fn params_and_defaults() {
        let n = super::new();
        let got: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(got, ["pointattribs", "distancethreshold", "kernelradius"]);
        assert_eq!(n.params()[1].default, f(1.0));
        assert_eq!(n.params()[2].default, f(0.0), "blend off by default");
        assert_eq!(n.inputs().len(), 2);
    }
    #[test]
    fn offset_grid_nearest_mapping_by_hand() {
        // source columns sit at -0.4, 0.6, 1.6 against target -1, 0, 1 -> nearest column per target = [0, 0, 1]
        let (t, s) = shifted_src();
        let o = xfer(&t, &s, &[]).expect("cooks");
        let m = [0usize, 0, 1];
        let want: Vec<f32> = (0..9).map(|i| 10.0 * (3 * m[i / 3] + m[i % 3]) as f32).collect();
        assert_eq!(f32s(&o, "v"), want);
        assert_eq!(o.attr(AttrClass::Point, "n").expect("n").data, AttrData::I32((0..9).map(|i| (3 * m[i / 3] + m[i % 3]) as i32).collect()));
        match &o.attr(AttrClass::Point, "tag").expect("tag").data {
            AttrData::Str { idx, table } => {
                let got: Vec<&str> = idx.iter().map(|&k| table[k as usize].as_str()).collect();
                let want: Vec<String> = (0..9).map(|i| format!("t{}", 3 * m[i / 3] + m[i % 3])).collect();
                assert_eq!(got, want.iter().map(String::as_str).collect::<Vec<_>>());
            }
            x => panic!("{x:?}"),
        }
        assert_eq!(o.p(), t.p(), "P never transferred by the default pattern");
        assert_eq!((o.point_count, &o.prims), (t.point_count, &t.prims), "topology untouched");
    }
    #[test]
    fn distance_threshold_gates_points_and_keeps_existing_values() {
        let (t, s) = shifted_src();
        // distances: 4 points at 0.6*sqrt(2)... no: nearest d = sqrt(0.4^2+0.4^2) = 0.5657 for ids 4,5,7,8
        let o = xfer(&t, &s, &[("distancethreshold", f(0.6))]).expect("cooks");
        let v = f32s(&o, "v");
        let m = [0usize, 0, 1];
        for i in 0..9 {
            let inside = (i / 3 >= 1) && (i % 3 >= 1);
            let want = if inside { 10.0 * (3 * m[i / 3] + m[i % 3]) as f32 } else { 0.0 };
            assert_eq!(v[i], want, "point {i}");
        }
        assert_eq!(f32s(&xfer(&t, &s, &[("distancethreshold", f(0.5))]).expect("cooks"), "v"), vec![0.0; 9]);
        // existing target values survive outside the threshold
        let mut t2 = (*t).clone();
        t2.add_attr(mk("v", AttrData::F32(vec![-1.0; 9])));
        let o = xfer(&Arc::new(t2), &s, &[("distancethreshold", f(0.5))]).expect("cooks");
        assert_eq!(f32s(&o, "v"), vec![-1.0; 9]);
    }
    #[test]
    fn pattern_selects_attributes_and_p_is_opt_in() {
        let (t, s) = shifted_src();
        let o = xfer(&t, &s, &[("pointattribs", ParamValue::S("v tag".into()))]).expect("cooks");
        assert!(o.attr(AttrClass::Point, "v").is_some() && o.attr(AttrClass::Point, "tag").is_some());
        assert!(o.attr(AttrClass::Point, "n").is_none());
        let o = xfer(&t, &s, &[("pointattribs", ParamValue::S("* ^v ^tag".into()))]).expect("cooks");
        assert!(o.attr(AttrClass::Point, "n").is_some() && o.attr(AttrClass::Point, "v").is_none());
        let o = xfer(&t, &s, &[("pointattribs", ParamValue::S("P".into()))]).expect("cooks");
        let m = [0usize, 0, 1];
        let sp = s.p();
        for i in 0..9 {
            assert_eq!(o.p()[i], sp[3 * m[i / 3] + m[i % 3]], "P snapped to nearest source point {i}");
        }
        let o = xfer(&t, &s, &[("pointattribs", ParamValue::S(String::new()))]).expect("cooks");
        assert_eq!(*o, *t, "empty pattern transfers nothing");
    }
    #[test]
    fn kernel_blend_is_linear_falloff_and_off_by_default() {
        let src = {
            let mut d = pts(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
            d.add_attr(mk("v", AttrData::F32(vec![0.0, 10.0])));
            d.add_attr(mk("c", AttrData::V3(vec![[0.0; 3], [10.0, 20.0, 30.0]])));
            Arc::new(d)
        };
        let t = Arc::new(pts(&[[0.5, 0.0, 0.0], [0.25, 0.0, 0.0], [0.75, 0.0, 0.0], [5.0, 0.0, 0.0]]));
        // off: nearest (ties -> lowest source index)
        let o = xfer(&t, &src, &[("distancethreshold", f(2.0))]).expect("cooks");
        assert_eq!(f32s(&o, "v"), [0.0, 0.0, 10.0, 0.0], "point 3 is beyond the threshold");
        // radius 1: w = (1-d) -> x=0.5: (.5,.5) -> 5 ; x=0.25: (.75,.25) -> 2.5 ; x=0.75 -> 7.5
        let o = xfer(&t, &src, &[("distancethreshold", f(2.0)), ("kernelradius", f(1.0))]).expect("cooks");
        assert_eq!(f32s(&o, "v"), [5.0, 2.5, 7.5, 0.0]);
        match &o.attr(AttrClass::Point, "c").expect("c").data {
            AttrData::V3(c) => assert_eq!(c[0], [5.0, 10.0, 15.0]),
            x => panic!("{x:?}"),
        }
        // radius smaller than the nearest distance -> falls back to nearest
        let o = xfer(&t, &src, &[("distancethreshold", f(2.0)), ("kernelradius", f(0.1))]).expect("cooks");
        assert_eq!(f32s(&o, "v"), [0.0, 0.0, 10.0, 0.0]);
    }
    #[test]
    fn empty_inputs_and_hostile_params() {
        let (t, s) = shifted_src();
        let empty = Arc::new(Detail::default());
        assert!(Arc::ptr_eq(&xfer(&t, &empty, &[]).expect("cooks"), &t));
        assert_eq!(xfer(&empty, &s, &[]).expect("cooks").point_count, 0);
        let one = run("attribtransfer", &[], Some(&t)).expect("cooks");
        assert!(Arc::ptr_eq(&one, &t), "missing input 1 is empty");
        for (k, v) in [
            ("distancethreshold", f(-1.0)),
            ("kernelradius", f(-0.5)),
            ("kernelradius", ParamValue::S("x".into())),
            ("pointattribs", ParamValue::I(1)),
        ] {
            assert!(matches!(xfer(&t, &s, &[(k, v)]), Err(CookError::BadParam(_))), "{k}");
        }
    }
}
