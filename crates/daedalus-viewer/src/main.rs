mod camera;
mod filedlg;
mod render;
mod tools;
mod ui;

use camera::Camera;
use daedalus_core::inspect::{self, DEFAULT_TOLERANCE};
use daedalus_core::{HalfEdgeMesh, Mesh, Model, Plane};
use glam::Vec3;
use render::{LineVertex, Renderer, UiFrame};
use filedlg::{FileDialog, Outcome, Purpose};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tools::{Cursor, Hit, Snap, Tool, Tools, AXIS_COLORS};
use ui::{UiAction, UiView};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

/// How close (in pixels) the cursor must be to a point to snap onto it.
const SNAP_RADIUS: f32 = 12.0;

struct State {
    window: Arc<Window>,
    gpu: Renderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    camera: Camera,
    /// Cursor in physical pixels.
    cursor: (f32, f32),
    over_ui: bool,
    left_down: bool,
    middle_down: bool,
    right_down: bool,
    press_at: (f32, f32),
    modifiers: ModifiersState,
    model: Model,
    /// Set when an import has non-manifold topology: shown, but not editable.
    readonly: Option<Mesh>,
    /// What is on screen: the model, a push/pull preview, or the read-only import.
    display: Mesh,
    tools: Tools,
    /// Sorted face indices flagged by the last inspection.
    flagged: Vec<usize>,
    /// Text typed into the Measurements box, applied on Enter.
    typed: String,
    message: Option<String>,
    last_cursor: Option<Cursor>,
    shown_highlight: Option<usize>,
    /// The document's file (None until it is saved), how many times the whole document was replaced (open, new, repair), and the
    /// (replacement, model state) it was saved or opened at: it is unchanged while those match.
    path: Option<PathBuf>,
    doc: u64,
    saved: (u64, u64),
    dialog: Option<FileDialog>,
    /// "Save changes?" while something would throw the changes away, and what to do once they are saved.
    ask: Option<Pending>,
    after_save: Option<Pending>,
    quit: bool,
    shown_title: String,
}

/// Something that replaces the document or ends the program, held while the person is asked about unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    New,
    Open,
    Import,
    Quit,
}

impl State {
    fn new(window: Arc<Window>, mesh: &Mesh, path: Option<PathBuf>) -> State {
        let gpu = pollster::block_on(Renderer::new(window.clone()));
        let egui_ctx = egui::Context::default();
        egui_ctx.set_visuals(egui::Visuals::light());
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let (model, readonly) = match HalfEdgeMesh::from_mesh(mesh) {
            Ok(h) => (Model::from_mesh(h), None),
            Err(e) => {
                eprintln!("note: model is read-only until repaired with F ({e})");
                (Model::new(), Some(mesh.clone()))
            }
        };
        let camera = match mesh.bounds() {
            Some((lo, hi)) => Camera::fit(Vec3::from(lo), Vec3::from(hi)),
            // An empty model: frame a room-sized area around the origin.
            None => Camera::fit(Vec3::new(-1.0, -1.0, 0.0), Vec3::new(6.0, 6.0, 2.0)),
        };
        let mut s = State {
            window,
            gpu,
            egui_ctx,
            egui_state,
            camera,
            cursor: (0.0, 0.0),
            over_ui: false,
            left_down: false,
            middle_down: false,
            right_down: false,
            press_at: (0.0, 0.0),
            modifiers: ModifiersState::empty(),
            model,
            readonly,
            display: Mesh::new(),
            tools: Tools::default(),
            flagged: Vec::new(),
            typed: String::new(),
            message: None,
            last_cursor: None,
            shown_highlight: None,
            // a .daedalus file opened at start is the document's file; an import is a new, unsaved document
            path: path.filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("daedalus"))),
            doc: 0,
            saved: (0, 0),
            dialog: None,
            ask: None,
            after_save: None,
            quit: false,
            shown_title: String::new(),
        };
        s.refresh();
        s
    }

    fn changed(&self) -> bool {
        (self.doc, self.model.state()) != self.saved
    }

    /// "name — Daedalus", with a dot while there are unsaved changes.
    fn title(&self) -> String {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
        format!("{}{name} — Daedalus", if self.changed() { "• " } else { "" })
    }

    /// What a save or export writes: the model, or a read-only import as it is.
    fn document_mesh(&self) -> Mesh {
        self.readonly.clone().unwrap_or_else(|| self.model.mesh().to_mesh())
    }

    /// Replaces the document (a new one, an opened file, or an import).
    fn replace(&mut self, mesh: &Mesh, path: Option<PathBuf>) {
        let (model, readonly) = match HalfEdgeMesh::from_mesh(mesh) {
            Ok(h) => (Model::from_mesh(h), None),
            Err(e) => {
                eprintln!("note: model is read-only until repaired with F ({e})");
                (Model::new(), Some(mesh.clone()))
            }
        };
        self.model = model;
        self.readonly = readonly;
        self.doc += 1;
        self.path = path;
        // an import is a new, unsaved document; an opened file or an empty one is unchanged
        self.saved = if mesh.positions.is_empty() || self.path.is_some() { (self.doc, self.model.state()) } else { (u64::MAX, 0) };
        self.tools = Tools::default();
        self.flagged.clear();
        self.typed.clear();
        self.camera = match mesh.bounds() {
            Some((lo, hi)) => Camera::fit(Vec3::from(lo), Vec3::from(hi)),
            None => Camera::fit(Vec3::new(-1.0, -1.0, 0.0), Vec3::new(6.0, 6.0, 2.0)),
        };
        self.refresh();
    }

    /// Starts something that would throw away unsaved changes: asks first when there are any.
    fn begin(&mut self, p: Pending) {
        if self.changed() {
            self.ask = Some(p);
        } else {
            self.carry_out(p);
        }
    }

    fn carry_out(&mut self, p: Pending) {
        match p {
            Pending::New => {
                self.replace(&Mesh::new(), None);
                self.message = Some("New model.".into());
            }
            Pending::Open => self.dialog = Some(FileDialog::new(Purpose::Open, self.path.as_deref(), "")),
            Pending::Import => self.dialog = Some(FileDialog::new(Purpose::Import, self.path.as_deref(), "")),
            Pending::Quit => self.quit = true,
        }
    }

    /// Save: to the document's file, or ask where (Save As) when it has none. `then` runs once it is saved.
    fn save(&mut self, then: Option<Pending>) {
        match self.path.clone() {
            Some(p) => {
                if self.save_to(&p) {
                    if let Some(t) = then {
                        self.carry_out(t);
                    }
                }
            }
            None => self.save_as(then),
        }
    }

    fn save_as(&mut self, then: Option<Pending>) {
        self.after_save = then;
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.dialog = Some(FileDialog::new(Purpose::SaveAs, self.path.as_deref(), &name));
    }

    fn save_to(&mut self, path: &Path) -> bool {
        match daedalus_io::export_mesh(&self.document_mesh(), path) {
            Ok(()) => {
                self.path = Some(path.to_path_buf());
                self.saved = (self.doc, self.model.state());
                self.message = Some(format!("Saved {}.", short(path)));
                true
            }
            Err(e) => {
                self.message = Some(format!("Not saved: {e}."));
                false
            }
        }
    }

    fn open_file(&mut self, path: &Path, import: bool) {
        match daedalus_io::import_mesh(path) {
            Ok(m) => {
                self.replace(&m, if import { None } else { Some(path.to_path_buf()) });
                self.message = Some(format!("{} {}.", if import { "Imported" } else { "Opened" }, short(path)));
            }
            Err(e) => self.message = Some(format!("Could not open {}: {e}.", short(path))),
        }
    }

    /// A file chosen in the dialog.
    fn dialog_done(&mut self, purpose: Purpose, path: PathBuf) {
        match purpose {
            Purpose::Open => self.open_file(&path, false),
            Purpose::Import => self.open_file(&path, true),
            Purpose::SaveAs => {
                if self.save_to(&path) {
                    if let Some(t) = self.after_save.take() {
                        self.carry_out(t);
                    }
                }
            }
            Purpose::Export => match daedalus_io::export_mesh(&self.document_mesh(), &path) {
                Ok(()) => self.message = Some(format!("Exported {}.", short(&path))),
                Err(e) => self.message = Some(format!("Not exported: {e}.")),
            },
        }
    }

    fn size(&self) -> (f32, f32) {
        (self.gpu.config.width as f32, self.gpu.config.height as f32)
    }

    /// Rebuilds GPU geometry after an edit, a selection change or a preview step.
    fn refresh(&mut self) {
        let preview = self.tools.preview_mesh(&self.model);
        self.display = match (&self.readonly, &preview) {
            (Some(m), _) => m.clone(),
            (None, Some(p)) => p.to_mesh(),
            (None, None) => self.model.mesh().to_mesh(),
        };
        let highlight = if preview.is_some() { None } else { self.tools.highlight() };
        self.shown_highlight = highlight;
        self.gpu.set_scene(&self.display, highlight, &self.flagged);
        self.window.request_redraw();
    }

    /// The mouse ray, snapped to nearby endpoints and midpoints, else the face
    /// under it, else the ground plane.
    fn resolve_cursor(&self) -> Cursor {
        let (w, h) = self.size();
        let (origin, dir) = self.camera.ray(self.cursor.0, self.cursor.1, w, h);
        let face_hit = self.display.pick_face(origin.into(), dir.into());

        let mut candidates: Vec<(Vec3, Snap)> = Vec::new();
        for (a, b) in self.display.edges() {
            let (pa, pb) = (Vec3::from(self.display.positions[a as usize]), Vec3::from(self.display.positions[b as usize]));
            candidates.push((pa, Snap::Endpoint));
            candidates.push((pb, Snap::Endpoint));
            candidates.push(((pa + pb) * 0.5, Snap::Midpoint));
        }
        candidates.extend(self.tools.pending_points().into_iter().map(|p| (Vec3::from(p), Snap::Endpoint)));

        let visible = |p: Vec3| match face_hit {
            Some((_, t)) => (p - origin).dot(dir) <= t * 1.001 + 1e-4,
            None => true,
        };
        let mut best: Option<(Vec3, Snap, f32)> = None;
        for (p, snap) in candidates {
            let Some((x, y)) = self.camera.project(p, w, h) else { continue };
            let d = ((x - self.cursor.0).powi(2) + (y - self.cursor.1).powi(2)).sqrt();
            // Endpoints win over midpoints at the same distance.
            let rank = d + if snap == Snap::Midpoint { 2.0 } else { 0.0 };
            if d <= SNAP_RADIUS && best.is_none_or(|b| rank < b.2) && visible(p) {
                best = Some((p, snap, rank));
            }
        }

        let face_plane = |f: usize, at: Vec3| Plane::from_point_normal(at.into(), render::newell(&self.display, &self.display.faces[f]));
        let hit = match (best, face_hit) {
            (Some((p, snap, _)), fh) => {
                let on_face = fh.and_then(|(f, _)| {
                    let plane = face_plane(f, p)?;
                    let off = (p - Vec3::from(plane.origin)).dot(Vec3::from(plane.normal)).abs();
                    let face_point = origin + dir * fh?.1;
                    let off_face = (p - face_point).dot(Vec3::from(plane.normal)).abs();
                    (off < 1e-4 && off_face < 1e-3 * (1.0 + p.length())).then_some((f, plane))
                });
                let (face, plane) = match on_face {
                    Some((f, plane)) => (Some(f), plane),
                    None if p.z.abs() < 1e-5 => (None, Plane::ground()),
                    None => (None, Plane { origin: p.into(), ..Plane::ground() }),
                };
                Some(Hit { point: p.into(), plane, face, snap })
            }
            (None, Some((f, t))) => {
                let p = origin + dir * t;
                face_plane(f, p).map(|plane| Hit { point: p.into(), plane, face: Some(f), snap: Snap::OnFace })
            }
            (None, None) => Plane::ground()
                .intersect_ray(origin.into(), dir.into())
                .map(|p| Hit { point: p, plane: Plane::ground(), face: None, snap: Snap::Ground }),
        };
        Cursor { origin: origin.into(), dir: dir.into(), hit }
    }

    fn mouse_moved(&mut self) {
        if self.over_ui {
            self.last_cursor = None;
            return;
        }
        let c = self.resolve_cursor();
        self.last_cursor = Some(c);
        self.tools.hover(&c);
        if self.tools.preview_mesh(&self.model).is_some() || self.tools.highlight() != self.shown_highlight {
            self.refresh();
        }
        self.window.request_redraw();
    }

    fn click(&mut self) {
        let c = self.resolve_cursor();
        if self.readonly.is_some() && self.tools.tool.edits() {
            self.message = Some("This model is read-only; press F to repair it.".into());
            return;
        }
        self.message = None;
        let result = match &self.readonly {
            // Selection still works on a read-only import.
            Some(_) => {
                self.tools.selected = c.hit.and_then(|h| h.face);
                Ok(())
            }
            None => self.tools.click(&mut self.model, &c),
        };
        if let Err(e) = result {
            self.message = Some(e);
        }
        self.after_edit();
    }

    fn after_edit(&mut self) {
        self.flagged.clear(); // face indices change with every edit
        self.refresh();
        self.mouse_moved();
    }

    fn enter_typed(&mut self) {
        let text = std::mem::take(&mut self.typed);
        if self.readonly.is_some() {
            return;
        }
        self.message = self.tools.type_entry(&mut self.model, &text).err();
        self.after_edit();
    }

    fn set_tool(&mut self, tool: Tool) {
        self.tools.set_tool(tool);
        self.typed.clear();
        self.message = None;
        self.refresh();
    }

    fn history(&mut self, undo: bool) {
        if self.readonly.is_some() {
            return;
        }
        let changed = if undo { self.model.undo() } else { self.model.redo() };
        if changed {
            self.tools.model_changed();
            self.after_edit();
        }
    }

    fn erase_selected(&mut self) {
        let Some(face) = self.tools.selected.filter(|_| self.readonly.is_none()) else { return };
        if let Err(e) = self.model.erase_face(face) {
            self.message = Some(e.to_string());
        }
        self.tools.model_changed();
        self.after_edit();
    }

    /// Reports solid problems on stdout and marks the offending faces red.
    fn inspect(&mut self) {
        let report = inspect::inspect(&self.display, DEFAULT_TOLERANCE);
        println!("--- solid inspector ---\n{}", report.summary());
        self.message = Some(if report.is_solid() {
            "Solid Inspector: this is a valid solid.".into()
        } else {
            "Solid Inspector: problem faces shown in red (details on the console).".into()
        });
        self.flagged = report.flagged_faces();
        self.refresh();
    }

    /// Repairs the model. A repaired import that is manifold becomes editable.
    fn repair(&mut self) {
        let (fixed, summary) = inspect::repair(&self.display, DEFAULT_TOLERANCE);
        if !summary.changed_anything() {
            self.message = Some("Repair: nothing to fix.".into());
            return;
        }
        println!("repair: {summary:?}");
        match HalfEdgeMesh::from_mesh(&fixed) {
            Ok(h) => {
                self.model = Model::from_mesh(h);
                self.readonly = None;
                self.message = Some("Repair: the model is now editable.".into());
            }
            Err(e) => {
                self.readonly = Some(fixed);
                self.message = Some(format!("Repair: still read-only ({e})."));
            }
        }
        // the repaired model replaces the document: a change to save
        self.doc += 1;
        self.tools.model_changed();
        self.flagged.clear();
        self.refresh();
    }

    fn zoom_extents(&mut self) {
        if let Some((lo, hi)) = self.display.bounds() {
            self.camera.zoom_extents(Vec3::from(lo), Vec3::from(hi));
        }
        self.window.request_redraw();
    }

    fn apply(&mut self, action: UiAction) {
        match action {
            UiAction::Tool(t) => self.set_tool(t),
            UiAction::ZoomExtents => self.zoom_extents(),
            UiAction::Top => self.camera.top(),
            UiAction::Iso => self.camera.iso(),
            UiAction::ToggleOrtho => self.camera.ortho = !self.camera.ortho,
            UiAction::Undo => self.history(true),
            UiAction::Redo => self.history(false),
            UiAction::Inspect => self.inspect(),
            UiAction::Repair => self.repair(),
            UiAction::New => self.begin(Pending::New),
            UiAction::Open => self.begin(Pending::Open),
            UiAction::Save => self.save(None),
            UiAction::SaveAs => self.save_as(None),
            UiAction::Import => self.begin(Pending::Import),
            UiAction::Export => {
                let stem = self.path.as_ref().and_then(|p| p.file_stem()).map(|n| format!("{}.stl", n.to_string_lossy())).unwrap_or_default();
                self.dialog = Some(FileDialog::new(Purpose::Export, self.path.as_deref(), &stem));
            }
        }
        self.window.request_redraw();
    }

    fn key(&mut self, key: &Key) {
        let ctrl = self.modifiers.control_key();
        match key {
            Key::Named(NamedKey::Escape) => {
                if self.typed.is_empty() {
                    self.tools.cancel();
                    self.message = None;
                } else {
                    self.typed.clear();
                }
                self.refresh();
            }
            Key::Named(NamedKey::Enter) if !self.typed.is_empty() => self.enter_typed(),
            Key::Named(NamedKey::Backspace) => {
                self.typed.pop();
            }
            Key::Named(NamedKey::Delete) => self.erase_selected(),
            Key::Named(NamedKey::Space) => self.set_tool(Tool::Select),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("n") => self.begin(Pending::New),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("o") => self.begin(Pending::Open),
            Key::Character(c) if ctrl && self.modifiers.shift_key() && c.eq_ignore_ascii_case("s") => self.save_as(None),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("s") => self.save(None),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("z") => self.history(true),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("y") => self.history(false),
            Key::Character(_) if ctrl => {}
            Key::Character(c) if self.modifiers.shift_key() && c.eq_ignore_ascii_case("z") => self.zoom_extents(),
            // Digits go to the Measurements box without clicking it, as in SketchUp.
            Key::Character(c) if c.chars().all(|ch| ch.is_ascii_digit() || ".,;-".contains(ch)) => {
                self.typed.push_str(c);
            }
            Key::Character(c) if c.eq_ignore_ascii_case("s") && !self.typed.is_empty() => self.typed.push('s'),
            Key::Character(c) if c.eq_ignore_ascii_case("i") => self.inspect(),
            Key::Character(c) if c.eq_ignore_ascii_case("f") => self.repair(),
            Key::Character(c) => {
                if let Some(t) = Tool::from_key(c) {
                    if self.readonly.is_some() && t.edits() {
                        self.message = Some("This model is read-only; press F to repair it.".into());
                    } else {
                        self.set_tool(t);
                    }
                }
            }
            _ => {}
        }
        self.window.request_redraw();
    }

    /// Drawing axes: solid on the positive side, faded on the negative side.
    fn axes(&self) -> Vec<LineVertex> {
        let reach = self.camera.distance * 40.0;
        let mut out = Vec::new();
        for (i, color) in AXIS_COLORS.iter().take(3).enumerate() {
            let faded = [color[0] * 0.4 + 0.55, color[1] * 0.4 + 0.55, color[2] * 0.4 + 0.55, 1.0];
            for (sign, color) in [(1.0, *color), (-1.0, faded)] {
                let mut end = [0.0; 3];
                end[i] = reach * sign;
                out.push(LineVertex { position: [0.0; 3], color });
                out.push(LineVertex { position: end, color });
            }
        }
        out
    }

    fn overlay(&self) -> Vec<LineVertex> {
        self.tools
            .preview_lines()
            .into_iter()
            .flat_map(|(a, b, color)| [LineVertex { position: a, color }, LineVertex { position: b, color }])
            .collect()
    }

    fn render(&mut self) {
        let ctx = self.egui_ctx.clone();
        let (w, h) = self.size();
        let ppp = ctx.pixels_per_point();
        let marker = match self.tools.tool {
            Tool::Line | Tool::Rectangle | Tool::Circle | Tool::Polygon if !self.over_ui => self
                .last_cursor
                .and_then(|c| self.tools.marker(&c))
                .and_then(|(p, snap)| self.camera.project(Vec3::from(p), w, h).map(|(x, y)| (egui::pos2(x / ppp, y / ppp), snap))),
            _ => None,
        };
        let (label, current) = self.tools.measurement();
        let view = UiView {
            tool: self.tools.tool,
            editable: self.readonly.is_none(),
            ortho: self.camera.ortho,
            can_undo: self.model.can_undo(),
            can_redo: self.model.can_redo(),
            status: self.tools.status(),
            message: self.message.as_deref(),
            measure_label: label,
            measure_value: if self.typed.is_empty() { &current } else { &self.typed },
            typing: !self.typed.is_empty(),
            marker,
        };
        let raw = self.egui_state.take_egui_input(&self.window);
        let mut actions = Vec::new();
        // the file dialog and the "save changes?" question are drawn over the chrome; taken out while drawn
        let mut dialog = self.dialog.take();
        let mut dialog_out = None;
        let ask = self.ask;
        let mut answer: Option<&'static str> = None;
        let doc_name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
        let out = ctx.run(raw, |ctx| {
            actions = ui::draw(ctx, &view);
            if let Some(d) = dialog.as_mut() {
                dialog_out = d.show(ctx).map(|o| (d.purpose, o));
            }
            if ask.is_some() {
                egui::Window::new("Save changes?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("{doc_name} has changes that are not saved."));
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            answer = Some("save");
                        }
                        if ui.button("Don't Save").clicked() {
                            answer = Some("discard");
                        }
                        if ui.button("Cancel").clicked() {
                            answer = Some("cancel");
                        }
                    });
                });
            }
        });
        self.dialog = if dialog_out.is_some() { None } else { dialog };
        self.egui_state.handle_platform_output(&self.window, out.platform_output);
        let ui = UiFrame {
            primitives: ctx.tessellate(out.shapes, out.pixels_per_point),
            textures: out.textures_delta,
            pixels_per_point: out.pixels_per_point,
        };
        let (axes, overlay) = (self.axes(), self.overlay());
        self.gpu.render(self.camera.view_proj(w / h), &axes, &overlay, ui);

        if out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay.is_zero()) {
            self.window.request_redraw();
        }
        for a in actions {
            self.apply(a);
        }
        if let Some((purpose, o)) = dialog_out {
            match o {
                Outcome::Chosen(path) => self.dialog_done(purpose, path),
                Outcome::Cancelled => self.after_save = None,
            }
        }
        if let (Some(p), Some(a)) = (ask, answer) {
            self.ask = None;
            match a {
                "save" => self.save(Some(p)),
                "discard" => self.carry_out(p),
                _ => {}
            }
        }
        let title = self.title();
        if title != self.shown_title {
            self.window.set_title(&title);
            self.shown_title = title;
        }
        if self.dialog.is_some() || self.ask.is_some() {
            self.window.request_redraw();
        }
    }
}

/// A file's name for a message (the whole path would run into the Measurements box).
fn short(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

struct App {
    mesh: Mesh,
    path: Option<PathBuf>,
    title: String,
    state: Option<State>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title(self.title.clone());
        // App id / WM class matching packaging/daedalus.desktop, so the taskbar shows its icon.
        #[cfg(target_os = "linux")]
        let attrs = {
            let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, "daedalus", "daedalus");
            winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "daedalus", "daedalus")
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.state = Some(State::new(window, &self.mesh, self.path.clone()));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let response = s.egui_state.on_window_event(&s.window, &event);
        if response.repaint {
            s.window.request_redraw();
        }
        match event {
            // closing with unsaved changes asks first, as SketchUp does
            WindowEvent::CloseRequested => {
                s.begin(Pending::Quit);
                s.window.request_redraw();
            }
            WindowEvent::Resized(size) => {
                s.gpu.resize(size.width, size.height);
                s.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                s.render();
                if s.quit {
                    event_loop.exit();
                }
            }
            WindowEvent::ModifiersChanged(m) => s.modifiers = m.state(),
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                // Presses that land on the toolbar or status bar belong to egui.
                if down && s.egui_ctx.is_pointer_over_area() {
                    return;
                }
                match button {
                    MouseButton::Left => {
                        if down {
                            s.press_at = s.cursor;
                            s.left_down = true;
                        } else if s.left_down {
                            s.left_down = false;
                            // A release close to the press point is a click, not a drag.
                            let (dx, dy) = (s.cursor.0 - s.press_at.0, s.cursor.1 - s.press_at.1);
                            if dx * dx + dy * dy < 16.0 && !s.tools.tool.navigates() {
                                s.click();
                            }
                        }
                    }
                    MouseButton::Middle => s.middle_down = down,
                    MouseButton::Right => s.right_down = down,
                    _ => {}
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if !s.egui_ctx.wants_keyboard_input() {
                    s.key(&event.logical_key);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = (position.x as f32, position.y as f32);
                let (dx, dy) = (x - s.cursor.0, y - s.cursor.1);
                s.cursor = (x, y);
                s.over_ui = s.egui_ctx.is_pointer_over_area();
                let tool = s.tools.tool;
                // Middle-drag orbits and Shift+middle-drag pans, as in SketchUp;
                // right-drag pans until there is a context menu.
                let orbit = (s.middle_down && !s.modifiers.shift_key()) || (s.left_down && tool == Tool::Orbit);
                let pan = (s.middle_down && s.modifiers.shift_key()) || s.right_down || (s.left_down && tool == Tool::Pan);
                if orbit {
                    s.camera.orbit(dx, dy);
                } else if pan {
                    s.camera.pan(dx, dy);
                } else if s.left_down && tool == Tool::Zoom {
                    s.camera.zoom(-dy * 0.05, None);
                } else {
                    s.mouse_moved();
                    return;
                }
                s.window.request_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                s.last_cursor = None;
                s.window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if s.over_ui {
                    return;
                }
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                // Zoom toward the point under the cursor.
                let focus = s.resolve_cursor().hit.map(|h| Vec3::from(h.point));
                s.camera.zoom(amount, focus);
                s.mouse_moved();
                s.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    let (mesh, title, path) = match std::env::args().nth(1) {
        Some(arg) => {
            let path = PathBuf::from(&arg);
            match daedalus_io::import_mesh(&path) {
                Ok(m) => (m, format!("Daedalus - {arg}"), Some(path)),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => (Mesh::new(), "Daedalus".to_string(), None),
    };
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App { mesh, path, title, state: None };
    event_loop.run_app(&mut app).expect("run app");
}
