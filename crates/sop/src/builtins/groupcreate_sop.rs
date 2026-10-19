//! Group Create SOP (token `groupcreate`, Houdini 20.5 `groupcreate`; the legacy `group` SOP stays as-is) + the pattern
//! evaluator shared with `groupcombine`.
//!
//! # Semantics (hython 20.5 probes, `fixtures/sop-groupcreate*`, `oracle/houdini/parmgaps/`)
//! Creates / edits ONE group `groupname` of class `grouptype` (primitive | point | edge | vertex, the last two below).
//! Name: whitespace and `*` `?` become `_` (`x y` -> `x_y`); an empty name passes the input through.
//! * `groupbase` (on) + `basegroup`: element pattern, empty = every element. Tokens (whitespace separated, UNIONED):
//!   `*`; `N`; `N-M` (reversed range accepted: `2-0` = 0-2); `N-M:step`; group name of the same class (`*`/`?` glob);
//!   `!X` = COMPLEMENT of X (unioned: `0-2 !1` = everything); `^X` = subtract X from the running set. An unknown name or an id
//!   past the end selects nothing (Houdini warns); a `,` token -> `Unsupported`. `groupbase` off with no other criterion = EMPTY group.
//! * `groupbounding`: `boundtype` bbox or bsphere (object / volume / hull -> `Unsupported`), centre `t`, full extent `size`
//!   (sphere = the ellipsoid inscribed in that box), boundary INCLUSIVE, a negative extent selects nothing (a zero sphere
//!   radius too). Point = P inside. Prim = EVERY point inside, or ANY point with `includenotwhollycontained`. AND-ed with
//!   the base set (also when `groupbase` is off).
//! * `groupnormal`: keep elements whose normal is within `angle` degrees of `dir` (inclusive, 1e-6 slack; `oppositenormals` also
//!   accepts `-dir`). Prim normal = Newell, point = normalised sum of its prims' normals. Zero `dir` or no normal -> excluded
//!   (empty group, no error).
//! * `mergeop` vs the existing group of that class + name: replace | union | intersect | subtract. The group is always created.
//!
//!
//! ## Edge groups (`grouptype` edge; hython 20.5 on box / grid / ellipsoid / torus)
//! The edge list is ORDERED (written in that order to `.geo`): the mesh edges are every polygon / polyline segment, prim
//! order then corner order (closing edge of a closed poly last), each unordered pair once with its first orientation.
//! Pattern tokens, left to right; a union APPENDS (an edge already present keeps its place and orientation):
//! * `*` every mesh edge; `N` / `N-M` = the edges of those prims, prim by prim in corner order. An id past the last prim
//!   or a `:step` EMPTIES THE WHOLE PATTERN; a reversed range selects nothing. `NeM` = edge M of prim N (corner M -> M+1),
//!   nothing when out of range.
//! * `pA-B-C...` = the path's consecutive pairs, oriented as written, only pairs that are mesh edges; `pA` = every mesh edge
//!   at point A. A vertex token (`NvM`) empties the whole pattern.
//! * a name (glob) = the edges of a primitive group's prims, then an edge group's edges; point / vertex / unknown names
//!   select nothing.
//! * `!X` appends every mesh edge not in X (mesh order); `^X` removes X's edges (`^NeM` is a no-op in Houdini 20.5).
//!
//! Bounding: wholly contained = both ends inside; `includenotwhollycontained` = the SEGMENT meets the region. Normal = the
//! normalised sum of its two point normals (point normals as above). `mergeop` union appends the new edges after the
//! existing ones, intersect / subtract keep the existing order. `groupedges` (Include by Edges) has no effect here.
//!
//! ## Vertex groups (`grouptype` vertex)
//! Pattern: `*`; `N` / `N-M` = every vertex of those prims (ids past the end ignored, reversed = nothing); `NvM` / `NvA-B`
//! = vertex M (A..=B) of prim N (out of range = nothing); a primitive group name = its prims' vertices, a vertex group name
//! = its vertices. Any `!` / `^` token, a `:step`, an edge token (`NeM`, `p...`) or an unknown name EMPTIES THE WHOLE
//! PATTERN (Houdini 20.5). Bounding: the vertex's point inside (partial = whole). `groupnormal` and `groupedges` have no
//! effect (every vertex passes).
//!
//! Not modelled: `geotype` filter, `ordered`, `groupedges*` on points / prims, `grouprandom*`, `camerapath`, `nonplanar`,
//! bracket globs `[bc]`, `,` increments.
use super::group_sop::{ANGLE_EPS, V3, dot, glob_match, normals, prim_verts, set_group, token_set, unit, widen};
use crate::SopNode;
use hexe_geo::{AttrClass, Detail, PrimKind};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSpec, ParamValue};
use std::collections::HashSet;
use std::sync::Arc;

const GT_PRIM: u32 = 0;
const GT_POINT: u32 = 1;
const GT_EDGE: u32 = 2;
const GT_VERTEX: u32 = 3;
const BOUND_BOX: u32 = 0;
const BOUND_SPHERE: u32 = 1;
const MERGE_UNION: u32 = 1;
const MERGE_INTERSECT: u32 = 2;
const MERGE_SUBTRACT: u32 = 3;
const OP: &str = "groupcreate";

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("groupname", ParamValue::S("group1".into())).label("Group Name"),
        ParamSpec::new("grouptype", ParamValue::Menu(GT_PRIM)).label("Group Type").menu(&[
            ("primitive", "Primitives"),
            ("point", "Points"),
            ("edge", "Edges"),
            ("vertex", "Vertices"),
        ]),
        ParamSpec::new("mergeop", ParamValue::Menu(0)).label("Merge Operation").menu(&[
            ("replace", "Replace Existing"),
            ("union", "Union with Existing"),
            ("intersect", "Intersect with Existing"),
            ("subtract", "Subtract from Existing"),
        ]),
        ParamSpec::new("groupbase", ParamValue::B(true)).label("Base Group"),
        ParamSpec::new("basegroup", ParamValue::S(String::new())).label("Base Group Pattern"),
        ParamSpec::new("ordered", ParamValue::B(false)).label("Ordered"),
        ParamSpec::new("groupbounding", ParamValue::B(false)).label("Keep in Bounding Regions"),
        ParamSpec::new("boundtype", ParamValue::Menu(0)).label("Bounding Type").menu(&[
            ("usebbox", "Bounding Box"),
            ("usebsphere", "Bounding Sphere"),
            ("usebobject", "Bounding Object"),
            ("usebvolume", "Bounding Volume"),
            ("usebconvex", "Bounding Convex Hull"),
        ]),
        ParamSpec::new("size", ParamValue::V3([1.0, 1.0, 1.0])).label("Size").range(0.0, 10.0),
        ParamSpec::new("t", ParamValue::V3([0.0, 0.0, 0.0])).label("Center").range(-10.0, 10.0),
        ParamSpec::new("includenotwhollycontained", ParamValue::B(false)).label("Include by Not Wholly Contained"),
        ParamSpec::new("groupnormal", ParamValue::B(false)).label("Keep by Normals"),
        ParamSpec::new("dir", ParamValue::V3([0.0, 0.0, 1.0])).label("Direction").range(-1.0, 1.0),
        ParamSpec::new("angle", ParamValue::F(180.0)).label("Spread Angle").range(0.0, 180.0),
        ParamSpec::new("oppositenormals", ParamValue::B(false)).label("Opposite Normals"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(GroupCreate { specs: spec() })
}
struct GroupCreate {
    specs: Vec<ParamSpec>,
}

/// Group name as Houdini stores it: whitespace and glob characters become `_`.
pub(super) fn sanitize(name: &str) -> String {
    name.trim().chars().map(|c| if c.is_whitespace() || c == '*' || c == '?' { '_' } else { c }).collect()
}
fn numeric_token(body: &str, n: usize) -> Option<Vec<bool>> {
    let (range, step) = match body.split_once(':') {
        Some((r, s)) => (r, s.parse::<u64>().ok().filter(|s| *s > 0)?),
        None => (body, 1),
    };
    let (lo, hi) = match range.split_once('-') {
        Some((a, b)) => (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?),
        None => {
            let v = range.parse::<u64>().ok()?;
            (v, v)
        }
    };
    let (lo, hi) = if lo > hi { (hi, lo) } else { (lo, hi) };
    Some((0..n as u64).map(|i| i >= lo && i <= hi && (i - lo) % step == 0).collect())
}
fn one_token(d: &Detail, class: AttrClass, body: &str, n: usize, who: &str) -> Result<Vec<bool>, CookError> {
    if body.contains(',') {
        return Err(CookError::Unsupported(format!("{who}: `,` increments in patterns are not modelled")));
    }
    if body.starts_with(|c: char| c.is_ascii_digit())
        && let Some(set) = numeric_token(body, n)
    {
        return Ok(set);
    }
    match token_set(d, class, body, n, who) {
        Err(CookError::Failed(_)) => Ok(vec![false; n]),
        other => other,
    }
}
/// Houdini group pattern over `class` (module docs). Empty pattern -> empty set (callers decide what that means).
pub(super) fn eval_pattern(d: &Detail, class: AttrClass, pat: &str, who: &str) -> Result<Vec<bool>, CookError> {
    let n = d.class_len(class);
    let mut run = vec![false; n];
    for raw in pat.split_whitespace() {
        let (kind, body) = match raw.chars().next() {
            Some('!') => ('!', &raw[1..]),
            Some('^') => ('^', &raw[1..]),
            _ => (' ', raw),
        };
        if body.is_empty() {
            continue;
        }
        let set = one_token(d, class, body, n, who)?;
        for (r, s) in run.iter_mut().zip(&set) {
            match kind {
                '!' => *r |= !*s,
                '^' => *r &= !*s,
                _ => *r |= *s,
            }
        }
    }
    Ok(run)
}
fn bad(m: impl AsRef<str>) -> CookError {
    CookError::BadParam(format!("{OP}: {}", m.as_ref()))
}
/// Point ids of each element of `class` (point / vertex / prim).
fn element_points(d: &Detail, class: AttrClass) -> Vec<Vec<usize>> {
    match class {
        AttrClass::Point => (0..d.point_count as usize).map(|i| vec![i]).collect(),
        AttrClass::Vertex => d.vertex_point.iter().map(|&i| vec![i as usize]).collect(),
        _ => d
            .prims
            .iter()
            .map(|pr| {
                d.vertex_point
                    .get(prim_verts(d, pr))
                    .map_or_else(Vec::new, |vs| vs.iter().map(|&v| v as usize).collect())
            })
            .collect(),
    }
}

/// Bounding region (`boundtype` bbox / bsphere): centre + half extents.
struct Region {
    c: V3,
    h: V3,
    sphere: bool,
}
impl Region {
    fn contains(&self, q: V3) -> bool {
        if self.sphere {
            self.h.iter().all(|&r| r > 0.0)
                && (0..3).map(|k| ((q[k] - self.c[k]) / self.h[k]).powi(2)).sum::<f64>() <= 1.0
        } else {
            (0..3).all(|k| (q[k] - self.c[k]).abs() <= self.h[k])
        }
    }
    /// Does the segment a-b meet the region (boundary inclusive)?
    fn meets_segment(&self, a: V3, b: V3) -> bool {
        if self.contains(a) || self.contains(b) {
            return true;
        }
        if self.sphere {
            if !self.h.iter().all(|&r| r > 0.0) {
                return false;
            }
            // unit-sphere space: closest point of the segment to the origin
            let ua: V3 = std::array::from_fn(|k| (a[k] - self.c[k]) / self.h[k]);
            let ub: V3 = std::array::from_fn(|k| (b[k] - self.c[k]) / self.h[k]);
            let dv: V3 = std::array::from_fn(|k| ub[k] - ua[k]);
            let dd = dot(dv, dv);
            let t = if dd > 0.0 { (-dot(ua, dv) / dd).clamp(0.0, 1.0) } else { 0.0 };
            let q: V3 = std::array::from_fn(|k| ua[k] + t * dv[k]);
            return dot(q, q) <= 1.0;
        }
        // slab clipping of t in [0, 1]
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for k in 0..3 {
            let (min, max) = (self.c[k] - self.h[k], self.c[k] + self.h[k]);
            if min > max {
                return false;
            }
            let dk = b[k] - a[k];
            if dk == 0.0 {
                if a[k] < min || a[k] > max {
                    return false;
                }
                continue;
            }
            let (t0, t1) = ((min - a[k]) / dk, (max - a[k]) / dk);
            let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
            lo = lo.max(t0);
            hi = hi.min(t1);
            if lo > hi {
                return false;
            }
        }
        true
    }
}

pub(super) type Edge = [u32; 2];
pub(super) fn ekey([a, b]: Edge) -> (u32, u32) {
    (a.min(b), a.max(b))
}
/// The mesh edges (module docs): ordered list + per-prim corner-order segments (degenerate ones kept as `None` so the
/// `NeM` index stays the corner index).
pub(super) struct EdgeTable {
    pub(super) list: Vec<Edge>,
    set: HashSet<(u32, u32)>,
    per_prim: Vec<Vec<Option<Edge>>>,
}
pub(super) fn edge_table(d: &Detail) -> EdgeTable {
    let mut t = EdgeTable { list: Vec::new(), set: HashSet::new(), per_prim: Vec::with_capacity(d.prims.len()) };
    for pr in &d.prims {
        let closed = match pr.kind {
            PrimKind::Poly { closed } => closed,
            PrimKind::Polyline => false,
            _ => {
                t.per_prim.push(Vec::new());
                continue;
            }
        };
        let pts: &[u32] = d.vertex_point.get(prim_verts(d, pr)).unwrap_or(&[]);
        let n = pts.len();
        let segs = if closed && n > 2 { n } else { n.saturating_sub(1) };
        let mut mine = Vec::with_capacity(segs);
        for k in 0..segs {
            let (a, b) = (pts.get(k).copied(), pts.get((k + 1) % n.max(1)).copied());
            let e = match (a, b) {
                (Some(a), Some(b)) if a != b => Some([a, b]),
                _ => None,
            };
            if let Some(e) = e
                && t.set.insert(ekey(e))
            {
                t.list.push(e);
            }
            mine.push(e);
        }
        t.per_prim.push(mine);
    }
    t
}
/// Ordered edge set: union appends, first orientation wins.
#[derive(Default)]
pub(super) struct EdgeRun {
    pub(super) list: Vec<Edge>,
    set: HashSet<(u32, u32)>,
}
impl EdgeRun {
    pub(super) fn push(&mut self, e: Edge) {
        if self.set.insert(ekey(e)) {
            self.list.push(e);
        }
    }
    pub(super) fn remove(&mut self, gone: &HashSet<(u32, u32)>) {
        self.list.retain(|&e| !gone.contains(&ekey(e)));
        self.set.retain(|k| !gone.contains(k));
    }
}
fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())
}
/// `N` / `N-M` (digits only) -> (lo, hi) unsorted; `None` = not that shape (or a u64 overflow).
fn id_range(s: &str) -> Option<(u64, u64)> {
    match s.split_once('-') {
        Some((a, b)) if digits(a) && digits(b) => Some((a.parse().ok()?, b.parse().ok()?)),
        None if digits(s) => {
            let v = s.parse().ok()?;
            Some((v, v))
        }
        _ => None,
    }
}
/// One edge token; `Err(())` = the whole pattern comes out empty (Houdini 20.5). `loose` (Edge Divide's parse) keeps
/// `pA-B` pairs that are no mesh edge (degenerate ones too) as long as both ids are existing points.
fn edge_token(d: &Detail, t: &EdgeTable, body: &str, loose: bool) -> Result<Vec<Edge>, ()> {
    let prims = t.per_prim.len() as u64;
    let prim_edges = |lo: u64, hi: u64| -> Vec<Edge> {
        let mut run = EdgeRun::default();
        for q in lo..=hi {
            for e in t.per_prim.get(usize::try_from(q).unwrap_or(usize::MAX)).into_iter().flatten().flatten() {
                run.push(*e);
            }
        }
        run.list
    };
    if body == "*" {
        return Ok(t.list.clone());
    }
    if body.starts_with(|c: char| c.is_ascii_digit()) {
        if let Some((pn, en)) = body.split_once('e')
            && digits(pn)
            && digits(en)
        {
            let e = pn.parse::<usize>().ok().and_then(|p| t.per_prim.get(p)).zip(en.parse::<usize>().ok());
            return Ok(e.and_then(|(edges, m)| edges.get(m).copied().flatten()).into_iter().collect());
        }
        if body.contains(['v', ':']) {
            return Err(());
        }
        return match id_range(body) {
            Some((lo, hi)) if lo > hi => Ok(Vec::new()),
            Some((_, hi)) if hi >= prims => Err(()),
            Some((lo, hi)) => Ok(prim_edges(lo, hi)),
            None => Ok(Vec::new()),
        };
    }
    if let Some(path) = body.strip_prefix('p')
        && path.starts_with(|c: char| c.is_ascii_digit())
    {
        let ids: Option<Vec<u32>> = path.split('-').map(|s| if digits(s) { s.parse().ok() } else { None }).collect();
        let Some(ids) = ids else { return Ok(Vec::new()) };
        return Ok(match ids.as_slice() {
            [one] => t.list.iter().copied().filter(|e| e.contains(one)).collect(),
            many if loose => {
                many.windows(2).map(|w| [w[0], w[1]]).filter(|e| e.iter().all(|&p| p < d.point_count)).collect()
            }
            many => {
                many.windows(2).map(|w| [w[0], w[1]]).filter(|&e| e[0] != e[1] && t.set.contains(&ekey(e))).collect()
            }
        });
    }
    // names: primitive groups (their prims' edges), then edge groups
    let mut run = EdgeRun::default();
    let mut in_prim = vec![false; t.per_prim.len()];
    for g in d.groups.iter().filter(|g| g.class == AttrClass::Prim && glob_match(body, &g.name)) {
        for (m, s) in in_prim.iter_mut().zip(&g.mask) {
            *m |= *s;
        }
    }
    for (q, _) in in_prim.iter().enumerate().filter(|(_, m)| **m) {
        for e in t.per_prim.get(q).into_iter().flatten().flatten() {
            run.push(*e);
        }
    }
    for g in d.edge_groups.iter().filter(|g| glob_match(body, &g.name)) {
        for &e in &g.edges {
            run.push(e);
        }
    }
    Ok(run.list)
}
/// Edge-group pattern (module docs); an empty pattern is handled by the caller.
fn eval_edge_pattern(d: &Detail, t: &EdgeTable, pat: &str) -> Vec<Edge> {
    eval_edge_pattern_with(d, t, pat, false)
}
/// [`eval_edge_pattern`], optionally `loose` (see [`edge_token`]); shared with `edgedivide`.
pub(super) fn eval_edge_pattern_with(d: &Detail, t: &EdgeTable, pat: &str, loose: bool) -> Vec<Edge> {
    let mut run = EdgeRun::default();
    for raw in pat.split_whitespace() {
        let (kind, body) = match raw.chars().next() {
            Some('!') => ('!', &raw[1..]),
            Some('^') => ('^', &raw[1..]),
            _ => (' ', raw),
        };
        if body.is_empty() {
            continue;
        }
        let Ok(set) = edge_token(d, t, body, loose) else { return Vec::new() };
        match kind {
            '!' => {
                let not: HashSet<(u32, u32)> = set.iter().map(|&e| ekey(e)).collect();
                for &e in t.list.iter().filter(|&&e| !not.contains(&ekey(e))) {
                    run.push(e);
                }
            }
            '^' => {
                let prim_edge = body.split_once('e').is_some_and(|(a, b)| digits(a) && digits(b));
                if !prim_edge {
                    run.remove(&set.iter().map(|&e| ekey(e)).collect());
                }
            }
            _ => set.into_iter().for_each(|e| run.push(e)),
        }
    }
    run.list
}
/// Vertex-group pattern (module docs) as a mask over the global vertices; `None` = empty pattern result.
fn eval_vertex_pattern(d: &Detail, pat: &str) -> Vec<bool> {
    fn mark_in(mask: &mut [bool], r: std::ops::Range<usize>) {
        let n = mask.len();
        for slot in mask.get_mut(r.start.min(n)..r.end.min(n)).into_iter().flatten() {
            *slot = true;
        }
    }
    let nv = d.vertex_point.len();
    let mut mask = vec![false; nv];
    let prim_range = |q: usize| d.prims.get(q).map_or(0..0, |pr| prim_verts(d, pr));
    for raw in pat.split_whitespace() {
        if raw.starts_with(['!', '^']) {
            if raw.len() > 1 {
                return vec![false; nv];
            }
            continue;
        }
        let mut mark = |r: std::ops::Range<usize>| mark_in(&mut mask, r);
        if raw == "*" {
            mark(0..nv);
        } else if raw.starts_with(|c: char| c.is_ascii_digit()) {
            if raw.contains([':', 'e']) {
                return vec![false; nv];
            }
            if let Some((pn, vr)) = raw.split_once('v') {
                let (Some((p0, p1)), Some((lo, hi))) = (id_range(pn), id_range(vr)) else { continue };
                if p0 != p1 || lo > hi {
                    continue;
                }
                let r = prim_range(usize::try_from(p0).unwrap_or(usize::MAX));
                let len = r.len() as u64;
                if lo < len {
                    let s = r.start + lo as usize;
                    mark(s..r.start + (hi.min(len - 1) as usize) + 1);
                }
            } else if let Some((lo, hi)) = id_range(raw)
                && lo <= hi
            {
                let last = (d.prims.len() as u64).min(hi.saturating_add(1));
                for q in lo..last {
                    mark(prim_range(q as usize));
                }
            }
        } else if raw.starts_with('p') && raw[1..].starts_with(|c: char| c.is_ascii_digit()) {
            return vec![false; nv];
        } else {
            let mut hit = false;
            for g in d.groups.iter().filter(|g| glob_match(raw, &g.name)) {
                match g.class {
                    AttrClass::Prim => {
                        hit = true;
                        for (q, _) in g.mask.iter().enumerate().filter(|(_, m)| **m) {
                            mark_in(&mut mask, prim_range(q));
                        }
                    }
                    AttrClass::Vertex => {
                        hit = true;
                        for (m, s) in mask.iter_mut().zip(&g.mask) {
                            *m |= *s;
                        }
                    }
                    _ => {}
                }
            }
            if !hit {
                return vec![false; nv];
            }
        }
    }
    mask
}

/// Edge-group cook (module docs).
#[allow(clippy::too_many_arguments)]
fn cook_edge_group(
    d: &Detail,
    name: &str,
    merge: u32,
    base: Option<&str>,
    criteria: bool,
    region: Option<(&Region, bool)>,
    normal: Option<(Option<V3>, f64, bool)>,
) -> Detail {
    let t = edge_table(d);
    let mut new: Vec<Edge> = match base {
        Some(pat) if !pat.trim().is_empty() => eval_edge_pattern(d, &t, pat),
        None if !criteria => Vec::new(),
        _ => t.list.clone(),
    };
    let pos = d.p();
    let at = |i: u32| pos.get(i as usize).copied().map(widen);
    if let Some((reg, whole)) = region {
        new.retain(|&[a, b]| match (at(a), at(b)) {
            (Some(pa), Some(pb)) if whole => reg.contains(pa) && reg.contains(pb),
            (Some(pa), Some(pb)) => reg.meets_segment(pa, pb),
            _ => false,
        });
    }
    if let Some((dir, max, opposite)) = normal {
        let pn = normals(d, AttrClass::Point);
        new.retain(|&[a, b]| {
            let (na, nb) = (pn.get(a as usize).copied().flatten(), pn.get(b as usize).copied().flatten());
            let nv = na.zip(nb).and_then(|(x, y)| unit([x[0] + y[0], x[1] + y[1], x[2] + y[2]]));
            match (nv, dir) {
                (Some(nv), Some(dv)) => {
                    let ang = dot(nv, dv).clamp(-1.0, 1.0).acos().to_degrees();
                    ang <= max || (opposite && 180.0 - ang <= max)
                }
                _ => false,
            }
        });
    }
    let old: Vec<Edge> = d.edge_group(name).map(|g| g.edges.clone()).unwrap_or_default();
    let new_set: HashSet<(u32, u32)> = new.iter().map(|&e| ekey(e)).collect();
    let out: Vec<Edge> = match merge {
        MERGE_UNION => old.into_iter().chain(new).collect(),
        MERGE_INTERSECT => old.into_iter().filter(|&e| new_set.contains(&ekey(e))).collect(),
        MERGE_SUBTRACT => old.into_iter().filter(|&e| !new_set.contains(&ekey(e))).collect(),
        _ => new,
    };
    let mut res = d.clone();
    res.set_edge_group(name, out);
    res
}

impl Node<Detail> for GroupCreate {
    fn type_name(&self) -> &'static str {
        OP
    }
    fn inputs(&self) -> &[InputSpec] {
        &[InputSpec { name: "input0" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p = ctx.params;
        let miss = |n: &str| bad(format!("`{n}` missing or wrong type"));
        let name = sanitize(p.string("groupname").ok_or_else(|| miss("groupname"))?);
        let gt = p.menu("grouptype").ok_or_else(|| miss("grouptype"))?;
        let class = match gt {
            GT_PRIM => AttrClass::Prim,
            GT_POINT => AttrClass::Point,
            GT_VERTEX | GT_EDGE => AttrClass::Vertex, // edges: class unused (own path below)
            m => return Err(bad(format!("grouptype menu index {m} out of range"))),
        };
        let merge = p.menu("mergeop").ok_or_else(|| miss("mergeop"))?;
        if merge > MERGE_SUBTRACT {
            return Err(bad(format!("mergeop menu index {merge} out of range")));
        }
        let base = p.toggle("groupbase").ok_or_else(|| miss("groupbase"))?;
        let basegroup = p.string("basegroup").ok_or_else(|| miss("basegroup"))?;
        let bounding = p.toggle("groupbounding").ok_or_else(|| miss("groupbounding"))?;
        let use_normal = p.toggle("groupnormal").ok_or_else(|| miss("groupnormal"))?;
        let boundtype = p.menu("boundtype").ok_or_else(|| miss("boundtype"))?;
        if bounding && boundtype != BOUND_BOX && boundtype != BOUND_SPHERE {
            return Err(CookError::Unsupported(format!(
                "{OP}: only the bounding box / sphere regions are modelled (object / volume / hull need a second input)"
            )));
        }
        let size = p.vec3("size").ok_or_else(|| miss("size"))?;
        let centre = p.vec3("t").ok_or_else(|| miss("t"))?;
        let whole = !p.toggle("includenotwhollycontained").ok_or_else(|| miss("includenotwhollycontained"))?;
        let dir = p.vec3("dir").ok_or_else(|| miss("dir"))?;
        let angle = p.float("angle").ok_or_else(|| miss("angle"))?;
        let opposite = p.toggle("oppositenormals").ok_or_else(|| miss("oppositenormals"))?;
        if size.iter().chain(&centre).chain(&dir).any(|c| !c.is_finite()) || !angle.is_finite() {
            return Err(bad("non-finite vector component"));
        }
        let empty = Arc::new(Detail::default());
        let d: &Arc<Detail> = inputs.first().unwrap_or(&empty);
        if name.is_empty() {
            return Ok(Arc::clone(d));
        }
        let region = Region { c: widen(centre), h: widen(size).map(|s| s * 0.5), sphere: boundtype == BOUND_SPHERE };
        if gt == GT_EDGE {
            let normal = use_normal.then(|| (unit(widen(dir)), f64::from(angle) + ANGLE_EPS, opposite));
            let base_pat = base.then_some(basegroup);
            let crit = bounding || use_normal;
            return Ok(Arc::new(cook_edge_group(
                d,
                &name,
                merge,
                base_pat,
                crit,
                bounding.then_some((&region, whole)),
                normal,
            )));
        }
        let n = d.class_len(class);
        let mut new = match (base, basegroup.trim().is_empty()) {
            (true, false) if gt == GT_VERTEX => eval_vertex_pattern(d, basegroup),
            (true, false) => eval_pattern(d, class, basegroup, OP)?,
            (false, _) if !bounding && !use_normal => vec![false; n],
            _ => vec![true; n],
        };
        if bounding {
            let pos = d.p();
            let inside = |i: usize| pos.get(i).copied().map(widen).is_some_and(|q| region.contains(q));
            for (m, pts) in new.iter_mut().zip(element_points(d, class)) {
                let hit = !pts.is_empty()
                    && if whole { pts.iter().all(|&i| inside(i)) } else { pts.iter().any(|&i| inside(i)) };
                *m = *m && hit;
            }
        }
        if use_normal && gt != GT_VERTEX {
            let dir: Option<V3> = unit(widen(dir));
            let max = f64::from(angle) + ANGLE_EPS;
            for (m, nv) in new.iter_mut().zip(normals(d, class)) {
                let ok = match (nv, dir) {
                    (Some(nv), Some(dv)) => {
                        let a = dot(nv, dv).clamp(-1.0, 1.0).acos().to_degrees();
                        a <= max || (opposite && 180.0 - a <= max)
                    }
                    _ => false,
                };
                *m = *m && ok;
            }
        }
        let old: Vec<bool> = match d.group(class, &name) {
            Some(g) => (0..n).map(|i| g.mask.get(i).copied().unwrap_or(false)).collect(),
            None => vec![false; n],
        };
        let out: Vec<bool> = new
            .iter()
            .zip(&old)
            .map(|(&nw, &ol)| match merge {
                MERGE_UNION => ol || nw,
                MERGE_INTERSECT => ol && nw,
                MERGE_SUBTRACT => ol && !nw,
                _ => nw,
            })
            .collect();
        let mut res = (**d).clone();
        set_group(&mut res, class, &name, out);
        Ok(Arc::new(res))
    }
}

#[cfg(test)]
mod tests {
    use super::super::group_sop::tt::*;
    use super::*;

    fn gc(ov: &[(&str, ParamValue)], d: &Arc<Detail>) -> Result<Arc<Detail>, CookError> {
        run(OP, ov, Some(d))
    }
    fn prim(d: &Detail) -> Vec<usize> {
        mask(d, AttrClass::Prim, "group1")
    }
    fn pt() -> ParamValue {
        ParamValue::Menu(GT_POINT)
    }
    fn on(k: &'static str) -> (&'static str, ParamValue) {
        (k, ParamValue::B(true))
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), OP);
        let names: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert!(names.starts_with(&["groupname", "grouptype", "mergeop", "groupbase", "basegroup"]));
        assert_eq!(
            n.params().iter().find(|s| s.name == "angle").map(|s| s.default.clone()),
            Some(ParamValue::F(180.0))
        );
    }
    /// hython: default / groupbase off / `0-2` / `!0-2` / `0 3-4` / `2-0` on the default box (prims and points).
    #[test]
    fn base_patterns_match_hython() {
        let b = cube();
        assert_eq!(prim(&gc(&[], &b).expect("cooks")), [0, 1, 2, 3, 4, 5]);
        assert!(prim(&gc(&[("groupbase", ParamValue::B(false))], &b).expect("cooks")).is_empty());
        let cases = [("0-2", vec![0, 1, 2]), ("!0-2", vec![3, 4, 5]), ("0 3-4", vec![0, 3, 4]), ("2-0", vec![0, 1, 2])];
        for (pat, want) in cases {
            assert_eq!(prim(&gc(&[("basegroup", s(pat))], &b).expect("cooks")), want, "{pat}");
        }
        let pts = gc(&[("grouptype", pt()), ("basegroup", s("!0-2"))], &b).expect("cooks");
        assert_eq!(mask(&pts, AttrClass::Point, "group1"), [3, 4, 5, 6, 7]);
    }
    /// hython: `!` is a UNIONED complement, `^` subtracts, steps, unknown names / out of range ids select nothing.
    #[test]
    fn pattern_corner_cases_match_hython() {
        let a = gc(&[("groupname", s("a")), ("basegroup", s("0-2"))], &cube()).expect("cooks");
        let sel = |pat: &str| {
            mask(&gc(&[("groupname", s("c")), ("basegroup", s(pat))], &a).expect("cooks"), AttrClass::Prim, "c")
        };
        assert_eq!(sel("0-2 !1"), [0, 1, 2, 3, 4, 5]);
        assert_eq!(sel("a !1"), [0, 1, 2, 3, 4, 5]);
        assert_eq!(sel("!a"), [3, 4, 5]);
        assert_eq!(sel("a 5"), [0, 1, 2, 5]);
        assert_eq!(sel("a ^1"), [0, 2]);
        assert_eq!(sel("0-5:2"), [0, 2, 4]);
        assert_eq!(sel("3-1 a"), [0, 1, 2, 3]);
        assert_eq!(sel("a nope"), [0, 1, 2]);
        assert!(sel("nope").is_empty() && sel("9").is_empty() && sel("-1").is_empty());
        assert_eq!(sel("0-100"), [0, 1, 2, 3, 4, 5]);
        assert_eq!(sel("a*"), [0, 1, 2]);
        assert!(matches!(gc(&[("basegroup", s("1,3"))], &a), Err(CookError::Unsupported(_))));
    }
    /// hython merge ops against an existing group `a` = 0-2 with a new set 1-4.
    #[test]
    fn mergeop_matches_hython() {
        let a = gc(&[("groupname", s("a")), ("basegroup", s("0-2"))], &cube()).expect("cooks");
        for (op, want) in [(0, vec![1, 2, 3, 4]), (1, vec![0, 1, 2, 3, 4]), (2, vec![1, 2]), (3, vec![0])] {
            let ov = [("groupname", s("a")), ("basegroup", s("1-4")), ("mergeop", ParamValue::Menu(op))];
            assert_eq!(mask(&gc(&ov, &a).expect("cooks"), AttrClass::Prim, "a"), want, "mergeop {op}");
        }
    }
    /// hython: box bbox (size 1.2/2/1.2 at x=.5): wholly -> prim 1, not-wholly -> 0 1 2 4 5, points 1 2 5 6; boundary inclusive.
    #[test]
    fn bounding_box_matches_hython() {
        let b = cube();
        let bb = |extra: &[(&'static str, ParamValue)]| -> Vec<(&'static str, ParamValue)> {
            let mut v = vec![
                on("groupbounding"),
                ("size", ParamValue::V3([1.2, 2.0, 1.2])),
                ("t", ParamValue::V3([0.5, 0.0, 0.0])),
            ];
            v.extend(extra.iter().cloned());
            v
        };
        let pts = |ov: &[(&str, ParamValue)]| mask(&gc(ov, &b).expect("cooks"), AttrClass::Point, "group1");
        assert_eq!(prim(&gc(&bb(&[]), &b).expect("cooks")), [1]);
        assert_eq!(prim(&gc(&bb(&[on("includenotwhollycontained")]), &b).expect("cooks")), [0, 1, 2, 4, 5]);
        assert_eq!(pts(&bb(&[("grouptype", pt())])), [1, 2, 5, 6]);
        assert_eq!(pts(&[on("groupbounding"), ("grouptype", pt())]).len(), 8, "boundary is inclusive");
        let shy = [on("groupbounding"), ("size", ParamValue::V3([0.9999, 1.0, 1.0])), ("grouptype", pt())];
        assert!(pts(&shy).is_empty());
        let neg = [on("groupbounding"), ("size", ParamValue::V3([-1.0, 1.0, 1.0])), ("grouptype", pt())];
        assert!(pts(&neg).is_empty());
        assert_eq!(prim(&gc(&bb(&[("groupbase", ParamValue::B(false))]), &b).expect("cooks")), [1]);
        let sphere = bb(&[("boundtype", ParamValue::Menu(BOUND_SPHERE))]);
        assert_eq!(prim(&gc(&sphere, &b).expect("cooks")), [1]); // hython: the inscribed ellipsoid keeps prim 1 too
        for other in [2, 3, 4] {
            let obj = bb(&[("boundtype", ParamValue::Menu(other))]);
            assert!(matches!(gc(&obj, &b), Err(CookError::Unsupported(_))), "boundtype {other}");
        }
    }
    /// hython: normals on the box (dir z default angle 180 = all; y10 -> prim 5; y90 -> 0 1 2 3 5; opposite 45 -> 4 5; dir 0 -> none).
    #[test]
    fn normals_match_hython() {
        let b = cube();
        let y = |angle: f32, extra: &[(&'static str, ParamValue)]| {
            let mut v =
                vec![on("groupnormal"), ("dir", ParamValue::V3([0.0, 1.0, 0.0])), ("angle", ParamValue::F(angle))];
            v.extend(extra.iter().cloned());
            v
        };
        assert_eq!(prim(&gc(&[on("groupnormal")], &b).expect("cooks")), [0, 1, 2, 3, 4, 5]);
        assert_eq!(prim(&gc(&y(10.0, &[]), &b).expect("cooks")), [5]);
        assert_eq!(prim(&gc(&y(90.0, &[]), &b).expect("cooks")), [0, 1, 2, 3, 5]);
        assert_eq!(prim(&gc(&y(45.0, &[on("oppositenormals")]), &b).expect("cooks")), [4, 5]);
        assert_eq!(prim(&gc(&y(180.0, &[]), &b).expect("cooks")), [0, 1, 2, 3, 4, 5]);
        let z = [on("groupnormal"), ("dir", ParamValue::V3([0.0; 3])), ("angle", ParamValue::F(10.0))];
        assert!(prim(&gc(&z, &b).expect("cooks")).is_empty());
        let p90 = gc(&y(90.0, &[("grouptype", pt())]), &b).expect("cooks");
        assert_eq!(mask(&p90, AttrClass::Point, "group1"), [4, 5, 6, 7]);
        let p10 = gc(&y(10.0, &[("grouptype", pt())]), &b).expect("cooks");
        assert!(mask(&p10, AttrClass::Point, "group1").is_empty());
    }
    #[test]
    fn names_and_unsupported_and_hostile() {
        let b = cube();
        let o = gc(&[("groupname", s("x y"))], &b).expect("cooks");
        assert!(o.group(AttrClass::Prim, "x_y").is_some());
        let o = gc(&[("groupname", s("a*"))], &b).expect("cooks");
        assert!(o.group(AttrClass::Prim, "a_").is_some());
        assert!(Arc::ptr_eq(&gc(&[("groupname", s(""))], &b).expect("cooks"), &b));
        for gt in [GT_EDGE, GT_VERTEX] {
            assert!(
                gc(&[("grouptype", ParamValue::Menu(gt)), ("groupname", s(""))], &b).is_ok_and(|o| Arc::ptr_eq(&o, &b))
            );
        }
        assert!(matches!(gc(&[("grouptype", ParamValue::Menu(9))], &b), Err(CookError::BadParam(_))));
        assert!(matches!(gc(&[("angle", ParamValue::F(f32::NAN))], &b), Err(CookError::BadParam(_))));
        assert!(matches!(gc(&[("groupname", ParamValue::I(1))], &b), Err(CookError::BadParam(_))));
        assert!(run(OP, &[], None).is_ok());
        for pat in ["!", "^", "::", "1:0", "9-1:3", "\u{0}"] {
            assert!(gc(&[("basegroup", s(pat))], &b).is_ok(), "{pat:?}");
        }
        let huge = gc(&[("basegroup", s("0-99999999999999999999"))], &b);
        assert!(matches!(huge, Err(CookError::BadParam(_))), "u64 overflow is an error, not a wrap");
    }

    // ---- edge / vertex groups + bounding sphere vs Houdini 20.5 (fixtures/sop-groupcreate-edgevtx, made by
    // oracle/houdini/parmgaps/groupcreate_edge_vertex.py from synthetic inputs) ----------------------------------------
    fn fx(file: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sop-groupcreate-edgevtx").join(file)
    }
    fn load(file: &str) -> Arc<Detail> {
        Arc::new(hexe_geo::read_geo(&std::fs::read(fx(file)).expect("fixture")).expect("parses"))
    }
    fn json_param(spec: &ParamSpec, v: &serde_json::Value) -> Option<ParamValue> {
        let f = |x: &serde_json::Value| x.as_f64().map(|x| x as f32);
        Some(match &spec.default {
            ParamValue::S(_) => ParamValue::S(v.as_str()?.into()),
            ParamValue::B(_) => ParamValue::B(v.as_bool()?),
            ParamValue::F(_) => ParamValue::F(f(v)?),
            ParamValue::I(_) => ParamValue::I(i32::try_from(v.as_i64()?).ok()?),
            ParamValue::Menu(_) => ParamValue::Menu(u32::try_from(v.as_u64()?).ok()?),
            ParamValue::V3(_) => {
                let a = v.as_array()?;
                ParamValue::V3([f(a.first()?)?, f(a.get(1)?)?, f(a.get(2)?)?])
            }
        })
    }
    fn members(d: &Detail, gt: u32, name: &str) -> serde_json::Value {
        let ids = |c: AttrClass| d.group(c, name).map_or_else(Vec::new, |_| mask(d, c, name));
        match gt {
            GT_EDGE => serde_json::json!(d.edge_group(name).map(|g| g.edges.clone()).unwrap_or_default()),
            GT_VERTEX => serde_json::json!(ids(AttrClass::Vertex)),
            GT_POINT => serde_json::json!(ids(AttrClass::Point)),
            _ => serde_json::json!(ids(AttrClass::Prim)),
        }
    }
    /// 388 hython cases: edge + vertex patterns (box / grid / pre-grouped box), merge ops, bounding box + sphere (whole and
    /// partial, box / grid / ellipsoid), normals (box / ellipsoid / torus), plus bounding sphere on points and prims.
    /// Edge results compare as ORDERED point-pair lists.
    #[test]
    fn edge_vertex_and_bsphere_cases_match_houdini() {
        let text = std::fs::read_to_string(fx("cases.json")).expect("cases.json");
        let cases: Vec<serde_json::Value> = serde_json::from_str(&text).expect("json");
        assert!(cases.len() > 300, "{}", cases.len());
        let specs = spec();
        let mut inputs: std::collections::HashMap<String, Arc<Detail>> = std::collections::HashMap::new();
        let mut fails = Vec::new();
        for c in &cases {
            let src = c["input"].as_str().expect("input").to_string();
            let d = Arc::clone(inputs.entry(src.clone()).or_insert_with(|| load(&format!("{src}_in.geo"))));
            let params = c["params"].as_object().expect("params");
            let ov: Vec<(&str, ParamValue)> = params
                .iter()
                .filter_map(|(k, v)| {
                    let sp = specs.iter().find(|s| s.name == *k)?; // Houdini-only parms (groupedges, unshared) skipped
                    Some((sp.name.as_str(), json_param(sp, v).expect("param converts")))
                })
                .collect();
            let gt = params["grouptype"].as_u64().and_then(|g| u32::try_from(g).ok()).expect("grouptype");
            let name = params["groupname"].as_str().expect("groupname");
            let got = members(&gc(&ov, &d).expect("cooks"), gt, name);
            if got != c["result"] {
                fails.push(format!("{src} {:?}: hexe {got} != houdini {}", c["params"], c["result"]));
            }
        }
        assert!(fails.is_empty(), "{} of {} cases differ:\n{}", fails.len(), cases.len(), fails.join("\n"));
    }
    /// Whole files: an edge-group union (existing `ea` + `1 p4-5`, order kept) and a vertex group, every other group of the
    /// input untouched; hexe's writer round-trips both (edge order included).
    #[test]
    fn edge_and_vertex_group_files_match_houdini() {
        let b = load("boxpre_in.geo");
        let ov = [("groupname", s("ea")), ("grouptype", ParamValue::Menu(GT_EDGE)), ("basegroup", s("1 p4-5"))];
        let e = gc(&[ov[0].clone(), ov[1].clone(), ov[2].clone(), ("mergeop", ParamValue::Menu(MERGE_UNION))], &b)
            .expect("cooks");
        let want = load("boxpre_edge_union_houdini.geo");
        assert_eq!(e.edge_groups, want.edge_groups);
        assert_eq!(
            e.edge_group("ea").map(|g| g.edges.clone()),
            Some(vec![[0, 1], [1, 2], [2, 6], [6, 5], [5, 1], [4, 5]])
        );
        let v = gc(
            &[("groupname", s("vg")), ("grouptype", ParamValue::Menu(GT_VERTEX)), ("basegroup", s("1 3v2 5v0-1"))],
            &b,
        )
        .expect("cooks");
        let want = load("boxpre_vertex_houdini.geo");
        // same groups; Houdini's file lists the new vertex group before `va` (its own order), so compare by name
        let by_name = |d: &Detail| {
            let mut g: Vec<(String, Vec<bool>)> =
                d.groups.iter().map(|g| (format!("{:?}:{}", g.class, g.name), g.mask.clone())).collect();
            g.sort();
            g
        };
        assert_eq!(by_name(&v), by_name(&want));
        assert_eq!(mask(&v, AttrClass::Vertex, "vg"), [4, 5, 6, 7, 14, 20, 21]);
        for out in [&e, &v] {
            let back = hexe_geo::read_geo(&hexe_geo::write_geo_json(out)).expect("round trip");
            assert_eq!((&back.groups, &back.edge_groups), (&out.groups, &out.edge_groups));
        }
    }
    #[test]
    fn edge_and_vertex_hostile_input_never_panics() {
        let b = cube();
        let pats = [
            "p",
            "p-",
            "p--",
            "p0-",
            "p99999999999999999999",
            "p4294967295-0",
            "0e",
            "e0",
            "0e99999999999999999999",
            "0v",
            "v0",
            "0v99999999999999999999-0",
            "99999999999999999999",
            "0-99999999999999999999",
            "!",
            "^",
            "!!0",
            "^^0",
            "*v*",
            "**",
            "\u{0}",
            "p0-1-2-3-4-5-6-7-0-1-2",
            "0v0-99999999999",
            ":",
            "-",
            "0--1",
        ];
        for gt in [GT_EDGE, GT_VERTEX] {
            for pat in pats {
                let ov = [("grouptype", ParamValue::Menu(gt)), ("basegroup", s(pat))];
                assert!(gc(&ov, &b).is_ok(), "grouptype {gt} pattern {pat:?}");
            }
            // empty input, degenerate bounding / normal, NaN rejected
            assert!(run(OP, &[("grouptype", ParamValue::Menu(gt))], None).is_ok());
            for bt in [BOUND_BOX, BOUND_SPHERE] {
                let ov = [
                    ("grouptype", ParamValue::Menu(gt)),
                    on("groupbounding"),
                    ("boundtype", ParamValue::Menu(bt)),
                    ("size", ParamValue::V3([0.0, -1.0, 1e30])),
                    on("includenotwhollycontained"),
                ];
                assert!(gc(&ov, &b).is_ok_and(|o| members(&o, gt, "group1") == serde_json::json!([])), "{gt} {bt}");
            }
            let nan = [("grouptype", ParamValue::Menu(gt)), ("t", ParamValue::V3([f32::NAN, 0.0, 0.0]))];
            assert!(matches!(gc(&nan, &b), Err(CookError::BadParam(_))));
        }
        // a Detail whose prim ranges point past the vertex list: no panic
        let mut broken = (*b).clone();
        broken.vertex_point.truncate(5);
        let broken = Arc::new(broken);
        for gt in [GT_EDGE, GT_VERTEX] {
            for pat in ["", "*", "0-5", "5", "p0-1", "0e3", "5v3", "a"] {
                let ov = [("grouptype", ParamValue::Menu(gt)), ("basegroup", s(pat)), on("groupnormal")];
                assert!(gc(&ov, &broken).is_ok(), "{gt} {pat:?}");
            }
        }
    }
}
