//! hexe geo (L0): `Detail` + attributes + groups + Houdini `.geo` readers/writer. Hostile input is the default (AGENTS.md §4).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod detail;
mod error;
mod gpd;
mod json_read;
mod json_write;
mod types;

pub use error::GeoError;
pub use gpd::read_geo_gpd;
pub use json_read::read_geo_json;
pub use json_write::write_geo_json;
pub use types::*;

/// Sniff the container: legacy `PGEOMETRY` text (GPD) vs JSON. Binary `BgeoV` is wave 2 (`Err(Unsupported)`).
pub fn read_geo(bytes: &[u8]) -> Result<Detail, GeoError> {
    let head = bytes.iter().copied().find(|b| !b.is_ascii_whitespace());
    match head {
        Some(b'P') => read_geo_gpd(bytes),
        Some(b'[') | Some(b'{') => read_geo_json(bytes),
        Some(b'B') => Err(GeoError::Unsupported("binary bgeo is wave 2".into())),
        _ => Err(GeoError::Syntax("unrecognised geometry container".into())),
    }
}
