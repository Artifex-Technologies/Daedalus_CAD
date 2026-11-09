//! File format import/export for Daedalus CAD.
//!
//! Each format implements [`MeshImporter`] and/or [`MeshExporter`]. `.ddls` (native.rs; `.daedalus` before it) is Daedalus's
//! own; STEP (step.rs) holds a whole model too, groups and components as an assembly. formats.rs says where Save writes and
//! what each format leaves out. Formats with
//! licensing or SDK constraints (DWG, SKP, FBX) belong in their own optional
//! modules or crates so they can be feature-gated.

use daedalus_core::Mesh;
use std::fmt;
use std::path::Path;

pub mod dae;
pub mod dxf;
pub mod dxf2d;
pub mod formats;
pub mod image_io;
pub mod native;
pub mod obj;
pub mod skp;
pub mod step;
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
    let importers: [&dyn MeshImporter; 6] = [&native::Native, &stl::Stl, &obj::Obj, &dxf::Dxf, &dae::Dae, &step::Step];
    let importer = importers
        .iter()
        .find(|i| i.extensions().contains(&ext.as_str()))
        .ok_or_else(|| IoError::Unsupported(format!(".{ext}")))?;
    importer.import(&std::fs::read(path)?)
}

/// Opens a model: a `.ddls` or `.daedalus` file whole (groups, components, notes), a STEP file whole (its assembly as groups
/// and components; refused, naming them, if it has faces Daedalus cannot read yet: `step::read` reads the rest), any other
/// format as its geometry.
pub fn open_scene(path: &Path) -> Result<daedalus_core::scene::Scene, IoError> {
    match extension(path).as_deref() {
        Some("ddls" | "daedalus") => return native::read_scene(&std::fs::read(path)?),
        // a SketchUp model, as skp-convert describes it (skp.rs)
        Some("dskp") => return skp::read(&std::fs::read(path)?),
        Some("step" | "stp") => return step::read_scene(&std::fs::read(path)?),
        _ => {}
    }
    Ok(daedalus_core::scene::Scene::from_mesh(import_mesh(path)?))
}

/// Saves a model: whole to a `.ddls` (or `.daedalus`) file; to STEP as an assembly, in millimetres; to OBJ, STL, DAE or DXF
/// its geometry with every instance put in place. [`save_scene_with`] takes the units and says what was left out.
pub fn save_scene(scene: &daedalus_core::scene::Scene, path: &Path) -> Result<(), IoError> {
    save_scene_with(scene, path, &SaveOptions::default()).map(|_| ())
}

/// How [`save_scene_with`] saves.
#[derive(Debug, Clone, Copy)]
pub struct SaveOptions {
    /// The units lengths are shown in: a STEP file is written in them (feet and inches as inches).
    pub units: daedalus_core::units::Units,
}

impl Default for SaveOptions {
    fn default() -> Self {
        SaveOptions { units: daedalus_core::units::Units::Millimetres }
    }
}

/// Saves a model in the format its file's ending names (see [`save_scene`]), and returns what that format could not hold, in
/// plain words, one line each (empty for `.ddls`, and whenever nothing was lost). A format Daedalus does not write (`.skp`)
/// is refused, the file left as it was.
pub fn save_scene_with(scene: &daedalus_core::scene::Scene, path: &Path, opts: &SaveOptions) -> Result<Vec<String>, IoError> {
    use formats::Format;
    match Format::of(path) {
        Some(Format::Native) => {
            std::fs::write(path, native::write_scene(scene))?;
            Ok(Vec::new())
        }
        Some(Format::Step) => {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let written = step::write(scene, &step::WriteOptions { units: opts.units, name })?;
            std::fs::write(path, written.text)?;
            Ok(written.left_out)
        }
        Some(f) => {
            let lost = formats::left_out(f, scene);
            export_mesh(&scene.flatten(), path)?;
            Ok(lost)
        }
        None => Err(IoError::Unsupported(match extension(path) {
            Some(e) => format!("Daedalus does not write .{e} files"),
            None => "no file extension".into(),
        })),
    }
}

/// Exports a mesh, choosing the exporter from the file extension.
pub fn export_mesh(mesh: &Mesh, path: &Path) -> Result<(), IoError> {
    let ext = extension(path).ok_or_else(|| IoError::Unsupported("no file extension".into()))?;
    let exporters: [&dyn MeshExporter; 6] = [&native::Native, &stl::Stl, &obj::Obj, &dxf::Dxf, &dae::Dae, &step::Step];
    let exporter = exporters
        .iter()
        .find(|e| e.extensions().contains(&ext.as_str()))
        .ok_or_else(|| IoError::Unsupported(format!(".{ext}")))?;
    std::fs::write(path, exporter.export(mesh)?)?;
    Ok(())
}
