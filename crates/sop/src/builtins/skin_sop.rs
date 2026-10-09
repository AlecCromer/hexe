//! Skin SOP (token `skin`): loft a surface through the curves of input 0 (one curve prim = one section, in prim order).
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! * Sections = every `Poly`/`Polyline` prim with >= 2 corners; fewer than 2 sections = empty output. The surface is closed
//!   around `u` (the ring direction) iff EVERY section is closed; otherwise it is open and the sections' point lists are used
//!   as they are.
//! * `surftype` menu `quads` (0, default) | `triangles` (1: each quad split along its first diagonal).
//! * `closev` B off: also loft the LAST section back to the FIRST (needs >= 3 sections).
//! * `resample` B on: sections with differing point counts are resampled by arclength to the LARGEST count (closed: evenly
//!   around the loop starting at the first point; open: end points kept); off + differing counts = `Failed`. Equal counts
//!   use the input positions unchanged (no resampling).
//! * Quad `(s, j)` = `[(s,j), (s+1,j), (s+1,j+1), (s,j+1)]` reversed to `A,D,C,B`: the Houdini normal is `u x v` (ring direction x
//!   loft direction), outward for CCW rings lofted along their axis.
//! * `order` menu `prim`(0) | `reverse`(1) | `x`/`y`/`z`(2..=4: sections sorted ascending by their mean coordinate).
//! * `divs` I 1 (1..=256): stations per section pair (the last section closes an open loft). `keepshape` B on = straight
//!   ruled loft (lerp between sections); off = Catmull-Rom (Barry-Goldman) interpolation through the neighbouring sections
//!   (end sections extrapolate linearly, `closev` wraps) - exact for quadratic profiles at uniform spacing. `uniform` B on
//!   = uniform knots; off = chord-length knots (mean point-to-point distance between sections).
//! * `carryattrs` B on: every input point attr (not `P`) is inherited by the output points through the same linear weights
//!   as the position (resampled points blend their two neighbours, intermediate stations blend the two sections; floats =
//!   weighted mean, int/str = heaviest source); prim attrs: quads between sections `s` and `s+1` copy section `s`'s prim.
//! * Output = new points + vertex `uv` V3 (`u` = column / columns, `v` = station / stations). NOT modelled (DEFERRED):
//!   NURBS output.
use super::meshutil::{V3, add, bad, failed, positions, scale, sub, toggle};
use super::surfutil::{Curve, Mesh, curves, dist};
use crate::SopNode;
use hexe_geo::Detail;
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const OP: &str = "skin";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("surftype", ParamValue::Menu(0))
            .label("Surface Type")
            .menu(&[("quads", "Quadrilaterals"), ("triangles", "Triangles")]),
        ParamSpec::new("closev", ParamValue::B(false)).label("Close V"),
        ParamSpec::new("resample", ParamValue::B(true)).label("Resample Mismatched Sections"),
        ParamSpec::new("order", ParamValue::Menu(0)).label("Section Order").menu(&[
            ("prim", "Primitive Order"),
            ("reverse", "Reverse"),
            ("x", "Sort By X"),
            ("y", "Sort By Y"),
            ("z", "Sort By Z"),
        ]),
        ParamSpec::new("divs", ParamValue::I(1)).label("Divisions Between Sections").range(1.0, 16.0),
        ParamSpec::new("keepshape", ParamValue::B(true)).label("Keep Shape (Straight Loft)"),
        ParamSpec::new("uniform", ParamValue::B(true)).label("Uniform Knots"),
        ParamSpec::new("carryattrs", ParamValue::B(true)).label("Inherit Attributes"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Skin { specs: spec() })
}
struct Skin {
    specs: Vec<ParamSpec>,
}

/// `n` points evenly spaced by arclength along `pos` (closed: loop, last point NOT repeated; open: end points kept). Each
/// result carries its position and the two source indices (into `pos`) with linear weights.
fn resample(pos: &[V3], closed: bool, n: usize) -> Vec<(V3, Vec<(usize, f64)>)> {
    let mut path = pos.to_vec();
    if closed {
        path.push(pos[0]);
    }
    let mut acc = vec![0.0f64];
    for k in 1..path.len() {
        acc.push(acc[k - 1] + dist(path[k - 1], path[k]));
    }
    let total = acc[acc.len() - 1];
    if total <= 0.0 {
        return (0..n).map(|_| (pos[0], vec![(0, 1.0)])).collect();
    }
    let mut seg = 0;
    (0..n)
        .map(|k| {
            let t = total * k as f64 / if closed { n as f64 } else { (n - 1) as f64 };
            while seg + 2 < path.len() && acc[seg + 1] < t {
                seg += 1;
            }
            let len = acc[seg + 1] - acc[seg];
            let f = if len > 0.0 { ((t - acc[seg]) / len).clamp(0.0, 1.0) } else { 0.0 };
            let (a, b) = (seg % pos.len(), (seg + 1) % pos.len());
            (add(path[seg], scale(sub(path[seg + 1], path[seg]), f)), vec![(a, 1.0 - f), (b, f)])
        })
        .collect()
}

/// One section as loft stations: positions + attribute blend weights (over SOURCE point ids).
#[derive(Clone)]
struct Row {
    p: Vec<V3>,
    w: Vec<Vec<(usize, f64)>>,
}

fn lerp_row(a: &Row, b: &Row, t: f64) -> Row {
    let p = a.p.iter().zip(&b.p).map(|(x, y)| add(scale(*x, 1.0 - t), scale(*y, t))).collect();
    let w =
        a.w.iter()
            .zip(&b.w)
            .map(|(x, y)| {
                x.iter()
                    .map(|&(i, v)| (i, v * (1.0 - t)))
                    .chain(y.iter().map(|&(i, v)| (i, v * t)))
                    .filter(|c| c.1 > 0.0)
                    .collect()
            })
            .collect();
    Row { p, w }
}

/// Barry-Goldman Catmull-Rom through `p[1]..p[2]` with knots `k`, at parameter `t` in `[k1, k2]`.
fn barry_goldman(p: [V3; 4], k: [f64; 4], t: f64) -> V3 {
    let mix = |a: V3, b: V3, ka: f64, kb: f64| {
        let d = kb - ka;
        add(scale(a, (kb - t) / d), scale(b, (t - ka) / d))
    };
    let a1 = mix(p[0], p[1], k[0], k[1]);
    let a2 = mix(p[1], p[2], k[1], k[2]);
    let a3 = mix(p[2], p[3], k[2], k[3]);
    let b1 = mix(a1, a2, k[0], k[2]);
    let b2 = mix(a2, a3, k[1], k[3]);
    mix(b1, b2, k[1], k[2])
}

/// Mean point-to-point distance of two rows (chord length of the loft direction).
fn row_dist(a: &Row, b: &Row) -> f64 {
    a.p.iter().zip(&b.p).map(|(x, y)| dist(*x, *y)).sum::<f64>() / a.p.len().max(1) as f64
}

/// Station between sections `rows[s]` and `rows[s2]` at `t` in `[0, 1)`; `smooth` = Catmull-Rom through the neighbouring
/// sections (uniform or chord-length knots), else a straight lerp. Attribute weights always lerp.
fn station(rows: &[Row], s: usize, s2: usize, t: f64, wrap: bool, smooth: bool, uniform: bool) -> Row {
    let (a, b) = (&rows[s], &rows[s2]);
    let mut r = lerp_row(a, b, t);
    if !smooth || t == 0.0 || rows.len() < 3 {
        return r;
    }
    let m = rows.len();
    let ghost = |edge: usize, inner: usize| Row {
        p: rows[edge].p.iter().zip(&rows[inner].p).map(|(e, i)| sub(scale(*e, 2.0), *i)).collect(),
        w: rows[edge].w.clone(),
    };
    let before = if s > 0 {
        rows[s - 1].clone()
    } else if wrap {
        rows[m - 1].clone()
    } else {
        ghost(s, s2)
    };
    let after = if s2 + 1 < m {
        rows[s2 + 1].clone()
    } else if wrap {
        rows[(s2 + 1) % m].clone()
    } else {
        ghost(s2, s)
    };
    let k = if uniform {
        [0.0, 1.0, 2.0, 3.0]
    } else {
        let (d01, d12, d23) = (row_dist(&before, a), row_dist(a, b), row_dist(b, &after));
        [0.0, d01, d01 + d12, d01 + d12 + d23]
    };
    if k[1] - k[0] < 1e-12 || k[2] - k[1] < 1e-12 || k[3] - k[2] < 1e-12 {
        return r;
    }
    let tt = k[1] + (k[2] - k[1]) * t;
    for j in 0..r.p.len() {
        r.p[j] = barry_goldman([before.p[j], a.p[j], b.p[j], after.p[j]], k, tt);
    }
    r
}

const ORDER_PRIM: u32 = 0;
const ORDER_REVERSE: u32 = 1;
const MAX_DIVS: i32 = 256;

impl Node<Detail> for Skin {
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
        let tris = match p.menu("surftype") {
            Some(v @ 0..=1) => v == 1,
            Some(v) => return Err(bad(format!("skin: `surftype`: menu index {v} out of range 0..=1"))),
            None => return Err(bad("skin: `surftype`: missing or not a menu")),
        };
        let order = match p.menu("order") {
            Some(v @ 0..=4) => v,
            Some(v) => return Err(bad(format!("skin: `order`: menu index {v} out of range 0..=4"))),
            None => return Err(bad("skin: `order`: missing or not a menu")),
        };
        let divs = match p.int("divs") {
            Some(d) if (1..=MAX_DIVS).contains(&d) => d as usize,
            _ => return Err(bad(format!("skin: `divs` must be an int in 1..={MAX_DIVS}"))),
        };
        let (closev, rs) = (toggle(p, "closev")?, toggle(p, "resample")?);
        let (keepshape, uniform, carry) = (toggle(p, "keepshape")?, toggle(p, "uniform")?, toggle(p, "carryattrs")?);
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = positions(src);
        let mut secs = curves(src);
        if secs.len() < 2 {
            return Ok(Arc::new(Detail::default()));
        }
        match order {
            ORDER_PRIM => {}
            ORDER_REVERSE => secs.reverse(),
            axis => {
                let ax = (axis - 2) as usize;
                let key = |c: &Curve| c.pts.iter().map(|&i| pos[i as usize][ax]).sum::<f64>() / c.pts.len() as f64;
                secs.sort_by(|a, b| key(a).total_cmp(&key(b)));
            }
        }
        let closed_u = secs.iter().all(|c| c.closed);
        let n = secs.iter().map(|c| c.pts.len()).max().unwrap_or(0);
        if !rs && secs.iter().any(|c| c.pts.len() != n) {
            return Err(failed(OP, "sections have different point counts and `resample` is off"));
        }
        let mut rows: Vec<Row> = Vec::with_capacity(secs.len());
        for c in &secs {
            let q: Vec<V3> = c.pts.iter().map(|&i| pos[i as usize]).collect();
            let row = if q.len() == n {
                Row { p: q, w: c.pts.iter().map(|&i| vec![(i as usize, 1.0)]).collect() }
            } else {
                let r = resample(&q, c.closed, n);
                Row {
                    p: r.iter().map(|x| x.0).collect(),
                    w: r.iter().map(|x| x.1.iter().map(|&(l, f)| (c.pts[l] as usize, f)).collect()).collect(),
                }
            };
            rows.push(row);
        }
        let s_count = rows.len();
        let wrap_v = closev && s_count >= 3;
        let v_segs = if wrap_v { s_count } else { s_count - 1 };
        // stations: `divs` per section pair (+ the last section when open)
        let mut stations: Vec<Row> = Vec::with_capacity(v_segs * divs + 1);
        let mut station_sec: Vec<usize> = Vec::new();
        for s in 0..v_segs {
            let s2 = (s + 1) % s_count;
            for d in 0..divs {
                stations.push(station(&rows, s, s2, d as f64 / divs as f64, wrap_v, !keepshape, uniform));
                station_sec.push(s);
            }
        }
        if !wrap_v {
            stations.push(rows[s_count - 1].clone());
        }
        let mut m = Mesh::default();
        let mut ids: Vec<Vec<u32>> = Vec::with_capacity(stations.len());
        let mut wts: Vec<Vec<(usize, f64)>> = Vec::new();
        for st in &stations {
            let mut row = Vec::with_capacity(n);
            for (pt, w) in st.p.iter().zip(&st.w) {
                row.push(m.point(OP, *pt)?);
                wts.push(w.clone());
            }
            ids.push(row);
        }
        let r_count = ids.len();
        let quads = if wrap_v { r_count } else { r_count - 1 };
        let u_segs = if closed_u { n } else { n - 1 };
        for k in 0..quads {
            let k2 = (k + 1) % r_count;
            m.cur_prim = Some(secs[station_sec[k]].prim);
            let (v0, v1) = (k as f32 / quads as f32, (k + 1) as f32 / quads as f32);
            for j in 0..u_segs {
                let j2 = (j + 1) % n;
                let (u0, u1) = (j as f32 / u_segs as f32, (j + 1) as f32 / u_segs as f32);
                m.quad(
                    OP,
                    [ids[k][j], ids[k2][j], ids[k2][j2], ids[k][j2]],
                    [[u0, v0, 0.0], [u0, v1, 0.0], [u1, v1, 0.0], [u1, v0, 0.0]],
                    tris,
                )?;
            }
        }
        Ok(Arc::new(if carry { m.finish_blended(src, &wts) } else { m.finish() }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::{positions, prim_normal, testkit};
    use super::super::surfutil::testkit::curve;
    use super::*;
    use crate::load_scene;
    use hexe_geo::{AttrClass, AttrData};
    use hexe_graph::ParamSet;

    fn run(d: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        n.cook(&ctx, &[Arc::new(d)])
    }
    fn defaults() -> ParamSet {
        testkit::defaults(&*new())
    }
    fn sq(z: f32, s: f32) -> Detail {
        curve(&[[-s, -s, z], [s, -s, z], [s, s, z], [-s, s, z]], true)
    }
    fn stack(zs: &[(f32, f32)]) -> Detail {
        let mut d = sq(zs[0].0, zs[0].1);
        for &(z, s) in &zs[1..] {
            d.merge(&sq(z, s));
        }
        d
    }

    fn with_attr(mut d: Detail, name: &str, class: AttrClass, data: AttrData) -> Detail {
        d.add_attr(hexe_geo::Attribute { name: name.into(), class, data, type_info: hexe_geo::TypeInfo::None });
        d
    }

    #[test]
    fn divs_insert_linear_stations_between_sections() {
        let src = stack(&[(0.0, 0.5), (1.0, 0.5), (2.0, 0.5)]);
        let d = run(src, defaults().with("divs", ParamValue::I(3))).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (7 * 4, 6 * 4));
        for k in 0..7 {
            assert!((d.p()[4 * k][2] - k as f32 / 3.0).abs() < 1e-6, "{k} {:?}", d.p()[4 * k]);
        }
        // closev wraps: 3 sections, divs 2 -> 6 stations, 6 quad rings
        let c = run(
            stack(&[(0.0, 0.5), (1.0, 0.5), (2.0, 0.5)]),
            defaults().with("divs", ParamValue::I(2)).with("closev", ParamValue::B(true)),
        )
        .expect("cooks");
        assert_eq!((c.point_count, c.prim_count()), (6 * 4, 6 * 4));
    }

    #[test]
    fn smooth_keepshape_off_is_catmull_rom_exact_for_quadratic_profiles() {
        // ring half-size s = 1 + z^2 at z = 0..3; divs 2 -> station 3 is z = 1.5 -> s = 3.25 (linear would give 3.5)
        let src = stack(&[(0.0, 1.0), (1.0, 2.0), (2.0, 5.0), (3.0, 10.0)]);
        let lin = run(src.clone(), defaults().with("divs", ParamValue::I(2))).expect("cooks");
        let cr =
            run(src, defaults().with("divs", ParamValue::I(2)).with("keepshape", ParamValue::B(false))).expect("cooks");
        assert_eq!(cr.point_count, 7 * 4);
        assert!((lin.p()[12][0] + 3.5).abs() < 1e-6 && (lin.p()[12][2] - 1.5).abs() < 1e-6);
        assert!((cr.p()[12][0] + 3.25).abs() < 1e-6 && (cr.p()[12][2] - 1.5).abs() < 1e-6, "{:?}", cr.p()[12]);
        // the outer segments extrapolate linearly: station 1 (z=0.5) = 1 + 0.25 + ghost correction, finite and between 1 and 2
        assert!(cr.p()[4][0] < -1.0 && cr.p()[4][0] > -2.0 && (cr.p()[4][2] - 0.5).abs() < 1e-6);
        // sections themselves are untouched by smoothing
        assert_eq!(cr.p()[8][0], -2.0);
        // chord-length knots on equally spaced linear sections give the same exact midpoint (2.5)
        let eq = stack(&[(0.0, 1.0), (1.0, 2.0), (2.0, 3.0), (3.0, 4.0)]);
        let ch = run(
            eq,
            defaults()
                .with("divs", ParamValue::I(2))
                .with("keepshape", ParamValue::B(false))
                .with("uniform", ParamValue::B(false)),
        )
        .expect("cooks");
        assert!((ch.p()[12][0] + 2.5).abs() < 1e-6, "{:?}", ch.p()[12]);
    }

    #[test]
    fn order_menu_reorders_the_sections() {
        let src = stack(&[(2.0, 0.5), (0.0, 0.5), (1.0, 0.5)]);
        let z0 = |o: u32| run(src.clone(), defaults().with("order", ParamValue::Menu(o))).expect("cooks").p()[0][2];
        assert_eq!((z0(0), z0(1), z0(4)), (2.0, 1.0, 0.0));
        // by x: all sections share x -> stable sort keeps prim order
        assert_eq!(z0(2), 2.0);
    }

    #[test]
    fn point_and_prim_attrs_are_inherited_with_the_loft_weights() {
        let src = with_attr(
            stack(&[(0.0, 0.5), (1.0, 0.5)]),
            "t",
            AttrClass::Point,
            AttrData::F32(vec![0.0, 1.0, 2.0, 3.0, 10.0, 11.0, 12.0, 13.0]),
        );
        let src = with_attr(src, "cls", AttrClass::Prim, AttrData::F32(vec![1.0, 2.0]));
        let d = run(src, defaults().with("divs", ParamValue::I(2))).expect("cooks");
        assert_eq!(
            d.attr(AttrClass::Point, "t").map(|a| a.data.clone()),
            Some(AttrData::F32(vec![0.0, 1.0, 2.0, 3.0, 5.0, 6.0, 7.0, 8.0, 10.0, 11.0, 12.0, 13.0]))
        );
        assert_eq!(d.attr(AttrClass::Prim, "cls").map(|a| a.data.clone()), Some(AttrData::F32(vec![1.0; 8])));
        // resampled section: 2-point line gets 3 points; the middle one blends both ends
        let mut b = curve(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], false);
        b.merge(&curve(&[[0.0, 0.0, 1.0], [2.0, 0.0, 1.0]], false));
        let b = with_attr(b, "t", AttrClass::Point, AttrData::F32(vec![0.0, 10.0, 20.0, 0.0, 20.0]));
        let r = run(b.clone(), defaults()).expect("cooks");
        assert_eq!(r.point_count, 6);
        assert_eq!(
            r.attr(AttrClass::Point, "t").map(|a| a.data.clone()),
            Some(AttrData::F32(vec![0.0, 10.0, 20.0, 0.0, 10.0, 20.0]))
        );
        assert!((r.p()[4][0] - 1.0).abs() < 1e-6);
        // carry off
        let n = run(b, defaults().with("carryattrs", ParamValue::B(false))).expect("cooks");
        assert!(n.attr(AttrClass::Point, "t").is_none());
    }

    #[test]
    fn new_params_reject_bad_values() {
        for (k, v) in [
            ("divs", ParamValue::I(0)),
            ("divs", ParamValue::I(257)),
            ("order", ParamValue::Menu(5)),
            ("keepshape", ParamValue::I(1)),
        ] {
            assert!(
                matches!(run(stack(&[(0.0, 0.5), (1.0, 0.5)]), defaults().with(k, v)), Err(CookError::BadParam(_))),
                "{k}"
            );
        }
    }

    #[test]
    fn two_squares_loft_into_four_outward_quads() {
        let d = run(stack(&[(0.0, 0.5), (1.0, 0.5)]), defaults()).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (8, 4));
        let pos = positions(&d);
        for pr in &d.prims {
            let ids: Vec<usize> =
                d.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&i| i as usize).collect();
            let nrm = prim_normal(&pos, &ids).expect("normal");
            let c: V3 = ids.iter().fold([0.0; 3], |a, &i| add(a, scale(pos[i], 0.25)));
            assert!(nrm[2].abs() < 1e-9 && nrm[0] * c[0] + nrm[1] * c[1] > 0.0);
        }
        let t = run(stack(&[(0.0, 0.5), (1.0, 0.5)]), defaults().with("surftype", ParamValue::Menu(1))).expect("cooks");
        assert_eq!(t.prim_count(), 8);
        assert!(t.prims.iter().all(|p| p.verts.len() == 3));
    }

    #[test]
    fn closev_wraps_three_sections_and_uv_spans_the_unit_square() {
        let s = stack(&[(0.0, 0.5), (1.0, 1.0), (2.0, 0.5)]);
        assert_eq!(run(s.clone(), defaults()).expect("cooks").prim_count(), 8);
        let d = run(s, defaults().with("closev", ParamValue::B(true))).expect("cooks");
        assert_eq!(d.prim_count(), 12);
        let uv = match d.attr(AttrClass::Vertex, "uv").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v.clone(),
            o => panic!("{o:?}"),
        };
        assert_eq!(uv.len(), d.vertex_point.len());
        assert!(uv.iter().any(|c| c[0] == 1.0) && uv.iter().any(|c| c[1] == 1.0));
        // closev with only 2 sections is ignored (would double the same strip)
        assert_eq!(
            run(stack(&[(0.0, 1.0), (1.0, 1.0)]), defaults().with("closev", ParamValue::B(true)))
                .expect("cooks")
                .prim_count(),
            4
        );
    }

    #[test]
    fn mismatched_counts_resample_to_the_largest_or_fail() {
        let mut d = sq(0.0, 1.0);
        d.merge(&curve(&[[1.0, 0.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, -1.0, 1.0]], true));
        let o = run(d.clone(), defaults()).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (8, 4));
        // the resampled triangle keeps its first point and walks the loop
        assert_eq!(o.p()[4], [1.0, 0.0, 1.0]);
        assert!(matches!(run(d, defaults().with("resample", ParamValue::B(false))), Err(CookError::Failed(_))));
    }

    #[test]
    fn open_curves_make_an_open_sheet_and_mixed_closure_is_open() {
        let mut d = curve(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], false);
        d.merge(&curve(&[[0.0, 1.0, 0.0], [1.0, 1.0, 1.0], [2.0, 1.0, 0.0]], false));
        let o = run(d.clone(), defaults()).expect("cooks");
        assert_eq!((o.point_count, o.prim_count()), (6, 2));
        d.merge(&sq(5.0, 1.0));
        let mixed = run(d, defaults()).expect("cooks");
        assert_eq!(mixed.prim_count(), 2 * 3, "mixed closure -> open loft over the largest count");
    }

    #[test]
    fn hostile_params_and_geometry() {
        for (k, v) in
            [("surftype", ParamValue::Menu(2)), ("closev", ParamValue::I(1)), ("resample", ParamValue::F(1.0))]
        {
            assert!(matches!(run(sq(0.0, 1.0), defaults().with(k, v)), Err(CookError::BadParam(_))), "{k}");
        }
        assert_eq!(run(Detail::default(), defaults()).expect("cooks").prim_count(), 0);
        assert_eq!(run(sq(0.0, 1.0), defaults()).expect("one section").prim_count(), 0);
        let mut d = stack(&[(0.0, 1.0), (1.0, 1.0)]);
        d.vertex_point[0] = 99;
        assert_eq!(run(d, defaults()).expect("dangling section dropped").prim_count(), 0);
        let mut z = sq(0.0, 0.0);
        z.merge(&curve(&[[0.0; 3], [0.0; 3]], false));
        assert!(run(z, defaults()).is_ok());
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let ex = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/skin.json"))
            .expect("example present");
        let (mut g, id) = load_scene(&ex).expect("example loads");
        let out = g.cook(id, 0.0).expect("example cooks");
        assert!(out.prim_count() >= 12);
        assert_eq!(crate::registry().create("skin").map(|n| n.type_name()), Some("skin"));
    }
}
