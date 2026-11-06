mod camera;
mod filedlg;
mod frame;
mod theme;
mod icons;
mod settings;
mod render;
mod tools;
mod ui;

use camera::Camera;
use daedalus_core::inspect::{self, DEFAULT_TOLERANCE};
use daedalus_core::{HalfEdgeMesh, Mesh, Model, Plane};
use glam::Vec3;
use render::{FaceStyle, LineVertex, Renderer, Style, UiFrame};
use filedlg::{FileDialog, Outcome, Purpose};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tools::{Cursor, Guide, Hit, Snap, Tool, Tools, AXIS_COLORS};
use ui::{UiAction, UiView};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

/// How close (in pixels) the cursor must be to a point to snap onto it.
const SNAP_RADIUS: f32 = 12.0;

/// Position Camera: eye height above the point clicked (SketchUp's 5' 6").
const EYE_HEIGHT: f32 = 1.676;

/// How many earlier views Camera > Previous can go back through.
const VIEW_HISTORY: usize = 50;

const GUIDE_COLOR: [f32; 4] = [0.35, 0.35, 0.4, 1.0];

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
    /// The pointer is over the window (for the resize edges, which also lie over the chrome).
    pointer_in: bool,
    shown_highlight: Vec<usize>,
    /// Display style (View > Face Style, Back Edges; remembered in settings.conf).
    style: Style,
    /// Earlier views, for Camera > Previous; when the last one was saved (a run of wheel steps counts as one view change).
    views: Vec<Camera>,
    view_saved_at: std::time::Instant,
    /// Clicks in a row at about the same place: 2 is a double-click, 3 a triple-click (Select tool).
    clicks: (u32, std::time::Instant, (f32, f32)),
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
    /// View > Toolbar: down the left side instead of across the top (remembered in settings.conf).
    toolbar_left: bool,
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
    fn new(window: Arc<Window>, mesh: &Mesh, path: Option<PathBuf>, dark: bool) -> State {
        let gpu = pollster::block_on(Renderer::new(window.clone()));
        let egui_ctx = egui::Context::default();
        egui_ctx.set_visuals(theme::visuals(dark));
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
                eprintln!("note: model is read-only until repaired with Tools > Repair Solid ({e})");
                (Model::new(), Some(mesh.clone()))
            }
        };
        let camera = match mesh.bounds() {
            Some((lo, hi)) => Camera::fit(Vec3::from(lo), Vec3::from(hi)),
            // An empty model: frame a room-sized area around the origin.
            None => Camera::fit(Vec3::new(-1.0, -1.0, 0.0), Vec3::new(6.0, 6.0, 2.0)),
        };
        let mut tools = Tools::default();
        tools.units = settings::load_units();
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
            tools,
            flagged: Vec::new(),
            typed: String::new(),
            message: None,
            last_cursor: None,
            pointer_in: false,
            shown_highlight: Vec::new(),
            style: Style {
                faces: settings::load("style").as_deref().and_then(FaceStyle::from_key).unwrap_or(FaceStyle::Shaded),
                back_edges: settings::load("back_edges").as_deref() == Some("on"),
            },
            views: Vec::new(),
            view_saved_at: std::time::Instant::now(),
            clicks: (0, std::time::Instant::now(), (0.0, 0.0)),
            // a .daedalus file opened at start is the document's file; an import is a new, unsaved document
            path: path.filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("daedalus"))),
            doc: 0,
            saved: (0, 0),
            dialog: None,
            ask: None,
            after_save: None,
            quit: false,
            shown_title: String::new(),
            // the Large Tool Set down the left side unless across the top was chosen
            toolbar_left: settings::load("toolbar").as_deref() != Some("top"),
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
                eprintln!("note: model is read-only until repaired with Tools > Repair Solid ({e})");
                (Model::new(), Some(mesh.clone()))
            }
        };
        self.model = model;
        self.readonly = readonly;
        self.doc += 1;
        self.path = path;
        // an import is a new, unsaved document; an opened file or an empty one is unchanged
        self.saved = if mesh.positions.is_empty() || self.path.is_some() { (self.doc, self.model.state()) } else { (u64::MAX, 0) };
        let units = self.tools.units;
        self.tools = Tools::default();
        self.tools.units = units;
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
        let highlight = if preview.is_some() { Vec::new() } else { self.tools.highlight() };
        self.gpu.set_scene(&self.display, &highlight, &self.flagged);
        self.shown_highlight = highlight;
        self.window.request_redraw();
    }

    /// The mouse ray, snapped to nearby endpoints and midpoints, else the face
    /// under it, else the ground plane.
    fn resolve_cursor(&self) -> Cursor {
        let (w, h) = self.size();
        let (origin, dir) = self.camera.ray(self.cursor.0, self.cursor.1, w, h);
        let face_hit = self.display.pick_face(origin.into(), dir.into());

        let mut candidates: Vec<(Vec3, Snap)> = Vec::new();
        for (a, b) in self.display.visible_edges() {
            let (pa, pb) = (Vec3::from(self.display.positions[a as usize]), Vec3::from(self.display.positions[b as usize]));
            candidates.push((pa, Snap::Endpoint));
            candidates.push((pb, Snap::Endpoint));
            candidates.push(((pa + pb) * 0.5, Snap::Midpoint));
        }
        candidates.extend(self.tools.pending_points().into_iter().map(|p| (Vec3::from(p), Snap::Endpoint)));
        // the origin, and the point of each axis nearest the mouse (as SketchUp snaps onto the axes)
        candidates.push((Vec3::ZERO, Snap::Origin));
        for (i, axis) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
            let b = axis.dot(dir);
            let denom = 1.0 - b * b;
            if denom.abs() > 1e-6 {
                let s = (b * dir.dot(-origin) - axis.dot(-origin)) / denom;
                candidates.push((axis * s, Snap::OnAxis(i as u8)));
            }
        }
        // guide points, and the point of each guide line nearest the mouse
        for g in &self.tools.guides {
            match *g {
                Guide::Point(p) => candidates.push((Vec3::from(p), Snap::OnGuide)),
                Guide::Line(p, d) => {
                    let (p, d) = (Vec3::from(p), Vec3::from(d));
                    let b = d.dot(dir);
                    let denom = 1.0 - b * b;
                    if denom.abs() > 1e-6 {
                        let w0 = p - origin;
                        let s = (b * dir.dot(w0) - d.dot(w0)) / denom;
                        candidates.push((p + d * s, Snap::OnGuide));
                    }
                }
            }
        }

        let visible = |p: Vec3| match face_hit {
            Some((_, t)) => (p - origin).dot(dir) <= t * 1.001 + 1e-4,
            None => true,
        };
        let mut best: Option<(Vec3, Snap, f32)> = None;
        for (p, snap) in candidates {
            let Some((x, y)) = self.camera.project(p, w, h) else { continue };
            let d = ((x - self.cursor.0).powi(2) + (y - self.cursor.1).powi(2)).sqrt();
            // Endpoints win over midpoints, and both over a guide, at the same distance.
            let rank = d + match snap {
                Snap::Midpoint => 2.0,
                Snap::OnGuide => 3.0,
                Snap::OnAxis(_) => 4.0,
                _ => 0.0,
            };
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
                Some(Hit { point: p.into(), plane, face, line: None, snap })
            }
            (None, Some((f, t))) => {
                let p = origin + dir * t;
                face_plane(f, p).map(|plane| Hit { point: p.into(), plane, face: Some(f), line: None, snap: Snap::OnFace })
            }
            (None, None) => Plane::ground()
                .intersect_ray(origin.into(), dir.into())
                .map(|p| Hit { point: p, plane: Plane::ground(), face: None, line: None, snap: Snap::Ground }),
        };
        // the loose line nearest the mouse on screen, for the Eraser
        let line = self.display.lines.iter().enumerate().filter_map(|(i, &[a, b])| {
            let pa = self.camera.project(Vec3::from(self.display.positions[a as usize]), w, h)?;
            let pb = self.camera.project(Vec3::from(self.display.positions[b as usize]), w, h)?;
            let d = segment_distance(self.cursor, pa, pb);
            (d <= SNAP_RADIUS * 0.6).then_some((i, d))
        });
        let line = line.min_by(|x, y| x.1.total_cmp(&y.1)).map(|(i, _)| i);
        let hit = hit.map(|h| Hit { line, ..h });
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
            self.message = Some("This model is read-only; use Tools > Repair Solid to repair it.".into());
            return;
        }
        self.message = None;
        // double- and triple-clicks: within half a second, a few pixels apart
        let now = std::time::Instant::now();
        let (n, at, pos) = self.clicks;
        let near = (pos.0 - self.cursor.0).abs() < 5.0 && (pos.1 - self.cursor.1).abs() < 5.0;
        let count = if near && now.duration_since(at).as_millis() < 500 { n + 1 } else { 1 };
        self.clicks = (count, now, self.cursor);
        // Paint Bucket with Alt: pick up the face's colour instead of painting
        if self.tools.tool == Tool::PaintBucket && self.modifiers.alt_key() {
            if let Some(f) = c.hit.and_then(|h| h.face) {
                self.tools.paint_color = self.display.color_of(f);
            }
            return;
        }
        if self.tools.tool == Tool::PositionCamera {
            if let Some(h) = c.hit {
                self.remember_view();
                self.camera.stand_at(Vec3::from(h.point), EYE_HEIGHT);
                self.set_tool(Tool::LookAround);
            }
            return;
        }
        let result = match (&self.readonly, self.tools.tool) {
            // Select: Ctrl adds, Shift toggles, a triple-click takes everything joined (works on a read-only import too)
            (_, Tool::Select) => {
                let face = c.hit.and_then(|h| h.face);
                let (add, toggle) = (self.modifiers.control_key(), self.modifiers.shift_key());
                self.tools.select(&self.model, face, add, toggle, count >= 3);
                Ok(())
            }
            (Some(_), _) => Ok(()),
            (None, _) => self.tools.click(&mut self.model, &c),
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
        if self.readonly.is_some() || self.tools.selected.is_empty() {
            return;
        }
        if let Err(e) = self.model.erase_faces(&self.tools.selected) {
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
        // and coplanar faces merged: an import's triangle pairs become the faces they were drawn as
        let (fixed, _) = daedalus_core::merge::merge_coplanar(&fixed, daedalus_core::model::MERGE_TOLERANCE);
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

    fn merge_coplanar(&mut self) {
        if self.readonly.is_some() {
            self.message = Some("This model is read-only; use Tools > Repair Solid to repair it.".into());
            return;
        }
        match self.model.merge_coplanar() {
            Ok(0) => self.message = Some("No coplanar faces to merge.".into()),
            Ok(n) => {
                self.message = Some(format!("Merged: {n} face{} fewer.", if n == 1 { "" } else { "s" }));
                self.tools.model_changed();
                self.refresh();
            }
            Err(e) => self.message = Some(format!("Not merged: {e}.")),
        }
    }

    /// Keeps the current view for Camera > Previous (not when it is the one kept last).
    fn remember_view(&mut self) {
        if self.views.last() != Some(&self.camera) {
            if self.views.len() >= VIEW_HISTORY {
                self.views.remove(0);
            }
            self.views.push(self.camera);
        }
        self.view_saved_at = std::time::Instant::now();
    }

    /// A standard view (Top, Front, Iso, ...): the previous one can be gone back to.
    fn view(&mut self, f: fn(&mut Camera)) {
        self.remember_view();
        f(&mut self.camera);
    }

    fn previous_view(&mut self) {
        match self.views.pop() {
            Some(c) => self.camera = c,
            None => self.message = Some("No earlier view.".into()),
        }
        self.window.request_redraw();
    }

    fn zoom_extents(&mut self) {
        self.remember_view();
        if let Some((lo, hi)) = self.display.bounds() {
            self.camera.zoom_extents(Vec3::from(lo), Vec3::from(hi));
        }
        self.window.request_redraw();
    }

    /// Whether the window is maximized. The window's own state is not enough: Hyprland reports a
    /// floating window as maximized (seen on 0.57, 2026-11-06), so it must also fill its screen.
    fn maximized(&self) -> bool {
        let size = self.window.outer_size();
        let screen = self.window.current_monitor().map(|m| m.size());
        self.window.is_maximized() && screen.is_none_or(|m| frame::fills(size.width, size.height, m.width, m.height))
    }

    /// The window edge under the pointer, when the window draws its own frame and is not maximized.
    fn resize_edge(&self) -> Option<winit::window::ResizeDirection> {
        if !frame::OWN_FRAME || self.maximized() || self.window.fullscreen().is_some() || !self.pointer_in {
            return None;
        }
        let size = self.window.inner_size();
        frame::edge(self.cursor, (size.width, size.height), self.window.scale_factor())
    }

    fn apply(&mut self, action: UiAction) {
        match action {
            UiAction::Tool(t) => self.set_tool(t),
            UiAction::ZoomExtents => self.zoom_extents(),
            UiAction::Top => self.view(Camera::top),
            UiAction::Iso => self.view(Camera::iso),
            UiAction::Front => self.view(Camera::front),
            UiAction::Back => self.view(Camera::back),
            UiAction::Right => self.view(Camera::right_side),
            UiAction::Left => self.view(Camera::left_side),
            UiAction::Previous => self.previous_view(),
            UiAction::FaceStyle(f) => {
                self.style.faces = f;
                settings::save("style", f.key());
            }
            UiAction::PaintColor(c) => self.tools.paint_color = c,
            UiAction::SectionCuts => self.tools.section_cuts = !self.tools.section_cuts,
            UiAction::DeleteSection => self.tools.section = None,
            UiAction::DeleteDimensions => self.tools.delete_dims(),
            UiAction::BackEdges => {
                self.style.back_edges = !self.style.back_edges;
                settings::save("back_edges", if self.style.back_edges { "on" } else { "off" });
            }
            UiAction::SelectAll => {
                self.tools.select_all(&self.model);
                self.refresh();
            }
            UiAction::DeleteGuides => self.tools.delete_guides(),
            UiAction::ToggleOrtho => self.camera.ortho = !self.camera.ortho,
            UiAction::Undo => self.history(true),
            UiAction::Redo => self.history(false),
            UiAction::Inspect => self.inspect(),
            UiAction::Repair => self.repair(),
            UiAction::MergeCoplanar => self.merge_coplanar(),
            UiAction::ToolbarLeft(left) => {
                self.toolbar_left = left;
                settings::save("toolbar", if left { "left" } else { "top" });
            }
            UiAction::Units(u) => {
                self.tools.units = u;
                settings::save_units(u);
                self.message = Some(format!("Lengths in {}.", u.name().to_lowercase()));
            }
            UiAction::New => self.begin(Pending::New),
            UiAction::Open => self.begin(Pending::Open),
            UiAction::Save => self.save(None),
            UiAction::SaveAs => self.save_as(None),
            UiAction::Import => self.begin(Pending::Import),
            UiAction::MoveWindow => {
                let _ = self.window.drag_window();
            }
            UiAction::ToggleMaximize => self.window.set_maximized(!self.maximized()),
            UiAction::Minimize => self.window.set_minimized(true),
            UiAction::Close => self.begin(Pending::Quit),
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
            // while a value is being typed, a space belongs to it (4' 6 1/2")
            Key::Named(NamedKey::Space) if !self.typed.is_empty() => self.typed.push(' '),
            Key::Named(NamedKey::Space) => self.set_tool(Tool::Select),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("n") => self.begin(Pending::New),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("o") => self.begin(Pending::Open),
            Key::Character(c) if ctrl && self.modifiers.shift_key() && c.eq_ignore_ascii_case("s") => self.save_as(None),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("s") => self.save(None),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("z") => self.history(true),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("y") => self.history(false),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("a") => self.apply(UiAction::SelectAll),
            // Walk: the arrow keys step and turn
            Key::Named(NamedKey::ArrowUp) if self.tools.tool == Tool::Walk => self.camera.walk(0.3),
            Key::Named(NamedKey::ArrowDown) if self.tools.tool == Tool::Walk => self.camera.walk(-0.3),
            Key::Named(NamedKey::ArrowLeft) if self.tools.tool == Tool::Walk => self.camera.look_around(40.0, 0.0),
            Key::Named(NamedKey::ArrowRight) if self.tools.tool == Tool::Walk => self.camera.look_around(-40.0, 0.0),
            Key::Character(_) if ctrl => {}
            Key::Character(c) if self.modifiers.shift_key() && c.eq_ignore_ascii_case("z") => self.zoom_extents(),
            // Digits go to the Measurements box without clicking it, as in SketchUp.
            Key::Character(c) if c.chars().all(|ch| ch.is_ascii_digit() || ".,;-".contains(ch)) => {
                self.typed.push_str(c);
            }
            // once a value is being typed, units and fractions go to it too, not to the tool shortcuts (as in SketchUp)
            Key::Character(c) if !self.typed.is_empty() && c.chars().all(|ch| "smcikft'\"/".contains(ch.to_ascii_lowercase())) => {
                self.typed.push_str(c)
            }
            Key::Character(c) if c.eq_ignore_ascii_case("s") && !self.typed.is_empty() => self.typed.push('s'),
            Key::Character(c) if c.eq_ignore_ascii_case("i") => self.inspect(),
            Key::Character(c) => {
                if let Some(t) = Tool::from_key(c) {
                    if self.readonly.is_some() && t.edits() {
                        self.message = Some("This model is read-only; use Tools > Repair Solid to repair it.".into());
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
        // dimensions, in dark grey
        for (a, b, color) in self.tools.dim_lines([0.15, 0.15, 0.18, 1.0]) {
            out.push(LineVertex { position: a, color });
            out.push(LineVertex { position: b, color });
        }
        // guides: lines reach as far as the axes; points are small crosses
        let guides = self.tools.guides.iter().copied().chain(self.tools.guide_preview());
        let cross = self.camera.distance * 0.008;
        for g in guides {
            let mut seg = |a: Vec3, b: Vec3| {
                out.push(LineVertex { position: a.into(), color: GUIDE_COLOR });
                out.push(LineVertex { position: b.into(), color: GUIDE_COLOR });
            };
            match g {
                Guide::Line(p, d) => {
                    let (p, d) = (Vec3::from(p), Vec3::from(d));
                    // dashed: pieces along the line
                    let n = 400;
                    let step = 2.0 * reach / n as f32;
                    for i in (0..n).step_by(2) {
                        let a = p + d * (-reach + step * i as f32);
                        seg(a, a + d * step);
                    }
                }
                Guide::Point(p) => {
                    let p = Vec3::from(p);
                    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                        seg(p - axis * cross, p + axis * cross);
                    }
                }
            }
        }
        out
    }

    fn overlay(&self) -> Vec<LineVertex> {
        let size = self.camera.distance * 0.6;
        self.tools
            .preview_lines()
            .into_iter()
            .chain(self.tools.section_outline(size))
            .flat_map(|(a, b, color)| [LineVertex { position: a, color }, LineVertex { position: b, color }])
            .collect()
    }

    fn render(&mut self) {
        let ctx = self.egui_ctx.clone();
        let (w, h) = self.size();
        let ppp = ctx.pixels_per_point();
        let marker = match self.tools.tool {
            Tool::Line
            | Tool::Rectangle
            | Tool::RotatedRectangle
            | Tool::Circle
            | Tool::Polygon
            | Tool::Pie
            | Tool::Arc
            | Tool::TwoPointArc
            | Tool::ThreePointArc
            | Tool::Freehand
            | Tool::Move
            | Tool::Rotate
            | Tool::TapeMeasure
            | Tool::Protractor
                if !self.over_ui =>
            {
                self

                .last_cursor
                .and_then(|c| self.tools.marker(&c))
                .and_then(|(p, snap)| self.camera.project(Vec3::from(p), w, h).map(|(x, y)| (egui::pos2(x / ppp, y / ppp), snap)))
            }
            _ => None,
        };
        let (label, current) = self.tools.measurement();
        let view = UiView {
            tool: self.tools.tool,
            units: self.tools.units,
            toolbar_left: self.toolbar_left,
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
            style: self.style,
            paint_color: self.tools.paint_color,
            section: self.tools.section.map(|_| self.tools.section_cuts),
            labels: self
                .tools
                .dim_labels()
                .into_iter()
                .filter_map(|(p, t)| self.camera.project(Vec3::from(p), w, h).map(|(x, y)| (egui::pos2(x / ppp, y / ppp), t)))
                .collect(),
            can_go_back: !self.views.is_empty(),
            own_frame: frame::OWN_FRAME,
            maximized: self.maximized(),
            title: &self.shown_title,
        };
        // the resize handle under the pointer (own frame only, not while maximized)
        let resize_at = self.resize_edge();
        let zoom_rect = (self.left_down && self.tools.tool == Tool::ZoomWindow).then(|| {
            egui::Rect::from_two_pos(egui::pos2(self.press_at.0 / ppp, self.press_at.1 / ppp), egui::pos2(self.cursor.0 / ppp, self.cursor.1 / ppp))
        });
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
            if let Some(d) = resize_at {
                ctx.set_cursor_icon(frame::cursor(d));
            }
            // Zoom Window: the rectangle being dragged
            if let Some(r) = zoom_rect {
                ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("zoom-window"))).rect_stroke(
                    r,
                    0.0,
                    egui::Stroke::new(1.0_f32, egui::Color32::from_gray(40)),
                    egui::StrokeKind::Inside,
                );
            }
            if view.own_frame && !view.maximized {
                // a hairline around the window, so its edge shows without a system frame
                let rect = ctx.screen_rect().shrink(0.5);
                ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("window-outline")))
                    .rect_stroke(rect, 0.0, egui::Stroke::new(1.0_f32, egui::Color32::from_gray(150)), egui::StrokeKind::Inside);
            }
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
        // edges about 1.5 points wide, like SketchUp's default
        let line_width = 1.4 * self.window.scale_factor() as f32;
        let section = self.tools.section.filter(|_| self.tools.section_cuts);
        self.gpu.render(self.camera.view_proj(w / h), self.style, line_width, section, &axes, &overlay, ui);

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

/// Events from outside the window: the desktop switched between light and dark.
#[derive(Debug, Clone, Copy)]
enum AppEvent {
    Theme { dark: bool },
}

struct App {
    proxy: EventLoopProxy<AppEvent>,
    mesh: Mesh,
    path: Option<PathBuf>,
    title: String,
    state: Option<State>,
}

impl ApplicationHandler<AppEvent> for App {
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        let AppEvent::Theme { dark } = event;
        if let Some(s) = self.state.as_mut() {
            s.egui_ctx.set_visuals(theme::visuals(dark));
            s.window.request_redraw();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        // Room to work on a large screen; on Linux no system frame, Daedalus draws its own (frame.rs).
        let attrs = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0))
            .with_decorations(!frame::OWN_FRAME);
        // App id / WM class matching packaging/daedalus.desktop, so the taskbar shows its icon.
        #[cfg(target_os = "linux")]
        let attrs = {
            let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, "daedalus", "daedalus");
            winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "daedalus", "daedalus")
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        // light or dark like the desktop: the portal on Linux (and its later switches), else what the window reports
        let proxy = self.proxy.clone();
        let dark = theme::watch(move |dark| {
            let _ = proxy.send_event(AppEvent::Theme { dark });
        })
        .unwrap_or(window.theme() == Some(winit::window::Theme::Dark));
        self.state = Some(State::new(window, &self.mesh, self.path.clone(), dark));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        // a press on the window's edge resizes it, and goes no further (own frame, frame.rs)
        if let WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } = event {
            if let Some(d) = s.resize_edge() {
                let _ = s.window.drag_resize_window(d);
                return;
            }
        }
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
            // Windows reports a switch of the system's light or dark style here
            WindowEvent::ThemeChanged(t) => {
                s.egui_ctx.set_visuals(theme::visuals(t == winit::window::Theme::Dark));
                s.window.request_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                // Presses that land on the toolbar or status bar belong to egui.
                if down && s.egui_ctx.is_pointer_over_area() {
                    return;
                }
                // the view before an orbit, pan or zoom, for Camera > Previous
                if down && (button == MouseButton::Middle || button == MouseButton::Right || s.tools.tool.navigates()) {
                    s.remember_view();
                }
                match button {
                    MouseButton::Left => {
                        if down {
                            s.press_at = s.cursor;
                            s.left_down = true;
                            if s.tools.tool == Tool::Freehand && s.readonly.is_none() {
                                let c = s.resolve_cursor();
                                s.tools.freehand_press(&c);
                            }
                        } else if s.left_down && s.tools.tool == Tool::Freehand {
                            s.left_down = false;
                            let r = s.tools.freehand_release(&mut s.model);
                            s.message = r.err();
                            s.after_edit();
                        } else if s.left_down && s.tools.tool == Tool::ZoomWindow {
                            s.left_down = false;
                            let (w, h) = s.size();
                            let (a, b) = (s.press_at, s.cursor);
                            s.camera.zoom_window(a.0, a.1, b.0, b.1, w, h);
                            s.window.request_redraw();
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
                s.pointer_in = true;
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
                } else if s.left_down && tool == Tool::ZoomWindow {
                    // the rectangle is drawn in render()
                } else if s.left_down && tool == Tool::LookAround {
                    s.camera.look_around(-dx, dy);
                } else if s.left_down && tool == Tool::Walk {
                    // up walks forward, sideways turns
                    s.camera.walk(-dy * 0.02);
                    s.camera.look_around(-dx * 0.5, 0.0);
                } else {
                    s.mouse_moved();
                    return;
                }
                s.window.request_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                s.last_cursor = None;
                s.pointer_in = false;
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
                // a run of wheel steps is one view change for Camera > Previous
                if s.view_saved_at.elapsed().as_millis() > 800 {
                    s.remember_view();
                } else {
                    s.view_saved_at = std::time::Instant::now();
                }
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
    let event_loop = EventLoop::<AppEvent>::with_user_event().build().expect("event loop");
    let mut app = App { proxy: event_loop.create_proxy(), mesh, path, title, state: None };
    event_loop.run_app(&mut app).expect("run app");
}

/// Distance in pixels from point `p` to the segment `a`-`b` on screen.
fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}
