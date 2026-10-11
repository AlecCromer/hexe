//! Carve SOP (token `carve`): cut sub-curves out of polylines / polygon boundaries by parametric U domain (Houdini 20.5 `carve`).
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-carve*`)
//! Per selected prim (`group` = prim pattern, empty = all) with `n` points; U runs over `[0,1]`: parametric = `k / spans` at vertex `k`
//! (open `spans = n-1`, closed `spans = n`), `arclen` = cumulative edge length / total. Domain `lo = domainu1` if `firstu` else 0, `hi = domainu2`
//! if `secondu` else 1, both clamped to `[0,1]`, swapped when `lo > hi`. Cut points land on an existing vertex when within 1e-7.
//! * `keepin`: the pieces of `[lo,hi]`, split into `max(divsu-1, 1)` EQUAL pieces in U (boundary points duplicated per piece, original
//!   vertices strictly inside kept). `keepout`: `[0,lo]` before and `[hi,1]` after (never divided). Order out-first, in, out-last; empty pieces omitted.
//! * Open output: every output vertex is a NEW point (cut points linearly blend the two neighbours; floats lerp, ints copy the heavier
//!   source: DEVIATION, Houdini zeroes an int blended at a cut), numbered in output-prim order. Prims NOT selected come FIRST and keep
//!   their original points (numbered in original order); carved prims follow. Output prims are open polygons; prim attrs / groups copy.
//! * CLOSED polys: with `secondu` on the same piece rule (all-new points; `keepout` = ONE open prim `[hi,1]` + `[0,lo]` with the seam point
//!   twice); with `secondu` off the loop is re-opened at `lo` (default 0.25): start cut (an ORIGINAL point when exactly on a vertex, else a
//!   new point), every other vertex ONCE (original points), then a NEW end point copying the start. New points come after the original
//!   points. This is also how a polygon mesh (grid quads) cooks with default parms.
//!
//! Not modelled: V domain, `onlybreakpoints`, `allubreakpoints`, `extractop`, `keepOriginal`, attribute-driven domains (`usedomain*attrib`),
//! `divsu` in the closed re-open case, NURBS / Bezier curve & surface prims (-> `Unsupported`; Houdini does knot insertion), closed
//! `keepin`+`keepout` order (guess: in, out).
use super::group_sop::parse_pattern;
use super::meshutil::{MAX_ELEMS, bad, failed, float, int, rebuild_w, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "carve";
const EPS: f64 = 1e-7;
const MAX_DIVS: i32 = 1 << 14;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("arclen", ParamValue::B(false)).label("Domain By Arc Length"),
        ParamSpec::new("firstu", ParamValue::B(true)).label("First U"),
        ParamSpec::new("domainu1", ParamValue::F(0.25)).label("First U Domain").range(0.0, 1.0),
        ParamSpec::new("secondu", ParamValue::B(false)).label("Second U"),
        ParamSpec::new("domainu2", ParamValue::F(0.75)).label("Second U Domain").range(0.0, 1.0),
        ParamSpec::new("divsu", ParamValue::I(2)).label("U Divisions").range(1.0, 20.0),
        ParamSpec::new("keepin", ParamValue::B(true)).label("Keep Inside"),
        ParamSpec::new("keepout", ParamValue::B(false)).label("Keep Outside"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Carve { specs: spec() })
}
struct Carve {
    specs: Vec<ParamSpec>,
}
/// Position along a curve: on vertex `k` (`k == n` on a closed curve = vertex 0) or `w` of the way from `k` to `k+1`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum At {
    Exact(usize),
    Cut(usize, f64),
}
/// Output point: an original point, or a new one blending sources with weights.
enum Pt {
    Orig(usize),
    New(Vec<(usize, f64)>),
}
struct Curve {
    pts: Vec<usize>,
    v0: usize,
    closed: bool,
    t: Vec<f64>,
}
impl Curve {
    fn at(&self, t: f64) -> At {
        if let Some(k) = self.t.iter().position(|&x| (t - x).abs() <= EPS) {
            return At::Exact(k);
        }
        let k = (0..self.t.len().saturating_sub(1)).rfind(|&k| self.t[k] <= t).unwrap_or(0);
        let (a, b) = (self.t[k], self.t.get(k + 1).copied().unwrap_or(self.t[k]));
        At::Cut(k, if b > a { (t - a) / (b - a) } else { 0.0 })
    }
    /// Nodes of `[a, b]`: cut at `a`, vertices strictly inside, cut at `b`.
    fn piece(&self, a: f64, b: f64) -> Vec<At> {
        if b - a <= EPS {
            return Vec::new();
        }
        let mut v = vec![self.at(a)];
        v.extend((0..self.t.len()).filter(|&k| self.t[k] > a + EPS && self.t[k] < b - EPS).map(At::Exact));
        v.push(self.at(b));
        v
    }
    fn src_vertex(&self, n: At) -> usize {
        let k = match n {
            At::Exact(k) | At::Cut(k, _) => k,
        };
        self.v0 + k % self.pts.len()
    }
    fn weights(&self, n: At) -> Vec<(usize, f64)> {
        let m = self.pts.len();
        match n {
            At::Exact(k) => vec![(self.pts[k % m], 1.0)],
            At::Cut(k, w) => vec![(self.pts[k % m], 1.0 - w), (self.pts[(k + 1) % m], w)],
        }
    }
}
struct OutPrim {
    src: usize,
    nodes: Vec<(Pt, Option<usize>)>,
}
impl Node<Detail> for Carve {
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
        let pat = p.string("group").ok_or_else(|| bad("`group`: missing or not a string"))?;
        let (arclen, firstu, secondu) = (toggle(p, "arclen")?, toggle(p, "firstu")?, toggle(p, "secondu")?);
        let (keepin, keepout) = (toggle(p, "keepin")?, toggle(p, "keepout")?);
        let (d1, d2) = (float(p, "domainu1")?, float(p, "domainu2")?);
        let divs = int(p, "divsu")?;
        if !(1..=MAX_DIVS).contains(&divs) {
            return Err(bad(format!("`divsu`: {divs} outside 1..={MAX_DIVS}")));
        }
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let sel = parse_pattern(src, AttrClass::Prim, pat, OP)?;
        let (mut lo, mut hi) =
            (if firstu { d1.clamp(0.0, 1.0) } else { 0.0 }, if secondu { d2.clamp(0.0, 1.0) } else { 1.0 });
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        let pieces = usize::try_from(divs - 1).unwrap_or(1).max(1);
        let pos = src.p();
        let mut kept: Vec<usize> = Vec::new();
        let mut carved: Vec<OutPrim> = Vec::new();
        let mut budget = 0u64;
        for (i, pr) in src.prims.iter().enumerate() {
            let selected = sel.as_ref().is_none_or(|m| m.get(i).copied().unwrap_or(false));
            let Some(c) = selected.then(|| curve_of(src, pr, arclen)).transpose()?.flatten() else {
                kept.push(i);
                continue;
            };
            let mut outs: Vec<Vec<(Pt, Option<usize>)>> = Vec::new();
            let tag = |c: &Curve, v: Vec<At>, open: bool| -> Vec<(Pt, Option<usize>)> {
                v.into_iter()
                    .map(|a| {
                        let pt = match a {
                            At::Exact(k) if !open => Pt::Orig(c.pts[k % c.pts.len()]),
                            _ => Pt::New(c.weights(a)),
                        };
                        (pt, Some(c.src_vertex(a)))
                    })
                    .collect()
            };
            if c.closed && !secondu {
                // re-open the loop at `lo`: start, every other vertex once, new end copying the start
                if keepin {
                    let start = c.at(lo.min(1.0 - EPS));
                    let n = c.pts.len();
                    let (k0, cnt) = match start {
                        At::Exact(k) => (k, n - 1),
                        At::Cut(k, _) => (k, n),
                    };
                    let mut v = vec![(
                        match start {
                            At::Exact(k) => Pt::Orig(c.pts[k % n]),
                            At::Cut(..) => Pt::New(c.weights(start)),
                        },
                        Some(c.src_vertex(start)),
                    )];
                    for j in 1..=cnt {
                        let k = (k0 + j) % n;
                        v.push((Pt::Orig(c.pts[k]), Some(c.v0 + k)));
                    }
                    v.push((Pt::New(c.weights(start)), Some(c.src_vertex(start))));
                    outs.push(v);
                }
            } else {
                if keepout && !c.closed && lo > EPS {
                    outs.push(tag(&c, c.piece(0.0, lo), true));
                }
                if keepin {
                    for q in 0..pieces {
                        let f = |j: usize| lo + (hi - lo) * j as f64 / pieces as f64;
                        let v = c.piece(f(q), f(q + 1));
                        if !v.is_empty() {
                            outs.push(tag(&c, v, true));
                        }
                    }
                }
                if keepout && !c.closed && hi < 1.0 - EPS {
                    outs.push(tag(&c, c.piece(hi, 1.0), true));
                }
                if keepout && c.closed {
                    let mut v = tag(&c, c.piece(hi, 1.0), true);
                    v.extend(tag(&c, c.piece(0.0, lo), true));
                    if !v.is_empty() {
                        outs.push(v);
                    }
                }
            }
            for nodes in outs {
                budget += nodes.len() as u64;
                if budget > MAX_ELEMS {
                    return Err(failed(OP, "output too large"));
                }
                carved.push(OutPrim { src: i, nodes });
            }
        }
        assemble(src, pos, &kept, &carved)
    }
}
/// Curve view of a selected prim; `None` = pass through, `Unsupported` for spline prims.
fn curve_of(d: &Detail, pr: &Prim, arclen: bool) -> Result<Option<Curve>, CookError> {
    let closed = match pr.kind {
        PrimKind::Poly { closed } => closed,
        PrimKind::Polyline => false,
        PrimKind::BezierCurve { .. }
        | PrimKind::NurbsCurve { .. }
        | PrimKind::BezierSurf { .. }
        | PrimKind::NurbsSurf { .. } => {
            return Err(CookError::Unsupported(
                "carve: Bezier / NURBS primitives (knot insertion) are not modelled".into(),
            ));
        }
        _ => return Ok(None),
    };
    let (lo, hi) = (pr.verts.start as usize, pr.verts.end as usize);
    let Some(vs) = d.vertex_point.get(lo..hi) else { return Ok(None) };
    let pts: Vec<usize> = vs.iter().map(|&q| q as usize).collect();
    let n = pts.len();
    if n < 2 || (closed && n < 3) || pts.iter().any(|&q| q >= d.point_count as usize) {
        return Ok(None);
    }
    let spans = if closed { n } else { n - 1 };
    let p = d.p();
    let mut cum = vec![0.0f64];
    for k in 0..spans {
        let (a, b) = (p[pts[k]], p[pts[(k + 1) % n]]);
        let l: f64 = (0..3).map(|j| (f64::from(b[j]) - f64::from(a[j])).powi(2)).sum::<f64>().sqrt();
        cum.push(cum[k] + l);
    }
    let total = cum[spans];
    let t: Vec<f64> = if arclen && total > 0.0 {
        let mut t: Vec<f64> = cum.iter().map(|c| c / total).collect();
        t[spans] = 1.0;
        t
    } else {
        (0..=spans).map(|k| k as f64 / spans as f64).collect()
    };
    Ok(Some(Curve { pts, v0: lo, closed, t }))
}
fn assemble(src: &Detail, pos: &[[f32; 3]], kept: &[usize], carved: &[OutPrim]) -> Result<Arc<Detail>, CookError> {
    let ssrc = |i: usize| src.vertex_point.get(src.prims[i].verts.start as usize..src.prims[i].verts.end as usize);
    // original points still used, in original order
    let mut used = vec![false; src.point_count as usize];
    for &i in kept {
        for &q in ssrc(i).unwrap_or(&[]) {
            if let Some(u) = used.get_mut(q as usize) {
                *u = true;
            }
        }
    }
    for o in carved {
        for (pt, _) in &o.nodes {
            if let Pt::Orig(q) = pt {
                used[*q] = true;
            }
        }
    }
    let mut remap = vec![u32::MAX; used.len()];
    let mut point_w: Vec<Vec<(usize, f64)>> = Vec::new();
    let mut p: Vec<[f32; 3]> = Vec::new();
    for (q, _) in used.iter().enumerate().filter(|(_, u)| **u) {
        remap[q] = point_w.len() as u32;
        point_w.push(vec![(q, 1.0)]);
        p.push(pos[q]);
    }
    let mut vertex_point: Vec<u32> = Vec::new();
    let mut vert_src: Vec<Option<usize>> = Vec::new();
    let mut prims: Vec<Prim> = Vec::new();
    let mut prim_src: Vec<Option<usize>> = Vec::new();
    for &i in kept {
        let pr = &src.prims[i];
        let start = vertex_point.len() as u32;
        for (j, &q) in ssrc(i).unwrap_or(&[]).iter().enumerate() {
            vertex_point.push(remap.get(q as usize).copied().unwrap_or(0));
            vert_src.push(Some(pr.verts.start as usize + j));
        }
        prims.push(Prim { kind: pr.kind.clone(), verts: start..vertex_point.len() as u32 });
        prim_src.push(Some(i));
    }
    for o in carved {
        let start = vertex_point.len() as u32;
        for (pt, vs) in &o.nodes {
            let id = match pt {
                Pt::Orig(q) => remap[*q],
                Pt::New(w) => {
                    let (mut s, mut ws) = ([0.0f64; 3], 0.0f64);
                    for &(q, wt) in w {
                        for (j, sj) in s.iter_mut().enumerate() {
                            *sj += wt * f64::from(pos[q][j]);
                        }
                        ws += wt;
                    }
                    let id = point_w.len() as u32;
                    point_w.push(w.clone());
                    p.push([(s[0] / ws) as f32, (s[1] / ws) as f32, (s[2] / ws) as f32]);
                    id
                }
            };
            vertex_point.push(id);
            vert_src.push(*vs);
        }
        prims.push(Prim { kind: PrimKind::Poly { closed: false }, verts: start..vertex_point.len() as u32 });
        prim_src.push(Some(o.src));
    }
    Ok(Arc::new(rebuild_w(OP, src, vertex_point, prims, &point_w, &vert_src, &prim_src, p)?))
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, defaults, houdini_twin};
    use super::*;
    use hexe_graph::ParamSet;
    fn line() -> Detail {
        let mut d = Detail::default();
        d.add_points(5);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [3.0, 2.0, 0.0], [4.0, 0.0, 0.0], [5.0, 1.0, 1.0]];
        d.add_poly(&[0, 1, 2, 3, 4], false);
        d
    }
    fn run(d: Detail, f: impl FnOnce(ParamSet) -> ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        cook(&*n, d, &f(defaults(&*n)))
    }
    #[test]
    fn default_cuts_from_quarter_to_end() {
        let d = run(line(), |p| p).expect("cooks");
        assert_eq!(d.p(), &[[1.0, 2.0, 0.0], [3.0, 2.0, 0.0], [4.0, 0.0, 0.0], [5.0, 1.0, 1.0]]);
        assert_eq!(d.prims.len(), 1);
    }
    #[test]
    fn domain_divisions_and_outside_pieces() {
        let d = run(line(), |p| {
            p.with("secondu", ParamValue::B(true))
                .with("domainu1", ParamValue::F(0.2))
                .with("domainu2", ParamValue::F(0.7))
                .with("divsu", ParamValue::I(4))
        })
        .expect("cooks");
        assert_eq!((d.point_count, d.prims.len()), (8, 3));
        assert!((d.p()[2][0] - 1.933_333).abs() < 1e-5 && d.p()[2] == d.p()[3]);
        let o = run(line(), |p| {
            p.with("secondu", ParamValue::B(true))
                .with("domainu2", ParamValue::F(0.5))
                .with("keepout", ParamValue::B(true))
                .with("keepin", ParamValue::B(false))
        })
        .expect("cooks");
        // out-first [0,0.25] = p0, p1 ; out-last [0.5,1] = p2, p3, p4
        assert_eq!(o.prims.len(), 2);
        assert_eq!(o.vertex_point, vec![0, 1, 2, 3, 4]);
    }
    #[test]
    fn reversed_domain_swaps_and_clamps() {
        let a = run(line(), |p| {
            p.with("secondu", ParamValue::B(true))
                .with("domainu1", ParamValue::F(0.7))
                .with("domainu2", ParamValue::F(0.2))
        });
        let b = run(line(), |p| {
            p.with("secondu", ParamValue::B(true))
                .with("domainu1", ParamValue::F(0.2))
                .with("domainu2", ParamValue::F(0.7))
        });
        assert_eq!(a.expect("a").p(), b.expect("b").p());
        let c = run(line(), |p| {
            p.with("secondu", ParamValue::B(true))
                .with("domainu1", ParamValue::F(-3.0))
                .with("domainu2", ParamValue::F(9.0))
        });
        assert_eq!(c.expect("c").point_count, 5);
    }
    #[test]
    fn unselected_prims_come_first_and_keep_points() {
        let mut d = line();
        d.add_points(2);
        let n = d.point_count;
        d.p_mut()[(n - 2) as usize..].copy_from_slice(&[[0.0, 0.0, 1.0], [2.0, 0.0, 1.0]]);
        d.add_poly(&[5, 6], false);
        d.groups.push(hexe_geo::Group { name: "a".into(), class: AttrClass::Prim, mask: vec![true, false] });
        let o = run(d, |p| p.with("group", ParamValue::S("a".into()))).expect("cooks");
        assert_eq!(o.prims.len(), 2);
        assert_eq!(&o.vertex_point[0..2], &[0, 1]);
        assert_eq!(o.p()[0], [0.0, 0.0, 1.0]);
        assert_eq!(o.p()[2], [1.0, 2.0, 0.0]);
    }
    #[test]
    fn hostile_params() {
        assert!(run(line(), |p| p.with("divsu", ParamValue::I(0))).is_err());
        assert!(run(line(), |p| p.with("divsu", ParamValue::I(i32::MAX))).is_err());
        assert!(run(line(), |p| p.with("group", ParamValue::S("nope".into()))).is_err());
        assert_eq!(run(Detail::default(), |p| p).expect("empty").point_count, 0);
    }
    macro_rules! twin {
        ($n:literal) => {
            houdini_twin(
                include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
            )
        };
    }
    /// Hexe twin == real Houdini 20.5 `.geo` (open default, domain + divs + in/out, grid of closed loops, wrapped outside piece).
    #[test]
    fn carve_matches_houdini_fixtures() {
        for n in [twin!("sop-carve"), twin!("sop-carve-divs"), twin!("sop-carve-grid"), twin!("sop-carve-closed")] {
            assert_eq!(n, Ok(()));
        }
    }
}
