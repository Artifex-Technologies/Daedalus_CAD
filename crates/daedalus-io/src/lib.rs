//! File format import/export for Daedalus CAD.
//!
//! Each format implements [`MeshImporter`] and/or [`MeshExporter`]. Formats with
//! licensing or SDK constraints (DWG, SKP, FBX) belong in their own optional
//! modules or crates so they can be feature-gated.

use daedalus_core::Mesh;
use std::fmt;
use std::path::Path;

pub mod dae;
pub mod dxf;
pub mod dxf2d;
pub mod image_io;
pub mod obj;
pub mod stl;
mod weld;

#[derive(Debug)]
pub enum IoError {
    Io(std::io::Error),
    Parse(String),
    Unsupported(String),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::Io(e) => write!(f, "io error: {e}"),
            IoError::Parse(s) => write!(f, "parse error: {s}"),
            IoError::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for IoError {}

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> Self {
        IoError::Io(e)
    }
}

pub trait MeshImporter {
    /// Lowercase file extensions without the dot.
    fn extensions(&self) -> &'static [&'static str];
    fn import(&self, bytes: &[u8]) -> Result<Mesh, IoError>;
}

pub trait MeshExporter {
    fn extensions(&self) -> &'static [&'static str];
    fn export(&self, mesh: &Mesh) -> Result<Vec<u8>, IoError>;
}

fn extension(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_lowercase())
}

/// Imports a mesh, choosing the importer from the file extension.
pub fn import_mesh(path: &Path) -> Result<Mesh, IoError> {
    let ext = extension(path).ok_or_else(|| IoError::Unsupported("no file extension".into()))?;
    let importers: [&dyn MeshImporter; 4] = [&stl::Stl, &obj::Obj, &dxf::Dxf, &dae::Dae];
    let importer = importers
        .iter()
        .find(|i| i.extensions().contains(&ext.as_str()))
        .ok_or_else(|| IoError::Unsupported(format!(".{ext}")))?;
    importer.import(&std::fs::read(path)?)
}

/// Exports a mesh, choosing the exporter from the file extension.
pub fn export_mesh(mesh: &Mesh, path: &Path) -> Result<(), IoError> {
    let ext = extension(path).ok_or_else(|| IoError::Unsupported("no file extension".into()))?;
    let exporters: [&dyn MeshExporter; 4] = [&stl::Stl, &obj::Obj, &dxf::Dxf, &dae::Dae];
    let exporter = exporters
        .iter()
        .find(|e| e.extensions().contains(&ext.as_str()))
        .ok_or_else(|| IoError::Unsupported(format!(".{ext}")))?;
    std::fs::write(path, exporter.export(mesh)?)?;
    Ok(())
}
