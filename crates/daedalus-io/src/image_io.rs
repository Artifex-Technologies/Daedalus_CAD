//! Raster image loading (JPEG, PNG, TIFF, BMP/DIB via the `image` crate).
//! PSD loads the flattened composite only. PDF is tracked in the README.

use crate::IoError;
use std::path::Path;

pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

pub fn load_raster(path: &Path) -> Result<Raster, IoError> {
    let is_psd = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("psd"));
    if is_psd {
        let bytes = std::fs::read(path)?;
        let psd = psd::Psd::from_bytes(&bytes).map_err(|e| IoError::Parse(e.to_string()))?;
        return Ok(Raster { width: psd.width(), height: psd.height(), rgba8: psd.rgba() });
    }
    let img = image::open(path)
        .map_err(|e| IoError::Parse(e.to_string()))?
        .into_rgba8();
    Ok(Raster {
        width: img.width(),
        height: img.height(),
        rgba8: img.into_raw(),
    })
}
