mod camera;
mod filedlg;
mod frame;
mod theme;
mod icons;
mod settings;
mod text3d;
mod render;
mod tools;
mod ui;

use camera::Camera;
use daedalus_core::inspect::{self, DEFAULT_TOLERANCE};
use daedalus_core::scene::{Scene, Section};
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
    /// What is on screen: the geometry being edited (or a preview of it, or the read-only import), then the open
    /// context's groups and components, then everything outside the open context (faded).
    display: Mesh,
    /// Faces of `display` below this are the geometry being edited; up to `faded_from`, faces of instances (`owners`
    /// names whose, one per face); from `faded_from` on, outside the open context.
    active_faces: usize,
    owners: Vec<usize>,
    faded_from: usize,
    faded_vertex: u32,
    /// The instances and the outside, as of the model state they were built for (rebuilt when it changes).
    parts: Option<((u64, u64, usize), Vec<(usize, Mesh)>, Mesh)>,
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
    /// Text being typed into: its number in the notes and the words so far.
    label_edit: Option<(usize, String)>,
    /// The text box opens this frame (its words get selected).
    label_fresh: bool,
    /// Where the section plane cuts the model (black lines), rebuilt with the display.
    cut_lines: Vec<LineVertex>,
    /// The 3D Text window's fields (kept for next time).
    text3d: Text3dForm,
}

/// What the 3D Text window asks: the words, the letters' height, and whether they are extruded and how far.
#[derive(Debug, Clone)]
struct Text3dForm {
    text: String,
    height: String,
    extruded: bool,
    depth: String,
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
    fn new(window: Arc<Window>, scene: &Scene, path: Option<PathBuf>, dark: bool) -> State {
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
        let (model, readonly) = load(scene);
        let camera = match scene.flatten().bounds() {
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
            active_faces: 0,
            owners: Vec::new(),
            faded_from: 0,
            faded_vertex: 0,
            parts: None,
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
            label_edit: None,
            label_fresh: false,
            cut_lines: Vec::new(),
            text3d: Text3dForm {
                text: "Enter text".into(),
                height: daedalus_core::units::format(0.3, settings::load_units()),
                extruded: true,
                depth: daedalus_core::units::format(0.05, settings::load_units()),
            },
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
    fn document(&self) -> Scene {
        match &self.readonly {
            Some(m) => Scene::from_mesh(m.clone()),
            None => self.model.document(),
        }
    }

    /// Replaces the document (a new one, an opened file, or an import).
    fn replace(&mut self, scene: &Scene, path: Option<PathBuf>) {
        let (model, readonly) = load(scene);
        let mesh = scene.flatten();
        self.model = model;
        self.parts = None;
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
                self.replace(&Scene::default(), None);
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
        match daedalus_io::save_scene(&self.document(), path) {
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
        match daedalus_io::open_scene(path) {
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
            Purpose::Export => match daedalus_io::save_scene(&self.document(), &path) {
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
        self.tools.axes = self.model.notes().axes;
        let preview = self.tools.preview_mesh(&self.model);
        self.display = match (&self.readonly, &preview) {
            (Some(m), _) => m.clone(),
            (None, Some(p)) => p.to_mesh(),
            (None, None) => self.model.mesh().to_mesh(),
        };
        self.active_faces = self.display.faces.len();
        self.owners.clear();
        let mut highlight = if preview.is_some() { Vec::new() } else { self.tools.highlight() };
        if self.readonly.is_none() {
            // the groups and components, rebuilt only when the model changed
            let key = (self.doc, self.model.state(), self.model.open_depth());
            if self.parts.as_ref().is_none_or(|p| p.0 != key) {
                let (inside, outside) = self.model.parts();
                self.parts = Some((key, inside, outside));
            }
            let lit = self.tools.highlight_instances();
            let moving = self.tools.preview_instances();
            let (_, inside, outside) = self.parts.as_ref().expect("built above");
            for (i, m) in inside {
                let mut place = |m: &Mesh, owners: &mut Vec<usize>, display: &mut Mesh, lit_now: bool| {
                    let from = display.faces.len();
                    display.append(m);
                    owners.extend(std::iter::repeat_n(*i, m.faces.len()));
                    if lit_now {
                        highlight.extend(from..display.faces.len());
                    }
                };
                match &moving {
                    Some((ids, xf, copy)) if ids.contains(i) => {
                        if *copy {
                            place(m, &mut self.owners, &mut self.display, false);
                        }
                        place(&m.transformed(xf), &mut self.owners, &mut self.display, true);
                    }
                    _ => place(m, &mut self.owners, &mut self.display, lit.contains(i)),
                }
            }
            self.faded_from = self.display.faces.len();
            self.faded_vertex = self.display.positions.len() as u32;
            self.display.append(outside);
        } else {
            self.faded_from = self.display.faces.len();
            self.faded_vertex = self.display.positions.len() as u32;
        }
        // the section cut: black lines round it, a hair on the kept side so the cut does not hide them
        self.cut_lines = match self.model.notes().section.filter(|s| s.active) {
            Some(s) => {
                let back = Vec3::from(s.normal) * -1e-4;
                daedalus_core::section::cut_lines(&self.display, s.point, s.normal)
                    .into_iter()
                    .flat_map(|(a, b)| [Vec3::from(a) + back, Vec3::from(b) + back])
                    .map(|p| LineVertex { position: p.into(), color: [0.0, 0.0, 0.0, 1.0] })
                    .collect()
            }
            None => Vec::new(),
        };
        highlight.sort_unstable();
        self.gpu.set_scene(&self.display, &highlight, &self.flagged, (self.faded_from, self.faded_vertex));
        self.shown_highlight = highlight;
        self.window.request_redraw();
    }

    /// What a face of the display is: the geometry being edited (its face number), an instance, or outside.
    fn owner(&self, f: usize) -> (Option<usize>, Option<usize>) {
        if f < self.active_faces {
            (Some(f), None)
        } else if f < self.faded_from {
            (None, self.owners.get(f - self.active_faces).copied())
        } else {
            (None, None)
        }
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
        let axes = self.model.notes().axes;
        let axes_at = Vec3::from(axes.origin);
        candidates.push((axes_at, Snap::Origin));
        for (i, axis) in axes.dirs.into_iter().map(Vec3::from).enumerate() {
            let b = axis.dot(dir);
            let denom = 1.0 - b * b;
            if denom.abs() > 1e-6 {
                let w0 = axes_at - origin;
                let s = (b * dir.dot(w0) - axis.dot(w0)) / denom;
                candidates.push((axes_at + axis * s, Snap::OnAxis(i as u8)));
            }
        }
        // guide points, and the point of each guide line nearest the mouse
        for g in &self.model.notes().guides {
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
                let ground = self.ground();
                let (face, plane) = match on_face {
                    Some((f, plane)) => (Some(f), plane),
                    None if (p - Vec3::from(ground.origin)).dot(Vec3::from(ground.normal)).abs() < 1e-5 => (None, ground),
                    None => (None, Plane { origin: p.into(), ..ground }),
                };
                let (face, instance) = face.map_or((None, None), |f| self.owner(f));
                Some(Hit { point: p.into(), plane, face, line: None, instance, snap })
            }
            (None, Some((f, t))) => {
                let p = origin + dir * t;
                let (face, instance) = self.owner(f);
                face_plane(f, p).map(|plane| Hit { point: p.into(), plane, face, line: None, instance, snap: Snap::OnFace })
            }
            (None, None) => {
                let ground = self.ground();
                ground.intersect_ray(origin.into(), dir.into()).map(|p| Hit { point: p, plane: ground, face: None, line: None, instance: None, snap: Snap::Ground })
            }
        };
        // the loose line nearest the mouse on screen, for the Eraser
        let active_lines = if self.readonly.is_some() { 0 } else { self.model.mesh().lines.len() };
        let line = self.display.lines.iter().take(active_lines).enumerate().filter_map(|(i, &[a, b])| {
            let pa = self.camera.project(Vec3::from(self.display.positions[a as usize]), w, h)?;
            let pb = self.camera.project(Vec3::from(self.display.positions[b as usize]), w, h)?;
            let d = segment_distance(self.cursor, pa, pb);
            (d <= SNAP_RADIUS * 0.6).then_some((i, d))
        });
        let line = line.min_by(|x, y| x.1.total_cmp(&y.1)).map(|(i, _)| i);
        let hit = hit.map(|h| Hit { line, ..h });
        Cursor { origin: origin.into(), dir: dir.into(), hit }
    }

    /// The ground for drawing: the red-green plane of the drawing axes.
    fn ground(&self) -> Plane {
        let a = self.model.notes().axes;
        Plane { origin: a.origin, u: a.dirs[0], v: a.dirs[1], normal: a.dirs[2] }
    }

    /// The text under the mouse, if any (by where it is drawn on screen).
    fn label_at(&self) -> Option<usize> {
        let (w, h) = self.size();
        let ppp = self.egui_ctx.pixels_per_point();
        self.model.notes().labels.iter().enumerate().rev().find_map(|(i, l)| {
            let (x, y) = self.camera.project(Vec3::from(l.at), w, h)?;
            let longest = l.text.lines().map(|t| t.chars().count()).max().unwrap_or(1).max(4) as f32;
            let half = (longest * 3.6 + 6.0) * ppp;
            let tall = (l.text.lines().count().max(1) as f32 * 8.0 + 4.0) * ppp;
            ((self.cursor.0 - x).abs() <= half && (self.cursor.1 - y).abs() <= tall).then_some(i)
        })
    }

    /// Starts typing into text `i`.
    fn edit_label(&mut self, i: usize) {
        if let Some(l) = self.model.notes().labels.get(i) {
            self.label_edit = Some((i, l.text.clone()));
            self.label_fresh = true;
        }
    }

    /// Typing into text is done: the words go in (no words: the text goes).
    fn finish_label(&mut self) {
        let Some((i, text)) = self.label_edit.take() else { return };
        let text = text.trim().to_string();
        let unchanged = self.model.notes().labels.get(i).is_some_and(|l| l.text == text);
        if !unchanged {
            self.model.edit_notes(|n| {
                if i < n.labels.len() {
                    if text.is_empty() {
                        n.labels.remove(i);
                    } else {
                        n.labels[i].text = text;
                    }
                }
            });
        }
        self.window.request_redraw();
    }

    fn mouse_moved(&mut self) {
        if self.over_ui {
            self.last_cursor = None;
            return;
        }
        let c = self.resolve_cursor();
        self.last_cursor = Some(c);
        self.tools.hover(&c);
        if self.tools.preview_mesh(&self.model).is_some() || self.tools.preview_instances().is_some() || self.highlight_changed() {
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
        // text: a double-click with Select types into it, the Eraser takes it
        if let Some(i) = self.label_at() {
            match self.tools.tool {
                Tool::Select if count == 2 => {
                    self.edit_label(i);
                    return;
                }
                Tool::Eraser => {
                    self.model.edit_notes(|n| {
                        n.labels.remove(i);
                    });
                    self.window.request_redraw();
                    return;
                }
                _ => {}
            }
        }
        // Select in an open group: a click outside it, or a double-click on nothing, closes it (as in SketchUp)
        if self.tools.tool == Tool::Select && self.model.open_depth() > 0 {
            let outside = c.hit.is_some_and(|h| h.face.is_none() && h.instance.is_none() && h.snap == Snap::OnFace);
            if outside || (count == 2 && c.hit.is_none_or(|h| h.face.is_none() && h.instance.is_none())) {
                self.close_group();
                return;
            }
        }
        // Select: a double-click on a group or component opens it for editing
        if self.tools.tool == Tool::Select && count == 2 && self.readonly.is_none() {
            if let Some(i) = c.hit.and_then(|h| h.instance) {
                self.open_group(i);
                return;
            }
        }
        let result = match (&self.readonly, self.tools.tool) {
            // Select: Ctrl adds, Shift toggles, a triple-click takes everything joined (works on a read-only import too)
            (_, Tool::Select) => {
                let (face, instance) = (c.hit.and_then(|h| h.face), c.hit.and_then(|h| h.instance));
                let (add, toggle) = (self.modifiers.control_key(), self.modifiers.shift_key());
                self.tools.select(&self.model, face, instance, add, toggle, count >= 3);
                Ok(())
            }
            (Some(_), _) => Ok(()),
            (None, _) => self.tools.click(&mut self.model, &c),
        };
        if let Err(e) = result {
            self.message = Some(e);
        }
        // text just placed: type its words now
        if let Some(i) = self.tools.edit_label.take() {
            self.edit_label(i);
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
        if self.readonly.is_some() || !self.tools.has_selection() {
            return;
        }
        if let Err(e) = self.model.erase_all(&self.tools.selected, &self.tools.instances) {
            self.message = Some(e.to_string());
        }
        self.tools.model_changed();
        self.after_edit();
    }

    fn highlight_changed(&self) -> bool {
        // cheap test: the faces the tools would light, against what was lit last (instances are rebuilt in refresh)
        let faces = self.tools.highlight();
        let shown: Vec<usize> = self.shown_highlight.iter().copied().filter(|&f| f < self.active_faces).collect();
        let inst = self.tools.highlight_instances();
        let shown_inst: Vec<usize> = {
            let mut v: Vec<usize> = self.shown_highlight.iter().filter_map(|&f| self.owner(f).1).collect();
            v.dedup();
            v
        };
        let mut want_inst = inst;
        want_inst.sort_unstable();
        want_inst.dedup();
        let mut shown_inst = shown_inst;
        shown_inst.sort_unstable();
        shown_inst.dedup();
        faces != shown || want_inst != shown_inst
    }

    /// Make Group / Make Component from the selection.
    fn make_group(&mut self, component: bool) {
        if self.readonly.is_some() || !self.tools.has_selection() {
            self.message = Some("Select the faces (a triple-click takes a whole object) or groups to put in it.".into());
            return;
        }
        match self.model.make_group(&self.tools.selected, &self.tools.instances, component) {
            Ok(i) => {
                self.tools.model_changed();
                self.tools.instances = vec![i];
                let name = self.model.instances().get(i).and_then(|inst| self.model.definition(inst.def)).map(|d| d.name.clone()).unwrap_or_default();
                self.message = Some(format!("Made {name}. Double-click it to edit inside."));
            }
            Err(e) => self.message = Some(format!("Not grouped: {e}.")),
        }
        self.after_edit();
    }

    /// The one selected group or component, if exactly one is selected.
    fn one_instance(&self) -> Option<usize> {
        match self.tools.instances[..] {
            [i] if self.tools.selected.is_empty() => Some(i),
            _ => None,
        }
    }

    fn open_group(&mut self, i: usize) {
        match self.model.open(i) {
            Ok(()) => {
                self.tools.model_changed();
                self.message = None;
            }
            Err(e) => self.message = Some(format!("Cannot open it: {e}.")),
        }
        self.after_edit();
    }

    fn close_group(&mut self) {
        if self.model.close() {
            self.tools.model_changed();
            self.message = None;
            self.after_edit();
        }
    }

    fn explode(&mut self) {
        let Some(i) = self.one_instance() else { return };
        match self.model.explode(i) {
            Ok(()) => self.message = Some("Exploded: its geometry is loose again.".into()),
            Err(e) => self.message = Some(format!("Not exploded: {e}.")),
        }
        self.tools.model_changed();
        self.after_edit();
    }

    fn make_unique(&mut self) {
        let Some(i) = self.one_instance() else { return };
        match self.model.make_unique(i) {
            Ok(()) => self.message = Some("This copy now has a definition of its own.".into()),
            Err(e) => self.message = Some(e.to_string()),
        }
        self.refresh();
    }

    /// Reports solid problems on stdout and marks the offending faces red.
    fn inspect(&mut self) {
        // the geometry being edited (faces of groups have numbers of their own)
        let report = inspect::inspect(&self.display_active(), DEFAULT_TOLERANCE);
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
        let (fixed, summary) = inspect::repair(&self.display_active(), DEFAULT_TOLERANCE);
        // and coplanar faces merged: an import's triangle pairs become the faces they were drawn as
        let (fixed, _) = daedalus_core::merge::merge_coplanar(&fixed, daedalus_core::model::MERGE_TOLERANCE);
        if !summary.changed_anything() {
            self.message = Some("Repair: nothing to fix.".into());
            return;
        }
        println!("repair: {summary:?}");
        match HalfEdgeMesh::from_mesh(&fixed) {
            Ok(h) if self.readonly.is_none() => {
                self.model.set_mesh(h);
                self.message = Some("Repair: done (Undo takes it back).".into());
            }
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

    /// The geometry being edited, without the groups and what lies outside.
    fn display_active(&self) -> Mesh {
        match &self.readonly {
            Some(m) => m.clone(),
            None => self.model.mesh().to_mesh(),
        }
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
            UiAction::SectionCuts => {
                self.model.edit_notes(|n| {
                    if let Some(s) = n.section.as_mut() {
                        s.active = !s.active;
                    }
                });
                self.refresh();
            }
            UiAction::DeleteSection => {
                self.model.edit_notes(|n| n.section = None);
                self.refresh();
            }
            UiAction::DeleteDimensions => self.model.edit_notes(|n| n.dims.clear()),
            UiAction::MakeGroup => self.make_group(false),
            UiAction::MakeComponent => self.make_group(true),
            UiAction::EditGroup => {
                if let Some(i) = self.one_instance() {
                    self.open_group(i);
                }
            }
            UiAction::CloseGroup => self.close_group(),
            UiAction::Explode => self.explode(),
            UiAction::MakeUnique => self.make_unique(),
            UiAction::BackEdges => {
                self.style.back_edges = !self.style.back_edges;
                settings::save("back_edges", if self.style.back_edges { "on" } else { "off" });
            }
            UiAction::SelectAll => {
                self.tools.select_all(&self.model);
                self.refresh();
            }
            UiAction::DeleteGuides => self.model.edit_notes(|n| n.guides.clear()),
            UiAction::ResetAxes => {
                self.model.edit_notes(|n| n.axes = daedalus_core::scene::Axes::default());
                self.refresh();
            }
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
                if !self.typed.is_empty() {
                    self.typed.clear();
                } else if self.tools.tool == Tool::Select && !self.tools.has_selection() && self.model.open_depth() > 0 {
                    // nothing left to cancel: Esc closes the open group
                    self.close_group();
                } else {
                    self.tools.cancel();
                    self.message = None;
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
            // an array right after a Move copy: x5, 5x, *5 or /5
            Key::Character(c) if (self.tools.can_array() || !self.typed.is_empty()) && (c == "*" || c == "/" || c.eq_ignore_ascii_case("x")) => {
                self.typed.push_str(c)
            }
            Key::Character(c) if c.eq_ignore_ascii_case("g") && self.typed.is_empty() => self.make_group(true),
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
        let axes = self.model.notes().axes;
        for (i, color) in AXIS_COLORS.iter().take(3).enumerate() {
            let faded = [color[0] * 0.4 + 0.55, color[1] * 0.4 + 0.55, color[2] * 0.4 + 0.55, 1.0];
            for (sign, color) in [(1.0, *color), (-1.0, faded)] {
                let end = (Vec3::from(axes.origin) + Vec3::from(axes.dirs[i]) * reach * sign).into();
                out.push(LineVertex { position: axes.origin, color });
                out.push(LineVertex { position: end, color });
            }
        }
        out.extend_from_slice(&self.cut_lines);
        // text leaders, from the point to the text
        for l in &self.model.notes().labels {
            if l.point != l.at {
                out.push(LineVertex { position: l.point, color: [0.15, 0.15, 0.18, 1.0] });
                out.push(LineVertex { position: l.at, color: [0.15, 0.15, 0.18, 1.0] });
            }
        }
        // dimensions, in dark grey
        for (a, b, color) in self.tools.dim_lines(&self.model, [0.15, 0.15, 0.18, 1.0]) {
            out.push(LineVertex { position: a, color });
            out.push(LineVertex { position: b, color });
        }
        // guides: lines reach as far as the axes; points are small crosses
        let guides = self.model.notes().guides.iter().copied().chain(self.tools.guide_preview());
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
            .chain(self.tools.section_outline(&self.model, size))
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
            | Tool::Text
            | Tool::Axes
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
            section: self.model.notes().section.map(|s| s.active),
            has_selection: self.tools.has_selection(),
            one_instance: self.one_instance().and_then(|i| self.model.instances().get(i)).and_then(|inst| self.model.definition(inst.def)).map(|d| d.component),
            editing: (self.model.open_depth() > 0).then(|| self.model.open_names().join(" › ")),
            labels: self
                .tools
                .dim_labels(&self.model)
                .into_iter()
                .filter_map(|(p, t)| self.camera.project(Vec3::from(p), w, h).map(|(x, y)| (egui::pos2(x / ppp, y / ppp), t)))
                .collect(),
            texts: self
                .model
                .notes()
                .labels
                .iter()
                .enumerate()
                .filter(|(i, _)| self.label_edit.as_ref().is_none_or(|e| e.0 != *i))
                .filter_map(|(_, l)| self.camera.project(Vec3::from(l.at), w, h).map(|(x, y)| (egui::pos2(x / ppp, y / ppp), l.text.clone())))
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
        let mut label_edit = self.label_edit.take();
        let label_pos = label_edit
            .as_ref()
            .and_then(|(i, _)| self.model.notes().labels.get(*i))
            .and_then(|l| self.camera.project(Vec3::from(l.at), w, h))
            .map(|(x, y)| egui::pos2(x / ppp, y / ppp));
        let mut label_done = false;
        // 3D Text: its window while it waits for the words
        let mut form = self.tools.wants_text3d().then(|| self.text3d.clone());
        let mut form_answer: Option<bool> = None;
        let label_fresh = std::mem::take(&mut self.label_fresh);
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
            if let Some(f) = form.as_mut() {
                egui::Window::new("3D Text").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut f.text).desired_rows(3).desired_width(260.0));
                    egui::Grid::new("text3d-grid").num_columns(2).show(ui, |ui| {
                        ui.label("Height");
                        ui.add(egui::TextEdit::singleline(&mut f.height).desired_width(120.0));
                        ui.end_row();
                        ui.checkbox(&mut f.extruded, "Extruded");
                        ui.add_enabled(f.extruded, egui::TextEdit::singleline(&mut f.depth).desired_width(120.0));
                        ui.end_row();
                    });
                    ui.label(egui::RichText::new("Ubuntu Light. Lengths in any unit (300mm, 1').").small());
                    ui.horizontal(|ui| {
                        if ui.button("Place").clicked() {
                            form_answer = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            form_answer = Some(false);
                        }
                    });
                });
            }
            // typing into text: a box where the text stands; Enter or a click elsewhere finishes
            if let (Some((_, text)), Some(pos)) = (label_edit.as_mut(), label_pos) {
                egui::Area::new(egui::Id::new("text-edit")).fixed_pos(pos - egui::vec2(80.0, 12.0)).order(egui::Order::Foreground).show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        let out = egui::TextEdit::singleline(text).desired_width(160.0).hint_text("Text").show(ui);
                        if label_fresh {
                            // the words it starts with are selected, so typing replaces them (as in SketchUp)
                            let mut st = out.state.clone();
                            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
                            st.cursor.set_char_range(Some(all));
                            st.store(ui.ctx(), out.response.id);
                        }
                        let r = out.response;
                        r.request_focus();
                        if r.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)) {
                            label_done = true;
                        }
                    });
                });
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
        self.label_edit = label_edit;
        if label_done {
            self.finish_label();
        }
        if let Some(f) = form {
            self.text3d = f;
            match form_answer {
                Some(true) => {
                    let units = self.tools.units;
                    let f = &self.text3d;
                    let height = daedalus_core::units::parse(&f.height, units);
                    let depth = if f.extruded { daedalus_core::units::parse(&f.depth, units).map(Some) } else { Some(None) };
                    let r = match (height, depth) {
                        (Some(h), Some(d)) => self.tools.begin_text3d(&f.text.clone(), h, d),
                        _ => Err("cannot read the height or the depth".into()),
                    };
                    self.message = r.err();
                }
                Some(false) => self.set_tool(Tool::Select),
                None => {}
            }
            self.window.request_redraw();
        }
        self.egui_state.handle_platform_output(&self.window, out.platform_output);
        let ui = UiFrame {
            primitives: ctx.tessellate(out.shapes, out.pixels_per_point),
            textures: out.textures_delta,
            pixels_per_point: out.pixels_per_point,
        };
        let (axes, overlay) = (self.axes(), self.overlay());
        // edges about 1.5 points wide, like SketchUp's default
        let line_width = 1.4 * self.window.scale_factor() as f32;
        let section = self.model.notes().section.filter(|s| s.active).map(|s: Section| (s.point, s.normal));
        let eye = if self.camera.ortho {
            let d = (self.camera.eye() - self.camera.target).normalize_or_zero();
            [d.x, d.y, d.z, 0.0]
        } else {
            let e = self.camera.eye();
            [e.x, e.y, e.z, 1.0]
        };
        self.gpu.render(self.camera.view_proj(w / h), eye, self.style, line_width, section, &axes, &overlay, ui);

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

/// A model to edit, or (when its top level is not a valid surface) its geometry shown read-only.
fn load(scene: &Scene) -> (Model, Option<Mesh>) {
    match Model::from_scene(scene.clone()) {
        Ok(m) => (m, None),
        Err(e) => {
            eprintln!("note: model is read-only until repaired with Tools > Repair Solid ({e})");
            (Model::new(), Some(scene.flatten()))
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
    scene: Scene,
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
        self.state = Some(State::new(window, &self.scene, self.path.clone(), dark));
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
                // Move: a tap on Ctrl switches between moving and copying, as in SketchUp
                if event.logical_key == Key::Named(NamedKey::Control) && !event.repeat && s.tools.tool == Tool::Move {
                    s.tools.copy = !s.tools.copy;
                    s.refresh();
                    return;
                }
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
    let (scene, title, path) = match std::env::args().nth(1) {
        Some(arg) => {
            let path = PathBuf::from(&arg);
            match daedalus_io::open_scene(&path) {
                Ok(m) => (m, format!("Daedalus - {arg}"), Some(path)),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => (Scene::default(), "Daedalus".to_string(), None),
    };
    let event_loop = EventLoop::<AppEvent>::with_user_event().build().expect("event loop");
    let mut app = App { proxy: event_loop.create_proxy(), scene, path, title, state: None };
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
