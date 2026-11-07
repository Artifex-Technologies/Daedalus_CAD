//! Stress Test (Tools > Stress Test, and its panel in the tray): will a part hold?
//!
//! Click a solid group or component, then the faces that hold it (supports, shown blue) and the faces that are pushed (loads,
//! shown orange); give the force, its direction and the material; Run. The part is filled with cubes and solved
//! (`daedalus_fea`) on a thread of its own, so the window stays live; the result paints the part from blue (no stress) to red
//! (the highest), bent by the load (exaggerated so the bend can be seen), with the highest stress, the safety factor against
//! the material's yield strength and the largest deflection in the panel.
//!
//! Linear and static only: the material stays springy and deflections are small. Buckling, fatigue, bolts, welds and contact
//! are not modelled; the panel says so.

use crate::ui::{panel_head, UiAction, UiView};
use daedalus_core::halfedge::HalfEdgeMesh;
use daedalus_core::mesh::{Mesh, Paint};
use daedalus_core::model::Model;
use daedalus_fea::{Load, Material, Part, Report, Study};
use egui::{Color32, RichText};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const DIRECTIONS: [&str; 8] = ["Down (−Z)", "Up (+Z)", "Along +X (red)", "Along −X", "Along +Y (green)", "Along −Y", "Into the faces", "Out of the faces"];

/// Supports are drawn this blue, loads this orange.
const SUPPORT: [u8; 3] = [40, 110, 230];
const LOAD: [u8; 3] = [240, 140, 20];

/// What the panel's fields hold: the force (N), its direction (index into `DIRECTIONS`), the part's own weight, the material
/// (index into `Material::presets`) and the cubes along the part's longest side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub force: f64,
    pub direction: usize,
    pub gravity: bool,
    pub material: usize,
    pub divisions: usize,
}

/// A result, as the panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultView {
    pub max_mpa: f64,
    pub yield_mpa: f64,
    pub safety: f64,
    pub max_mm: f64,
    pub mass_kg: f64,
    pub cubes: usize,
    pub cube_mm: f64,
    pub seconds: f64,
    pub converged: bool,
    pub notes: Vec<String>,
    /// The scale the bend is drawn at (1: true size).
    pub scale: f64,
}

/// What the Stress Test panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct StressView {
    /// The part being tested (None: none picked yet).
    pub name: Option<String>,
    /// Clicks add supports (true) or loads.
    pub hold: bool,
    pub supports: usize,
    pub loads: usize,
    pub settings: Settings,
    /// A sheet-metal part's thickness (mm), and the part's longest side (mm), for the cube size.
    pub thickness_mm: Option<f64>,
    pub longest_mm: f64,
    /// Running: how far, and what it is doing.
    pub running: Option<(f32, String)>,
    pub result: Option<ResultView>,
    pub error: Option<String>,
    pub show: bool,
    pub exaggerate: bool,
    /// The model changed since the result was worked out.
    pub stale: bool,
}

struct Job {
    progress: Arc<Mutex<(f32, String)>>,
    done: Arc<Mutex<Option<Result<Report, String>>>>,
    started: Instant,
    state: u64,
}

/// A stress test under way: the part, its picks and settings, and the last result.
pub struct Stress {
    pub instance: usize,
    /// The model state the part was read at (a change to the model makes a new part to read).
    pub state: u64,
    name: String,
    /// The part in model coordinates (metres), face numbers as the display numbers the instance's faces.
    mesh: Mesh,
    part: Part,
    thickness_mm: Option<f64>,
    pub hold: bool,
    supports: Vec<usize>,
    loads: Vec<usize>,
    pub settings: Settings,
    job: Option<Job>,
    result: Option<(Report, f64, u64)>,
    error: Option<String>,
    pub show: bool,
    pub exaggerate: bool,
    /// The result drawn (exaggerated or not), with its triangles.
    drawn: Option<(bool, Mesh, Vec<Vec<[u32; 3]>>)>,
    /// The part with its supports and loads painted, with its triangles.
    painted: Option<(Mesh, Vec<Vec<[u32; 3]>>)>,
}

/// Instance `i` of the open context as a part: its faces in model coordinates, closed all round.
fn read_part(model: &Model, i: usize) -> Result<(String, Mesh, Part, Option<f64>), String> {
    let inst = *model.instances().get(i).ok_or("that is not a group or component here")?;
    let def = model.definition(inst.def).ok_or("that group has no definition")?;
    let (inside, _) = model.parts();
    let mesh = inside.into_iter().find(|(j, _)| *j == i).map(|(_, m)| m).ok_or("that group is not here")?;
    let closed = HalfEdgeMesh::from_mesh(&mesh).map(|h| h.is_closed()).unwrap_or(false);
    if !closed || mesh.faces.is_empty() {
        return Err(format!("{} is not a solid (it must be closed all round, every edge between two faces); Tools > Solid Inspector finds the gaps", def.name));
    }
    let mut part = Part::default();
    for (f, tris) in mesh.face_triangles().iter().enumerate() {
        for t in tris {
            part.triangles.push(t.map(|v| mesh.positions[v as usize]));
            part.face_of.push(f);
        }
    }
    let thickness = def.part.as_deref().and_then(|j| daedalus_sheet::part::SheetPart::from_text(j).ok()).map(|p| p.thickness());
    Ok((def.name.clone(), mesh, part, thickness))
}

fn longest(part: &Part) -> f64 {
    let (lo, hi) = daedalus_fea::voxel::bounds(&part.triangles);
    (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max)
}

/// Cubes along the longest side: enough for two across a sheet's thickness (as many as allowed), else 0 (the solver chooses:
/// the coarsest grid that fills the part's volume).
fn suggested_divisions(longest_m: f64, thickness_mm: Option<f64>) -> usize {
    match thickness_mm {
        Some(t) if t > 0.0 => ((longest_m * 1000.0 / (t / 2.0)).ceil() as usize).clamp(40, 300),
        _ => 0,
    }
}

/// A force typed with its unit: N (the default), kN, kg or kgf (its weight), lb or lbf.
pub fn parse_force(text: &str) -> Option<f64> {
    let t = text.trim().to_lowercase();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let v: f64 = num.trim().parse().ok()?;
    let factor = match unit.trim() {
        "" | "n" => 1.0,
        "kn" => 1000.0,
        "kg" | "kgf" => 9.80665,
        "lb" | "lbs" | "lbf" => 4.448_222,
        _ => return None,
    };
    Some(v * factor).filter(|f| f.is_finite())
}

/// Blue (none) through cyan, green and yellow to red (`t` = 1, the highest).
fn ramp(t: f64) -> [u8; 3] {
    let stops: [[f64; 3]; 5] = [[0.15, 0.3, 0.95], [0.1, 0.8, 0.95], [0.2, 0.85, 0.3], [0.98, 0.85, 0.15], [0.92, 0.15, 0.1]];
    let x = t.clamp(0.0, 1.0) * 4.0;
    let i = (x.floor() as usize).min(3);
    let f = x - i as f64;
    std::array::from_fn(|c| ((stops[i][c] * (1.0 - f) + stops[i + 1][c] * f) * 255.0).round() as u8)
}

impl Stress {
    /// A stress test of instance `i`.
    pub fn new(model: &Model, i: usize) -> Result<Stress, String> {
        let (name, mesh, part, thickness_mm) = read_part(model, i)?;
        let divisions = suggested_divisions(longest(&part), thickness_mm);
        Ok(Stress {
            instance: i,
            state: model.state(),
            name,
            mesh,
            part,
            thickness_mm,
            hold: true,
            supports: vec![],
            loads: vec![],
            settings: Settings { force: 100.0, direction: 0, gravity: false, material: 0, divisions },
            job: None,
            result: None,
            error: None,
            show: false,
            exaggerate: true,
            drawn: None,
            painted: None,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// A click on the part: the face under it becomes a support or a load (or stops being one). Returns a message.
    pub fn pick(&mut self, point: [f64; 3]) -> String {
        let Some(f) = daedalus_fea::nearest_face(&self.part, point) else { return "that part has no faces".into() };
        self.show = false;
        let (list, other, what) =
            if self.hold { (&mut self.supports, &mut self.loads, "support") } else { (&mut self.loads, &mut self.supports, "load") };
        other.retain(|x| *x != f);
        if let Some(at) = list.iter().position(|x| *x == f) {
            list.remove(at);
            format!("Stress Test: that face is no longer a {what}.")
        } else {
            list.push(f);
            format!("Stress Test: {} {what} face(s); click more, or Run in the panel.", list.len())
        }
    }

    pub fn clear(&mut self, supports: bool) {
        if supports { self.supports.clear() } else { self.loads.clear() }
        self.show = false;
    }

    /// The load's direction: one of the axes, or into (out of) the load faces (their average normal).
    fn direction(&self) -> [f64; 3] {
        match self.settings.direction {
            0 => [0.0, 0.0, -1.0],
            1 => [0.0, 0.0, 1.0],
            2 => [1.0, 0.0, 0.0],
            3 => [-1.0, 0.0, 0.0],
            4 => [0.0, 1.0, 0.0],
            5 => [0.0, -1.0, 0.0],
            d => {
                let mut n = [0.0; 3];
                for &f in &self.loads {
                    let fn_ = crate::render::newell(&self.mesh, &self.mesh.faces[f]);
                    for a in 0..3 {
                        n[a] += fn_[a];
                    }
                }
                // the part's faces look outward: into the faces is against their normal
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
                let s = if d == 6 { -1.0 } else { 1.0 };
                // a part wound inward is turned round by the solver; follow it here
                let turned = if daedalus_fea::voxel::mesh_volume(&self.part.triangles) < 0.0 { -1.0 } else { 1.0 };
                n.map(|c| s * turned * c / l)
            }
        }
    }

    /// Starts the solver on its own thread.
    pub fn run(&mut self, model_state: u64) -> Result<(), String> {
        if self.job.is_some() {
            return Err("it is running already".into());
        }
        if self.supports.is_empty() {
            return Err("click at least one face that holds the part (a support)".into());
        }
        if self.loads.is_empty() && !self.settings.gravity {
            return Err("click the faces that are pushed (a load), or tick the part's own weight".into());
        }
        let material = Material::presets().get(self.settings.material).cloned().unwrap_or_else(|| Material::presets()[0].clone());
        let mut study = Study::new(material);
        study.fixed = self.supports.clone();
        study.divisions = self.settings.divisions;
        if !self.loads.is_empty() && self.settings.force != 0.0 {
            let d = self.direction();
            study.loads = vec![Load { faces: self.loads.clone(), force: d.map(|c| c * self.settings.force) }];
        }
        study.gravity = self.settings.gravity.then_some([0.0, 0.0, -9.80665]);
        let part = self.part.clone();
        let progress = Arc::new(Mutex::new((0.0f32, "starting".to_string())));
        let done = Arc::new(Mutex::new(None));
        let (p, d) = (progress.clone(), done.clone());
        std::thread::Builder::new()
            .name("stress test".into())
            .spawn(move || {
                let r = daedalus_fea::run(&part, &study, &mut |f, what| {
                    if let Ok(mut g) = p.lock() {
                        *g = (f, what.to_string());
                    }
                });
                if let Ok(mut g) = d.lock() {
                    *g = Some(r);
                }
            })
            .map_err(|e| format!("could not start the solver: {e}"))?;
        self.error = None;
        self.job = Some(Job { progress, done, started: Instant::now(), state: model_state });
        Ok(())
    }

    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Takes the solver's result when it is done. Returns true when something changed (a result or an error arrived).
    pub fn poll(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        let Some(r) = job.done.lock().ok().and_then(|mut g| g.take()) else { return false };
        let (seconds, state) = (job.started.elapsed().as_secs_f64(), job.state);
        self.job = None;
        match r {
            Ok(report) => {
                self.result = Some((report, seconds, state));
                self.show = true;
                self.drawn = None;
            }
            Err(e) => self.error = Some(e),
        }
        true
    }

    /// How much the bend is drawn larger than it is: so the largest deflection shows as a twentieth of the part's size.
    fn scale(&self) -> f64 {
        match &self.result {
            Some((r, _, _)) if self.exaggerate && r.max_displacement > 0.0 => (longest(&self.part) / 20.0 / r.max_displacement).max(1.0),
            _ => 1.0,
        }
    }

    /// What to draw for the instance instead of its own faces: the result (when shown), or the part with its supports and
    /// loads painted (while it is being set up). None: draw it as it is.
    pub fn display(&mut self, model_state: u64) -> Option<(&Mesh, &[Vec<[u32; 3]>])> {
        if model_state != self.state {
            return None;
        }
        if self.show && self.result.is_some() {
            if self.drawn.as_ref().is_none_or(|d| d.0 != self.exaggerate) {
                let scale = self.scale();
                let (r, _, _) = self.result.as_ref().unwrap();
                let top = r.max_stress.max(1e-30);
                let mut m = Mesh::new();
                for q in &r.surface {
                    let base = m.positions.len() as u32;
                    for c in 0..4 {
                        m.positions.push(std::array::from_fn(|a| q.corners[c][a] + q.displacement[c][a] * scale));
                    }
                    m.add_face(vec![base, base + 1, base + 2, base + 3]);
                    let f = m.faces.len() - 1;
                    let mean = q.stress.iter().sum::<f64>() / 4.0;
                    m.set_color(f, Some(Paint { rgb: ramp(mean / top), texture: None }));
                    // no edges drawn between the squares: a smooth colour map
                    for e in 0..4 {
                        m.seams.push([base + e, base + (e + 1) % 4]);
                    }
                }
                let t = m.face_triangles();
                self.drawn = Some((self.exaggerate, m, t));
            }
            let (_, m, t) = self.drawn.as_ref().unwrap();
            return Some((m, t));
        }
        if self.supports.is_empty() && self.loads.is_empty() {
            return None;
        }
        let mut m = self.mesh.clone();
        for &f in &self.supports {
            m.set_color(f, Some(Paint { rgb: SUPPORT, texture: None }));
        }
        for &f in &self.loads {
            m.set_color(f, Some(Paint { rgb: LOAD, texture: None }));
        }
        let t = m.face_triangles();
        self.painted = Some((m, t));
        let (m, t) = self.painted.as_ref().unwrap();
        Some((m, t))
    }

    pub fn view(&self, model_state: u64) -> StressView {
        let running = self.job.as_ref().map(|j| j.progress.lock().map(|g| g.clone()).unwrap_or((0.0, String::new())));
        let result = self.result.as_ref().map(|(r, seconds, _)| {
            let yield_mpa = Material::presets().get(self.settings.material).map_or(0.0, |m| m.yield_strength / 1e6);
            ResultView {
                max_mpa: r.max_stress / 1e6,
                yield_mpa,
                safety: r.safety_factor,
                max_mm: r.max_displacement * 1000.0,
                mass_kg: r.mass,
                cubes: r.cubes,
                cube_mm: r.cube_size * 1000.0,
                seconds: *seconds,
                converged: r.converged,
                notes: r.notes.clone(),
                scale: self.scale(),
            }
        });
        StressView {
            name: Some(self.name.clone()),
            hold: self.hold,
            supports: self.supports.len(),
            loads: self.loads.len(),
            settings: self.settings,
            thickness_mm: self.thickness_mm,
            longest_mm: longest(&self.part) * 1000.0,
            running,
            result,
            error: self.error.clone(),
            show: self.show,
            exaggerate: self.exaggerate,
            stale: self.result.as_ref().is_some_and(|(_, _, s)| *s != model_state) || model_state != self.state,
        }
    }
}

/// The Stress Test panel.
pub fn panel(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    ui.add_space(6.0);
    panel_head(ui, "Stress Test", UiAction::StressPanel, actions);
    let Some(s) = &v.stress else {
        ui.label("Click a solid group or component to test.");
        ui.label(RichText::new("Then click the faces that hold it and the faces that are pushed, give the force and the material, and Run.").weak().small());
        if v.tool != crate::tools::Tool::Stress && ui.button("Pick a part").clicked() {
            actions.push(UiAction::Tool(crate::tools::Tool::Stress));
        }
        return;
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(s.name.clone().unwrap_or_default()).strong());
        if ui.small_button("Another part").clicked() {
            actions.push(UiAction::StressNew);
        }
    });
    if s.stale {
        ui.label(RichText::new("The model changed: the part is read again at the next click or Run.").weak().small());
    }
    ui.add_space(4.0);
    ui.label("Clicks on the part mark");
    ui.horizontal(|ui| {
        ui.label(RichText::new("■").color(Color32::from_rgb(SUPPORT[0], SUPPORT[1], SUPPORT[2])));
        if ui.selectable_label(s.hold, "Supports").on_hover_text("Faces held in place: bolted, welded or clamped").clicked() {
            actions.push(UiAction::StressPick(true));
        }
        ui.add_space(8.0);
        ui.label(RichText::new("■").color(Color32::from_rgb(LOAD[0], LOAD[1], LOAD[2])));
        if ui.selectable_label(!s.hold, "Loads").on_hover_text("Faces the force pushes on (spread evenly over them)").clicked() {
            actions.push(UiAction::StressPick(false));
        }
    });
    ui.horizontal(|ui| {
        ui.label(format!("{} support face(s), {} load face(s)", s.supports, s.loads));
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(s.supports > 0, egui::Button::new("Clear supports").small()).clicked() {
            actions.push(UiAction::StressClear(true));
        }
        if ui.add_enabled(s.loads > 0, egui::Button::new("Clear loads").small()).clicked() {
            actions.push(UiAction::StressClear(false));
        }
    });
    ui.add_space(6.0);
    let mut set = s.settings;
    let id = egui::Id::new("stress-force");
    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| format!("{} N", s.settings.force));
    ui.horizontal(|ui| {
        ui.label("Force");
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0)).on_hover_text("The total, in N, kN, kg (its weight) or lb");
        if r.changed() {
            if let Some(f) = parse_force(&text) {
                set.force = f;
            }
        }
        if parse_force(&text).is_none() {
            ui.label(RichText::new("N, kN, kg or lb").color(Color32::from_rgb(200, 60, 40)).small());
        }
    });
    ui.data_mut(|d| d.insert_temp(id, text));
    egui::ComboBox::from_label("direction").selected_text(DIRECTIONS[set.direction.min(DIRECTIONS.len() - 1)]).show_ui(ui, |ui| {
        for (i, d) in DIRECTIONS.iter().enumerate() {
            ui.selectable_value(&mut set.direction, i, *d);
        }
    });
    ui.checkbox(&mut set.gravity, "Add the part's own weight");
    let presets = Material::presets();
    egui::ComboBox::from_label("material").selected_text(presets.get(set.material).map_or("", |m| m.name.as_str()).to_string()).show_ui(ui, |ui| {
        for (i, m) in presets.iter().enumerate() {
            ui.selectable_value(&mut set.material, i, m.name.as_str()).on_hover_text(format!(
                "E {:.0} GPa, yield {:.0} MPa, {:.0} kg/m³ (typical values)",
                m.youngs_modulus / 1e9,
                m.yield_strength / 1e6,
                m.density
            ));
        }
    });
    let mut auto = set.divisions == 0;
    if ui.checkbox(&mut auto, "Choose the cube size for me").on_hover_text("The coarsest cubes that fill the part to within 3% of its volume, so thin walls keep their thickness").changed() {
        set.divisions = if auto { 0 } else { 60 };
    }
    if !auto {
        ui.horizontal(|ui| {
            ui.label("Cubes along the longest side");
            ui.add(egui::DragValue::new(&mut set.divisions).range(10..=400));
        });
        let cube = s.longest_mm / set.divisions.max(1) as f64;
        ui.label(RichText::new(match s.thickness_mm {
            Some(t) if cube > t / 2.0 + 1e-9 => format!("Cubes of {cube:.2} mm: fewer than two across the {t} mm sheet; raise the number for a true result"),
            Some(t) => format!("Cubes of {cube:.2} mm ({:.1} across the {t} mm sheet)", t / cube),
            None => format!("Cubes of {cube:.2} mm; walls want two or more across"),
        }).weak().small());
    }
    if set != s.settings {
        actions.push(UiAction::StressSet(set));
    }
    ui.add_space(6.0);
    match &s.running {
        Some((f, what)) => {
            ui.add(egui::ProgressBar::new(*f).text(what.clone()));
        }
        None => {
            if ui.add_enabled(v.editable, egui::Button::new("Run")).clicked() {
                actions.push(UiAction::StressRun);
            }
        }
    }
    if let Some(e) = &s.error {
        ui.label(RichText::new(e).color(Color32::from_rgb(200, 60, 40)));
    }
    if let Some(r) = &s.result {
        ui.add_space(6.0);
        ui.separator();
        let (verdict, color) = if r.safety < 1.0 {
            ("It would bend for good (the stress passes the yield strength)", Color32::from_rgb(210, 40, 30))
        } else if r.safety < 2.0 {
            ("It holds, with little margin", Color32::from_rgb(220, 140, 20))
        } else {
            ("It holds", Color32::from_rgb(40, 150, 60))
        };
        ui.label(RichText::new(verdict).color(color).strong());
        egui::Grid::new("stress-result").num_columns(2).show(ui, |ui| {
            ui.label("Highest stress");
            ui.label(format!("{:.1} MPa (yield {:.0})", r.max_mpa, r.yield_mpa));
            ui.end_row();
            ui.label("Safety factor");
            ui.label(RichText::new(if r.safety.is_finite() { format!("{:.2}", r.safety) } else { "–".into() }).color(color).strong());
            ui.end_row();
            ui.label("Largest deflection");
            ui.label(format!("{:.3} mm", r.max_mm));
            ui.end_row();
            ui.label("Mass");
            ui.label(format!("{:.3} kg", r.mass_kg));
            ui.end_row();
            ui.label("Cubes");
            ui.label(format!("{} of {:.2} mm, {:.1} s", r.cubes, r.cube_mm, r.seconds));
            ui.end_row();
        });
        // the colour key
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(300.0), 12.0), egui::Sense::hover());
        let steps = 40;
        for k in 0..steps {
            let c = ramp(k as f64 / (steps - 1) as f64);
            let x0 = rect.left() + rect.width() * k as f32 / steps as f32;
            let x1 = rect.left() + rect.width() * (k + 1) as f32 / steps as f32;
            ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom())), 0.0, Color32::from_rgb(c[0], c[1], c[2]));
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("0").small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.1} MPa", r.max_mpa)).small());
            });
        });
        let (mut show, mut exaggerate) = (s.show, s.exaggerate);
        ui.checkbox(&mut show, "Show the result on the part");
        ui.add_enabled(show, egui::Checkbox::new(&mut exaggerate, format!("Bend drawn larger (x{:.0})", r.scale)));
        if (show, exaggerate) != (s.show, s.exaggerate) {
            actions.push(UiAction::StressShow(show, exaggerate));
        }
        if !r.converged {
            ui.label(RichText::new("The solver stopped before it settled: the numbers are rough.").color(Color32::from_rgb(220, 140, 20)).small());
        }
        for n in &r.notes {
            ui.label(RichText::new(format!("Note: {n}.")).weak().small());
        }
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new(
            "Linear and static: the material stays springy, deflections are small. Not modelled: buckling, fatigue, bolts, welds, \
             contact. Sharp inside corners and the edges of supports read high. Material values are typical, not a supplier's.",
        )
        .weak()
        .small(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forces_are_typed_with_their_unit() {
        assert_eq!(parse_force("100"), Some(100.0));
        assert_eq!(parse_force("2.5 kN"), Some(2500.0));
        assert!((parse_force("10 kg").unwrap() - 98.0665).abs() < 1e-9);
        assert!((parse_force("20lb").unwrap() - 88.96444).abs() < 1e-4);
        assert_eq!(parse_force("ten"), None);
        assert_eq!(parse_force("5 parsecs"), None);
    }

    #[test]
    fn colours_run_from_blue_to_red() {
        let (lo, hi) = (ramp(0.0), ramp(1.0));
        assert!(lo[2] > 200 && lo[0] < 60);
        assert!(hi[0] > 200 && hi[2] < 60);
        assert_eq!(ramp(-1.0), lo);
        assert_eq!(ramp(2.0), hi);
    }

    #[test]
    fn a_bar_group_tested_from_the_first_click_to_the_result() {
        // a 100 x 10 x 10 mm bar as a group: held at one end, 100 N down on the other
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [0.1, 0.0, 0.0], [0.0, 0.01, 0.0]).unwrap();
        m.push_pull(0, 0.01).unwrap();
        let all: Vec<usize> = (0..m.mesh().face_count()).collect();
        let g = m.make_group(&all, &[], false).unwrap();
        let mut s = Stress::new(&m, g).unwrap();
        let (lo, hi) = daedalus_fea::voxel::bounds(&s.part.triangles);
        let mid = |a: usize| (lo[a] + hi[a]) / 2.0;
        assert!((hi[0] - lo[0] - 0.1).abs() < 1e-9 && (hi[2] - lo[2] - 0.01).abs() < 1e-9);
        assert!(s.pick([lo[0], mid(1), mid(2)]).contains("1 support"));
        s.hold = false;
        assert!(s.pick([hi[0], mid(1), mid(2)]).contains("1 load"));
        assert!(s.display(m.state()).is_some(), "painted while being set up");
        s.settings.divisions = 40;
        s.run(m.state()).unwrap();
        let t = Instant::now();
        while !s.poll() {
            assert!(t.elapsed().as_secs() < 60, "the solver finished");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let v = s.view(m.state());
        assert_eq!(v.error, None);
        let r = v.result.unwrap();
        // PL³/3EI + shear for A36 (E 200 GPa, ν 0.26): about 0.2015 mm
        assert!((r.max_mm - 0.2015).abs() < 0.01, "tip {} mm", r.max_mm);
        assert!(r.safety > 3.0 && r.safety < 5.0, "60 MPa against 250: {}", r.safety);
        assert!(v.show && s.display(m.state()).is_some(), "the result is drawn");
        // into the end face pulls the bar shorter: direction against its outward normal (+x)
        s.settings.direction = 6;
        let d = s.direction();
        assert!((d[0] + 1.0).abs() < 1e-9, "{d:?}");
        // a click away from the part's faces never panics; a second click on a support takes it off
        s.hold = true;
        assert!(s.pick([lo[0], mid(1), mid(2)]).contains("no longer"));
    }

    #[test]
    fn a_sheet_gets_two_cubes_across() {
        // 200 mm long, 2 mm sheet: 1 mm cubes = 200 along it
        assert_eq!(suggested_divisions(0.2, Some(2.0)), 200);
        assert_eq!(suggested_divisions(0.2, None), 0, "chosen by the solver");
        assert_eq!(suggested_divisions(2.0, Some(1.0)), 300, "capped");
    }
}
