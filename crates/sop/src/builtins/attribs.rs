//! Shared attribute helpers for the H24 attrib family (`attribpromote`, `attribtransfer`, `attribcopy`, `attribdelete`).
//!
//! # Attribute pattern (Houdini grammar; UNVERIFIED vs Houdini, hython blocked)
//! Space-separated tokens, `*`/`?` glob. A token starting with `^` EXCLUDES matches. Evaluated left to right per attribute
//! name: include adds, exclude removes. A pattern whose FIRST token is an exclusion starts from "everything" (`^P` = all but
//! `P`). Empty pattern selects nothing.
use super::group_sop::{bad, glob_match, prim_verts};
use hexe_geo::{AttrClass, AttrData, Attribute, Detail};
use hexe_graph::{CookError, ParamSet};

/// Class menu shared by the family: `point | vertex | prim | detail`.
pub(super) fn class_param(p: &ParamSet, name: &str, who: &str) -> Result<AttrClass, CookError> {
    match p.menu(name).ok_or_else(|| bad(who, format!("`{name}` missing or not a menu")))? {
        0 => Ok(AttrClass::Point),
        1 => Ok(AttrClass::Vertex),
        2 => Ok(AttrClass::Prim),
        3 => Ok(AttrClass::Detail),
        m => Err(bad(who, format!("`{name}`: menu index {m} out of range"))),
    }
}
pub(super) fn str_param<'a>(p: &'a ParamSet, name: &str, who: &str) -> Result<&'a str, CookError> {
    p.string(name).ok_or_else(|| bad(who, format!("`{name}` missing or not a string")))
}
/// Is `name` selected by the attribute pattern `pat`?
pub(super) fn selected(pat: &str, name: &str) -> bool {
    let mut on = pat.split_whitespace().next().is_some_and(|t| t.starts_with('^'));
    for tok in pat.split_whitespace() {
        match tok.strip_prefix('^') {
            Some(body) => {
                if glob_match(body, name) {
                    on = false;
                }
            }
            None => {
                if glob_match(tok, name) {
                    on = true;
                }
            }
        }
    }
    on
}
fn rank(c: AttrClass) -> u8 {
    match c {
        AttrClass::Point => 0,
        AttrClass::Vertex => 1,
        AttrClass::Prim => 2,
        AttrClass::Detail => 3,
    }
}
/// Replace a same class+name attribute in place, else insert keeping attrs ordered by class (stable).
pub(super) fn set_attr(d: &mut Detail, a: Attribute) {
    if let Some(x) = d.attrs.iter_mut().find(|x| x.class == a.class && x.name == a.name) {
        *x = a;
        return;
    }
    let pos = d.attrs.iter().rposition(|x| rank(x.class) <= rank(a.class)).map_or(0, |p| p + 1);
    d.attrs.insert(pos, a);
}
/// Same variant and tuple size (so elements can be copied across).
pub(super) fn same_shape(a: &AttrData, b: &AttrData) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b) && a.tuple_size() == b.tuple_size()
}
/// `n` default elements shaped like `src` (string table kept).
pub(super) fn default_like(src: &AttrData, n: usize) -> AttrData {
    let mut a = match src {
        AttrData::F32(_) => AttrData::F32(Vec::new()),
        AttrData::I32(_) => AttrData::I32(Vec::new()),
        AttrData::V2(_) => AttrData::V2(Vec::new()),
        AttrData::V3(_) => AttrData::V3(Vec::new()),
        AttrData::V4(_) => AttrData::V4(Vec::new()),
        AttrData::Str { table, .. } => AttrData::Str { idx: Vec::new(), table: table.clone() },
        AttrData::FN { size, .. } => AttrData::FN { size: *size, data: Vec::new() },
        AttrData::IN { size, .. } => AttrData::IN { size: *size, data: Vec::new() },
    };
    a.extend_default(n);
    a
}
/// Starting data for writing into `class` of `d`: the existing same-name attribute if its shape matches `src`, else defaults.
pub(super) fn base_for(d: &Detail, class: AttrClass, name: &str, src: &AttrData) -> AttrData {
    match d.attr(class, name) {
        Some(x) if same_shape(&x.data, src) && x.data.len() == d.class_len(class) => x.data.clone(),
        _ => default_like(src, d.class_len(class)),
    }
}
/// Copy element `j` of `src` into element `i` of `dst` (same shape required). `false` = nothing copied.
pub(super) fn copy_elem(dst: &mut AttrData, i: usize, src: &AttrData, j: usize) -> bool {
    if !same_shape(dst, src) {
        return false;
    }
    fn put<T: Clone>(d: &mut [T], i: usize, s: &[T], j: usize) -> bool {
        match (s.get(j), d.get_mut(i)) {
            (Some(v), Some(slot)) => {
                *slot = v.clone();
                true
            }
            _ => false,
        }
    }
    fn put_n<T: Clone>(d: &mut [T], i: usize, s: &[T], j: usize, n: usize) -> bool {
        let (a, b) = (i.checked_mul(n), j.checked_mul(n));
        let (Some(a), Some(b)) = (a, b) else { return false };
        match (s.get(b..b.saturating_add(n)), d.get_mut(a..a.saturating_add(n))) {
            (Some(v), Some(slot)) if v.len() == n && slot.len() == n => {
                slot.clone_from_slice(v);
                true
            }
            _ => false,
        }
    }
    match (dst, src) {
        (AttrData::F32(d), AttrData::F32(s)) => put(d, i, s, j),
        (AttrData::I32(d), AttrData::I32(s)) => put(d, i, s, j),
        (AttrData::V2(d), AttrData::V2(s)) => put(d, i, s, j),
        (AttrData::V3(d), AttrData::V3(s)) => put(d, i, s, j),
        (AttrData::V4(d), AttrData::V4(s)) => put(d, i, s, j),
        (AttrData::FN { size, data: d }, AttrData::FN { data: s, .. }) => put_n(d, i, s, j, *size as usize),
        (AttrData::IN { size, data: d }, AttrData::IN { data: s, .. }) => put_n(d, i, s, j, *size as usize),
        (AttrData::Str { idx: di, table: dt }, AttrData::Str { idx: si, table: st }) => {
            let (Some(&k), true) = (si.get(j), i < di.len()) else { return false };
            let m = match usize::try_from(k).ok().and_then(|u| st.get(u)) {
                None => -1,
                Some(s) => {
                    let pos = match dt.iter().position(|x| x == s) {
                        Some(p) => p,
                        None => {
                            dt.push(s.clone());
                            dt.len() - 1
                        }
                    };
                    i32::try_from(pos).unwrap_or(-1)
                }
            };
            match di.get_mut(i) {
                Some(slot) => {
                    *slot = m;
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}
/// `true` for every variant but `Str`.
pub(super) fn is_numeric(a: &AttrData) -> bool {
    !matches!(a, AttrData::Str { .. })
}
/// Element `j` as f64 components (numeric variants only).
fn row(a: &AttrData, j: usize) -> Option<Vec<f64>> {
    fn f<const N: usize>(v: &[[f32; N]], j: usize) -> Option<Vec<f64>> {
        v.get(j).map(|r| r.iter().map(|x| f64::from(*x)).collect())
    }
    match a {
        AttrData::F32(v) => v.get(j).map(|x| vec![f64::from(*x)]),
        AttrData::I32(v) => v.get(j).map(|x| vec![f64::from(*x)]),
        AttrData::V2(v) => f(v, j),
        AttrData::V3(v) => f(v, j),
        AttrData::V4(v) => f(v, j),
        AttrData::FN { size, data } => {
            let n = *size as usize;
            let s = j.checked_mul(n)?;
            data.get(s..s.checked_add(n)?).map(|r| r.iter().map(|x| f64::from(*x)).collect())
        }
        AttrData::IN { size, data } => {
            let n = *size as usize;
            let s = j.checked_mul(n)?;
            data.get(s..s.checked_add(n)?).map(|r| r.iter().map(|x| f64::from(*x)).collect())
        }
        AttrData::Str { .. } => None,
    }
}
#[allow(clippy::cast_possible_truncation)]
fn to_i32(x: f64) -> i32 {
    // `as` saturates for out-of-range floats and maps NaN to 0.
    x.round() as i32
}
/// Write f64 components into element `i` (ints round, saturating).
#[allow(clippy::cast_possible_truncation)]
fn set_row(a: &mut AttrData, i: usize, r: &[f64]) -> bool {
    fn f<const N: usize>(v: &mut [[f32; N]], i: usize, r: &[f64]) -> bool {
        match (v.get_mut(i), r.len() == N) {
            (Some(slot), true) => {
                for (s, x) in slot.iter_mut().zip(r) {
                    *s = *x as f32;
                }
                true
            }
            _ => false,
        }
    }
    match a {
        AttrData::F32(v) => match (v.get_mut(i), r.first()) {
            (Some(s), Some(x)) => {
                *s = *x as f32;
                true
            }
            _ => false,
        },
        AttrData::I32(v) => match (v.get_mut(i), r.first()) {
            (Some(s), Some(x)) => {
                *s = to_i32(*x);
                true
            }
            _ => false,
        },
        AttrData::V2(v) => f(v, i, r),
        AttrData::V3(v) => f(v, i, r),
        AttrData::V4(v) => f(v, i, r),
        AttrData::FN { size, data } => {
            let n = *size as usize;
            let Some(s) = i.checked_mul(n) else { return false };
            match data.get_mut(s..s.saturating_add(n)) {
                Some(slot) if slot.len() == n && r.len() == n => {
                    for (d, x) in slot.iter_mut().zip(r) {
                        *d = *x as f32;
                    }
                    true
                }
                _ => false,
            }
        }
        AttrData::IN { size, data } => {
            let n = *size as usize;
            let Some(s) = i.checked_mul(n) else { return false };
            match data.get_mut(s..s.saturating_add(n)) {
                Some(slot) if slot.len() == n && r.len() == n => {
                    for (d, x) in slot.iter_mut().zip(r) {
                        *d = to_i32(*x);
                    }
                    true
                }
                _ => false,
            }
        }
        AttrData::Str { .. } => false,
    }
}
/// Promotion / aggregation method (menu order is the ABI of scene JSON).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Method {
    Max,
    Min,
    Average,
    First,
    Last,
    Sum,
}
impl Method {
    pub(super) fn from_menu(m: u32) -> Option<Method> {
        [Method::Max, Method::Min, Method::Average, Method::First, Method::Last, Method::Sum].get(m as usize).copied()
    }
}
/// Aggregate the source elements `idx` into `dst[i]` with `m`. Empty `idx` leaves the default. Strings: first/last only.
pub(super) fn reduce_into(dst: &mut AttrData, i: usize, src: &AttrData, idx: &[usize], m: Method) -> bool {
    let (Some(&first), Some(&last)) = (idx.first(), idx.last()) else { return false };
    if !is_numeric(src) || matches!(m, Method::First | Method::Last) || idx.len() == 1 {
        let j = if m == Method::Last { last } else { first };
        return copy_elem(dst, i, src, j);
    }
    let rows: Vec<Vec<f64>> = idx.iter().filter_map(|&j| row(src, j)).collect();
    let Some(head) = rows.first() else { return false };
    let mut acc = head.clone();
    for r in rows.iter().skip(1) {
        for (a, x) in acc.iter_mut().zip(r) {
            match m {
                Method::Max => *a = a.max(*x),
                Method::Min => *a = a.min(*x),
                _ => *a += *x,
            }
        }
    }
    if m == Method::Average {
        let n = rows.len() as f64;
        acc.iter_mut().for_each(|a| *a /= n);
    }
    set_row(dst, i, &acc)
}
/// Weighted average of source elements `w` (index, weight) into `dst[i]` (numeric only). Zero total weight: no write.
pub(super) fn blend_into(dst: &mut AttrData, i: usize, src: &AttrData, w: &[(usize, f64)]) -> bool {
    let mut acc: Vec<f64> = Vec::new();
    let mut tot = 0.0f64;
    for &(j, wt) in w {
        let Some(r) = row(src, j) else { continue };
        if acc.is_empty() {
            acc = vec![0.0; r.len()];
        }
        for (a, x) in acc.iter_mut().zip(&r) {
            *a += x * wt;
        }
        tot += wt;
    }
    if tot <= 0.0 || acc.is_empty() {
        return false;
    }
    acc.iter_mut().for_each(|a| *a /= tot);
    set_row(dst, i, &acc)
}
/// Source-element lists per destination element for a class change `from -> to` (same class = identity).
/// Aggregation sets: point->prim = the prim's vertices' points; vertex->point = the point's vertices; prim->point = the
/// DISTINCT prims touching the point; prim->vertex / point->vertex = the owner prim / point; X->detail = all of X;
/// detail->X = element 0 for all.
pub(super) fn promote_lists(d: &Detail, from: AttrClass, to: AttrClass) -> Vec<Vec<usize>> {
    use AttrClass::{Detail as Dt, Point, Prim, Vertex};
    let n_to = d.class_len(to);
    let n_from = d.class_len(from);
    if from == to {
        return (0..n_to).map(|i| vec![i]).collect();
    }
    match (from, to) {
        (Dt, _) => vec![vec![0]; n_to],
        (_, Dt) => vec![(0..n_from).collect()],
        (Point, Vertex) => d.vertex_point.iter().map(|&p| vec![p as usize]).collect(),
        (Point, Prim) => d
            .prims
            .iter()
            .map(|pr| prim_verts(d, pr).filter_map(|v| d.vertex_point.get(v).map(|&p| p as usize)).collect())
            .collect(),
        (Vertex, Point) => {
            let mut l = vec![Vec::new(); n_to];
            for (v, &p) in d.vertex_point.iter().enumerate() {
                if let Some(x) = l.get_mut(p as usize) {
                    x.push(v);
                }
            }
            l
        }
        (Vertex, Prim) => d.prims.iter().map(|pr| prim_verts(d, pr).collect()).collect(),
        (Prim, Vertex) => {
            let mut l = vec![Vec::new(); n_to];
            for (i, pr) in d.prims.iter().enumerate() {
                for v in prim_verts(d, pr) {
                    if let Some(x) = l.get_mut(v) {
                        x.push(i);
                    }
                }
            }
            l
        }
        (Prim, Point) => {
            let mut l: Vec<Vec<usize>> = vec![Vec::new(); n_to];
            for (i, pr) in d.prims.iter().enumerate() {
                for v in prim_verts(d, pr) {
                    let slot = d.vertex_point.get(v).and_then(|&p| l.get_mut(p as usize));
                    if let Some(x) = slot.filter(|x| x.last() != Some(&i)) {
                        x.push(i);
                    }
                }
            }
            l
        }
        _ => Vec::new(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pattern_grammar() {
        assert!(selected("*", "P") && selected("N Cd", "Cd") && !selected("N Cd", "P"));
        assert!(!selected("* ^P", "P") && selected("* ^P", "Cd"));
        assert!(!selected("^P", "P") && selected("^P", "N"), "leading exclusion starts from everything");
        assert!(selected("^P *", "P"), "left to right: later include wins");
        assert!(selected("C?", "Cd") && !selected("C?", "Cdd") && !selected("", "P"));
    }
    #[test]
    fn copy_and_reduce_elements() {
        let src = AttrData::V2(vec![[1.0, 4.0], [3.0, 2.0], [5.0, 0.0]]);
        let mut dst = default_like(&src, 2);
        assert!(reduce_into(&mut dst, 0, &src, &[0, 1, 2], Method::Average));
        assert!(reduce_into(&mut dst, 1, &src, &[0, 1, 2], Method::Max));
        assert_eq!(dst, AttrData::V2(vec![[3.0, 2.0], [5.0, 4.0]]));
        assert!(!reduce_into(&mut dst, 0, &src, &[], Method::Sum));
        let s = AttrData::Str { idx: vec![0, 1], table: vec!["a".into(), "b".into()] };
        let mut t = default_like(&s, 2);
        assert!(copy_elem(&mut t, 0, &s, 1) && !copy_elem(&mut t, 5, &s, 0));
        assert_eq!(t, AttrData::Str { idx: vec![1, -1], table: vec!["a".into(), "b".into()] });
    }
}
