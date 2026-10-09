//! Wavefront OBJ reader/writer (`geo-read-obj`, `geo-write-obj`).
//! Read: `v` -> points (`P`); `f` -> closed poly prims, `l` -> polyline prims; corner `v/vt/vn` (1-based, negative = relative to
//! the counts so far) -> vertices; `vn` -> vertex `N` (V3, Normal), `vt` -> vertex `uv` (V3 with z = 0 / `w`; Houdini stores uv
//! as 3 floats); `g a b` -> prim group per name; `usemtl` -> prim string attrib `shop_materialpath`; `o` -> prim group. `s`, `mtllib`, `vp`, curves,
//! surfaces and unknown keywords are ignored. Faces < 3 / lines < 2 corners, index 0, out-of-range index, non-finite numbers -> Err.
//! Write: points `v`, vertex-or-point `N`/`uv` -> `vn`/`vt`, closed polys -> `f`, polylines / open polys -> `l`, `shop_materialpath` ->
//! `usemtl`, prim groups -> `g`. Any other prim kind -> `Unsupported` (never silently dropped).
use crate::error::{MAX_ELEMS, check_len};
use crate::*;
use std::fmt::Write as _;

fn syn<T>(s: impl Into<String>) -> Result<T, GeoError> {
    Err(GeoError::Syntax(s.into()))
}
fn num(tok: &str, line: usize) -> Result<f32, GeoError> {
    match tok.parse::<f32>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ => syn(format!("line {line}: bad number `{tok}`")),
    }
}
/// 1-based or negative OBJ index -> 0-based absolute (positive range checked at the end: forward references are legal).
fn resolve(tok: &str, count: usize, line: usize) -> Result<u32, GeoError> {
    let i: i64 = tok.parse().map_err(|_| GeoError::Syntax(format!("line {line}: bad index `{tok}`")))?;
    let abs = match i {
        0 => return Err(GeoError::Index(format!("line {line}: index 0 (OBJ indices are 1-based)"))),
        1.. => i - 1,
        _ => i64::try_from(count).unwrap_or(i64::MAX) + i,
    };
    if abs < 0 {
        return Err(GeoError::Index(format!("line {line}: relative index {i} reaches before the first element")));
    }
    u32::try_from(abs).map_err(|_| GeoError::Index(format!("line {line}: index {i} too large")))
}
#[derive(Clone, Copy)]
struct Corner {
    v: u32,
    t: Option<u32>,
    n: Option<u32>,
}
struct Counts {
    v: usize,
    t: usize,
    n: usize,
}
fn corner(tok: &str, c: &Counts, line: usize) -> Result<Corner, GeoError> {
    let mut it = tok.split('/');
    let v = resolve(it.next().unwrap_or(""), c.v, line)?;
    let t = match it.next() {
        None | Some("") => None,
        Some(s) => Some(resolve(s, c.t, line)?),
    };
    let n = match it.next() {
        None | Some("") => None,
        Some(s) => Some(resolve(s, c.n, line)?),
    };
    if it.next().is_some() {
        return syn(format!("line {line}: corner `{tok}` has more than 3 parts"));
    }
    Ok(Corner { v, t, n })
}
fn vec3(it: &mut std::str::SplitAsciiWhitespace<'_>, min: usize, line: usize) -> Result<[f32; 3], GeoError> {
    let mut out = [0.0f32; 3];
    let mut got = 0;
    for (slot, tok) in out.iter_mut().zip(it) {
        *slot = num(tok, line)?;
        got += 1;
    }
    if got < min { syn(format!("line {line}: expected at least {min} numbers")) } else { Ok(out) }
}
/// Parse OBJ text into a `Detail`.
pub fn read_obj(bytes: &[u8]) -> Result<Detail, GeoError> {
    check_len(bytes.len())?;
    let text = String::from_utf8_lossy(bytes);
    let (mut p, mut vt, mut vn) = (Vec::<[f32; 3]>::new(), Vec::<[f32; 3]>::new(), Vec::<[f32; 3]>::new());
    let mut corners: Vec<Corner> = Vec::new();
    let mut prims: Vec<(bool, u32, u32)> = Vec::new(); // (is_line, first corner, end corner)
    let (mut mats, mut mat_table, mut cur_mat) = (Vec::<i32>::new(), Vec::<String>::new(), -1i32);
    let (mut group_names, mut group_hits) = (Vec::<String>::new(), Vec::<(usize, usize)>::new());
    let mut cur_groups: Vec<usize> = Vec::new();
    let cap = MAX_ELEMS as usize;
    for (ln, raw) in text.lines().enumerate() {
        let line = ln + 1;
        let mut it = raw.split('#').next().unwrap_or("").split_ascii_whitespace();
        let Some(key) = it.next() else { continue };
        match key {
            "v" | "vn" | "vt" => {
                let (store, min) = match key {
                    "v" => (&mut p, 3),
                    "vn" => (&mut vn, 3),
                    _ => (&mut vt, 1),
                };
                if store.len() >= cap {
                    return Err(GeoError::Count(format!("line {line}: more than {MAX_ELEMS} `{key}` entries")));
                }
                // `v x y z [w | r g b]`: only xyz kept; `vt u [v [w]]`: missing -> 0.
                store.push(vec3(&mut it, min, line)?);
            }
            "f" | "l" => {
                let is_line = key == "l";
                let c = Counts { v: p.len(), t: vt.len(), n: vn.len() };
                let first = corners.len();
                for tok in it {
                    if corners.len() >= cap {
                        return Err(GeoError::Count(format!("line {line}: more than {MAX_ELEMS} corners")));
                    }
                    corners.push(corner(tok, &c, line)?);
                }
                let n = corners.len() - first;
                if (is_line && n < 2) || (!is_line && n < 3) {
                    return syn(format!("line {line}: `{key}` with {n} corners"));
                }
                let id = prims.len();
                for &g in &cur_groups {
                    group_hits.push((g, id));
                }
                prims.push((is_line, first as u32, corners.len() as u32));
                mats.push(cur_mat);
            }
            "g" | "o" => {
                if key == "g" {
                    cur_groups.clear();
                }
                for name in it {
                    let gi = match group_names.iter().position(|g| g == name) {
                        Some(i) => i,
                        None => {
                            group_names.push(name.to_string());
                            group_names.len() - 1
                        }
                    };
                    cur_groups.push(gi);
                }
            }
            "usemtl" => {
                let name = it.next().unwrap_or("");
                let ix = match mat_table.iter().position(|m| m == name) {
                    Some(i) => i,
                    None => {
                        mat_table.push(name.to_string());
                        mat_table.len() - 1
                    }
                };
                cur_mat = i32::try_from(ix).unwrap_or(-1);
            }
            _ => {}
        }
    }
    for c in &corners {
        let bad = |what: &str, i: u32, n: usize| {
            GeoError::Index(format!("{what} index {} out of range (1..={n})", i as u64 + 1))
        };
        if c.v as usize >= p.len() {
            return Err(bad("vertex", c.v, p.len()));
        }
        if let Some(t) = c.t.filter(|&t| t as usize >= vt.len()) {
            return Err(bad("texcoord", t, vt.len()));
        }
        if let Some(n) = c.n.filter(|&n| n as usize >= vn.len()) {
            return Err(bad("normal", n, vn.len()));
        }
    }
    let mut d = Detail {
        point_count: u32::try_from(p.len()).map_err(|_| GeoError::Count("points".into()))?,
        vertex_point: corners.iter().map(|c| c.v).collect(),
        ..Detail::default()
    };
    for &(is_line, a, b) in &prims {
        let kind = if is_line { PrimKind::Polyline } else { PrimKind::Poly { closed: true } };
        d.prims.push(Prim { kind, verts: a..b });
    }
    d.attrs.push(Attribute {
        name: "P".into(),
        class: AttrClass::Point,
        data: AttrData::V3(p),
        type_info: TypeInfo::Point,
    });
    if corners.iter().any(|c| c.n.is_some()) {
        let v = corners.iter().map(|c| c.n.and_then(|i| vn.get(i as usize)).copied().unwrap_or([0.0; 3])).collect();
        d.attrs.push(Attribute {
            name: "N".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(v),
            type_info: TypeInfo::Normal,
        });
    }
    if corners.iter().any(|c| c.t.is_some()) {
        let v = corners.iter().map(|c| c.t.and_then(|i| vt.get(i as usize)).copied().unwrap_or([0.0; 3])).collect();
        d.attrs.push(Attribute {
            name: "uv".into(),
            class: AttrClass::Vertex,
            data: AttrData::V3(v),
            type_info: TypeInfo::None,
        });
    }
    if !mat_table.is_empty() {
        let data = AttrData::Str { idx: mats, table: mat_table };
        d.attrs.push(Attribute { name: "shop_materialpath".into(), class: AttrClass::Prim, data, type_info: TypeInfo::None });
    }
    for (gi, name) in group_names.into_iter().enumerate() {
        let mut mask = vec![false; prims.len()];
        for &(_, pi) in group_hits.iter().filter(|h| h.0 == gi) {
            if let Some(m) = mask.get_mut(pi) {
                *m = true;
            }
        }
        d.groups.push(Group { name, class: AttrClass::Prim, mask });
    }
    d.canonicalize();
    Ok(d)
}
/// V3 (or V2 widened with z = 0) attribute of `class`/`name`.
pub(crate) fn v3_of(d: &Detail, class: AttrClass, name: &str) -> Option<Vec<[f32; 3]>> {
    match &d.attr(class, name)?.data {
        AttrData::V3(v) => Some(v.clone()),
        AttrData::V2(v) => Some(v.iter().map(|q| [q[0], q[1], 0.0]).collect()),
        _ => None,
    }
}
fn finite(v: &[[f32; 3]]) -> Result<(), GeoError> {
    if v.iter().flatten().all(|c| c.is_finite()) { Ok(()) } else { syn("non-finite value cannot be written") }
}
/// Serialise to OBJ text.
pub fn write_obj(d: &Detail) -> Result<Vec<u8>, GeoError> {
    let p = d.p();
    if p.len() != d.point_count as usize {
        return Err(GeoError::Count(format!("P has {} entries for {} points", p.len(), d.point_count)));
    }
    finite(p)?;
    let pick = |name: &str| -> (Option<Vec<[f32; 3]>>, bool) {
        match v3_of(d, AttrClass::Vertex, name) {
            Some(v) => (Some(v), true),
            None => (v3_of(d, AttrClass::Point, name), false),
        }
    };
    let ((n, n_per_vertex), (t, t_per_vertex)) = (pick("N"), pick("uv"));
    for (a, per_vertex, what) in [(&n, n_per_vertex, "N"), (&t, t_per_vertex, "uv")] {
        let want = if per_vertex { d.vertex_point.len() } else { d.point_count as usize };
        if a.as_ref().is_some_and(|v| v.len() != want) {
            return Err(GeoError::Count(format!("{what} length differs from its class length {want}")));
        }
    }
    let mut out = String::from("# hexe obj\n");
    for q in p {
        let _ = writeln!(out, "v {} {} {}", q[0], q[1], q[2]);
    }
    if let Some(n) = &n {
        finite(n)?;
        for q in n {
            let _ = writeln!(out, "vn {} {} {}", q[0], q[1], q[2]);
        }
    }
    if let Some(t) = &t {
        finite(t)?;
        let w = t.iter().any(|q| q[2] != 0.0);
        for q in t {
            let _ =
                if w { writeln!(out, "vt {} {} {}", q[0], q[1], q[2]) } else { writeln!(out, "vt {} {}", q[0], q[1]) };
        }
    }
    let mat = match d.attr(AttrClass::Prim, "shop_materialpath").map(|a| &a.data) {
        Some(AttrData::Str { idx, table }) => Some((idx, table)),
        _ => None,
    };
    // Build prim -> groups map
    let mut prim_groups: Vec<Vec<&str>> = vec![Vec::new(); d.prims.len()];
    for g in d.groups.iter().filter(|g| g.class == AttrClass::Prim) {
        for (pi, &m) in g.mask.iter().enumerate() {
            if m && pi < prim_groups.len() {
                prim_groups[pi].push(&g.name);
            }
        }
    }
    let mut cur_mat: Option<i32> = None;
    let mut cur_groups: Vec<&str> = Vec::new();
    for (pi, prim) in d.prims.iter().enumerate() {
        let key = match prim.kind {
            PrimKind::Poly { closed: true } => "f",
            PrimKind::Poly { closed: false } | PrimKind::Polyline => "l",
            _ => {
                return Err(GeoError::Unsupported(format!(
                    "prim {pi}: only polygons and polylines can be written to OBJ"
                )));
            }
        };
        // Emit `g` if groups changed
        let pg = prim_groups.get(pi).map(|v| v.as_slice()).unwrap_or(&[]);
        if pg != cur_groups.as_slice() {
            if pg.is_empty() {
                let _ = writeln!(out, "g");
            } else {
                let _ = write!(out, "g");
                for name in pg {
                    let _ = write!(out, " {name}");
                }
                out.push('\n');
            }
            cur_groups = pg.to_vec();
        }
        if let Some((idx, table)) = mat {
            let m = idx.get(pi).copied().unwrap_or(-1);
            if m >= 0 && cur_mat != Some(m) {
                if let Some(name) = table.get(m as usize) {
                    let _ = writeln!(out, "usemtl {name}");
                }
                cur_mat = Some(m);
            }
        }
        out.push_str(key);
        for vid in prim.verts.clone() {
            let pid = d.vertex_point.get(vid as usize).copied().filter(|&q| q < d.point_count);
            let pid = pid.ok_or_else(|| GeoError::Index(format!("prim {pi}: vertex {vid} has no valid point")))?;
            let ti = t.as_ref().map(|_| (if t_per_vertex { vid } else { pid }) + 1);
            let ni = n.as_ref().map(|_| (if n_per_vertex { vid } else { pid }) + 1);
            let _ = match (ti, ni) {
                (Some(t), Some(n)) => write!(out, " {}/{t}/{n}", pid + 1),
                (Some(t), None) => write!(out, " {}/{t}", pid + 1),
                (None, Some(n)) => write!(out, " {}//{n}", pid + 1),
                (None, None) => write!(out, " {}", pid + 1),
            };
        }
        out.push('\n');
    }
    Ok(out.into_bytes())
}
