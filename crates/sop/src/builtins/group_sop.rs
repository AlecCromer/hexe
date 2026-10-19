//! Group SOP (token `group`) + the element-selection helpers shared by `groupdelete`, `grouppromote`, `blast`.
//!
//! # Semantics (UNVERIFIED vs Houdini: hython blocked; analytical oracle only)
//! Creates / edits ONE group `groupname` of class `grouptype` (point | prim | vertex; edge -> `Unsupported`).
//! Criteria are AND-ed: `pattern` (empty = no criterion), `bbox` (on), `normal` (on). No criterion on = every element.
//! * `pattern`: whitespace tokens, left to right. `*` all; `N`; `N-M` (N <= M, else `BadParam`); anything else = group name of
//!   the same class (`*`/`?` glob; a plain unknown name -> `Failed`). A `!` prefix subtracts the token's set; if the FIRST
//!   token is negated the running set starts as everything (`!0-2` = all but 0..=2; `0-5 !2` = 0,1,3,4,5). Ids >= count are
//!   ignored. Vertex ids are GLOBAL vertex indices (no `prim:vertex` syntax yet).
//! * `bbox`: element position (point = P, vertex = its point, prim = mean of its points) inside the box
//!   `bbox_center +- |bbox_size| / 2`, faces INCLUSIVE. Flat input (grid plane) works: only the extent on each axis matters.
//! * `normal`: keeps elements whose normal makes an angle <= `angle` degrees with `dir` (INCLUSIVE, 1e-6 deg slack:
//!   angle 90 keeps strictly-perpendicular side faces, angle 180 keeps all). Prim normal = Newell (unit); vertex = its
//!   prim's; point = normalised sum of the unit normals of every vertex's prim. No normal (degenerate / non-poly / unused
//!   point) -> excluded. Zero `dir` or negative `angle` -> `BadParam`.
//! * `merge_op` vs an existing group of that class+name: replace | union | intersect (existing & new) | subtract (existing & !new).
//!   Missing existing group = empty. The group is always created (possibly empty).
//!
//! Defaults `grouptype=prim`, `dir=(0,1,0)`, `angle=45`, `bbox_size=(1,1,1)` are guesses (UNVERIFIED).
use super::meshutil::newell_corners;
use crate::SopNode;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, Group, Prim};
use hexe_graph::{CookCtx, CookError, InputSpec, Node, ParamSet, ParamSpec, ParamValue};
use std::sync::Arc;

pub(super) const GT_POINT: u32 = 0;
pub(super) const GT_PRIM: u32 = 1;
pub(super) const GT_VERTEX: u32 = 2;
const GT_EDGE: u32 = 3;
const MERGE_REPLACE: u32 = 0;
const MERGE_UNION: u32 = 1;
const MERGE_INTERSECT: u32 = 2;
const MERGE_SUBTRACT: u32 = 3;
/// Degrees of slack on the (inclusive) normal-angle boundary.
pub(super) const ANGLE_EPS: f64 = 1e-6;
const MIN_LEN: f64 = 1e-12;

pub fn spec() -> Vec<ParamSpec> {
    vec![
        ParamSpec::new("groupname", ParamValue::S("group1".into())).label("Group Name"),
        ParamSpec::new("grouptype", ParamValue::Menu(GT_PRIM)).label("Group Type").menu(&[
            ("point", "Points"),
            ("prim", "Primitives"),
            ("vertex", "Vertices"),
            ("edge", "Edges"),
        ]),
        // Number tab: `groupnumber` off = the pattern is ignored (Houdini; a .hip keeps a stale pattern there)
        ParamSpec::new("groupnumber", ParamValue::B(true)).label("Enable Number"),
        ParamSpec::new("pattern", ParamValue::S(String::new())).label("Pattern"),
        ParamSpec::new("bbox", ParamValue::B(false)).label("Keep in Bounding Box"),
        ParamSpec::new("bbox_size", ParamValue::V3([1.0, 1.0, 1.0])).label("Size").range(0.0, 10.0),
        ParamSpec::new("bbox_center", ParamValue::V3([0.0, 0.0, 0.0])).label("Center").range(-10.0, 10.0),
        ParamSpec::new("normal", ParamValue::B(false)).label("Keep by Normals"),
        ParamSpec::new("dir", ParamValue::V3([0.0, 1.0, 0.0])).label("Direction").range(-1.0, 1.0),
        ParamSpec::new("angle", ParamValue::F(45.0)).label("Spread Angle").range(0.0, 180.0),
        ParamSpec::new("merge_op", ParamValue::Menu(MERGE_REPLACE)).label("Merge Operation").menu(&[
            ("replace", "Replace Existing"),
            ("union", "Union with Existing"),
            ("intersect", "Intersect with Existing"),
            ("subtract", "Subtract from Existing"),
        ]),
        // Edges tab (edge groups only; Houdini parm names so .hip values import as-is)
        ParamSpec::new("groupedges", ParamValue::B(false)).label("Enable Edges"),
        ParamSpec::new("dominedgeangle", ParamValue::B(false)).label("Use Min Edge Angle"),
        ParamSpec::new("minedgeangle", ParamValue::F(20.0)).label("Min Edge Angle").range(0.0, 180.0),
        ParamSpec::new("domaxedgeangle", ParamValue::B(false)).label("Use Max Edge Angle"),
        ParamSpec::new("maxedgeangle", ParamValue::F(20.0)).label("Max Edge Angle").range(0.0, 180.0),
        ParamSpec::new("edgeanglebetweenedges", ParamValue::B(false)).label("Edge Angle Uses Angle Between Edges"),
        ParamSpec::new("dominedgelen", ParamValue::B(false)).label("Use Min Edge Length"),
        ParamSpec::new("minedgelen", ParamValue::F(0.0)).label("Min Edge Length").range(0.0, 10.0),
        ParamSpec::new("domaxedgelen", ParamValue::B(false)).label("Use Max Edge Length"),
        ParamSpec::new("maxedgelen", ParamValue::F(0.0)).label("Max Edge Length").range(0.0, 10.0),
        ParamSpec::new("unshared", ParamValue::B(false)).label("Unshared Edges"),
    ]
}
pub fn new() -> Box<SopNode> {
    Box::new(GroupSop { specs: spec() })
}
struct GroupSop {
    specs: Vec<ParamSpec>,
}

pub(super) fn bad(who: &str, m: impl AsRef<str>) -> CookError {
    CookError::BadParam(format!("{who}: {}", m.as_ref()))
}
pub(super) fn failed(who: &str, m: impl AsRef<str>) -> CookError {
    CookError::Failed(format!("{who}: {}", m.as_ref()))
}
pub(super) type V3 = [f64; 3];
pub(super) fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(super) fn widen(q: [f32; 3]) -> V3 {
    [f64::from(q[0]), f64::from(q[1]), f64::from(q[2])]
}
/// `*`/`?` glob, iterative (no recursion, no backtracking blow-up).
pub(super) fn glob_match(pat: &str, s: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pat.chars().collect(), s.chars().collect());
    let (mut i, mut j, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while j < t.len() {
        match p.get(i) {
            Some('*') => {
                star = Some(i);
                mark = j;
                i += 1;
            }
            Some(c) if *c == '?' || Some(c) == t.get(j) => {
                i += 1;
                j += 1;
            }
            _ => match star {
                Some(s) => {
                    mark += 1;
                    j = mark;
                    i = s + 1;
                }
                None => return false,
            },
        }
    }
    p.get(i..).is_some_and(|rest| rest.iter().all(|c| *c == '*'))
}
/// Vertex ids of a prim (empty on a bad range).
pub(super) fn prim_verts(d: &Detail, p: &Prim) -> std::ops::Range<usize> {
    let (lo, hi) = (usize::try_from(p.verts.start).unwrap_or(0), usize::try_from(p.verts.end).unwrap_or(0));
    if lo <= hi && hi <= d.vertex_point.len() { lo..hi } else { 0..0 }
}
pub(super) fn token_set(d: &Detail, class: AttrClass, body: &str, n: usize, who: &str) -> Result<Vec<bool>, CookError> {
    if body == "*" {
        return Ok(vec![true; n]);
    }
    let numeric =
        body.chars().all(|c| c.is_ascii_digit() || c == '-') && body.starts_with(|c: char| c.is_ascii_digit());
    if numeric {
        let (lo, hi) = match body.split_once('-') {
            None => (body, body),
            Some((a, b)) => (a, b),
        };
        let num = |s: &str| s.parse::<u64>().map_err(|_| bad(who, format!("bad range `{body}`")));
        let (lo, hi) = (num(lo)?, num(hi)?);
        if lo > hi {
            return Err(bad(who, format!("reversed range `{body}`")));
        }
        let mut set = vec![false; n];
        let lo = usize::try_from(lo).unwrap_or(usize::MAX);
        let hi = usize::try_from(hi).unwrap_or(usize::MAX);
        for slot in set.iter_mut().take(hi.saturating_add(1)).skip(lo) {
            *slot = true;
        }
        return Ok(set);
    }
    let wild = body.contains(['*', '?']);
    let mut set = vec![false; n];
    let mut hit = false;
    for g in d.groups.iter().filter(|g| g.class == class && glob_match(body, &g.name)) {
        hit = true;
        for (s, m) in set.iter_mut().zip(&g.mask) {
            *s |= *m;
        }
    }
    if !hit && !wild {
        return Err(failed(who, format!("unknown group `{body}`")));
    }
    Ok(set)
}
/// Selection by `pattern` over `class`; `None` = empty pattern (no criterion). See module docs for the grammar.
pub(super) fn parse_pattern(
    d: &Detail,
    class: AttrClass,
    pat: &str,
    who: &str,
) -> Result<Option<Vec<bool>>, CookError> {
    let toks: Vec<&str> = pat.split_whitespace().collect();
    if toks.is_empty() {
        return Ok(None);
    }
    let n = d.class_len(class);
    let mut mask = vec![false; n];
    for (i, raw) in toks.iter().enumerate() {
        let (neg, body) = match raw.strip_prefix('!') {
            Some(b) => (true, b),
            None => (false, *raw),
        };
        if body.is_empty() {
            return Err(bad(who, "empty token after `!`"));
        }
        if neg && i == 0 {
            mask.fill(true);
        }
        let set = token_set(d, class, body, n, who)?;
        for (m, s) in mask.iter_mut().zip(&set) {
            if neg {
                if *s {
                    *m = false;
                }
            } else {
                *m |= *s;
            }
        }
    }
    Ok(Some(mask))
}
fn rank(c: AttrClass) -> u8 {
    match c {
        AttrClass::Point => 0,
        AttrClass::Vertex => 1,
        AttrClass::Prim => 2,
        AttrClass::Detail => 3,
    }
}
/// Replace (in place) or add the group, keeping groups ordered by class (stable).
pub(super) fn set_group(d: &mut Detail, class: AttrClass, name: &str, mask: Vec<bool>) {
    if let Some(g) = d.groups.iter_mut().find(|g| g.class == class && g.name == name) {
        g.mask = mask;
    } else {
        d.groups.push(Group { name: name.to_string(), class, mask });
        d.groups.sort_by_key(|g| rank(g.class));
    }
}
/// `idx[k]`-th element of every entry of the attribute data; out-of-range -> default element. Total.
fn gather(d: &AttrData, idx: &[usize]) -> AttrData {
    fn pick<T: Copy>(v: &[T], idx: &[usize], def: T) -> Vec<T> {
        idx.iter().map(|&i| v.get(i).copied().unwrap_or(def)).collect()
    }
    fn pick_n<T: Copy>(v: &[T], size: u32, idx: &[usize], def: T) -> Vec<T> {
        let s = usize::try_from(size).unwrap_or(0);
        let mut o = Vec::with_capacity(idx.len().saturating_mul(s));
        for &i in idx {
            let chunk = i.checked_mul(s).and_then(|a| v.get(a..a.checked_add(s)?));
            match chunk {
                Some(c) => o.extend_from_slice(c),
                None => o.extend(std::iter::repeat_n(def, s)),
            }
        }
        o
    }
    match d {
        AttrData::F32(v) => AttrData::F32(pick(v, idx, 0.0)),
        AttrData::I32(v) => AttrData::I32(pick(v, idx, 0)),
        AttrData::V2(v) => AttrData::V2(pick(v, idx, [0.0; 2])),
        AttrData::V3(v) => AttrData::V3(pick(v, idx, [0.0; 3])),
        AttrData::V4(v) => AttrData::V4(pick(v, idx, [0.0; 4])),
        AttrData::Str { idx: si, table } => AttrData::Str { idx: pick(si, idx, -1), table: table.clone() },
        AttrData::FN { size, data } => AttrData::FN { size: *size, data: pick_n(data, *size, idx, 0.0) },
        AttrData::IN { size, data } => AttrData::IN { size: *size, data: pick_n(data, *size, idx, 0) },
    }
}
/// Keep the flagged points and prims (a kept prim keeps all its vertices; its points MUST be kept). Order is preserved,
/// attributes and groups follow. Detail attributes and skipped-attribute records are copied.
pub(super) fn compact(src: &Detail, keep_pt: &[bool], keep_prim: &[bool], who: &str) -> Result<Detail, CookError> {
    let ids = |m: &[bool]| -> Vec<usize> { m.iter().enumerate().filter(|(_, k)| **k).map(|(i, _)| i).collect() };
    let pidx = ids(keep_pt);
    let mut pmap: Vec<Option<u32>> = vec![None; src.point_count as usize];
    for (new, &old) in pidx.iter().enumerate() {
        if let Some(s) = pmap.get_mut(old) {
            *s = u32::try_from(new).ok();
        }
    }
    let (mut vidx, mut prims, mut prim_idx, mut vertex_point) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, pr) in src.prims.iter().enumerate() {
        if !keep_prim.get(i).copied().unwrap_or(false) {
            continue;
        }
        let start = u32::try_from(vertex_point.len()).map_err(|_| failed(who, "vertex id overflow"))?;
        for v in prim_verts(src, pr) {
            let old = src.vertex_point.get(v).copied().ok_or_else(|| failed(who, "vertex out of range"))?;
            let new = pmap
                .get(old as usize)
                .copied()
                .flatten()
                .ok_or_else(|| failed(who, "kept primitive references a removed point"))?;
            vertex_point.push(new);
            vidx.push(v);
        }
        let end = u32::try_from(vertex_point.len()).map_err(|_| failed(who, "vertex id overflow"))?;
        prims.push(Prim { kind: pr.kind.clone(), verts: start..end });
        prim_idx.push(i);
    }
    let pick = |c: AttrClass| -> &[usize] {
        match c {
            AttrClass::Point => &pidx,
            AttrClass::Vertex => &vidx,
            AttrClass::Prim => &prim_idx,
            AttrClass::Detail => &[],
        }
    };
    let attrs = src
        .attrs
        .iter()
        .map(|a| Attribute {
            name: a.name.clone(),
            class: a.class,
            data: if a.class == AttrClass::Detail { a.data.clone() } else { gather(&a.data, pick(a.class)) },
            type_info: a.type_info,
        })
        .collect();
    let groups = src
        .groups
        .iter()
        .map(|g| Group {
            name: g.name.clone(),
            class: g.class,
            mask: pick(g.class).iter().map(|&i| g.mask.get(i).copied().unwrap_or(false)).collect(),
        })
        .collect();
    let mut out = Detail {
        point_count: u32::try_from(pidx.len()).map_err(|_| failed(who, "point id overflow"))?,
        vertex_point,
        prims,
        attrs,
        groups,
        edge_groups: Vec::new(),
        skipped_attribs: src.skipped_attribs.clone(),
    };
    let lists: Vec<Vec<usize>> = pidx.iter().map(|&i| vec![i]).collect();
    super::meshutil::follow_edge_groups(src, &super::meshutil::inverse_lists(src.point_count, &lists), &mut out);
    Ok(out)
}

/// Unit Houdini normal of a prim (`meshutil::newell_corners`); `None` for non-polygons / < 3 corners / zero or non-finite area / bad ids.
fn prim_normal(d: &Detail, pr: &Prim) -> Option<V3> {
    if !matches!(pr.kind, hexe_geo::PrimKind::Poly { .. }) {
        return None;
    }
    let r = prim_verts(d, pr);
    let p = d.p();
    let pos: Vec<V3> = d
        .vertex_point
        .get(r)?
        .iter()
        .map(|&i| p.get(usize::try_from(i).ok()?).copied().map(widen))
        .collect::<Option<_>>()?;
    if pos.len() < 3 || pos.iter().flatten().any(|c| !c.is_finite()) {
        return None;
    }
    let nw = newell_corners(&pos);
    unit(nw)
}
pub(super) fn unit(a: V3) -> Option<V3> {
    let l = dot(a, a).sqrt();
    (l.is_finite() && l > MIN_LEN).then(|| [a[0] / l, a[1] / l, a[2] / l])
}
/// Element normals for `class` (`None` = no normal).
pub(super) fn normals(d: &Detail, class: AttrClass) -> Vec<Option<V3>> {
    let pn: Vec<Option<V3>> = d.prims.iter().map(|p| prim_normal(d, p)).collect();
    match class {
        AttrClass::Prim => pn,
        AttrClass::Vertex => {
            let mut out = vec![None; d.vertex_point.len()];
            for (pr, n) in d.prims.iter().zip(&pn) {
                for v in prim_verts(d, pr) {
                    if let Some(s) = out.get_mut(v) {
                        *s = *n;
                    }
                }
            }
            out
        }
        _ => {
            let mut sum = vec![[0.0f64; 3]; d.point_count as usize];
            for (pr, n) in d.prims.iter().zip(&pn) {
                let Some(n) = n else { continue };
                for v in prim_verts(d, pr) {
                    let slot = d.vertex_point.get(v).and_then(|&p| sum.get_mut(p as usize));
                    if let Some(s) = slot {
                        *s = [s[0] + n[0], s[1] + n[1], s[2] + n[2]];
                    }
                }
            }
            sum.into_iter().map(unit).collect()
        }
    }
}
/// Position used by the bbox criterion (`None` = unplaceable, excluded).
fn positions(d: &Detail, class: AttrClass) -> Vec<Option<V3>> {
    let p = d.p();
    let at = |i: u32| p.get(i as usize).copied().map(widen);
    match class {
        AttrClass::Point => (0..d.point_count).map(at).collect(),
        AttrClass::Vertex => d.vertex_point.iter().map(|&i| at(i)).collect(),
        _ => d
            .prims
            .iter()
            .map(|pr| {
                let pts: Vec<V3> =
                    d.vertex_point.get(prim_verts(d, pr))?.iter().map(|&i| at(i)).collect::<Option<_>>()?;
                let k = pts.len() as f64;
                (k > 0.0).then(|| {
                    let s = pts.iter().fold([0.0; 3], |a, q| [a[0] + q[0], a[1] + q[1], a[2] + q[2]]);
                    [s[0] / k, s[1] / k, s[2] / k]
                })
            })
            .collect(),
    }
}

fn param_err(n: &str, what: &str) -> CookError {
    bad("group", format!("`{n}`: {what}"))
}
/// One polygon edge of the source: first-seen orientation, the prims using it, its length.
struct MeshEdge {
    ends: [u32; 2],
    prims: Vec<usize>,
    len: f64,
}
/// Every polygon edge (prim order, then corner order), each once.
fn mesh_edges(d: &Detail) -> Vec<MeshEdge> {
    let p = d.p();
    let mut at = std::collections::HashMap::new();
    let mut out: Vec<MeshEdge> = Vec::new();
    for (q, pr) in d.prims.iter().enumerate() {
        let closed = match pr.kind {
            hexe_geo::PrimKind::Poly { closed } => closed,
            _ => continue,
        };
        let pts: Vec<u32> = pr.verts.clone().filter_map(|v| d.vertex_point.get(v as usize).copied()).collect();
        let n = pts.len();
        let segs = if closed && n > 2 { n } else { n.saturating_sub(1) };
        for k in 0..segs {
            let (a, b) = (pts[k], pts[(k + 1) % n]);
            if a == b {
                continue;
            }
            let i = *at.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let len = match (p.get(a as usize), p.get(b as usize)) {
                    (Some(x), Some(y)) => (0..3).map(|c| f64::from(x[c] - y[c]).powi(2)).sum::<f64>().sqrt(),
                    _ => 0.0,
                };
                out.push(MeshEdge { ends: [a, b], prims: Vec::new(), len });
                out.len() - 1
            });
            if let Some(e) = out.get_mut(i)
                && e.prims.last() != Some(&q)
            {
                e.prims.push(q);
            }
        }
    }
    out
}
/// Newell normal of prim `q` (unnormalised; zero for degenerate / non-poly).
fn newell(d: &Detail, q: usize) -> V3 {
    let p = d.p();
    let Some(pr) = d.prims.get(q) else { return [0.0; 3] };
    let pts: Vec<V3> = pr
        .verts
        .clone()
        .filter_map(|v| d.vertex_point.get(v as usize))
        .filter_map(|&i| p.get(i as usize))
        .map(|&x| widen(x))
        .collect();
    let mut n = [0.0; 3];
    for k in 0..pts.len() {
        let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn angle_deg(a: V3, b: V3) -> Option<f64> {
    let (la, lb) = (dot(a, a).sqrt(), dot(b, b).sqrt());
    (la > 0.0 && lb > 0.0).then(|| (dot(a, b) / (la * lb)).clamp(-1.0, 1.0).acos().to_degrees())
}

/// Edge groups (`grouptype` edge). Rules measured against Houdini 20.5 (legacy `group` SOP, hython, sphere / box /
/// folded grid; every case matched edge for edge):
/// * Number tab (`groupnumber`): `pattern` tokens `*` (every edge), `pA-B-C...` (the path's consecutive pairs), or an
///   existing edge group's name.
/// * Edges tab (`groupedges`), families OR-ed with each other and with the pattern: edge ANGLE = angle between the Newell
///   normals of its two prims (edges without exactly 2 prims never qualify), min / max both inclusive and AND-ed; with
///   `edgeanglebetweenedges` an edge qualifies when ANY edge sharing one of its points spreads within min / max of it
///   (angle at the shared point); LENGTH min / max inclusive, AND-ed; `unshared` = edges used by one prim.
/// * Nothing enabled = empty group. `merge_op` vs an existing same-named edge group. Bounding / normal tabs on edges:
///   not modelled (error).
fn cook_edges(p: &ParamSet, inputs: &[Arc<Detail>], name: &str) -> Result<Arc<Detail>, CookError> {
    let tog = |k: &str| p.toggle(k).ok_or_else(|| param_err(k, "missing or not a toggle"));
    let num = |k: &str| {
        p.float(k).filter(|v| v.is_finite()).map(f64::from).ok_or_else(|| param_err(k, "missing or not a finite float"))
    };
    if tog("bbox")? || tog("normal")? {
        return Err(CookError::Unsupported("group: bounding / normal criteria on edge groups are not modelled".into()));
    }
    let merge = p.menu("merge_op").ok_or_else(|| param_err("merge_op", "missing or not a menu"))?;
    if merge > MERGE_SUBTRACT {
        return Err(param_err("merge_op", &format!("menu index {merge} out of range")));
    }
    let empty = Arc::new(Detail::default());
    let d: &Arc<Detail> = inputs.first().unwrap_or(&empty);
    let edges = mesh_edges(d);
    let key = |[a, b]: [u32; 2]| (a.min(b), a.max(b));
    let mut picked: Vec<[u32; 2]> = Vec::new();
    // Number tab
    if p.toggle("groupnumber").unwrap_or(true) {
        for tok in p.string("pattern").unwrap_or("").split_whitespace() {
            if tok == "*" {
                picked.extend(edges.iter().map(|e| e.ends));
            } else if let Some(path) = tok.strip_prefix('p') {
                let ids: Result<Vec<u32>, _> = path.split('-').map(str::parse::<u32>).collect();
                let ids = ids.map_err(|_| param_err("pattern", &format!("bad edge path `{tok}`")))?;
                picked.extend(ids.windows(2).map(|w| [w[0], w[1]]));
            } else if let Some(g) = d.edge_group(tok) {
                picked.extend(g.edges.iter().copied());
            } else {
                return Err(CookError::Failed(format!("group: unknown edge group `{tok}`")));
            }
        }
    }
    // Edges tab
    if tog("groupedges")? {
        let (dmin_a, dmax_a, between) = (tog("dominedgeangle")?, tog("domaxedgeangle")?, tog("edgeanglebetweenedges")?);
        let (min_a, max_a) = (num("minedgeangle")? - ANGLE_EPS, num("maxedgeangle")? + ANGLE_EPS);
        let (dmin_l, dmax_l, unshared) = (tog("dominedgelen")?, tog("domaxedgelen")?, tog("unshared")?);
        let (min_l, max_l) = (num("minedgelen")?, num("maxedgelen")?);
        let in_angle = |a: f64| (!dmin_a || a >= min_a) && (!dmax_a || a <= max_a);
        let normals: Vec<V3> = (0..d.prims.len()).map(|q| newell(d, q)).collect();
        let mut at_point: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
        if between {
            for (i, e) in edges.iter().enumerate() {
                for &pt in &e.ends {
                    at_point.entry(pt).or_default().push(i);
                }
            }
        }
        let pos = |i: u32| d.p().get(i as usize).map(|&x| widen(x));
        for (i, e) in edges.iter().enumerate() {
            let angle_ok = (dmin_a || dmax_a)
                && if between {
                    // any edge sharing a point, measured at that point
                    e.ends.iter().any(|&pt| {
                        let other = |f: &MeshEdge| if f.ends[0] == pt { f.ends[1] } else { f.ends[0] };
                        let (Some(o), Some(c)) = (pos(other(e)), pos(pt)) else { return false };
                        at_point.get(&pt).into_iter().flatten().filter(|&&j| j != i).any(|&j| {
                            edges
                                .get(j)
                                .and_then(|f| pos(other(f)))
                                .and_then(|o2| angle_deg(sub(o, c), sub(o2, c)))
                                .is_some_and(in_angle)
                        })
                    })
                } else if let [q0, q1] = e.prims[..] {
                    angle_deg(normals[q0], normals[q1]).is_some_and(in_angle)
                } else {
                    false
                };
            let len_ok = (dmin_l || dmax_l) && (!dmin_l || e.len >= min_l) && (!dmax_l || e.len <= max_l);
            if angle_ok || len_ok || (unshared && e.prims.len() == 1) {
                picked.push(e.ends);
            }
        }
    }
    // merge with an existing same-named edge group
    let new: std::collections::HashSet<(u32, u32)> = picked.iter().map(|&e| key(e)).collect();
    let old: Vec<[u32; 2]> = d.edge_group(name).map(|g| g.edges.clone()).unwrap_or_default();
    let result: Vec<[u32; 2]> = match merge {
        MERGE_UNION => old.iter().copied().chain(picked.iter().copied()).collect(),
        MERGE_INTERSECT => old.iter().copied().filter(|&e| new.contains(&key(e))).collect(),
        MERGE_SUBTRACT => old.iter().copied().filter(|&e| !new.contains(&key(e))).collect(),
        _ => picked,
    };
    let mut out = (**d).clone();
    out.set_edge_group(name, result);
    Ok(Arc::new(out))
}

impl Node<Detail> for GroupSop {
    fn type_name(&self) -> &'static str {
        "group"
    }
    /// Houdini: only the first input is required.
    fn min_inputs(&self) -> usize {
        1
    }
    fn inputs(&self) -> &[InputSpec] {
        // Houdini input labels; the Bounding Object (Bounding tab "use bounding object") is not modelled yet
        &[InputSpec { name: "Source data" }, InputSpec { name: "Bounding Object" }]
    }
    fn params(&self) -> &[ParamSpec] {
        &self.specs
    }
    fn cook(&self, ctx: &CookCtx, inputs: &[Arc<Detail>]) -> Result<Arc<Detail>, CookError> {
        let p: &ParamSet = ctx.params;
        let name = p.string("groupname").ok_or_else(|| param_err("groupname", "missing or not a string"))?.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(param_err("groupname", "must be a non-empty name without whitespace"));
        }
        let class = match p.menu("grouptype").ok_or_else(|| param_err("grouptype", "missing or not a menu"))? {
            GT_POINT => AttrClass::Point,
            GT_PRIM => AttrClass::Prim,
            GT_VERTEX => AttrClass::Vertex,
            GT_EDGE => return cook_edges(p, inputs, name),
            m => return Err(param_err("grouptype", &format!("menu index {m} out of range"))),
        };
        let merge = p.menu("merge_op").ok_or_else(|| param_err("merge_op", "missing or not a menu"))?;
        if merge > MERGE_SUBTRACT {
            return Err(param_err("merge_op", &format!("menu index {merge} out of range")));
        }
        let pattern = p.string("pattern").ok_or_else(|| param_err("pattern", "missing or not a string"))?;
        let use_box = p.toggle("bbox").ok_or_else(|| param_err("bbox", "missing or not a toggle"))?;
        let size = p.vec3("bbox_size").ok_or_else(|| param_err("bbox_size", "missing or not a vector"))?;
        let center = p.vec3("bbox_center").ok_or_else(|| param_err("bbox_center", "missing or not a vector"))?;
        let use_normal = p.toggle("normal").ok_or_else(|| param_err("normal", "missing or not a toggle"))?;
        let dir = p.vec3("dir").ok_or_else(|| param_err("dir", "missing or not a vector"))?;
        let angle = p.float("angle").ok_or_else(|| param_err("angle", "missing or not a float"))?;
        if size.iter().chain(&center).chain(&dir).any(|c| !c.is_finite()) {
            return Err(bad("group", "non-finite vector component"));
        }
        if !angle.is_finite() || angle < 0.0 {
            return Err(param_err("angle", "must be finite and >= 0"));
        }
        let dir = unit(widen(dir));
        if use_normal && dir.is_none() {
            return Err(param_err("dir", "zero-length direction"));
        }
        let empty = Arc::new(Detail::default());
        let d: &Arc<Detail> = inputs.first().unwrap_or(&empty);
        let n = d.class_len(class);
        // Number tab off: no pattern criterion; with no other criterion either, nothing is grouped (Houdini)
        let number = p.toggle("groupnumber").unwrap_or(true);
        let mut new = if number {
            parse_pattern(d, class, pattern, "group")?.unwrap_or_else(|| vec![true; n])
        } else {
            vec![use_box || use_normal; n]
        };
        if use_box {
            let (c, h) = (widen(center), widen(size));
            let inside = |q: &V3| (0..3).all(|k| (q[k] - c[k]).abs() <= h[k].abs() * 0.5);
            for (m, q) in new.iter_mut().zip(positions(d, class)) {
                *m = *m && q.as_ref().is_some_and(inside);
            }
        }
        if let (true, Some(dir)) = (use_normal, dir) {
            let max = f64::from(angle) + ANGLE_EPS;
            for (m, nv) in new.iter_mut().zip(normals(d, class)) {
                *m = *m && nv.is_some_and(|nv| dot(nv, dir).clamp(-1.0, 1.0).acos().to_degrees() <= max);
            }
        }
        let old: Vec<bool> = match d.group(class, name) {
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
        set_group(&mut res, class, name, out);
        Ok(Arc::new(res))
    }
}

#[cfg(test)]
pub(super) mod tt {
    use super::*;
    /// Cook a registered SOP with default params + overrides.
    pub fn run(ty: &str, ov: &[(&str, ParamValue)], input: Option<&Arc<Detail>>) -> Result<Arc<Detail>, CookError> {
        let node = crate::registry().create(ty).expect("registered");
        let mut ps = node.params().iter().fold(ParamSet::new(), |p, s| p.with(&s.name, s.default.clone()));
        for (k, v) in ov {
            ps = ps.with(k, v.clone());
        }
        let ctx = CookCtx { time: 0.0, fps: 24.0, params: &ps, log: &|_| {} };
        node.cook(&ctx, input.map(std::slice::from_ref).unwrap_or(&[]))
    }
    pub fn grid3() -> Arc<Detail> {
        run(
            "grid",
            &[("size", ParamValue::V3([2.0, 2.0, 0.0])), ("rows", ParamValue::I(3)), ("cols", ParamValue::I(3))],
            None,
        )
        .expect("grid cooks")
    }
    pub fn cube() -> Arc<Detail> {
        run("box", &[], None).expect("box cooks")
    }
    pub fn s(v: &str) -> ParamValue {
        ParamValue::S(v.into())
    }
    pub fn mask(d: &Detail, c: AttrClass, n: &str) -> Vec<usize> {
        let g = d.group(c, n).expect("group exists");
        g.mask.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::tt::*;
    use super::*;

    fn pt() -> ParamValue {
        ParamValue::Menu(GT_POINT)
    }
    fn grp(d: &Arc<Detail>, ov: &[(&str, ParamValue)]) -> Result<Arc<Detail>, CookError> {
        run("group", ov, Some(d))
    }

    /// Edge-group counts recorded from Houdini 20.5 hython (legacy `group`, entity edge, Number off, Edges on) on the
    /// polymesh-triangles sphere rows 20 x cols 20 (hexe's sphere is point-for-point Houdini's there).
    #[test]
    fn edge_groups_match_houdini_on_the_triangle_sphere() {
        let sphere = Arc::new(
            crate::load_scene(r#"{"nodes":[{"name":"s","type":"sphere","params":{"type":2,"surftype":3,"rows":20,"cols":20}}],"display":"s"}"#)
                .map(|(mut g, id)| g.cook(id, 0.0).expect("sphere cooks"))
                .expect("scene"),
        );
        let b = |v: bool| ParamValue::B(v);
        let f = |v: f32| ParamValue::F(v);
        let base = [("grouptype", ParamValue::Menu(GT_EDGE)), ("groupnumber", b(false)), ("groupedges", b(true))];
        type Case<'a> = (&'a str, Vec<(&'a str, ParamValue)>, usize);
        let cases: Vec<Case> = vec![
            ("min = max = 20 (your file)", vec![("dominedgeangle", b(true)), ("domaxedgeangle", b(true))], 0),
            ("min 10", vec![("dominedgeangle", b(true)), ("minedgeangle", f(10.0))], 220),
            ("max 10", vec![("domaxedgeangle", b(true)), ("maxedgeangle", f(10.0))], 860),
            (
                "5..15",
                vec![
                    ("dominedgeangle", b(true)),
                    ("minedgeangle", f(5.0)),
                    ("domaxedgeangle", b(true)),
                    ("maxedgeangle", f(15.0)),
                ],
                520,
            ),
            (
                "length 0.3..0.6",
                vec![
                    ("dominedgelen", b(true)),
                    ("minedgelen", f(0.3)),
                    ("domaxedgelen", b(true)),
                    ("maxedgelen", f(0.6)),
                ],
                220,
            ),
            (
                "length >= 0.3 OR angle >= 10",
                vec![
                    ("dominedgelen", b(true)),
                    ("minedgelen", f(0.3)),
                    ("dominedgeangle", b(true)),
                    ("minedgeangle", f(10.0)),
                ],
                440,
            ),
            (
                "between edges <= 30",
                vec![("edgeanglebetweenedges", b(true)), ("domaxedgeangle", b(true)), ("maxedgeangle", f(30.0))],
                420,
            ),
            (
                "between edges >= 30",
                vec![("edgeanglebetweenedges", b(true)), ("dominedgeangle", b(true)), ("minedgeangle", f(30.0))],
                1080,
            ),
            ("unshared (closed sphere)", vec![("unshared", b(true))], 0),
            ("nothing enabled", vec![], 0),
        ];
        for (what, extra, want) in cases {
            let ov: Vec<(&str, ParamValue)> = base.iter().cloned().chain(extra).collect();
            let d = grp(&sphere, &ov).expect("cooks");
            assert_eq!(d.edge_group("group1").map(|g| g.edges.len()), Some(want), "{what}");
        }
        // Number tab: pattern paths and `*`; off = ignored (the stale pattern in your .hip)
        let path = grp(&sphere, &[("grouptype", ParamValue::Menu(GT_EDGE)), ("pattern", s("p98-99-100 p5-6"))])
            .expect("cooks");
        assert_eq!(path.edge_group("group1").unwrap().edges, vec![[98, 99], [99, 100], [5, 6]]);
        let all = grp(&sphere, &[("grouptype", ParamValue::Menu(GT_EDGE)), ("pattern", s("*"))]).expect("cooks");
        assert_eq!(all.edge_group("group1").unwrap().edges.len(), 1080);
        let off = grp(
            &sphere,
            &[("grouptype", ParamValue::Menu(GT_EDGE)), ("groupnumber", b(false)), ("pattern", s("p1-2"))],
        )
        .expect("cooks");
        assert!(off.edge_group("group1").unwrap().edges.is_empty());
        assert!(grp(&sphere, &[("grouptype", ParamValue::Menu(GT_EDGE)), ("pattern", s("p1-x"))]).is_err());
    }

    #[test]
    fn params_and_defaults() {
        let n = new();
        assert_eq!(n.type_name(), "group");
        let got: Vec<&str> = n.params().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            got,
            [
                "groupname",
                "grouptype",
                "groupnumber",
                "pattern",
                "bbox",
                "bbox_size",
                "bbox_center",
                "normal",
                "dir",
                "angle",
                "merge_op",
                "groupedges",
                "dominedgeangle",
                "minedgeangle",
                "domaxedgeangle",
                "maxedgeangle",
                "edgeanglebetweenedges",
                "dominedgelen",
                "minedgelen",
                "domaxedgelen",
                "maxedgelen",
                "unshared"
            ]
        );
        assert_eq!(n.params()[0].default, s("group1"));
    }
    #[test]
    fn grid_pattern_0_2_is_3_points_and_negated_is_6() {
        let g = grid3();
        assert_eq!((g.point_count, g.prim_count()), (9, 4));
        let a = grp(&g, &[("grouptype", pt()), ("pattern", s("0-2"))]).expect("cooks");
        assert_eq!(mask(&a, AttrClass::Point, "group1"), [0, 1, 2]);
        let b = grp(&g, &[("grouptype", pt()), ("pattern", s("!0-2"))]).expect("cooks");
        assert_eq!(mask(&b, AttrClass::Point, "group1"), [3, 4, 5, 6, 7, 8]);
        // token algebra: left to right, `!` subtracts, ids >= count ignored, `*` all
        let c = grp(&g, &[("grouptype", pt()), ("pattern", s("0-5 !2 7 100-200"))]).expect("cooks");
        assert_eq!(mask(&c, AttrClass::Point, "group1"), [0, 1, 3, 4, 5, 7]);
        let all = grp(&g, &[("grouptype", pt()), ("pattern", s("*"))]).expect("cooks");
        assert_eq!(mask(&all, AttrClass::Point, "group1").len(), 9);
    }
    #[test]
    fn pattern_refers_to_existing_groups_and_unknown_fails() {
        let g = grid3();
        let a = grp(&g, &[("groupname", s("a")), ("grouptype", pt()), ("pattern", s("0-1"))]).expect("cooks");
        let b = grp(&a, &[("groupname", s("b")), ("grouptype", pt()), ("pattern", s("a 8"))]).expect("cooks");
        assert_eq!(mask(&b, AttrClass::Point, "b"), [0, 1, 8]);
        let c = grp(&b, &[("groupname", s("c")), ("grouptype", pt()), ("pattern", s("!a"))]).expect("cooks");
        assert_eq!(mask(&c, AttrClass::Point, "c").len(), 7);
        let w = grp(&b, &[("groupname", s("w")), ("grouptype", pt()), ("pattern", s("?"))]).expect("cooks");
        assert_eq!(mask(&w, AttrClass::Point, "w"), [0, 1, 8]);
        let miss = grp(&g, &[("grouptype", pt()), ("pattern", s("nope"))]);
        assert!(matches!(miss, Err(CookError::Failed(_))));
    }
    #[test]
    fn grid_bbox_unit_box_keeps_exactly_the_centre_point() {
        let g = grid3();
        let (lo, hi) = g.bbox();
        assert_eq!((lo, hi), ([-1.0, 0.0, -1.0], [1.0, 0.0, 1.0]));
        let o = grp(
            &g,
            &[
                ("grouptype", pt()),
                ("bbox", ParamValue::B(true)),
                ("bbox_size", ParamValue::V3([1.0; 3])),
                ("bbox_center", ParamValue::V3([0.0; 3])),
            ],
        )
        .expect("cooks");
        let ids = mask(&o, AttrClass::Point, "group1");
        assert_eq!(ids, [4]);
        assert_eq!(g.p()[4], [0.0, 0.0, 0.0]);
        // faces are inclusive: size 2 keeps the full 3x3 plane
        let all =
            grp(&g, &[("grouptype", pt()), ("bbox", ParamValue::B(true)), ("bbox_size", ParamValue::V3([2.0; 3]))])
                .expect("cooks");
        assert_eq!(mask(&all, AttrClass::Point, "group1").len(), 9);
        // prims: centre of each quad is at (+-0.5, 0, +-0.5): box centred on one keeps exactly that quad
        let q = grp(
            &g,
            &[
                ("bbox", ParamValue::B(true)),
                ("bbox_size", ParamValue::V3([0.2; 3])),
                ("bbox_center", ParamValue::V3([0.5, 0.0, 0.5])),
            ],
        )
        .expect("cooks");
        assert_eq!(mask(&q, AttrClass::Prim, "group1").len(), 1);
    }
    fn top_prim(d: &Detail) -> usize {
        (0..d.prims.len())
            .find(|&i| {
                let pts: Vec<[f32; 3]> =
                    d.vertex_point[prim_verts(d, &d.prims[i])].iter().map(|&p| d.p()[p as usize]).collect();
                pts.iter().all(|q| q[1] > 0.0)
            })
            .expect("a +Y face exists")
    }
    #[test]
    fn box_normal_dir_y_angle_10_is_top_only_and_angle_90_is_all_but_bottom() {
        let b = cube();
        assert_eq!((b.point_count, b.prim_count()), (8, 6));
        let args = |a: f32| {
            [("normal", ParamValue::B(true)), ("dir", ParamValue::V3([0.0, 1.0, 0.0])), ("angle", ParamValue::F(a))]
        };
        let t = grp(&b, &args(10.0)).expect("cooks");
        assert_eq!(mask(&t, AttrClass::Prim, "group1"), [top_prim(&b)]);
        // DECISION: boundary inclusive (angle <= param). 90 deg keeps the 4 strictly-perpendicular sides + top; bottom (180) out.
        let s = grp(&b, &args(90.0)).expect("cooks");
        let ids = mask(&s, AttrClass::Prim, "group1");
        assert_eq!(ids.len(), 5);
        assert!(ids.contains(&top_prim(&b)));
        // just under 90 -> top only again; 180 -> everything
        assert_eq!(mask(&grp(&b, &args(89.0)).expect("cooks"), AttrClass::Prim, "group1").len(), 1);
        assert_eq!(mask(&grp(&b, &args(180.0)).expect("cooks"), AttrClass::Prim, "group1").len(), 6);
        // point normals (averaged): 4 top corners at 60 deg... corner normal = (+-1,1,+-1)/sqrt3 -> 54.7 deg
        let p = grp(&b, &[("grouptype", pt()), ("normal", ParamValue::B(true)), ("angle", ParamValue::F(55.0))])
            .expect("cooks");
        assert_eq!(mask(&p, AttrClass::Point, "group1").len(), 4);
        let p = grp(&b, &[("grouptype", pt()), ("normal", ParamValue::B(true)), ("angle", ParamValue::F(54.0))])
            .expect("cooks");
        assert!(mask(&p, AttrClass::Point, "group1").is_empty());
    }
    #[test]
    fn merge_ops_against_existing_group() {
        let g = grid3();
        let a = grp(&g, &[("grouptype", pt()), ("pattern", s("0-3"))]).expect("cooks");
        let go = |op: u32, pat: &str| {
            let o = grp(&a, &[("grouptype", pt()), ("pattern", s(pat)), ("merge_op", ParamValue::Menu(op))])
                .expect("cooks");
            assert_eq!(o.groups.len(), 1, "same name edits in place");
            mask(&o, AttrClass::Point, "group1")
        };
        assert_eq!(go(MERGE_REPLACE, "2-5"), [2, 3, 4, 5]);
        assert_eq!(go(MERGE_UNION, "2-5"), [0, 1, 2, 3, 4, 5]);
        assert_eq!(go(MERGE_INTERSECT, "2-5"), [2, 3]);
        assert_eq!(go(MERGE_SUBTRACT, "2-5"), [0, 1]);
    }
    #[test]
    fn vertex_group_and_hostile_params() {
        let b = cube();
        let v = grp(&b, &[("grouptype", ParamValue::Menu(GT_VERTEX)), ("pattern", s("0-4"))]).expect("cooks");
        assert_eq!(mask(&v, AttrClass::Vertex, "group1"), [0, 1, 2, 3, 4]);
        // an edge group with no criterion enabled is empty (Houdini); bounding / normal on edges are not modelled
        let e =
            grp(&b, &[("grouptype", ParamValue::Menu(GT_EDGE)), ("groupnumber", ParamValue::B(false))]).expect("cooks");
        assert!(e.edge_group("group1").is_some_and(|g| g.edges.is_empty()));
        let bb = grp(&b, &[("grouptype", ParamValue::Menu(GT_EDGE)), ("bbox", ParamValue::B(true))]);
        assert!(matches!(bb, Err(CookError::Unsupported(_))));
        for (k, val) in [
            ("grouptype", ParamValue::Menu(9)),
            ("merge_op", ParamValue::Menu(9)),
            ("groupname", s("")),
            ("groupname", s("a b")),
            ("pattern", s("5-2")),
            ("pattern", s("99999999999999999999999")),
            ("pattern", s("!")),
            ("angle", ParamValue::F(-1.0)),
        ] {
            assert!(matches!(grp(&b, &[(k, val)]), Err(CookError::BadParam(_))), "{k}");
        }
        let zero = grp(&b, &[("normal", ParamValue::B(true)), ("dir", ParamValue::V3([0.0; 3]))]);
        assert!(matches!(zero, Err(CookError::BadParam(_))));
        // empty / absent input -> empty group, no panic
        let e = run("group", &[], None).expect("cooks");
        assert!(e.group(AttrClass::Prim, "group1").is_some_and(|g| g.mask.is_empty()));
    }
    #[test]
    fn glob_matcher() {
        assert!(
            glob_match("*", "") && glob_match("a*c", "abbc") && glob_match("a?c", "abc") && glob_match("*b*", "abc")
        );
        assert!(!glob_match("a?c", "ac") && !glob_match("a*d", "abc") && !glob_match("", "a"));
        assert!(glob_match(&"*a".repeat(40), &"a".repeat(40)), "no exponential backtracking");
    }
}
