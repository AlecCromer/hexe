//! Extension-dispatched file IO: `read_any` / `write_any`.
//! Read: `.geo .bgeo .bgeo.sc` -> `read_geo` (sniffs), `.obj`, `.ply`. Write: `.geo` (JSON), `.bgeo` (binary JSON), `.obj`, `.ply`
//! (ascii); `.bgeo.sc` (blosc) is read-only.
//! Reads are size-capped by `MAX_BYTES` before the file is loaded.
use crate::error::check_len;
use crate::*;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Geo,
    Obj,
    Ply,
}
fn kind_of(path: &Path) -> Result<Kind, GeoError> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let ext = |s: &str| name.ends_with(s);
    if ext(".geo") || ext(".bgeo") || ext(".bgeo.sc") {
        Ok(Kind::Geo)
    } else if ext(".obj") {
        Ok(Kind::Obj)
    } else if ext(".ply") {
        Ok(Kind::Ply)
    } else {
        Err(GeoError::Unsupported(format!(
            "{}: unknown geometry extension (.geo .bgeo .bgeo.sc .obj .ply)",
            path.display()
        )))
    }
}
fn io(path: &Path, e: std::io::Error) -> GeoError {
    GeoError::Io(format!("{}: {e}", path.display()))
}
pub fn read_any(path: impl AsRef<Path>) -> Result<Detail, GeoError> {
    let path = path.as_ref();
    let kind = kind_of(path)?;
    let len = std::fs::metadata(path).map_err(|e| io(path, e))?.len();
    check_len(usize::try_from(len).unwrap_or(usize::MAX))?;
    let bytes = std::fs::read(path).map_err(|e| io(path, e))?;
    match kind {
        Kind::Geo => read_geo(&bytes),
        Kind::Obj => read_obj(&bytes),
        Kind::Ply => read_ply(&bytes),
    }
}
pub fn write_any(path: impl AsRef<Path>, d: &Detail) -> Result<(), GeoError> {
    let path = path.as_ref();
    let bytes = match kind_of(path)? {
        Kind::Geo if path.to_str().is_some_and(|s| s.to_ascii_lowercase().ends_with(".geo")) => write_geo_json(d),
        Kind::Geo if path.to_str().is_some_and(|s| s.to_ascii_lowercase().ends_with(".bgeo")) => write_geo_bjson(d)?,
        Kind::Geo => return Err(GeoError::Unsupported("writing blosc-compressed .bgeo.sc (use .bgeo or .geo)".into())),
        Kind::Obj => write_obj(d)?,
        Kind::Ply => write_ply(d)?,
    };
    std::fs::write(path, bytes).map_err(|e| io(path, e))
}
