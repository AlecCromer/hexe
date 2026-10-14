//! Extension-dispatched file IO: `read_any` / `write_any`.
//! Read: `.geo .bgeo .bgeo.sc` -> `read_geo` (sniffs), `.obj`, `.ply`. Write: `.geo` (JSON), `.bgeo` (binary JSON), `.obj`, `.ply`
//! (ascii); `.bgeo.sc` (blosc) is read-only. Also read-only: Endorphin `.ens .nmc .nmb .nma` -> rest-pose geometry (`endorphin_geo`).
//! Reads are size-capped by `MAX_BYTES` before the file is loaded.
use crate::error::check_len;
use crate::*;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Geo,
    Obj,
    Ply,
    Endorphin,
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
    } else if [".ens", ".nmc", ".nmb", ".nma"].iter().any(|e| ext(e)) {
        Ok(Kind::Endorphin)
    } else {
        Err(GeoError::Unsupported(format!(
            "{}: unknown geometry extension (.geo .bgeo .bgeo.sc .obj .ply .ens .nmc .nmb .nma)",
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
        Kind::Endorphin => Ok(crate::endorphin_geo::document_detail(&crate::endorphin::read_endorphin(&bytes)?)),
    }
}
/// `true` for `.ens .nmc .nmb .nma` (time-sampled by `endo_time`).
pub fn is_endorphin_path(path: impl AsRef<Path>) -> bool {
    matches!(kind_of(path.as_ref()), Ok(Kind::Endorphin))
}
/// Size-capped read + parse of an Endorphin file (callers cache the `Document` and sample it with `document_detail_at`).
pub fn read_endorphin_file(path: impl AsRef<Path>) -> Result<crate::endorphin::Document, GeoError> {
    let path = path.as_ref();
    let len = std::fs::metadata(path).map_err(|e| io(path, e))?.len();
    check_len(usize::try_from(len).unwrap_or(usize::MAX))?;
    crate::endorphin::read_endorphin(&std::fs::read(path).map_err(|e| io(path, e))?)
}
/// `read_any`, sampling keyed Endorphin scene channels at `t` seconds (other formats ignore `t`).
pub fn read_any_at(path: impl AsRef<Path>, t: f32) -> Result<Detail, GeoError> {
    let path = path.as_ref();
    if is_endorphin_path(path) {
        return Ok(crate::endorphin_geo::document_detail_at(&read_endorphin_file(path)?, t));
    }
    read_any(path)
}
pub fn write_any(path: impl AsRef<Path>, d: &Detail) -> Result<(), GeoError> {
    let path = path.as_ref();
    let bytes = match kind_of(path)? {
        Kind::Geo if path.to_str().is_some_and(|s| s.to_ascii_lowercase().ends_with(".geo")) => write_geo_json(d),
        Kind::Geo if path.to_str().is_some_and(|s| s.to_ascii_lowercase().ends_with(".bgeo")) => write_geo_bjson(d)?,
        Kind::Geo => return Err(GeoError::Unsupported("writing blosc-compressed .bgeo.sc (use .bgeo or .geo)".into())),
        Kind::Obj => write_obj(d)?,
        Kind::Ply => write_ply(d)?,
        Kind::Endorphin => return Err(GeoError::Unsupported("Endorphin files are read-only".into())),
    };
    std::fs::write(path, bytes).map_err(|e| io(path, e))
}
