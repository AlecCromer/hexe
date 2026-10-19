//! Join SOP (token `join`): joins open polygons / polylines end to end into one curve per subgroup.
//!
//! # Semantics (hython 20.5 probes `oracle/houdini/join/*.py`, cases pinned in the tests below)
//! Prims of `group` (prim group names, empty = all) are split into subgroups by `joinop`: `all` = one subgroup;
//! `group` = consecutive runs of `inc` (a run of one, or `inc <= 1`, stays untouched); `skip` = residue classes
//! `i mod inc` (all prims when `inc <= 1` or `2 * inc > n`). Each subgroup of two or more becomes one NEW prim:
//! * the first prim's points are copied in its vertex order; a closed prim is opened by appending a copy of its first point;
//! * `proximity` (Connect Closest Ends): the first pair picks the closest of (end,start) (end,end) (start,start)
//!   (start,end) (ties = distances within 1e-5: the earlier wins) and flips accordingly; every later prim keeps its
//!   order and flips when its end is closer (by more than 1e-5) to the chain's end. Off: no flips;
//! * `blend` on and `tolerance` t > 0 (clamped to 1): the chain's last point MOVES to `last * bias + next_first *
//!   (1 - bias)`, a cut point at `t` is inserted on the chain's last segment (`prev * t + last * (1 - t)`) and on the next
//!   prim's first segment (`second * t + first * (1 - t)`); the next prim's first point is dropped and its remaining
//!   points are copied after the cuts. t = 1: cuts land on the neighbours and are not made. Off / t = 0: the next
//!   prim's points are all copied (a straight connecting segment);
//! * `loop` (Wrap Last to First): closes the result; with blend the last point moves to the blend of last and first,
//!   cuts go on both end segments and the first point is deleted (the cut on the first segment starts the polygon);
//! * point / vertex attributes follow the same weights (ints / strings copy the heavier source), prim attributes and
//!   prim groups come from the subgroup's first prim.
//! * `onlyconnected`: inside a subgroup, prim B follows the chain only when B's first point IS (same point number) the
//!   chain's last point; no flips; every resulting chain (also a single prim) is re-emitted.
//!
//! Output order: untouched prims and their points first (original order), then each joined prim with its points in
//! creation order. Points used only by joined prims go; `prim` (Keep Primitives) keeps every input prim and point.
//! `dir` (u / v) and `knotmult` only affect surfaces / NURBS: a selected prim that is not a polygon or polyline is
//! `Unsupported`.
//! UNVERIFIED: prims sharing end points without `onlyconnected` (Houdini then walks a point-sharing order not derived
//! here; hexe treats them like any other ends), onlyconnected chains that would need prepending.
use super::meshutil::{blend_weighted, rebuild_w, selection, toggle};
use crate::SopNode;
use hexe_geo::{AttrClass, Attribute, Detail, Prim, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::sync::Arc;

const INPUTS: [InputSpec; 1] = [InputSpec { name: "input0" }];
const OP: &str = "join";
const JOIN_ALL: u32 = 0;
const JOIN_GROUP: u32 = 1;
const JOIN_SKIP: u32 = 2;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("group", ParamValue::S(String::new())).label("Group"),
        ParamSpec::new("blend", ParamValue::B(true)).label("Blend"),
        ParamSpec::new("tolerance", ParamValue::F(1.0)).label("Tolerance").range(0.0, 1.0),
        ParamSpec::new("bias", ParamValue::F(0.5)).label("Bias").range(0.0, 1.0),
        ParamSpec::new("knotmult", ParamValue::B(false)).label("Multiplicity"),
        ParamSpec::new("proximity", ParamValue::B(true)).label("Connect Closest Ends"),
        ParamSpec::new("dir", ParamValue::Menu(0)).label("Direction").menu(&[("ujoin", "U"), ("vjoin", "V")]),
        ParamSpec::new("joinop", ParamValue::Menu(JOIN_ALL)).label("Join").menu(&[
            ("all", "All Primitives"),
            ("group", "Groups of N Primitives"),
            ("skip", "Skip Every Nth Primitive"),
        ]),
        ParamSpec::new("inc", ParamValue::I(2)).label("N").range(1.0, 10.0),
        ParamSpec::new("loop", ParamValue::B(false)).label("Wrap Last to First"),
        ParamSpec::new("prim", ParamValue::B(false)).label("Keep Primitives"),
        ParamSpec::new("onlyconnected", ParamValue::B(false)).label("Only Connected"),
    ]
}

pub fn new() -> Box<SopNode> {
    Box::new(Join { specs: spec() })
}

struct Join {
    specs: Vec<ParamSpec>,
}

/// Source elements with weights (a point or vertex recipe).
type W = Vec<(usize, f64)>;

fn one(i: usize) -> W {
    vec![(i, 1.0)]
}
/// `a * wa + b * (1 - wa)`, equal sources merged.
fn mix(a: &W, b: &W, wa: f64) -> W {
    let mut o: W = Vec::with_capacity(a.len() + b.len());
    for (s, k) in [(a, wa), (b, 1.0 - wa)] {
        for &(i, w) in s {
            let w = w * k;
            match o.iter_mut().find(|x| x.0 == i) {
                Some(x) => x.1 += w,
                None => o.push((i, w)),
            }
        }
    }
    o.retain(|x| x.1 != 0.0);
    if o.is_empty() {
        o.extend(a.first().map(|x| (x.0, 1.0)));
    }
    o
}

/// One joined prim under construction: local point recipes (`None` = deleted) and (local point, vertex recipe) list.
struct Chain {
    pts: Vec<Option<W>>,
    verts: Vec<(usize, W)>,
    closed: bool,
    first_prim: usize,
}

struct Ctx<'a> {
    src: &'a Detail,
    blend: bool,
    t: f64,
    bias: f64,
}

impl Ctx<'_> {
    fn vp(&self, v: usize) -> usize {
        self.src.vertex_point.get(v).map_or(0, |&p| p as usize)
    }
    fn pos(&self, w: &W) -> [f64; 3] {
        let p = self.src.p();
        let mut o = [0.0; 3];
        for &(i, k) in w {
            if let Some(x) = p.get(i) {
                for c in 0..3 {
                    o[c] += k * f64::from(x[c]);
                }
            }
        }
        o
    }
    /// A prim's source vertices, opened (closed prims repeat their first vertex at the end).
    fn piece(&self, prim: usize) -> Vec<usize> {
        let Some(pr) = self.src.prims.get(prim) else { return Vec::new() };
        let mut v: Vec<usize> = (pr.verts.start as usize..pr.verts.end as usize).collect();
        if matches!(pr.kind, PrimKind::Poly { closed: true }) {
            v.extend(v.first().copied());
        }
        v
    }
    fn end_pos(&self, piece: &[usize], last: bool) -> [f64; 3] {
        let v = if last { piece.last() } else { piece.first() };
        v.map_or([0.0; 3], |&v| self.pos(&one(self.vp(v))))
    }
}

/// Houdini's end-distance tie tolerance (hython: distances closer than 1e-5 tie at every scale, so the earlier choice wins).
const TIE: f64 = 1e-5;
fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|c| (a[c] - b[c]).powi(2)).sum::<f64>().sqrt()
}

impl Chain {
    fn push_pt(&mut self, w: W) -> usize {
        self.pts.push(Some(w));
        self.pts.len() - 1
    }
    fn start(cx: &Ctx, prim: usize, flip: bool) -> Chain {
        let mut c = Chain { pts: Vec::new(), verts: Vec::new(), closed: false, first_prim: prim };
        let piece = cx.piece(prim);
        let closed = cx.src.prims.get(prim).is_some_and(|p| matches!(p.kind, PrimKind::Poly { closed: true }));
        let body = if closed { &piece[..piece.len().saturating_sub(1)] } else { &piece[..] };
        for &v in body {
            let p = c.push_pt(one(cx.vp(v)));
            c.verts.push((p, one(v)));
        }
        if let (true, Some(&v)) = (closed, piece.first()) {
            let p = c.push_pt(one(cx.vp(v)));
            c.verts.push((p, one(v)));
        }
        if flip {
            c.verts.reverse();
        }
        c
    }
    fn pt(&self, i: usize) -> W {
        self.pts.get(i).cloned().flatten().unwrap_or_default()
    }
    fn end(&self, cx: &Ctx) -> [f64; 3] {
        self.verts.last().map_or([0.0; 3], |v| cx.pos(&self.pt(v.0)))
    }
    /// Append source vertices `nv` (already oriented).
    fn append(&mut self, cx: &Ctx, nv: &[usize]) {
        let Some(&f) = nv.first() else { return };
        if !(cx.blend && cx.t > 0.0) || self.verts.is_empty() {
            for &v in nv {
                let p = self.push_pt(one(cx.vp(v)));
                self.verts.push((p, one(v)));
            }
            return;
        }
        let n = self.verts.len();
        let (lp, lw) = self.verts[n - 1].clone();
        let (l2p, l2w) = if n >= 2 { self.verts[n - 2].clone() } else { (lp, lw.clone()) };
        let f2 = nv.get(1).copied().unwrap_or(f);
        let (lpt, l2pt, fpt, f2pt) = (self.pt(lp), self.pt(l2p), one(cx.vp(f)), one(cx.vp(f2)));
        self.pts[lp] = Some(mix(&lpt, &fpt, cx.bias));
        self.verts.pop();
        let cut = cx.t < 1.0;
        if cut {
            let c = self.push_pt(mix(&l2pt, &lpt, cx.t));
            self.verts.push((c, mix(&l2w, &lw, cx.t)));
        }
        self.verts.push((lp, mix(&lw, &one(f), cx.bias)));
        if cut {
            let c = self.push_pt(mix(&f2pt, &fpt, cx.t));
            self.verts.push((c, mix(&one(f2), &one(f), cx.t)));
        }
        for &v in &nv[1..] {
            let p = self.push_pt(one(cx.vp(v)));
            self.verts.push((p, one(v)));
        }
    }
    fn close(&mut self, cx: &Ctx) {
        self.closed = true;
        let n = self.verts.len();
        if !(cx.blend && cx.t > 0.0) || n < 2 {
            return;
        }
        let (lp, lw) = self.verts[n - 1].clone();
        let (l2p, l2w) = self.verts[n - 2].clone();
        let (fp, fw) = self.verts[0].clone();
        let (f2p, f2w) = self.verts[1].clone();
        let (lpt, l2pt, fpt, f2pt) = (self.pt(lp), self.pt(l2p), self.pt(fp), self.pt(f2p));
        self.pts[lp] = Some(mix(&lpt, &fpt, cx.bias));
        let blend_v = mix(&lw, &fw, cx.bias);
        let cut = cx.t < 1.0;
        let mut verts = Vec::with_capacity(n + 1);
        let mut tail = Vec::with_capacity(2);
        if cut {
            let cl = self.push_pt(mix(&l2pt, &lpt, cx.t));
            tail.push((cl, mix(&l2w, &lw, cx.t)));
            let cf = self.push_pt(mix(&f2pt, &fpt, cx.t));
            verts.push((cf, mix(&f2w, &fw, cx.t)));
        }
        verts.extend(self.verts[1..n - 1].iter().cloned());
        verts.extend(tail);
        verts.push((lp, blend_v));
        if fp != lp && !verts.iter().any(|v| v.0 == fp) {
            self.pts[fp] = None;
        }
        self.verts = verts;
    }
}

/// Subgroups of the selected prims per `joinop` / `inc` (a one-prim run is left out unless `keep_single`).
fn subgroups(sel: &[usize], op: u32, inc: i32, keep_single: bool) -> Vec<Vec<usize>> {
    let n = sel.len();
    let inc = usize::try_from(inc).unwrap_or(0);
    let sets: Vec<Vec<usize>> = match op {
        JOIN_GROUP if inc >= 2 => sel.chunks(inc).map(<[usize]>::to_vec).collect(),
        JOIN_GROUP => sel.iter().map(|&p| vec![p]).collect(),
        JOIN_SKIP if inc >= 2 && inc.saturating_mul(2) <= n => {
            (0..inc).map(|r| sel.iter().copied().skip(r).step_by(inc).collect()).collect()
        }
        _ => vec![sel.to_vec()],
    };
    sets.into_iter().filter(|s| s.len() >= 2 || (keep_single && !s.is_empty())).collect()
}

/// `onlyconnected` chains inside one subgroup: B follows when its first point is the chain's last point.
fn connected(cx: &Ctx, set: &[usize]) -> Vec<Vec<usize>> {
    let ends = |p: usize, last: bool| -> Option<usize> {
        let pc = cx.piece(p);
        let v = if last { pc.last() } else { pc.first() };
        v.map(|&v| cx.vp(v))
    };
    let mut used = vec![false; set.len()];
    let mut out = Vec::new();
    for i in 0..set.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let mut chain = vec![set[i]];
        // bounded: each step marks one prim used
        while let Some(end) = chain.last().and_then(|&p| ends(p, true)) {
            let Some(j) = (0..set.len()).find(|&j| !used[j] && ends(set[j], false) == Some(end)) else { break };
            used[j] = true;
            chain.push(set[j]);
        }
        out.push(chain);
    }
    out
}

fn build(cx: &Ctx, set: &[usize], proximity: bool, loop_: bool) -> Option<Chain> {
    let (&first, rest) = set.split_first()?;
    let mut flip_first = false;
    let mut flip_second = false;
    if proximity && let Some(&second) = rest.first() {
        let (a, b) = (cx.piece(first), cx.piece(second));
        let (a0, a1, b0, b1) =
            (cx.end_pos(&a, false), cx.end_pos(&a, true), cx.end_pos(&b, false), cx.end_pos(&b, true));
        // tie order (hython): (end,start) before (start,start) before (end,end); (start,end) last
        let opts = [
            (dist(a1, b0), false, false),
            (dist(a0, b0), true, false),
            (dist(a1, b1), false, true),
            (dist(a0, b1), true, true),
        ];
        let mut best = opts[0];
        for o in &opts[1..] {
            if o.0 < best.0 - TIE {
                best = *o;
            }
        }
        (flip_first, flip_second) = (best.1, best.2);
    }
    let mut c = Chain::start(cx, first, flip_first);
    for (k, &p) in rest.iter().enumerate() {
        let mut nv = cx.piece(p);
        let flip = if k == 0 {
            flip_second
        } else {
            proximity && {
                let e = c.end(cx);
                dist(e, cx.end_pos(&nv, true)) < dist(e, cx.end_pos(&nv, false)) - TIE
            }
        };
        if flip {
            nv.reverse();
        }
        c.append(cx, &nv);
    }
    if loop_ {
        c.close(cx);
    }
    Some(c)
}

fn float01(ctx: &CookCtx, n: &str) -> Result<f64, CookError> {
    let v = ctx.params.float(n).ok_or_else(|| CookError::BadParam(format!("join.{n}: missing or not a float")))?;
    if !v.is_finite() {
        return Err(CookError::BadParam(format!("join.{n}: not finite")));
    }
    Ok(f64::from(v).clamp(0.0, 1.0))
}

impl Node<Detail> for Join {
    fn type_name(&self) -> &'static str {
        "join"
    }
    fn inputs(&self) -> &[InputSpec] {
        &INPUTS
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let Some(src) = inputs.first() else { return Ok(Arc::new(Detail::default())) };
        let p = ctx.params;
        let sel = selection(OP, src, p.string("group").unwrap_or(""))?;
        let cx = Ctx { src, blend: toggle(p, "blend")?, t: float01(ctx, "tolerance")?, bias: float01(ctx, "bias")? };
        let (proximity, loop_, keep, only) =
            (toggle(p, "proximity")?, toggle(p, "loop")?, toggle(p, "prim")?, toggle(p, "onlyconnected")?);
        let op = p.menu("joinop").unwrap_or(JOIN_ALL);
        let inc = p.int("inc").unwrap_or(2);
        let mut chosen = Vec::new();
        for (i, pr) in src.prims.iter().enumerate() {
            if !sel.as_ref().is_none_or(|s| s.get(i).copied().unwrap_or(false)) || pr.verts.is_empty() {
                continue;
            }
            if !matches!(pr.kind, PrimKind::Poly { .. } | PrimKind::Polyline) {
                return Err(CookError::Unsupported(format!(
                    "join: prim {i} is not a polygon / polyline (surface and NURBS joins are not modelled)"
                )));
            }
            chosen.push(i);
        }
        let mut sets = subgroups(&chosen, op, inc, false);
        if only {
            sets = subgroups(&chosen, op, inc, true).iter().flat_map(|s| connected(&cx, s)).collect();
        }
        let chains: Vec<Chain> = sets.iter().filter_map(|s| build(&cx, s, proximity && !only, loop_)).collect();
        if chains.is_empty() {
            return Ok(Arc::clone(src));
        }
        let mut joined = vec![false; src.prims.len()];
        if !keep {
            for s in &sets {
                for &q in s {
                    joined[q] = true;
                }
            }
        }
        let np = src.point_count as usize;
        let mut used_kept = vec![false; np];
        let mut used_joined = vec![false; np];
        for (i, pr) in src.prims.iter().enumerate() {
            for v in pr.verts.clone() {
                let pt = cx.vp(v as usize);
                if let Some(u) = if joined[i] { used_joined.get_mut(pt) } else { used_kept.get_mut(pt) } {
                    *u = true;
                }
            }
        }
        let mut point_w: Vec<W> = Vec::new();
        let mut remap = vec![u32::MAX; np];
        for i in 0..np {
            if used_kept[i] || !used_joined[i] {
                remap[i] =
                    u32::try_from(point_w.len()).map_err(|_| CookError::Failed("join: point id overflow".into()))?;
                point_w.push(one(i));
            }
        }
        let mut vertex_point = Vec::new();
        let mut vert_w: Vec<W> = Vec::new();
        let mut prims = Vec::new();
        let mut prim_src = Vec::new();
        let vid = |n: usize| u32::try_from(n).map_err(|_| CookError::Failed("join: vertex id overflow".into()));
        for (i, pr) in src.prims.iter().enumerate().filter(|(i, _)| !joined[*i]) {
            let start = vid(vertex_point.len())?;
            for v in pr.verts.clone() {
                vertex_point.push(remap.get(cx.vp(v as usize)).copied().unwrap_or(0));
                vert_w.push(one(v as usize));
            }
            prims.push(Prim { kind: pr.kind.clone(), verts: start..vid(vertex_point.len())? });
            prim_src.push(Some(i));
        }
        for c in &chains {
            let mut local = vec![u32::MAX; c.pts.len()];
            for (k, w) in c.pts.iter().enumerate() {
                if let Some(w) = w {
                    local[k] = vid(point_w.len())?;
                    point_w.push(w.clone());
                }
            }
            let start = vid(vertex_point.len())?;
            for (lp, w) in &c.verts {
                vertex_point.push(local.get(*lp).copied().unwrap_or(0));
                vert_w.push(w.clone());
            }
            let kind = match src.prims.get(c.first_prim).map(|p| &p.kind) {
                _ if c.closed => PrimKind::Poly { closed: true },
                Some(PrimKind::Polyline) => PrimKind::Polyline,
                _ => PrimKind::Poly { closed: false },
            };
            prims.push(Prim { kind, verts: start..vid(vertex_point.len())? });
            prim_src.push(Some(c.first_prim));
        }
        let pos: Vec<[f32; 3]> = point_w.iter().map(|w| cx.pos(w).map(|x| x as f32)).collect();
        let vert_src: Vec<Option<usize>> = vert_w.iter().map(|w| w.first().map(|x| x.0)).collect();
        let mut out = rebuild_w(OP, src, vertex_point, prims, &point_w, &vert_src, &prim_src, pos)?;
        for a in src.attrs.iter().filter(|a| a.class == AttrClass::Vertex) {
            let data = blend_weighted(&a.data, &vert_w);
            out.add_attr(Attribute { name: a.name.clone(), class: a.class, data, type_info: a.type_info });
        }
        Ok(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hexe_geo::{AttrData, Group, TypeInfo};
    use hexe_graph::ParamSet;

    type Curve<'a> = (&'a [[f32; 3]], bool);

    fn curves(cs: &[Curve]) -> Detail {
        let mut d = Detail::default();
        for (i, (pts, closed)) in cs.iter().enumerate() {
            let r = d.add_points(pts.len() as u32);
            for (k, p) in pts.iter().enumerate() {
                d.p_mut()[r.start as usize + k] = *p;
            }
            let ids: Vec<u32> = r.collect();
            d.add_poly(&ids, *closed);
            let _ = i;
        }
        d
    }
    /// Houdini probe attrs: point Cd = (curve, index, 0), vertex uv = (index, curve), prim id = 10 + curve.
    fn with_attrs(mut d: Detail, cs: &[Curve]) -> Detail {
        let mut cd = Vec::new();
        let mut uv = Vec::new();
        for (i, (pts, _)) in cs.iter().enumerate() {
            for k in 0..pts.len() {
                cd.push([i as f32, k as f32, 0.0]);
                uv.push([k as f32, i as f32, 0.0]);
            }
        }
        let id = (0..cs.len() as i32).map(|i| i + 10).collect();
        d.add_attr(Attribute {
            name: "Cd".into(),
            class: AttrClass::Point,
            data: AttrData::V3(cd),
            type_info: TypeInfo::Color,
        });
        d.add_attr(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(uv),
            type_info: TypeInfo::None,
        });
        d.add_attr(Attribute {
            name: "id".into(),
            class: AttrClass::Prim,
            data: AttrData::I32(id),
            type_info: TypeInfo::None,
        });
        d
    }
    fn params(ov: &[(&str, ParamValue)]) -> ParamSet {
        let mut p = ParamSet::new();
        for s in spec() {
            p = p.with(&s.name, s.default.clone());
        }
        for (k, v) in ov {
            p = p.with(k, v.clone());
        }
        p
    }
    fn cook(d: Detail, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        let p = params(ov);
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &p, log: &|_| {} };
        new().cook(&ctx, &[Arc::new(d)])
    }
    fn xs(d: &Detail) -> Vec<f32> {
        d.p().iter().map(|p| (p[0] * 1000.0).round() / 1000.0).collect()
    }
    fn prim_pts(d: &Detail, i: usize) -> Vec<u32> {
        let r = &d.prims[i].verts;
        d.vertex_point[r.start as usize..r.end as usize].to_vec()
    }
    fn prim_pos(d: &Detail, i: usize) -> Vec<[f32; 3]> {
        prim_pts(d, i).iter().map(|&p| d.p()[p as usize].map(|x| (x * 1000.0).round() / 1000.0)).collect()
    }
    const A: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    const B: [[f32; 3]; 3] = [[3.0, 0.0, 0.0], [4.0, 0.0, 0.0], [5.0, 1.0, 0.0]];
    const C: [[f32; 3]; 2] = [[5.0, 3.0, 0.0], [3.0, 3.0, 0.0]];
    fn off() -> (&'static str, ParamValue) {
        ("blend", ParamValue::B(false))
    }
    fn tol(t: f32) -> (&'static str, ParamValue) {
        ("tolerance", ParamValue::F(t))
    }

    #[test]
    fn defaults_blend_two_curves_into_one_shared_point() {
        // hython: blend on, tolerance 1, bias .5 -> 5 pts [0,1,2.5,4,(5,1)]
        let out = cook(curves(&[(&A, false), (&B, false)]), &[]).expect("cooks");
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.5, 4.0, 5.0]);
        assert_eq!(prim_pts(&out, 0), vec![0, 1, 2, 3, 4]);
        assert_eq!(out.prims[0].kind, PrimKind::Poly { closed: false });
    }

    #[test]
    fn tolerance_cuts_both_end_segments_in_houdini_point_order() {
        // hython (tolerance 0.727): pts [0,1,2.5,1.273,3.727,4,5], verts [0,1,3,2,4,5,6]
        let out = cook(curves(&[(&A, false), (&B, false)]), &[tol(0.727)]).expect("cooks");
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.5, 1.273, 3.727, 4.0, 5.0]);
        assert_eq!(prim_pts(&out, 0), vec![0, 1, 3, 2, 4, 5, 6]);
        // three curves: second joint cuts on B's last and C's first segment
        let out = cook(curves(&[(&A, false), (&B, false), (&C, false)]), &[tol(0.727)]).expect("cooks");
        assert_eq!(
            prim_pos(&out, 0),
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.273, 0.0, 0.0],
                [2.5, 0.0, 0.0],
                [3.727, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.273, 0.273, 0.0],
                [5.0, 2.0, 0.0],
                [3.546, 3.0, 0.0],
                [3.0, 3.0, 0.0]
            ]
        );
        assert_eq!(prim_pts(&out, 0), vec![0, 1, 3, 2, 4, 5, 7, 6, 8, 9]);
    }

    #[test]
    fn bias_weights_the_next_prim_by_one_minus_bias_with_attributes() {
        // hython: tol .5 bias .25 -> blend 2.75 Cd (.75,.5), cuts 1.5 / 3.5; uv blend (.5,.75); prim id 10
        let cs: [Curve; 2] = [(&A, false), (&B, false)];
        let out = cook(with_attrs(curves(&cs), &cs), &[tol(0.5), ("bias", ParamValue::F(0.25))]).expect("cooks");
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.75, 1.5, 3.5, 4.0, 5.0]);
        let Some(AttrData::V3(cd)) = out.attr(AttrClass::Point, "Cd").map(|a| &a.data) else { panic!("Cd") };
        assert_eq!(cd[2], [0.75, 0.5, 0.0]);
        assert_eq!(cd[3], [0.0, 1.5, 0.0]);
        assert_eq!(cd[4], [1.0, 0.5, 0.0]);
        let Some(AttrData::V3(uv)) = out.attr(AttrClass::Vertex, "uv").map(|a| &a.data) else { panic!("uv") };
        let uv2: Vec<[f32; 2]> = uv.iter().map(|u| [u[0], u[1]]).collect();
        assert_eq!(uv2, vec![[0.0, 0.0], [1.0, 0.0], [1.5, 0.0], [0.5, 0.75], [0.5, 1.0], [1.0, 1.0], [2.0, 1.0]]);
        assert_eq!(out.attr(AttrClass::Prim, "id").map(|a| &a.data), Some(&AttrData::I32(vec![10])));
    }

    #[test]
    fn proximity_flips_the_first_pair_then_only_the_next_prim() {
        // hython first-flip: A + [(-1,0),(-2,1)] no blend -> verts [2,1,0,3,4]
        let d = [[-1.0, 0.0, 0.0], [-2.0, 1.0, 0.0]];
        let out = cook(curves(&[(&A, false), (&d, false)]), &[off()]).expect("cooks");
        assert_eq!(prim_pts(&out, 0), vec![2, 1, 0, 3, 4]);
        // hython A, C, B: order kept, C and B flipped
        let out = cook(curves(&[(&A, false), (&C, false), (&B, false)]), &[off()]).expect("cooks");
        let x: Vec<[f32; 2]> = prim_pos(&out, 0).iter().map(|p| [p[0], p[1]]).collect();
        assert_eq!(
            x,
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 3.0], [5.0, 3.0], [5.0, 1.0], [4.0, 0.0], [3.0, 0.0]]
        );
        // proximity off: plain order
        let out = cook(curves(&[(&A, false), (&C, false), (&B, false)]), &[off(), ("proximity", ParamValue::B(false))])
            .expect("cooks");
        assert_eq!(prim_pts(&out, 0), (0..8).collect::<Vec<_>>());
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.0, 5.0, 3.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn proximity_ties_within_1e5_keep_the_earlier_choice() {
        // hython tie.py / tie2.py: A (0,0)->(5,0), B (0,1)->(5,1+e): |e| < 1e-5 ties -> (start,start) flips A
        let run = |e: f32, rev: bool| {
            let mut b = vec![[0.0, 1.0, 0.0], [2.0, 1.0, 0.0], [5.0, 1.0 + e, 0.0]];
            if rev {
                b.reverse();
            }
            let a = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [5.0, 0.0, 0.0]];
            let out = cook(curves(&[(&a, false), (&b, false)]), &[off()]).expect("cooks");
            prim_pos(&out, 0)[0][0]
        };
        assert_eq!(run(0.0, false), 5.0);
        assert_eq!(run(-9.5e-6, false), 5.0);
        assert_eq!(run(-1.05e-5, false), 0.0);
        assert_eq!(run(-1e-3, false), 0.0);
        // (end,start) wins its tie with (start,end)
        assert_eq!(run(0.0, true), 0.0);
        assert_eq!(run(1e-3, true), 5.0);
    }

    #[test]
    fn blend_after_flip_reuses_the_first_prims_point_slot() {
        // hython blend+flip: reversed A, reversed B, tol .5 -> pts [2.5,1,0,1.5,3.5,4,5] verts [2,1,3,0,4,5,6]
        let ar: Vec<[f32; 3]> = A.iter().rev().copied().collect();
        let br: Vec<[f32; 3]> = B.iter().rev().copied().collect();
        let out = cook(curves(&[(&ar, false), (&br, false)]), &[tol(0.5)]).expect("cooks");
        assert_eq!(xs(&out), vec![2.5, 1.0, 0.0, 1.5, 3.5, 4.0, 5.0]);
        assert_eq!(prim_pts(&out, 0), vec![2, 1, 3, 0, 4, 5, 6]);
    }

    #[test]
    fn single_point_prim_and_closed_input_follow_houdini() {
        // hython 1pt middle tol .5: pts [0,1,2.5,1.5,3,2.75,3.5,4,5] verts [0,1,3,2,5,4,6,7,8]
        let one_pt = [[3.0, 0.0, 0.0]];
        let out = cook(curves(&[(&A, false), (&one_pt, false), (&B, false)]), &[tol(0.5)]).expect("cooks");
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.5, 1.5, 3.0, 2.75, 3.5, 4.0, 5.0]);
        assert_eq!(prim_pts(&out, 0), vec![0, 1, 3, 2, 5, 4, 6, 7, 8]);
        // closed A is opened with a copy of its first point (hython: [0,1,2,0,3,4,5] no blend)
        let out = cook(curves(&[(&A, true), (&B, false)]), &[off()]).expect("cooks");
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.0, 0.0, 3.0, 4.0, 5.0]);
        let out = cook(curves(&[(&A, true), (&B, false)]), &[tol(0.5)]).expect("cooks");
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.0, 1.5, 1.0, 3.5, 4.0, 5.0]);
        assert_eq!(prim_pts(&out, 0), vec![0, 1, 2, 4, 3, 5, 6, 7]);
    }

    #[test]
    fn loop_blend_drops_the_first_point_and_starts_at_its_cut() {
        // hython loop tol .5: pts [1,2.5,1.5,3.5,4,(2.5,.5),(4.5,.5),0.5] verts [7,0,2,1,3,4,6,5] closed
        let out = cook(curves(&[(&A, false), (&B, false)]), &[tol(0.5), ("loop", ParamValue::B(true))]).expect("cooks");
        assert_eq!(xs(&out), vec![1.0, 2.5, 1.5, 3.5, 4.0, 2.5, 4.5, 0.5]);
        assert_eq!(prim_pts(&out, 0), vec![7, 0, 2, 1, 3, 4, 6, 5]);
        assert_eq!(out.prims[0].kind, PrimKind::Poly { closed: true });
        // tolerance 1: [1,2.5,4,(2.5,.5)]
        let out = cook(curves(&[(&A, false), (&B, false)]), &[("loop", ParamValue::B(true))]).expect("cooks");
        assert_eq!(xs(&out), vec![1.0, 2.5, 4.0, 2.5]);
        assert_eq!(prim_pts(&out, 0), vec![0, 1, 2, 3]);
    }

    fn segs(n: usize) -> Detail {
        let mut d = Detail::default();
        for i in 0..n {
            let r = d.add_points(2);
            d.p_mut()[r.start as usize] = [(2 * i) as f32, 0.0, 0.0];
            d.p_mut()[r.start as usize + 1] = [(2 * i + 1) as f32, 0.0, 0.0];
            d.add_poly(&[r.start, r.start + 1], false);
        }
        d
    }
    /// Each output prim as the list of source segment indices it holds.
    fn members(d: &Detail) -> Vec<Vec<usize>> {
        (0..d.prims.len()).map(|i| prim_pos(d, i).iter().step_by(2).map(|p| p[0] as usize / 2).collect()).collect()
    }

    #[test]
    fn joinop_subgroups_match_houdini() {
        let run = |n: usize, op: u32, inc: i32| {
            members(
                &cook(segs(n), &[off(), ("joinop", ParamValue::Menu(op)), ("inc", ParamValue::I(inc))]).expect("cooks"),
            )
        };
        assert_eq!(run(5, JOIN_SKIP, 2), vec![vec![0, 2, 4], vec![1, 3]]);
        assert_eq!(run(5, JOIN_SKIP, 3), vec![vec![0, 1, 2, 3, 4]]);
        assert_eq!(run(7, JOIN_SKIP, 3), vec![vec![0, 3, 6], vec![1, 4], vec![2, 5]]);
        assert_eq!(run(6, JOIN_SKIP, 4), vec![vec![0, 1, 2, 3, 4, 5]]);
        assert_eq!(run(9, JOIN_SKIP, 4), vec![vec![0, 4, 8], vec![1, 5], vec![2, 6], vec![3, 7]]);
        assert_eq!(run(5, JOIN_SKIP, 0), vec![vec![0, 1, 2, 3, 4]]);
        // a leftover single stays untouched and comes first
        assert_eq!(run(5, JOIN_GROUP, 2), vec![vec![4], vec![0, 1], vec![2, 3]]);
        assert_eq!(run(5, JOIN_GROUP, 3), vec![vec![0, 1, 2], vec![3, 4]]);
        assert_eq!(run(7, JOIN_GROUP, 3), vec![vec![6], vec![0, 1, 2], vec![3, 4, 5]]);
        assert_eq!(run(5, JOIN_GROUP, 0), vec![vec![0], vec![1], vec![2], vec![3], vec![4]]);
    }

    #[test]
    fn group_parm_keeps_the_rest_first_and_keep_prims_copies() {
        // hython: prims A,B,C,D, group {A,C,D}: B (id 11) first with its points, then the joined prim (id 10)
        let cs: [Curve; 4] = [(&A, false), (&B, false), (&C, false), (&[[8.0, 0.0, 0.0], [9.0, 0.0, 0.0]], false)];
        let mut d = with_attrs(curves(&cs), &cs);
        d.groups.push(Group { name: "sel".into(), class: AttrClass::Prim, mask: vec![true, false, true, true] });
        let out = cook(d, &[off(), ("proximity", ParamValue::B(false)), ("group", ParamValue::S("sel".into()))])
            .expect("cooks");
        assert_eq!(out.attr(AttrClass::Prim, "id").map(|a| &a.data), Some(&AttrData::I32(vec![11, 10])));
        assert_eq!(xs(&out), vec![3.0, 4.0, 5.0, 0.0, 1.0, 2.0, 5.0, 3.0, 8.0, 9.0]);
        let g = out.group(AttrClass::Prim, "sel").expect("group follows");
        assert_eq!(g.mask, vec![false, true]);
        // keep prims: originals stay, the joined prim gets fresh points
        let out = cook(curves(&[(&A, false), (&B, false)]), &[off(), ("prim", ParamValue::B(true))]).expect("cooks");
        assert_eq!((out.point_count, out.prims.len()), (12, 3));
        assert_eq!(prim_pts(&out, 2), (6..12).collect::<Vec<_>>());
    }

    #[test]
    fn only_connected_follows_shared_points_without_flips() {
        // hython probe4: A=0,1,2  B=2,3,4  C=5,6  D=4,7 -> [A B D] (shared points duplicated), then C
        let mut d = Detail::default();
        d.add_points(8);
        *d.p_mut() = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [9.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [5.0, 1.0, 0.0],
        ];
        for ids in [&[0, 1, 2][..], &[2, 3, 4], &[5, 6], &[4, 7]] {
            d.add_poly(ids, false);
        }
        let only = ("onlyconnected", ParamValue::B(true));
        let out = cook(d.clone(), &[off(), only.clone()]).expect("cooks");
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.0, 2.0, 3.0, 4.0, 4.0, 5.0, 9.0, 10.0]);
        assert_eq!(prim_pts(&out, 0), (0..8).collect::<Vec<_>>());
        assert_eq!(prim_pts(&out, 1), vec![8, 9]);
        let out = cook(d, &[tol(0.5), only]).expect("cooks");
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.0, 1.5, 2.5, 3.0, 4.0, 3.5, 4.5, 5.0, 9.0, 10.0]);
        assert_eq!(prim_pts(&out, 0), vec![0, 1, 3, 2, 4, 5, 7, 6, 8, 9]);
        // a reversed neighbour is not connected (no flips)
        let mut e = Detail::default();
        e.add_points(5);
        *e.p_mut() = (0..5).map(|i| [i as f32, 0.0, 0.0]).collect();
        e.add_poly(&[2, 1, 0], false);
        e.add_poly(&[2, 3, 4], false);
        let out = cook(e, &[off(), ("onlyconnected", ParamValue::B(true))]).expect("cooks");
        assert_eq!(out.prims.len(), 2);
    }

    /// Synthetic hython cases (`oracle/houdini/join/sop_join_cases.py`): `arcs2` (two 127-point arcs, near-tied ends,
    /// tolerance 0.727) and `arcs7` (seven reversed 19-point arcs, tolerance 1), blend + proximity: Houdini 20.5 in / out.
    #[test]
    fn recorded_arcs_match_houdini_exactly() {
        let fx = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/sop-join/");
        for (n, t) in [("arcs2", 0.727f32), ("arcs7", 1.0)] {
            let read = |f: &str| {
                let bytes = std::fs::read(format!("{fx}{n}_{f}.geo")).expect("fixture");
                hexe_geo::read_geo(&bytes).expect("geo")
            };
            let (input, h) = (read("in"), read("houdini"));
            let d = cook(input, &[tol(t)]).expect("cooks");
            assert_eq!(
                (d.point_count, d.prims.len(), d.vertex_point.len()),
                (h.point_count, h.prims.len(), h.vertex_point.len()),
                "{n}"
            );
            for (k, (a, b)) in d.p().iter().zip(h.p()).enumerate() {
                assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-5), "{n} P[{k}] hexe {a:?} houdini {b:?}");
            }
            assert_eq!(d.vertex_point, h.vertex_point, "{n} vertex order");
            for (a, b) in d.prims.iter().zip(&h.prims) {
                assert_eq!(
                    matches!(a.kind, PrimKind::Poly { closed: true }),
                    matches!(b.kind, PrimKind::Poly { closed: true }),
                    "{n} closedness"
                );
            }
        }
    }

    #[test]
    fn hostile_inputs_are_errors_or_passthrough() {
        let two = || curves(&[(&A, false), (&B, false)]);
        assert!(matches!(cook(two(), &[("tolerance", ParamValue::F(f32::NAN))]), Err(CookError::BadParam(_))));
        assert!(matches!(cook(two(), &[("bias", ParamValue::F(f32::INFINITY))]), Err(CookError::BadParam(_))));
        assert!(matches!(cook(two(), &[("group", ParamValue::S("nope".into()))]), Err(CookError::Failed(_))));
        // out-of-range values clamp (Houdini: tolerance 1.5 == 1)
        let out = cook(two(), &[tol(1.5)]).expect("cooks");
        assert_eq!(xs(&out), vec![0.0, 1.0, 2.5, 4.0, 5.0]);
        let mut d = two();
        d.prims[1].kind = PrimKind::Sphere { xform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] };
        assert!(matches!(cook(d, &[]), Err(CookError::Unsupported(_))));
        // one prim / empty input: unchanged
        let out = cook(curves(&[(&A, false)]), &[]).expect("cooks");
        assert_eq!(out.point_count, 3);
        let out = cook(Detail::default(), &[("inc", ParamValue::I(i32::MIN)), ("joinop", ParamValue::Menu(JOIN_SKIP))])
            .expect("cooks");
        assert_eq!(out.prims.len(), 0);
    }
}
