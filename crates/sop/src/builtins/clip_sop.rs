//! Clip SOP (token `clip`) + the plane-cut core shared with `knife` (`knife_sop.rs`).
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Plane = point `origin + n * dist`, normal `n = normalize(dir)`. Signed distance `d(P) = (P - origin) . n - dist`; a point
//! is ABOVE when `d > 1e-7`, BELOW when `d < -1e-7`, ON otherwise (ON points belong to both sides, are never split).
//! `keep` menu: 0 above (default), 1 below, 2 both.
//! * Closed polys: Sutherland-Hodgman against the half-space (exact for convex polys; a concave poly may get zero-width
//!   bridges). Result < 3 corners = dropped. A crossing edge gets ONE new point, shared by every prim on that edge (and by
//!   both sides in `both`); position `lo + t (hi - lo)` from the lower point id, so neighbours agree bit-for-bit.
//! * Open polys / polylines: split into runs on the kept side (run < 2 points dropped).
//! * Other prim kinds (sphere, tube, unknown): kept whole iff every vertex is on the kept side, else dropped.
//! * Loose points (in no prim): kept iff on the kept side. In `both`, a prim lying entirely ON the plane is emitted once.
//! * Output points: kept source points in ascending source order, then cut points in creation order. Prims: pass above
//!   (source order), then pass below. Point float attrs of a cut point are linearly interpolated (F32/V2/V3/V4, weight
//!   `t`); int / string attrs copy the lower-id point; vertex attrs of a cut corner copy the edge's start corner; prim
//!   attrs and groups follow the source prim.
//! * QB additions (`clip` node only): `group` S = PRIM group filter (empty = all; unflagged prims are emitted whole, uncut,
//!   once); `ptgroup` S = point group (a prim is cut only when ALL its points are in it; AND-ed with `group`); `capclip` B
//!   off = close the cut: the open boundary loops of each emitted side that lie entirely on the plane become one n-gon each
//!   (winding closes the body outward: above-side cap faces -n, below-side cap +n; `keep` all caps both sides);
//!   `recomputen` B off = write point `N` = uniform-averaged unit face normals of the output (with caps when `capclip`).
//!   The keep menu's third entry is labelled `Keep All` (index 2, unchanged).
//! * NOT modelled (DEFERRED): polysoup / NURBS surface clipping (no such prim kinds in hexe-geo).
use super::meshutil::{MAX_ELEMS, Remap, V3, add, bad, dot, failed, positions, rebuild, scale, sub, to_f32};
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::collections::HashMap;
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const EPS: f64 = 1e-7;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Keep {
    Above,
    Below,
    Both,
}

/// Plane + keep params shared by `clip` and `knife`.
pub(super) fn plane_specs(keep_default: u32) -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("origin", ParamValue::V3([0.0; 3])).label("Origin"),
        ParamSpec::new("dir", ParamValue::V3([0.0, 1.0, 0.0])).label("Direction"),
        ParamSpec::new("dist", ParamValue::F(0.0)).label("Distance").range(-10.0, 10.0),
        ParamSpec::new("keep", ParamValue::Menu(keep_default)).label("Keep").menu(&[
            ("above", "Keep Above"),
            ("below", "Keep Below"),
            ("both", "Keep Both"),
        ]),
    ]
}

/// `(point on plane, unit normal, keep)` from the node params; hostile values -> `BadParam`.
pub(super) fn plane_params(op: &str, p: &ParamSet) -> Result<(V3, V3, Keep), CookError> {
    let v3 = |n: &str| match p.vec3(n) {
        Some(v) if v.iter().all(|c| c.is_finite()) => Ok(v.map(f64::from)),
        _ => Err(bad(format!("{op}: `{n}`: missing or not a finite vector"))),
    };
    let (origin, dir) = (v3("origin")?, v3("dir")?);
    let dist = match p.float("dist") {
        Some(v) if v.is_finite() => f64::from(v),
        _ => return Err(bad(format!("{op}: `dist`: missing or not a finite float"))),
    };
    let len = dot(dir, dir).sqrt();
    if !(len.is_finite() && len > 1e-12) {
        return Err(bad(format!("{op}: `dir` must be non-zero")));
    }
    let n = scale(dir, 1.0 / len);
    let keep = match p.menu("keep") {
        Some(0) => Keep::Above,
        Some(1) => Keep::Below,
        Some(2) => Keep::Both,
        Some(m) => return Err(bad(format!("{op}: `keep` menu {m} out of range 0..=2"))),
        None => return Err(bad(format!("{op}: `keep`: missing or not a menu"))),
    };
    Ok((add(origin, scale(n, dist)), n, keep))
}

/// Result of a plane cut.
pub(super) struct Cut {
    pub detail: Detail,
    /// Point mask: cut points + kept source points lying ON the plane.
    pub seam: Vec<bool>,
    /// Prim masks: emitted by the above / below pass.
    pub above: Vec<bool>,
    pub below: Vec<bool>,
}

#[derive(Clone, Copy)]
enum PRef {
    S(usize),
    C(usize),
    /// A not-yet-numbered cut on the source edge (lo, hi).
    E(usize, usize),
}
type Corner = (PRef, Option<usize>);
struct Emit {
    kind: PrimKind,
    corners: Vec<Corner>,
    src: usize,
    above: bool,
}

/// (vertex id, point id) of every vertex of `pr`; `None` when any id is out of range.
fn prim_vpts(src: &Detail, pr: &Prim, np: usize) -> Option<Vec<(usize, usize)>> {
    let (lo, hi) = (usize::try_from(pr.verts.start).ok()?, usize::try_from(pr.verts.end).ok()?);
    let vs = src.vertex_point.get(lo..hi)?;
    vs.iter().enumerate().map(|(k, &p)| usize::try_from(p).ok().filter(|&p| p < np).map(|p| (lo + k, p))).collect()
}

#[derive(Default)]
struct Cuts {
    list: Vec<(usize, usize)>,
    ix: HashMap<(usize, usize), usize>,
}
impl Cuts {
    fn get(&mut self, a: usize, b: usize) -> PRef {
        let key = (a.min(b), a.max(b));
        let n = self.list.len();
        PRef::C(*self.ix.entry(key).or_insert_with(|| {
            self.list.push(key);
            n
        }))
    }
}

/// Cut `src` by the plane (`o`, unit `n`), keeping `keep`. See the module doc.
pub(super) fn clip(op: &str, src: &Detail, o: V3, n: V3, keep: Keep) -> Result<Cut, CookError> {
    clip_sel(op, src, o, n, keep, None)
}
/// `clip` restricted to the prims flagged in `sel` (None = all): unflagged prims are emitted whole, uncut, once.
pub(super) fn clip_sel(
    op: &str,
    src: &Detail,
    o: V3,
    n: V3,
    keep: Keep,
    sel: Option<&[bool]>,
) -> Result<Cut, CookError> {
    let pos = positions(src);
    let np = pos.len();
    let d: Vec<f64> = pos.iter().map(|&p| dot(sub(p, o), n)).collect();
    if d.iter().any(|x| !x.is_finite()) {
        return Err(failed(op, "non-finite position"));
    }
    if u64::from(src.vertex_count()).saturating_mul(4) > MAX_ELEMS {
        return Err(failed(op, "too many vertices"));
    }
    let side: Vec<i8> = d
        .iter()
        .map(|&x| {
            if x > EPS {
                1
            } else if x < -EPS {
                -1
            } else {
                0
            }
        })
        .collect();
    let passes: &[i8] = match keep {
        Keep::Above => &[1],
        Keep::Below => &[-1],
        Keep::Both => &[1, -1],
    };
    let mut in_prim = vec![false; np];
    for pr in &src.prims {
        for &(_, p) in prim_vpts(src, pr, np).iter().flatten() {
            if let Some(f) = in_prim.get_mut(p) {
                *f = true;
            }
        }
    }
    // Pieces per (source prim, side). Cut points are only NAMED here (`PRef::E`); ids are minted afterwards in walk order.
    let piece = |pr: &Prim, vp: &[(usize, usize)], s: i8| -> Vec<(PrimKind, Vec<Corner>)> {
        let inside = |p: usize| side.get(p).is_some_and(|&x| x * s >= 0);
        let crosses = |a: usize, b: usize| side.get(a).zip(side.get(b)).is_some_and(|(&x, &y)| x * y < 0);
        let edge = |a: usize, b: usize| PRef::E(a.min(b), a.max(b));
        match &pr.kind {
            PrimKind::Poly { closed: true } => {
                // walk: emit the corner if inside, then the cut of the edge to the next corner if it crosses
                let mut out: Vec<Corner> = Vec::new();
                for (k, &(va, a)) in vp.iter().enumerate() {
                    let Some(&(_, b)) = vp.get((k + 1) % vp.len()) else { continue };
                    if inside(a) {
                        out.push((PRef::S(a), Some(va)));
                    }
                    if crosses(a, b) {
                        out.push((edge(a, b), Some(va)));
                    }
                }
                if out.len() >= 3 { vec![(pr.kind.clone(), out)] } else { Vec::new() }
            }
            PrimKind::Poly { closed: false } | PrimKind::Polyline => {
                let mut runs: Vec<Vec<Corner>> = Vec::new();
                let mut run: Vec<Corner> = Vec::new();
                for (k, &(vb, b)) in vp.iter().enumerate() {
                    let Some(&(va, a)) = k.checked_sub(1).and_then(|j| vp.get(j)) else {
                        if inside(b) {
                            run.push((PRef::S(b), Some(vb)));
                        }
                        continue;
                    };
                    match (inside(a), inside(b)) {
                        (true, true) => run.push((PRef::S(b), Some(vb))),
                        (true, false) => {
                            if crosses(a, b) {
                                run.push((edge(a, b), Some(va)));
                            }
                            runs.push(std::mem::take(&mut run));
                        }
                        (false, true) => {
                            if crosses(a, b) {
                                run.push((edge(a, b), Some(va)));
                            }
                            run.push((PRef::S(b), Some(vb)));
                        }
                        (false, false) => {}
                    }
                }
                runs.push(run);
                runs.into_iter().filter(|r| r.len() >= 2).map(|r| (pr.kind.clone(), r)).collect()
            }
            _ => {
                if vp.iter().all(|&(_, p)| inside(p)) {
                    vec![(pr.kind.clone(), vp.iter().map(|&(v, p)| (PRef::S(p), Some(v))).collect())]
                } else {
                    Vec::new()
                }
            }
        }
    };
    let want_above = keep != Keep::Below;
    let want_below = keep != Keep::Above;
    // slot layout (Houdini): every source prim keeps its slot holding its BELOW piece (or, when it has none, its ABOVE
    // piece); the ABOVE halves of prims that were split are appended afterwards in source order.
    type Slot = (usize, Vec<(PrimKind, Vec<Corner>)>, bool);
    let mut slots: Vec<Slot> = Vec::new();
    let mut appended: Vec<Slot> = Vec::new();
    for (qi, pr) in src.prims.iter().enumerate() {
        let Some(vp) = prim_vpts(src, pr, np) else { continue };
        if sel.is_some_and(|s| !s.get(qi).copied().unwrap_or(false)) {
            let corners: Vec<Corner> = vp.iter().map(|&(v, p)| (PRef::S(p), Some(v))).collect();
            slots.push((qi, vec![(pr.kind.clone(), corners)], keep != Keep::Below));
            continue;
        }
        let on_plane = vp.iter().all(|&(_, p)| side.get(p) == Some(&0));
        let above = piece(pr, &vp, 1);
        let below = if keep == Keep::Both && on_plane { Vec::new() } else { piece(pr, &vp, -1) };
        if below.is_empty() {
            slots.push((qi, above, true));
        } else {
            slots.push((qi, below, false));
            if !above.is_empty() {
                appended.push((qi, above, true));
            }
        }
    }
    let mut cuts = Cuts::default();
    let mut emits: Vec<Emit> = Vec::new();
    for (qi, pieces, is_above) in slots.into_iter().chain(appended) {
        if (is_above && !want_above) || (!is_above && !want_below) {
            continue;
        }
        for (kind, mut corners) in pieces {
            // mint cut ids in walk order, then start a closed cycle at its first kept source corner
            for c in corners.iter_mut() {
                if let PRef::E(a, b) = c.0 {
                    c.0 = cuts.get(a, b);
                }
            }
            if kind == (PrimKind::Poly { closed: true })
                && let Some(k) = corners.iter().position(|c| matches!(c.0, PRef::S(_)))
            {
                corners.rotate_left(k);
            }
            emits.push(Emit { kind, corners, src: qi, above: is_above });
        }
    }
    // kept source points: referenced by an emitted corner, or loose and on a kept side
    let mut keep_pt = vec![false; np];
    for e in &emits {
        for (r, _) in &e.corners {
            if let PRef::S(i) = r
                && let Some(f) = keep_pt.get_mut(*i)
            {
                *f = true;
            }
        }
    }
    for (i, kp) in keep_pt.iter_mut().enumerate() {
        if !in_prim.get(i).copied().unwrap_or(true) && passes.iter().any(|&s| side.get(i).is_some_and(|&x| x * s >= 0))
        {
            *kp = true;
        }
    }
    let kept: Vec<usize> = (0..np).filter(|&i| keep_pt.get(i).copied().unwrap_or(false)).collect();
    let mut new_ix = vec![0usize; np];
    for (k, &i) in kept.iter().enumerate() {
        new_ix[i] = k;
    }
    let n_src = kept.len();
    let mut point_src: Vec<Vec<usize>> = kept.iter().map(|&i| vec![i]).collect();
    let mut p: Vec<V3> = kept.iter().map(|&i| pos[i]).collect();
    let mut weights = Vec::with_capacity(cuts.list.len());
    for &(lo, hi) in &cuts.list {
        let (dl, dh) = (d[lo], d[hi]);
        let t = dl / (dl - dh);
        weights.push(t);
        point_src.push(vec![lo, hi]);
        p.push(add(pos[lo], scale(sub(pos[hi], pos[lo]), t)));
    }
    let (mut vertex_point, mut prims, mut vert_src, mut prim_src) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut above, mut below) = (Vec::new(), Vec::new());
    for e in &emits {
        let start = u32::try_from(vertex_point.len()).map_err(|_| failed(op, "vertex id overflow"))?;
        for (r, v) in &e.corners {
            let ix = match r {
                PRef::S(i) => new_ix[*i],
                PRef::C(c) => n_src + c,
                PRef::E(..) => return Err(failed(op, "unresolved cut")),
            };
            vertex_point.push(u32::try_from(ix).map_err(|_| failed(op, "point id overflow"))?);
            vert_src.push(*v);
        }
        let end = u32::try_from(vertex_point.len()).map_err(|_| failed(op, "vertex id overflow"))?;
        prims.push(Prim { kind: e.kind.clone(), verts: start..end });
        prim_src.push(Some(e.src));
        above.push(e.above);
        below.push(!e.above);
    }
    let seam: Vec<bool> =
        (0..p.len()).map(|k| k >= n_src || kept.get(k).is_some_and(|&i| side.get(i) == Some(&0))).collect();
    let rm = Remap { point_src, vert_src, prim_src };
    let mut out = rebuild(op, src, vertex_point, prims, &rm, to_f32(op, &p)?)?;
    lerp_cut_attrs(src, &mut out, n_src, &cuts.list, &weights);
    Ok(Cut { detail: out, seam, above, below })
}

/// Cut points carry `a + t (b - a)` of the float-like point attrs (rebuild would have averaged with weight 1/2).
fn lerp_cut_attrs(src: &Detail, out: &mut Detail, n_src: usize, cuts: &[(usize, usize)], w: &[f64]) {
    fn lerp<const N: usize>(o: &mut [[f32; N]], s: &[[f32; N]], n_src: usize, cuts: &[(usize, usize)], w: &[f64]) {
        for (k, (&(a, b), &t)) in cuts.iter().zip(w).enumerate() {
            let (Some(x), Some(y)) = (s.get(a), s.get(b)) else { continue };
            if let Some(dst) = o.get_mut(n_src + k) {
                for j in 0..N {
                    let (xj, yj) = (f64::from(x[j]), f64::from(y[j]));
                    dst[j] = (xj + t * (yj - xj)) as f32;
                }
            }
        }
    }
    for a in out.attrs.iter_mut().filter(|a| a.class == AttrClass::Point && a.name != "P") {
        let Some(sa) = src.attr(AttrClass::Point, &a.name) else { continue };
        match (&mut a.data, &sa.data) {
            (AttrData::F32(o), AttrData::F32(s)) => {
                for (k, (&(i, j), &t)) in cuts.iter().zip(w).enumerate() {
                    if let (Some(&x), Some(&y), Some(dst)) = (s.get(i), s.get(j), o.get_mut(n_src + k)) {
                        *dst = (f64::from(x) + t * (f64::from(y) - f64::from(x))) as f32;
                    }
                }
            }
            (AttrData::V2(o), AttrData::V2(s)) => lerp(o, s, n_src, cuts, w),
            (AttrData::V3(o), AttrData::V3(s)) => lerp(o, s, n_src, cuts, w),
            (AttrData::V4(o), AttrData::V4(s)) => lerp(o, s, n_src, cuts, w),
            _ => {}
        }
    }
}

pub fn spec() -> Vec<ParamSpec> {
    let mut v = plane_specs(0);
    // Houdini names the third entry "Keep All"
    if let Some(k) = v.iter_mut().find(|s| s.name == "keep") {
        *k = ParamSpec::new("keep", ParamValue::Menu(0)).label("Keep").menu(&[
            ("above", "Keep Above"),
            ("below", "Keep Below"),
            ("all", "Keep All"),
        ]);
    }
    v.push(ParamSpec::new("group", ParamValue::S(String::new())).label("Primitive Group"));
    v.push(ParamSpec::new("ptgroup", ParamValue::S(String::new())).label("Point Group (prims fully inside)"));
    v.push(ParamSpec::new("capclip", ParamValue::B(false)).label("Cap Clipped Surface"));
    v.push(ParamSpec::new("recomputen", ParamValue::B(false)).label("Recompute Normals"));
    v
}
pub fn new() -> Box<SopNode> {
    Box::new(Clip { specs: spec() })
}
struct Clip {
    specs: Vec<ParamSpec>,
}
impl Node<Detail> for Clip {
    fn type_name(&self) -> &'static str {
        "clip"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let (o, n, keep) = plane_params("clip", p)?;
        let capclip = p.toggle("capclip").ok_or_else(|| bad("clip: `capclip`: missing or not a toggle"))?;
        let recompute = p.toggle("recomputen").ok_or_else(|| bad("clip: `recomputen`: missing or not a toggle"))?;
        let names = |k: &str| -> Result<String, CookError> {
            p.string(k).map(str::to_string).ok_or_else(|| bad(format!("clip: `{k}`: missing or not a string")))
        };
        let (group, ptgroup) = (names("group")?, names("ptgroup")?);
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        // prim selection: prim group AND (when given) prims whose points are ALL in the point group
        let mut sel = super::meshutil::selection("clip", src, &group)?;
        if ptgroup.split_whitespace().next().is_some() {
            let mut pts = vec![false; src.point_count as usize];
            for g in ptgroup.split_whitespace() {
                let grp = src
                    .group(AttrClass::Point, g)
                    .ok_or_else(|| failed("clip", &format!("unknown point group `{g}`")))?;
                for (s, m) in pts.iter_mut().zip(&grp.mask) {
                    *s |= *m;
                }
            }
            let np = src.point_count as usize;
            let inside: Vec<bool> = src
                .prims
                .iter()
                .map(|pr| {
                    prim_vpts(src, pr, np)
                        .is_some_and(|vp| vp.iter().all(|&(_, q)| pts.get(q).copied().unwrap_or(false)))
                })
                .collect();
            sel = Some(match sel {
                Some(s) => s.iter().zip(&inside).map(|(a, b)| *a && *b).collect(),
                None => inside,
            });
        }
        let cut = clip_sel("clip", src, o, n, keep, sel.as_deref())?;
        let mut out = cut.detail;
        if capclip {
            // close the cut: boundary loops of each emitted side whose points all lie ON the plane (cut / on-plane points)
            let mut caps: Vec<Vec<u32>> = Vec::new();
            for (mask, want) in [(&cut.above, keep != Keep::Below), (&cut.below, keep != Keep::Above)] {
                if want {
                    caps.extend(
                        super::polyfill_sop::boundary_loops(&out, Some(mask))
                            .into_iter()
                            .filter(|l| l.iter().all(|&i| cut.seam.get(i as usize).copied().unwrap_or(false))),
                    );
                }
            }
            for l in &caps {
                out.add_poly(l, true);
            }
        }
        if recompute {
            let pos = positions(&out);
            let nn = super::meshutil::point_normals(&out, &pos);
            out.add_attr(hexe_geo::Attribute {
                name: "N".into(),
                class: AttrClass::Point,
                data: AttrData::V3(nn.iter().map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect()),
                type_info: hexe_geo::TypeInfo::Normal,
            });
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::super::meshutil::testkit::{cook, cube, defaults, houdini_twin, prim_pts};
    use super::*;
    use hexe_geo::{Attribute, TypeInfo};

    fn with(keep: u32, y: f32) -> ParamSet {
        defaults(&*new()).with("keep", ParamValue::Menu(keep)).with("dist", ParamValue::F(y))
    }
    fn on_plane(d: &Detail, y: f32) -> usize {
        d.p().iter().filter(|p| (p[1] - y).abs() < 1e-6).count()
    }
    fn line5() -> Detail {
        let mut d = Detail::default();
        d.add_points(5);
        *d.p_mut() = (0..5).map(|i| [0.0, i as f32, 0.0]).collect();
        d.add_poly(&[0, 1, 2, 3, 4], false);
        d
    }

    fn vol(d: &Detail) -> f64 {
        super::super::surfutil::testkit::signed_volume(d)
    }
    fn run_clip(d: Detail, p: ParamSet) -> Result<Arc<Detail>, CookError> {
        let n = new();
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        n.cook(&ctx, &[Arc::new(d)])
    }
    fn grouped(name: &str, prims: &[usize], pts: Option<Vec<bool>>) -> Detail {
        let mut d = cube();
        d.groups.push(hexe_geo::Group {
            name: name.into(),
            class: AttrClass::Prim,
            mask: (0..d.prims.len()).map(|i| prims.contains(&i)).collect(),
        });
        if let Some(m) = pts {
            d.groups.push(hexe_geo::Group { name: "toppts".into(), class: AttrClass::Point, mask: m });
        }
        d
    }

    #[test]
    fn capclip_closes_the_cut_outward_with_exact_volume() {
        let cap = |keep: u32, y: f32| with(keep, y).with("capclip", ParamValue::B(true));
        // keep above y=0: top + 4 half sides + cap(-Y) -> half the unit box
        let a = cook(&*new(), cube(), &cap(0, 0.0)).expect("cooks");
        assert_eq!((a.point_count, a.prim_count()), (8, 6));
        assert!((vol(&a) - 0.5).abs() < 1e-6, "{}", vol(&a));
        let pos = positions(&a);
        let last: Vec<usize> = {
            let pr = &a.prims[5];
            a.vertex_point[pr.verts.start as usize..pr.verts.end as usize].iter().map(|&i| i as usize).collect()
        };
        let nrm = super::super::meshutil::prim_normal(&pos, &last).expect("cap normal");
        assert!(nrm[1] < -0.99, "{nrm:?}");
        // keep below: cap faces +Y
        let b = cook(&*new(), cube(), &cap(1, 0.0)).expect("cooks");
        assert!((vol(&b) - 0.5).abs() < 1e-6);
        // a thinner slab: y > 0.25 -> volume 0.25
        let t = cook(&*new(), cube(), &cap(0, 0.25)).expect("cooks");
        assert!((vol(&t) - 0.25).abs() < 1e-6 && t.prim_count() == 6);
        // keep all: both halves closed -> total volume 1, two caps
        let both = cook(&*new(), cube(), &cap(2, 0.0)).expect("cooks");
        assert_eq!(both.prim_count(), 10 + 2);
        assert!((vol(&both) - 1.0).abs() < 1e-6, "{}", vol(&both));
        // off (default): open shell, no cap
        let off = cook(&*new(), cube(), &with(0, 0.0)).expect("cooks");
        assert_eq!(off.prim_count(), 5);
    }

    #[test]
    fn group_filters_leave_unselected_prims_whole_and_uncut() {
        let ungrouped = cook(&*new(), cube(), &with(0, 0.0)).expect("cooks");
        assert_eq!((ungrouped.point_count, ungrouped.prim_count()), (8, 5));
        // only the +Y face (prim 3) is selected: it is above -> kept; the other five are emitted whole, no cut points
        let g =
            run_clip(grouped("g", &[3], None), with(0, 0.0).with("group", ParamValue::S("g".into()))).expect("cooks");
        assert_eq!((g.point_count, g.prim_count()), (8, 6));
        assert_eq!(on_plane(&g, 0.0), 0);
        // every prim selected == no filter
        let all =
            run_clip(grouped("g", &[0, 1, 2, 3, 4, 5], None), with(0, 0.0).with("group", ParamValue::S("g".into())))
                .expect("cooks");
        assert_eq!((all.point_count, all.prim_count()), (8, 5));
        // point group = the four top points: only the top face lies fully inside -> same as selecting prim 3
        let top: Vec<bool> = cube().p().iter().map(|p| p[1] > 0.0).collect();
        let p = run_clip(
            grouped("g", &[], Some(top.clone())),
            with(0, 0.0).with("ptgroup", ParamValue::S("toppts".into())),
        )
        .expect("cooks");
        assert_eq!((p.point_count, p.prim_count()), (8, 6));
        // the two filters AND together: prim group excluding the top face + the top point group -> nothing selected
        let none = run_clip(
            grouped("g", &[0, 1, 2, 4, 5], Some(top)),
            with(0, 0.0).with("group", ParamValue::S("g".into())).with("ptgroup", ParamValue::S("toppts".into())),
        )
        .expect("cooks");
        assert_eq!((none.point_count, none.prim_count()), (8, 6));
        // unknown groups fail
        for k in ["group", "ptgroup"] {
            assert!(
                matches!(
                    run_clip(cube(), with(0, 0.0).with(k, ParamValue::S("nope".into()))),
                    Err(CookError::Failed(_))
                ),
                "{k}"
            );
        }
        assert!(matches!(
            run_clip(cube(), with(0, 0.0).with("capclip", ParamValue::I(1))),
            Err(CookError::BadParam(_))
        ));
    }

    #[test]
    fn recomputen_writes_point_normals_from_the_output_faces() {
        let r = cook(
            &*new(),
            cube(),
            &with(0, 0.0).with("capclip", ParamValue::B(true)).with("recomputen", ParamValue::B(true)),
        )
        .expect("cooks");
        let Some(AttrData::V3(n)) = r.attr(AttrClass::Point, "N").map(|a| a.data.clone()) else { panic!("N") };
        assert_eq!(n.len(), r.point_count as usize);
        let s3 = (1.0f32 / 3.0f32.sqrt()).abs();
        let at = |x: f32, y: f32, z: f32| {
            r.p()
                .iter()
                .position(|p| (p[0] - x).abs() < 1e-6 && (p[1] - y).abs() < 1e-6 && (p[2] - z).abs() < 1e-6)
                .expect("point")
        };
        // cut corner (0.5,0,0.5): sides +X, +Z and the -Y cap -> (1,-1,1)/sqrt3
        let c = n[at(0.5, 0.0, 0.5)];
        assert!((c[0] - s3).abs() < 1e-5 && (c[1] + s3).abs() < 1e-5 && (c[2] - s3).abs() < 1e-5, "{c:?}");
        // top corner (0.5,0.5,0.5): top + sides +X, +Z -> (1,1,1)/sqrt3
        let t = n[at(0.5, 0.5, 0.5)];
        assert!((t[0] - s3).abs() < 1e-5 && (t[1] - s3).abs() < 1e-5 && (t[2] - s3).abs() < 1e-5, "{t:?}");
        // off by default
        assert!(cook(&*new(), cube(), &with(0, 0.0)).expect("cooks").attr(AttrClass::Point, "N").is_none());
    }

    /// Unit box (y in +-0.5) cut at y=0: 4 side quads are halved (quad with 2 old top + 2 cut points), top kept, bottom gone.
    #[test]
    fn box_keep_above_at_y0_is_top_plus_four_half_sides() {
        let out = cook(&*new(), cube(), &with(0, 0.0)).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (8, 5), "4 top corners + 4 cut points; top + 4 sides");
        assert!(out.prims.iter().all(|p| p.kind == PrimKind::Poly { closed: true } && p.verts.len() == 4));
        assert!(out.p().iter().all(|p| p[1] >= 0.0));
        assert_eq!(on_plane(&out, 0.0), 4);
        assert_eq!(on_plane(&out, 0.5), 4);
        // the 4 cut points are the corners of the unit square at y=0
        let mut cp: Vec<[f32; 2]> = out.p().iter().filter(|p| p[1] == 0.0).map(|p| [p[0], p[2]]).collect();
        cp.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        assert_eq!(cp, vec![[-0.5, -0.5], [-0.5, 0.5], [0.5, -0.5], [0.5, 0.5]]);
    }

    #[test]
    fn box_keep_below_mirrors_and_both_shares_the_cut_points() {
        let below = cook(&*new(), cube(), &with(1, 0.0)).expect("cooks");
        assert_eq!((below.point_count, below.prim_count()), (8, 5));
        assert!(below.p().iter().all(|p| p[1] <= 0.0));
        let both = cook(&*new(), cube(), &with(2, 0.0)).expect("cooks");
        assert_eq!((both.point_count, both.prim_count()), (12, 10), "8 corners + 4 SHARED cut points; 5 + 5 prims");
        assert_eq!(on_plane(&both, 0.0), 4);
        for (k, p) in both.p().iter().enumerate() {
            if p[1] == 0.0 {
                let users = both.vertex_point.iter().filter(|&&v| v as usize == k).count();
                assert_eq!(users, 4, "each cut point sits in 2 above + 2 below prims");
            }
        }
    }

    #[test]
    fn dir_and_dist_place_the_plane_and_on_plane_faces_are_kept() {
        // dir (0,2,0) is normalised; dist 0.25 -> y = 0.25
        let p = with(0, 0.25).with("dir", ParamValue::V3([0.0, 2.0, 0.0]));
        let out = cook(&*new(), cube(), &p).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (8, 5));
        assert_eq!(on_plane(&out, 0.25), 4);
        // plane exactly on the top face: above = the top quad alone; below (alone) keeps everything (top is ON)
        let top = cook(&*new(), cube(), &with(0, 0.5)).expect("cooks");
        assert_eq!((top.point_count, top.prim_count()), (4, 1));
        let all = cook(&*new(), cube(), &with(1, 0.5)).expect("cooks");
        assert_eq!((all.point_count, all.prim_count()), (8, 6));
        // both: the ON top face is emitted once (above pass); the 4 sides are untouched below
        let both = cook(&*new(), cube(), &with(2, 0.5)).expect("cooks");
        assert_eq!((both.point_count, both.prim_count()), (8, 6));
    }

    #[test]
    fn open_polyline_is_split_into_runs() {
        let above = cook(&*new(), line5(), &with(0, 1.5)).expect("cooks");
        assert_eq!((above.point_count, above.prim_count()), (4, 1), "cut(1.5), 2, 3, 4");
        assert_eq!(above.p()[3], [0.0, 1.5, 0.0], "cut point is created after the kept source points");
        assert_eq!(above.prims[0].kind, PrimKind::Poly { closed: false });
        let both = cook(&*new(), line5(), &with(2, 1.5)).expect("cooks");
        assert_eq!((both.point_count, both.prim_count()), (6, 2));
        assert_eq!(prim_pts(&both, 0).len() + prim_pts(&both, 1).len(), 4 + 3, "above: cut,2,3,4; below: 0,1,cut");
    }

    #[test]
    fn cut_point_attrs_use_the_exact_weight_not_the_midpoint() {
        let mut d = line5();
        d.add_attr(Attribute {
            name: "w".into(),
            class: AttrClass::Point,
            data: AttrData::F32((0..5).map(|i| (i * 10) as f32).collect()),
            type_info: TypeInfo::None,
        });
        let out = cook(&*new(), d, &with(0, 1.25)).expect("cooks");
        let Some(AttrData::F32(w)) = out.attr(AttrClass::Point, "w").map(|a| &a.data) else { panic!("w missing") };
        // between y=1 (w 10) and y=2 (w 20): t = .25 -> 12.5 (a midpoint blend would say 15)
        assert_eq!(w.last().copied(), Some(12.5));
    }

    #[test]
    fn triangle_clipped_to_x_ge_half_has_exact_area() {
        let mut d = Detail::default();
        d.add_points(3);
        *d.p_mut() = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        d.add_poly(&[0, 1, 2], true);
        let p = defaults(&*new()).with("dir", ParamValue::V3([1.0, 0.0, 0.0])).with("dist", ParamValue::F(0.5));
        let out = cook(&*new(), d, &p).expect("cooks");
        assert_eq!((out.point_count, out.prim_count()), (3, 1));
        let q = prim_pts(&out, 0);
        let area: f32 = (0..3).map(|i| q[i][0] * q[(i + 1) % 3][1] - q[(i + 1) % 3][0] * q[i][1]).sum::<f32>() / 2.0;
        assert_eq!(area, 0.125, "vertices (.5,0) (1,0) (.5,.5)");
    }

    #[test]
    fn hostile_params_and_inputs() {
        for bad in [
            defaults(&*new()).with("dir", ParamValue::V3([0.0; 3])),
            defaults(&*new()).with("dir", ParamValue::V3([f32::NAN, 0.0, 1.0])),
            defaults(&*new()).with("keep", ParamValue::Menu(3)),
            ParamSet::new(),
        ] {
            assert!(matches!(cook(&*new(), cube(), &bad), Err(CookError::BadParam(_))));
        }
        assert_eq!(cook(&*new(), Detail::default(), &with(2, 0.0)).expect("cooks").point_count, 0);
        // loose point + dangling vertex id do not panic
        let mut d = cube();
        d.add_points(1);
        d.vertex_point.push(99);
        d.prims.push(Prim { kind: PrimKind::Poly { closed: true }, verts: 24..25 });
        let out = cook(&*new(), d, &with(0, 0.0)).expect("cooks");
        assert_eq!(out.prim_count(), 5, "dangling prim dropped");
    }

    /// H33: the oracle twin of `sop-clip` equals real Houdini 20.5 (`fixtures/sop-clip/houdini.geo`) point-for-point and
    /// prim-for-prim: kept points first, cut points in walk order (+x,-z first), intact prims first, then the split sides
    /// each starting at their first kept source corner (`[1,2,3,0]`, `[1,0,5,4]`, ...).
    #[test]
    fn clip_box_matches_houdini_fixture_order() {
        let r = houdini_twin(
            include_str!("../../../../fixtures/sop-clip/scene.json"),
            include_bytes!("../../../../fixtures/sop-clip/houdini.geo"),
        );
        assert_eq!(r, Ok(()));
    }

    #[test]
    fn example_scene_cooks_and_registry_has_slot() {
        let (mut g, id) = crate::load_scene(include_str!("../../../../examples/clip.json")).expect("loads");
        let d = g.cook(id, 0.0).expect("cooks");
        assert_eq!((d.point_count, d.prim_count()), (8, 5));
        assert_eq!(crate::registry().create("clip").map(|n| n.type_name()), Some("clip"));
    }
}
