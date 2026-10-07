//! Houdini `.geo` JSON reader. Houdini writes every container as an alternating `[key, value, key, value, ...]` array;
//! plain JSON objects are accepted too. Everything is bounds-checked; hostile input → `Err`, never panic.

use crate::error::{MAX_ELEMS, check_len, count_of};
use crate::{AttrClass, AttrData, Attribute, Detail, GeoError, Group, Prim, PrimKind, TypeInfo};
use serde_json::Value;

pub(crate) type Kv<'a> = Vec<(&'a str, &'a Value)>;

/// Cap on decoded scalar count of one attribute (guards constant-page amplification).
const MAX_SCALARS: u64 = 1 << 27;
const MAX_DEPTH: usize = 12;

pub(crate) fn syn<T>(s: impl Into<String>) -> Result<T, GeoError> {
    Err(GeoError::Syntax(s.into()))
}

/// Alternating-array or object → key/value list. `None` if neither.
pub(crate) fn kv(v: &Value) -> Option<Kv<'_>> {
    match v {
        Value::Object(m) => Some(m.iter().map(|(k, v)| (k.as_str(), v)).collect()),
        Value::Array(a) if a.len() % 2 == 0 => {
            let mut out = Vec::with_capacity(a.len() / 2);
            for ch in a.chunks(2) {
                out.push((ch.first()?.as_str()?, ch.get(1)?));
            }
            Some(out)
        }
        _ => None,
    }
}

pub(crate) fn find<'a>(k: &Kv<'a>, key: &str) -> Option<&'a Value> {
    k.iter().find(|(n, _)| *n == key).map(|(_, v)| *v)
}

fn need_kv<'a>(v: &'a Value, what: &str) -> Result<Kv<'a>, GeoError> {
    kv(v).map_or_else(|| syn(format!("{what}: expected key/value container")), Ok)
}

fn int_field(k: &Kv<'_>, key: &str) -> Result<i64, GeoError> {
    match find(k, key).and_then(Value::as_i64) {
        Some(i) => Ok(i),
        None => syn(format!("missing/non-integer `{key}`")),
    }
}

fn flatten(v: &Value, out: &mut Vec<f64>, depth: usize) -> Result<(), GeoError> {
    if depth > MAX_DEPTH {
        return syn("numeric array nested too deeply");
    }
    match v {
        Value::Array(a) => {
            for x in a {
                flatten(x, out, depth + 1)?;
            }
            Ok(())
        }
        Value::Number(n) => {
            out.push(n.as_f64().unwrap_or(0.0));
            Ok(())
        }
        Value::Bool(b) => {
            out.push(f64::from(u8::from(*b)));
            Ok(())
        }
        _ => syn("non-numeric element in numeric array"),
    }
}

fn flat(v: &Value) -> Result<Vec<f64>, GeoError> {
    let mut o = Vec::new();
    flatten(v, &mut o, 0)?;
    Ok(o)
}

fn ints(v: &Value) -> Result<Vec<i64>, GeoError> {
    Ok(flat(v)?.into_iter().map(|f| f as i64).collect())
}

/// Is page `p` of packing group `g` flagged constant? `flags` = the `constantpageflags` value.
fn page_constant(flags: Option<&Value>, groups: usize, g: usize, p: usize) -> bool {
    let Some(Value::Array(top)) = flags else { return false };
    // one array per packing group; tolerate a single flat array when there is one group
    let per = if top.len() == groups && top.iter().all(|x| x.is_array() || x.is_number()) { top.get(g) } else { None };
    let arr_owned;
    let (words, from_top): (&[Value], bool) = match per {
        Some(Value::Array(a)) => (a.as_slice(), false),
        Some(single @ Value::Number(_)) => {
            arr_owned = [single.clone()];
            (&arr_owned[..], false)
        }
        _ if groups == 1 => (top.as_slice(), true),
        _ => return false,
    };
    let _ = from_top;
    if words.iter().all(Value::is_boolean) {
        return words.get(p).and_then(Value::as_bool).unwrap_or(false);
    }
    let Some(w) = words.get(p / 32).and_then(Value::as_u64) else { return false };
    (w >> (p % 32)) & 1 == 1
}

fn decode_pages(k: &Kv<'_>, raw: &[f64], count: usize, size: usize) -> Result<Vec<f64>, GeoError> {
    let packing: Vec<usize> = match find(k, "packing") {
        Some(v) => ints(v)?.into_iter().map(|i| usize::try_from(i).unwrap_or(0)).collect(),
        None => vec![size],
    };
    if packing.is_empty() || packing.contains(&0) || packing.iter().sum::<usize>() != size {
        return syn("rawpagedata: packing does not sum to tuple size");
    }
    let pagesize = match find(k, "pagesize").and_then(Value::as_u64) {
        Some(0) => return syn("rawpagedata: pagesize 0"),
        Some(p) => usize::try_from(p).unwrap_or(usize::MAX),
        None => 1024,
    };
    let total = count.checked_mul(size).filter(|&t| (t as u64) <= MAX_SCALARS);
    let Some(total) = total else { return Err(GeoError::Count("attribute too large".into())) };
    let flags = find(k, "constantpageflags");
    let mut out = vec![0.0f64; total];
    let mut cur = 0usize;
    let mut base = 0usize;
    let mut page = 0usize;
    while base < count {
        let n = pagesize.min(count - base);
        let mut off = 0usize;
        for (g, &w) in packing.iter().enumerate() {
            if page_constant(flags, packing.len(), g, page) {
                let src = raw.get(cur..cur + w).ok_or_else(|| GeoError::Count("rawpagedata truncated".into()))?;
                cur += w;
                for e in 0..n {
                    for (c, v) in src.iter().enumerate() {
                        if let Some(slot) = out.get_mut((base + e) * size + off + c) {
                            *slot = *v;
                        }
                    }
                }
            } else {
                let len = n * w;
                let src = raw.get(cur..cur + len).ok_or_else(|| GeoError::Count("rawpagedata truncated".into()))?;
                cur += len;
                for e in 0..n {
                    for c in 0..w {
                        if let (Some(v), Some(slot)) = (src.get(e * w + c), out.get_mut((base + e) * size + off + c)) {
                            *slot = *v;
                        }
                    }
                }
            }
            off += w;
        }
        base += n;
        page += 1;
    }
    Ok(out)
}

/// Decode a `values` container to `(size, is_float, flat scalars)` with exactly `count * size` scalars.
pub(crate) fn read_values(v: &Value, count: usize) -> Result<(usize, bool, Vec<f64>), GeoError> {
    let k = need_kv(v, "values")?;
    let size = int_field(&k, "size")?;
    if !(1..=4096).contains(&size) {
        return Err(GeoError::Count(format!("tuple size {size} out of range")));
    }
    let size = size as usize;
    let storage = find(&k, "storage").and_then(Value::as_str).unwrap_or("fpreal32");
    let is_float = storage.starts_with("fpreal");
    let want = count.checked_mul(size).filter(|&t| (t as u64) <= MAX_SCALARS);
    let Some(want) = want else { return Err(GeoError::Count("attribute too large".into())) };
    let data = if let Some(t) = find(&k, "tuples").or_else(|| find(&k, "arrays")) {
        flat(t)?
    } else if let Some(r) = find(&k, "rawpagedata") {
        decode_pages(&k, &flat(r)?, count, size)?
    } else {
        return syn("values: none of tuples/arrays/rawpagedata");
    };
    if data.len() != want {
        return Err(GeoError::Count(format!("values hold {} scalars, expected {want}", data.len())));
    }
    Ok((size, is_float, data))
}

pub(crate) fn pack(size: usize, float: bool, d: &[f64]) -> AttrData {
    let s = size as u32;
    if float {
        let f: Vec<f32> = d.iter().map(|&x| x as f32).collect();
        match size {
            1 => AttrData::F32(f),
            2 => AttrData::V2(f.as_chunks::<2>().0.to_vec()),
            3 => AttrData::V3(f.as_chunks::<3>().0.to_vec()),
            4 => AttrData::V4(f.as_chunks::<4>().0.to_vec()),
            _ => AttrData::FN { size: s, data: f },
        }
    } else {
        let i: Vec<i32> = d.iter().map(|&x| x as i32).collect();
        if size == 1 { AttrData::I32(i) } else { AttrData::IN { size: s, data: i } }
    }
}

fn type_info_of(meta: &Kv<'_>) -> TypeInfo {
    let name = find(meta, "options")
        .and_then(kv)
        .and_then(|o| find(&o, "type"))
        .and_then(|t| match t {
            Value::String(s) => Some(s.as_str()),
            other => kv(other).and_then(|tk| find(&tk, "value")).and_then(Value::as_str),
        })
        .unwrap_or("");
    match name {
        "point" => TypeInfo::Point,
        "vector" | "hpoint" => TypeInfo::Vector,
        "normal" => TypeInfo::Normal,
        "color" => TypeInfo::Color,
        "quaternion" => TypeInfo::Quaternion,
        _ => TypeInfo::None,
    }
}

/// `Ok(None)` = attribute kind we deliberately skip (dict/array/etc), never an error.
fn read_attr(entry: &Value, class: AttrClass, count: usize) -> Result<Option<Attribute>, GeoError> {
    let Value::Array(pair) = entry else { return syn("attribute entry is not a [meta, data] pair") };
    let (Some(m), Some(d)) = (pair.first(), pair.get(1)) else { return syn("attribute entry needs [meta, data]") };
    let meta = need_kv(m, "attribute meta")?;
    let data = need_kv(d, "attribute data")?;
    let Some(name) = find(&meta, "name").and_then(Value::as_str) else { return syn("attribute without name") };
    let ty = find(&meta, "type").and_then(Value::as_str).unwrap_or("numeric");
    let type_info = type_info_of(&meta);
    let attr = |data: AttrData, ti: TypeInfo| Attribute { name: name.to_string(), class, data, type_info: ti };
    match ty {
        "numeric" => {
            let Some(vals) = find(&data, "values") else { return syn(format!("attribute `{name}` without values")) };
            let (size, float, d) = read_values(vals, count)?;
            if class == AttrClass::Point && name == "P" {
                if size < 3 || !float {
                    return syn("P must be a float tuple of size >= 3");
                }
                let p: Vec<[f32; 3]> = d.chunks_exact(size).map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]).collect();
                return Ok(Some(attr(AttrData::V3(p), TypeInfo::Point)));
            }
            Ok(Some(attr(pack(size, float, &d), type_info)))
        }
        "string" => {
            let table: Vec<String> = match find(&data, "strings") {
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|s| s.as_str().map(str::to_string))
                    .collect::<Option<_>>()
                    .ok_or_else(|| GeoError::Syntax("non-string in string table".into()))?,
                _ => return syn(format!("string attribute `{name}` without strings")),
            };
            let Some(iv) = find(&data, "indices") else {
                return syn(format!("string attribute `{name}` without indices"));
            };
            // Houdini writes an empty string attribute (e.g. detail `varmap`) with `size 0`, no strings, no data: all unset.
            if table.is_empty() && kv(iv).and_then(|k| find(&k, "size").and_then(Value::as_i64)) == Some(0) {
                return Ok(Some(attr(AttrData::Str { idx: vec![-1; count], table }, TypeInfo::None)));
            }
            let (size, _, d) = read_values(iv, count)?;
            if size != 1 {
                return syn("string indices must have size 1");
            }
            let mut idx = Vec::with_capacity(d.len());
            for x in d {
                let i = x as i64;
                if i < -1 || i >= table.len() as i64 {
                    return Err(GeoError::Index(format!("string index {i} outside table of {}", table.len())));
                }
                idx.push(i as i32);
            }
            Ok(Some(attr(AttrData::Str { idx, table }, TypeInfo::None)))
        }
        _ => Ok(None),
    }
}

fn expand_count_bool_rle(v: &Value, n: usize) -> Option<Vec<bool>> {
    let a = v.as_array()?;
    if a.is_empty() || a.len() % 2 != 0 || !a.chunks(2).all(|c| c[0].is_u64() && c[1].is_boolean()) {
        return None;
    }
    let mut m = Vec::with_capacity(n);
    for c in a.chunks(2) {
        let cnt = usize::try_from(c[0].as_u64()?).ok()?;
        if m.len().checked_add(cnt)? > n {
            return None;
        }
        m.resize(m.len() + cnt, c[1].as_bool()?);
    }
    (m.len() == n).then_some(m)
}
fn expand_rle(arr: &[i64], n: usize) -> Option<Vec<bool>> {
    fn sum(mut it: impl Iterator<Item = i64>) -> Option<i64> {
        it.try_fold(0i64, |a, b| if b < 0 { None } else { a.checked_add(b) })
    }
    // interpretation 1: (value, count) pairs
    if arr.len().is_multiple_of(2) && arr.chunks(2).all(|c| c[0] == 0 || c[0] == 1) {
        let total = sum(arr.chunks(2).map(|c| c[1]));
        if total == Some(n as i64) {
            let mut m = Vec::with_capacity(n);
            for c in arr.chunks(2) {
                m.resize(m.len() + c[1] as usize, c[0] == 1);
            }
            return Some(m);
        }
    }
    // interpretation 2: alternating run lengths starting with `false`
    if sum(arr.iter().copied()) == Some(n as i64) {
        let mut m = Vec::with_capacity(n);
        let mut v = false;
        for &c in arr {
            m.resize(m.len() + c as usize, v);
            v = !v;
        }
        return Some(m);
    }
    None
}

fn read_groups(v: &Value, class: AttrClass, n: usize, out: &mut Vec<Group>) -> Result<(), GeoError> {
    let Value::Array(entries) = v else { return syn("groups: expected array") };
    for e in entries {
        // entry = flat kv, or [meta-kv, data-kv]
        let mut merged: Vec<(&str, &Value)> = Vec::new();
        match e {
            Value::Array(a) if !a.is_empty() && a.iter().all(|x| x.is_array() || x.is_object()) => {
                for part in a {
                    merged.extend(need_kv(part, "group part")?);
                }
            }
            other => merged.extend(need_kv(other, "group")?),
        }
        let Some(name) = find(&merged, "name").and_then(Value::as_str) else { return syn("group without name") };
        let mut mask = vec![false; n];
        if let Some(sel) = find(&merged, "selection") {
            let sk = need_kv(sel, "selection")?;
            if let Some(u) = find(&sk, "unordered") {
                let uk = need_kv(u, "unordered")?;
                if let Some(r) = find(&uk, "boolRLE") {
                    // Houdini 20.5 (seen in shipped .bgeo): `[count, bool, count, bool, ...]` with real JSON booleans.
                    if let Some(m) = expand_count_bool_rle(r, n) {
                        out.push(Group { name: name.to_string(), class, mask: m });
                        continue;
                    }
                    let a = ints(r)?;
                    mask = expand_rle(&a, n).ok_or_else(|| {
                        GeoError::Count(format!("group `{name}` boolRLE does not cover {n} elements"))
                    })?;
                }
            } else if let Some(o) = find(&sk, "ordered") {
                for i in ints(o)? {
                    match usize::try_from(i).ok().filter(|&u| u < n) {
                        Some(u) => mask[u] = true,
                        None => return Err(GeoError::Index(format!("group `{name}` ordered index {i} out of range"))),
                    }
                }
            }
        }
        out.push(Group { name: name.to_string(), class, mask });
    }
    Ok(())
}

fn transform9(body: &Kv<'_>) -> [f32; 9] {
    let mut x = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0f32];
    if let Some(Ok(t)) = find(body, "transform").map(flat) {
        let stride = if t.len() == 16 { 4 } else { 3 };
        if t.len() == 9 || t.len() == 16 {
            for r in 0..3 {
                for c in 0..3 {
                    x[r * 3 + c] = t[r * stride + c] as f32;
                }
            }
        }
    }
    x
}

fn contiguous(vs: &[i64], nverts: u32) -> Result<Option<std::ops::Range<u32>>, GeoError> {
    let mut prev: Option<i64> = None;
    for &v in vs {
        if v < 0 || v >= i64::from(nverts) {
            return Err(GeoError::Index(format!("primitive vertex {v} outside {nverts} vertices")));
        }
        if prev.is_some_and(|p| v != p + 1) {
            return Ok(None);
        }
        prev = Some(v);
    }
    Ok(match (vs.first(), vs.last()) {
        (Some(&a), Some(&b)) => Some(a as u32..(b as u32) + 1),
        _ => Some(0..0),
    })
}

fn read_prims(v: &Value, nverts: u32, nprims: u32) -> Result<Vec<Prim>, GeoError> {
    let Value::Array(entries) = v else { return syn("primitives: expected array") };
    let mut out: Vec<Prim> = Vec::new();
    for e in entries {
        let Value::Array(pair) = e else { return syn("primitive entry is not [header, body]") };
        let hdr =
            need_kv(pair.first().ok_or_else(|| GeoError::Syntax("empty primitive entry".into()))?, "primitive header")?;
        let ty = find(&hdr, "type").and_then(Value::as_str).unwrap_or("");
        let body = pair.get(1).and_then(kv).unwrap_or_default();
        let unknown = |range: std::ops::Range<u32>| Prim {
            kind: PrimKind::Unknown { ty: ty.to_string(), json: serde_json::to_string(e).unwrap_or_default() },
            verts: range,
        };
        match ty {
            // `Polygon_run`/`PolygonCurve_run` = long names (authored schema); `p_r`/`c_r` + `s_v`/`n_p`/`r_v`/`n_v` = the
            // abbreviations Houdini 20.5 binary .bgeo files use (seen in the shipped corpus). `n_v` = one vertex count per prim.
            "Polygon_run" | "PolygonCurve_run" | "p_r" | "c_r" => {
                let closed = ty == "Polygon_run" || ty == "p_r";
                let start = find(&body, "startvertex").or_else(|| find(&body, "s_v")).and_then(Value::as_i64);
                let Some(start) = start else { return syn("run without start vertex") };
                let n = find(&body, "nprimitives").or_else(|| find(&body, "n_p")).and_then(Value::as_i64);
                let Some(n) = n else { return syn("run without primitive count") };
                if start < 0 || n < 0 {
                    return Err(GeoError::Count("negative run start/count".into()));
                }
                let rle = if let Some(r) = find(&body, "nvertices_rle").or_else(|| find(&body, "r_v")) {
                    ints(r)?
                } else if let Some(l) = find(&body, "n_v") {
                    ints(l)?.into_iter().flat_map(|c| [c, 1]).collect()
                } else {
                    return syn("run without vertex counts");
                };
                if rle.len() % 2 != 0 {
                    return syn("nvertices_rle has odd length");
                }
                let mut cursor = start;
                let mut made = 0i64;
                for c in rle.chunks(2) {
                    let (nv, cnt) = (c[0], c[1]);
                    if nv < 0 || cnt < 0 {
                        return Err(GeoError::Count("negative nvertices_rle entry".into()));
                    }
                    for _ in 0..cnt {
                        let end = cursor.checked_add(nv).filter(|&e| e <= i64::from(nverts));
                        let Some(end) = end else {
                            return Err(GeoError::Index("polygon run exceeds vertex count".into()));
                        };
                        if out.len() as u64 >= MAX_ELEMS || out.len() as u64 > u64::from(nprims) {
                            return Err(GeoError::Count("more primitives than primitivecount".into()));
                        }
                        out.push(Prim { kind: PrimKind::Poly { closed }, verts: cursor as u32..end as u32 });
                        cursor = end;
                        made += 1;
                    }
                }
                if made != n {
                    return Err(GeoError::Count(format!("run claims {n} primitives, rle yields {made}")));
                }
            }
            // Packed-primitive run (Houdini 20.5 bgeo): `[["type","run","runtype",T,"varyingfields",F,"uniformfields",U], [item, ...]]`,
            // each item = `[varying-values, vertex]`. One passthrough prim per item, normalised to the single-prim shape.
            "run" => {
                let hv: Value = pair.first().cloned().unwrap_or(Value::Null);
                let hk = kv(&hv).unwrap_or_default();
                let rt = find(&hk, "runtype").and_then(Value::as_str).unwrap_or("run").to_string();
                let items = pair.get(1).and_then(Value::as_array).cloned().unwrap_or_default();
                for it in items {
                    if out.len() as u64 > u64::from(nprims) {
                        return Err(GeoError::Count("more primitives than primitivecount".into()));
                    }
                    let vtx = it.as_array().and_then(|a| a.last()).and_then(Value::as_i64).unwrap_or(-1);
                    let r = contiguous(&[vtx], nverts)?.unwrap_or(0..0);
                    let mut b = vec![Value::from("vertex"), Value::from(vec![vtx]), Value::from("item"), it.clone()];
                    for k in ["varyingfields", "uniformfields"] {
                        if let Some(x) = find(&hk, k) {
                            b.push(Value::from(k));
                            b.push(x.clone());
                        }
                    }
                    let norm = Value::Array(vec![Value::Array(vec![Value::from("type"), Value::from(rt.clone())]), Value::Array(b)]);
                    out.push(Prim {
                        kind: PrimKind::Unknown { ty: rt.clone(), json: serde_json::to_string(&norm).unwrap_or_default() },
                        verts: r,
                    });
                }
            }
            "Polygon" | "Poly" => {
                let vs =
                    body.iter().find(|(k, _)| *k == "vertex").map(|(_, v)| ints(v)).transpose()?.unwrap_or_default();
                let closed = find(&body, "closed").and_then(Value::as_bool).unwrap_or(true);
                match contiguous(&vs, nverts)? {
                    Some(r) => out.push(Prim { kind: PrimKind::Poly { closed }, verts: r }),
                    None => out.push(unknown(0..0)),
                }
            }
            "Sphere" | "Tube" => {
                let vs = find(&body, "vertex").map(ints).transpose()?.unwrap_or_default();
                let Some(&v0) = vs.first() else { return syn(format!("{ty} without vertex")) };
                if v0 < 0 || v0 >= i64::from(nverts) {
                    return Err(GeoError::Index(format!("{ty} vertex {v0} out of range")));
                }
                let r = v0 as u32..v0 as u32 + 1;
                let xform = transform9(&body);
                let kind = if ty == "Sphere" {
                    PrimKind::Sphere { xform }
                } else {
                    let taper = find(&body, "taper").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                    let closed = find(&body, "closed").and_then(Value::as_bool).unwrap_or(true);
                    PrimKind::Tube { xform, taper, closed }
                };
                out.push(Prim { kind, verts: r });
            }
            _ => {
                let vs = find(&body, "vertex").and_then(|v| ints(v).ok()).unwrap_or_default();
                let r = contiguous(&vs, nverts).ok().flatten().unwrap_or(0..0);
                out.push(unknown(r));
            }
        }
    }
    if out.len() as u64 != u64::from(nprims) {
        return Err(GeoError::Count(format!("primitivecount {nprims} but {} primitives present", out.len())));
    }
    Ok(out)
}

pub fn read_geo_json(bytes: &[u8]) -> Result<Detail, GeoError> {
    check_len(bytes.len())?;
    let root: Value = serde_json::from_slice(bytes).map_err(|e| GeoError::Syntax(format!("json: {e}")))?;
    detail_from_value(&root)
}
/// Shared by the ASCII (`read_geo_json`) and binary (`read_geo_bjson`) paths: one `Value` tree → one `Detail`.
pub(crate) fn detail_from_value(root: &Value) -> Result<Detail, GeoError> {
    let top = need_kv(root, "top level")?;
    let np = count_of("pointcount", int_field(&top, "pointcount")?)?;
    let nv = count_of("vertexcount", int_field(&top, "vertexcount")?)?;
    let nq = count_of("primitivecount", int_field(&top, "primitivecount")?)?;

    let mut d = Detail { point_count: np, ..Detail::default() };
    if let Some(t) = find(&top, "topology") {
        let tk = need_kv(t, "topology")?;
        if let Some(pr) = find(&tk, "pointref") {
            let pk = need_kv(pr, "pointref")?;
            let Some(ix) = find(&pk, "indices") else { return syn("pointref without indices") };
            let Value::Array(ix) = ix else { return syn("pointref.indices is not an array") };
            if ix.len() as u64 != u64::from(nv) {
                return Err(GeoError::Count(format!("vertexcount {nv} but pointref has {} indices", ix.len())));
            }
            d.vertex_point.reserve(ix.len());
            for x in ix {
                let i = x.as_i64().ok_or_else(|| GeoError::Syntax("non-integer pointref index".into()))?;
                if i < 0 || i >= i64::from(np) {
                    return Err(GeoError::Index(format!("pointref index {i} >= pointcount {np}")));
                }
                d.vertex_point.push(i as u32);
            }
        }
    }
    if d.vertex_point.len() as u64 != u64::from(nv) {
        return Err(GeoError::Count(format!("vertexcount {nv} but topology provides {}", d.vertex_point.len())));
    }

    if let Some(a) = find(&top, "attributes") {
        let ak = need_kv(a, "attributes")?;
        for (key, class, n) in [
            ("pointattributes", AttrClass::Point, np),
            ("vertexattributes", AttrClass::Vertex, nv),
            ("primitiveattributes", AttrClass::Prim, nq),
            ("globalattributes", AttrClass::Detail, 1),
        ] {
            let Some(list) = find(&ak, key) else { continue };
            let Value::Array(list) = list else { return syn(format!("{key}: expected array")) };
            for e in list {
                if let Some(at) = read_attr(e, class, n as usize)? {
                    d.attrs.push(at);
                }
            }
        }
    }
    if d.attr(AttrClass::Point, "P").is_none() {
        let _ = d.p_mut();
    }
    if let Some(p) = find(&top, "primitives") {
        d.prims = read_prims(p, nv, nq)?;
    } else if nq != 0 {
        return Err(GeoError::Count("primitivecount > 0 but no primitives section".into()));
    }
    for (key, class, n) in [
        ("pointgroups", AttrClass::Point, np),
        ("vertexgroups", AttrClass::Vertex, nv),
        ("primitivegroups", AttrClass::Prim, nq),
    ] {
        if let Some(g) = find(&top, key) {
            read_groups(g, class, n as usize, &mut d.groups)?;
        }
    }
    Ok(d)
}
