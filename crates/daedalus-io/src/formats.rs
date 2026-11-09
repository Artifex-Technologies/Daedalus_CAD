//! Where File > Save writes a model and in what, and what each format leaves out of it.
//!
//! The rules (my decision, 2026-11-08: files are shared, so they must not need Daedalus to be opened):
//! - a model is saved in the format it was opened in when Daedalus writes that format: STEP, the native `.ddls` (a
//!   `.daedalus` file keeps its own name), OBJ, STL, DAE, DXF; Save writes it in place, with no dialog;
//! - a new model asks where, STEP chosen to start with (`.ddls` keeps everything);
//! - a model opened from a format Daedalus does not write (SketchUp's `.skp`, a `.dskp`) asks too, starting beside it with its
//!   name and `.step`: the file it came from is never written over. Once saved, Save goes to the file chosen;
//! - so does a file Daedalus read only in part (a STEP file with curved faces), so what it did not read stays in that file.
//!
//! After a save to a format that cannot hold everything, the person is told what was left out ([`left_out`], and STEP's own
//! list from [`crate::step::write`]).

use daedalus_core::scene::Scene;
use std::path::{Path, PathBuf};

/// A format a whole model can be saved in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Step,
    /// Daedalus's own: keeps everything (`.ddls`; `.daedalus` before it).
    Native,
    Obj,
    Stl,
    Dae,
    Dxf,
}

impl Format {
    /// What the Save dialog offers, in its order: STEP first, the format new models start in.
    pub const SAVE: [Format; 6] = [Format::Step, Format::Native, Format::Obj, Format::Stl, Format::Dae, Format::Dxf];

    /// The format a file is in, by its ending; None for one Daedalus does not write (`.skp`, `.dskp`).
    pub fn of(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_string_lossy().to_lowercase();
        Format::SAVE.into_iter().find(|f| f.extensions().contains(&ext.as_str()))
    }

    /// The endings it is written with, the one a new file gets first.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Format::Step => &["step", "stp"],
            Format::Native => &["ddls", "daedalus"],
            Format::Obj => &["obj"],
            Format::Stl => &["stl"],
            Format::Dae => &["dae"],
            Format::Dxf => &["dxf"],
        }
    }

    /// The ending a new file in this format gets.
    pub fn extension(self) -> &'static str {
        self.extensions()[0]
    }

    /// How the Save dialog names it.
    pub fn label(self) -> &'static str {
        match self {
            Format::Step => "STEP (.step): opens in every CAD program",
            Format::Native => "Daedalus model (.ddls): keeps everything",
            Format::Obj => "OBJ (.obj)",
            Format::Stl => "STL (.stl)",
            Format::Dae => "COLLADA (.dae)",
            Format::Dxf => "DXF (.dxf)",
        }
    }
}

/// What File > Save or Save As does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveTo {
    /// Write this file, as it is, with no dialog.
    File(PathBuf),
    /// Ask with the Save dialog: starting in `folder` (None: wherever the dialog starts), `name` typed in, `format` chosen.
    Ask { folder: Option<PathBuf>, name: String, format: Format },
}

/// What File > Save (`save_as` false) or Save As does, for a document whose file is `file`: the one it was opened from or
/// last saved to (None: a new model, or an import, not saved yet). `whole`: the document holds all of that file; when it holds
/// only part (a STEP file with curved faces, read without them), Save asks for a new file rather than write over that one.
pub fn save_target(file: Option<&Path>, whole: bool, save_as: bool) -> SaveTo {
    let Some(file) = file else {
        return SaveTo::Ask { folder: None, name: format!("Untitled.{}", Format::Step.extension()), format: Format::Step };
    };
    let folder = file.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf);
    let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
    match Format::of(file) {
        // kept as it is, at its own path (a .daedalus file too: never renamed)
        Some(_) if whole && !save_as => SaveTo::File(file.to_path_buf()),
        // read in part: a file of its own beside it, so the faces not read stay where they are
        Some(f) if !whole => SaveTo::Ask { folder, name: format!("{stem} (Daedalus).{}", f.extension()), format: f },
        // Save As starts in the same format; a new native file is .ddls
        Some(Format::Native) => SaveTo::Ask { folder, name: format!("{stem}.{}", Format::Native.extension()), format: Format::Native },
        Some(f) => SaveTo::Ask { folder, name: file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), format: f },
        // a SketchUp model (or anything Daedalus cannot write): beside it, as STEP
        None => SaveTo::Ask { folder, name: format!("{stem}.{}", Format::Step.extension()), format: Format::Step },
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// What a file in `format` cannot hold of `scene`, one line each in plain words; empty when nothing is lost. For STEP use
/// the list [`crate::step::write`] makes as it writes (this one writes the file to find out); `.ddls` loses nothing.
pub fn left_out(format: Format, scene: &Scene) -> Vec<String> {
    match format {
        Format::Native => Vec::new(),
        Format::Step => crate::step::write(scene, &crate::step::WriteOptions::default()).map(|w| w.left_out).unwrap_or_default(),
        _ => mesh_left_out(format, scene),
    }
}

/// OBJ, STL, DAE and DXF as Daedalus writes them: the model as one mesh of faces, in metres.
fn mesh_left_out(format: Format, scene: &Scene) -> Vec<String> {
    let mut out = Vec::new();
    let flat = scene.flatten();
    if !scene.root.instances.is_empty() {
        out.push("groups and components (all written as one mesh)".into());
    }
    let n = &scene.notes;
    for (k, one, many) in [(n.guides.len(), "guide", "guides"), (n.dims.len(), "dimension", "dimensions"), (n.labels.len(), "text note", "text notes"), (n.sections.len(), "section plane", "section planes")] {
        if k > 0 {
            out.push(count(k, one, many));
        }
    }
    if !n.axes.is_default() {
        out.push("the drawing axes".into());
    }
    if flat.colors.iter().any(|c| c.is_some()) {
        out.push("colours".into());
    }
    if !scene.textures.is_empty() {
        out.push(count(scene.textures.len(), "texture", "textures"));
    }
    if !flat.lines.is_empty() {
        out.push(count(flat.lines.len(), "loose line", "loose lines"));
    }
    let edges: std::collections::HashSet<(u32, u32)> = flat.edges().into_iter().collect();
    let soft: std::collections::HashSet<(u32, u32)> = flat.seams.iter().map(|&[a, b]| (a.min(b), a.max(b))).filter(|e| edges.contains(e)).collect();
    if !soft.is_empty() {
        out.push(format!("{} (every edge is drawn)", count(soft.len(), "soft edge", "soft edges")));
    }
    // OBJ and COLLADA keep faces of any shape but no holes; STL and DXF only triangles
    let holed = (0..flat.faces.len()).filter(|&f| !flat.holes_of(f).is_empty()).count();
    match format {
        Format::Stl | Format::Dxf => {
            let shaped = (0..flat.faces.len()).filter(|&f| flat.faces[f].len() > 3 || !flat.holes_of(f).is_empty()).count();
            if shaped > 0 {
                out.push(format!("the faces' shapes ({} written as triangles)", count(shaped, "face", "faces")));
            }
        }
        _ if holed > 0 => out.push(format!("holes ({} written as triangles)", count(holed, "face with holes", "faces with holes"))),
        _ => {}
    }
    let dynamic = scene.defs.iter().filter(|d| !d.attributes.is_empty()).count();
    if dynamic > 0 {
        out.push(format!("Dynamic Component attributes ({} written as drawn now)", count(dynamic, "component", "components")));
    }
    let parts = scene.defs.iter().filter(|d| d.part.is_some()).count();
    if parts > 0 {
        out.push(format!("{} (as their faces)", count(parts, "sheet-metal part", "sheet-metal parts")));
    }
    // COLLADA says its unit; the others hold bare numbers, which Daedalus writes in metres
    if format != Format::Dae && !flat.faces.is_empty() {
        out.push("the length unit (the file holds plain numbers, in metres)".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use daedalus_core::scene::{Context, Definition, Guide, Instance, Xform};
    use daedalus_core::Mesh;

    fn ask(folder: Option<&str>, name: &str, format: Format) -> SaveTo {
        SaveTo::Ask { folder: folder.map(PathBuf::from), name: name.into(), format }
    }

    #[test]
    fn save_keeps_the_format_a_model_was_opened_in() {
        for f in ["/m/house.step", "/m/house.STP", "/m/house.ddls", "/m/part.obj", "/m/part.stl", "/m/scene.dae", "/m/plan.dxf"] {
            assert_eq!(save_target(Some(Path::new(f)), true, false), SaveTo::File(PathBuf::from(f)), "{f}: in place, no dialog");
        }
        // an old native file is saved back to its own name, never renamed
        assert_eq!(save_target(Some(Path::new("/m/old.daedalus")), true, false), SaveTo::File(PathBuf::from("/m/old.daedalus")));
    }

    #[test]
    fn a_new_model_asks_with_step_chosen() {
        assert_eq!(save_target(None, true, false), ask(None, "Untitled.step", Format::Step));
        assert_eq!(save_target(None, true, true), ask(None, "Untitled.step", Format::Step));
    }

    #[test]
    fn a_sketchup_model_is_saved_beside_itself_as_step() {
        for f in ["/m/Sample 2.skp", "/m/Sample 2.dskp", "/m/Sample 2.SKP"] {
            assert_eq!(save_target(Some(Path::new(f)), true, false), ask(Some("/m"), "Sample 2.step", Format::Step), "{f}: never written over");
            assert_eq!(save_target(Some(Path::new(f)), true, true), ask(Some("/m"), "Sample 2.step", Format::Step));
        }
    }

    #[test]
    fn a_file_read_in_part_is_not_written_over() {
        assert_eq!(save_target(Some(Path::new("/m/bracket.step")), false, false), ask(Some("/m"), "bracket (Daedalus).step", Format::Step));
        assert_eq!(save_target(Some(Path::new("/m/bracket.stp")), false, true), ask(Some("/m"), "bracket (Daedalus).step", Format::Step));
    }

    #[test]
    fn save_as_starts_in_the_same_format() {
        assert_eq!(save_target(Some(Path::new("/m/part.obj")), true, true), ask(Some("/m"), "part.obj", Format::Obj));
        assert_eq!(save_target(Some(Path::new("/m/house.stp")), true, true), ask(Some("/m"), "house.stp", Format::Step));
        // a new native file is .ddls
        assert_eq!(save_target(Some(Path::new("/m/old.daedalus")), true, true), ask(Some("/m"), "old.ddls", Format::Native));
        assert_eq!(save_target(Some(Path::new("house.ddls")), true, true), ask(None, "house.ddls", Format::Native));
    }

    #[test]
    fn formats_by_their_endings() {
        assert_eq!(Format::of(Path::new("a.STEP")), Some(Format::Step));
        assert_eq!(Format::of(Path::new("a.daedalus")), Some(Format::Native));
        assert_eq!(Format::of(Path::new("a.skp")), None);
        assert_eq!(Format::of(Path::new("a")), None);
        assert_eq!(Format::SAVE.map(Format::extension), ["step", "ddls", "obj", "stl", "dae", "dxf"]);
    }

    fn square() -> Mesh {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        m
    }

    #[test]
    fn what_each_format_leaves_out() {
        // one triangle, nothing else: only STL's, OBJ's and DXF's missing unit
        let mut tri = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            tri.add_vertex(p);
        }
        tri.add_face(vec![0, 1, 2]);
        let plain = Scene::from_mesh(tri);
        assert!(left_out(Format::Dae, &plain).is_empty());
        assert!(left_out(Format::Native, &plain).is_empty());
        assert!(left_out(Format::Step, &plain).is_empty());
        assert_eq!(left_out(Format::Stl, &plain), vec!["the length unit (the file holds plain numbers, in metres)".to_string()]);
        // a painted square with a loose line and a guide, placed as a group
        let mut m = square();
        m.set_color(0, Some([200, 0, 0].into()));
        let v = m.add_vertex([2.0, 0.0, 0.0]);
        m.lines.push([1, v]);
        let mut s = Scene::default();
        s.defs.push(Definition { name: "G".into(), component: false, glue: false, cut: false, attributes: Vec::new(), part: None, ctx: Context { mesh: m, instances: Vec::new() } });
        s.root.instances.push(Instance { def: 0, xf: Xform::IDENTITY });
        s.notes.guides.push(Guide::Point([0.0; 3]));
        let stl = left_out(Format::Stl, &s);
        assert_eq!(
            stl,
            vec![
                "groups and components (all written as one mesh)",
                "1 guide",
                "colours",
                "1 loose line",
                "the faces' shapes (1 face written as triangles)",
                "the length unit (the file holds plain numbers, in metres)"
            ]
        );
        assert!(left_out(Format::Obj, &s).iter().all(|l| !l.contains("triangles")), "OBJ keeps a square a square");
        assert_eq!(left_out(Format::Step, &s), vec!["1 guide".to_string()], "STEP keeps the rest");
    }

    #[test]
    fn saving_says_what_was_left_out_and_never_writes_a_sketchup_file() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("daedalus-formats-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Scene::from_mesh(square());
        s.notes.guides.push(Guide::Point([0.0; 3]));
        let save = |p: &Path| crate::save_scene_with(&s, p, &crate::SaveOptions::default());
        // .ddls keeps everything
        let p = dir.join("m.ddls");
        assert!(save(&p).unwrap().is_empty());
        assert_eq!(crate::open_scene(&p).unwrap(), s);
        assert_eq!(crate::import_mesh(&p).unwrap().faces.len(), 1);
        // a .daedalus file still opens, and saves under its own name
        let old = dir.join("old.daedalus");
        std::fs::write(&old, crate::native::write_scene(&s)).unwrap();
        assert_eq!(crate::open_scene(&old).unwrap(), s);
        assert!(save(&old).unwrap().is_empty());
        // STEP and STL say what they cannot hold
        assert_eq!(save(&dir.join("m.step")).unwrap(), vec!["1 guide".to_string()]);
        assert!(save(&dir.join("m.stl")).unwrap().contains(&"1 guide".to_string()));
        // a SketchUp file is refused and left as it was
        let skp = dir.join("m.skp");
        std::fs::write(&skp, b"SketchUp model").unwrap();
        assert!(save(&skp).unwrap_err().to_string().contains("does not write .skp"));
        assert_eq!(std::fs::read(&skp).unwrap(), b"SketchUp model");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
