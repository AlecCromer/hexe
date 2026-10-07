use crate::{Detail, GeoError};
pub fn read_geo_gpd(_b: &[u8]) -> Result<Detail, GeoError> { Err(GeoError::Unsupported("stub".into())) }
