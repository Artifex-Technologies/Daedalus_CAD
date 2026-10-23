mod camera;
mod render;
mod tools;
mod ui;

use camera::Camera;
use daedalus_core::inspect::{self, DEFAULT_TOLERANCE};
use daedalus_core::{HalfEdgeMesh, Mesh, Model, Plane};
use glam::Vec3;
use render::{LineVertex, Renderer, UiFrame};
use std::path::PathBuf;
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
}

impl State {
    fn new(window: Arc<Window>, mesh: &Mesh) -> State {
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
        };
        s.refresh();
        s
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
        let out = ctx.run(raw, |ctx| actions = ui::draw(ctx, &view));
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
    }
}

struct App {
    mesh: Mesh,
    title: String,
    state: Option<State>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title(self.title.clone()))
                .expect("create window"),
        );
        self.state = Some(State::new(window, &self.mesh));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let response = s.egui_state.on_window_event(&s.window, &event);
        if response.repaint {
            s.window.request_redraw();
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                s.gpu.resize(size.width, size.height);
                s.window.request_redraw();
            }
            WindowEvent::RedrawRequested => s.render(),
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
    let (mesh, title) = match std::env::args().nth(1) {
        Some(arg) => {
            let path = PathBuf::from(&arg);
            match daedalus_io::import_mesh(&path) {
                Ok(m) => (m, format!("Daedalus - {arg}")),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => (Mesh::new(), "Daedalus".to_string()),
    };
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App { mesh, title, state: None };
    event_loop.run_app(&mut app).expect("run app");
}
