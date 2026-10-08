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
//! * Output = new points only + vertex `uv` V3 (`u` = column / columns, `v` = section / sections). Input attributes are NOT
//!   carried. NOT modelled (DEFERRED): `keepshape`/knot interpolation (this is a straight ruled loft), `uniform` sampling,
//!   per-section `order`, extra `divs` between sections, NURBS output.
use super::meshutil::{V3, add, bad, failed, positions, scale, sub, toggle};
use super::surfutil::{Mesh, curves, dist};
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
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Skin { specs: spec() })
}
struct Skin {
    specs: Vec<ParamSpec>,
}

/// `n` points evenly spaced by arclength along `pos` (closed: loop, last point NOT repeated; open: end points kept).
fn resample(pos: &[V3], closed: bool, n: usize) -> Vec<V3> {
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
        return vec![pos[0]; n];
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
            add(path[seg], scale(sub(path[seg + 1], path[seg]), f))
        })
        .collect()
}

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
        let (closev, rs) = (toggle(p, "closev")?, toggle(p, "resample")?);
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let pos = positions(src);
        let secs = curves(src);
        if secs.len() < 2 {
            return Ok(Arc::new(Detail::default()));
        }
        let closed_u = secs.iter().all(|c| c.closed);
        let n = secs.iter().map(|c| c.pts.len()).max().unwrap_or(0);
        if !rs && secs.iter().any(|c| c.pts.len() != n) {
            return Err(failed(OP, "sections have different point counts and `resample` is off"));
        }
        let mut m = Mesh::default();
        let mut rows: Vec<Vec<u32>> = Vec::with_capacity(secs.len());
        for c in &secs {
            let q: Vec<V3> = c.pts.iter().map(|&i| pos[i as usize]).collect();
            let q = if q.len() == n { q } else { resample(&q, c.closed, n) };
            let mut row = Vec::with_capacity(n);
            for pt in q {
                row.push(m.point(OP, pt)?);
            }
            rows.push(row);
        }
        let s_count = rows.len();
        let wrap_v = closev && s_count >= 3;
        let v_segs = if wrap_v { s_count } else { s_count - 1 };
        let u_segs = if closed_u { n } else { n - 1 };
        for s in 0..v_segs {
            let s2 = (s + 1) % s_count;
            let (v0, v1) = (s as f32 / v_segs as f32, (s + 1) as f32 / v_segs as f32);
            for j in 0..u_segs {
                let j2 = (j + 1) % n;
                let (u0, u1) = (j as f32 / u_segs as f32, (j + 1) as f32 / u_segs as f32);
                m.quad(
                    OP,
                    [rows[s][j], rows[s2][j], rows[s2][j2], rows[s][j2]],
                    [[u0, v0, 0.0], [u0, v1, 0.0], [u1, v1, 0.0], [u1, v0, 0.0]],
                    tris,
                )?;
            }
        }
        Ok(Arc::new(m.finish()))
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
