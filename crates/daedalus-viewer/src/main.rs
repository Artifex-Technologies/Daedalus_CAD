mod boxsel;
mod camera;
mod cursors;
mod filedlg;
mod frame;
#[cfg(target_os = "linux")]
mod portal;
mod theme;
mod icons;
mod pieces;
mod settings;
mod sheetui;
mod stressui;
mod text3d;
mod render;
mod tools;
mod ui;

use camera::Camera;
use daedalus_core::inspect;
use daedalus_core::scene::{Scene, Section};
use daedalus_core::{HalfEdgeMesh, Mesh, Model, Plane};
use daedalus_io::formats::SaveTo;
use glam::DVec3;
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
/// How close (points, so the same reach on a high-density screen) the Select tool must be to an edge to take it rather than
/// the face behind.
const EDGE_PICK_RADIUS: f32 = 6.0;

/// Position Camera: eye height above the point clicked (SketchUp's 5' 6").
const EYE_HEIGHT: f64 = 1.676;

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
    /// Where the right button went down (a release near it opens the right-click menu; a drag pans).
    right_press_at: (f32, f32),
    /// The right-click menu, open at this point (physical pixels).
    context_at: Option<(f32, f32)>,
    /// Each tool's own mouse cursor (cursors.rs), and which tool's is showing now (None: the system's).
    tool_cursors: std::collections::HashMap<Tool, winit::window::CustomCursor>,
    cursor_shown: Option<Tool>,
    /// The menu opened since the last frame (the click that opened it must not close it).
    context_fresh: bool,
    /// The Entity Info panel is open.
    info_open: bool,
    /// The Dynamic Components panels are open; what they hand back (a whole new attribute list).
    attributes_open: bool,
    options_open: bool,
    dc_out: std::cell::RefCell<Option<(Vec<daedalus_core::dynamic::Attribute>, bool)>>,
    /// The texture files on the GPU (in the model's order), and each one's average colour (what Shaded shows).
    loaded_textures: Vec<String>,
    texture_colors: Vec<[u8; 3]>,
    /// The Sheet Metal panel is open, its tab, and what the Flange tool adds (height mm, angle degrees, position).
    sheet_open: bool,
    sheet_tab: usize,
    flange_defaults: (f64, f64, usize, bool, bool),
    /// What the Fold tool adds (a jog, angle degrees, a jog's step mm, a fold's position, down), and its first point (the
    /// part's instance and the point, part mm) once picked.
    fold_defaults: (bool, f64, f64, usize, bool),
    fold_first: Option<(usize, [f64; 3])>,
    /// What the Hole tool cuts (round, size mm), and whether the Flange tool makes path flanges (with these legs).
    hole_defaults: (bool, [f64; 2]),
    path_defaults: (bool, Vec<[f64; 2]>),
    /// The fabricator's bend table (`~/.config/daedalus/bend-table.csv`), and what in it did not read.
    bend_table: Vec<daedalus_sheet::table::TableRow>,
    bend_table_note: Vec<String>,
    /// The selected part as the panel shows it, for the model state and selection it was made for.
    sheet_cache: Option<(u64, Option<ui::SheetView>)>,
    /// The flat pattern DXF being saved is in inches.
    dxf_inches: bool,
    /// Convert to Sheet Metal under way: the solid's instance, the point on its face that stays put and on each bend edge
    /// (its own coordinates, metres).
    convert: Option<(usize, Option<[f64; 3]>, Vec<[f64; 3]>)>,
    /// The Stress Test panel is open, and the test under way (its part, picks, settings and last result).
    stress_open: bool,
    stress: Option<stressui::Stress>,
    /// The Outliner is open; its rows, for the model state they were read at.
    outliner_open: bool,
    outline_cache: Option<(u64, Vec<(usize, String, bool, Vec<usize>)>)>,
    /// What it shows, for the model state and selection it was worked out for.
    info_cache: Option<(u64, Option<ui::Info>)>,
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
    /// Vertices of the display that belong to the geometry being edited (they come first, numbered as in the model).
    active_vertices: u32,
    owners: Vec<usize>,
    faded_from: usize,
    faded_vertex: u32,
    /// The instances and the outside, piece by piece with their triangles and edges, as of the document and the model's
    /// parts revision they were built for (rebuilt when either changes, from `piece_cache`: only the pieces an edit changed
    /// are split again).
    parts: Option<((u64, u64), pieces::Parts)>,
    piece_cache: pieces::PieceCache,
    /// The geometry being edited split into triangles, and its edges, as of the document, revision and read-only import they
    /// were worked out for (none while a preview runs).
    active: Option<((u64, u64, bool), Tris, Vec<(u32, u32)>)>,
    /// `display`'s faces as triangles, and its edges to draw and snap to: worked out in refresh, not on every mouse move.
    tris: Tris,
    edges: Vec<(u32, u32)>,
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
    /// The document's file (None for a new model or an import, until it is saved; a SketchUp model's own file), how many times
    /// the whole document was replaced (open, new, repair), and the (replacement, model state) it was saved or opened at: it is
    /// unchanged while those match.
    path: Option<PathBuf>,
    doc: u64,
    saved: (u64, u64),
    /// The document's file was not read whole (a STEP file with curved faces): Save asks for a new file rather than write
    /// over it.
    partial: bool,
    /// A note in a small window until it is closed, work going on meanwhile: what a save left out, or what an opened file held
    /// that Daedalus does not read.
    notice: Option<Notice>,
    dialog: Option<FileDialog>,
    /// "Save changes?" while something would throw the changes away, and what to do once they are saved.
    ask: Option<Pending>,
    after_save: Option<Pending>,
    quit: bool,
    shown_title: String,
    /// View > Toolbar: down the left side instead of across the top (remembered in settings.conf).
    toolbar_left: bool,
    /// Sends events to the window (the file chooser's answer comes back this way).
    proxy: EventLoopProxy<AppEvent>,
    /// The desktop's file chooser is open; and the name and place it was asked with (for the built-in one, should there be no
    /// portal).
    picking: bool,
    pending_name: String,
    pending_at: Option<PathBuf>,
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
    /// The font's file (None: the built-in one), remembered between runs.
    font: Option<PathBuf>,
}

/// The notice's window: its title, its lines, and where its "also save a full copy" button saves (None: no button).
#[derive(Debug, Clone)]
struct Notice {
    title: String,
    lines: Vec<String>,
    copy: Option<PathBuf>,
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
    fn new(window: Arc<Window>, scene: &Scene, path: Option<PathBuf>, dark: bool, proxy: EventLoopProxy<AppEvent>) -> State {
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
            Some((lo, hi)) => Camera::fit(DVec3::from(lo), DVec3::from(hi)),
            // An empty model: frame a room-sized area around the origin.
            None => Camera::fit(DVec3::new(-1.0, -1.0, 0.0), DVec3::new(6.0, 6.0, 2.0)),
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
            right_press_at: (0.0, 0.0),
            context_at: None,
            tool_cursors: std::collections::HashMap::new(),
            cursor_shown: None,
            context_fresh: false,
            info_open: false,
            attributes_open: false,
            options_open: false,
            dc_out: std::cell::RefCell::new(None),
            loaded_textures: Vec::new(),
            texture_colors: Vec::new(),
            sheet_open: false,
            stress_open: false,
            stress: None,
            sheet_tab: 0,
            flange_defaults: (20.0, 90.0, 0, false, false),
            fold_defaults: (false, 90.0, 10.0, 0, false),
            fold_first: None,
            hole_defaults: (true, [6.0, 6.0]),
            path_defaults: (false, vec![[20.0, 90.0], [12.0, 90.0]]),
            bend_table: Vec::new(),
            bend_table_note: Vec::new(),
            sheet_cache: None,
            dxf_inches: false,
            convert: None,
            outliner_open: false,
            outline_cache: None,
            info_cache: None,
            press_at: (0.0, 0.0),
            modifiers: ModifiersState::empty(),
            model,
            readonly,
            display: Mesh::new(),
            active_faces: 0,
            active_vertices: 0,
            owners: Vec::new(),
            faded_from: 0,
            faded_vertex: 0,
            parts: None,
            piece_cache: pieces::PieceCache::default(),
            active: None,
            tris: Vec::new(),
            edges: Vec::new(),
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
            // a file given at start is opened below, as File > Open opens it
            path: None,
            doc: 0,
            saved: (0, 0),
            partial: false,
            notice: None,
            dialog: None,
            ask: None,
            after_save: None,
            quit: false,
            shown_title: String::new(),
            // the Large Tool Set down the left side unless across the top was chosen
            toolbar_left: settings::load("toolbar").as_deref() != Some("top"),
            label_edit: None,
            label_fresh: false,
            proxy,
            picking: false,
            pending_name: String::new(),
            pending_at: None,
            cut_lines: Vec::new(),
            text3d: Text3dForm {
                text: "Enter text".into(),
                height: daedalus_core::units::format(0.3, settings::load_units()),
                extruded: true,
                depth: daedalus_core::units::format(0.05, settings::load_units()),
                font: settings::load("text3d-font").map(PathBuf::from).filter(|p| p.is_file()),
            },
        };
        s.load_bend_table();
        s.refresh();
        if let Some(p) = path {
            s.open_file(&p, false);
        }
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
        // the old document's pieces are of no more use (keeping them would be harmless, only memory)
        self.parts = None;
        self.piece_cache = pieces::PieceCache::default();
        self.active = None;
        self.readonly = readonly;
        self.doc += 1;
        self.path = path;
        self.partial = false;
        self.notice = None;
        // an import is a new, unsaved document; an opened file or an empty one is unchanged
        self.saved = if mesh.positions.is_empty() || self.path.is_some() { (self.doc, self.model.state()) } else { (u64::MAX, 0) };
        let units = self.tools.units;
        self.tools = Tools::default();
        self.tools.units = units;
        self.flagged.clear();
        self.typed.clear();
        self.camera = match mesh.bounds() {
            Some((lo, hi)) => Camera::fit(DVec3::from(lo), DVec3::from(hi)),
            None => Camera::fit(DVec3::new(-1.0, -1.0, 0.0), DVec3::new(6.0, 6.0, 2.0)),
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
            Pending::Open => self.choose(Purpose::Open, ""),
            Pending::Import => self.choose(Purpose::Import, ""),
            Pending::Quit => self.quit = true,
        }
    }

    /// File > Save (Ctrl+S): in place, in the format the file was opened in when Daedalus writes it; else the Save dialog
    /// (a new model; a SketchUp model, beside it as STEP; a file read in part), as `formats::save_target` decides. `then`
    /// runs once it is saved.
    fn save(&mut self, then: Option<Pending>) {
        match daedalus_io::formats::save_target(self.path.as_deref(), !self.partial, false) {
            SaveTo::File(p) => {
                if self.save_to(&p) {
                    if let Some(t) = then {
                        self.carry_out(t);
                    }
                }
            }
            SaveTo::Ask { folder, name, format } => self.ask_where(then, folder, &name, format),
        }
    }

    /// File > Save As: the Save dialog, starting at the document's file and in its format (STEP when Daedalus does not write
    /// it).
    fn save_as(&mut self, then: Option<Pending>) {
        if let SaveTo::Ask { folder, name, format } = daedalus_io::formats::save_target(self.path.as_deref(), !self.partial, true) {
            self.ask_where(then, folder, &name, format);
        }
    }

    fn ask_where(&mut self, then: Option<Pending>, folder: Option<PathBuf>, name: &str, format: daedalus_io::formats::Format) {
        self.after_save = then;
        let at = folder.map(|f| f.join(name));
        self.choose_at(Purpose::SaveAs(format), at.as_deref(), name);
    }

    /// Saves the document to `path` in the format its ending names, the lengths of a STEP file in the units shown. What that
    /// format could not hold is shown in a notice, with a button to keep everything in a `.ddls` copy too.
    fn save_to(&mut self, path: &Path) -> bool {
        let opts = daedalus_io::SaveOptions { units: self.tools.units };
        match daedalus_io::save_scene_with(&self.document(), path, &opts) {
            Ok(left_out) => {
                self.path = Some(path.to_path_buf());
                self.partial = false;
                self.saved = (self.doc, self.model.state());
                self.message = Some(format!("Saved {}.", short(path)));
                self.notice = saved_notice(path, &left_out);
                true
            }
            Err(e) => {
                self.message = Some(format!("Not saved: {e}."));
                false
            }
        }
    }

    /// The notice's button: a full copy as `.ddls` beside the file just saved; the Save dialog first when a file of that name
    /// is there already, so it is not written over unasked.
    fn full_copy(&mut self, copy: PathBuf) {
        if copy.exists() {
            self.choose_at(Purpose::Copy, Some(&copy), &short(&copy));
        } else {
            self.save_copy(&copy);
        }
    }

    /// Writes the whole document to a `.ddls` file; the document's own file and its saved state stay as they are.
    fn save_copy(&mut self, path: &Path) {
        match std::fs::write(path, daedalus_io::native::write_scene(&self.document())) {
            Ok(()) => {
                self.message = Some(format!("Saved a full copy as {}.", short(path)));
                self.notice = None;
            }
            Err(e) => self.message = Some(format!("The full copy was not saved: {e}.")),
        }
    }

    fn open_file(&mut self, path: &Path, import: bool) {
        // the document's file, unless it is imported: Save writes it again (a SketchUp model: Save asks, beside it)
        let file = (!import).then(|| path.to_path_buf());
        let verb = if import { "Imported" } else { "Opened" };
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        // a SketchUp model: read through skp-convert (OpenSKP)
        if ext == "skp" || ext == "skb" {
            match from_sketchup(path) {
                Ok(scene) => {
                    self.replace(&scene, file);
                    self.message = Some(format!("{verb} {} from SketchUp. Save writes a STEP file beside it; the .skp is not changed.", short(path)));
                }
                Err(e) => self.message = Some(e),
            }
            return;
        }
        // STEP: what Daedalus reads of it, in the file's own units; what it does not read (curved faces) is named
        if ext == "step" || ext == "stp" {
            match std::fs::read(path).map_err(daedalus_io::IoError::from).and_then(|b| daedalus_io::step::read(&b)) {
                Ok(r) => {
                    if let Some(u) = r.units {
                        self.tools.units = u;
                    }
                    self.replace(&r.scene, file);
                    self.message = Some(format!("{verb} {} (lengths in {}).", short(path), self.tools.units.name().to_lowercase()));
                    if !r.skipped.is_empty() {
                        self.partial = !import;
                        let mut lines = vec!["Daedalus does not read these yet, so they are not in the model:".to_string()];
                        lines.extend(r.skipped.iter().map(|l| format!("•  {l}")));
                        if !import {
                            lines.push("Save writes a new STEP file beside it, so the file stays whole.".into());
                        }
                        self.notice = Some(Notice { title: format!("{} is not all read", short(path)), lines, copy: None });
                    }
                }
                Err(e) => self.message = Some(format!("Could not open {}: {e}.", short(path))),
            }
            return;
        }
        match daedalus_io::open_scene(path) {
            Ok(m) => {
                self.replace(&m, file);
                self.message = Some(format!("{verb} {}.", short(path)));
            }
            Err(e) => self.message = Some(format!("Could not open {}: {e}.", short(path))),
        }
    }

    /// Asks for a file: the desktop's own dialog (Minos on Yggdrasil) through the portal, else the built-in browser; starting
    /// in the document's folder.
    fn choose(&mut self, purpose: Purpose, name: &str) {
        let at = self.path.clone();
        self.choose_at(purpose, at.as_deref(), name);
    }

    /// [`State::choose`], starting in the folder `at` is in.
    fn choose_at(&mut self, purpose: Purpose, at: Option<&Path>, name: &str) {
        #[cfg(target_os = "linux")]
        {
            if self.picking {
                return; // one is open already
            }
            self.picking = true;
            let proxy = self.proxy.clone();
            portal::ask(purpose, at, name, move |answer| {
                let _ = proxy.send_event(AppEvent::Picked(purpose, answer));
            });
            self.pending_name = name.to_string();
            self.pending_at = at.map(Path::to_path_buf);
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.dialog = Some(FileDialog::new(purpose, at, name));
        }
    }

    /// The portal's answer.
    #[cfg(target_os = "linux")]
    fn picked(&mut self, purpose: Purpose, answer: portal::Answer) {
        self.picking = false;
        match answer {
            portal::Answer::Chosen(mut path) => {
                // a save dialog may hand back a name without its ending (the portal adds the chosen filter's when it says)
                if path.extension().is_none() {
                    match purpose {
                        Purpose::SaveAs(f) => {
                            path.set_extension(f.extension());
                        }
                        Purpose::Copy => {
                            path.set_extension(daedalus_io::formats::Format::Native.extension());
                        }
                        Purpose::Export => {
                            path.set_extension("stl");
                        }
                        Purpose::Step | Purpose::PartStep => {
                            path.set_extension("step");
                        }
                        Purpose::FlatDxf => {
                            path.set_extension("dxf");
                        }
                        _ => {}
                    }
                }
                self.dialog_done(purpose, path);
            }
            portal::Answer::Cancelled => self.after_save = None,
            portal::Answer::NoPortal => {
                let name = std::mem::take(&mut self.pending_name);
                let at = self.pending_at.take();
                self.dialog = Some(FileDialog::new(purpose, at.as_deref(), &name));
            }
        }
        self.window.request_redraw();
    }

    /// A file chosen in the dialog.
    fn dialog_done(&mut self, purpose: Purpose, path: PathBuf) {
        match purpose {
            Purpose::Open => self.open_file(&path, false),
            Purpose::Import => self.open_file(&path, true),
            Purpose::SaveAs(_) => {
                if self.save_to(&path) {
                    if let Some(t) = self.after_save.take() {
                        self.carry_out(t);
                    }
                }
            }
            Purpose::Copy => self.save_copy(&path),
            Purpose::Export => match daedalus_io::save_scene(&self.document(), &path) {
                Ok(()) => self.message = Some(format!("Exported {}.", short(&path))),
                Err(e) => self.message = Some(format!("Not exported: {e}.")),
            },
            Purpose::Texture => self.add_texture_from(&path),
            Purpose::BendTable => self.import_bend_table(&path),
            Purpose::Step => match daedalus_sheet::export::scene_step(&self.document()) {
                Ok(r) => match std::fs::write(&path, r.text) {
                    Ok(()) => {
                        let mut note = format!("Exported {}: {} sheet-metal part(s), {} other solid(s).", short(&path), r.parts, r.solids);
                        if !r.skipped.is_empty() {
                            note.push_str(&format!(" Left out (not closed solids): {}.", r.skipped.join(", ")));
                        }
                        self.message = Some(note);
                    }
                    Err(e) => self.message = Some(format!("Not exported: {e}.")),
                },
                Err(e) => self.message = Some(format!("Not exported: {e}.")),
            },
            Purpose::PartStep => {
                let made = self.sheet_part().ok_or_else(|| "select a sheet-metal part first".to_string()).and_then(|(i, _, p)| {
                    let w = self.model.instance_world(i).unwrap_or_default();
                    p.solid_placed(&w).and_then(|b| daedalus_sheet::part::step(&[b])).map_err(|e| e.to_string())
                });
                self.message = Some(match made.and_then(|t| std::fs::write(&path, t).map_err(|e| e.to_string())) {
                    Ok(()) => format!("Exported {}.", short(&path)),
                    Err(e) => format!("Not exported: {e}."),
                });
            }
            Purpose::FlatDxf => {
                let units = if self.dxf_inches { daedalus_io::dxf2d::Units::Inches } else { daedalus_io::dxf2d::Units::Millimeters };
                let made = self.sheet_part().ok_or_else(|| "select a sheet-metal part first".to_string()).and_then(|(_, _, p)| p.flat_dxf(units).map_err(|e| e.to_string()));
                self.message = Some(match made.and_then(|t| std::fs::write(&path, t).map_err(|e| e.to_string())) {
                    Ok(()) => format!("Exported the flat pattern to {}.", short(&path)),
                    Err(e) => format!("Not exported: {e}."),
                });
            }
        }
    }

    /// A texture from an image file: added to the model (a tile 1 m square to start with; Entity Info changes it) and
    /// taken up by Paint Bucket.
    fn add_texture_from(&mut self, path: &Path) {
        if self.readonly.is_some() {
            return;
        }
        if image::open(path).is_err() {
            self.message = Some(format!("{} is not an image Daedalus can read (JPEG, PNG or BMP).", short(path)));
            return;
        }
        let name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Texture".into());
        let i = self.model.add_texture(daedalus_core::scene::Texture { name, path: path.display().to_string(), size: [1.0, 1.0] });
        self.sync_textures();
        self.tools.paint_color = Some(daedalus_core::Paint { rgb: self.texture_colors.get(i).copied().unwrap_or([200, 200, 200]), texture: Some(i as u16) });
        self.set_tool(Tool::PaintBucket);
        self.message = Some("Texture ready: click faces to paint them with it (View > Shaded with Textures shows it).".into());
    }

    /// The model's textures onto the GPU, when they changed: each image resized to one layer (a missing file shows as a
    /// grey check), and its average colour kept.
    fn sync_textures(&mut self) {
        let paths: Vec<String> = self.model.textures().iter().map(|t| t.path.clone()).collect();
        if paths == self.loaded_textures {
            return;
        }
        let side = render::TEXTURE_SIZE;
        let mut layers = Vec::new();
        let mut colors = Vec::new();
        for p in &paths {
            let img = match image::open(p) {
                Ok(i) => i.resize_exact(side, side, image::imageops::FilterType::Triangle).to_rgba8(),
                Err(_) => image::RgbaImage::from_fn(side, side, |x, y| if (x / 32 + y / 32) % 2 == 0 { image::Rgba([170, 170, 175, 255]) } else { image::Rgba([210, 210, 215, 255]) }),
            };
            let n = (side * side) as u64;
            let sum = img.pixels().fold([0u64; 3], |s, px| [s[0] + px[0] as u64, s[1] + px[1] as u64, s[2] + px[2] as u64]);
            colors.push(sum.map(|c| (c / n) as u8));
            layers.push(img.into_raw());
        }
        self.gpu.set_textures(&layers);
        self.texture_colors = colors;
        self.loaded_textures = paths;
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
        self.active_vertices = self.display.positions.len() as u32;
        // its triangles and edges: kept while the model is unchanged (a highlight or a hover does not split every face again)
        let key = (self.doc, self.model.revision(), self.readonly.is_some());
        (self.tris, self.edges) = match (&preview, &self.active) {
            (None, Some((k, t, e))) if *k == key => (t.clone(), e.clone()),
            _ => {
                let (t, e) = (self.display.face_triangles(), self.display.visible_edges());
                self.active = if preview.is_none() { Some((key, t.clone(), e.clone())) } else { None };
                (t, e)
            }
        };
        self.owners.clear();
        let mut highlight = if preview.is_some() { Vec::new() } else { self.tools.highlight() };
        if self.readonly.is_none() {
            // the groups and components, rebuilt when they may have changed: only the pieces an edit changed are split again
            let key = (self.doc, self.model.parts_revision());
            if self.parts.as_ref().is_none_or(|p| p.0 != key) {
                let cache = &mut self.piece_cache;
                let parts = self.model.with_part_pieces(|inside, outside| cache.build(inside, outside));
                self.parts = Some((key, parts));
            }
            let lit = self.tools.highlight_instances();
            let moving = self.tools.preview_instances();
            let (_, parts) = self.parts.as_ref().expect("built above");
            let (tris, edges) = (&mut self.tris, &mut self.edges);
            let state = self.model.state();
            let mut stress = self.stress.as_mut();
            for (i, placed) in &parts.inside {
                // the part under a stress test: painted with its supports and loads, or drawn as the result
                if let Some((sm, st)) = stress.as_mut().filter(|s| s.instance == *i && moving.is_none()).and_then(|s| s.display(state)) {
                    let base = self.display.positions.len() as u32;
                    tris.extend(st.iter().map(|ts| ts.iter().map(|tri| tri.map(|v| v + base)).collect::<Vec<_>>()));
                    edges.extend(sm.visible_edges().into_iter().map(|(a, b)| (a + base, b + base)));
                    self.display.append(sm);
                    self.owners.extend(std::iter::repeat_n(*i, sm.faces.len()));
                    continue;
                }
                let mut place = |xf: Option<&daedalus_core::scene::Xform>, owners: &mut Vec<usize>, display: &mut Mesh, lit_now: bool| {
                    let from = display.faces.len();
                    for p in placed {
                        pieces::append(display, tris, edges, p, xf);
                        owners.extend(std::iter::repeat_n(*i, p.mesh.faces.len()));
                    }
                    if lit_now {
                        highlight.extend(from..display.faces.len());
                    }
                };
                match &moving {
                    Some((ids, xf, copy)) if ids.contains(i) => {
                        if *copy {
                            place(None, &mut self.owners, &mut self.display, false);
                        }
                        place(Some(xf), &mut self.owners, &mut self.display, true);
                    }
                    _ => place(None, &mut self.owners, &mut self.display, lit.contains(i)),
                }
            }
            self.faded_from = self.display.faces.len();
            self.faded_vertex = self.display.positions.len() as u32;
            for p in &parts.outside {
                pieces::append(&mut self.display, &mut self.tris, &mut self.edges, p, None);
            }
        } else {
            self.faded_from = self.display.faces.len();
            self.faded_vertex = self.display.positions.len() as u32;
        }
        // the section cut: black lines round it, a hair on the kept side so the cut does not hide them
        self.cut_lines = match self.model.notes().cutting() {
            Some(s) => {
                let back = DVec3::from(s.normal) * -1e-4;
                daedalus_core::section::cut_lines(&self.display, s.point, s.normal)
                    .into_iter()
                    .flat_map(|(a, b)| [DVec3::from(a) + back, DVec3::from(b) + back])
                    .map(|p| LineVertex::at(p, [0.0, 0.0, 0.0, 1.0]))
                    .collect()
            }
            None => Vec::new(),
        };
        highlight.sort_unstable();
        // `self.edges` is the display's visible edges, put together piece by piece above (each piece's own vertices follow
        // the ones before, so the lists add up in order: what `self.display.visible_edges()` gives, without sorting it all)
        self.sync_textures();
        let tiles: Vec<[f64; 2]> = self.model.textures().iter().map(|t| t.size).collect();
        self.gpu.set_scene(&self.display, &self.tris, &self.edges, &highlight, &self.tools.edges, &self.flagged, (self.faded_from, self.faded_vertex), &tiles);
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
        let face_hit = self.display.pick_triangles(&self.tris, origin.into(), dir.into());

        let mut candidates: Vec<(DVec3, Snap)> = Vec::new();
        for &(a, b) in &self.edges {
            let (pa, pb) = (DVec3::from(self.display.positions[a as usize]), DVec3::from(self.display.positions[b as usize]));
            candidates.push((pa, Snap::Endpoint));
            candidates.push((pb, Snap::Endpoint));
            candidates.push(((pa + pb) * 0.5, Snap::Midpoint));
        }
        candidates.extend(self.tools.pending_points().into_iter().map(|p| (DVec3::from(p), Snap::Endpoint)));
        // the origin, and the point of each axis nearest the mouse (as SketchUp snaps onto the axes)
        let axes = self.model.notes().axes;
        let axes_at = DVec3::from(axes.origin);
        candidates.push((axes_at, Snap::Origin));
        for (i, axis) in axes.dirs.into_iter().map(DVec3::from).enumerate() {
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
                Guide::Point(p) => candidates.push((DVec3::from(p), Snap::OnGuide)),
                Guide::Line(p, d) => {
                    let (p, d) = (DVec3::from(p), DVec3::from(d));
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

        let visible = |p: DVec3| match face_hit {
            Some((_, t)) => (p - origin).dot(dir) <= t * 1.001 + 1e-4,
            None => true,
        };
        let mut best: Option<(DVec3, Snap, f32)> = None;
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

        let face_plane = |f: usize, at: DVec3| Plane::from_point_normal(at.into(), render::newell(&self.display, &self.display.faces[f]));
        let hit = match (best, face_hit) {
            (Some((p, snap, _)), fh) => {
                let on_face = fh.and_then(|(f, _)| {
                    let plane = face_plane(f, p)?;
                    let off = (p - DVec3::from(plane.origin)).dot(DVec3::from(plane.normal)).abs();
                    let face_point = origin + dir * fh?.1;
                    let off_face = (p - face_point).dot(DVec3::from(plane.normal)).abs();
                    (off < 1e-4 && off_face < 1e-3 * (1.0 + p.length())).then_some((f, plane))
                });
                let ground = self.ground();
                let (face, plane) = match on_face {
                    Some((f, plane)) => (Some(f), plane),
                    None if (p - DVec3::from(ground.origin)).dot(DVec3::from(ground.normal)).abs() < 1e-5 => (None, ground),
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
            let pa = self.camera.project(DVec3::from(self.display.positions[a as usize]), w, h)?;
            let pb = self.camera.project(DVec3::from(self.display.positions[b as usize]), w, h)?;
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
            let (x, y) = self.camera.project(DVec3::from(l.at), w, h)?;
            let longest = l.text.lines().map(|t| t.chars().count()).max().unwrap_or(1).max(4) as f32;
            let half = (longest * 3.6 + 6.0) * ppp;
            let tall = (l.text.lines().count().max(1) as f32 * 8.0 + 4.0) * ppp;
            ((self.cursor.0 - x).abs() <= half && (self.cursor.1 - y).abs() <= tall).then_some(i)
        })
    }

    /// The edge or loose line of the geometry being edited nearest the mouse on screen (within a few pixels), if no face
    /// stands in front of it: what a Select click near a line takes, as in SketchUp, rather than the face behind.
    fn edge_at(&self) -> Option<[u32; 2]> {
        let (w, h) = self.size();
        let (origin, dir) = self.camera.ray(self.cursor.0, self.cursor.1, w, h);
        let face_t = self.display.pick_triangles(&self.tris, origin.into(), dir.into()).map(|(_, t)| t);
        let mut best: Option<([u32; 2], f32)> = None;
        for &(a, b) in &self.edges {
            if a >= self.active_vertices || b >= self.active_vertices {
                continue;
            }
            let (pa, pb) = (DVec3::from(self.display.positions[a as usize]), DVec3::from(self.display.positions[b as usize]));
            let (Some(sa), Some(sb)) = (self.camera.project(pa, w, h), self.camera.project(pb, w, h)) else { continue };
            let d = segment_distance(self.cursor, sa, sb);
            if d > EDGE_PICK_RADIUS * self.window.scale_factor() as f32 || best.is_some_and(|(_, bd)| d >= bd) {
                continue;
            }
            // the point of the edge under the mouse must not be hidden by a face nearer the eye (its own faces are at the
            // same depth, so they do not hide it)
            let (dx, dy) = (sb.0 - sa.0, sb.1 - sa.1);
            let l2 = dx * dx + dy * dy;
            let s = if l2 > 0.0 { (((self.cursor.0 - sa.0) * dx + (self.cursor.1 - sa.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let q = pa + (pb - pa) * s as f64;
            let depth = (q - origin).dot(dir);
            if face_t.is_some_and(|t| depth > t * 1.001 + 1e-3 * (1.0 + depth.abs())) {
                continue;
            }
            best = Some(([a.min(b), a.max(b)], d));
        }
        best.map(|(e, _)| e)
    }

    /// What Entity Info shows about the selection (worked out again only when the model or the selection changed).
    fn info(&mut self) -> Option<ui::Info> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.doc, self.model.state(), self.model.open_depth(), &self.tools.selected, &self.tools.edges, &self.tools.instances, self.tools.units.key()).hash(&mut h);
        let key = h.finish();
        if let Some((k, info)) = &self.info_cache {
            if *k == key {
                return info.clone();
            }
        }
        let units = self.tools.units;
        let mesh = self.model.mesh().to_mesh();
        let (faces, edges, inst) = (&self.tools.selected, &self.tools.edges, &self.tools.instances);
        let info = if faces.is_empty() && inst.is_empty() && !edges.is_empty() {
            let curve = self.model.curve_at(edges[0][0], edges[0][1]).filter(|c| {
                let mut ce = c.edges();
                ce.sort_unstable();
                ce == *edges
            });
            match curve {
                Some(c) => Some(ui::Info::Curve {
                    arc: !c.closed,
                    segments: c.segments(),
                    radius: daedalus_core::units::format(c.radius, units),
                    length: daedalus_core::units::format(c.length(&mesh), units),
                    editable: self.readonly.is_none() && daedalus_core::curve::reshapeable(&mesh, &c),
                    key,
                }),
                None if edges.len() == 1 => {
                    let [a, b] = edges[0];
                    let (p, q) = (DVec3::from(mesh.positions[a as usize]), DVec3::from(mesh.positions[b as usize]));
                    Some(ui::Info::Edge { length: daedalus_core::units::format(p.distance(q), units) })
                }
                None => Some(ui::Info::Many(edges.len())),
            }
        } else if faces.len() == 1 && edges.is_empty() && inst.is_empty() && faces[0] < mesh.faces.len() {
            let f = faces[0];
            let area: f64 = mesh.triangulate_face(f).iter().map(|t| {
                let [a, b, c] = t.map(|v| DVec3::from(mesh.positions[v as usize]));
                (b - a).cross(c - a).length() / 2.0
            }).sum();
            let texture = mesh.color_of(f).and_then(|p| p.texture).and_then(|t| self.model.textures().get(t as usize)).map(|t| {
                (t.name.clone(), daedalus_core::units::format(t.size[0], units), daedalus_core::units::format(t.size[1], units), key)
            });
            Some(ui::Info::Face { area: daedalus_core::units::format_area(area, units), holes: mesh.holes_of(f).len(), texture })
        } else if inst.len() == 1 && faces.is_empty() && edges.is_empty() {
            let def = self.model.instances().get(inst[0]).map(|i| i.def);
            def.and_then(|d| self.model.definition(d)).map(|d| ui::Info::Group { name: d.name.clone(), component: d.component, glue: d.glue, cut: d.cut })
        } else if self.tools.has_selection() {
            Some(ui::Info::Many(faces.len() + edges.len() + inst.len()))
        } else {
            None
        };
        self.info_cache = Some((key, info.clone()));
        info
    }

    /// The selected sheet-metal part: its instance, its definition and its description.
    fn sheet_part(&self) -> Option<(usize, usize, daedalus_sheet::part::SheetPart)> {
        let i = self.one_instance()?;
        let def = self.model.instances().get(i)?.def;
        let text = self.model.definition(def)?.part.as_deref().filter(|t| daedalus_sheet::part::is_sheet(t))?;
        daedalus_sheet::part::SheetPart::from_text(text).ok().map(|p| (i, def, p))
    }

    /// The part as the Sheet Metal panel shows it (worked out again only when the model or the selection changed).
    fn sheet_view(&mut self) -> Option<ui::SheetView> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.doc, self.model.state(), self.one_instance()).hash(&mut h);
        let key = h.finish();
        if let Some((k, v)) = &self.sheet_cache {
            if *k == key {
                return v.clone();
            }
        }
        let view = self.sheet_part().map(|(_, def, p)| {
            let (flat_loops, bend_lines) = p.flat_drawing().unwrap_or_default();
            let base = p.outline();
            ui::SheetView {
                name: self.model.definition(def).map(|d| d.name.clone()).unwrap_or_default(),
                material: p.material.clone(),
                thickness: p.rule.thickness,
                k_factor: p.rule.k_factor,
                bend_radius: p.rule.bend_radius,
                relief: (relief_index(p.rule.corner_relief_shape), p.rule.corner_relief, p.rule.relief_width, p.rule.relief_depth),
                flat: p.shown_flat,
                flanges: p
                    .flanges
                    .iter()
                    .map(|f| {
                        use daedalus_sheet::part::{FlangePosition, FoldPosition, StepKind};
                        let kind = f.kind();
                        let pos = if kind == StepKind::Fold {
                            FoldPosition::ALL.iter().position(|q| Some(*q) == FoldPosition::from_key(&f.position)).unwrap_or(0)
                        } else {
                            FlangePosition::ALL.iter().position(|q| Some(*q) == FlangePosition::from_key(&f.position)).unwrap_or(0)
                        };
                        let k = match kind {
                            StepKind::Flange => 0,
                            StepKind::Hem => 1,
                            StepKind::Fold => 2,
                            StepKind::Jog => 3,
                            StepKind::Hole => 4,
                            StepKind::Path => 5,
                        };
                        (f.height, f.angle_deg, pos, f.down, k)
                    })
                    .collect(),
                step_text: p.flanges.iter().map(|f| step_text(&p, f)).collect(),
                base,
                flat_loops,
                bend_lines,
                flat_size: p.flat_size().ok(),
                bends: p.bend_table(),
                checks: p.shop_checks(),
                notes: p.notes.clone(),
                key,
            }
        });
        self.sheet_cache = Some((key, view.clone()));
        view
    }

    /// A change to the selected part: `f` changes it; its faces are made again and put in the model as one undo step.
    fn change_part(&mut self, what: &str, f: impl FnOnce(&mut daedalus_sheet::part::SheetPart) -> Result<(), daedalus_sheet::util::DocError>) {
        if self.readonly.is_some() {
            return;
        }
        let Some((i, def, mut p)) = self.sheet_part() else {
            self.message = Some(format!("{what}: select a sheet-metal part first."));
            return;
        };
        let done = f(&mut p).and_then(|()| p.faces(p.shown_flat)).map_err(|e| e.to_string()).and_then(|faces| self.model.set_part(def, p.to_text(), faces).map_err(|e| e.to_string()));
        match done {
            Ok(()) => self.message = None,
            Err(e) => self.message = Some(format!("{what}: {e}.")),
        }
        self.tools.model_changed();
        // the part stays selected (its instance number does not change), so the panel keeps showing it
        self.tools.selected.clear();
        self.tools.edges.clear();
        self.tools.instances = vec![i];
        self.after_edit();
    }

    /// Make Sheet Metal Part: the selected face becomes a flat part of that material.
    fn make_sheet_part(&mut self, t: f64, k: f64, r: f64, preset: usize) {
        use daedalus_sheet::part::part_from_face;
        use daedalus_sheet::sheet::RuleValues;
        if self.readonly.is_some() {
            return;
        }
        let [f] = self.tools.selected[..] else {
            self.message = Some("Make Sheet Metal Part: select one face.".into());
            return;
        };
        let name = self.material_name(preset, t);
        // square corner reliefs, as cutting shops such as SendCutSend ask for (SolidCraft's own default cuts none)
        let rule = RuleValues { k_factor: k, bend_radius: r, corner_relief_shape: daedalus_sheet::sheet::ReliefShape::Rectangular, ..RuleValues::steel(t) };
        // the face as the model has it (open context coordinates are model coordinates while editing)
        let mesh = self.model.mesh().to_mesh();
        let made = part_from_face(&mesh, f, &name, rule)
            .and_then(|(p, place)| p.faces(false).map(|faces| (p, place, faces)))
            .map_err(|e| e.to_string())
            .and_then(|(p, place, faces)| self.model.place_part("Sheet Part", p.to_text(), &faces, &place, Some(f)).map_err(|e| e.to_string()));
        match made {
            Ok(i) => {
                self.tools.model_changed();
                self.tools.selected.clear();
                self.tools.instances = vec![i];
                self.sheet_tab = 1;
                self.message = Some("Sheet-metal part made. The Flange tool adds flanges on its edges.".into());
            }
            Err(e) => self.message = Some(format!("Make Sheet Metal Part: {e}.")),
        }
        self.after_edit();
    }

    /// The Flange tool's click: a flange on the part's edge nearest the point clicked.
    fn add_flange_at(&mut self, i: usize, point: [f64; 3]) {
        use daedalus_sheet::part::FlangePosition;
        let Some(world) = self.model.instance_world(i) else { return };
        let Some(back) = world.inverse() else { return };
        let def = self.model.instances()[i].def;
        let is_part = self.model.definition(def).and_then(|d| d.part.as_deref()).is_some_and(daedalus_sheet::part::is_sheet);
        if !is_part {
            self.message = Some("Flange: click an edge of a sheet-metal part (Tools > Sheet Metal makes one of a face).".into());
            return;
        }
        // select it so the panel shows it, then add the flange there
        self.tools.selected.clear();
        self.tools.instances = vec![i];
        let local = back.apply(point);
        let pick = [local[0] * 1000.0, local[1] * 1000.0, local[2] * 1000.0];
        let (h, a, pos, down, hem) = self.flange_defaults;
        let position = FlangePosition::ALL[pos.min(FlangePosition::ALL.len() - 1)];
        let flat = self.sheet_part().is_some_and(|p| p.2.shown_flat);
        if flat {
            self.message = Some("Flange: show the part bent first (Sheet Metal panel, 3D tab).".into());
            return;
        }
        if self.path_defaults.0 {
            let legs = self.path_defaults.1.clone();
            self.change_part("Path flange", |p| p.add_path_flange(pick, &legs));
        } else if hem {
            self.change_part("Hem", |p| p.add_hem(pick, h, down));
        } else {
            self.change_part("Flange", |p| p.add_flange(pick, h, a, position, down));
        }
        self.sheet_open = true;
    }

    /// The Fold tool's click: the first point of the line, then the second, which folds (or jogs) the part along it.
    fn fold_at(&mut self, i: usize, point: [f64; 3]) {
        use daedalus_sheet::part::FoldPosition;
        let Some(back) = self.model.instance_world(i).and_then(|w| w.inverse()) else { return };
        let def = self.model.instances()[i].def;
        let is_part = self.model.definition(def).and_then(|d| d.part.as_deref()).is_some_and(daedalus_sheet::part::is_sheet);
        if !is_part {
            self.message = Some("Fold: click a face of a sheet-metal part (Tools > Sheet Metal makes one of a face).".into());
            self.fold_first = None;
            return;
        }
        self.tools.selected.clear();
        self.tools.instances = vec![i];
        if self.sheet_part().is_some_and(|p| p.2.shown_flat) {
            self.message = Some("Fold: show the part bent first (Sheet Metal panel, 3D tab).".into());
            return;
        }
        let local = back.apply(point);
        let pick = [local[0] * 1000.0, local[1] * 1000.0, local[2] * 1000.0];
        match self.fold_first.take() {
            Some((j, a)) if j == i => {
                if dist3(a, pick) < 0.5 {
                    self.fold_first = Some((j, a));
                    self.message = Some("Fold: click a second point, apart from the first, on the line to fold along.".into());
                    return;
                }
                let (jog, angle, step, pos, down) = self.fold_defaults;
                if jog {
                    self.change_part("Jog", |p| p.add_jog(a, pick, step, angle, down));
                } else {
                    let position = FoldPosition::ALL[pos.min(FoldPosition::ALL.len() - 1)];
                    self.change_part("Fold", |p| p.add_fold(a, pick, angle, position, down));
                }
            }
            _ => {
                self.fold_first = Some((i, pick));
                self.message = Some("Fold: now click a second point on the line to fold along.".into());
            }
        }
        self.sheet_open = true;
    }

    /// The Hole tool's click: a hole (or cut-out) of the panel's size centred where the part was clicked, on that panel.
    fn hole_at(&mut self, i: usize, point: [f64; 3]) {
        let Some(back) = self.model.instance_world(i).and_then(|w| w.inverse()) else { return };
        let def = self.model.instances()[i].def;
        let is_part = self.model.definition(def).and_then(|d| d.part.as_deref()).is_some_and(daedalus_sheet::part::is_sheet);
        if !is_part {
            self.message = Some("Hole: click a face of a sheet-metal part (Tools > Sheet Metal makes one of a face).".into());
            return;
        }
        self.tools.selected.clear();
        self.tools.instances = vec![i];
        let local = back.apply(point);
        let pick = [local[0] * 1000.0, local[1] * 1000.0, local[2] * 1000.0];
        let Some((_, _, part)) = self.sheet_part() else { return };
        let Some((panel, at)) = part.panel_at(pick, part.shown_flat) else {
            self.message = Some("Hole: click a flat face of the part, not a bend or an edge.".into());
            self.sheet_open = true;
            return;
        };
        let (round, size) = self.hole_defaults;
        self.change_part("Hole", |p| p.add_hole(panel, at, round, size));
        self.sheet_open = true;
    }

    /// The materials the Sheet Metal panel offers: the built-in presets, then the bend table's rows (name, thickness,
    /// K-factor, inside radius).
    fn materials(&self) -> Vec<(String, f64, f64, f64)> {
        let presets = daedalus_sheet::part::MATERIALS.iter().map(|m| (m.name.to_string(), m.thickness, m.k_factor, m.bend_radius));
        presets.chain(self.bend_table.iter().map(|r| (r.name(), r.thickness, r.k_factor, r.radius))).collect()
    }

    /// The name of material `preset` (into [`materials`](Self::materials)), or one for typed-in values.
    fn material_name(&self, preset: usize, t: f64) -> String {
        self.materials().get(preset).map(|m| m.0.clone()).unwrap_or_else(|| format!("{t} mm sheet"))
    }

    /// The bend table read again from `~/.config/daedalus/bend-table.csv` (none there: no rows).
    fn load_bend_table(&mut self) {
        let text = settings::config_file("bend-table.csv").and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        (self.bend_table, self.bend_table_note) = daedalus_sheet::table::read(&text);
    }

    /// A bend table chosen in the dialog: read, and kept as Daedalus's own copy if any row reads.
    fn import_bend_table(&mut self, path: &std::path::Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.message = Some(format!("Bend table: {e}."));
                return;
            }
        };
        let (rows, note) = daedalus_sheet::table::read(&text);
        if rows.is_empty() {
            self.message = Some(format!("Bend table: no row of {} reads ({}).", short(path), note.first().map(String::as_str).unwrap_or("it is empty")));
            return;
        }
        let kept = settings::config_file("bend-table.csv").ok_or_else(|| "no home folder".to_string()).and_then(|to| {
            std::fs::create_dir_all(to.parent().unwrap_or(&to)).map_err(|e| e.to_string())?;
            std::fs::write(&to, &text).map_err(|e| e.to_string())
        });
        match kept {
            Ok(()) => {
                self.load_bend_table();
                let skipped = if note.is_empty() { String::new() } else { format!(", {} line(s) passed over", note.len()) };
                self.message = Some(format!("Bend table: {} material(s) from {}{skipped}.", rows.len(), short(path)));
            }
            Err(e) => self.message = Some(format!("Bend table not kept: {e}.")),
        }
    }

    /// Stress Test: a click on a solid group or component. A new part (another one, or the same after the model changed) is
    /// read; a click on the part being tested marks the face under it as a support or a load.
    fn stress_pick(&mut self, i: usize, point: [f64; 3]) {
        let state = self.model.state();
        if self.stress.as_ref().is_some_and(|s| s.running()) {
            self.message = Some("Stress Test: wait for the run to finish.".into());
            return;
        }
        let same = self.stress.as_ref().is_some_and(|s| s.instance == i && s.state == state);
        if !same {
            match stressui::Stress::new(&self.model, i) {
                Ok(mut s) => {
                    // a part read again after an edit keeps its settings (its face picks may not fit any more)
                    if let Some(old) = &self.stress {
                        if old.instance == i {
                            s.settings = old.settings;
                        }
                    }
                    self.message = Some(format!("Stress Test {}: click the faces that hold it (supports), then switch to Loads in the panel.", s.name()));
                    self.stress = Some(s);
                }
                Err(e) => self.message = Some(format!("Stress Test: {e}.")),
            }
            self.stress_open = true;
            return;
        }
        if let Some(s) = &mut self.stress {
            self.message = Some(s.pick(point));
        }
    }

    /// Convert to Sheet Metal: a click on a solid group or component. The first picks the face that stays put; the next
    /// ones the edges that become bends (each taken onto the nearest edge of the solid).
    fn convert_pick(&mut self, i: usize, point: [f64; 3]) {
        let Some(back) = self.model.instance_world(i).and_then(|w| w.inverse()) else { return };
        let def = self.model.instances()[i].def;
        let Some(d) = self.model.definition(def) else { return };
        if d.part.is_some() {
            self.message = Some("Convert to Sheet Metal: that is a sheet-metal part already.".into());
            return;
        }
        let local = back.apply(point);
        match &mut self.convert {
            Some((at, Some(_), bends)) if *at == i => {
                let mesh = &d.ctx.mesh;
                let nearest = mesh
                    .face_edges()
                    .map(|(_, a, b)| nearest_on_segment(local, mesh.positions[a as usize], mesh.positions[b as usize]))
                    .min_by(|p, q| dist3(*p, local).total_cmp(&dist3(*q, local)));
                match nearest {
                    Some(p) => {
                        bends.push(p);
                        self.message = Some(format!("Convert: {} bend edge(s) picked; click more, or Convert in the Sheet Metal panel.", bends.len()));
                    }
                    None => self.message = Some("Convert: that solid has no edges.".into()),
                }
            }
            _ => {
                self.convert = Some((i, Some(local), Vec::new()));
                self.tools.selected.clear();
                self.tools.instances = vec![i];
                self.message = Some(format!("Convert {}: the face that stays put is picked; now click each edge that becomes a bend.", d.name));
            }
        }
        self.sheet_open = true;
    }

    /// Convert to Sheet Metal: the picked solid made a part.
    fn convert_sheet(&mut self, t: f64, k: f64, r: f64, preset: usize) {
        use daedalus_sheet::sheet::RuleValues;
        let Some((i, Some(fixed), bends)) = self.convert.clone() else {
            self.message = Some("Convert: click the face that stays put, then the bend edges.".into());
            return;
        };
        let Some(def) = self.model.instances().get(i).map(|x| x.def) else { return };
        let Some(mesh) = self.model.definition(def).map(|d| d.ctx.mesh.clone()) else { return };
        let name = self.material_name(preset, t);
        // square corner reliefs, as cutting shops such as SendCutSend ask for (SolidCraft's own default cuts none)
        let rule = RuleValues { k_factor: k, bend_radius: r, corner_relief_shape: daedalus_sheet::sheet::ReliefShape::Rectangular, ..RuleValues::steel(t) };
        let made = daedalus_sheet::part::part_from_solid(&mesh, fixed, &bends, &name, rule)
            .map(|mut p| {
                // a shelled solid keeps its own walls: named by them when they are not the preset's thickness
                if (p.thickness() - t).abs() > 1e-6 {
                    p.material = format!("{} mm sheet (the solid's walls)", (p.thickness() * 1000.0).round() / 1000.0);
                }
                p
            })
            .and_then(|p| p.faces(false).map(|f| (p, f)))
            .map_err(|e| e.to_string())
            .and_then(|(p, f)| self.model.set_part(def, p.to_text(), f).map_err(|e| e.to_string()).map(|()| p));
        match made {
            Ok(p) => {
                self.convert = None;
                self.tools.model_changed();
                self.tools.instances = vec![i];
                self.sheet_tab = 2;
                self.set_tool(Tool::Select);
                self.message = Some(if p.notes.is_empty() { "Converted to sheet metal.".into() } else { format!("Converted; note: {}", p.notes.join("; ")) });
            }
            Err(e) => self.message = Some(format!("Convert to Sheet Metal: {e}.")),
        }
        self.after_edit();
    }

    /// The selected component for the Dynamic Components panels: its attributes and what each works out to now.
    fn dc_view(&self) -> Option<ui::DcView> {
        let i = self.one_instance()?;
        let inst = *self.model.instances().get(i)?;
        let d = self.model.definition(inst.def)?;
        let values = self.model.attribute_values(i);
        let attrs = d
            .attributes
            .iter()
            .map(|a| {
                let now = match &values {
                    Ok(v) => v.get(&a.name.to_lowercase()).map(|x| x.text()).unwrap_or_default(),
                    Err(e) => format!("error: {e}"),
                };
                (a.clone(), now)
            })
            .collect();
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.doc, self.model.state(), i, inst.def).hash(&mut h);
        Some(ui::DcView { name: d.name.clone(), attrs, key: h.finish() })
    }

    /// The Outliner's rows (worked out again only when the model changed): depth, name, component, path from the top.
    fn outline(&mut self) -> &[(usize, String, bool, Vec<usize>)] {
        let key = self.doc.wrapping_mul(1_000_003) ^ self.model.state();
        if self.outline_cache.as_ref().is_none_or(|(k, _)| *k != key) {
            self.outline_cache = Some((key, self.model.outline()));
        }
        &self.outline_cache.as_ref().expect("just filled").1
    }

    /// The rows as the Outliner shows them: depth, name, component, selected, open for editing.
    fn outline_rows(&mut self) -> Vec<(usize, String, bool, bool, bool)> {
        let open = self.model.open_path().to_vec();
        let picked = self.tools.instances.clone();
        self.outline()
            .iter()
            .map(|(depth, name, component, path)| {
                let (parent, last) = path.split_at(path.len() - 1);
                let selected = parent == open.as_slice() && picked.contains(&last[0]);
                let is_open = path.len() <= open.len() && open[..path.len()] == path[..];
                (*depth, name.clone(), *component, selected, is_open)
            })
            .collect()
    }

    /// An Outliner row picked: the groups it is in are opened and it is selected (an open one is gone back into).
    fn outliner_pick(&mut self, i: usize) {
        if self.readonly.is_some() {
            return;
        }
        let Some(path) = self.outline().get(i).map(|r| r.3.clone()) else { return };
        let open = self.model.open_path().to_vec();
        let is_open = path.len() <= open.len() && open[..path.len()] == path[..];
        let (into, pick) = if is_open { (&path[..], None) } else { (&path[..path.len() - 1], path.last().copied()) };
        if open != into {
            self.model.close_all();
            for &k in into {
                if let Err(e) = self.model.open(k) {
                    self.message = Some(format!("Cannot open it: {e}."));
                    break;
                }
            }
            self.tools.model_changed();
        }
        self.tools.instances.clear();
        self.tools.selected.clear();
        self.tools.edges.clear();
        if let Some(k) = pick {
            self.tools.instances.push(k);
        }
        self.after_edit();
    }

    /// Entity Info on the selected arc or circle: new segments and/or radius.
    fn reshape_curve(&mut self, segments: Option<usize>, radius: Option<f64>) {
        let Some(&[a, b]) = self.tools.edges.first() else { return };
        let Some(c) = self.model.curve_at(a, b) else { return };
        match self.model.reshape_curve(a, b, segments.unwrap_or(c.segments()), radius.unwrap_or(c.radius)) {
            Ok(made) => {
                self.tools.model_changed();
                self.tools.select_edges(&made.edges(), false, false);
                self.message = None;
            }
            Err(e) => self.message = Some(e),
        }
        self.after_edit();
    }

    /// Selects what is under the mouse in the open context: an edge or line near it (a whole curve), else the face or group.
    fn select_under_cursor(&mut self) {
        if let Some(e) = self.edge_at() {
            match self.model.curve_at(e[0], e[1]) {
                Some(c) => self.tools.select_edges(&c.edges(), false, false),
                None => self.tools.select_edge(e, false, false),
            }
        } else {
            let c = self.resolve_cursor();
            let (face, instance) = (c.hit.and_then(|h| h.face), c.hit.and_then(|h| h.instance));
            self.tools.select(&self.model, face, instance, false, false, false);
        }
        self.refresh();
    }

    /// Right-click: what is under the mouse is selected first if it is not already (as in SketchUp), then the menu opens.
    fn open_context_menu(&mut self) {
        if self.tools.tool.edits() || self.tools.tool == Tool::Select {
            if let Some(e) = self.edge_at() {
                if self.tools.edges.binary_search(&e).is_err() {
                    self.tools.select_edge(e, false, false);
                }
            } else {
                let c = self.resolve_cursor();
                let (face, instance) = (c.hit.and_then(|h| h.face), c.hit.and_then(|h| h.instance));
                let selected = face.is_some_and(|f| self.tools.selected.binary_search(&f).is_ok())
                    || instance.is_some_and(|i| self.tools.instances.binary_search(&i).is_ok());
                if !selected && (face.is_some() || instance.is_some()) {
                    self.tools.select(&self.model, face, instance, false, false, false);
                }
            }
            self.refresh();
        }
        self.context_at = Some(self.cursor);
        self.context_fresh = true;
        self.window.request_redraw();
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
                self.camera.stand_at(DVec3::from(h.point), EYE_HEIGHT);
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
        // Select near an edge or loose line takes the line, not the face behind it (a triple-click still takes everything
        // joined)
        if self.tools.tool == Tool::Select && count < 3 {
            if let Some(e) = self.edge_at() {
                if count == 1 {
                    let (add, toggle) = (self.modifiers.control_key(), self.modifiers.shift_key());
                    // an arc or circle is taken whole, as in SketchUp
                    match self.readonly.is_none().then(|| self.model.curve_at(e[0], e[1])).flatten() {
                        Some(c) => self.tools.select_edges(&c.edges(), add, toggle),
                        None => self.tools.select_edge(e, add, toggle),
                    }
                    self.refresh();
                }
                return;
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
            // a sheet-metal part is edited in its panel, not as loose geometry (its faces are made from its description)
            if let Some(i) = c.hit.and_then(|h| h.instance).filter(|&i| {
                let def = self.model.instances()[i].def;
                self.model.definition(def).and_then(|d| d.part.as_deref()).is_some_and(daedalus_sheet::part::is_sheet)
            }) {
                self.tools.selected.clear();
                self.tools.edges.clear();
                self.tools.instances = vec![i];
                self.sheet_open = true;
                self.refresh();
                return;
            }
            if let Some(i) = c.hit.and_then(|h| h.instance) {
                self.open_group(i);
                // and what was double-clicked inside it is selected, a line before the face behind it, as in SketchUp
                self.select_under_cursor();
                return;
            }
        }
        // Smoove, Flip Edge and Add Detail work inside the ground: a click on a ground group opens it, as SketchUp lets them
        // work on it without opening it first
        if matches!(self.tools.tool, Tool::Smoove | Tool::FlipEdge) && self.readonly.is_none() {
            if let Some(i) = c.hit.and_then(|h| h.instance).filter(|_| c.hit.is_some_and(|h| h.face.is_none())) {
                self.open_group(i);
                self.message = Some("Editing the ground: click on it again.".into());
                return;
            }
        }
        // Flip Edge: the edge under the mouse turned the other way
        if self.tools.tool == Tool::FlipEdge {
            if self.readonly.is_some() {
                return;
            }
            match self.edge_at() {
                Some([a, b]) => {
                    if self.model.flip_edge(a, b).is_err() {
                        self.message = Some("Flip Edge: that edge is not between two triangles that can turn it.".into());
                    }
                    self.tools.model_changed();
                    self.after_edit();
                }
                None => self.message = Some("Flip Edge: click an edge between two triangles of the ground (double-click the ground first).".into()),
            }
            return;
        }
        // Stress Test: the part, then its supports and loads
        if self.tools.tool == Tool::Stress {
            match c.hit.and_then(|h| h.instance.map(|i| (i, h.point))) {
                Some((i, point)) => self.stress_pick(i, point),
                None => self.message = Some("Stress Test: click a solid group or component (make one of a solid first).".into()),
            }
            self.refresh();
            return;
        }
        // Convert to Sheet Metal: the face that stays put, then the bend edges
        if self.tools.tool == Tool::SheetConvert {
            if self.readonly.is_some() {
                return;
            }
            match c.hit.and_then(|h| h.instance.map(|i| (i, h.point))) {
                Some((i, point)) => self.convert_pick(i, point),
                None => self.message = Some("Convert to Sheet Metal: click a solid group or component (make one of a solid first).".into()),
            }
            self.refresh();
            return;
        }
        // Fold: two points on a face of a sheet-metal part, the line to fold (or jog) along
        if self.tools.tool == Tool::Fold {
            if self.readonly.is_some() {
                return;
            }
            match c.hit.and_then(|h| h.instance.map(|i| (i, h.point))) {
                Some((i, point)) => self.fold_at(i, point),
                None => self.message = Some("Fold: click a face of a sheet-metal part.".into()),
            }
            self.refresh();
            return;
        }
        // Hole: a hole or cut-out on the flat face of a sheet-metal part under the mouse
        if self.tools.tool == Tool::Hole {
            if self.readonly.is_some() {
                return;
            }
            match c.hit.and_then(|h| h.instance.map(|i| (i, h.point))) {
                Some((i, point)) => self.hole_at(i, point),
                None => self.message = Some("Hole: click a flat face of a sheet-metal part.".into()),
            }
            return;
        }
        // Flange: a flange on the sheet-metal part's edge under the mouse
        if self.tools.tool == Tool::Flange {
            if self.readonly.is_some() {
                return;
            }
            match c.hit.and_then(|h| h.instance.map(|i| (i, h.point))) {
                Some((i, point)) => self.add_flange_at(i, point),
                None => self.message = Some("Flange: click an edge of a sheet-metal part.".into()),
            }
            return;
        }
        // Interact: the component's onClick actions
        if self.tools.tool == Tool::Interact {
            if self.readonly.is_some() {
                return;
            }
            match c.hit.and_then(|h| h.instance) {
                Some(i) => {
                    match self.model.interact(i) {
                        Ok(true) => self.message = None,
                        Ok(false) => self.message = Some("Interact: it has no onClick actions (Component Attributes adds them).".into()),
                        Err(e) => self.message = Some(format!("Interact: {e}.")),
                    }
                    self.tools.model_changed();
                    // it stays selected, so its panels keep showing it
                    self.tools.instances = vec![i];
                    self.after_edit();
                }
                None => self.message = Some("Interact: click a component.".into()),
            }
            return;
        }
        // Stamp and Drape: the object, then the ground
        if matches!(self.tools.tool, Tool::Stamp | Tool::Drape) {
            if self.readonly.is_some() {
                return;
            }
            let Some(inst) = c.hit.and_then(|h| h.instance) else {
                self.message = Some("Click a group: first the object, then the ground.".into());
                return;
            };
            match self.tools.solid_first.take() {
                None => {
                    self.tools.solid_first = Some(inst);
                    self.tools.instances = vec![inst];
                    self.message = None;
                }
                Some(object) => {
                    let drape = self.tools.tool == Tool::Drape;
                    match self.model.sandbox_onto(inst, object, drape, 1.0) {
                        Ok(()) => {
                            self.tools.model_changed();
                            self.message = Some(if drape { "Drape: the edges lie on the ground." } else { "Stamp: the ground is flattened under it." }.into());
                        }
                        Err(e) => {
                            self.tools.instances.clear();
                            self.message = Some(format!("{}: {e}.", self.tools.tool.label()));
                        }
                    }
                }
            }
            self.after_edit();
            return;
        }
        // Solid Tools: a solid group or component, then another
        if let Some(kind) = self.tools.tool.solid() {
            if self.readonly.is_some() {
                return;
            }
            let Some(inst) = c.hit.and_then(|h| h.instance) else {
                self.message = Some("Click a group or component that is a solid.".into());
                return;
            };
            match self.tools.solid_first {
                None => {
                    self.tools.solid_first = Some(inst);
                    self.tools.selected.clear();
                    self.tools.edges.clear();
                    self.tools.instances = vec![inst];
                    self.message = None;
                }
                Some(first) => {
                    self.tools.solid_first = None;
                    match self.model.solid_tool(first, inst, kind) {
                        Ok(made) => {
                            self.tools.model_changed();
                            self.tools.instances = made.clone();
                            self.message = Some(match made.len() {
                                1 => format!("{}: one solid group.", self.tools.tool.label()),
                                n => format!("{}: {n} solid groups.", self.tools.tool.label()),
                            });
                        }
                        Err(e) => {
                            self.tools.instances.clear();
                            self.message = Some(format!("{}: {e}.", self.tools.tool.label()));
                        }
                    }
                }
            }
            self.after_edit();
            return;
        }
        // Eraser near an edge or loose line: it goes (and the faces on it, or two faces in one plane become one), as in SketchUp
        if self.tools.tool == Tool::Eraser && self.readonly.is_none() {
            if let Some(e) = self.edge_at() {
                match self.model.erase_all(&[], &[e], &[]) {
                    Ok(0) => {}
                    Ok(_) => self.message = Some("The edge was kept: the faces either side would not make one simple face.".into()),
                    Err(e) => self.message = Some(e.to_string()),
                }
                self.tools.model_changed();
                self.after_edit();
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
        if matches!(tool, Tool::Flange | Tool::SheetConvert | Tool::Fold | Tool::Hole) {
            self.sheet_open = true;
        }
        self.fold_first = None;
        if tool == Tool::Stress {
            self.stress_open = true;
        }
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

    /// The left button has moved far enough from where it went down to be a drag, not a click.
    fn dragged(&self) -> bool {
        let (dx, dy) = (self.cursor.0 - self.press_at.0, self.cursor.1 - self.press_at.1);
        dx * dx + dy * dy >= 16.0
    }

    /// Select tool drag: left to right takes what lies wholly in the box, right to left whatever it touches (SketchUp's
    /// window and crossing selection). Ctrl adds, Shift toggles, Ctrl+Shift takes out.
    fn box_select(&mut self) {
        let (w, h) = self.size();
        let owners = &self.owners[..self.owners.len().min(self.faded_from.saturating_sub(self.active_faces))];
        let d = boxsel::Display { mesh: &self.display, edges: &self.edges, active_faces: self.active_faces, active_vertices: self.active_vertices, owners };
        let r = boxsel::Rect::new(self.press_at, self.cursor);
        let picked = boxsel::pick(&d, |p| self.camera.project(DVec3::from(p), w, h), r, boxsel::is_crossing(self.press_at, self.cursor));
        self.tools.select_box(&picked.faces, &picked.edges, &picked.instances, self.modifiers.control_key(), self.modifiers.shift_key());
        self.refresh();
    }

    fn erase_selected(&mut self) {
        if self.readonly.is_some() || !self.tools.has_selection() {
            return;
        }
        match self.model.erase_all(&self.tools.selected, &self.tools.edges, &self.tools.instances) {
            Ok(0) => {}
            Ok(n) => {
                self.message = Some(format!(
                    "{n} edge{} kept: the faces either side lie in one plane but would not make one simple face.",
                    if n == 1 { " was" } else { "s were" }
                ))
            }
            Err(e) => self.message = Some(e.to_string()),
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
        let active = self.display_active();
        // imports (STL is 32-bit) are rougher than the model's own tolerance: ten times it
        let report = inspect::inspect(&active, active.tolerance() * 10.0);
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
        let active = self.display_active();
        let (fixed, summary) = inspect::repair(&active, active.tolerance() * 10.0);
        // and coplanar faces merged: an import's triangle pairs become the faces they were drawn as
        let (fixed, _) = daedalus_core::merge::merge_coplanar(&fixed, fixed.tolerance() * 10.0);
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
            self.camera.zoom_extents(DVec3::from(lo), DVec3::from(hi));
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
            UiAction::PaintColor(c) => self.tools.paint_color = c.map(daedalus_core::Paint::from),
            UiAction::PaintTexture(i) => {
                self.tools.paint_color = Some(daedalus_core::Paint { rgb: self.texture_colors.get(i).copied().unwrap_or([200, 200, 200]), texture: Some(i as u16) });
            }
            UiAction::PickTexture => self.choose(Purpose::Texture, ""),
            UiAction::TextureSize(w, h) => {
                let t = self.tools.selected.first().and_then(|&f| self.model.mesh().to_mesh().color_of(f)).and_then(|p| p.texture);
                if let Some(t) = t {
                    if let Err(e) = self.model.set_texture_size(t as usize, [w, h]) {
                        self.message = Some(format!("Not changed: {e}."));
                    }
                    self.after_edit();
                }
            }
            UiAction::SectionCuts => {
                // the current plane cuts or stops (one cuts at most)
                self.model.edit_notes(|n| {
                    if let Some(i) = n.current_section() {
                        let on = !n.sections[i].active;
                        n.activate_section(on.then_some(i));
                    }
                });
                self.refresh();
            }
            UiAction::ActiveSection(i) => {
                self.model.edit_notes(|n| n.activate_section(i));
                self.refresh();
            }
            UiAction::DeleteSection => {
                self.model.edit_notes(|n| {
                    if let Some(i) = n.current_section() {
                        n.sections.remove(i);
                    }
                });
                self.refresh();
            }
            UiAction::DeleteDimensions => self.model.edit_notes(|n| n.dims.clear()),
            UiAction::MakeGroup => self.make_group(false),
            UiAction::Erase => self.erase_selected(),
            UiAction::FromContours => {
                if self.readonly.is_some() {
                    return;
                }
                let edges = self.tools.edges.clone();
                match self.model.sandbox_from_contours(&edges) {
                    Ok(i) => {
                        self.tools.model_changed();
                        self.tools.instances = vec![i];
                        self.message = Some("From Contours: the ground is a group (double-click it to shape it with Smoove).".into());
                    }
                    Err(e) => self.message = Some(format!("From Contours: {e}.")),
                }
                self.after_edit();
            }
            UiAction::AddDetail => {
                if self.readonly.is_some() || self.tools.selected.is_empty() {
                    self.message = Some("Add Detail: select ground faces first (double-click the ground to edit it).".into());
                    return;
                }
                let faces = self.tools.selected.clone();
                if let Err(e) = self.model.add_detail(&faces) {
                    self.message = Some(format!("Add Detail: {e}."));
                }
                self.tools.model_changed();
                self.after_edit();
            }
            UiAction::Outliner => {
                self.outliner_open = !self.outliner_open;
                self.window.request_redraw();
            }
            UiAction::OutlinerPick(i) => self.outliner_pick(i),
            UiAction::ComponentAttributes => {
                self.attributes_open = !self.attributes_open;
                self.window.request_redraw();
            }
            UiAction::ComponentOptions => {
                self.options_open = !self.options_open;
                self.window.request_redraw();
            }
            UiAction::EntityInfo => {
                self.info_open = !self.info_open;
                self.window.request_redraw();
            }
            UiAction::CurveSegments(n) => self.reshape_curve(Some(n), None),
            UiAction::CurveRadius(r) => self.reshape_curve(None, Some(r)),
            UiAction::GlueCut(glue, cut) => {
                if let Some(def) = self.one_instance().and_then(|i| self.model.instances().get(i)).map(|i| i.def) {
                    if let Err(e) = self.model.set_glue_cut(def, glue, cut) {
                        self.message = Some(format!("Not changed: {e}."));
                    }
                    self.after_edit();
                }
            }
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
                self.choose(Purpose::Export, &stem);
            }
            UiAction::ExportStep => {
                // a name of its own: the model may be saved as STEP under its own name already
                let stem = self.path.as_ref().and_then(|p| p.file_stem()).map(|n| format!("{} fabrication.step", n.to_string_lossy())).unwrap_or_else(|| "fabrication.step".into());
                self.choose(Purpose::Step, &stem);
            }
            UiAction::ExportPartStep => match self.sheet_part() {
                Some((_, def, _)) => {
                    let name = self.model.definition(def).map(|d| d.name.replace('#', " ")).unwrap_or_default();
                    self.choose(Purpose::PartStep, &format!("{name}.step"));
                }
                None => self.message = Some("Select a sheet-metal part first.".into()),
            },
            UiAction::ExportFlatDxf(inches) => match self.sheet_part() {
                Some((_, def, _)) => {
                    self.dxf_inches = inches;
                    let name = self.model.definition(def).map(|d| d.name.replace('#', " ")).unwrap_or_default();
                    self.choose(Purpose::FlatDxf, &format!("{name} flat.dxf"));
                }
                None => self.message = Some("Select a sheet-metal part first.".into()),
            },
            UiAction::SheetMetal => {
                self.sheet_open = !self.sheet_open;
                self.window.request_redraw();
            }
            UiAction::StressPanel => {
                self.stress_open = !self.stress_open;
                if !self.stress_open {
                    // shut: the part is drawn as it is again
                    if let Some(s) = &mut self.stress {
                        s.show = false;
                    }
                    if self.tools.tool == Tool::Stress {
                        self.set_tool(Tool::Select);
                    }
                }
                self.refresh();
            }
            UiAction::StressPick(hold) => {
                if let Some(s) = &mut self.stress {
                    s.hold = hold;
                }
                if self.tools.tool != Tool::Stress {
                    self.set_tool(Tool::Stress);
                }
            }
            UiAction::StressClear(supports) => {
                if let Some(s) = &mut self.stress {
                    s.clear(supports);
                }
                self.refresh();
            }
            UiAction::StressSet(set) => {
                if let Some(s) = &mut self.stress {
                    s.settings = set;
                }
            }
            UiAction::StressRun => {
                let state = self.model.state();
                if let Some(s) = &mut self.stress {
                    if s.state != state {
                        self.message = Some("Stress Test: the model changed; click the part again to read it as it is now.".into());
                    } else {
                        match s.run(state) {
                            Ok(()) => self.message = Some("Stress Test: solving…".into()),
                            Err(e) => self.message = Some(format!("Stress Test: {e}.")),
                        }
                    }
                }
                self.window.request_redraw();
            }
            UiAction::StressShow(show, exaggerate) => {
                if let Some(s) = &mut self.stress {
                    s.show = show;
                    s.exaggerate = exaggerate;
                }
                self.refresh();
            }
            UiAction::StressNew => {
                if !self.stress.as_ref().is_some_and(|s| s.running()) {
                    self.stress = None;
                    self.set_tool(Tool::Stress);
                    self.message = Some("Stress Test: click the solid group or component to test.".into());
                    self.refresh();
                }
            }
            UiAction::SheetTab(t) => self.sheet_tab = t,
            UiAction::ConvertSheet(t, k, r, preset) => self.convert_sheet(t, k, r, preset),
            UiAction::ConvertReset => {
                self.convert = None;
                self.message = Some("Convert: click the face that stays put.".into());
            }
            UiAction::MakeSheetPart(t, k, r, preset) => self.make_sheet_part(t, k, r, preset),
            UiAction::SheetRule(t, k, r, preset) => {
                let name = self.material_name(preset, t);
                self.change_part("Material", |p| {
                    let rule = daedalus_sheet::sheet::RuleValues { thickness: t, k_factor: k, bend_radius: r, ..p.rule };
                    p.set_rule(&name, rule)
                });
            }
            UiAction::SheetRelief(shape, corner, width, depth) => self.change_part("Reliefs", |p| {
                let rule = daedalus_sheet::sheet::RuleValues { corner_relief_shape: RELIEFS[shape.min(3)], corner_relief: corner, relief_width: width, relief_depth: depth, ..p.rule };
                let material = p.material.clone();
                p.set_rule(&material, rule)
            }),
            UiAction::SheetFlat(flat) => self.change_part("Show", |p| {
                p.shown_flat = flat;
                Ok(())
            }),
            UiAction::SheetFlange(i, h, a, pos, down) => {
                let position = daedalus_sheet::part::FlangePosition::ALL[pos.min(2)];
                self.change_part("Flange", |p| p.set_flange(i, h, a, position, down));
            }
            UiAction::SheetRemoveFlange(i) => self.change_part("Flange", |p| p.remove_flange(i)),
            UiAction::FlangeDefaults(h, a, pos, down, hem) => self.flange_defaults = (h, a, pos, down, hem),
            UiAction::SheetFold(i, step, a, pos, down) => {
                let position = daedalus_sheet::part::FoldPosition::ALL[pos.min(1)];
                self.change_part("Fold", |p| p.set_fold(i, step, a, position, down));
            }
            UiAction::FoldDefaults(jog, a, step, pos, down) => self.fold_defaults = (jog, a, step, pos, down),
            UiAction::SheetHole(i, at, round, size) => self.change_part("Hole", |p| p.set_hole(i, at, round, size)),
            UiAction::HoleDefaults(round, size) => self.hole_defaults = (round, size),
            UiAction::SheetPath(i, legs) => self.change_part("Path flange", |p| p.set_path(i, legs.get())),
            UiAction::PathDefaults(on, legs) => self.path_defaults = (on, legs.get().to_vec()),
            UiAction::ImportBendTable => self.choose(Purpose::BendTable, ""),
        }
        self.window.request_redraw();
    }

    fn key(&mut self, key: &Key) {
        let ctrl = self.modifiers.control_key();
        if self.context_at.is_some() {
            // the right-click menu takes Esc (it closes); other keys close it and go on as usual
            self.context_at = None;
            self.window.request_redraw();
            if *key == Key::Named(NamedKey::Escape) {
                return;
            }
        }
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
            // Backspace takes back a typed character; with nothing typed it erases the selection, as Delete does (a Mac's,
            // and an iPad's, "delete" key is Backspace)
            Key::Named(NamedKey::Backspace) if !self.typed.is_empty() => {
                self.typed.pop();
            }
            Key::Named(NamedKey::Backspace) => self.erase_selected(),
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
                let end = DVec3::from(axes.origin) + DVec3::from(axes.dirs[i]) * reach * sign;
                out.push(LineVertex::at(axes.origin, color));
                out.push(LineVertex::at(end, color));
            }
        }
        out.extend_from_slice(&self.cut_lines);
        // text leaders, from the point to the text
        for l in &self.model.notes().labels {
            if l.point != l.at {
                out.push(LineVertex::at(l.point, [0.15, 0.15, 0.18, 1.0]));
                out.push(LineVertex::at(l.at, [0.15, 0.15, 0.18, 1.0]));
            }
        }
        // dimensions, in dark grey
        for (a, b, color) in self.tools.dim_lines(&self.model, [0.15, 0.15, 0.18, 1.0]) {
            out.push(LineVertex::at(a, color));
            out.push(LineVertex::at(b, color));
        }
        // guides: lines reach as far as the axes; points are small crosses
        let guides = self.model.notes().guides.iter().copied().chain(self.tools.guide_preview());
        let cross = self.camera.distance * 0.008;
        for g in guides {
            let mut seg = |a: DVec3, b: DVec3| {
                out.push(LineVertex::at(a, GUIDE_COLOR));
                out.push(LineVertex::at(b, GUIDE_COLOR));
            };
            match g {
                Guide::Line(p, d) => {
                    let (p, d) = (DVec3::from(p), DVec3::from(d));
                    // dashed: pieces along the line
                    let n = 400;
                    let step = 2.0 * reach / n as f64;
                    for i in (0..n).step_by(2) {
                        let a = p + d * (-reach + step * i as f64);
                        seg(a, a + d * step);
                    }
                }
                Guide::Point(p) => {
                    let p = DVec3::from(p);
                    for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
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
            .flat_map(|(a, b, color)| [LineVertex::at(a, color), LineVertex::at(b, color)])
            .collect()
    }

    fn render(&mut self) {
        // the stress test's solver runs on its own thread: its result is taken here, and the window redraws while it works
        let mut stress_done = false;
        if let Some(s) = &mut self.stress {
            if s.poll() {
                stress_done = true;
            } else if s.running() {
                self.window.request_redraw();
            }
        }
        if stress_done {
            self.message = Some("Stress Test: done.".into());
            self.refresh();
        }
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
                .and_then(|(p, snap)| self.camera.project(DVec3::from(p), w, h).map(|(x, y)| (egui::pos2(x / ppp, y / ppp), snap)))
            }
            _ => None,
        };
        let (label, current) = self.tools.measurement();
        let info = if self.info_open { self.info() } else { None };
        let outline = if self.outliner_open { self.outline_rows() } else { Vec::new() };
        let dc = if self.attributes_open || self.options_open { self.dc_view() } else { None };
        let sheet = if self.sheet_open || self.context_at.is_some() { self.sheet_view() } else { None };
        let face_for_part = self.readonly.is_none() && self.tools.selected.len() == 1 && self.tools.instances.is_empty();
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
            paint_color: self.tools.paint_color.filter(|p| p.texture.is_none()).map(|p| p.rgb),
            paint_texture: self.tools.paint_color.and_then(|p| p.texture).map(|t| t as usize),
            textures: self.model.textures().iter().map(|t| t.name.clone()).collect(),
            section: self.model.notes().section().map(|s| s.active),
            sections: self.model.notes().sections.iter().map(|s| s.active).collect(),
            has_selection: self.tools.has_selection(),
            info_open: self.info_open,
            info,
            outliner_open: self.outliner_open,
            outline,
            attributes_open: self.attributes_open,
            options_open: self.options_open,
            dc,
            dc_out: &self.dc_out,
            sheet_open: self.sheet_open,
            sheet_tab: self.sheet_tab,
            sheet,
            face_for_part,
            flange_defaults: self.flange_defaults,
            fold_defaults: self.fold_defaults,
            fold_first: self.fold_first.is_some(),
            hole_defaults: self.hole_defaults,
            path_defaults: self.path_defaults.clone(),
            materials: self.materials(),
            table_rows: self.bend_table.len(),
            table_note: self.bend_table_note.clone(),
            stress_open: self.stress_open,
            stress: self.stress.as_ref().map(|s| s.view(self.model.state())),
            converting: self.convert.as_ref().and_then(|(i, fixed, bends)| {
                let def = self.model.instances().get(*i)?.def;
                Some((self.model.definition(def)?.name.clone(), fixed.is_some(), bends.len()))
            }),
            one_instance: self.one_instance().and_then(|i| self.model.instances().get(i)).and_then(|inst| self.model.definition(inst.def)).map(|d| d.component),
            editing: (self.model.open_depth() > 0).then(|| self.model.open_names().join(" › ")),
            labels: self
                .tools
                .dim_labels(&self.model)
                .into_iter()
                .filter_map(|(p, t)| self.camera.project(DVec3::from(p), w, h).map(|(x, y)| (egui::pos2(x / ppp, y / ppp), t)))
                .collect(),
            texts: self
                .model
                .notes()
                .labels
                .iter()
                .enumerate()
                .filter(|(i, _)| self.label_edit.as_ref().is_none_or(|e| e.0 != *i))
                .filter_map(|(_, l)| self.camera.project(DVec3::from(l.at), w, h).map(|(x, y)| (egui::pos2(x / ppp, y / ppp), l.text.clone())))
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
        // Select: the box being dragged (dashed when it goes right to left: a crossing box)
        let select_rect = (self.left_down && self.tools.tool == Tool::Select && self.dragged()).then(|| {
            let r = egui::Rect::from_two_pos(egui::pos2(self.press_at.0 / ppp, self.press_at.1 / ppp), egui::pos2(self.cursor.0 / ppp, self.cursor.1 / ppp));
            (r, boxsel::is_crossing(self.press_at, self.cursor))
        });
        let raw = self.egui_state.take_egui_input(&self.window);
        let mut actions = Vec::new();
        // the file dialog and the "save changes?" question are drawn over the chrome; taken out while drawn
        let mut dialog = self.dialog.take();
        let mut label_edit = self.label_edit.take();
        let label_pos = label_edit
            .as_ref()
            .and_then(|(i, _)| self.model.notes().labels.get(*i))
            .and_then(|l| self.camera.project(DVec3::from(l.at), w, h))
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
        let context_at = self.context_at;
        let context_fresh = std::mem::take(&mut self.context_fresh);
        let mut context_open = context_at.is_some();
        // the notice: drawn beside the work, which goes on (true: its button pressed, false: closed)
        let notice = self.notice.take();
        let mut notice_answer: Option<bool> = None;
        let out = ctx.run(raw, |ctx| {
            actions = ui::draw(ctx, &view);
            if let Some(n) = &notice {
                egui::Window::new(n.title.as_str())
                    .id(egui::Id::new("notice"))
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -40.0])
                    .show(ctx, |ui| {
                        ui.set_max_width(380.0);
                        for line in &n.lines {
                            ui.label(line);
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if let Some(copy) = &n.copy {
                                let label = format!("Also save a full copy ({})", short(copy));
                                if ui.button(label).on_hover_text("A .ddls file keeps everything; the file just saved stays as it is.").clicked() {
                                    notice_answer = Some(true);
                                }
                            }
                            if ui.button("Close").clicked() {
                                notice_answer = Some(false);
                            }
                        });
                    });
            }
            if let Some((x, y)) = context_at {
                let (picked, open) = ui::context_menu(ctx, egui::pos2(x / ppp, y / ppp), &view, context_fresh);
                actions.extend(picked);
                context_open = open;
            }
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
            if let Some((r, crossing)) = select_rect {
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("select-box")));
                let stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_gray(40));
                if crossing {
                    let c = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                    painter.extend(egui::Shape::dashed_line(&c, stroke, 5.0, 4.0));
                } else {
                    painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
                }
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
                    // the font, from those on the system (the list is read once, the first time it is opened)
                    let fonts = text3d::fonts();
                    let now = fonts.iter().find(|x| x.path == f.font).map_or("Ubuntu Light (built in)", |x| x.name.as_str());
                    egui::ComboBox::from_label("Font").selected_text(now).width(220.0).height(320.0).show_ui(ui, |ui| {
                        for x in fonts {
                            ui.selectable_value(&mut f.font, x.path.clone(), &x.name);
                        }
                    });
                    ui.label(egui::RichText::new("Lengths in any unit (300mm, 1').").small());
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
        // a notice set while drawing (none is) wins over the one shown
        if self.notice.is_none() {
            self.notice = notice;
        }
        match notice_answer {
            Some(true) => {
                if let Some(copy) = self.notice.as_ref().and_then(|n| n.copy.clone()) {
                    self.full_copy(copy);
                }
            }
            Some(false) => self.notice = None,
            None => {}
        }
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
                        (Some(h), Some(d)) => {
                            settings::save("text3d-font", &f.font.as_ref().map(|p| p.display().to_string()).unwrap_or_default());
                            self.tools.begin_text3d(&f.text.clone(), h, d, f.font.clone().as_deref())
                        }
                        _ => Err("cannot read the height or the depth".into()),
                    };
                    self.message = r.err();
                }
                Some(false) => self.set_tool(Tool::Select),
                None => {}
            }
            self.window.request_redraw();
        }
        // over the model, the tool's own cursor; egui is told the arrow then, so it sets its own again when the pointer is
        // back over the toolbar or a menu
        let want = (self.pointer_in && !self.over_ui && resize_at.is_none() && self.context_at.is_none() && cursors::has_cursor(self.tools.tool) && self.tool_cursors.contains_key(&self.tools.tool)).then_some(self.tools.tool);
        let mut platform = out.platform_output;
        if want.is_some() {
            platform.cursor_icon = egui::CursorIcon::Default;
        }
        self.egui_state.handle_platform_output(&self.window, platform);
        if want != self.cursor_shown {
            match want.and_then(|t| self.tool_cursors.get(&t)) {
                Some(c) => self.window.set_cursor(c.clone()),
                None => self.window.set_cursor(winit::window::CursorIcon::Default),
            }
            self.cursor_shown = want;
        }
        let ui = UiFrame {
            primitives: ctx.tessellate(out.shapes, out.pixels_per_point),
            textures: out.textures_delta,
            pixels_per_point: out.pixels_per_point,
        };
        let (axes, overlay) = (self.axes(), self.overlay());
        // edges about 1.5 points wide, like SketchUp's default
        let line_width = 1.4 * self.window.scale_factor() as f32;
        let section = self.model.notes().cutting().map(|s: Section| (render::gpu(s.point), render::gpu(s.normal)));
        let eye = if self.camera.ortho {
            let d = (self.camera.eye() - self.camera.target).normalize_or_zero();
            [d.x as f32, d.y as f32, d.z as f32, 0.0]
        } else {
            let e = self.camera.eye();
            [e.x as f32, e.y as f32, e.z as f32, 1.0]
        };
        self.gpu.render(self.camera.view_proj(w / h), eye, self.style, line_width, section, &axes, &overlay, ui);

        if out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay.is_zero()) {
            self.window.request_redraw();
        }
        if !context_open && self.context_at.is_some() {
            self.context_at = None;
            self.window.request_redraw();
        }
        for a in actions {
            self.apply(a);
        }
        // the Dynamic Components panels' changes: the selected component's attributes, all of them
        let handed = self.dc_out.borrow_mut().take();
        if let Some((list, this_copy)) = handed {
            if let Some((i, def)) = self.one_instance().and_then(|i| self.model.instances().get(i).map(|x| (i, x.def))) {
                // Component Options change the copy selected (it gets a component of its own when shared); Component
                // Attributes the component, every copy of it
                let done = if this_copy { self.model.set_copy_attributes(i, list) } else { self.model.set_attributes(def, list) };
                match done {
                    Ok(()) => self.message = None,
                    Err(e) => self.message = Some(format!("Not applied: {e}.")),
                }
                let keep = self.tools.instances.clone();
                self.after_edit();
                self.tools.instances = keep;
                self.refresh();
            }
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

/// A SketchUp model read with `skp-convert --one` (OpenSKP, installed apart from Daedalus), which writes a `.dskp` into the
/// cache that daedalus-io then reads.
fn from_sketchup(path: &Path) -> Result<Scene, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let converter = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .chain([home.join(".local/bin")])
        .map(|d| d.join("skp-convert"))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("{} is a SketchUp file. Reading those needs the skp-convert helper (OpenSKP), which is not installed here.", short(path)))?;
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("daedalus/sketchup");
    let out = cache.join(format!("{}.dskp", path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "model".into())));
    let run = std::process::Command::new(&converter).arg("--one").arg(path).arg(&out).output().map_err(|e| format!("skp-convert did not start: {e}"))?;
    if !run.status.success() {
        let why = String::from_utf8_lossy(&run.stderr);
        let last = why.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no reason given").to_string();
        return Err(format!("{} could not be read: {last}", short(path)));
    }
    daedalus_io::open_scene(&out).map_err(|e| format!("{} could not be read: {e}", short(path)))
}

/// Each face's triangles.
type Tris = Vec<Vec<[u32; 3]>>;

/// A file's name for a message (the whole path would run into the Measurements box).
fn short(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

/// After a save: what the file's format left out, in a notice with a button to save a full `.ddls` copy beside it; none when
/// nothing was left out.
fn saved_notice(path: &Path, left_out: &[String]) -> Option<Notice> {
    (!left_out.is_empty()).then(|| Notice {
        title: format!("Saved {}", short(path)),
        lines: std::iter::once("These are not in it (the file's format has no place for them):".to_string()).chain(left_out.iter().map(|l| format!("•  {l}"))).collect(),
        copy: Some(path.with_extension(daedalus_io::formats::Format::Native.extension())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_that_left_something_out_says_what_and_offers_a_full_copy() {
        assert!(saved_notice(Path::new("/m/house.step"), &[]).is_none(), "nothing left out: no notice");
        let n = saved_notice(Path::new("/m/house.step"), &["1 guide".to_string(), "2 textures".to_string()]).unwrap();
        assert_eq!(n.title, "Saved house.step");
        assert_eq!(n.lines[1..], ["•  1 guide".to_string(), "•  2 textures".to_string()]);
        assert_eq!(n.copy, Some(PathBuf::from("/m/house.ddls")), "the copy goes beside the file, as .ddls");
    }
}

/// Events from outside the window: the desktop switched between light and dark; the file chooser answered.
#[derive(Debug, Clone)]
enum AppEvent {
    Theme { dark: bool },
    #[cfg(target_os = "linux")]
    Picked(Purpose, portal::Answer),
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
        let Some(s) = self.state.as_mut() else { return };
        match event {
            AppEvent::Theme { dark } => {
                s.egui_ctx.set_visuals(theme::visuals(dark));
                s.window.request_redraw();
            }
            #[cfg(target_os = "linux")]
            AppEvent::Picked(purpose, answer) => s.picked(purpose, answer),
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
        self.state = Some(State::new(window, &self.scene, self.path.clone(), dark, self.proxy.clone()));
        // the tools' cursors, at the window's scale
        if let Some(s) = self.state.as_mut() {
            let size = ((32.0 * s.window.scale_factor()).round() as u32).clamp(24, 128);
            for t in Tool::ALL.into_iter().filter(|&t| cursors::has_cursor(t)) {
                let img = cursors::image(t, size);
                if let Ok(src) = winit::window::CustomCursor::from_rgba(img.rgba, size as u16, size as u16, img.hotspot.0, img.hotspot.1) {
                    s.tool_cursors.insert(t, event_loop.create_custom_cursor(src));
                }
            }
        }
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
                    // the window and the GPU go now, while the event loop (and its Wayland connection) is still there: dropped
                    // after it, the graphics driver's clean-up talks to a connection that is gone and crashes
                    self.state = None;
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
                // a press outside the open right-click menu closes it and does nothing else
                if down && s.context_at.is_some() {
                    s.context_at = None;
                    s.window.request_redraw();
                    return;
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
                            } else if s.tools.tool == Tool::Select {
                                s.box_select();
                            }
                        }
                    }
                    MouseButton::Middle => s.middle_down = down,
                    MouseButton::Right => {
                        if down {
                            s.right_press_at = s.cursor;
                        } else if s.right_down {
                            let (dx, dy) = (s.cursor.0 - s.right_press_at.0, s.cursor.1 - s.right_press_at.1);
                            if dx * dx + dy * dy < 16.0 {
                                s.open_context_menu();
                            }
                        }
                        s.right_down = down;
                    }
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
                // right-drag pans (a right-click without a drag opens the menu).
                let orbit = (s.middle_down && !s.modifiers.shift_key()) || (s.left_down && tool == Tool::Orbit);
                let pan = (s.middle_down && s.modifiers.shift_key()) || s.right_down || (s.left_down && tool == Tool::Pan);
                if orbit {
                    s.camera.orbit(dx, dy);
                } else if pan {
                    s.camera.pan(dx, dy);
                } else if s.left_down && tool == Tool::Zoom {
                    s.camera.zoom(-dy * 0.05, None);
                } else if s.left_down && (tool == Tool::ZoomWindow || tool == Tool::Select) {
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
                let focus = s.resolve_cursor().hit.map(|h| DVec3::from(h.point));
                s.camera.zoom(amount, focus);
                s.mouse_moved();
                s.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    // a file given (the file manager opening one, say) is opened once the window is up, as File > Open opens it: a SketchUp
    // model through skp-convert, a STEP file with what it does not read named; a file that does not open says why there
    let path = std::env::args().nth(1).map(PathBuf::from);
    let (scene, title) = (Scene::default(), "Daedalus".to_string());
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

/// The point of segment ab nearest p.
fn nearest_on_segment(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let l2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let t = if l2 > 0.0 { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1] + (p[2] - a[2]) * d[2]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    [a[0] + t * d[0], a[1] + t * d[1], a[2] + t * d[2]]
}

/// The corner relief shapes as the Sheet Metal panel numbers them.
const RELIEFS: [daedalus_sheet::sheet::ReliefShape; 4] = [
    daedalus_sheet::sheet::ReliefShape::Notch,
    daedalus_sheet::sheet::ReliefShape::Round,
    daedalus_sheet::sheet::ReliefShape::Rectangular,
    daedalus_sheet::sheet::ReliefShape::Tear,
];

fn relief_index(shape: daedalus_sheet::sheet::ReliefShape) -> usize {
    RELIEFS.iter().position(|r| *r == shape).unwrap_or(0)
}

/// The text a step's row shows where numbers do not say it: a hole's size, where it sits and its panel; a path flange's legs.
fn step_text(p: &daedalus_sheet::part::SheetPart, f: &daedalus_sheet::part::FlangeAsked) -> (String, String, String) {
    let fmt = |x: f64| {
        let s = format!("{x:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    if let Some(h) = &f.hole {
        // the panel by the step that made it
        let on = match h.panel {
            None => "base".to_string(),
            Some(k) => {
                let mut made = 0;
                let mut name = format!("flange {}", k + 1);
                for (n, g) in p.flanges.iter().enumerate() {
                    let c = match g.kind() {
                        daedalus_sheet::part::StepKind::Hole => 0,
                        daedalus_sheet::part::StepKind::Jog => 2,
                        daedalus_sheet::part::StepKind::Path => g.path.len(),
                        _ => 1,
                    };
                    if (made..made + c).contains(&k) {
                        name = if c > 1 { format!("step {} ({})", n + 1, k - made + 1) } else { format!("step {}", n + 1) };
                        break;
                    }
                    made += c;
                }
                // a converted part's own bends come first
                if p.solid.is_some() { format!("flange {}", k + 1) } else { name }
            }
        };
        return (h.size_text(), format!("{}, {}", fmt(h.at[0]), fmt(h.at[1])), on);
    }
    if !f.path.is_empty() {
        return (daedalus_sheet::part::legs_text(&f.path), String::new(), String::new());
    }
    (String::new(), String::new(), String::new())
}

fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
