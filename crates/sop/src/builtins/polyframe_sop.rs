//! PolyFrame SOP (token `polyframe`): per-point `N` / `tangentu` / `tangentv` frame from the polygons (Houdini 20.5 `polyframe`).
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-polyframe*`)
//! Output is always POINT attributes for the edge / centroid styles (`entity` primitives and points give identical results in every probe: kept
//! as a menu, no effect). Per prim: `N` = the unit Houdini polygon normal (open polys count as closed). `N` of a point = normalised sum of the
//! unit normals of the prims that use it. `tangentu` is taken from the FIRST prim (lowest index) that uses the point, at its vertex `i`:
//! * `edge1` (First Edge): `P[i-1] - P[i]` (closed: wraps; first vertex of an OPEN prim: `P[0] - P[1]`);
//! * `edge2` (Two Edges, default): closed `(P[i-1]-P[i]) + (P[i+1]-P[i])`; open `P[i-1] - P[i+1]` (ends: one-sided as `edge1`);
//! * `primC` (Primitive Centroid): `centroid - P[i]`;
//!
//! each normalised (zero stays zero). `ortho`: `tangentu -= N (N.tangentu)` (NOT renormalised: |tangentu| = sin of its angle to N). `tangentv` =
//! `cross(tangentu, N)` (NOT normalised); with `ortho` it is `cross(N, tangentu)` and `lefthanded` flips its sign (no effect without `ortho`).
//! Toggles `Non` / `tangentuon` / `tangentvon` (default 1/1/0) decide which attributes are written, names from `N` / `tangentu` / `tangentv`.
//! Points no (selected) prim uses get zero vectors. `N` is typed Normal, the tangents Vector.
//!
//! Not modelled: styles `texuv`, `tex`, `attrib`, `mikkt` (vertex-class outputs, -> `Unsupported`), `signson` (only meaningful there), whether
//! the `N` average is area-weighted (uniform here), orphan-point values in Houdini, prim `group` on shared points.
use super::group_sop::parse_pattern;
use super::meshutil::{V3, add, bad, cross, dot, menu, positions, prim_normal, scale, sub, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, PrimKind, TypeInfo};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "polyframe";
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const EDGE1: u32 = 0;
const EDGE2: u32 = 1;
const PRIMC: u32 = 2;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("entity", ParamValue::Menu(0))
            .label("Entity")
            .menu(&[("primitive", "Primitives"), ("point", "Points")]),
        ParamSpec::new("style", ParamValue::Menu(EDGE2)).label("Frame Style").menu(&[
            ("edge1", "First Edge"),
            ("edge2", "Two Edges"),
            ("primC", "Primitive Centroid"),
            ("texuv", "Texture UV"),
            ("tex", "Texture UV Gradient"),
            ("attrib", "Attribute Gradient"),
            ("mikkt", "MikkT"),
        ]),
        ParamSpec::new("attribname", ParamValue::S(String::new())).label("Attribute"),
        ParamSpec::new("Non", ParamValue::B(true)).label("Normal"),
        ParamSpec::new("N", ParamValue::S("N".into())).label("Normal Name"),
        ParamSpec::new("tangentuon", ParamValue::B(true)).label("Tangent U"),
        ParamSpec::new("tangentu", ParamValue::S("tangentu".into())).label("Tangent U Name"),
        ParamSpec::new("tangentvon", ParamValue::B(false)).label("Tangent V"),
        ParamSpec::new("tangentv", ParamValue::S("tangentv".into())).label("Tangent V Name"),
        ParamSpec::new("signson", ParamValue::B(false)).label("Signs"),
        ParamSpec::new("signs", ParamValue::S("signs".into())).label("Signs Name"),
        ParamSpec::new("ortho", ParamValue::B(false)).label("Orthonormal"),
        ParamSpec::new("lefthanded", ParamValue::B(false)).label("Left Handed"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(PolyFrame { specs: spec() })
}
struct PolyFrame {
    specs: Vec<ParamSpec>,
}
fn unit(v: V3) -> V3 {
    let l = dot(v, v).sqrt();
    if l.is_finite() && l > 1e-12 { scale(v, 1.0 / l) } else { [0.0; 3] }
}
fn name<'a>(p: &'a hexe_graph::ParamSet, n: &str) -> Result<&'a str, CookError> {
    match p.string(n) {
        Some(s) if !s.is_empty() && s.len() <= 256 => Ok(s),
        _ => Err(bad(format!("`{n}`: missing or empty attribute name"))),
    }
}
/// Tangent direction (unnormalised) at vertex `i` of the prim with positions `c`.
fn edge_dir(style: u32, c: &[V3], closed: bool, i: usize) -> V3 {
    let n = c.len();
    let at = |k: usize| c.get(k).copied().unwrap_or([0.0; 3]);
    match style {
        PRIMC => {
            let s = c.iter().fold([0.0; 3], |a, &q| add(a, q));
            sub(scale(s, 1.0 / n as f64), at(i))
        }
        _ if closed => {
            let (prev, next) = (at((i + n - 1) % n), at((i + 1) % n));
            if style == EDGE1 { sub(prev, at(i)) } else { add(sub(prev, at(i)), sub(next, at(i))) }
        }
        _ => {
            if i == 0 {
                sub(at(0), at(1))
            } else if style == EDGE1 || i + 1 == n {
                sub(at(i - 1), at(i))
            } else {
                sub(at(i - 1), at(i + 1))
            }
        }
    }
}
impl Node<Detail> for PolyFrame {
    fn type_name(&self) -> &'static str {
        OP
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let style = menu(p, "style")?;
        if style > PRIMC {
            if style > 6 {
                return Err(bad(format!("`style`: menu index {style} out of range")));
            }
            return Err(CookError::Unsupported(
                "polyframe: texture / attribute / MikkT styles (vertex outputs) are not modelled".into(),
            ));
        }
        if menu(p, "entity")? > 1 {
            return Err(bad("`entity`: menu index out of range (primitive | point)"));
        }
        let (non, tu_on, tv_on) = (toggle(p, "Non")?, toggle(p, "tangentuon")?, toggle(p, "tangentvon")?);
        let (ortho, left) = (toggle(p, "ortho")?, toggle(p, "lefthanded")?);
        let (n_name, tu_name, tv_name) = (name(p, "N")?, name(p, "tangentu")?, name(p, "tangentv")?);
        let pat = p.string("group").ok_or_else(|| bad("`group`: missing or not a string"))?;
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        if !(non || tu_on || tv_on) {
            return Ok(Arc::clone(src));
        }
        let sel = parse_pattern(src, AttrClass::Prim, pat, OP)?;
        let pos = positions(src);
        let np = pos.len();
        let mut nacc = vec![[0.0f64; 3]; np];
        let mut tu: Vec<Option<V3>> = vec![None; np];
        for (i, pr) in src.prims.iter().enumerate() {
            if !sel.as_ref().is_none_or(|m| m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            let closed = match pr.kind {
                PrimKind::Poly { closed } => closed,
                PrimKind::Polyline => false,
                _ => continue,
            };
            let Some(vs) = src.vertex_point.get(pr.verts.start as usize..pr.verts.end as usize) else { continue };
            let ids: Vec<usize> = vs.iter().map(|&q| q as usize).collect();
            if ids.len() < 2 || ids.iter().any(|&q| q >= np) {
                continue;
            }
            let c: Vec<V3> = ids.iter().map(|&q| pos[q]).collect();
            let nrm = if ids.len() >= 3 { prim_normal(&pos, &ids).unwrap_or([0.0; 3]) } else { [0.0; 3] };
            for (k, &q) in ids.iter().enumerate() {
                nacc[q] = add(nacc[q], nrm);
                if tu[q].is_none() {
                    tu[q] = Some(unit(edge_dir(style, &c, closed, k)));
                }
            }
        }
        let n: Vec<V3> = nacc.into_iter().map(unit).collect();
        let mut u: Vec<V3> = tu.into_iter().map(|t| t.unwrap_or([0.0; 3])).collect();
        if ortho {
            for (t, nn) in u.iter_mut().zip(&n) {
                *t = sub(*t, scale(*nn, dot(*nn, *t)));
            }
        }
        let v: Vec<V3> = u
            .iter()
            .zip(&n)
            .map(|(t, nn)| {
                if ortho {
                    let c = cross(*nn, *t);
                    if left { scale(c, -1.0) } else { c }
                } else {
                    cross(*t, *nn)
                }
            })
            .collect();
        let f32s = |a: &[V3]| AttrData::V3(a.iter().map(|q| [q[0] as f32, q[1] as f32, q[2] as f32]).collect());
        let mut out = (**src).clone();
        if non {
            out.add_attr(Attribute {
                name: n_name.into(),
                class: AttrClass::Point,
                data: f32s(&n),
                type_info: TypeInfo::Normal,
            });
        }
        if tu_on {
            out.add_attr(Attribute {
                name: tu_name.into(),
                class: AttrClass::Point,
                data: f32s(&u),
                type_info: TypeInfo::Vector,
            });
        }
        if tv_on {
            out.add_attr(Attribute {
                name: tv_name.into(),
                class: AttrClass::Point,
                data: f32s(&v),
                type_info: TypeInfo::Vector,
            });
        }
        Ok(Arc::new(out))
    }
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cube, defaults, houdini_twin, houdini_values};
    use super::*;
    use hexe_graph::ParamSet;
    fn quad() -> Detail {
        let mut d = Detail::default();
        d.add_points(4);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.5, 1.5, 0.5], [0.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2, 3], true);
        d
    }
    fn run(d: Detail, f: impl FnOnce(ParamSet) -> ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        cook(&*n, d, &f(defaults(&*n)))
    }
    fn v3(d: &Detail, n: &str) -> Vec<[f32; 3]> {
        match d.attr(AttrClass::Point, n).map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }
    #[test]
    fn quad_styles_match_the_probed_numbers() {
        let e2 = run(quad(), |p| p).expect("cooks");
        assert!(
            near(v3(&e2, "tangentu")[0], [0.8944, 0.4472, 0.0])
                && near(v3(&e2, "tangentu")[1], [-0.6882, 0.6882, 0.2294])
        );
        assert!(near(v3(&e2, "N")[2], [0.0891, 0.1782, -0.98]));
        let e1 = run(quad(), |p| p.with("style", ParamValue::Menu(EDGE1))).expect("cooks");
        assert!(
            near(v3(&e1, "tangentu")[0], [0.0, 1.0, 0.0]) && near(v3(&e1, "tangentu")[3], [0.9623, 0.1925, 0.1925])
        );
        let pc = run(quad(), |p| p.with("style", ParamValue::Menu(PRIMC))).expect("cooks");
        assert!(near(v3(&pc, "tangentu")[0], [0.8701, 0.4834, 0.0967]));
    }
    #[test]
    fn tangent_v_ortho_and_handedness() {
        let t = run(quad(), |p| p.with("tangentvon", ParamValue::B(true))).expect("cooks");
        assert!(near(v3(&t, "tangentv")[0], [-0.4383, 0.8765, 0.1195]), "{:?}", v3(&t, "tangentv"));
        let o = run(quad(), |p| p.with("tangentvon", ParamValue::B(true)).with("ortho", ParamValue::B(true)))
            .expect("cooks");
        assert!(
            near(v3(&o, "tangentu")[0], [0.8802, 0.4188, 0.1562])
                && near(v3(&o, "tangentv")[0], [0.4383, -0.8765, -0.1195])
        );
        let l = run(quad(), |p| {
            p.with("tangentvon", ParamValue::B(true))
                .with("ortho", ParamValue::B(true))
                .with("lefthanded", ParamValue::B(true))
        })
        .expect("cooks");
        assert!(near(v3(&l, "tangentv")[0], [-0.4383, 0.8765, 0.1195]));
    }
    #[test]
    fn box_corner_takes_first_prim_tangent_and_open_polys_are_one_sided() {
        let b = run(cube(), |p| p).expect("cooks");
        assert_eq!(b.point_count, 8);
        assert!(v3(&b, "N").iter().all(|n| n.iter().all(|c| (c.abs() - 0.577_35).abs() < 1e-4)));
        let mut l = Detail::default();
        l.add_points(3);
        *l.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        l.add_poly(&[0, 1, 2], false);
        let o = run(l, |p| p).expect("cooks");
        let t = v3(&o, "tangentu");
        assert!(
            near(t[0], [-1.0, 0.0, 0.0])
                && near(t[1], [-std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2, 0.0])
                && near(t[2], [0.0, -1.0, 0.0])
        );
        assert!(near(v3(&o, "N")[0], [0.0, 0.0, -1.0]));
    }
    #[test]
    fn names_toggles_and_hostile_params() {
        let n = run(quad(), |p| {
            p.with("Non", ParamValue::B(false))
                .with("tangentu", ParamValue::S("tu".into()))
                .with("tangentvon", ParamValue::B(true))
        })
        .expect("cooks");
        assert!(
            n.attr(AttrClass::Point, "N").is_none()
                && n.attr(AttrClass::Point, "tu").is_some()
                && n.attr(AttrClass::Point, "tangentv").is_some()
        );
        let same = run(quad(), |p| p.with("Non", ParamValue::B(false)).with("tangentuon", ParamValue::B(false)))
            .expect("cooks");
        assert!(same.attr(AttrClass::Point, "tangentu").is_none());
        assert!(matches!(run(quad(), |p| p.with("style", ParamValue::Menu(3))), Err(CookError::Unsupported(_))));
        assert!(run(quad(), |p| p.with("style", ParamValue::Menu(9))).is_err());
        assert!(run(quad(), |p| p.with("N", ParamValue::S(String::new()))).is_err());
        assert_eq!(run(Detail::default(), |p| p).expect("empty").point_count, 0);
    }
    macro_rules! twin {
        ($n:literal) => {{
            let (scene, geo) = (
                include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
            );
            houdini_twin(scene, geo).and_then(|()| houdini_values(scene, geo, 1e-4))
        }};
    }
    /// Hexe twin == real Houdini 20.5 `.geo`: topology, P, attribute names AND attribute values (1e-4).
    #[test]
    fn polyframe_matches_houdini_fixtures() {
        for n in [
            twin!("sop-polyframe"),
            twin!("sop-polyframe-edge1"),
            twin!("sop-polyframe-quad"),
            twin!("sop-polyframe-primc"),
        ] {
            assert_eq!(n, Ok(()));
        }
    }
}
