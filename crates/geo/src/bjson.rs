//! Houdini binary JSON (`.bgeo`, magic `7f 'N' 'S' 'J' 'b'`) → the same `serde_json::Value` tree the ASCII reader walks,
//! then the shared `detail_from_value`. Token table: see `tok` (inferred from the 20.5.487 shipped corpus, not from docs).
//! Hostile input → `Err`, never panic: depth cap, every length checked against the bytes left, checked arithmetic.
use crate::error::check_len;
use crate::json_read::detail_from_value;
use crate::{Detail, GeoError};
use serde_json::{Map, Number, Value};

/// File magic: `0x7f` + ASCII `NSJb`.
pub const MAGIC: [u8; 5] = [0x7f, b'N', b'S', b'J', b'b'];
/// Token bytes (all single-byte, then payload).
mod tok {
    pub const NULL: u8 = 0x00;
    pub const BOOL: u8 = 0x10; // + 1 byte
    pub const I8: u8 = 0x11;
    pub const I16: u8 = 0x12;
    pub const I32: u8 = 0x13;
    pub const I64: u8 = 0x14;
    pub const U8: u8 = 0x15;
    pub const U16: u8 = 0x16;
    pub const F16: u8 = 0x18;
    pub const F32: u8 = 0x19;
    pub const F64: u8 = 0x1a;
    pub const PAD: u8 = 0x20;
    pub const REF: u8 = 0x26; // + len-coded table index
    pub const STRING: u8 = 0x27; // + len + bytes
    pub const DEF: u8 = 0x2b; // + len-coded index + len + bytes; defines table[index], yields no value
    pub const FALSE: u8 = 0x30;
    pub const TRUE: u8 = 0x31;
    pub const UNIFORM: u8 = 0x40; // + element token + len-coded count + packed little-endian elements
    pub const ARRAY: u8 = 0x5b;
    pub const ARRAY_END: u8 = 0x5d;
    pub const MAP: u8 = 0x7b;
    pub const MAP_END: u8 = 0x7d;
}
const MAX_DEPTH: usize = 64;
const MAX_TABLE: usize = 1 << 20;

fn syn<T>(s: impl Into<String>) -> Result<T, GeoError> {
    Err(GeoError::Syntax(s.into()))
}
struct Rd<'a> {
    b: &'a [u8],
    p: usize,
    table: Vec<Option<String>>,
}
impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], GeoError> {
        let end = self.p.checked_add(n).filter(|&e| e <= self.b.len());
        let Some(end) = end else { return syn(format!("bjson truncated at byte {}", self.p)) };
        let s = self.b.get(self.p..end).unwrap_or_default();
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, GeoError> {
        match self.take(1)?.first() {
            Some(&b) => Ok(b),
            None => syn("bjson truncated"),
        }
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], GeoError> {
        let mut o = [0u8; N];
        o.copy_from_slice(self.take(N)?);
        Ok(o)
    }
    /// Length / count / table index: one byte < 0xF0, else `0xF1`/`0xF2`/`0xF4`/`0xF8` + that many LE bytes.
    fn len(&mut self) -> Result<usize, GeoError> {
        let at = self.p;
        let v: u64 = match self.u8()? {
            b if b <= 0xF0 => u64::from(b),
            0xF1 => u64::from(self.u8()?),
            0xF2 => u64::from(u16::from_le_bytes(self.arr()?)),
            0xF3 => u64::from(u32::from_le_bytes(self.arr()?)),
            0xF4 => u64::from_le_bytes(self.arr()?),
            b => return syn(format!("bjson: unknown length prefix 0x{b:02x} at byte {at}")),
        };
        usize::try_from(v).or_else(|_| syn("bjson: length exceeds usize"))
    }
    fn string(&mut self) -> Result<String, GeoError> {
        let n = self.len()?;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    /// Next token byte, transparently consuming table definitions and padding.
    fn tag(&mut self) -> Result<u8, GeoError> {
        loop {
            match self.u8()? {
                tok::DEF => {
                    let i = self.len()?;
                    let s = self.string()?;
                    if i >= MAX_TABLE {
                        return Err(GeoError::Index(format!("bjson string table index {i} too large")));
                    }
                    if self.table.len() <= i {
                        self.table.resize(i + 1, None);
                    }
                    if let Some(slot) = self.table.get_mut(i) {
                        *slot = Some(s);
                    }
                }
                tok::PAD => {}
                t => return Ok(t),
            }
        }
    }
    fn table_ref(&mut self) -> Result<String, GeoError> {
        let i = self.len()?;
        match self.table.get(i) {
            Some(Some(s)) => Ok(s.clone()),
            _ => Err(GeoError::Index(format!("bjson string table ref {i} undefined"))),
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value, GeoError> {
        let t = self.tag()?;
        self.value_of(t, depth)
    }
    fn value_of(&mut self, t: u8, depth: usize) -> Result<Value, GeoError> {
        if depth > MAX_DEPTH {
            return syn("bjson nested too deeply");
        }
        Ok(match t {
            tok::NULL => Value::Null,
            tok::FALSE => Value::Bool(false),
            tok::TRUE => Value::Bool(true),
            tok::BOOL => Value::Bool(self.u8()? != 0),
            tok::I8 | tok::I16 | tok::I32 | tok::I64 | tok::U8 | tok::U16 | tok::F16 | tok::F32 | tok::F64 => self.scalar(t)?,
            tok::STRING => Value::String(self.string()?),
            tok::REF => Value::String(self.table_ref()?),
            tok::ARRAY => {
                let mut v = Vec::new();
                loop {
                    let t = self.tag()?;
                    if t == tok::ARRAY_END {
                        break;
                    }
                    v.push(self.value_of(t, depth + 1)?);
                }
                Value::Array(v)
            }
            tok::MAP => {
                let mut m = Map::new();
                loop {
                    let t = self.tag()?;
                    if t == tok::MAP_END {
                        break;
                    }
                    let Value::String(k) = self.value_of(t, depth + 1)? else { return syn("bjson: map key is not a string") };
                    let v = self.value(depth + 1)?;
                    m.insert(k, v);
                }
                Value::Object(m)
            }
            tok::UNIFORM => self.uniform()?,
            b => return syn(format!("bjson: unknown token 0x{b:02x} at byte {}", self.p.saturating_sub(1))),
        })
    }
    fn scalar(&mut self, t: u8) -> Result<Value, GeoError> {
        Ok(match t {
            tok::I8 => Value::from(i8::from_le_bytes(self.arr()?)),
            tok::I16 => Value::from(i16::from_le_bytes(self.arr()?)),
            tok::I32 => Value::from(i32::from_le_bytes(self.arr()?)),
            tok::I64 => Value::from(i64::from_le_bytes(self.arr()?)),
            tok::U8 => Value::from(u8::from_le_bytes(self.arr()?)),
            tok::U16 => Value::from(u16::from_le_bytes(self.arr()?)),
            tok::F16 => real(f64::from(f16_to_f32(u16::from_le_bytes(self.arr()?)))),
            tok::F32 => real(f64::from(f32::from_le_bytes(self.arr()?))),
            _ => real(f64::from_le_bytes(self.arr()?)),
        })
    }
    fn uniform(&mut self) -> Result<Value, GeoError> {
        let at = self.p;
        let et = self.u8()?;
        let n = self.len()?;
        if et == tok::BOOL {
            // bit-packed (element i = bit i%8 of byte i/8), byte count padded up to a multiple of 4
            let nbytes = n.div_ceil(32).checked_mul(4).filter(|&x| x <= self.b.len().saturating_sub(self.p));
            let Some(nbytes) = nbytes else { return syn(format!("bjson bool array of {n} overruns the data at byte {at}")) };
            let bytes = self.take(nbytes)?;
            let v = (0..n).map(|i| Value::Bool(bytes.get(i / 8).is_some_and(|b| (b >> (i % 8)) & 1 == 1))).collect();
            return Ok(Value::Array(v));
        }
        let w: usize = match et {
            tok::I8 | tok::U8 => 1,
            tok::I16 | tok::U16 | tok::F16 => 2,
            tok::I32 | tok::F32 => 4,
            tok::I64 | tok::F64 => 8,
            tok::STRING | tok::REF => 1, // at least one byte each
            b => return syn(format!("bjson: unknown uniform-array element token 0x{b:02x} at byte {at}")),
        };
        // cap before allocating: the payload must be physically present
        let need = w.checked_mul(n);
        if need.is_none_or(|x| x > self.b.len().saturating_sub(self.p)) {
            return syn(format!("bjson uniform array of {n} overruns the data at byte {}", self.p));
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(match et {
                tok::STRING => Value::String(self.string()?),
                tok::REF => Value::String(self.table_ref()?),
                t => self.scalar(t)?,
            });
        }
        Ok(Value::Array(v))
    }
}
/// Non-finite reals have no JSON form; they become `null` (and then fail the numeric-array check, never silently 0).
fn real(f: f64) -> Value {
    Number::from_f64(f).map_or(Value::Null, Value::Number)
}
fn f16_to_f32(h: u16) -> f32 {
    let s = if h & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let e = i32::from((h >> 10) & 0x1f);
    let m = f32::from(h & 0x3ff);
    if e == 0 {
        s * m * 2f32.powi(-24)
    } else if e == 31 {
        if m == 0.0 { s * f32::INFINITY } else { f32::NAN }
    } else {
        s * (1.0 + m / 1024.0) * 2f32.powi(e - 15)
    }
}
/// Decode a binary-JSON container to the generic tree (also used by tests to compare against ASCII).
pub fn bjson_to_value(bytes: &[u8]) -> Result<Value, GeoError> {
    check_len(bytes.len())?;
    if !bytes.starts_with(&MAGIC) {
        return syn("not a bjson file (bad magic)");
    }
    let mut r = Rd { b: bytes, p: MAGIC.len(), table: Vec::new() };
    let v = r.value(0)?;
    if r.p != bytes.len() {
        return syn(format!("bjson: {} trailing bytes after the top-level value", bytes.len() - r.p));
    }
    Ok(v)
}
/// Houdini 20 binary `.bgeo` → `Detail` (identical in-memory result to the ASCII `.geo` path).
pub fn read_geo_bjson(bytes: &[u8]) -> Result<Detail, GeoError> {
    detail_from_value(&bjson_to_value(bytes)?)
}
