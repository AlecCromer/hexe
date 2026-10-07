use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeoError {
    /// Malformed / truncated container.
    Syntax(String),
    /// A count is negative, absurd, or disagrees with the data present.
    Count(String),
    /// An index (pointref, group, string table) is out of range.
    Index(String),
    TooLarge {
        len: usize,
        max: usize,
    },
    Unsupported(String),
    /// File could not be read/written (`read_any` / `write_any`).
    Io(String),
}

impl fmt::Display for GeoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeoError::Syntax(s) => write!(f, "syntax: {s}"),
            GeoError::Count(s) => write!(f, "count: {s}"),
            GeoError::Index(s) => write!(f, "index: {s}"),
            GeoError::TooLarge { len, max } => write!(f, "input of {len} bytes exceeds the {max}-byte cap"),
            GeoError::Unsupported(s) => write!(f, "unsupported: {s}"),
            GeoError::Io(s) => write!(f, "io: {s}"),
        }
    }
}

impl std::error::Error for GeoError {}

/// Input cap (bytes) and element cap (points/vertices/prims) for every reader.
pub const MAX_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_ELEMS: u64 = 1 << 28;

pub(crate) fn check_len(len: usize) -> Result<(), GeoError> {
    if len > MAX_BYTES { Err(GeoError::TooLarge { len, max: MAX_BYTES }) } else { Ok(()) }
}

/// Validate a header count given as i64: non-negative and <= `MAX_ELEMS`.
pub(crate) fn count_of(name: &str, v: i64) -> Result<u32, GeoError> {
    if v < 0 {
        return Err(GeoError::Count(format!("{name} is negative ({v})")));
    }
    if (v as u64) > MAX_ELEMS {
        return Err(GeoError::Count(format!("{name} = {v} exceeds cap {MAX_ELEMS}")));
    }
    Ok(v as u32)
}
