//! Sort SOP (token `sort`): renumbers points and/or primitives.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Two independent orderings, `ptsort` (points) and `primsort` (prims), each a menu:
//! `0 none | 1 byx | 2 byy | 3 byz | 4 byattribute | 5 byexpression | 6 random | 7 reverse | 8 shift | 9 proximity`.
//! Both permutations are computed on the INPUT, then applied together. All sorts are stable (ties keep input order).
//! * byx/byy/byz: ascending coordinate of `P` (prim = mean of its points).
//! * byattribute (`*attrib`): ascending, tuples lexicographically, strings by byte order. Missing attribute -> `Failed`.
//! * byexpression (`*expr`): VEX-subset expression per element (`@P.y * -1`, `@ptnum % 3`); numeric result, ascending.
//! * random (`*seed`): ascending splitmix64(seed, element) key. Deterministic; NOT Houdini's stream.
//! * reverse: last becomes first.
//! * shift (`*offset`): `new[i] = old[(i + offset) mod n]` (positive offset rotates the front to the back). Sign convention a guess.
//! * proximity (`*proxpos`): ascending distance to the position (prim = centroid).
//!
//! Point renumbering remaps every `vertex_point`; prim reordering rebuilds vertex storage contiguously in the new prim
//! order. Point/vertex/prim attributes and groups travel with their element. Nothing moves -> input `Arc` shared.
//! NOT modelled: vertex sorting, by-vector-dot, `*reverse` toggle next to a key sort.
use super::group_sop::{bad, failed, prim_verts};
use super::selutil::{eval_expr, position, rebuild};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::cmp::Ordering;
use std::sync::Arc;
const WHO: &str = "sort";
pub(super) const NONE: u32 = 0;
const BY_X: u32 = 1;
const BY_Y: u32 = 2;
const BY_Z: u32 = 3;
const BY_ATTR: u32 = 4;
const BY_EXPR: u32 = 5;
const RANDOM: u32 = 6;
const REVERSE: u32 = 7;
const SHIFT: u32 = 8;
const PROXIMITY: u32 = 9;
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("ptsort", ParamValue::Menu(NONE)).label("Point Sort"),
        ParamSpec::new("ptattrib", ParamValue::S(String::new())).label("Point Attribute"),
        ParamSpec::new("ptexpr", ParamValue::S(String::new())).label("Point Expression"),
        ParamSpec::new("ptseed", ParamValue::I(0)).label("Point Seed"),
        ParamSpec::new("ptoffset", ParamValue::I(0)).label("Point Offset"),
        ParamSpec::new("ptproxpos", ParamValue::V3([0.0; 3])).label("Point Proximity Position"),
        ParamSpec::new("primsort", ParamValue::Menu(NONE)).label("Primitive Sort"),
        ParamSpec::new("primattrib", ParamValue::S(String::new())).label("Primitive Attribute"),
        ParamSpec::new("primexpr", ParamValue::S(String::new())).label("Primitive Expression"),
        ParamSpec::new("primseed", ParamValue::I(0)).label("Primitive Seed"),
        ParamSpec::new("primoffset", ParamValue::I(0)).label("Primitive Offset"),
        ParamSpec::new("primproxpos", ParamValue::V3([0.0; 3])).label("Primitive Proximity Position"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Sort { specs: spec() })
}
struct Sort {
    specs: Vec<ParamSpec>,
}
fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
/// Per-element sort key of attribute `name` (tuple, lexicographic; strings ranked by byte order).
fn attr_keys(d: &Detail, class: AttrClass, name: &str) -> Result<Vec<Vec<f64>>, CookError> {
    let a = d.attr(class, name).ok_or_else(|| failed(WHO, format!("no {class:?} attribute `{name}`")))?;
    let n = d.class_len(class);
    let (size, flat): (usize, Vec<f64>) = match &a.data {
        AttrData::F32(v) => (1, v.iter().map(|&x| f64::from(x)).collect()),
        AttrData::I32(v) => (1, v.iter().map(|&x| f64::from(x)).collect()),
        AttrData::V2(v) => (2, v.iter().flatten().map(|&x| f64::from(x)).collect()),
        AttrData::V3(v) => (3, v.iter().flatten().map(|&x| f64::from(x)).collect()),
        AttrData::V4(v) => (4, v.iter().flatten().map(|&x| f64::from(x)).collect()),
        AttrData::FN { size, data } => (*size as usize, data.iter().map(|&x| f64::from(x)).collect()),
        AttrData::IN { size, data } => (*size as usize, data.iter().map(|&x| f64::from(x)).collect()),
        AttrData::Str { idx, table } => {
            let mut sorted: Vec<&String> = table.iter().collect();
            sorted.sort();
            let rank = |k: i32| {
                usize::try_from(k)
                    .ok()
                    .and_then(|k| table.get(k))
                    .map_or(-1.0, |s| sorted.iter().position(|t| *t == s).map_or(-1.0, |r| r as f64))
            };
            (1, idx.iter().map(|&k| rank(k)).collect())
        }
    };
    Ok((0..n).map(|i| flat.get(i * size..(i + 1) * size).map(<[f64]>::to_vec).unwrap_or_default()).collect())
}
fn cmp_keys(a: &[f64], b: &[f64]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let o = x.total_cmp(y);
        if o != Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}
/// `perm[new] = old` for one class. `pre` = param prefix (`pt` | `prim`).
fn order(d: &Detail, class: AttrClass, p: &ParamSet, pre: &str) -> Result<Vec<usize>, CookError> {
    let n = d.class_len(class);
    let ident: Vec<usize> = (0..n).collect();
    let mode = p.menu(&format!("{pre}sort")).ok_or_else(|| bad(WHO, format!("`{pre}sort` missing or not a menu")))?;
    let by_keys = |keys: Vec<Vec<f64>>| -> Vec<usize> {
        let mut o = ident.clone();
        o.sort_by(|&a, &b| cmp_keys(keys.get(a).map_or(&[], |k| k), keys.get(b).map_or(&[], |k| k)));
        o
    };
    let coord =
        |axis: usize| by_keys((0..n).map(|i| vec![position(d, class, i).get(axis).copied().unwrap_or(0.0)]).collect());
    Ok(match mode {
        NONE => ident,
        BY_X => coord(0),
        BY_Y => coord(1),
        BY_Z => coord(2),
        BY_ATTR => {
            let name = p.string(&format!("{pre}attrib")).ok_or_else(|| bad(WHO, "attrib param missing"))?;
            by_keys(attr_keys(d, class, name.trim())?)
        }
        BY_EXPR => {
            let src = p.string(&format!("{pre}expr")).ok_or_else(|| bad(WHO, "expr param missing"))?;
            by_keys(eval_expr(d, class, src, WHO)?.into_iter().map(|x| vec![x]).collect())
        }
        RANDOM => {
            let seed = p.int(&format!("{pre}seed")).ok_or_else(|| bad(WHO, "seed param missing"))?;
            let s = splitmix(i64::from(seed) as u64);
            by_keys((0..n).map(|i| vec![(splitmix(s ^ i as u64) >> 11) as f64]).collect())
        }
        REVERSE => ident.into_iter().rev().collect(),
        SHIFT => {
            let off = p.int(&format!("{pre}offset")).ok_or_else(|| bad(WHO, "offset param missing"))?;
            let k = (i64::from(off)).rem_euclid(n.max(1) as i64) as usize;
            (0..n).map(|i| (i + k) % n.max(1)).collect()
        }
        PROXIMITY => {
            let c = p.vec3(&format!("{pre}proxpos")).ok_or_else(|| bad(WHO, "proxpos param missing"))?;
            if !c.iter().all(|v| v.is_finite()) {
                return Err(bad(WHO, "non-finite proximity position"));
            }
            by_keys(
                (0..n)
                    .map(|i| {
                        let q = position(d, class, i);
                        vec![(0..3).map(|k| (q[k] - f64::from(c[k])).powi(2)).sum::<f64>()]
                    })
                    .collect(),
            )
        }
        m => return Err(bad(WHO, format!("`{pre}sort`: menu index {m} out of range (0..=9)"))),
    })
}
fn is_identity(p: &[usize]) -> bool {
    p.iter().enumerate().all(|(i, &o)| i == o)
}
impl Node<Detail> for Sort {
    fn type_name(&self) -> &'static str {
        "sort"
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let Some(d) = inputs.first() else { return Ok(Arc::default()) };
        let pperm = order(d, AttrClass::Point, ctx.params, "pt")?;
        let qperm = order(d, AttrClass::Prim, ctx.params, "prim")?;
        if is_identity(&pperm) && is_identity(&qperm) {
            return Ok(Arc::clone(d));
        }
        let prims: Vec<(usize, Vec<usize>)> = qperm
            .iter()
            .map(|&q| (q, d.prims.get(q).map(|pr| prim_verts(d, pr).collect()).unwrap_or_default()))
            .collect();
        Ok(Arc::new(rebuild(d, &pperm, &prims, WHO)?))
    }
}
#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use super::*;
    use hexe_geo::{Attribute, TypeInfo};
    fn sort(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        run("sort", ov, Some(d))
    }
    fn m(v: u32) -> ParamValue {
        ParamValue::Menu(v)
    }
    /// Points at the given x positions (y = z = 0), a point attribute `id` = original index, one open polyline through all.
    fn line_of(xs: &[f32]) -> Arc<Detail> {
        let mut d = Detail::default();
        let ids: Vec<u32> = d.add_points(xs.len() as u32).collect();
        *d.p_mut() = xs.iter().map(|&x| [x, 0.0, 0.0]).collect();
        d.add_attr(Attribute {
            name: "id".into(),
            class: AttrClass::Point,
            data: AttrData::I32((0..xs.len() as i32).collect()),
            type_info: TypeInfo::None,
        });
        d.add_poly(&ids, false);
        Arc::new(d)
    }
    fn xs(d: &Detail) -> Vec<f32> {
        d.p().iter().map(|q| q[0]).collect()
    }
    fn ids(d: &Detail) -> Vec<i32> {
        match d.attr(AttrClass::Point, "id").map(|a| &a.data) {
            Some(AttrData::I32(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
    /// x positions as visited along the (single) prim: geometry-preserving sorts leave this unchanged.
    fn walk(d: &Detail) -> Vec<f32> {
        d.vertex_point.iter().map(|&q| d.p()[q as usize][0]).collect()
    }
    /// 3x3 point lattice, x = c-1, y = r-1, point id = r*3+c; 4 quads (r,c) in row-major order.
    fn lattice() -> Arc<Detail> {
        let mut d = Detail::default();
        d.add_points(9);
        *d.p_mut() = (0..9).map(|i| [(i % 3) as f32 - 1.0, (i / 3) as f32 - 1.0, 0.0]).collect();
        for r in 0..2u32 {
            for c in 0..2u32 {
                let b = r * 3 + c;
                d.add_poly(&[b, b + 1, b + 4, b + 3], true);
            }
        }
        Arc::new(d)
    }
    #[test]
    fn params_and_defaults() {
        let n = new();
        let got: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            got,
            [
                "ptsort",
                "ptattrib",
                "ptexpr",
                "ptseed",
                "ptoffset",
                "ptproxpos",
                "primsort",
                "primattrib",
                "primexpr",
                "primseed",
                "primoffset",
                "primproxpos"
            ]
        );
        assert!(n.params().iter().all(|s| s.name.ends_with("sort") == matches!(s.default, ParamValue::Menu(NONE))));
    }
    #[test]
    fn line_of_5_points_reversed_is_exact_order_and_geometry_preserved() {
        let d = line_of(&[0.0, 1.0, 2.0, 3.0, 4.0]);
        let o = sort(&d, &[("ptsort", m(REVERSE))]).expect("cooks");
        assert_eq!(xs(&o), [4.0, 3.0, 2.0, 1.0, 0.0]);
        assert_eq!(ids(&o), [4, 3, 2, 1, 0], "point attributes travel");
        assert_eq!(o.vertex_point, [4, 3, 2, 1, 0], "vertex refs remapped through the permutation");
        assert_eq!(walk(&o), walk(&d), "the polyline still visits the same positions in the same order");
        assert_eq!((o.point_count, o.prim_count()), (5, 1));
        let back = sort(&o, &[("ptsort", m(REVERSE))]).expect("cooks");
        assert_eq!(*back, *d, "reverse twice = identity");
        // none = shared Arc
        assert!(Arc::ptr_eq(&sort(&d, &[]).expect("cooks"), &d));
    }
    #[test]
    fn by_axis_attribute_and_expression() {
        let d = line_of(&[3.0, 1.0, 4.0, 0.0, 2.0]);
        let by_x = sort(&d, &[("ptsort", m(BY_X))]).expect("cooks");
        assert_eq!(xs(&by_x), [0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(ids(&by_x), [3, 1, 4, 0, 2]);
        assert_eq!(walk(&by_x), walk(&d));
        // perm[new]=old=[3,1,4,0,2] -> inverse applied to vertex_point [0,1,2,3,4] (an involution here)
        assert_eq!(by_x.vertex_point, [3, 1, 4, 0, 2]);
        // y and z are all zero -> stable: input order
        assert_eq!(ids(&sort(&d, &[("ptsort", m(BY_Y))]).expect("cooks")), [0, 1, 2, 3, 4]);
        assert_eq!(ids(&sort(&d, &[("ptsort", m(BY_Z))]).expect("cooks")), [0, 1, 2, 3, 4]);
        let attr = sort(&d, &[("ptsort", m(BY_ATTR)), ("ptattrib", s("id"))]).expect("cooks");
        assert!(Arc::ptr_eq(&attr, &d), "already ordered by id -> shared");
        let e = sort(&d, &[("ptsort", m(BY_EXPR)), ("ptexpr", s("-@P.x"))]).expect("cooks");
        assert_eq!(xs(&e), [4.0, 3.0, 2.0, 1.0, 0.0]);
        let e = sort(&d, &[("ptsort", m(BY_EXPR)), ("ptexpr", s("@ptnum % 2"))]).expect("cooks");
        assert_eq!(ids(&e), [0, 2, 4, 1, 3], "evens then odds, stable");
    }
    #[test]
    fn shift_proximity_and_random() {
        let d = line_of(&[0.0, 1.0, 2.0, 3.0, 4.0]);
        let sh = sort(&d, &[("ptsort", m(SHIFT)), ("ptoffset", ParamValue::I(2))]).expect("cooks");
        assert_eq!(ids(&sh), [2, 3, 4, 0, 1]);
        let sh = sort(&d, &[("ptsort", m(SHIFT)), ("ptoffset", ParamValue::I(-1))]).expect("cooks");
        assert_eq!(ids(&sh), [4, 0, 1, 2, 3]);
        assert!(Arc::ptr_eq(&sort(&d, &[("ptsort", m(SHIFT)), ("ptoffset", ParamValue::I(5))]).expect("cooks"), &d));
        let px = sort(&d, &[("ptsort", m(PROXIMITY)), ("ptproxpos", ParamValue::V3([3.2, 0.0, 0.0]))]).expect("cooks");
        assert_eq!(ids(&px), [3, 4, 2, 1, 0], "distances .2 .8 1.2 2.2 3.2");
        let r = |seed: i32| ids(&sort(&d, &[("ptsort", m(RANDOM)), ("ptseed", ParamValue::I(seed))]).expect("cooks"));
        let (a, b) = (r(1), r(1));
        assert_eq!(a, b, "deterministic");
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 2, 3, 4], "a permutation");
        assert!((2..40).any(|s| r(s) != a), "seed changes the order");
        let o = sort(&d, &[("ptsort", m(RANDOM)), ("ptseed", ParamValue::I(7))]).expect("cooks");
        let mut w = walk(&o);
        w.sort_by(f32::total_cmp);
        assert_eq!(walk(&o), walk(&d), "random point sort never changes the geometry");
        assert_eq!(w, [0.0, 1.0, 2.0, 3.0, 4.0]);
    }
    #[test]
    fn prim_sorts_rebuild_vertices_and_carry_attrs() {
        let mut g = (*lattice()).clone();
        g.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Prim,
            data: AttrData::F32(vec![10.0, 11.0, 12.0, 13.0]),
            type_info: TypeInfo::None,
        });
        g.add_attr(Attribute {
            name: "vid".into(),
            class: AttrClass::Vertex,
            data: AttrData::I32((0..16).collect()),
            type_info: TypeInfo::None,
        });
        g.groups.push(hexe_geo::Group {
            name: "first".into(),
            class: AttrClass::Prim,
            mask: vec![true, false, false, false],
        });
        let g = Arc::new(g);
        let o = sort(&g, &[("primsort", m(REVERSE))]).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (9, 4));
        assert_eq!(o.p(), g.p(), "points untouched");
        let Some(AttrData::F32(w)) = o.attr(AttrClass::Prim, "w").map(|a| &a.data) else { panic!("w lost") };
        assert_eq!(w, &[13.0, 12.0, 11.0, 10.0]);
        // new prim 0 = old prim 3 = quad (1,1): points 4,5,8,7; verts are contiguous per prim
        assert_eq!(o.vertex_point[0..4], [4, 5, 8, 7]);
        assert_eq!(o.vertex_point[12..16], [0, 1, 4, 3]);
        assert!(o.prims.iter().enumerate().all(|(i, p)| p.verts == ((i as u32 * 4)..(i as u32 * 4 + 4))));
        let Some(AttrData::I32(v)) = o.attr(AttrClass::Vertex, "vid").map(|a| &a.data) else { panic!("vid lost") };
        assert_eq!(v[0..4], [12, 13, 14, 15]);
        assert_eq!(o.group(AttrClass::Prim, "first").map(|g| g.mask.clone()), Some(vec![false, false, false, true]));
        // by y descending: quads in row 1 (centroid y .5) before row 0 (-.5)
        let o = sort(&g, &[("primsort", m(BY_EXPR)), ("primexpr", s("-@P.y"))]).expect("cooks");
        let Some(AttrData::F32(w)) = o.attr(AttrClass::Prim, "w").map(|a| &a.data) else { panic!("w lost") };
        assert_eq!(w, &[12.0, 13.0, 10.0, 11.0]);
        // by x ascending (stable): centroid x = -.5 (c=0) then .5 (c=1)
        let o = sort(&g, &[("primsort", m(BY_X))]).expect("cooks");
        let Some(AttrData::F32(w)) = o.attr(AttrClass::Prim, "w").map(|a| &a.data) else { panic!("w lost") };
        assert_eq!(w, &[10.0, 12.0, 11.0, 13.0]);
        // prim by primnum expression; point + prim sorts combine
        let both = sort(&g, &[("ptsort", m(REVERSE)), ("primsort", m(REVERSE))]).expect("cooks");
        assert_eq!(both.vertex_point[0..4], [4, 3, 0, 1], "old quad (1,1) = points 4,5,8,7 -> 4,3,0,1");
    }
    #[test]
    fn string_attribute_sort_and_hostile() {
        let mut g = (*line_of(&[0.0, 1.0, 2.0])).clone();
        g.add_attr(Attribute {
            name: "name".into(),
            class: AttrClass::Point,
            data: AttrData::Str { idx: vec![0, 1, 2], table: vec!["pear".into(), "apple".into(), "fig".into()] },
            type_info: TypeInfo::None,
        });
        let g = Arc::new(g);
        let o = sort(&g, &[("ptsort", m(BY_ATTR)), ("ptattrib", s("name"))]).expect("cooks");
        assert_eq!(ids(&o), [1, 2, 0], "apple fig pear");
        assert!(matches!(sort(&g, &[("ptsort", m(BY_ATTR)), ("ptattrib", s("nope"))]), Err(CookError::Failed(_))));
        assert!(matches!(sort(&g, &[("ptsort", m(10))]), Err(CookError::BadParam(_))));
        assert!(matches!(sort(&g, &[("primsort", m(10))]), Err(CookError::BadParam(_))));
        assert!(matches!(sort(&g, &[("ptsort", m(BY_EXPR))]), Err(CookError::BadParam(_))), "empty expression");
        assert!(matches!(sort(&g, &[("ptsort", m(BY_EXPR)), ("ptexpr", s("@P.x +"))]), Err(CookError::BadParam(_))));
        assert!(matches!(sort(&g, &[("ptsort", m(BY_EXPR)), ("ptexpr", s("@P"))]), Err(CookError::Failed(_))));
        let nan = ParamValue::V3([f32::NAN, 0.0, 0.0]);
        assert!(matches!(sort(&g, &[("ptsort", m(PROXIMITY)), ("ptproxpos", nan)]), Err(CookError::BadParam(_))));
        assert_eq!(run("sort", &[("ptsort", m(REVERSE))], None).expect("cooks").point_count, 0);
        let e = Arc::new(Detail::default());
        for mode in 0..10 {
            let r = sort(
                &e,
                &[
                    ("ptsort", m(mode)),
                    ("primsort", m(mode)),
                    ("ptexpr", s("1")),
                    ("primexpr", s("1")),
                    ("ptattrib", s("P")),
                    ("primattrib", s("P")),
                ],
            );
            assert!(r.is_ok() || mode == BY_ATTR, "empty input mode {mode}");
        }
    }
    #[test]
    fn scene_loader_cooks_line_reverse() {
        let s = r#"{"nodes":[
            {"name":"l","type":"line","params":{"points":5,"dist":4.0,"dir":[1,0,0]}},
            {"name":"s","type":"sort","params":{"ptsort":7}}],
            "edges":[["l",0,"s",0]],"display":"s"}"#;
        let (mut g, id) = crate::load_scene(s).expect("loads");
        let out = g.cook(id, 0.0).expect("cooks");
        assert_eq!(xs(&out), [4.0, 3.0, 2.0, 1.0, 0.0]);
        assert_eq!(out.vertex_point, [4, 3, 2, 1, 0]);
        assert_eq!(crate::registry().create("sort").map(|n| n.type_name()), Some("sort"));
    }
}
