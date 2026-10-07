//! Stanford PLY reader (ascii + binary_little_endian) and ascii writer (`geo-read-ply`, `geo-write-ply`).
//! Read: element `vertex` x y z -> `P`; nx ny nz -> point `N` (Normal); red green blue -> point `Cd` (integer types / 255, float
//! types as-is); s t (or u v, texture_u texture_v) -> point `uv` (V3, z = 0); element `face` list `vertex_indices` (or
//! `vertex_index`, any count/item type) -> closed polys. Everything else (extra properties / elements) is recorded in
//! `skipped_attribs`, never silently dropped. `binary_big_endian` -> `Unsupported`. Faces with < 3 indices, indices past the
//! vertex count, truncated bodies, and counts the body cannot possibly hold -> Err before any allocation.
//! Write (ascii only): `P`, point `N`/`uv`/`Cd` (Cd quantised to uchar), closed polys with <= 255 corners.
//! Vertex-class attributes are NOT written (PLY has no per-corner data); other prim kinds -> `Unsupported`.
use crate::error::{MAX_ELEMS, check_len, count_of};
use crate::*;
use std::fmt::Write as _;

fn syn<T>(s: impl Into<String>) -> Result<T, GeoError> {
    Err(GeoError::Syntax(s.into()))
}
#[derive(Clone, Copy, PartialEq)]
enum Ty {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}
impl Ty {
    fn parse(s: &str) -> Option<Ty> {
        Some(match s {
            "char" | "int8" => Ty::I8,
            "uchar" | "uint8" => Ty::U8,
            "short" | "int16" => Ty::I16,
            "ushort" | "uint16" => Ty::U16,
            "int" | "int32" => Ty::I32,
            "uint" | "uint32" => Ty::U32,
            "float" | "float32" => Ty::F32,
            "double" | "float64" => Ty::F64,
            _ => return None,
        })
    }
    fn size(self) -> usize {
        match self {
            Ty::I8 | Ty::U8 => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::I32 | Ty::U32 | Ty::F32 => 4,
            Ty::F64 => 8,
        }
    }
    fn is_float(self) -> bool {
        matches!(self, Ty::F32 | Ty::F64)
    }
    fn range(self) -> (f64, f64) {
        match self {
            Ty::I8 => (-128.0, 127.0),
            Ty::U8 => (0.0, 255.0),
            Ty::I16 => (-32768.0, 32767.0),
            Ty::U16 => (0.0, 65535.0),
            Ty::I32 => (i32::MIN as f64, i32::MAX as f64),
            Ty::U32 => (0.0, u32::MAX as f64),
            _ => (f64::MIN, f64::MAX),
        }
    }
}
struct Prop {
    name: String,
    ty: Ty,
    /// `Some(count type)` for `property list`.
    list: Option<Ty>,
}
struct Element {
    name: String,
    count: u32,
    props: Vec<Prop>,
}
enum Src<'a> {
    Ascii(std::str::SplitAsciiWhitespace<'a>),
    Bin { data: &'a [u8], pos: usize },
}
impl Src<'_> {
    fn read(&mut self, ty: Ty) -> Result<f64, GeoError> {
        let v = match self {
            Src::Ascii(it) => {
                let tok = it.next().ok_or_else(|| GeoError::Syntax("truncated ascii body".into()))?;
                tok.parse::<f64>().map_err(|_| GeoError::Syntax(format!("bad number `{tok}`")))?
            }
            Src::Bin { data, pos } => {
                let end = pos.checked_add(ty.size()).ok_or_else(|| GeoError::Syntax("offset overflow".into()))?;
                let b = data.get(*pos..end).ok_or_else(|| GeoError::Syntax("truncated binary body".into()))?;
                *pos = end;
                let mut a = [0u8; 8];
                a[..b.len()].copy_from_slice(b);
                match ty {
                    Ty::I8 => f64::from(i8::from_le_bytes([a[0]])),
                    Ty::U8 => f64::from(a[0]),
                    Ty::I16 => f64::from(i16::from_le_bytes([a[0], a[1]])),
                    Ty::U16 => f64::from(u16::from_le_bytes([a[0], a[1]])),
                    Ty::I32 => f64::from(i32::from_le_bytes([a[0], a[1], a[2], a[3]])),
                    Ty::U32 => f64::from(u32::from_le_bytes([a[0], a[1], a[2], a[3]])),
                    Ty::F32 => f64::from(f32::from_le_bytes([a[0], a[1], a[2], a[3]])),
                    Ty::F64 => f64::from_le_bytes(a),
                }
            }
        };
        let (lo, hi) = ty.range();
        if !v.is_finite() || v < lo || v > hi || (!ty.is_float() && v.fract() != 0.0) {
            return syn(format!("value {v} invalid for its declared type"));
        }
        Ok(v)
    }
    /// Bytes (binary) / bytes (ascii) still unread.
    fn left(&self, body_len: usize) -> usize {
        match self {
            Src::Ascii(it) => it.clone().map(|t| t.len() + 1).sum::<usize>().min(body_len),
            Src::Bin { data, pos } => data.len().saturating_sub(*pos),
        }
    }
    /// Read one property into `out` (cleared first): 1 value for scalars, the items for lists.
    fn prop(&mut self, p: &Prop, out: &mut Vec<f64>) -> Result<(), GeoError> {
        out.clear();
        let Some(cty) = p.list else {
            out.push(self.read(p.ty)?);
            return Ok(());
        };
        let n = self.read(cty)? as usize;
        if let Src::Bin { data, pos } = self {
            let need = n.checked_mul(p.ty.size()).ok_or_else(|| GeoError::Count("list size overflow".into()))?;
            if need > data.len().saturating_sub(*pos) {
                return Err(GeoError::Count(format!("list of {n} items exceeds the remaining data")));
            }
        }
        for _ in 0..n {
            out.push(self.read(p.ty)?);
        }
        Ok(())
    }
}
const MAX_HEADER_ELEMENTS: usize = 64;
const MAX_HEADER_PROPS: usize = 256;
fn parse_header(head: &str) -> Result<(bool, Vec<Element>), GeoError> {
    let mut lines = head.lines().map(str::trim);
    if lines.next() != Some("ply") {
        return syn("missing `ply` magic");
    }
    let (mut ascii, mut els): (Option<bool>, Vec<Element>) = (None, Vec::new());
    for l in lines {
        let w: Vec<&str> = l.split_ascii_whitespace().collect();
        match w.as_slice() {
            [] | ["comment", ..] | ["obj_info", ..] => {}
            ["format", f, _version] => {
                ascii = Some(match *f {
                    "ascii" => true,
                    "binary_little_endian" => false,
                    "binary_big_endian" => return Err(GeoError::Unsupported("PLY binary_big_endian".into())),
                    other => return syn(format!("unknown PLY format `{other}`")),
                });
            }
            ["element", name, count] => {
                if els.len() >= MAX_HEADER_ELEMENTS {
                    return Err(GeoError::Count("too many PLY elements".into()));
                }
                let n: i64 = count.parse().map_err(|_| GeoError::Syntax(format!("bad element count `{count}`")))?;
                els.push(Element { name: name.to_string(), count: count_of("element count", n)?, props: Vec::new() });
            }
            ["property", "list", c, t, name] => {
                let (c, t) = (Ty::parse(c), Ty::parse(t));
                let (Some(c), Some(t)) = (c, t) else { return syn(format!("bad list property `{name}`")) };
                push_prop(&mut els, Prop { name: name.to_string(), ty: t, list: Some(c) })?;
            }
            ["property", t, name] => {
                let Some(t) = Ty::parse(t) else { return syn(format!("bad property type `{t}`")) };
                push_prop(&mut els, Prop { name: name.to_string(), ty: t, list: None })?;
            }
            _ => return syn(format!("bad header line `{l}`")),
        }
    }
    Ok((ascii.ok_or_else(|| GeoError::Syntax("missing `format` line".into()))?, els))
}
fn push_prop(els: &mut [Element], p: Prop) -> Result<(), GeoError> {
    let total: usize = els.iter().map(|e| e.props.len()).sum();
    if total >= MAX_HEADER_PROPS {
        return Err(GeoError::Count("too many PLY properties".into()));
    }
    els.last_mut().ok_or_else(|| GeoError::Syntax("property before any element".into()))?.props.push(p);
    Ok(())
}
fn find(e: &Element, names: &[&str]) -> Option<usize> {
    e.props.iter().position(|p| p.list.is_none() && names.contains(&p.name.as_str()))
}
/// Parse a PLY file.
pub fn read_ply(bytes: &[u8]) -> Result<Detail, GeoError> {
    check_len(bytes.len())?;
    const END: &[u8] = b"end_header";
    let at = bytes
        .windows(END.len())
        .position(|w| w == END)
        .ok_or_else(|| GeoError::Syntax("missing `end_header`".into()))?;
    let nl = bytes[at..].iter().position(|&b| b == b'\n').ok_or_else(|| GeoError::Syntax("truncated header".into()))?;
    let head = String::from_utf8_lossy(&bytes[..at]);
    let (ascii, els) = parse_header(&head)?;
    let body = &bytes[at + nl + 1..];
    let mut src = if ascii {
        Src::Ascii(
            std::str::from_utf8(body)
                .map_err(|_| GeoError::Syntax("ascii body is not UTF-8".into()))?
                .split_ascii_whitespace(),
        )
    } else {
        Src::Bin { data: body, pos: 0 }
    };
    let vcount = els.iter().find(|e| e.name == "vertex").map_or(0, |e| e.count);
    let mut d = Detail::default();
    let (mut p, mut n, mut cd, mut uv) =
        (Vec::<[f32; 3]>::new(), Vec::<[f32; 3]>::new(), Vec::<[f32; 3]>::new(), Vec::<[f32; 3]>::new());
    let mut buf: Vec<f64> = Vec::new();
    let mut have_vertex = false;
    for e in &els {
        // Every instance consumes >= 1 token / `min` bytes: a count the body cannot hold is rejected before allocating.
        let min: usize = e
            .props
            .iter()
            .map(|q| q.list.map_or(if ascii { 1 } else { q.ty.size() }, |c| if ascii { 1 } else { c.size() }))
            .sum();
        if min > 0 {
            let left = src.left(body.len());
            let need = (e.count as usize).saturating_mul(min).saturating_mul(if ascii { 2 } else { 1 });
            if need > left.saturating_add(if ascii { 1 } else { 0 }) {
                return Err(GeoError::Count(format!(
                    "element `{}` claims {} instances, body too short",
                    e.name, e.count
                )));
            }
        }
        match e.name.as_str() {
            "vertex" => {
                have_vertex = true;
                let (ix, iy, iz) = (find(e, &["x"]), find(e, &["y"]), find(e, &["z"]));
                let (Some(ix), Some(iy), Some(iz)) = (ix, iy, iz) else { return syn("vertex element lacks x/y/z") };
                let nrm = (find(e, &["nx"]), find(e, &["ny"]), find(e, &["nz"]));
                let col = (find(e, &["red"]), find(e, &["green"]), find(e, &["blue"]));
                let tex = (find(e, &["s", "u", "texture_u"]), find(e, &["t", "v", "texture_v"]));
                let nrm = if let (Some(a), Some(b), Some(c)) = nrm { Some([a, b, c]) } else { None };
                let col = if let (Some(a), Some(b), Some(c)) = col { Some([a, b, c]) } else { None };
                let tex = if let (Some(a), Some(b)) = tex { Some([a, b]) } else { None };
                let col_scale = col.is_some_and(|c| e.props.get(c[0]).is_some_and(|q| !q.ty.is_float()));
                let used: Vec<usize> =
                    [Some(vec![ix, iy, iz]), nrm.map(|v| v.to_vec()), col.map(|v| v.to_vec()), tex.map(|v| v.to_vec())]
                        .into_iter()
                        .flatten()
                        .flatten()
                        .collect();
                for q in e.props.iter().enumerate().filter(|(i, _)| !used.contains(i)).map(|(_, q)| q) {
                    d.skipped_attribs.push((AttrClass::Point, q.name.clone(), "ply property".into()));
                }
                let mut row = vec![0.0f64; e.props.len()];
                for _ in 0..e.count {
                    for (pi, q) in e.props.iter().enumerate() {
                        src.prop(q, &mut buf)?;
                        if let (Some(v), None) = (buf.first(), q.list)
                            && let Some(r) = row.get_mut(pi)
                        {
                            *r = *v;
                        }
                    }
                    let g = |i: usize| row.get(i).copied().unwrap_or(0.0) as f32;
                    p.push([g(ix), g(iy), g(iz)]);
                    if let Some(c) = nrm {
                        n.push([g(c[0]), g(c[1]), g(c[2])]);
                    }
                    if let Some(c) = col {
                        let k = if col_scale { 255.0 } else { 1.0 };
                        cd.push([g(c[0]) / k, g(c[1]) / k, g(c[2]) / k]);
                    }
                    if let Some(c) = tex {
                        uv.push([g(c[0]), g(c[1]), 0.0]);
                    }
                }
            }
            "face" => {
                let vi = e
                    .props
                    .iter()
                    .position(|q| q.list.is_some() && (q.name == "vertex_indices" || q.name == "vertex_index"));
                for q in e.props.iter().filter(|q| q.name != "vertex_indices" && q.name != "vertex_index") {
                    d.skipped_attribs.push((AttrClass::Prim, q.name.clone(), "ply property".into()));
                }
                for fi in 0..e.count {
                    let start = d.vertex_point.len();
                    for (pi, q) in e.props.iter().enumerate() {
                        src.prop(q, &mut buf)?;
                        if Some(pi) != vi {
                            continue;
                        }
                        if buf.len() < 3 {
                            return syn(format!("face {fi} has {} indices", buf.len()));
                        }
                        for &i in &buf {
                            if i < 0.0 || i >= f64::from(vcount) {
                                return Err(GeoError::Index(format!(
                                    "face {fi}: vertex index {i} out of range (0..{vcount})"
                                )));
                            }
                            if d.vertex_point.len() as u64 >= MAX_ELEMS {
                                return Err(GeoError::Count("too many face corners".into()));
                            }
                            d.vertex_point.push(i as u32);
                        }
                    }
                    if vi.is_some() {
                        d.prims.push(Prim {
                            kind: PrimKind::Poly { closed: true },
                            verts: start as u32..d.vertex_point.len() as u32,
                        });
                    }
                }
            }
            other => {
                d.skipped_attribs.push((AttrClass::Detail, other.to_string(), "ply element".into()));
                for _ in 0..e.count {
                    for q in &e.props {
                        src.prop(q, &mut buf)?;
                    }
                }
            }
        }
    }
    if !have_vertex && !d.prims.is_empty() {
        return syn("faces without a vertex element");
    }
    d.point_count = u32::try_from(p.len()).map_err(|_| GeoError::Count("points".into()))?;
    d.attrs.push(Attribute {
        name: "P".into(),
        class: AttrClass::Point,
        data: AttrData::V3(p),
        type_info: TypeInfo::Point,
    });
    for (name, v, ti) in [("N", n, TypeInfo::Normal), ("Cd", cd, TypeInfo::Color), ("uv", uv, TypeInfo::None)] {
        if !v.is_empty() {
            d.attrs.push(Attribute {
                name: name.into(),
                class: AttrClass::Point,
                data: AttrData::V3(v),
                type_info: ti,
            });
        }
    }
    d.canonicalize();
    Ok(d)
}
/// Serialise to ascii PLY.
pub fn write_ply(d: &Detail) -> Result<Vec<u8>, GeoError> {
    let p = d.p();
    if p.len() != d.point_count as usize {
        return Err(GeoError::Count(format!("P has {} entries for {} points", p.len(), d.point_count)));
    }
    let extra = |name: &str| crate::obj::v3_of(d, AttrClass::Point, name).filter(|v| v.len() == p.len());
    let (n, cd, uv) = (extra("N"), extra("Cd"), extra("uv"));
    let mut out = String::from("ply\nformat ascii 1.0\ncomment hexe\n");
    let _ = writeln!(out, "element vertex {}\nproperty float x\nproperty float y\nproperty float z", p.len());
    if n.is_some() {
        out.push_str("property float nx\nproperty float ny\nproperty float nz\n");
    }
    if cd.is_some() {
        out.push_str("property uchar red\nproperty uchar green\nproperty uchar blue\n");
    }
    if uv.is_some() {
        out.push_str("property float s\nproperty float t\n");
    }
    let _ = writeln!(out, "element face {}\nproperty list uchar int vertex_indices\nend_header", d.prims.len());
    for (i, q) in p.iter().enumerate() {
        let mut row = vec![q[0], q[1], q[2]];
        if let Some(v) = &n {
            row.extend(v[i]);
        }
        if row.iter().chain(uv.iter().flat_map(|v| &v[i][..2])).any(|c| !c.is_finite()) {
            return syn(format!("point {i}: non-finite value cannot be written"));
        }
        let mut line = row.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        if let Some(v) = &cd {
            line.extend(v[i].iter().map(|c| ((c.clamp(0.0, 1.0) * 255.0).round() as u8).to_string()));
        }
        if let Some(v) = &uv {
            line.extend(v[i][..2].iter().map(|c| c.to_string()));
        }
        out.push_str(&line.join(" "));
        out.push('\n');
    }
    for (pi, prim) in d.prims.iter().enumerate() {
        if !matches!(prim.kind, PrimKind::Poly { closed: true }) {
            return Err(GeoError::Unsupported(format!("prim {pi}: only closed polygons can be written to PLY")));
        }
        let n = prim.verts.len();
        if n > 255 {
            return Err(GeoError::Unsupported(format!("prim {pi}: {n} corners exceed the uchar list count")));
        }
        let _ = write!(out, "{n}");
        for vid in prim.verts.clone() {
            let pid = d.vertex_point.get(vid as usize).copied().filter(|&q| q < d.point_count);
            let pid = pid.ok_or_else(|| GeoError::Index(format!("prim {pi}: vertex {vid} has no valid point")))?;
            let _ = write!(out, " {pid}");
        }
        out.push('\n');
    }
    Ok(out.into_bytes())
}
