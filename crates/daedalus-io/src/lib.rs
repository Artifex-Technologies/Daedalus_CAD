//! File format import/export for Daedalus CAD.
//!
//! Each format implements [`MeshImporter`] and/or [`MeshExporter`]. `.daedalus` (native.rs) is Daedalus's own. Formats with
//! licensing or SDK constraints (DWG, SKP, FBX) belong in their own optional
//! modules or crates so they can be feature-gated.

use daedalus_core::Mesh;
use std::fmt;
use std::path::Path;

pub mod dae;
pub mod dxf;
pub mod dxf2d;
pub mod image_io;
pub mod native;
pub mod obj;
pub mod skp;
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
    if ext == "dskp" {
        return Ok(skp::read(&std::fs::read(path)?)?.flatten());
    }
    let importers: [&dyn MeshImporter; 5] = [&native::Native, &stl::Stl, &obj::Obj, &dxf::Dxf, &dae::Dae];
    let importer = importers
        .iter()
        .find(|i| i.extensions().contains(&ext.as_str()))
        .ok_or_else(|| IoError::Unsupported(format!(".{ext}")))?;
    importer.import(&std::fs::read(path)?)
}

/// Opens a model: a `.daedalus` file whole (groups, components, notes), any other format as its geometry.
pub fn open_scene(path: &Path) -> Result<daedalus_core::scene::Scene, IoError> {
    match extension(path).as_deref() {
        Some("daedalus") => return native::read_scene(&std::fs::read(path)?),
        // a SketchUp model, as skp-convert describes it (skp.rs)
        Some("dskp") => return skp::read(&std::fs::read(path)?),
        _ => {}
    }
    Ok(daedalus_core::scene::Scene::from_mesh(import_mesh(path)?))
}

/// Saves a model: whole to a `.daedalus` file; to any other format, its geometry with every instance put in place.
pub fn save_scene(scene: &daedalus_core::scene::Scene, path: &Path) -> Result<(), IoError> {
    if extension(path).as_deref() == Some("daedalus") {
        std::fs::write(path, native::write_scene(scene))?;
        return Ok(());
    }
    export_mesh(&scene.flatten(), path)
}

/// Exports a mesh, choosing the exporter from the file extension.
pub fn export_mesh(mesh: &Mesh, path: &Path) -> Result<(), IoError> {
    let ext = extension(path).ok_or_else(|| IoError::Unsupported("no file extension".into()))?;
    let exporters: [&dyn MeshExporter; 5] = [&native::Native, &stl::Stl, &obj::Obj, &dxf::Dxf, &dae::Dae];
    let exporter = exporters
        .iter()
        .find(|e| e.extensions().contains(&ext.as_str()))
        .ok_or_else(|| IoError::Unsupported(format!(".{ext}")))?;
    std::fs::write(path, exporter.export(mesh)?)?;
    Ok(())
}
