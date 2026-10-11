//! Polysplit SOP (token `polysplit`): split polygons along a path of edge points (Houdini 20.5 `polysplit::2.0`, `pathtype` shortest).
//!
//! # Semantics (hython 20.5 probes on a 3x3-point grid, `fixtures/sop-polysplit*`)
//! `splitloc` = whitespace separated tokens. `pA-B` = a NEW point on the edge between points A and B at `A + f*(B-A)`; `NeM` = the same
//! on edge M of prim N (`v[M] -> v[M+1]`, wraps). `f` = `edgepercent` when `edgepercenttoggle` is on, else 0.5. A lone `pN` token is an
//! error in Houdini ("error occurred while processing"), so it is one here. New points are numbered in token order after all original points.
//! * Every prim containing an edge point's edge gets that point inserted into its cycle (shared edges are split in BOTH prims).
//! * Consecutive tokens form a path segment; a segment whose two edge points lie on a common prim splits it into two polygons: with the
//!   cycle positions `i < j` of the two points, piece 1 = `cycle[i..=j]`, piece 2 = `cycle[j..] + cycle[..=i]`.
//! * Output order: prims NOT cut (with inserted points) in original order, then for every cut prim (original order) piece 1, piece 2.
//!
//! Not modelled (-> `Unsupported`): a segment whose edge points share no prim (Houdini routes it through intermediate edge points / vertices),
//! two segments in one prim, `pathtype` edge loop, `close`, groups output, open polys / polylines (copied unchanged).
use super::meshutil::{MAX_ELEMS, bad, failed, float, rebuild_w, toggle};
use crate::SopNode;
use hexe_geo::{Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;
const OP: &str = "polysplit";
const MAX_TOKENS: usize = 1 << 12;
const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("splitloc", ParamValue::S(String::new())).label("Split Locations"),
        ParamSpec::new("edgepercenttoggle", ParamValue::B(false)).label("Edge Percent Toggle"),
        ParamSpec::new("edgepercent", ParamValue::F(0.5)).label("Edge Percent").range(0.0, 1.0),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(Polysplit { specs: spec() })
}
struct Polysplit {
    specs: Vec<ParamSpec>,
}
/// New edge point: between points `a` and `b` at fraction `f`; `id` = output point id.
struct EdgePt {
    a: usize,
    b: usize,
    f: f64,
    id: usize,
}
fn num(s: &str) -> Result<usize, CookError> {
    s.parse::<usize>().map_err(|_| bad(format!("`splitloc`: bad number `{s}`")))
}
impl Node<Detail> for Polysplit {
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
        let loc = p.string("splitloc").ok_or_else(|| bad("`splitloc`: missing or not a string"))?;
        let f = if toggle(p, "edgepercenttoggle")? { float(p, "edgepercent")?.clamp(0.0, 1.0) } else { 0.5 };
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let np = src.point_count as usize;
        let cyc = |i: usize| -> Option<Vec<usize>> {
            let pr = src.prims.get(i)?;
            let vs = src.vertex_point.get(pr.verts.start as usize..pr.verts.end as usize)?;
            Some(vs.iter().map(|&q| q as usize).collect())
        };
        let is_poly = |pr: &Prim| matches!(pr.kind, PrimKind::Poly { closed: true });
        let toks: Vec<&str> = loc.split_whitespace().collect();
        if toks.len() > MAX_TOKENS {
            return Err(failed(OP, "too many split locations"));
        }
        // parse tokens -> new edge points (token order)
        let mut eps: Vec<EdgePt> = Vec::new();
        for t in &toks {
            let (a, b) = if let Some(r) = t.strip_prefix('p') {
                let (a, b) =
                    r.split_once('-').ok_or_else(|| bad(format!("`splitloc`: `{t}` is not an edge point `pA-B`")))?;
                (num(a)?, num(b)?)
            } else if let Some((n, m)) = t.split_once('e') {
                let (n, m) = (num(n)?, num(m)?);
                let c = cyc(n)
                    .filter(|_| src.prims.get(n).is_some_and(is_poly))
                    .ok_or_else(|| bad(format!("`splitloc`: prim {n} is not a polygon")))?;
                match (c.get(m), c.get((m + 1) % c.len().max(1))) {
                    (Some(&a), Some(&b)) => (a, b),
                    _ => return Err(bad(format!("`splitloc`: `{t}` edge out of range"))),
                }
            } else {
                return Err(bad(format!("`splitloc`: unknown token `{t}`")));
            };
            if a >= np || b >= np || a == b {
                return Err(bad(format!("`splitloc`: `{t}` point out of range")));
            }
            let on_edge = (0..src.prims.len()).filter(|&i| is_poly(&src.prims[i])).filter_map(cyc).any(|c| {
                let n = c.len();
                (0..n).any(|k| (c[k], c[(k + 1) % n]) == (a, b) || (c[k], c[(k + 1) % n]) == (b, a))
            });
            if !on_edge {
                return Err(bad(format!("`splitloc`: `{t}` is not an edge of any polygon")));
            }
            eps.push(EdgePt { a, b, f, id: np + eps.len() });
        }
        // per prim: cycle with edge points inserted
        let mut outcyc: Vec<Option<Vec<usize>>> = Vec::with_capacity(src.prims.len());
        for (i, pr) in src.prims.iter().enumerate() {
            let Some(c) = cyc(i).filter(|_| is_poly(pr)) else {
                outcyc.push(None);
                continue;
            };
            let n = c.len();
            let mut o: Vec<usize> = Vec::with_capacity(n + 2);
            for k in 0..n {
                let (x, y) = (c[k], c[(k + 1) % n]);
                o.push(x);
                for e in &eps {
                    if (e.a, e.b) == (x, y) || (e.a, e.b) == (y, x) {
                        o.push(e.id);
                    }
                }
            }
            outcyc.push(Some(o));
        }
        // path segments -> cuts
        let mut cuts: Vec<Option<(usize, usize)>> = vec![None; src.prims.len()];
        for w in eps.windows(2) {
            let (x, y) = (w[0].id, w[1].id);
            if x == y {
                continue;
            }
            let hit = outcyc.iter().position(|c| c.as_ref().is_some_and(|c| c.contains(&x) && c.contains(&y)));
            let Some(i) = hit else {
                return Err(CookError::Unsupported(
                    "polysplit: path segment across faces (intermediate edge points) is not modelled".into(),
                ));
            };
            if cuts[i].is_some() {
                return Err(CookError::Unsupported("polysplit: two path segments in one polygon".into()));
            }
            cuts[i] = Some((x, y));
        }
        // assemble
        let mut point_w: Vec<Vec<(usize, f64)>> = (0..np).map(|q| vec![(q, 1.0)]).collect();
        let mut pos: Vec<[f32; 3]> = src.p().to_vec();
        for e in &eps {
            let (pa, pb) = (src.p()[e.a], src.p()[e.b]);
            let mut q = [0.0f32; 3];
            for (j, qj) in q.iter_mut().enumerate() {
                *qj = (f64::from(pa[j]) + e.f * (f64::from(pb[j]) - f64::from(pa[j]))) as f32;
            }
            point_w.push(vec![(e.a, 1.0 - e.f), (e.b, e.f)]);
            pos.push(q);
        }
        let mut vertex_point: Vec<u32> = Vec::new();
        let mut vert_src: Vec<Option<usize>> = Vec::new();
        let mut prims: Vec<Prim> = Vec::new();
        let mut prim_src: Vec<Option<usize>> = Vec::new();
        let mut budget = 0u64;
        let mut emit = |pts: &[usize],
                        kind: PrimKind,
                        src_i: usize,
                        vs: &dyn Fn(usize) -> Option<usize>|
         -> Result<(), CookError> {
            budget += pts.len() as u64;
            if budget > MAX_ELEMS {
                return Err(failed(OP, "output too large"));
            }
            let start = vertex_point.len() as u32;
            for &q in pts {
                vertex_point.push(u32::try_from(q).map_err(|_| failed(OP, "point id overflow"))?);
                vert_src.push(vs(q));
            }
            prims.push(Prim { kind, verts: start..vertex_point.len() as u32 });
            prim_src.push(Some(src_i));
            Ok(())
        };
        let vsrc = |i: usize| {
            let pr = &src.prims[i];
            let base = pr.verts.start as usize;
            let c = cyc(i).unwrap_or_default();
            move |q: usize| c.iter().position(|&x| x == q).map(|k| base + k)
        };
        for (i, pr) in src.prims.iter().enumerate() {
            match (&outcyc[i], cuts[i]) {
                (Some(c), None) => emit(c, pr.kind.clone(), i, &vsrc(i))?,
                (None, _) => {
                    let c = cyc(i).unwrap_or_default();
                    emit(&c, pr.kind.clone(), i, &vsrc(i))?;
                }
                _ => {}
            }
        }
        for (i, cut) in cuts.iter().enumerate() {
            let (Some((x, y)), Some(c)) = (cut, &outcyc[i]) else { continue };
            let (mut a, mut b) =
                (c.iter().position(|q| q == x).unwrap_or(0), c.iter().position(|q| q == y).unwrap_or(0));
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            let piece1: Vec<usize> = c[a..=b].to_vec();
            let mut piece2: Vec<usize> = c[b..].to_vec();
            piece2.extend_from_slice(&c[..=a]);
            let vs = {
                let base = src.prims[i].verts.start as usize;
                let oc = cyc(i).unwrap_or_default();
                move |q: usize| oc.iter().position(|&z| z == q).map(|k| base + k)
            };
            let k = PrimKind::Poly { closed: true };
            emit(&piece1, k.clone(), i, &vs)?;
            emit(&piece2, k, i, &vs)?;
        }
        Ok(Arc::new(rebuild_w(OP, src, vertex_point, prims, &point_w, &vert_src, &prim_src, pos)?))
    }
}
#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, defaults, grid3, houdini_twin};
    use super::*;
    fn run(loc: &str, f: impl FnOnce(hexe_graph::ParamSet) -> hexe_graph::ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        cook(&*n, grid3(), &f(defaults(&*n).with("splitloc", ParamValue::S(loc.into()))))
    }
    fn cyc(d: &Detail, i: usize) -> Vec<u32> {
        d.vertex_point[d.prims[i].verts.start as usize..d.prims[i].verts.end as usize].to_vec()
    }
    #[test]
    fn edge_point_only_inserts_in_both_prims() {
        // grid3 prim 0 = [0,3,4,1], prim 1 = [1,4,5,2]: edge 1-4 is shared
        let d = run("p1-4", |p| p).expect("cooks");
        assert_eq!((d.point_count, d.prims.len()), (10, 4));
        assert_eq!(d.p()[9], [1.0, 0.5, 0.0]);
        assert!(cyc(&d, 0).contains(&9) && cyc(&d, 1).contains(&9));
    }
    #[test]
    fn path_in_one_prim_splits_it() {
        let d = run("p0-3 p4-1", |p| p).expect("cooks");
        assert_eq!(d.prims.len(), 5);
        assert_eq!(d.point_count, 11);
    }
    #[test]
    fn percent_is_measured_from_the_first_point() {
        let d =
            run("p0-3", |p| p.with("edgepercenttoggle", ParamValue::B(true)).with("edgepercent", ParamValue::F(0.25)))
                .expect("cooks");
        assert_eq!(d.p()[9], [0.0, 0.25, 0.0]);
        let e = run("p0-3", |p| p.with("edgepercent", ParamValue::F(0.25))).expect("cooks");
        assert_eq!(e.p()[9], [0.0, 0.5, 0.0]);
    }
    #[test]
    fn hostile_params() {
        assert!(run("p0", |p| p).is_err());
        assert!(run("p0-99", |p| p).is_err());
        assert!(run("p0-4", |p| p).is_err());
        assert!(run("zz", |p| p).is_err());
        assert!(run("9e0", |p| p).is_err());
        assert!(run("p-1", |p| p).is_err());
        assert_eq!(run("", |p| p).expect("empty").prims.len(), 4);
        assert!(cook(&*new(), Detail::default(), &defaults(&*new())).is_ok());
    }
    macro_rules! twin {
        ($n:literal) => {
            houdini_twin(
                include_str!(concat!("../../../../fixtures/", $n, "/scene.json")),
                include_bytes!(concat!("../../../../fixtures/", $n, "/houdini.geo")),
            )
        };
    }
    /// Hexe twin == real Houdini 20.5 `.geo` (edge point, one-prim path, prim-edge tokens, percent, 3-token chain).
    #[test]
    fn polysplit_matches_houdini_fixtures() {
        for n in [
            twin!("sop-polysplit"),
            twin!("sop-polysplit-edge"),
            twin!("sop-polysplit-pct"),
            twin!("sop-polysplit-chain"),
            twin!("sop-polysplit-single"),
        ] {
            assert_eq!(n, Ok(()));
        }
    }
}
