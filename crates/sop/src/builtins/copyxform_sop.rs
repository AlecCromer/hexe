//! Copy and Transform SOP (Houdini internal name `copyxform`): `ncy` copies of input 0 (the original included), copy `i`
//! transformed by the transform parms scaled `i` times.
//!
//! # Semantics (recorded from Houdini 20.5 via hython, `oracle/houdini/copyxform/`)
//! * Output = the WHOLE input untouched (copy 0) followed by copies `1..ncy` of the selected piece, in copy order.
//!   `ncy <= 0` empties the geometry (attribute and group definitions kept, detail attributes kept); `ncy == 1` = input.
//! * Copy `i` uses ONE matrix built from the parms with the per-copy values `t*i`, `r*i`, `s^i` (per component),
//!   `shear*i`, `scale^i`; the pivot `p` and pivot rotate `pr` are fixed. (It is NOT the copy-1 matrix to the `i`-th power.)
//!   Row-vector apply order (Houdini `buildTransform`): `T(-p) · Rp^-1 · [xOrd groups] · Rp · T(p)` where `Rp` is `pr`
//!   built in xyz order (NOT `rOrd`); scale group = `scale(s * scale)` then shear (`x += sh0*y + sh1*z ; y += sh2*z`).
//! * `sourcegroup` (`sourcegrouptype` guess / prims / points; numbers, ranges, `*`, `!`, group names, globs):
//!   prims -> the copied piece is the selected prims (prim order) + every point they use (point order);
//!   points -> the selected points (point order) + every prim whose points are ALL selected. Guess: a prim pattern,
//!   else a point pattern. A pattern that names no group of the chosen class: Houdini warns and copies nothing (the output
//!   is the original only); same here (warning via the cook log). Reversed ranges (`2-1`) select `1..=2`.
//! * Every attribute and group follows its element (groups of a copy keep the source membership); detail attributes
//!   are kept once. Attributes transformed per copy by type info (V3/V4 data only; P always as a point):
//!   `point` -> full affine; `vector` -> 3x3 part; `normal` -> inverse transpose, length kept, flipped when the 3x3
//!   determinant is negative, untouched when singular; `quaternion` -> `r ⊗ q` with `r` the rotation of the polar
//!   decomposition of the 3x3 (of its negation when the determinant is negative), untouched when singular. Colors and
//!   untyped attributes are copied verbatim. Winding is never reversed (also for negative scales).
//! * `docopyattrib` + `copyattrib`: prim I32 attribute = copy number (selected prims of the original = 0, prims outside
//!   `sourcegroup` = -1); an existing
//!   prim attribute of that name (any type) is replaced. The name is sanitised like Houdini (`[^A-Za-z0-9_]` -> `_`,
//!   leading digit -> `_` prefix, with a warning); empty = none; `P` is refused with a warning.
//! * `newgroups`: prim groups `<newgroupprefix><i>` for `i in 0..ncy` (group 0 = the selected prims of the original, all
//!   when no `sourcegroup`); same-name prim groups are replaced. Empty prefix = none; an invalid group name (Houdini rule
//!   `[A-Za-z_][A-Za-z0-9_]*`) creates no groups and warns.
//! * Edge groups: an edge of the piece (both ends copied) is repeated in every copy (UNVERIFIED vs Houdini).
//!
//! # Scope / hostile input
//! `pack` = 1 is `Unsupported` (hexe-geo has no packed-geometry payload); `pivot` / `viewportlod` only matter for packed
//! output and are validated but unused. Copies of `Sphere` / `Tube` / `Volume` / packed / unknown prims are `Unsupported`.
//! Non-finite or wrongly-typed parms and bad menu indices are `BadParam`; an output above `MAX_ELEMS` points / vertices /
//! prims, more than `MAX_GROUPS` new groups, a non-finite copy matrix or an f32-overflowing result is `Failed`.
use super::group_sop::parse_pattern;
use super::meshutil::{bad, failed, float, gather, int, menu, toggle, v3};
use crate::SopNode;
use glam::{DMat3, DMat4, DQuat, DVec3, DVec4};
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, Group, Prim, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

const OP: &str = "copyxform";
/// Output allocation cap per element class (AGENTS.md section 4).
const MAX_ELEMS: u64 = 1 << 24;
/// Cap on `newgroups` groups and on their total mask size.
const MAX_GROUPS: u64 = 1 << 16;
const MAX_GROUP_CELLS: u64 = 1 << 26;
/// `xOrd` menu tokens (index = menu value), apply order of the groups: s = scale+shear, r = rotate, t = translate.
const X_ORDERS: [&str; 6] = ["srt", "str", "rst", "rts", "tsr", "trs"];
/// `rOrd` menu tokens, apply order of the axis rotations.
const R_ORDERS: [&str; 6] = ["xyz", "xzy", "yxz", "yzx", "zxy", "zyx"];
const GROUP_GUESS: u32 = 0;
const GROUP_PRIMS: u32 = 1;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("sourcegroup", ParamValue::S(String::new())).label("Source Group"),
        ParamSpec::new("sourcegrouptype", ParamValue::Menu(0)).label("Source Group Type").menu(&[
            ("guess", "Guess from Group"),
            ("prims", "Primitives"),
            ("points", "Points"),
        ]),
        ParamSpec::new("ncy", ParamValue::I(2)).label("Total Number").range(0.0, 10.0),
        ParamSpec::new("pack", ParamValue::B(false)).label("Pack and Instance"),
        ParamSpec::new("pivot", ParamValue::Menu(1))
            .label("Pivot Location")
            .menu(&[("origin", "Origin"), ("centroid", "Centroid")]),
        ParamSpec::new("viewportlod", ParamValue::Menu(0)).label("Display As").menu(&[
            ("full", "Full Geometry"),
            ("points", "Point Cloud"),
            ("box", "Bounding Box"),
            ("centroid", "Centroid"),
            ("hidden", "Hidden"),
        ]),
        ParamSpec::new("xOrd", ParamValue::Menu(0)).label("Transform Order").menu(&[
            ("srt", "Scale Rotate Translate"),
            ("str", "Scale Translate Rotate"),
            ("rst", "Rotate Scale Translate"),
            ("rts", "Rotate Translate Scale"),
            ("tsr", "Translate Scale Rotate"),
            ("trs", "Translate Rotate Scale"),
        ]),
        ParamSpec::new("rOrd", ParamValue::Menu(0)).label("Rotate Order").menu(&[
            ("xyz", "Rx Ry Rz"),
            ("xzy", "Rx Rz Ry"),
            ("yxz", "Ry Rx Rz"),
            ("yzx", "Ry Rz Rx"),
            ("zxy", "Rz Rx Ry"),
            ("zyx", "Rz Ry Rx"),
        ]),
        ParamSpec::new("t", ParamValue::V3([0.0; 3])).label("Translate").range(-1.0, 1.0),
        ParamSpec::new("r", ParamValue::V3([0.0; 3])).label("Rotate").range(0.0, 360.0),
        ParamSpec::new("s", ParamValue::V3([1.0; 3])).label("Scale").range(-1.0, 1.0),
        ParamSpec::new("shear", ParamValue::V3([0.0; 3])).label("Shear").range(0.0, 10.0),
        ParamSpec::new("scale", ParamValue::F(1.0)).label("Uniform Scale").range(0.0, 10.0),
        ParamSpec::new("p", ParamValue::V3([0.0; 3])).label("Pivot Translate").range(-1.0, 1.0),
        ParamSpec::new("pr", ParamValue::V3([0.0; 3])).label("Pivot Rotate").range(0.0, 360.0),
        ParamSpec::new("newgroups", ParamValue::B(false)).label("Create Output Groups"),
        ParamSpec::new("newgroupprefix", ParamValue::S("copyGroup".into())).label("Output Group Prefix"),
        ParamSpec::new("docopyattrib", ParamValue::B(false)).label("Create Copy Number Attribute"),
        ParamSpec::new("copyattrib", ParamValue::S("copynum".into())).label("Copy Number Attribute"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(CopyXform { specs: spec() })
}

struct CopyXform {
    specs: Vec<ParamSpec>,
}

fn string(p: &ParamSet, n: &str) -> Result<String, CookError> {
    p.string(n).map(|s| s.to_string()).ok_or_else(|| bad(format!("`{n}`: missing or not a string")))
}

fn need_menu(p: &ParamSet, n: &str, len: usize) -> Result<usize, CookError> {
    let m = menu(p, n)?;
    match usize::try_from(m) {
        Ok(i) if i < len => Ok(i),
        _ => Err(bad(format!("`{n}`: menu index {m} out of range 0..{len}"))),
    }
}

fn dvec(v: [f64; 3]) -> DVec3 {
    DVec3::new(v[0], v[1], v[2])
}

/// Row-vector product in APPLY ORDER (`[A, B, C]` = Houdini `A·B·C`, A first) as a glam column matrix (`C·B·A`).
fn chain(apply_order: &[DMat4]) -> DMat4 {
    apply_order.iter().fold(DMat4::IDENTITY, |acc, m| *m * acc)
}

/// Shear `x' = x + sh0*y + sh1*z ; y' = y + sh2*z` (Houdini `buildTransform` shear).
fn shear_mat(sh: DVec3) -> DMat4 {
    DMat4::from_cols(
        DVec4::new(1.0, 0.0, 0.0, 0.0),
        DVec4::new(sh.x, 1.0, 0.0, 0.0),
        DVec4::new(sh.y, sh.z, 1.0, 0.0),
        DVec4::new(0.0, 0.0, 0.0, 1.0),
    )
}

/// Rotation of `deg` (degrees) with the axes applied in `rOrd` order.
fn rotation(deg: DVec3, rord: usize) -> DMat4 {
    let rad = deg * (std::f64::consts::PI / 180.0);
    let axis = |c: char| match c {
        'x' => DMat4::from_rotation_x(rad.x),
        'y' => DMat4::from_rotation_y(rad.y),
        _ => DMat4::from_rotation_z(rad.z),
    };
    let rot: Vec<DMat4> = R_ORDERS.get(rord).copied().unwrap_or("xyz").chars().map(axis).collect();
    chain(&rot)
}

/// The validated transform parms (per copy values derive from these).
struct Xf {
    t: DVec3,
    r: DVec3,
    s: DVec3,
    sh: DVec3,
    scale: f64,
    p: DVec3,
    pr: DVec3,
    xord: usize,
    rord: usize,
}

impl Xf {
    /// The matrix of copy `i` (column-vector glam form).
    fn matrix(&self, i: u32) -> DMat4 {
        let k = f64::from(i);
        let e = i32::try_from(i).unwrap_or(i32::MAX);
        let svec = DVec3::new(self.s.x.powi(e), self.s.y.powi(e), self.s.z.powi(e)) * self.scale.powi(e);
        let scl = chain(&[DMat4::from_scale(svec), shear_mat(self.sh * k)]);
        let rot = rotation(self.r * k, self.rord);
        let trn = DMat4::from_translation(self.t * k);
        let mut apply = vec![DMat4::from_translation(-self.p)];
        // Houdini builds the pivot rotation in xyz order whatever `rOrd` is (hython-verified)
        let piv = rotation(self.pr, 0);
        apply.push(piv.inverse());
        for c in X_ORDERS.get(self.xord).copied().unwrap_or("srt").chars() {
            apply.push(match c {
                's' => scl,
                'r' => rot,
                _ => trn,
            });
        }
        apply.push(piv);
        apply.push(DMat4::from_translation(self.p));
        chain(&apply)
    }
}

/// Per-copy attribute transforms.
struct CopyXf {
    m: DMat4,
    m3: DMat3,
    /// inverse transpose (times `sign(det)`); `None` = singular (normals untouched)
    nrm: Option<DMat3>,
    /// rotation for quaternions; `None` = singular
    quat: Option<DQuat>,
}

/// Rotation part of a 3x3 with positive determinant by polar decomposition (Newton iteration `Q = (Q + Q^-T) / 2`).
fn polar_rotation(a: DMat3) -> Option<DQuat> {
    let mut q = a;
    for _ in 0..64 {
        let det = q.determinant();
        if !det.is_finite() || det.abs() < 1e-300 {
            return None;
        }
        let next = (q + q.inverse().transpose()) * 0.5;
        let diff = (next - q).to_cols_array().iter().fold(0.0f64, |m, c| m.max(c.abs()));
        q = next;
        if diff < 1e-15 {
            break;
        }
    }
    let quat = DQuat::from_mat3(&q).normalize();
    quat.is_finite().then_some(quat)
}

impl CopyXf {
    fn new(m: DMat4) -> CopyXf {
        let m3 = DMat3::from_cols(m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate());
        let det = m3.determinant();
        let ok = det.is_finite() && det.abs() > 1e-300;
        let sign = if det < 0.0 { -1.0 } else { 1.0 };
        let nrm = ok.then(|| m3.inverse().transpose() * sign).filter(|n| n.is_finite());
        let quat = if ok { polar_rotation(m3 * sign) } else { None };
        CopyXf { m, m3, nrm, quat }
    }
}

fn f32x3(v: DVec3) -> [f32; 3] {
    [v.x as f32, v.y as f32, v.z as f32]
}

fn d3(v: [f32; 3]) -> DVec3 {
    DVec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]))
}

/// Overflow check: a finite input that became non-finite is an error.
fn checked(src: &[f32], out: &[f32]) -> Result<(), CookError> {
    if src.iter().all(|c| c.is_finite()) && !out.iter().all(|c| c.is_finite()) {
        Err(failed(OP, "transformed attribute overflows f32"))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Point,
    Vector,
    Normal,
    Quat,
}

fn kind_of(a: &Attribute) -> Option<Kind> {
    if a.class == AttrClass::Detail {
        return None;
    }
    if a.class == AttrClass::Point && a.name == "P" {
        return Some(Kind::Point);
    }
    match (a.type_info, &a.data) {
        (TypeInfo::Point, AttrData::V3(_)) => Some(Kind::Point),
        (TypeInfo::Vector, AttrData::V3(_)) => Some(Kind::Vector),
        (TypeInfo::Normal, AttrData::V3(_)) => Some(Kind::Normal),
        (TypeInfo::Quaternion, AttrData::V4(_)) => Some(Kind::Quat),
        _ => None,
    }
}

fn xform_v3(kind: Kind, x: &CopyXf, v: [f32; 3]) -> Result<[f32; 3], CookError> {
    let out = match kind {
        Kind::Point => f32x3(x.m.transform_point3(d3(v))),
        Kind::Vector => f32x3(x.m3 * d3(v)),
        Kind::Normal => {
            let n = d3(v);
            let len = n.length();
            match x.nrm {
                Some(it) if len > 0.0 && len.is_finite() => {
                    let o = it * n;
                    let ol = o.length();
                    if ol > 0.0 && ol.is_finite() { f32x3(o / ol * len) } else { v }
                }
                _ => v,
            }
        }
        Kind::Quat => v,
    };
    checked(&v, &out)?;
    Ok(out)
}

fn xform_v4(x: &CopyXf, q: [f32; 4]) -> [f32; 4] {
    match x.quat {
        Some(r) => {
            let src = DQuat::from_xyzw(f64::from(q[0]), f64::from(q[1]), f64::from(q[2]), f64::from(q[3]));
            let o = r * src;
            let out = [o.x as f32, o.y as f32, o.z as f32, o.w as f32];
            if out.iter().all(|c| c.is_finite()) { out } else { q }
        }
        None => q,
    }
}

/// Houdini attribute-name sanitising: `[^A-Za-z0-9_]` -> `_`, a leading digit gets a `_` prefix.
fn sanitize_attr(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

fn valid_group_name(name: &str) -> bool {
    let mut it = name.chars();
    it.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && it.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Vertex ids of a prim (empty on a bad range).
fn prim_verts(d: &Detail, p: &Prim) -> std::ops::Range<usize> {
    let (lo, hi) = (p.verts.start as usize, p.verts.end as usize);
    if lo <= hi && hi <= d.vertex_point.len() { lo..hi } else { 0..0 }
}

/// The copied piece: selected prims (prim order), points (point order) and the source-point -> piece-index map.
struct Piece {
    prims: Vec<usize>,
    pts: Vec<usize>,
    local: Vec<Option<u32>>,
    /// selected prims of the original (for `newgroups` group 0)
    prim_sel: Vec<bool>,
}

fn piece(d: &Detail, prim_mask: Option<Vec<bool>>, pt_mask: Option<Vec<bool>>) -> Piece {
    let np = d.point_count as usize;
    let vp = &d.vertex_point;
    let pt_of = |v: usize| vp.get(v).map(|&p| p as usize).filter(|&p| p < np);
    let (prim_sel, pt_sel): (Vec<bool>, Vec<bool>) = match (prim_mask, pt_mask) {
        (Some(pm), _) => {
            let mut pts = vec![false; np];
            for (pr, _) in d.prims.iter().zip(&pm).filter(|(_, s)| **s) {
                for v in prim_verts(d, pr) {
                    if let Some(p) = pt_of(v) {
                        pts[p] = true;
                    }
                }
            }
            (pm, pts)
        }
        (None, Some(ptm)) => {
            let prims = d
                .prims
                .iter()
                .map(|pr| {
                    let r = prim_verts(d, pr);
                    !r.is_empty()
                        && r.into_iter().all(|v| pt_of(v).is_some_and(|p| ptm.get(p).copied().unwrap_or(false)))
                })
                .collect();
            (prims, ptm)
        }
        (None, None) => (vec![true; d.prims.len()], vec![true; np]),
    };
    let prims: Vec<usize> = prim_sel.iter().enumerate().filter(|(_, s)| **s).map(|(i, _)| i).collect();
    let pts: Vec<usize> = pt_sel.iter().take(np).enumerate().filter(|(_, s)| **s).map(|(i, _)| i).collect();
    let mut local = vec![None; np];
    for (k, &p) in pts.iter().enumerate() {
        local[p] = u32::try_from(k).ok();
    }
    Piece { prims, pts, local, prim_sel }
}

impl Node<Detail> for CopyXform {
    fn type_name(&self) -> &'static str {
        "copyxform"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let src: Arc<Detail> = inputs.first().cloned().unwrap_or_default();
        let ncy = int(p, "ncy")?;
        let pack = toggle(p, "pack")?;
        need_menu(p, "pivot", 2)?;
        need_menu(p, "viewportlod", 5)?;
        let gtype = need_menu(p, "sourcegrouptype", 3)?;
        let group = string(p, "sourcegroup")?;
        let xf = Xf {
            t: dvec(v3(p, "t")?),
            r: dvec(v3(p, "r")?),
            s: dvec(v3(p, "s")?),
            sh: dvec(v3(p, "shear")?),
            scale: float(p, "scale")?,
            p: dvec(v3(p, "p")?),
            pr: dvec(v3(p, "pr")?),
            xord: need_menu(p, "xOrd", X_ORDERS.len())?,
            rord: need_menu(p, "rOrd", R_ORDERS.len())?,
        };
        let newgroups = toggle(p, "newgroups")?;
        let prefix = string(p, "newgroupprefix")?;
        let docopyattrib = toggle(p, "docopyattrib")?;
        let copyattrib = string(p, "copyattrib")?;
        if pack {
            return Err(CookError::Unsupported(format!("{OP}: pack (packed primitives) is not implemented")));
        }
        let ncy = u32::try_from(ncy).unwrap_or(0);
        if ncy == 0 {
            return Ok(Arc::new(emptied(&src)));
        }
        let d = &*src;

        // selection
        let (prim_mask, pt_mask) = if group.trim().is_empty() {
            (None, None)
        } else {
            let as_prims = || parse_pattern(d, AttrClass::Prim, &group, OP);
            let as_pts = || parse_pattern(d, AttrClass::Point, &group, OP);
            let r = match u32::try_from(gtype).unwrap_or(0) {
                GROUP_GUESS => match as_prims() {
                    Ok(m) => Ok((m, None)),
                    Err(_) => as_pts().map(|m| (None, m)),
                },
                GROUP_PRIMS => as_prims().map(|m| (m, None)),
                _ => as_pts().map(|m| (None, m)),
            };
            match r {
                Ok(m) => m,
                Err(_) => {
                    (ctx.log)(&format!("{OP}: '{}' is not a valid group", group.trim()));
                    (Some(vec![false; d.prims.len()]), None)
                }
            }
        };
        let pc = piece(d, prim_mask, pt_mask);
        let copies = ncy - 1;

        // sizes + caps
        let piece_verts: u64 =
            pc.prims.iter().map(|&i| d.prims.get(i).map_or(0, |pr| prim_verts(d, pr).len() as u64)).sum();
        let total = |base: u64, per: u64| base.saturating_add(per.saturating_mul(u64::from(copies)));
        let (tp, tv, tq) = (
            total(u64::from(d.point_count), pc.pts.len() as u64),
            total(d.vertex_point.len() as u64, piece_verts),
            total(d.prims.len() as u64, pc.prims.len() as u64),
        );
        if tp > MAX_ELEMS || tv > MAX_ELEMS || tq > MAX_ELEMS {
            return Err(failed(OP, &format!("output of {tp} points / {tv} vertices / {tq} prims exceeds the cap")));
        }
        if copies > 0 {
            for &i in &pc.prims {
                if let Some(pr) = d.prims.get(i) {
                    let ok = matches!(
                        pr.kind,
                        PrimKind::Poly { .. }
                            | PrimKind::Polyline
                            | PrimKind::BezierCurve { .. }
                            | PrimKind::NurbsCurve { .. }
                            | PrimKind::BezierSurf { .. }
                            | PrimKind::NurbsSurf { .. }
                    );
                    if !ok {
                        return Err(CookError::Unsupported(format!("{OP}: copying a {:?} primitive", pr.kind)));
                    }
                }
            }
        }
        let group_names: Vec<String> = if newgroups && !prefix.is_empty() {
            if u64::from(ncy) > MAX_GROUPS || u64::from(ncy).saturating_mul(tq) > MAX_GROUP_CELLS {
                return Err(failed(OP, &format!("{ncy} output groups exceed the cap")));
            }
            let first = format!("{prefix}0");
            if valid_group_name(&first) {
                (0..ncy).map(|i| format!("{prefix}{i}")).collect()
            } else {
                (ctx.log)(&format!("{OP}: '{first}' is not a valid name for a group"));
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // per-copy matrices (copy k >= 1)
        let mut xfs = Vec::with_capacity(if pc.pts.is_empty() && pc.prims.is_empty() { 0 } else { copies as usize });
        if !(pc.pts.is_empty() && pc.prims.is_empty()) {
            for k in 1..ncy {
                let m = xf.matrix(k);
                if !m.to_cols_array().iter().all(|c| c.is_finite()) {
                    return Err(failed(OP, &format!("copy {k}: transform matrix is not finite")));
                }
                xfs.push(CopyXf::new(m));
            }
        }
        let live = xfs.len();

        // element sources
        let np = d.point_count as usize;
        let nv = d.vertex_point.len();
        let nq = d.prims.len();
        let mut point_src: Vec<Option<usize>> = (0..np).map(Some).collect();
        let mut vert_src: Vec<Option<usize>> = (0..nv).map(Some).collect();
        let mut prim_src: Vec<Option<usize>> = (0..nq).map(Some).collect();
        let mut vertex_point = d.vertex_point.clone();
        let mut prims = d.prims.clone();
        for c in 0..live {
            let base = u32::try_from(np + c * pc.pts.len()).map_err(|_| failed(OP, "point id overflow"))?;
            point_src.extend(pc.pts.iter().map(|&i| Some(i)));
            for &qi in &pc.prims {
                let Some(pr) = d.prims.get(qi) else { continue };
                let start = u32::try_from(vertex_point.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
                for v in prim_verts(d, pr) {
                    let lp = d.vertex_point.get(v).and_then(|&p| pc.local.get(p as usize).copied().flatten());
                    vertex_point.push(base.saturating_add(lp.unwrap_or(0)));
                    vert_src.push(Some(v));
                }
                let end = u32::try_from(vertex_point.len()).map_err(|_| failed(OP, "vertex id overflow"))?;
                prims.push(Prim { kind: pr.kind.clone(), verts: start..end });
                prim_src.push(Some(qi));
            }
        }
        let point_count = u32::try_from(point_src.len()).map_err(|_| failed(OP, "point id overflow"))?;
        let mut out = Detail {
            point_count,
            vertex_point,
            prims,
            attrs: Vec::with_capacity(d.attrs.len() + 1),
            groups: Vec::with_capacity(d.groups.len() + group_names.len()),
            edge_groups: Vec::new(),
            skipped_attribs: d.skipped_attribs.clone(),
        };
        let (pp, pv, pq) = (pc.pts.len(), piece_verts as usize, pc.prims.len());
        let base_of = |class: AttrClass| match class {
            AttrClass::Point => (np, pp),
            AttrClass::Vertex => (nv, pv),
            _ => (nq, pq),
        };
        for a in &d.attrs {
            let mut data = match a.class {
                AttrClass::Point => gather(&a.data, &point_src),
                AttrClass::Vertex => gather(&a.data, &vert_src),
                AttrClass::Prim => gather(&a.data, &prim_src),
                AttrClass::Detail => a.data.clone(),
            };
            if let Some(kind) = kind_of(a) {
                let (base, per) = base_of(a.class);
                for (c, x) in xfs.iter().enumerate() {
                    let lo = base + c * per;
                    match &mut data {
                        AttrData::V3(v) => {
                            for e in v.iter_mut().skip(lo).take(per) {
                                *e = xform_v3(kind, x, *e)?;
                            }
                        }
                        AttrData::V4(v) if kind == Kind::Quat => {
                            for e in v.iter_mut().skip(lo).take(per) {
                                *e = xform_v4(x, *e);
                            }
                        }
                        _ => {}
                    }
                }
            }
            out.attrs.push(Attribute { name: a.name.clone(), class: a.class, data, type_info: a.type_info });
        }
        if out.attr(AttrClass::Point, "P").is_none() && out.point_count > 0 {
            let _ = out.p_mut();
        }
        for g in &d.groups {
            let at = |i: &Option<usize>| i.and_then(|i| g.mask.get(i).copied()).unwrap_or(false);
            let mask: Vec<bool> = match g.class {
                AttrClass::Point => point_src.iter().map(at).collect(),
                AttrClass::Vertex => vert_src.iter().map(at).collect(),
                AttrClass::Prim => prim_src.iter().map(at).collect(),
                AttrClass::Detail => continue,
            };
            out.groups.push(Group { name: g.name.clone(), class: g.class, mask });
        }
        for eg in &d.edge_groups {
            let mut edges = eg.edges.clone();
            for c in 0..live {
                let base = (np + c * pp) as u64;
                for &[a, b] in &eg.edges {
                    let l = |p: u32| pc.local.get(p as usize).copied().flatten();
                    if let (Some(x), Some(y)) = (l(a), l(b)) {
                        let (x, y) = (base + u64::from(x), base + u64::from(y));
                        if let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) {
                            edges.push([x, y]);
                        }
                    }
                }
            }
            out.set_edge_group(&eg.name, edges);
        }

        // copy number attribute
        if docopyattrib && !copyattrib.is_empty() {
            let name = sanitize_attr(&copyattrib);
            if name != copyattrib {
                (ctx.log)(&format!("{OP}: invalid attribute name {copyattrib}"));
            }
            if name == "P" {
                (ctx.log)(&format!("{OP}: unable to create integer attribute \"P\""));
            } else {
                // hython: selected prims of the original = 0, unselected ones = -1
                let mut v: Vec<i32> =
                    (0..nq).map(|q| if pc.prim_sel.get(q).copied().unwrap_or(false) { 0 } else { -1 }).collect();
                for c in 0..live {
                    let k = i32::try_from(c + 1).unwrap_or(i32::MAX);
                    v.extend(std::iter::repeat_n(k, pq));
                }
                out.add_attr(Attribute {
                    name,
                    class: AttrClass::Prim,
                    data: AttrData::I32(v),
                    type_info: TypeInfo::None,
                });
            }
        }
        // output groups
        let total_prims = out.prims.len();
        for (i, name) in group_names.iter().enumerate() {
            let mut mask = vec![false; total_prims];
            if i == 0 {
                for (m, s) in mask.iter_mut().zip(&pc.prim_sel) {
                    *m = *s;
                }
            } else if i <= live {
                let lo = nq + (i - 1) * pq;
                for m in mask.iter_mut().skip(lo).take(pq) {
                    *m = true;
                }
            }
            out.groups.retain(|g| !(g.class == AttrClass::Prim && g.name == *name));
            out.groups.push(Group { name: name.clone(), class: AttrClass::Prim, mask });
        }
        out.canonicalize();
        Ok(Arc::new(out))
    }
}

/// `ncy <= 0`: no elements, attribute / group definitions (and detail attributes) kept.
fn emptied(d: &Detail) -> Detail {
    let attrs = d
        .attrs
        .iter()
        .map(|a| {
            let data = if a.class == AttrClass::Detail { a.data.clone() } else { gather(&a.data, &[]) };
            Attribute { name: a.name.clone(), class: a.class, data, type_info: a.type_info }
        })
        .collect();
    let groups = d.groups.iter().map(|g| Group { name: g.name.clone(), class: g.class, mask: Vec::new() }).collect();
    let edge_groups =
        d.edge_groups.iter().map(|g| hexe_geo::EdgeGroup { name: g.name.clone(), edges: Vec::new() }).collect();
    Detail {
        point_count: 0,
        vertex_point: Vec::new(),
        prims: Vec::new(),
        attrs,
        groups,
        edge_groups,
        skipped_attribs: d.skipped_attribs.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::meshutil::testkit::{cook, defaults};
    use hexe_geo::read_geo;

    fn node() -> Box<SopNode> {
        new()
    }
    fn run(d: Detail, over: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        let n = node();
        let mut p = defaults(&*n);
        for (k, v) in over {
            p = p.with(k, v.clone());
        }
        cook(&*n, d, &p)
    }
    fn v3p(a: f32, b: f32, c: f32) -> ParamValue {
        ParamValue::V3([a, b, c])
    }
    /// 4 points, a closed triangle + an open 2-point poly, prim group `tri`, point group `first`.
    fn sample() -> Detail {
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.5]];
        d.add_poly(&[0, 1, 2], true);
        d.add_poly(&[2, 3], false);
        d.groups.push(Group { name: "tri".into(), class: AttrClass::Prim, mask: vec![true, false] });
        d.groups.push(Group { name: "first".into(), class: AttrClass::Point, mask: vec![true, false, false, false] });
        d
    }
    fn close(a: &[[f32; 3]], b: &[[f32; 3]]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.iter().zip(y).all(|(p, q)| (p - q).abs() <= 1e-5))
    }

    #[test]
    fn copy_i_uses_parms_scaled_by_i_not_matrix_power() {
        // hython: ncy 3, t (1,0,0), r (0,0,90): copy 2 = Rz(180) then +2x (M^2 would map (1,0,0) to (0,1,0))
        let out = run(sample(), &[("ncy", ParamValue::I(3)), ("t", v3p(1.0, 0.0, 0.0)), ("r", v3p(0.0, 0.0, 90.0))])
            .expect("cooks");
        assert_eq!((out.point_count, out.prims.len()), (12, 6));
        assert!(close(&out.p()[..4], sample().p()), "copy 0 untouched");
        assert!(close(&out.p()[4..8], &[[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.5]]));
        assert!(close(&out.p()[8..], &[[2.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, -1.0, 0.0], [2.0, -1.0, 0.5]]));
        // scale about a pivot: s^i ; hython s (2,1,1) p (1,0,0): copy 2 of (0,0,0) = 1 + (0-1)*4 = -3
        let out = run(sample(), &[("ncy", ParamValue::I(3)), ("s", v3p(2.0, 1.0, 1.0)), ("p", v3p(1.0, 0.0, 0.0))])
            .expect("s");
        assert!((out.p()[8][0] + 3.0).abs() < 1e-6);
        // topology: copies offset, winding kept
        let ids =
            |q: usize| out.vertex_point[out.prims[q].verts.start as usize..out.prims[q].verts.end as usize].to_vec();
        assert_eq!((ids(2), ids(3), ids(4), ids(5)), (vec![4, 5, 6], vec![6, 7], vec![8, 9, 10], vec![10, 11]));
        assert_eq!(
            out.group(AttrClass::Prim, "tri").map(|g| g.mask.clone()),
            Some(vec![true, false, true, false, true, false])
        );
    }

    #[test]
    fn ncy_zero_one_and_negative() {
        let one = run(sample(), &[("ncy", ParamValue::I(1)), ("t", v3p(0.0, 0.0, 1.0))]).expect("one");
        assert!(close(one.p(), sample().p()));
        assert_eq!(one.prims.len(), 2);
        for n in [0, -3, i32::MIN] {
            let e = run(sample(), &[("ncy", ParamValue::I(n))]).expect("empty");
            assert_eq!((e.point_count, e.prims.len(), e.vertex_point.len()), (0, 0, 0));
            assert!(e.group(AttrClass::Prim, "tri").is_some_and(|g| g.mask.is_empty()));
        }
    }

    #[test]
    fn source_groups_prims_points_and_unknown() {
        // prim group: original + the triangle (3 points)
        let out =
            run(sample(), &[("sourcegroup", ParamValue::S("tri".into())), ("t", v3p(0.0, 0.0, 1.0))]).expect("prims");
        assert_eq!((out.point_count, out.prims.len()), (7, 3));
        // point group by guess (no prim group `first`): only point 0 copied, no prims
        let out = run(sample(), &[("sourcegroup", ParamValue::S("first".into()))]).expect("points");
        assert_eq!((out.point_count, out.prims.len()), (5, 2));
        assert_eq!(
            out.group(AttrClass::Point, "first").map(|g| g.mask.clone()),
            Some(vec![true, false, false, false, true])
        );
        // a point selection copies the prims whose points are all selected
        let out =
            run(sample(), &[("sourcegroup", ParamValue::S("2-3".into())), ("sourcegrouptype", ParamValue::Menu(2))])
                .expect("pts");
        assert_eq!((out.point_count, out.prims.len()), (6, 3));
        // reversed range, numbers in guess mode = prims
        let out = run(sample(), &[("sourcegroup", ParamValue::S("1".into()))]).expect("prim 1");
        assert_eq!((out.point_count, out.prims.len()), (6, 3));
        // unknown group / wrong class: warning, original only
        for (g, t) in [("nope", 0), ("first", 1), ("@P.y>0", 0)] {
            let out = run(
                sample(),
                &[
                    ("sourcegroup", ParamValue::S(g.into())),
                    ("sourcegrouptype", ParamValue::Menu(t)),
                    ("ncy", ParamValue::I(5)),
                ],
            )
            .expect("passthrough");
            assert_eq!((out.point_count, out.prims.len()), (4, 2), "{g}");
        }
    }

    #[test]
    fn copy_number_attribute_and_groups() {
        let out = run(
            sample(),
            &[
                ("ncy", ParamValue::I(3)),
                ("docopyattrib", ParamValue::B(true)),
                ("newgroups", ParamValue::B(true)),
                ("sourcegroup", ParamValue::S("tri".into())),
            ],
        )
        .expect("cooks");
        assert_eq!(
            out.attr(AttrClass::Prim, "copynum").map(|a| a.data.clone()),
            Some(AttrData::I32(vec![0, -1, 1, 2]))
        );
        let g = |n: &str| out.group(AttrClass::Prim, n).map(|g| g.mask.clone());
        assert_eq!(g("copyGroup0"), Some(vec![true, false, false, false]));
        assert_eq!(g("copyGroup1"), Some(vec![false, false, true, false]));
        assert_eq!(g("copyGroup2"), Some(vec![false, false, false, true]));
        // sanitising / refusal / invalid group prefix
        let cook_name = |name: &str, prefix: &str| {
            run(
                sample(),
                &[
                    ("docopyattrib", ParamValue::B(true)),
                    ("copyattrib", ParamValue::S(name.into())),
                    ("newgroups", ParamValue::B(true)),
                    ("newgroupprefix", ParamValue::S(prefix.into())),
                ],
            )
            .expect("cooks")
        };
        let o = cook_name("1a b", "a b");
        assert!(o.attr(AttrClass::Prim, "_1a_b").is_some());
        assert!(!o.groups.iter().any(|g| g.name.starts_with('a')));
        let o = cook_name("P", "");
        assert!(o.attr(AttrClass::Prim, "P").is_none());
        assert!(o.groups.iter().all(|g| !g.name.is_empty()));
    }

    #[test]
    fn typed_attributes_transform_by_type_info() {
        let mut d = sample();
        let add = |d: &mut Detail, name: &str, ti: TypeInfo, data: AttrData| {
            d.add_attr(Attribute { name: name.into(), class: AttrClass::Point, data, type_info: ti });
        };
        add(&mut d, "v", TypeInfo::Vector, AttrData::V3(vec![[1.0, 0.0, 0.0]; 4]));
        add(&mut d, "bar", TypeInfo::None, AttrData::V3(vec![[1.0, 0.0, 0.0]; 4]));
        add(&mut d, "N", TypeInfo::Normal, AttrData::V3(vec![[2.0, 0.0, 0.0]; 4]));
        add(&mut d, "orient", TypeInfo::Quaternion, AttrData::V4(vec![[0.0, 0.0, 0.0, 1.0]; 4]));
        // hython: s (-1,1,1) flips the normal back (det < 0), v becomes (-1,0,0)
        let out = run(d.clone(), &[("s", v3p(-1.0, 1.0, 1.0))]).expect("cooks");
        let v3s = |n: &str| match out.attr(AttrClass::Point, n).map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            _ => Vec::new(),
        };
        assert_eq!(v3s("v")[4], [-1.0, 0.0, 0.0]);
        assert_eq!(v3s("bar")[4], [1.0, 0.0, 0.0]);
        assert_eq!(v3s("N")[4], [2.0, 0.0, 0.0]);
        // reflection: orient gets the 180-degree x rotation of polar(-M)
        match out.attr(AttrClass::Point, "orient").map(|a| &a.data) {
            Some(AttrData::V4(q)) => assert!(q[4].iter().zip([1.0, 0.0, 0.0, 0.0]).all(|(a, b)| (a - b).abs() < 1e-6)),
            other => panic!("{other:?}"),
        }
        // singular: normals / orient untouched, vectors collapse
        let out = run(d, &[("scale", ParamValue::F(0.0))]).expect("cooks");
        match out.attr(AttrClass::Point, "N").map(|a| &a.data) {
            Some(AttrData::V3(n)) => assert_eq!(n[4], [2.0, 0.0, 0.0]),
            other => panic!("{other:?}"),
        }
    }

    /// Synthetic hython cases (`oracle/houdini/copyxform/sop_copyxform_cases.py`): a box with typed point / vertex /
    /// prim attributes and groups through `xform` (every transform parm, ncy 4), `group` (prim group, negative scale)
    /// and `ptgroup` (point group): Houdini 20.5 in / out, every element, attribute value and group compared.
    #[test]
    fn recorded_cases_match_houdini_exactly() {
        let fx = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/sop-copyxform/");
        // KNOWN geo-reader gap: Houdini saves small groups with the `"unordered",["i8",[0,1,...]]` selection encoding,
        // which `read_geo` does not decode (all-false masks). Decode those here from the text (group names are unique
        // across classes in these fixtures) so the comparison is real.
        let read = |f: &str| {
            let bytes = std::fs::read(format!("{fx}{f}.geo")).expect("fixture");
            let mut d = read_geo(&bytes).expect("geo");
            let text = String::from_utf8_lossy(&bytes).replace(['\t', '\n', '\r', ' '], "");
            let mut rest = text.as_str();
            while let Some(at) = rest.find("[\"name\",\"") {
                rest = &rest[at + 9..];
                let name = rest[..rest.find('"').expect("name end")].to_string();
                let next = rest.find("[\"name\",\"").unwrap_or(rest.len());
                if let Some(i8_at) = rest[..next].find("\"i8\",[") {
                    let body = &rest[i8_at + 6..];
                    let body = &body[..body.find(']').expect("i8 end")];
                    let mask: Vec<bool> = body.split(',').map(|v| v.trim() == "1").collect();
                    for g in d.groups.iter_mut().filter(|g| g.name == name && g.mask.len() == mask.len()) {
                        g.mask = mask.clone();
                    }
                }
            }
            d
        };
        let input = read("box_in");
        let cases: [(&str, Vec<(&str, ParamValue)>); 3] = [
            (
                "xform",
                vec![
                    ("ncy", ParamValue::I(4)),
                    ("t", v3p(0.5, 0.25, -0.75)),
                    ("r", v3p(15.0, 30.0, 45.0)),
                    ("s", v3p(1.25, 0.8, 1.1)),
                    ("shear", v3p(0.1, 0.2, 0.3)),
                    ("scale", ParamValue::F(1.05)),
                    ("p", v3p(0.3, -0.2, 0.1)),
                    ("pr", v3p(10.0, 20.0, 30.0)),
                    ("xOrd", ParamValue::Menu(3)),
                    ("rOrd", ParamValue::Menu(4)),
                    ("docopyattrib", ParamValue::B(true)),
                    ("newgroups", ParamValue::B(true)),
                ],
            ),
            (
                "group",
                vec![
                    ("ncy", ParamValue::I(3)),
                    ("sourcegroup", ParamValue::S("top".into())),
                    ("t", v3p(0.0, 1.5, 0.0)),
                    ("r", v3p(0.0, 0.0, 20.0)),
                    ("s", v3p(-1.0, 1.0, 1.0)),
                    ("docopyattrib", ParamValue::B(true)),
                    ("copyattrib", ParamValue::S("cnum".into())),
                    ("newgroups", ParamValue::B(true)),
                    ("newgroupprefix", ParamValue::S("grp".into())),
                ],
            ),
            (
                "ptgroup",
                vec![
                    ("ncy", ParamValue::I(3)),
                    ("sourcegroup", ParamValue::S("low".into())),
                    ("sourcegrouptype", ParamValue::Menu(2)),
                    ("t", v3p(0.0, 0.0, 2.0)),
                    ("r", v3p(0.0, 90.0, 0.0)),
                ],
            ),
        ];
        for (name, over) in cases {
            let h = read(&format!("{name}_houdini"));
            let d = run(input.clone(), &over).expect("cooks");
            assert_eq!(
                (d.point_count, d.prims.len(), d.vertex_point.len()),
                (h.point_count, h.prims.len(), h.vertex_point.len()),
                "{name} counts"
            );
            assert_eq!(d.vertex_point, h.vertex_point, "{name} topology");
            for (a, b) in d.prims.iter().zip(&h.prims) {
                assert_eq!((&a.kind, &a.verts), (&b.kind, &b.verts), "{name} prims");
            }
            for ha in &h.attrs {
                let da =
                    d.attr(ha.class, &ha.name).unwrap_or_else(|| panic!("{name}: missing {:?} {}", ha.class, ha.name));
                let flat = |x: &AttrData| -> Option<Vec<f32>> {
                    match x {
                        AttrData::F32(v) => Some(v.clone()),
                        AttrData::V3(v) => Some(v.iter().flatten().copied().collect()),
                        AttrData::V4(v) => Some(v.iter().flatten().copied().collect()),
                        _ => None,
                    }
                };
                match (flat(&ha.data), flat(&da.data)) {
                    (Some(x), Some(y)) => {
                        assert_eq!(x.len(), y.len(), "{name} {}", ha.name);
                        for (k, (p, q)) in x.iter().zip(&y).enumerate() {
                            assert!(
                                (p - q).abs() <= 2e-5,
                                "{name} {:?} {}[{k}]: hexe {q} houdini {p}",
                                ha.class,
                                ha.name
                            );
                        }
                    }
                    _ => assert_eq!(ha.data, da.data, "{name} {:?} {}", ha.class, ha.name),
                }
            }
            let names = |x: &Detail| {
                let mut v: Vec<(String, String)> =
                    x.attrs.iter().map(|a| (format!("{:?}", a.class), a.name.clone())).collect();
                v.sort();
                v
            };
            assert_eq!(names(&d), names(&h), "{name} attribute sets");
            let groups = |x: &Detail| {
                let mut v: Vec<(String, String, Vec<bool>)> =
                    x.groups.iter().map(|g| (format!("{:?}", g.class), g.name.clone(), g.mask.clone())).collect();
                v.sort();
                v
            };
            assert_eq!(groups(&d), groups(&h), "{name} groups");
        }
    }

    #[test]
    fn hostile_inputs_never_panic() {
        // huge ncy: capped
        assert!(matches!(run(sample(), &[("ncy", ParamValue::I(i32::MAX))]), Err(CookError::Failed(_))));
        // huge ncy on empty input: nothing to copy, cheap
        let e = run(Detail::default(), &[("ncy", ParamValue::I(i32::MAX))]).expect("empty");
        assert_eq!(e.point_count, 0);
        assert!(
            run(Detail::default(), &[("ncy", ParamValue::I(i32::MAX)), ("newgroups", ParamValue::B(true))]).is_err()
        );
        // no input at all
        let n = node();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &defaults(&*n), log: &|_| {} };
        assert_eq!(n.cook(&ctx, &[]).expect("none").point_count, 0);
        // non-finite / wrong-typed / out-of-range parms
        for (k, v) in [
            ("t", v3p(f32::NAN, 0.0, 0.0)),
            ("r", v3p(0.0, f32::INFINITY, 0.0)),
            ("s", v3p(0.0, 0.0, f32::NEG_INFINITY)),
            ("shear", v3p(f32::NAN, 0.0, 0.0)),
            ("p", v3p(f32::INFINITY, 0.0, 0.0)),
            ("pr", v3p(f32::NAN, 0.0, 0.0)),
            ("scale", ParamValue::F(f32::NAN)),
            ("xOrd", ParamValue::Menu(6)),
            ("rOrd", ParamValue::Menu(u32::MAX)),
            ("sourcegrouptype", ParamValue::Menu(3)),
            ("pivot", ParamValue::Menu(2)),
            ("ncy", ParamValue::F(2.0)),
            ("t", ParamValue::F(1.0)),
        ] {
            assert!(matches!(run(sample(), &[(k, v.clone())]), Err(CookError::BadParam(_))), "{k} {v:?}");
        }
        // f32 overflow of the result, overflowing powers
        assert!(matches!(
            run(sample(), &[("t", v3p(3.0e38, 0.0, 0.0)), ("ncy", ParamValue::I(4))]),
            Err(CookError::Failed(_))
        ));
        assert!(run(sample(), &[("scale", ParamValue::F(1.0e30)), ("ncy", ParamValue::I(40))]).is_err());
        // bad group strings: never a panic
        for g in ["!", "!!", "5-", "-", "99999999999999999999999", "*", "?*", "0-4294967295", "\u{0}"] {
            let _ = run(sample(), &[("sourcegroup", ParamValue::S(g.into()))]);
        }
        // pack is not modelled
        assert!(matches!(run(sample(), &[("pack", ParamValue::B(true))]), Err(CookError::Unsupported(_))));
        // corrupt topology: vertex pointing past the points
        let mut d = sample();
        d.vertex_point[0] = 99;
        let _ = run(d, &[("ncy", ParamValue::I(3))]);
        // unsupported prim kinds in the copied piece
        let mut d = sample();
        d.prims[1].kind = PrimKind::Sphere { xform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] };
        assert!(matches!(run(d.clone(), &[]), Err(CookError::Unsupported(_))));
        assert_eq!(run(d, &[("ncy", ParamValue::I(1))]).expect("original only").prims.len(), 2);
    }

    #[test]
    fn registered_and_scene_loadable() {
        assert_eq!(crate::registry().create("copyxform").map(|n| n.type_name()), Some("copyxform"));
        let ok = r#"{"nodes":[{"name":"c","type":"copyxform","params":{"ncy":3,"t":[1,0,0],"sourcegroup":"","docopyattrib":true}}],"display":"c"}"#;
        let (mut g, id) = crate::load_scene(ok).expect("loads");
        assert_eq!(g.cook(id, 0.0).expect("cooks").point_count, 0);
    }
}
