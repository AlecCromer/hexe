//! Legacy `PGEOMETRY V2` ASCII reader (the format of Houdini's shipped `GPD/all.geo`). Token-based, fully bounds-checked.
//! Non-modelled primitives (curves, meshes, circles, metaballs, ...) are kept as `PrimKind::Unknown` with their raw tokens.

use crate::error::{check_len, count_of};
use crate::json_read::pack;
use crate::nurbs::basis_from_stored;
use crate::{AttrClass, AttrData, Attribute, Basis, Detail, GeoError, Group, Prim, PrimKind, TypeInfo};
use serde_json::json;

struct Toks<'a> {
    t: Vec<&'a str>,
    pos: usize,
}

fn tokenize(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in s.char_indices() {
        let special = matches!(c, '(' | ')' | '[' | ']');
        if c.is_whitespace() || special {
            if let Some(st) = start.take() {
                out.push(&s[st..i]);
            }
            if special {
                out.push(&s[i..i + 1]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

fn trunc<T>() -> Result<T, GeoError> {
    Err(GeoError::Syntax("truncated GPD file".into()))
}

impl<'a> Toks<'a> {
    fn next(&mut self) -> Result<&'a str, GeoError> {
        match self.t.get(self.pos) {
            Some(s) => {
                self.pos += 1;
                Ok(s)
            }
            None => trunc(),
        }
    }
    fn peek(&self) -> Option<&'a str> {
        self.t.get(self.pos).copied()
    }
    fn expect(&mut self, want: &str) -> Result<(), GeoError> {
        let g = self.next()?;
        if g == want { Ok(()) } else { Err(GeoError::Syntax(format!("expected `{want}`, found `{g}`"))) }
    }
    fn num(&mut self) -> Result<f64, GeoError> {
        let s = self.next()?;
        s.parse::<f64>().map_err(|_| GeoError::Syntax(format!("`{s}` is not a number")))
    }
    fn int(&mut self) -> Result<i64, GeoError> {
        let s = self.next()?;
        s.parse::<i64>().map_err(|_| GeoError::Syntax(format!("`{s}` is not an integer")))
    }
    /// Non-negative count that cannot exceed the tokens still available (stops allocation amplification).
    fn count(&mut self, what: &str) -> Result<usize, GeoError> {
        let v = self.int()?;
        let c = count_of(what, v)? as usize;
        Ok(c)
    }
    fn left(&self) -> usize {
        self.t.len().saturating_sub(self.pos)
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

fn read_dict(t: &mut Toks<'_>, n: usize, header: &str) -> Result<Vec<Def>, GeoError> {
    if n == 0 {
        return Ok(Vec::new());
    }
    t.expect(header)?;
    if n > t.left() {
        return Err(GeoError::Count(format!("{header}: {n} attributes cannot fit")));
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let name = t.next()?.to_string();
        let size = t.count("attribute size")?;
        if size == 0 || size > 4096 {
            return Err(GeoError::Count(format!("attribute `{name}` size {size}")));
        }
        let ty = t.next()?;
        let kind = match ty {
            "float" => Kind::Float,
            "int" => Kind::Int,
            "vector" => Kind::Vector,
            "index" => {
                let k = t.count("index table")?;
                if k > t.left() {
                    return Err(GeoError::Count("index table larger than file".into()));
                }
                let mut tab = Vec::with_capacity(k);
                for _ in 0..k {
                    tab.push(t.next()?.to_string());
                }
                Kind::Index(tab)
            }
            other => return Err(GeoError::Unsupported(format!("GPD attribute type `{other}`"))),
        };
        if !matches!(kind, Kind::Index(_)) {
            for _ in 0..size {
                t.num()?; // default value, unused
            }
        }
        out.push(Def { name, size, kind, flat: Vec::new() });
    }
    Ok(out)
}

/// `( v v v ... )` or `[ ... ]` data block for a dictionary.
fn read_block(t: &mut Toks<'_>, defs: &mut [Def], open: &str, close: &str) -> Result<(), GeoError> {
    if defs.is_empty() {
        return Ok(());
    }
    t.expect(open)?;
    for d in defs.iter_mut() {
        let n = if matches!(d.kind, Kind::Index(_)) { 1 } else { d.size };
        for _ in 0..n {
            d.flat.push(t.num()?);
        }
    }
    t.expect(close)
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

struct Ctx<'a> {
    t: Toks<'a>,
    np: u32,
    vert_point: Vec<u32>,
    vdefs: Vec<Def>,
    qdefs: Vec<Def>,
    prims: Vec<Prim>,
}

impl<'a> Ctx<'a> {
    fn vertex(&mut self) -> Result<u32, GeoError> {
        let p = self.t.int()?;
        if p < 0 || p >= i64::from(self.np) {
            return Err(GeoError::Index(format!("vertex references point {p} (NPoints {})", self.np)));
        }
        self.vert_point.push(p as u32);
        // split borrow: take defs out while reading the block
        let mut v = std::mem::take(&mut self.vdefs);
        let r = read_block(&mut self.t, &mut v, "(", ")");
        self.vdefs = v;
        r?;
        Ok(self.vert_point.len() as u32 - 1)
    }
    fn vertices(&mut self, n: usize) -> Result<std::ops::Range<u32>, GeoError> {
        if n > self.t.left() {
            return Err(GeoError::Count("vertex list longer than file".into()));
        }
        let start = self.vert_point.len() as u32;
        for _ in 0..n {
            self.vertex()?;
        }
        Ok(start..self.vert_point.len() as u32)
    }
    fn basis(&mut self, verts: i64, closed: bool, bezier: bool) -> Result<Basis, GeoError> {
        self.t.expect("Basis")?;
        let order = self.t.int()?;
        if !(2..=64).contains(&order) {
            return Err(GeoError::Count(format!("basis order {order}")));
        }
        let mut end_flag = true;
        let k = if bezier {
            if self.t.peek() == Some("use") {
                self.t.next()?;
                if closed { verts / (order - 1) } else { (verts - 1) / (order - 1) + 1 }
            } else {
                0
            }
        } else {
            let end = self.t.next()?;
            end_flag = end == "end";
            match (end, closed) {
                ("noend", _) => verts + order - 2,
                ("end", false) => verts - order + 2,
                ("end", true) => verts - order + 3,
                _ => return Err(GeoError::Syntax(format!("basis end flag `{end}`"))),
            }
        };
        if k < 0 || k as usize > self.t.left() {
            return Err(GeoError::Count(format!("basis knot count {k}")));
        }
        let mut stored = Vec::with_capacity(k as usize);
        for _ in 0..k {
            stored.push(self.t.num()? as f32);
        }
        Ok(basis_from_stored(order as u32, end_flag, bezier, closed, verts.max(0) as usize, &stored))
    }
    fn profiles(&mut self) -> Result<(), GeoError> {
        self.t.expect("Profiles:")?;
        if self.t.peek() == Some("none") {
            self.t.next()?;
            return Ok(());
        }
        self.t.expect("NPoints")?;
        let np = self.t.count("profile NPoints")?;
        self.t.expect("NPrims")?;
        let nq = self.t.count("profile NPrims")?;
        let mut nloops = 0;
        if self.t.peek() == Some("NLoops") {
            self.t.next()?;
            nloops = self.t.count("NLoops")?;
        }
        self.t.expect("NPointGroups")?;
        let g1 = self.t.count("profile point groups")?;
        self.t.expect("NPrimGroups")?;
        let g2 = self.t.count("profile prim groups")?;
        if self.t.peek() == Some("TrimLevel") {
            self.t.next()?;
            self.t.int()?;
        }
        if np.saturating_mul(3) > self.t.left() {
            return Err(GeoError::Count("profile points exceed file".into()));
        }
        for _ in 0..np.saturating_mul(3) {
            self.t.num()?;
        }
        let mut made = 0;
        while made < nq {
            let key = self.t.next()?;
            let (key, reps) = if key == "Run" {
                let n = self.t.count("profile run")?;
                (self.t.next()?, n)
            } else {
                (key, 1)
            };
            if made + reps > nq {
                return Err(GeoError::Count("profile run exceeds NPrims".into()));
            }
            for _ in 0..reps {
                self.t.next()?; // profile name
                let n = self.t.count("profile vertices")?;
                let flag = self.t.next()?;
                let closed = flag == "<";
                match key {
                    "Poly" => {}
                    "NURBCurve" => {
                        self.basis(n as i64, closed, false)?;
                    }
                    "BezierCurve" => {
                        self.basis(n as i64, closed, true)?;
                    }
                    other => return Err(GeoError::Unsupported(format!("profile primitive `{other}`"))),
                }
                if n > self.t.left() {
                    return Err(GeoError::Count("profile vertex list exceeds file".into()));
                }
                for _ in 0..n {
                    self.t.int()?;
                }
                made += 1;
            }
        }
        for _ in 0..(g1 + g2) {
            self.group_tokens()?;
        }
        for _ in 0..nloops {
            self.t.expect("TrimRegion")?;
            if self.t.peek() == Some("natural") {
                self.t.next()?;
            }
            let n = self.t.count("trim profiles")?;
            for _ in 0..n.saturating_mul(3) {
                self.t.num()?;
            }
        }
        Ok(())
    }
    /// Skip a group record (profile groups are discarded).
    fn group_tokens(&mut self) -> Result<(), GeoError> {
        self.group(u32::MAX as usize).map(|_| ())
    }
    /// `name type nbits mask [count elems..]`. `expect_n == u32::MAX` skips the size check.
    fn group(&mut self, expect_n: usize) -> Result<(String, Vec<bool>), GeoError> {
        let name = self.t.next()?.to_string();
        let ty = self.t.next()?;
        if ty != "unordered" && ty != "ordered" {
            return Err(GeoError::Syntax(format!("group type `{ty}`")));
        }
        let n = self.t.count("group size")?;
        if expect_n != u32::MAX as usize && n != expect_n {
            return Err(GeoError::Count(format!("group `{name}` has {n} bits, expected {expect_n}")));
        }
        let mut mask = Vec::new();
        while mask.len() < n {
            let tok = self.t.next()?;
            for c in tok.bytes() {
                match c {
                    b'0' => mask.push(false),
                    b'1' => mask.push(true),
                    _ => return Err(GeoError::Syntax(format!("group `{name}` bitmask has `{}`", c as char))),
                }
            }
            if mask.len() > n {
                return Err(GeoError::Count(format!("group `{name}` bitmask longer than {n}")));
            }
        }
        if ty == "ordered" {
            let k = self.t.count("ordered group list")?;
            if k > self.t.left() {
                return Err(GeoError::Count("ordered list exceeds file".into()));
            }
            for _ in 0..k {
                self.t.next()?;
            }
        }
        Ok((name, mask))
    }

    /// Parse one primitive body (key already consumed).
    fn prim(&mut self, key: &str) -> Result<(), GeoError> {
        let tok_start = self.t.pos;
        let (kind, verts) = match key {
            "Poly" => {
                let n = self.t.count("Poly vertex count")?;
                let flag = self.t.next()?;
                if flag != "<" && flag != ":" {
                    return Err(GeoError::Syntax(format!("Poly open/close flag `{flag}`")));
                }
                let r = self.vertices(n)?;
                (PrimKind::Poly { closed: flag == "<" }, r)
            }
            "Sphere" | "Tube" | "Circle" | "MetaBall" | "MetaSQuad" => {
                let r = self.vertices(1)?;
                let mut taper = 1.0;
                let mut closed = false;
                match key {
                    "Tube" => {
                        taper = self.t.num()? as f32;
                        closed = self.t.next()? == "closed";
                    }
                    "MetaBall" => {
                        self.t.next()?;
                        self.t.num()?;
                    }
                    "MetaSQuad" => {
                        self.t.num()?;
                        self.t.num()?;
                        self.t.next()?;
                        self.t.num()?;
                    }
                    _ => {}
                }
                let mut x = [0f32; 9];
                for s in x.iter_mut() {
                    *s = self.t.num()? as f32;
                }
                match key {
                    "Sphere" => (PrimKind::Sphere { xform: x }, r),
                    "Tube" => (PrimKind::Tube { xform: x, taper, closed }, r),
                    _ => (self.unknown(key, tok_start, &r), r),
                }
            }
            "NURBCurve" | "BezierCurve" => {
                let n = self.t.count("curve vertex count")?;
                let closed = self.t.next()? == "<";
                let basis = self.basis(n as i64, closed, key == "BezierCurve")?;
                let r = self.vertices(n)?;
                let kind = if key == "BezierCurve" {
                    PrimKind::BezierCurve { basis, closed }
                } else {
                    PrimKind::NurbsCurve { basis, closed }
                };
                (kind, r)
            }
            "Mesh" | "NURBMesh" | "BezierMesh" => {
                let cols = self.t.count("mesh cols")?;
                let rows = self.t.count("mesh rows")?;
                let uw = self.t.next()? == "wrap";
                let vw = self.t.next()? == "wrap";
                self.t.next()?; // connectivity
                let bases = if key != "Mesh" {
                    let ub = self.basis(cols as i64, uw, key == "BezierMesh")?;
                    let vb = self.basis(rows as i64, vw, key == "BezierMesh")?;
                    Some((ub, vb))
                } else {
                    None
                };
                let n = cols.checked_mul(rows).ok_or_else(|| GeoError::Count("mesh too large".into()))?;
                let r = self.vertices(n)?;
                if key != "Mesh" {
                    self.profiles()?;
                }
                let (c32, r32) = (cols as u32, rows as u32);
                match bases {
                    Some((ubasis, vbasis)) if key == "BezierMesh" => {
                        (PrimKind::BezierSurf { cols: c32, rows: r32, ubasis, vbasis, uwrap: uw, vwrap: vw }, r)
                    }
                    Some((ubasis, vbasis)) => {
                        (PrimKind::NurbsSurf { cols: c32, rows: r32, ubasis, vbasis, uwrap: uw, vwrap: vw }, r)
                    }
                    None => (self.unknown(key, tok_start, &r), r),
                }
            }
            other => return Err(GeoError::Unsupported(format!("GPD primitive `{other}`"))),
        };
        let mut q = std::mem::take(&mut self.qdefs);
        let r = read_block(&mut self.t, &mut q, "[", "]");
        self.qdefs = q;
        r?;
        self.prims.push(Prim { kind, verts });
        Ok(())
    }
    fn unknown(&self, key: &str, from: usize, r: &std::ops::Range<u32>) -> PrimKind {
        let toks: Vec<&str> = self.t.t.get(from..self.t.pos).unwrap_or(&[]).to_vec();
        let vs: Vec<u32> = (r.start..r.end).collect();
        let entry = json!([["type", key], ["vertex", vs, "legacy_tokens", toks]]);
        PrimKind::Unknown { ty: key.to_string(), json: entry.to_string() }
    }
}

pub fn read_geo_gpd(bytes: &[u8]) -> Result<Detail, GeoError> {
    check_len(bytes.len())?;
    let text = std::str::from_utf8(bytes).map_err(|_| GeoError::Syntax("GPD is not UTF-8 text".into()))?;
    let mut t = Toks { t: tokenize(text), pos: 0 };
    t.expect("PGEOMETRY")?;
    t.next()?; // version (V2)
    t.expect("NPoints")?;
    let np = t.count("NPoints")?;
    t.expect("NPrims")?;
    let nq = t.count("NPrims")?;
    t.expect("NPointGroups")?;
    let npg = t.count("NPointGroups")?;
    t.expect("NPrimGroups")?;
    let nqg = t.count("NPrimGroups")?;
    t.expect("NPointAttrib")?;
    let npa = t.count("NPointAttrib")?;
    t.expect("NVertexAttrib")?;
    let nva = t.count("NVertexAttrib")?;
    t.expect("NPrimAttrib")?;
    let nqa = t.count("NPrimAttrib")?;
    t.expect("NAttrib")?;
    let nda = t.count("NAttrib")?;
    // every point needs >= 3 tokens; every prim >= 2: reject absurd headers before allocating
    if np.saturating_mul(3) > t.left() || nq.saturating_mul(2) > t.left() {
        return Err(GeoError::Count("header counts exceed file size".into()));
    }

    let mut pdefs = read_dict(&mut t, npa, "PointAttrib")?;
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(np);
    let mut pw: Vec<f32> = Vec::with_capacity(np);
    for _ in 0..np {
        let (x, y, z) = (t.num()?, t.num()?, t.num()?);
        pw.push(t.num()? as f32); // w: kept as `Pw` only when a NURBS/Bezier prim exists and some w != 1
        pos.push([x as f32, y as f32, z as f32]);
        read_block(&mut t, &mut pdefs, "(", ")")?;
    }
    let vdefs = read_dict(&mut t, nva, "VertexAttrib")?;
    let qdefs = read_dict(&mut t, nqa, "PrimitiveAttrib")?;
    let mut c = Ctx { t, np: np as u32, vert_point: Vec::new(), vdefs, qdefs, prims: Vec::new() };
    while c.prims.len() < nq {
        let key = c.t.next()?;
        if key == "Run" {
            let n = c.t.count("Run length")?;
            let k = c.t.next()?;
            if c.prims.len().saturating_add(n) > nq {
                return Err(GeoError::Count("Run exceeds NPrims".into()));
            }
            for _ in 0..n {
                c.prim(k)?;
            }
        } else {
            c.prim(key)?;
        }
    }
    let mut ddefs = read_dict(&mut c.t, nda, "DetailAttrib")?;
    read_block(&mut c.t, &mut ddefs, "(", ")")?;

    let mut groups = Vec::new();
    for _ in 0..npg {
        let (name, mask) = c.group(np)?;
        groups.push(Group { name, class: AttrClass::Point, mask });
    }
    for _ in 0..nqg {
        let (name, mask) = c.group(nq)?;
        groups.push(Group { name, class: AttrClass::Prim, mask });
    }
    let mut d =
        Detail { point_count: np as u32, vertex_point: c.vert_point, prims: c.prims, groups, ..Detail::default() };
    d.attrs.push(Attribute {
        name: "P".into(),
        class: AttrClass::Point,
        data: AttrData::V3(pos),
        type_info: TypeInfo::Point,
    });
    build(pdefs, AttrClass::Point, &mut d.attrs)?;
    if d.has_spline_prims() && pw.iter().any(|&w| w != 1.0) && d.attr(AttrClass::Point, "Pw").is_none() {
        d.attrs.push(Attribute {
            name: "Pw".into(),
            class: AttrClass::Point,
            data: AttrData::F32(pw),
            type_info: TypeInfo::None,
        });
    }
    build(c.vdefs, AttrClass::Vertex, &mut d.attrs)?;
    build(c.qdefs, AttrClass::Prim, &mut d.attrs)?;
    build(ddefs, AttrClass::Detail, &mut d.attrs)?;
    Ok(d)
}
