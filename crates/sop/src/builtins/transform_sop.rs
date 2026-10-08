//! Transform SOP (Houdini internal name `xform`, registered token `xform`): moves `P` (and `N`) of input 0 by one
//! matrix built from translate / rotate / scale / shear / pivot.
//!
//! # Convention (UNVERIFIED vs Houdini: hython is blocked, nothing here was observed from a cook)
//! Houdini documents row vectors: `p' = p · M`, a product `A·B·C` applies `A` first. glam is column-vector
//! (`p' = M · p`), so a row product `A·B·C` is built as glam `C * B * A` (see `chain`). Everything below is written in
//! APPLY ORDER (first applied first).
//! ```text
//! M = T(-p) · X1 · X2 · X3 · T(p)            X1..X3 = the three groups below in `xOrd` order
//!   S  = scale(s * scale) then Sh          (non-uniform `s` times uniform `scale`; Sh = shear)
//!   R  = Ra · Rb · Rc                      (`rOrd`: "xyz" = Rx first, then Ry, then Rz; degrees; right-handed)
//!   Tr = translate(t)
//! xOrd srt (default): M = T(-p)·S·Sh·R·T(t)·T(p)  ==  T(-p)·S·Sh·R·T(p)·T(t)  (translations commute)
//! ```
//! Shear form: `x' = x + xy*y + xz*z ; y' = y + yz*z ; z' = z` with `shear = (xy, xz, yz)`. Which component is which in
//! Houdini is UNVERIFIED. `invertxform` applies `M^-1` (singular `M` → `Err`).
//!
//! # Normals
//! `N` (point class AND vertex class, `V3` only) is transformed by the inverse-transpose of the 3x3 part of `M`. With
//! `updatenmls` off (default) the length of each normal is kept (Houdini's `vlength` default); with `updatenmls` on the
//! result is unit length. Zero-length normals stay zero. A singular 3x3 with `N` present → `Err`. Every other attribute
//! is untouched; `Sphere`/`Tube` primitive transforms are NOT updated.
//!
//! # Scope
//! One input (clone of input 0). Supported params: `group` (a point-group NAME; `grouptype` guess/points only) `grouptype`
//! `xOrd` `rOrd` `t` `r` `s` `shear` `scale` `p` `pr` (must be 0) `updatenmls` `invertxform`. Not registered: `prexform_*`,
//! `movecentroid`, `movepivot`, `attribs`, `updateaffectednmls`, `vlength`, `addattrib`/`outputattrib`/`outputmerge`.
//! Hostile params (non-finite, bad menu index, `pr` ≠ 0, f32 overflow of the result) are `Err`; zero scale is allowed.
use crate::SopNode;
use glam::{DMat3, DMat4, DVec3, DVec4};
use hexe_geo::{AttrClass, AttrData, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

/// `xOrd` menu tokens (index = menu value), apply order of the groups: s = scale+shear, r = rotate, t = translate.
const X_ORDERS: [&str; 6] = ["srt", "str", "rst", "rts", "tsr", "trs"];
/// `rOrd` menu tokens, apply order of the axis rotations.
const R_ORDERS: [&str; 6] = ["xyz", "xzy", "yxz", "yzx", "zxy", "zyx"];
/// `grouptype` menu: guess, breakpoints, edges, points, prims.
const GROUP_TYPES: usize = 5;
const GROUP_GUESS: usize = 0;
const GROUP_POINTS: usize = 3;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("grouptype", ParamValue::Menu(0)).label("Group Type"),
        ParamSpec::new("xOrd", ParamValue::Menu(0)).label("Transform Order"),
        ParamSpec::new("rOrd", ParamValue::Menu(0)).label("Rotate Order"),
        ParamSpec::new("t", ParamValue::V3([0.0; 3])).label("Translate").range(-1.0, 1.0),
        ParamSpec::new("r", ParamValue::V3([0.0; 3])).label("Rotate").range(0.0, 360.0),
        ParamSpec::new("s", ParamValue::V3([1.0; 3])).label("Scale").range(-1.0, 1.0),
        ParamSpec::new("shear", ParamValue::V3([0.0; 3])).label("Shear").range(0.0, 10.0),
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Uniform Scale").range(0.0, 10.0),
        ParamSpec::new("p", ParamValue::V3([0.0; 3])).label("Pivot Translate").range(-1.0, 1.0),
        ParamSpec::new("pr", ParamValue::V3([0.0; 3])).label("Pivot Rotate").range(0.0, 360.0),
        ParamSpec::new("updatenmls", ParamValue::B(false)).label("Recompute Point Normals"),
        ParamSpec::new("invertxform", ParamValue::B(false)).label("Invert Transformation"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Transform { specs: spec() })
}

struct Transform {
    specs: Vec<ParamSpec>,
}

fn bad<T>(m: impl Into<String>) -> Result<T, CookError> {
    Err(CookError::BadParam(m.into()))
}

fn need_vec3(p: &ParamSet, name: &str) -> Result<[f32; 3], CookError> {
    let v = p.vec3(name).ok_or_else(|| CookError::BadParam(format!("`{name}`: missing or not a vector")))?;
    if v.iter().all(|c| c.is_finite()) { Ok(v) } else { bad(format!("`{name}`: non-finite component")) }
}

fn need_menu(p: &ParamSet, name: &str, len: usize) -> Result<usize, CookError> {
    let m = p.menu(name).ok_or_else(|| CookError::BadParam(format!("`{name}`: missing or not a menu")))?;
    match usize::try_from(m) {
        Ok(i) if i < len => Ok(i),
        _ => bad(format!("`{name}`: menu index {m} out of range 0..{len}")),
    }
}

fn need_toggle(p: &ParamSet, name: &str) -> Result<bool, CookError> {
    p.toggle(name).ok_or_else(|| CookError::BadParam(format!("`{name}`: missing or not a toggle")))
}

fn dvec(v: [f32; 3]) -> DVec3 {
    DVec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]))
}

/// Row-vector product in APPLY ORDER (`[A, B, C]` = Houdini `A·B·C`, A first) as a glam column matrix (`C·B·A`).
fn chain(apply_order: &[DMat4]) -> DMat4 {
    apply_order.iter().fold(DMat4::IDENTITY, |acc, m| *m * acc)
}

/// Shear `x' = x + xy*y + xz*z ; y' = y + yz*z` for `shear = (xy, xz, yz)`.
fn shear_mat(sh: DVec3) -> DMat4 {
    DMat4::from_cols(
        DVec4::new(1.0, 0.0, 0.0, 0.0),
        DVec4::new(sh.x, 1.0, 0.0, 0.0),
        DVec4::new(sh.y, sh.z, 1.0, 0.0),
        DVec4::new(0.0, 0.0, 0.0, 1.0),
    )
}

/// The parameters resolved and validated for one cook.
struct Config {
    matrix: DMat4,
    update_nmls: bool,
    group: Option<String>,
}

impl Config {
    fn read(p: &ParamSet) -> Result<Config, CookError> {
        let (t, r, s, sh) = (need_vec3(p, "t")?, need_vec3(p, "r")?, need_vec3(p, "s")?, need_vec3(p, "shear")?);
        let (pv, pr) = (need_vec3(p, "p")?, need_vec3(p, "pr")?);
        let uni = p.float("scale").ok_or_else(|| CookError::BadParam("`scale`: missing or not a float".into()))?;
        if !uni.is_finite() {
            return bad("`scale`: non-finite");
        }
        if pr != [0.0; 3] {
            return bad("`pr` (pivot rotate) is not implemented: must be 0 0 0");
        }
        let xord = need_menu(p, "xOrd", X_ORDERS.len())?;
        let rord = need_menu(p, "rOrd", R_ORDERS.len())?;
        let gtype = need_menu(p, "grouptype", GROUP_TYPES)?;
        let group = p.string("group").ok_or_else(|| CookError::BadParam("`group`: missing or not a string".into()))?;
        let group = group.trim();
        let group = (!group.is_empty()).then(|| group.to_string());
        if group.is_some() && gtype != GROUP_GUESS && gtype != GROUP_POINTS {
            return bad("`grouptype`: only guess/points is implemented");
        }
        let update_nmls = need_toggle(p, "updatenmls")?;
        let invert = need_toggle(p, "invertxform")?;

        // rotation: axis letters of `rOrd` in apply order
        let rad = dvec(r) * (std::f64::consts::PI / 180.0);
        let axis = |c: char| match c {
            'x' => DMat4::from_rotation_x(rad.x),
            'y' => DMat4::from_rotation_y(rad.y),
            _ => DMat4::from_rotation_z(rad.z),
        };
        let rot: Vec<DMat4> = R_ORDERS.get(rord).copied().unwrap_or("xyz").chars().map(axis).collect();
        let rot = chain(&rot);
        let scl = chain(&[DMat4::from_scale(dvec(s) * f64::from(uni)), shear_mat(dvec(sh))]);
        let trn = DMat4::from_translation(dvec(t));
        let groups: Vec<DMat4> = X_ORDERS
            .get(xord)
            .copied()
            .unwrap_or("srt")
            .chars()
            .map(|c| match c {
                's' => scl,
                'r' => rot,
                _ => trn,
            })
            .collect();
        let mut apply = vec![DMat4::from_translation(-dvec(pv))];
        apply.extend(groups);
        apply.push(DMat4::from_translation(dvec(pv)));
        let mut matrix = chain(&apply);
        if invert {
            let det = matrix.determinant();
            if !det.is_finite() || det == 0.0 {
                return Err(CookError::Failed("invertxform: transform matrix is singular".into()));
            }
            matrix = matrix.inverse();
        }
        if !matrix.to_cols_array().iter().all(|c| c.is_finite()) {
            return Err(CookError::Failed("transform matrix is not finite".into()));
        }
        Ok(Config { matrix, update_nmls, group })
    }
}

fn to_f32(v: DVec3) -> [f32; 3] {
    [v.x as f32, v.y as f32, v.z as f32]
}

/// Transform one position; `Err` if a finite input overflows f32.
fn xform_point(m: &DMat4, p: [f32; 3]) -> Result<[f32; 3], CookError> {
    let out = to_f32(m.transform_point3(dvec(p)));
    if out.iter().all(|c| c.is_finite()) || !p.iter().all(|c| c.is_finite()) {
        Ok(out)
    } else {
        Err(CookError::Failed("transformed position overflows f32".into()))
    }
}

/// Inverse-transpose of the 3x3 part of `m` applied to `n`; length kept unless `unit`.
fn xform_normal(it: &DMat3, n: [f32; 3], unit: bool) -> [f32; 3] {
    let n64 = dvec(n);
    let len = n64.length();
    if len == 0.0 || !len.is_finite() {
        return n;
    }
    let o = *it * n64;
    let ol = o.length();
    if ol == 0.0 || !ol.is_finite() {
        return n;
    }
    to_f32(o / ol * if unit { 1.0 } else { len })
}

impl Node<Detail> for Transform {
    fn type_name(&self) -> &'static str {
        "xform"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let cfg = Config::read(ctx.params)?;
        let mut d: Detail = inputs.first().map(|a| (**a).clone()).unwrap_or_default();
        let mask: Option<Vec<bool>> = match &cfg.group {
            None => None,
            Some(name) => match d.group(AttrClass::Point, name) {
                Some(g) => Some(g.mask.clone()),
                None => return bad(format!("`group`: no point group named `{name}`")),
            },
        };
        let in_group = |pt: usize| mask.as_ref().is_none_or(|m| m.get(pt).copied().unwrap_or(false));
        let m = cfg.matrix;
        if let Some(AttrData::V3(p)) = d.attr_mut(AttrClass::Point, "P").map(|a| &mut a.data) {
            for (i, v) in p.iter_mut().enumerate() {
                if in_group(i) {
                    *v = xform_point(&m, *v)?;
                }
            }
        }
        // inverse-transpose of the 3x3 part; only needed (and only required invertible) when an `N` exists
        let has_n = d.attrs.iter().any(|a| a.name == "N" && matches!(a.data, AttrData::V3(_)));
        if has_n {
            let m3 = DMat3::from_cols(m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate());
            let det = m3.determinant();
            if !det.is_finite() || det == 0.0 {
                return Err(CookError::Failed("N present but the transform's 3x3 part is singular".into()));
            }
            let it = m3.inverse().transpose();
            let vertex_point = &d.vertex_point;
            for a in d.attrs.iter_mut().filter(|a| a.name == "N") {
                match (a.class, &mut a.data) {
                    (AttrClass::Point, AttrData::V3(n)) => {
                        for (i, v) in n.iter_mut().enumerate() {
                            if in_group(i) {
                                *v = xform_normal(&it, *v, cfg.update_nmls);
                            }
                        }
                    }
                    (AttrClass::Vertex, AttrData::V3(n)) => {
                        for (i, v) in n.iter_mut().enumerate() {
                            let pt = vertex_point.get(i).map_or(0, |p| *p as usize);
                            if in_group(pt) {
                                *v = xform_normal(&it, *v, cfg.update_nmls);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Arc::new(d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hexe_geo::{Attribute, Group, TypeInfo, read_geo, write_geo_json};

    const EPS: f32 = 1e-5;

    fn ctx(p: &ParamSet) -> CookCtx<'_> {
        CookCtx { time: 0.0, fps: 24.0, params: p, log: &|_| {} }
    }
    /// Spec defaults overridden by `over`.
    fn params(over: &[(&str, ParamValue)]) -> ParamSet {
        let mut p = ParamSet::new();
        for s in spec() {
            p = p.with(&s.name, s.default.clone());
        }
        for (k, v) in over {
            p = p.with(k, v.clone());
        }
        p
    }
    fn run(d: &Detail, over: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        let p = params(over);
        new().cook(&ctx(&p), &[Arc::new(d.clone())])
    }
    fn v3(a: f32, b: f32, c: f32) -> ParamValue {
        ParamValue::V3([a, b, c])
    }
    /// Corners (±1,±1,±1) + offset (0.5,0,0) in the BOX SOP's Houdini order (bottom ring, top ring; ring = -x-z, +x-z, +x+z, -x+z).
    fn corners() -> Vec<[f32; 3]> {
        let mut v = Vec::new();
        for y in [-1.0f32, 1.0] {
            for (x, z) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                v.push([x + 0.5, y, z]);
            }
        }
        v
    }
    /// Hand-built 8-point box with 6 quads.
    fn box_detail() -> Detail {
        let mut d = Detail::default();
        let r = d.add_points(8);
        assert_eq!(r, 0..8);
        *d.p_mut() = corners();
        for q in [[1, 5, 4, 0], [2, 6, 5, 1], [3, 7, 6, 2], [0, 4, 7, 3], [2, 1, 0, 3], [5, 6, 7, 4]] {
            d.add_poly(&q, true);
        }
        d
    }
    fn assert_close(got: &[[f32; 3]], want: &[[f32; 3]]) {
        assert_eq!(got.len(), want.len());
        for (g, w) in got.iter().zip(want) {
            for k in 0..3 {
                assert!((g[k] - w[k]).abs() <= EPS, "got {g:?} want {w:?}");
            }
        }
    }
    fn mapped(f: impl Fn([f32; 3]) -> [f32; 3]) -> Vec<[f32; 3]> {
        corners().into_iter().map(f).collect()
    }
    fn add_n(d: &mut Detail, class: AttrClass, n: Vec<[f32; 3]>) {
        d.add_attr(Attribute { name: "N".into(), class, data: AttrData::V3(n), type_info: TypeInfo::Normal });
    }
    fn normals(d: &Detail, class: AttrClass) -> Vec<[f32; 3]> {
        match d.attr(class, "N").map(|a| &a.data) {
            Some(AttrData::V3(n)) => n.clone(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn translate_box_adds_t_and_keeps_topology() {
        let src = box_detail();
        let out = run(&src, &[("t", v3(1.0, -2.0, 3.5))]).expect("cooks");
        assert_close(out.p(), &mapped(|p| [p[0] + 1.0, p[1] - 2.0, p[2] + 3.5]));
        assert_eq!((out.point_count, out.vertex_count(), out.prim_count()), (8, 24, 6));
        assert_eq!((out.vertex_point.clone(), out.prims.clone()), (src.vertex_point, src.prims));
    }

    #[test]
    fn rotate_90_about_y_maps_x_to_minus_z() {
        let mut d = Detail::default();
        d.add_points(3);
        *d.p_mut() = vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let out = run(&d, &[("r", v3(0.0, 90.0, 0.0))]).expect("cooks");
        assert_close(out.p(), &[[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]);
        let out = run(&box_detail(), &[("r", v3(0.0, 90.0, 0.0))]).expect("cooks");
        assert_close(out.p(), &mapped(|p| [p[2], p[1], -p[0]]));
    }

    #[test]
    fn scale_about_pivot_and_uniform_scale() {
        let out =
            run(&box_detail(), &[("s", v3(2.0, 3.0, 4.0)), ("scale", ParamValue::F(0.5)), ("p", v3(1.0, 1.0, 1.0))])
                .expect("cooks");
        // p + (P - p) * s * scale
        let k = [1.0, 1.5, 2.0];
        assert_close(
            out.p(),
            &mapped(|q| [1.0 + (q[0] - 1.0) * k[0], 1.0 + (q[1] - 1.0) * k[1], 1.0 + (q[2] - 1.0) * k[2]]),
        );
    }

    #[test]
    fn transform_order_matters_closed_form() {
        let mut d = Detail::default();
        d.add_points(1);
        let t = [("t", v3(1.0, 0.0, 0.0)), ("s", v3(2.0, 2.0, 2.0)), ("r", v3(0.0, 90.0, 0.0))];
        // srt: scale 0 = 0, rotate = 0, +t = (1,0,0)
        let mut o = t.to_vec();
        o.push(("xOrd", ParamValue::Menu(0)));
        assert_close(run(&d, &o).expect("srt").p(), &[[1.0, 0.0, 0.0]]);
        // trs: +t = (1,0,0), Ry90 = (0,0,-1), *2 = (0,0,-2)
        let mut o = t.to_vec();
        o.push(("xOrd", ParamValue::Menu(5)));
        assert_close(run(&d, &o).expect("trs").p(), &[[0.0, 0.0, -2.0]]);
    }

    #[test]
    fn rotate_order_matters_closed_form() {
        let mut d = Detail::default();
        d.add_points(1);
        *d.p_mut() = vec![[0.0, 1.0, 0.0]];
        let r = ("r", v3(90.0, 90.0, 0.0));
        // xyz: Rx90 (0,1,0)->(0,0,1); Ry90 ->(1,0,0)
        assert_close(run(&d, &[r.clone(), ("rOrd", ParamValue::Menu(0))]).expect("xyz").p(), &[[1.0, 0.0, 0.0]]);
        // yxz: Ry90 leaves (0,1,0); Rx90 -> (0,0,1)
        assert_close(run(&d, &[r, ("rOrd", ParamValue::Menu(2))]).expect("yxz").p(), &[[0.0, 0.0, 1.0]]);
    }

    #[test]
    fn shear_closed_form_documented_convention() {
        let mut d = Detail::default();
        d.add_points(1);
        *d.p_mut() = vec![[0.0, 1.0, 2.0]];
        // shear=(xy,xz,yz)=(1,10,100): x' = 0 + 1*1 + 10*2 = 21 ; y' = 1 + 100*2 = 201 ; z' = 2
        let out = run(&d, &[("shear", v3(1.0, 10.0, 100.0))]).expect("cooks");
        assert_close(out.p(), &[[21.0, 201.0, 2.0]]);
    }

    #[test]
    fn identity_params_keep_p_bit_identical() {
        let src = box_detail();
        let out = run(&src, &[]).expect("cooks");
        assert_eq!(out.p(), src.p());
        assert_eq!(*out, src);
    }

    #[test]
    fn invert_round_trips_a_composed_transform() {
        let src = box_detail();
        let fwd = [
            ("t", v3(0.3, -1.0, 2.0)),
            ("r", v3(20.0, 35.0, -50.0)),
            ("s", v3(1.5, 0.7, 2.0)),
            ("shear", v3(0.1, 0.2, 0.3)),
            ("p", v3(0.5, 0.25, -1.0)),
            ("xOrd", ParamValue::Menu(2)),
            ("rOrd", ParamValue::Menu(4)),
        ];
        let mid = run(&src, &fwd).expect("forward");
        assert!(mid.p().iter().zip(src.p()).any(|(a, b)| (a[0] - b[0]).abs() > 0.1), "forward moved P");
        let mut inv = fwd.to_vec();
        inv.push(("invertxform", ParamValue::B(true)));
        let back = run(&mid, &inv).expect("inverse");
        assert_close(back.p(), src.p());
    }

    #[test]
    fn normals_use_inverse_transpose() {
        // plane normal (1,1,0) under s=(2,1,1): inverse-transpose = diag(1/2,1,1) -> direction (0.5,1,0)
        let mut d = box_detail();
        add_n(&mut d, AttrClass::Point, vec![[1.0, 1.0, 0.0]; 8]);
        add_n(&mut d, AttrClass::Vertex, vec![[0.0, 0.0, 2.0]; 24]);
        let s = ("s", v3(2.0, 1.0, 1.0));
        let u = (0.25f32 + 1.0).sqrt();
        // updatenmls off: direction transformed, original length (sqrt 2) kept
        let out = run(&d, std::slice::from_ref(&s)).expect("cooks");
        let k = 2.0f32.sqrt() / u;
        assert_close(&normals(&out, AttrClass::Point), &[[0.5 * k, k, 0.0]; 8]);
        assert_close(&normals(&out, AttrClass::Vertex), &[[0.0, 0.0, 2.0]; 24]);
        // updatenmls on: unit length
        let out = run(&d, &[s, ("updatenmls", ParamValue::B(true))]).expect("cooks");
        assert_close(&normals(&out, AttrClass::Point), &[[0.5 / u, 1.0 / u, 0.0]; 8]);
    }

    #[test]
    fn other_attributes_are_untouched() {
        let mut d = box_detail();
        d.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3(vec![[0.1, 0.2, 0.3]; 8]),
            type_info: TypeInfo::Color,
        });
        let out = run(&d, &[("t", v3(5.0, 5.0, 5.0)), ("s", v3(3.0, 3.0, 3.0))]).expect("cooks");
        assert_eq!(out.attr(AttrClass::Point, "Cd"), d.attr(AttrClass::Point, "Cd"));
    }

    #[test]
    fn group_limits_the_transform_and_unknown_group_errs() {
        let mut d = box_detail();
        d.groups.push(Group {
            name: "top".into(),
            class: AttrClass::Point,
            mask: corners().iter().map(|p| p[1] > 0.0).collect(),
        });
        let out = run(&d, &[("t", v3(0.0, 10.0, 0.0)), ("group", ParamValue::S("top".into()))]).expect("cooks");
        assert_close(out.p(), &mapped(|p| if p[1] > 0.0 { [p[0], p[1] + 10.0, p[2]] } else { p }));
        assert!(matches!(run(&d, &[("group", ParamValue::S("nope".into()))]), Err(CookError::BadParam(_))));
        assert!(matches!(
            run(&d, &[("group", ParamValue::S("top".into())), ("grouptype", ParamValue::Menu(4))]),
            Err(CookError::BadParam(_))
        ));
    }

    #[test]
    fn empty_input_and_no_input_cook_to_empty() {
        let p = params(&[("t", v3(1.0, 2.0, 3.0))]);
        assert_eq!(*new().cook(&ctx(&p), &[Arc::new(Detail::default())]).expect("empty"), Detail::default());
        assert_eq!(*new().cook(&ctx(&p), &[]).expect("none"), Detail::default());
    }

    #[test]
    fn hostile_params_are_errors() {
        let d = box_detail();
        for (k, v) in [
            ("t", v3(f32::NAN, 0.0, 0.0)),
            ("r", v3(0.0, f32::INFINITY, 0.0)),
            ("s", v3(0.0, 0.0, f32::NEG_INFINITY)),
            ("shear", v3(f32::NAN, 0.0, 0.0)),
            ("p", v3(f32::INFINITY, 0.0, 0.0)),
            ("scale", ParamValue::F(f32::NAN)),
            ("xOrd", ParamValue::Menu(6)),
            ("rOrd", ParamValue::Menu(u32::MAX)),
            ("grouptype", ParamValue::Menu(9)),
            ("pr", v3(0.0, 45.0, 0.0)),
            ("t", ParamValue::F(1.0)),
        ] {
            assert!(matches!(run(&d, &[(k, v.clone())]), Err(CookError::BadParam(_))), "{k} {v:?}");
        }
        // missing parameter entirely
        let empty = ParamSet::new();
        assert!(matches!(new().cook(&ctx(&empty), &[Arc::new(d.clone())]), Err(CookError::BadParam(_))));
        // f32 overflow of the result
        let big = run(&d, &[("t", v3(3.4e38, 0.0, 0.0)), ("scale", ParamValue::F(3.0e38))]);
        assert!(matches!(big, Err(CookError::Failed(_))));
    }

    #[test]
    fn zero_scale_is_allowed_but_not_invertible() {
        let d = box_detail();
        let out = run(&d, &[("scale", ParamValue::F(0.0))]).expect("collapse");
        assert_close(out.p(), &[[0.0; 3]; 8]);
        let inv = run(&d, &[("scale", ParamValue::F(0.0)), ("invertxform", ParamValue::B(true))]);
        assert!(matches!(inv, Err(CookError::Failed(_))));
        // normals through a singular matrix cannot be transformed
        let mut n = box_detail();
        add_n(&mut n, AttrClass::Point, vec![[0.0, 1.0, 0.0]; 8]);
        assert!(matches!(run(&n, &[("scale", ParamValue::F(0.0))]), Err(CookError::Failed(_))));
    }

    #[test]
    fn scene_loader_sets_params_by_token_and_rejects_hostile_json() {
        let ok = r#"{"nodes":[{"name":"x","type":"xform","params":{"t":[1,2,3],"r":[0,90,0],"xOrd":5,"rOrd":2,
            "scale":2,"updatenmls":true,"invertxform":false,"group":"","grouptype":3}}],"display":"x"}"#;
        let (mut g, id) = crate::load_scene(ok).expect("loads");
        assert_eq!(*g.cook(id, 0.0).expect("cooks"), Detail::default());
        for bad in [
            r#"{"nodes":[{"name":"x","type":"xform","params":{"t":[1e999,0,0]}}],"display":"x"}"#,
            r#"{"nodes":[{"name":"x","type":"xform","params":{"t":[1,0]}}],"display":"x"}"#,
            r#"{"nodes":[{"name":"x","type":"xform","params":{"xOrd":-1}}],"display":"x"}"#,
            r#"{"nodes":[{"name":"x","type":"xform","params":{"bogus":1}}],"display":"x"}"#,
        ] {
            assert!(crate::load_scene(bad).is_err(), "{bad}");
        }
        assert_eq!(crate::registry().create("xform").map(|n| n.type_name()), Some("xform"));
    }

    fn fixture_dir(case: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(case)
    }
    /// Oracle fixtures: scene = box(default) -> xform. Expected is INDEPENDENT closed-form maths on the box lane's
    /// documented point order (x-major, z fastest, low->high; first = (-x,-y,-z)) and its 6 quads.
    const FIXTURES: [(&str, &str); 2] = [
        (
            "xform-translate",
            r#"{
  "nodes": [
    { "name": "src", "type": "box" },
    { "name": "xform1", "type": "xform", "params": { "t": [1, 2, 3] } }
  ],
  "edges": [["src", 0, "xform1", 0]],
  "display": "xform1"
}
"#,
        ),
        (
            "xform-rot90y",
            r#"{
  "nodes": [
    { "name": "src", "type": "box" },
    { "name": "xform1", "type": "xform", "params": { "r": [0, 90, 0] } }
  ],
  "edges": [["src", 0, "xform1", 0]],
  "display": "xform1"
}
"#,
        ),
    ];

    /// Closed-form expected for a fixture case: the 8 unit-box corners (+-0.5) mapped analytically.
    fn expected_detail(case: &str) -> Detail {
        let map: fn([f32; 3]) -> [f32; 3] = match case {
            "xform-translate" => |p| [p[0] + 1.0, p[1] + 2.0, p[2] + 3.0],
            // Ry(+90 deg): (x,y,z) -> (z,y,-x)
            "xform-rot90y" => |p| [p[2], p[1], -p[0]],
            other => panic!("unknown case {other}"),
        };
        let mut d = Detail::default();
        d.add_points(8);
        let mut pts = Vec::new();
        // Houdini box order: bottom ring then top ring, ring = -x-z, +x-z, +x+z, -x+z
        for y in [-0.5f32, 0.5] {
            for (x, z) in [(-0.5f32, -0.5f32), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
                pts.push(map([x, y, z]));
            }
        }
        *d.p_mut() = pts;
        for q in [[1, 5, 4, 0], [2, 6, 5, 1], [3, 7, 6, 2], [0, 4, 7, 3], [2, 1, 0, 3], [5, 6, 7, 4]] {
            d.add_poly(&q, true);
        }
        d
    }

    #[test]
    fn fixtures_cook_to_committed_expected_geo() {
        for (case, _) in FIXTURES {
            let dir = fixture_dir(case);
            let scene = std::fs::read_to_string(dir.join("scene.json")).expect("scene.json");
            let (mut g, id) = crate::load_scene(&scene).expect("loads");
            let got = g.cook(id, 0.0).expect("cooks");
            let want = read_geo(&std::fs::read(dir.join("expected.geo")).expect("expected.geo")).expect("parses");
            let got = read_geo(&write_geo_json(&got)).expect("round trip");
            assert_eq!((got.point_count, got.vertex_count(), got.prim_count()), (8, 24, 6), "{case}");
            assert_close(got.p(), want.p());
            // committed file == independent closed form
            assert_close(want.p(), expected_detail(case).p());
        }
    }

    /// Geometry through the same writer/reader the CLI uses: box in -> closed-form out.
    #[test]
    fn geo_json_round_trip_of_transformed_box_matches_closed_form() {
        let moved = run(&box_detail(), &[("t", v3(1.0, 2.0, 3.0))]).expect("cooks");
        let back = read_geo(&write_geo_json(&moved)).expect("round trip");
        assert_close(back.p(), &mapped(|p| [p[0] + 1.0, p[1] + 2.0, p[2] + 3.0]));
    }

    /// `cargo test -p hexe-sop -- --ignored write_fixture` rewrites scene.json + expected.geo (expected = closed form,
    /// independent of the SOP).
    #[test]
    #[ignore = "writes fixture files"]
    fn write_fixture() {
        for (case, scene) in FIXTURES {
            let dir = fixture_dir(case);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join("scene.json"), scene).expect("scene");
            std::fs::write(dir.join("expected.geo"), write_geo_json(&expected_detail(case))).expect("geo");
        }
    }
}
