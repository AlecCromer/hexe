//! Legacy binary `BgeoV` reader (big-endian twin of `PGEOMETRY`). Clean-room from bytes, oracle = the text twin
//! (`fixtures/gpd-all/expected.geo`). Layout (v2, decoded from `GPD/all.bgeo`):
//! "BgeoV"(5) | version i32 | npoints, nprims, npointgroups, nprimgroups, npointattrib, nvertexattrib, nprimattrib,
//! ndetailattrib (8 x i32) | point dict (`u16 len, name, u16 size, i32 type, default`) | points (`P` 4 x f32, attribs)
//! | vertex dict, prim dict | prims (`u32 type`; `0xffffffff` = Run `u16 count, u32 type`) | detail dict + values | groups
//! | `0xff`. Primitive indices are `u16` while npoints < 65536, else `u32`. Hostile input: every read is bounds-checked
//! and every count is validated against the bytes left before any allocation.
use crate::error::{check_len, count_of};
use crate::json_read::pack;
use crate::{AttrClass, AttrData, Attribute, Detail, GeoError, Group, Prim, PrimKind, TypeInfo};
use serde_json::json;

pub(crate) const MAGIC: &[u8; 5] = b"BgeoV";
const RUN: u32 = 0xffff_ffff;
const MAX_ATTR_SIZE: usize = 4096;
const MAX_NAME: usize = 4096;

struct Cur<'a> {
    b: &'a [u8],
    pos: usize,
}
fn trunc<T>() -> Result<T, GeoError> {
    Err(GeoError::Syntax("truncated legacy bgeo".into()))
}
impl<'a> Cur<'a> {
    fn left(&self) -> usize {
        self.b.len().saturating_sub(self.pos)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], GeoError> {
        let end = self.pos.checked_add(n).ok_or_else(|| GeoError::Syntax("offset overflow".into()))?;
        match self.b.get(self.pos..end) {
            Some(s) => {
                self.pos = end;
                Ok(s)
            }
            None => trunc(),
        }
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], GeoError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, GeoError> {
        Ok(u8::from_be_bytes(self.arr()?))
    }
    fn u16(&mut self) -> Result<u16, GeoError> {
        Ok(u16::from_be_bytes(self.arr()?))
    }
    fn u32(&mut self) -> Result<u32, GeoError> {
        Ok(u32::from_be_bytes(self.arr()?))
    }
    fn i32(&mut self) -> Result<i32, GeoError> {
        Ok(i32::from_be_bytes(self.arr()?))
    }
    fn f32(&mut self) -> Result<f32, GeoError> {
        Ok(f32::from_be_bytes(self.arr()?))
    }
    fn count(&mut self, what: &str) -> Result<usize, GeoError> {
        let v = self.i32()?;
        Ok(count_of(what, i64::from(v))? as usize)
    }
    /// `n` elements of `width` bytes must still fit in the input.
    fn need(&self, what: &str, n: usize, width: usize) -> Result<(), GeoError> {
        if n.saturating_mul(width) > self.left() {
            return Err(GeoError::Count(format!("{what}: {n} elements cannot fit in the remaining bytes")));
        }
        Ok(())
    }
    fn string(&mut self) -> Result<String, GeoError> {
        let n = usize::from(self.u16()?);
        if n > MAX_NAME {
            return Err(GeoError::Count(format!("string length {n}")));
        }
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

enum Kind {
    Float,
    Int,
    Vector,
    Index(Vec<String>),
}
struct Def {
    name: String,
    size: usize,
    kind: Kind,
    flat: Vec<f64>,
}
/// Type enum: 0 float (decoded from the sample); 1 int, 4 vector, 5 index (table = `i32 n, n x string`) are from the
/// HDK enum order, UNVERIFIED (no sample).
fn read_dict(c: &mut Cur<'_>, n: usize) -> Result<Vec<Def>, GeoError> {
    c.need("attribute dictionary", n, 8)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let name = c.string()?;
        let size = usize::from(c.u16()?);
        if size == 0 || size > MAX_ATTR_SIZE {
            return Err(GeoError::Count(format!("attribute `{name}` size {size}")));
        }
        let kind = match c.i32()? {
            0 => Kind::Float,
            1 => Kind::Int,
            4 => Kind::Vector,
            5 => {
                let k = c.count("index table")?;
                c.need("index table", k, 2)?;
                let mut tab = Vec::with_capacity(k);
                for _ in 0..k {
                    tab.push(c.string()?);
                }
                Kind::Index(tab)
            }
            other => return Err(GeoError::Unsupported(format!("legacy bgeo attribute type {other}"))),
        };
        if matches!(kind, Kind::Index(_)) {
            c.i32()?;
        } else {
            c.need("attribute default", size, 4)?;
            c.take(size * 4)?; // default value, unused
        }
        out.push(Def { name, size, kind, flat: Vec::new() });
    }
    Ok(out)
}
fn read_values(c: &mut Cur<'_>, defs: &mut [Def]) -> Result<(), GeoError> {
    for d in defs.iter_mut() {
        match d.kind {
            Kind::Float | Kind::Vector => {
                for _ in 0..d.size {
                    d.flat.push(f64::from(c.f32()?));
                }
            }
            Kind::Int => {
                for _ in 0..d.size {
                    d.flat.push(f64::from(c.i32()?));
                }
            }
            Kind::Index(_) => d.flat.push(f64::from(c.i32()?)),
        }
    }
    Ok(())
}
fn build(defs: Vec<Def>, class: AttrClass, out: &mut Vec<Attribute>) -> Result<(), GeoError> {
    for d in defs {
        let ti = match d.name.as_str() {
            "N" => TypeInfo::Normal,
            "Cd" => TypeInfo::Color,
            _ if matches!(d.kind, Kind::Vector) => TypeInfo::Vector,
            _ => TypeInfo::None,
        };
        let data = match d.kind {
            Kind::Float | Kind::Vector => pack(d.size, true, &d.flat),
            Kind::Int => pack(d.size, false, &d.flat),
            Kind::Index(table) => {
                let mut idx = Vec::with_capacity(d.flat.len());
                for x in d.flat {
                    let i = x as i64;
                    if i < -1 || i >= table.len() as i64 {
                        return Err(GeoError::Index(format!("index attribute `{}` value {i} outside table", d.name)));
                    }
                    idx.push(i as i32);
                }
                AttrData::Str { idx, table }
            }
        };
        out.push(Attribute { name: d.name, class, data, type_info: ti });
    }
    Ok(())
}

/// Primitive type tokens (bit flags). `None` = unmodelled token -> `Unsupported`.
fn type_name(t: u32) -> Option<&'static str> {
    Some(match t {
        0x1 => "Poly",
        0x2 => "NURBCurve",
        0x4 => "BezierCurve",
        0x10 => "Mesh",
        0x20 => "NURBMesh",
        0x40 => "BezierMesh",
        0x1000 => "Circle",
        0x2000 => "Sphere",
        0x4000 => "Tube",
        0x10_0000 => "MetaBall",
        0x20_0000 => "MetaSQuad",
        _ => return None,
    })
}

struct Ctx<'a> {
    c: Cur<'a>,
    np: u32,
    wide: bool,
    vert_point: Vec<u32>,
    vdefs: Vec<Def>,
    qdefs: Vec<Def>,
    prims: Vec<Prim>,
}
impl Ctx<'_> {
    fn index(&mut self) -> Result<u32, GeoError> {
        let p = if self.wide { self.c.u32()? } else { u32::from(self.c.u16()?) };
        if p >= self.np {
            return Err(GeoError::Index(format!("vertex references point {p} (npoints {})", self.np)));
        }
        Ok(p)
    }
    fn vertices(&mut self, n: usize) -> Result<std::ops::Range<u32>, GeoError> {
        self.c.need("vertex list", n, if self.wide { 4 } else { 2 })?;
        let start = self.vert_point.len() as u32;
        for _ in 0..n {
            let p = self.index()?;
            self.vert_point.push(p);
            let mut v = std::mem::take(&mut self.vdefs);
            let r = read_values(&mut self.c, &mut v);
            self.vdefs = v;
            r?;
        }
        Ok(start..self.vert_point.len() as u32)
    }
    /// Basis: `i32 order`; Bezier: `u8 use`, knots only if `use`; NURBS: `u32 flags` (bit 16 = `end`) then knots.
    /// Knot counts follow the text rule (same as the GPD reader).
    fn basis(&mut self, verts: usize, closed: bool, bezier: bool) -> Result<(), GeoError> {
        let order = self.c.i32()?;
        if !(2..=64).contains(&order) {
            return Err(GeoError::Count(format!("basis order {order}")));
        }
        let (order, v) = (i64::from(order), verts as i64);
        let k = if bezier {
            if self.c.u8()? == 0 {
                0
            } else if closed {
                v / (order - 1)
            } else {
                (v - 1) / (order - 1) + 1
            }
        } else {
            let end = (self.c.u32()? >> 16) & 1 != 0;
            match (end, closed) {
                (false, _) => v + order - 2,
                (true, false) => v - order + 2,
                (true, true) => v - order + 3,
            }
        };
        if k < 0 {
            return Err(GeoError::Count(format!("basis knot count {k}")));
        }
        let k = k as usize;
        self.c.need("knots", k, 4)?;
        self.c.take(k * 4)?;
        Ok(())
    }
    /// Profile (trim) block of a NURBS/Bezier mesh: `u32 present` (0 none, 1 present); then 4 x i32 counts (points,
    /// prims, point groups, prim groups), `uvw` points, curve prims (`u32 type, u8 status, i32 n, u8 closed, ...`).
    fn profiles(&mut self) -> Result<(), GeoError> {
        match self.c.u32()? {
            0 => return Ok(()),
            1 => {}
            other => return Err(GeoError::Unsupported(format!("legacy bgeo profile block flag {other}"))),
        }
        let np = self.c.count("profile points")?;
        let nq = self.c.count("profile prims")?;
        let ng = self.c.count("profile point groups")?.saturating_add(self.c.count("profile prim groups")?);
        self.c.need("profile points", np, 12)?;
        self.c.take(np * 12)?;
        let mut made = 0usize;
        while made < nq {
            let mut t = self.c.u32()?;
            let mut reps = 1usize;
            if t == RUN {
                reps = usize::from(self.c.u16()?);
                t = self.c.u32()?;
            }
            if made.saturating_add(reps) > nq {
                return Err(GeoError::Count("profile run exceeds NPrims".into()));
            }
            for _ in 0..reps {
                self.c.u8()?; // status (idle/trim)
                let n = self.c.count("profile vertices")?;
                let closed = self.c.u8()? == b'<';
                match t {
                    1 => {}
                    2 => self.basis(n, closed, false)?,
                    4 => self.basis(n, closed, true)?,
                    other => return Err(GeoError::Unsupported(format!("legacy bgeo profile primitive 0x{other:x}"))),
                }
                self.c.need("profile indices", n, 2)?;
                self.c.take(n * 2)?;
                made += 1;
            }
        }
        if ng != 0 {
            return Err(GeoError::Unsupported("legacy bgeo profile groups".into()));
        }
        Ok(())
    }
    fn unknown(&self, name: &str, r: &std::ops::Range<u32>) -> PrimKind {
        let vs: Vec<u32> = (r.start..r.end).collect();
        PrimKind::Unknown { ty: name.to_string(), json: json!([["type", name], ["vertex", vs]]).to_string() }
    }
    fn xform(&mut self) -> Result<[f32; 9], GeoError> {
        let mut x = [0f32; 9];
        for s in x.iter_mut() {
            *s = self.c.f32()?;
        }
        Ok(x)
    }
    fn prim(&mut self, t: u32) -> Result<(), GeoError> {
        let name = type_name(t)
            .ok_or_else(|| GeoError::Unsupported(format!("legacy bgeo primitive type 0x{t:x} (no model)")))?;
        let (kind, verts) = match t {
            0x1 => {
                let n = self.c.count("Poly vertex count")?;
                let closed = self.c.u8()? == b'<';
                let r = self.vertices(n)?;
                (PrimKind::Poly { closed }, r)
            }
            0x1000 | 0x2000 | 0x4000 | 0x10_0000 | 0x20_0000 => {
                let r = self.vertices(1)?;
                let (mut taper, mut closed) = (1.0, false);
                match t {
                    0x4000 => {
                        taper = self.c.f32()?;
                        closed = self.c.u8()? != 0;
                    }
                    0x10_0000 => {
                        self.c.u8()?; // kernel char
                        self.c.f32()?; // weight
                    }
                    0x20_0000 => {
                        self.c.f32()?;
                        self.c.f32()?;
                        self.c.u8()?;
                        self.c.f32()?;
                    }
                    _ => {}
                }
                let x = self.xform()?;
                match t {
                    0x2000 => (PrimKind::Sphere { xform: x }, r),
                    0x4000 => (PrimKind::Tube { xform: x, taper, closed }, r),
                    _ => (self.unknown(name, &r), r),
                }
            }
            0x2 | 0x4 => {
                let n = self.c.count("curve vertex count")?;
                let closed = self.c.u8()? == b'<';
                self.basis(n, closed, t == 0x4)?;
                let r = self.vertices(n)?;
                (self.unknown(name, &r), r)
            }
            _ => {
                let cols = self.c.count("mesh cols")?;
                let rows = self.c.count("mesh rows")?;
                let flags = self.c.u32()?; // bit31 u-wrap, bit30 v-wrap; other bits are uninitialised garbage
                self.c.u32()?; // connectivity (4 = quad)
                if t != 0x10 {
                    self.basis(cols, (flags >> 31) & 1 != 0, t == 0x40)?;
                    self.basis(rows, (flags >> 30) & 1 != 0, t == 0x40)?;
                }
                let n = cols.checked_mul(rows).ok_or_else(|| GeoError::Count("mesh too large".into()))?;
                let r = self.vertices(n)?;
                if t != 0x10 {
                    self.profiles()?;
                }
                (self.unknown(name, &r), r)
            }
        };
        let mut q = std::mem::take(&mut self.qdefs);
        let r = read_values(&mut self.c, &mut q);
        self.qdefs = q;
        r?;
        self.prims.push(Prim { kind, verts });
        Ok(())
    }
    /// `u16 len, name, i32 nbits, ceil(nbits/32) x u32 words (bit i of word w = element 32w+i)`, then either an ordered
    /// list (`i32 count, count x i32`) + `u8 0`, or the single byte `u8 1` (unordered). List-vs-flag is inferred from ONE
    /// sample (UNVERIFIED): a list is taken iff its count fits and the byte right after the list is 0.
    fn group(&mut self, n: usize) -> Result<(String, Vec<bool>), GeoError> {
        let name = self.c.string()?;
        let nbits = self.c.count("group size")?;
        if nbits != n {
            return Err(GeoError::Count(format!("group `{name}` has {nbits} bits, expected {n}")));
        }
        let words = nbits.div_ceil(32);
        self.c.need("group mask", words, 4)?;
        let mut mask = Vec::with_capacity(nbits);
        for w in 0..words {
            let v = self.c.u32()?;
            for b in 0..32usize {
                if w * 32 + b < nbits {
                    mask.push((v >> b) & 1 != 0);
                }
            }
        }
        let rest = self.c.b.get(self.c.pos..).unwrap_or(&[]);
        let cnt = rest.get(..4).and_then(|s| <[u8; 4]>::try_from(s).ok()).map(|a| u32::from_be_bytes(a) as usize);
        let list = cnt.is_some_and(|k| {
            k <= nbits && k.checked_mul(4).and_then(|x| x.checked_add(4)).is_some_and(|e| rest.get(e) == Some(&0))
        });
        if list {
            let k = self.c.count("ordered list")?;
            self.c.need("ordered list", k, 4)?;
            self.c.take(k * 4)?;
            self.c.u8()?;
        } else if self.c.u8()? != 1 {
            return Err(GeoError::Syntax(format!("group `{name}` tail byte")));
        }
        Ok((name, mask))
    }
}

/// Read a legacy binary `BgeoV` file (version 2 layout). Unmodelled primitive types -> `Unsupported(type)`.
pub fn read_geo_bgeo_legacy(bytes: &[u8]) -> Result<Detail, GeoError> {
    check_len(bytes.len())?;
    let mut c = Cur { b: bytes, pos: 0 };
    if c.take(5)? != MAGIC {
        return Err(GeoError::Syntax("not a legacy BgeoV file".into()));
    }
    let version = c.i32()?;
    let np = c.count("npoints")?;
    let nq = c.count("nprims")?;
    let npg = c.count("npointgroups")?;
    let nqg = c.count("nprimgroups")?;
    let npa = c.count("npointattrib")?;
    let nva = c.count("nvertexattrib")?;
    let nqa = c.count("nprimattrib")?;
    let nda = c.count("ndetailattrib")?;
    if version != 2 {
        return Err(GeoError::Unsupported(format!("legacy bgeo version {version} (only v2 decoded)")));
    }
    c.need("points", np, 16)?;
    c.need("prims", nq, 4)?;
    let mut pdefs = read_dict(&mut c, npa)?;
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(np);
    for _ in 0..np {
        let (x, y, z) = (c.f32()?, c.f32()?, c.f32()?);
        c.f32()?; // w
        pos.push([x, y, z]);
        read_values(&mut c, &mut pdefs)?;
    }
    let vdefs = read_dict(&mut c, nva)?;
    let qdefs = read_dict(&mut c, nqa)?;
    let mut x = Ctx { c, np: np as u32, wide: np >= 65536, vert_point: Vec::new(), vdefs, qdefs, prims: Vec::new() };
    while x.prims.len() < nq {
        let t = x.c.u32()?;
        if t == RUN {
            let n = usize::from(x.c.u16()?);
            let t = x.c.u32()?;
            if x.prims.len().saturating_add(n) > nq {
                return Err(GeoError::Count("Run exceeds nprims".into()));
            }
            for _ in 0..n {
                x.prim(t)?;
            }
        } else {
            x.prim(t)?;
        }
    }
    let mut ddefs = read_dict(&mut x.c, nda)?;
    read_values(&mut x.c, &mut ddefs)?;
    let mut groups = Vec::new();
    for _ in 0..npg {
        let (name, mask) = x.group(np)?;
        groups.push(Group { name, class: AttrClass::Point, mask });
    }
    for _ in 0..nqg {
        let (name, mask) = x.group(nq)?;
        groups.push(Group { name, class: AttrClass::Prim, mask });
    }
    let mut d =
        Detail { point_count: np as u32, vertex_point: x.vert_point, prims: x.prims, groups, ..Detail::default() };
    d.attrs.push(Attribute {
        name: "P".into(),
        class: AttrClass::Point,
        data: AttrData::V3(pos),
        type_info: TypeInfo::Point,
    });
    build(pdefs, AttrClass::Point, &mut d.attrs)?;
    build(x.vdefs, AttrClass::Vertex, &mut d.attrs)?;
    build(x.qdefs, AttrClass::Prim, &mut d.attrs)?;
    build(ddefs, AttrClass::Detail, &mut d.attrs)?;
    Ok(d)
}
