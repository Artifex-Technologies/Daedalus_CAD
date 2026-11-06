//! A small file browser inside the window (Open, Save As, Import, Export). Built in rather than a toolkit's dialog, so the viewer
//! keeps no GTK or Qt dependency and looks the same on every desktop: the folder's path with Up and Home, its folders and the files
//! of the kinds that fit, a name box, and the action button. Hidden files are left out.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Open,
    SaveAs,
    Import,
    Export,
}

impl Purpose {
    fn title(self) -> &'static str {
        match self {
            Purpose::Open => "Open",
            Purpose::SaveAs => "Save As",
            Purpose::Import => "Import",
            Purpose::Export => "Export",
        }
    }

    /// The file kinds it shows, the first being the default for a name typed without one.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Purpose::Open => &["daedalus", "skp", "dskp"],
            Purpose::SaveAs => &["daedalus"],
            Purpose::Import => &["stl", "obj", "dxf", "dae", "skp", "dskp"],
            Purpose::Export => &["stl", "obj", "dxf", "dae"],
        }
    }

    fn writes(self) -> bool {
        matches!(self, Purpose::SaveAs | Purpose::Export)
    }
}

pub enum Outcome {
    Chosen(PathBuf),
    Cancelled,
}

pub struct FileDialog {
    pub purpose: Purpose,
    dir: PathBuf,
    name: String,
    entries: Vec<(String, bool)>,
    /// set when the chosen file exists and would be replaced: the next press replaces it
    confirm_replace: Option<PathBuf>,
    error: Option<String>,
}

/// The entries of a folder: (name, is a folder), folders first, then files with one of `exts`, each sorted without regard to case.
pub fn list(dir: &Path, exts: &[&str]) -> std::io::Result<Vec<(String, bool)>> {
    let mut out: Vec<(String, bool)> = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = e.path().is_dir();
            let fits = is_dir || Path::new(&name).extension().is_some_and(|x| exts.contains(&x.to_string_lossy().to_lowercase().as_str()));
            fits.then_some((name, is_dir))
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    Ok(out)
}

/// The path a typed name means: relative to the folder, with the default extension added when it has none of the allowed ones.
pub fn resolve(dir: &Path, name: &str, exts: &[&str]) -> PathBuf {
    let p = dir.join(name.trim());
    let fits = p.extension().is_some_and(|x| exts.contains(&x.to_string_lossy().to_lowercase().as_str()));
    if fits || exts.is_empty() {
        p
    } else {
        let mut s = p.into_os_string();
        s.push(format!(".{}", exts[0]));
        PathBuf::from(s)
    }
}

impl FileDialog {
    /// Opens in `start`'s folder (the current file's, else home), with `name` in the box.
    pub fn new(purpose: Purpose, start: Option<&Path>, name: &str) -> FileDialog {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        let dir = start.and_then(|p| p.parent()).filter(|d| d.is_dir()).map(Path::to_path_buf).unwrap_or(home);
        let mut d = FileDialog { purpose, dir, name: name.to_string(), entries: vec![], confirm_replace: None, error: None };
        d.reload();
        d
    }

    fn reload(&mut self) {
        match list(&self.dir, self.purpose.extensions()) {
            Ok(e) => {
                self.entries = e;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(format!("This folder cannot be read: {e}"));
            }
        }
    }

    fn go(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.confirm_replace = None;
        self.reload();
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<Outcome> {
        let mut outcome = None;
        let mut open = true;
        let exts = self.purpose.extensions();
        egui::Window::new(self.purpose.title())
            .collapsible(false)
            .resizable(true)
            .default_size([520.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Up").clicked() {
                        if let Some(p) = self.dir.parent().map(Path::to_path_buf) {
                            self.go(p);
                        }
                    }
                    if ui.button("Home").clicked() {
                        if let Some(h) = std::env::var_os("HOME") {
                            self.go(PathBuf::from(h));
                        }
                    }
                    ui.label(egui::RichText::new(self.dir.display().to_string()).monospace());
                });
                ui.separator();
                let mut enter: Option<PathBuf> = None;
                let mut pick: Option<(String, bool)> = None;
                egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, false]).show(ui, |ui| {
                    if let Some(e) = &self.error {
                        ui.colored_label(egui::Color32::from_rgb(190, 40, 40), e);
                    }
                    for (name, is_dir) in &self.entries {
                        let label = if *is_dir { format!("{name}/") } else { name.clone() };
                        let selected = !*is_dir && *name == self.name;
                        let r = ui.selectable_label(selected, label);
                        if *is_dir && r.clicked() {
                            enter = Some(self.dir.join(name));
                        } else if !*is_dir && r.clicked() {
                            pick = Some((name.clone(), false));
                        } else if !*is_dir && r.double_clicked() {
                            pick = Some((name.clone(), true));
                        }
                    }
                    if self.entries.is_empty() && self.error.is_none() {
                        ui.label(format!("No folders or .{} files here.", exts.join(", .")));
                    }
                });
                if let Some(d) = enter {
                    self.go(d);
                }
                let mut go_now = false;
                if let Some((name, double)) = pick {
                    self.name = name;
                    self.confirm_replace = None;
                    go_now = double && !self.purpose.writes();
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Name");
                    let r = ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(300.0));
                    if r.changed() {
                        self.confirm_replace = None;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        go_now = true;
                    }
                });
                if let Some(p) = &self.confirm_replace {
                    ui.colored_label(egui::Color32::from_rgb(190, 110, 0), format!("{} already exists. Press Replace to overwrite it.", p.file_name().unwrap_or_default().to_string_lossy()));
                }
                ui.horizontal(|ui| {
                    let action = if self.confirm_replace.is_some() { "Replace" } else { self.purpose.title() };
                    let ready = !self.name.trim().is_empty();
                    if ui.add_enabled(ready, egui::Button::new(action)).clicked() {
                        go_now = true;
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Some(Outcome::Cancelled);
                    }
                    ui.label(egui::RichText::new(format!(".{}", exts.join("  ."))).weak());
                });
                if go_now && !self.name.trim().is_empty() {
                    let path = resolve(&self.dir, &self.name, exts);
                    if path.is_dir() {
                        self.go(path);
                        self.name.clear();
                    } else if self.purpose.writes() && path.exists() && self.confirm_replace.as_ref() != Some(&path) {
                        self.confirm_replace = Some(path);
                    } else if !self.purpose.writes() && !path.is_file() {
                        self.error = Some(format!("There is no file {}.", path.display()));
                    } else {
                        outcome = Some(Outcome::Chosen(path));
                    }
                }
            });
        if !open {
            outcome = Some(Outcome::Cancelled);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_name_gets_the_default_kind() {
        let d = Path::new("/w");
        assert_eq!(resolve(d, "house", &["daedalus"]), PathBuf::from("/w/house.daedalus"));
        assert_eq!(resolve(d, "house.daedalus", &["daedalus"]), PathBuf::from("/w/house.daedalus"));
        assert_eq!(resolve(d, "part.STL", &["stl", "obj"]), PathBuf::from("/w/part.STL"));
        assert_eq!(resolve(d, "part.v2", &["stl"]), PathBuf::from("/w/part.v2.stl"));
    }

    #[test]
    fn a_folder_lists_folders_first_and_only_fitting_files() {
        let root = std::env::temp_dir().join(format!("daedalus-dlg-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Beta")).unwrap();
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        for f in ["b.daedalus", "A.daedalus", "notes.txt", ".hidden.daedalus"] {
            std::fs::write(root.join(f), "").unwrap();
        }
        let l = list(&root, &["daedalus"]).unwrap();
        let names: Vec<&str> = l.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["alpha", "Beta", "A.daedalus", "b.daedalus"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
