//! The window chrome, laid out like SketchUp: menu bar and toolbar on top, a
//! status bar with hints on the left and the Measurements box on the right.
//!
//! On Linux the window draws its own frame: many Wayland compositors (Hyprland among them) give a
//! window no title bar, so there would be nothing to move or resize it by. The menu bar doubles as
//! the title bar, as in VS Code: drag its empty part to move the window, double-click it to
//! maximize, and the window buttons sit at its right end. The edges resize (main.rs, `frame`).

use crate::icons::{self, Icon, Planned};
use crate::render::{FaceStyle, Style};
use crate::tools::{Snap, Tool, AXIS_COLORS};
use egui::{Align, Color32, Layout, Pos2, RichText, Stroke};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiAction {
    Tool(Tool),
    ZoomExtents,
    Top,
    Iso,
    Front,
    Back,
    Right,
    Left,
    /// Camera > Previous: back to the view before the last change.
    Previous,
    ToggleOrtho,
    FaceStyle(FaceStyle),
    BackEdges,
    /// View > Section Cuts on or off; Edit > Delete Section Plane.
    SectionCuts,
    DeleteSection,
    /// View > Section Planes: plane `i` cuts (None: none).
    ActiveSection(Option<usize>),
    DeleteDimensions,
    /// Paint Bucket's colour (None: the default material).
    PaintColor(Option<[u8; 3]>),
    /// Paint Bucket: texture `i`, or a new one from an image file (picked in the file dialog).
    PaintTexture(usize),
    PickTexture,
    /// Entity Info on a textured face: one tile's width and height (metres).
    TextureSize(f64, f64),
    SelectAll,
    DeleteGuides,
    /// Groups and components (Edit menu): make one from the selection, open the selected one, close the open one,
    /// explode it back into loose geometry, give a component copy its own definition.
    MakeGroup,
    MakeComponent,
    /// Erase the selection (Delete; the right-click menu's Erase).
    Erase,
    /// Sandbox on the selection: a terrain from the selected contour lines; selected ground faces split finer.
    FromContours,
    AddDetail,
    /// Window > Entity Info (the right-click menu's Entity Info): the panel on the right, open or shut.
    EntityInfo,
    /// Window > Outliner: the tree of groups and components, open or shut; and a row of it picked (selects that one,
    /// opening the groups it is in).
    Outliner,
    OutlinerPick(usize),
    /// Window > Component Attributes and Component Options: their panels, open or shut.
    ComponentAttributes,
    ComponentOptions,
    /// Entity Info on an arc or circle: its number of segments, or its radius (metres).
    CurveSegments(usize),
    /// Entity Info on a component: glue to faces, cut opening.
    GlueCut(bool, bool),
    CurveRadius(f64),
    EditGroup,
    CloseGroup,
    Explode,
    MakeUnique,
    /// Tools > Reset Axes: the drawing axes back at the origin, along the model's own.
    ResetAxes,
    Undo,
    Redo,
    Inspect,
    Repair,
    MergeCoplanar,
    Units(daedalus_core::units::Units),
    /// View > Toolbar: on the left side (true) or across the top.
    ToolbarLeft(bool),
    New,
    Open,
    Save,
    SaveAs,
    Import,
    Export,
    /// File > Export STEP for Fabrication: exact solids (sheet parts with true bends, closed solids flat-faced), for a shop;
    /// not the same as saving as STEP, which keeps the whole model.
    ExportStep,
    /// Sheet metal: the panel opened or shut, and its tab (0 Drawing, 1 3D, 2 Flat pattern).
    SheetMetal,
    SheetTab(usize),
    /// A part made from the selected face: thickness, K-factor and inside bend radius (mm), and the material preset picked
    /// (`usize::MAX`: typed in).
    MakeSheetPart(f64, f64, f64, usize),
    /// The selected part's rule changed: thickness, K-factor, bend radius (mm), preset (as above).
    SheetRule(f64, f64, f64, usize),
    /// The selected part's reliefs changed: corner relief shape (as SheetView::relief), its size, bend relief width, depth (mm).
    SheetRelief(usize, f64, f64, f64),
    /// The selected part shown unfolded (true) or folded.
    SheetFlat(bool),
    /// Flange `0` of the selected part changed to height (mm), angle (degrees), position (index into FlangePosition::ALL),
    /// bent down.
    SheetFlange(usize, f64, f64, usize, bool),
    SheetRemoveFlange(usize),
    /// What the Flange tool adds: height (mm), angle (degrees), position, bent down, a hem.
    FlangeDefaults(f64, f64, usize, bool, bool),
    /// Fold or jog `0` of the selected part changed to: step (mm, a jog's), angle (degrees), position (into
    /// FoldPosition::ALL, a fold's), bent down.
    SheetFold(usize, f64, f64, usize, bool),
    /// What the Fold tool adds: a jog (else a fold), angle (degrees), a jog's step (mm), a fold's position, bent down.
    FoldDefaults(bool, f64, f64, usize, bool),
    /// Hole `0` of the selected part changed to: centre on its panel (mm), round (else a cut-out), size (mm).
    SheetHole(usize, [f64; 2], bool, [f64; 2]),
    /// What the Hole tool cuts: round (else a rectangle), size (mm: across, or along the bend by across it).
    HoleDefaults(bool, [f64; 2]),
    /// Path flange `0` of the selected part with new legs (length mm, turn degrees each).
    SheetPath(usize, Legs),
    /// The Flange tool makes path flanges (else flanges or hems), with these legs.
    PathDefaults(bool, Legs),
    /// Choose a CSV bend table from the fabricator to use as materials.
    ImportBendTable,
    /// Convert to Sheet Metal: make the part (thickness, K-factor, radius, preset), or start the picks again.
    ConvertSheet(f64, f64, f64, usize),
    ConvertReset,
    /// The selected part's flat pattern as DXF (true: inches), and the part alone as STEP.
    ExportFlatDxf(bool),
    ExportPartStep,
    /// Stress Test: the panel opened or shut; clicks mark supports (true) or loads; clear the supports (true) or loads; the
    /// panel's fields; Run; show the result (and exaggerate the bend); test another part.
    StressPanel,
    StressPick(bool),
    StressClear(bool),
    StressSet(crate::stressui::Settings),
    StressRun,
    StressShow(bool, bool),
    StressNew,
    /// The window frame (Linux): start moving the window, maximize or restore, minimize, close.
    MoveWindow,
    ToggleMaximize,
    Minimize,
    Close,
}

/// A path flange's legs (length mm, turn degrees each) in a list of fixed size, so that a [`UiAction`] stays `Copy`: the panel
/// takes up to [`Legs::MAX`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Legs {
    n: usize,
    v: [[f64; 2]; Legs::MAX],
}

impl Legs {
    pub const MAX: usize = 16;

    pub fn new(legs: &[[f64; 2]]) -> Option<Legs> {
        let mut v = [[0.0; 2]; Legs::MAX];
        v.get_mut(..legs.len())?.copy_from_slice(legs);
        Some(Legs { n: legs.len(), v })
    }

    pub fn get(&self) -> &[[f64; 2]] {
        &self.v[..self.n]
    }
}

/// Everything the chrome shows, gathered by the viewer each frame.
/// The selected component for the Dynamic Components panels: its name, its attributes with each one's value now (or the
/// error its formula gives), and a key that changes with it (so half-typed rows are dropped when another is selected).
#[derive(Debug, Clone, PartialEq)]
pub struct DcView {
    pub name: String,
    pub attrs: Vec<(daedalus_core::dynamic::Attribute, String)>,
    pub key: u64,
}

/// The selected sheet-metal part, for the Sheet Metal panel (lengths in mm).
#[derive(Debug, Clone, PartialEq)]
pub struct SheetView {
    pub name: String,
    pub material: String,
    pub thickness: f64,
    pub k_factor: f64,
    pub bend_radius: f64,
    /// The reliefs: corner relief shape (0 none, 1 round, 2 square, 3 tear), its size, and the bend relief's width and
    /// depth (mm).
    pub relief: (usize, f64, f64, f64),
    pub flat: bool,
    /// Each step, in order: height (a jog's step), angle (degrees), position (into FlangePosition::ALL, or FoldPosition::ALL
    /// for a fold), bent down, and what it is (0 flange, 1 hem, 2 fold, 3 jog, 4 hole, 5 path flange).
    pub flanges: Vec<(f64, f64, usize, bool, usize)>,
    /// Each step's text where numbers do not say it: a hole's size, where it is and on which panel; a path flange's legs.
    pub step_text: Vec<(String, String, String)>,
    /// The base outline and holes (Drawing tab) and the flat pattern's loops and bend centre lines (Flat pattern tab).
    pub base: Vec<Vec<[f64; 2]>>,
    pub flat_loops: Vec<Vec<[f64; 2]>>,
    pub bend_lines: Vec<([f64; 2], [f64; 2])>,
    pub flat_size: Option<(f64, f64)>,
    /// The bend table: angle (degrees), inside radius, length.
    pub bends: Vec<(f64, f64, f64)>,
    /// What a bending shop would likely query (rules of thumb).
    pub checks: Vec<String>,
    /// What a conversion from a solid could not finish as the rule says.
    pub notes: Vec<String>,
    /// Changes with the part (so half-typed values are dropped when it changes).
    pub key: u64,
}

/// What Entity Info shows about the selection.
#[derive(Debug, Clone, PartialEq)]
pub enum Info {
    /// An arc or circle: whether it is an arc, its segments, radius and length (shown in the units), whether they can still
    /// change, and a key that changes with the curve (so a half-typed value is dropped when another is selected).
    Curve { arc: bool, segments: usize, radius: String, length: String, editable: bool, key: u64 },
    Edge { length: String },
    /// A face: its area, its holes, and its texture if it has one (name, a tile's width and height shown in the units,
    /// and a key for the values being typed).
    Face { area: String, holes: usize, texture: Option<(String, String, String, u64)> },
    /// A group or component: its name, and for a component whether it glues to faces and cuts an opening.
    Group { name: String, component: bool, glue: bool, cut: bool },
    /// Several things: how many.
    Many(usize),
}

pub struct UiView<'a> {
    pub tool: Tool,
    pub units: daedalus_core::units::Units,
    pub toolbar_left: bool,
    pub editable: bool,
    pub ortho: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub style: Style,
    pub paint_color: Option<[u8; 3]>,
    /// The texture Paint Bucket paints with (None: a plain colour), and every texture's name.
    pub paint_texture: Option<usize>,
    pub textures: Vec<String>,
    /// Dimension labels: where on screen (points) and the text.
    pub labels: Vec<(Pos2, String)>,
    /// Text (the Text tool): where on screen (points) and the words.
    pub texts: Vec<(Pos2, String)>,
    /// A section plane is placed, and whether it cuts.
    pub section: Option<bool>,
    /// Every section plane: whether it cuts.
    pub sections: Vec<bool>,
    /// Camera > Previous has a view to go back to.
    pub can_go_back: bool,
    /// Something is selected (faces or groups).
    pub has_selection: bool,
    /// The Entity Info panel is open, and what it shows (None: nothing selected).
    pub info_open: bool,
    pub info: Option<Info>,
    /// The Dynamic Components panels: open or shut, the selected component (None: not one component), and where their
    /// changes go (the whole new list of attributes).
    pub attributes_open: bool,
    pub options_open: bool,
    pub dc: Option<DcView>,
    /// What the Dynamic Components panels hand back: the attributes, and whether for the selected copy alone (Component
    /// Options) or for its whole component (Component Attributes).
    pub dc_out: &'a std::cell::RefCell<Option<(Vec<daedalus_core::dynamic::Attribute>, bool)>>,
    /// The Sheet Metal panel: open, its tab, the selected part (if one is), whether one face is selected (a part can be
    /// made of it), and what the Flange tool adds (height mm, angle, position, down, hem).
    pub sheet_open: bool,
    pub sheet_tab: usize,
    pub sheet: Option<SheetView>,
    pub face_for_part: bool,
    pub flange_defaults: (f64, f64, usize, bool, bool),
    /// What the Fold tool adds (a jog, angle, step, position, down), and whether its first point is picked.
    pub fold_defaults: (bool, f64, f64, usize, bool),
    pub fold_first: bool,
    /// What the Hole tool cuts (round, size mm) and whether the Flange tool makes path flanges (and their legs).
    pub hole_defaults: (bool, [f64; 2]),
    pub path_defaults: (bool, Vec<[f64; 2]>),
    /// The materials to pick from: the built-in presets, then the bend table's rows (name, thickness, K-factor, radius), how
    /// many of them come from the bend table, and what the table's file holds that did not read.
    pub materials: Vec<(String, f64, f64, f64)>,
    pub table_rows: usize,
    pub table_note: Vec<String>,
    /// Convert to Sheet Metal under way: the solid's name, whether its fixed face is picked, and how many bend edges.
    pub converting: Option<(String, bool, usize)>,
    /// The Stress Test panel: open, and the test under way (None: no part picked yet).
    pub stress_open: bool,
    pub stress: Option<crate::stressui::StressView>,
    /// The Outliner is open, and its rows: depth, name, component, selected, open for editing.
    pub outliner_open: bool,
    pub outline: Vec<(usize, String, bool, bool, bool)>,
    /// Exactly one group or component is selected: whether it is a component.
    pub one_instance: Option<bool>,
    /// The groups and components open for editing, from the top ("Group#1 › Component#2"), if any.
    pub editing: Option<String>,
    pub status: &'a str,
    /// Last error or note, shown instead of the hint until the next action.
    pub message: Option<&'a str>,
    pub measure_label: &'a str,
    pub measure_value: &'a str,
    /// True while the user is typing into the Measurements box.
    pub typing: bool,
    /// Inference marker in egui points, with what it snapped to.
    pub marker: Option<(Pos2, Snap)>,
    /// The window draws its own title bar and buttons (Linux), and whether it is maximized.
    pub own_frame: bool,
    pub maximized: bool,
    /// The window title, shown in the menu bar's empty part when the window draws its own frame.
    pub title: &'a str,
}

/// The right-click menu at `at` (points), as SketchUp's context menu: what can be done with the selection. Returns the
/// actions picked and whether the menu stays open (it closes on a pick, a click elsewhere or Esc).
/// `fresh`: the frame it opened in, whose clicks (the right-click that opened it) do not close it.
pub fn context_menu(ctx: &egui::Context, at: Pos2, v: &UiView, fresh: bool) -> (Vec<UiAction>, bool) {
    let mut actions = Vec::new();
    let mut open = true;
    let area = egui::Area::new(egui::Id::new("context-menu")).fixed_pos(at).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(170.0);
            // items as in a menu: no button box until the pointer is on one, the full width
            ui.style_mut().visuals.button_frame = false;
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            let mut item = |ui: &mut egui::Ui, enabled: bool, label: &str, shortcut: &str, action: UiAction| {
                if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut)).clicked() {
                    actions.push(action);
                    open = false;
                }
            };
            item(ui, true, "Entity Info", "", UiAction::EntityInfo);
            item(ui, true, "Outliner", "", UiAction::Outliner);
            item(ui, true, "Component Options", "", UiAction::ComponentOptions);
            item(ui, true, "Component Attributes", "", UiAction::ComponentAttributes);
            if v.sheet.is_some() {
                item(ui, true, "Sheet Metal Part", "", UiAction::SheetMetal);
            } else if v.face_for_part {
                item(ui, v.editable, "Make Sheet Metal Part…", "", UiAction::SheetMetal);
            }
            item(ui, v.editable && v.has_selection, "Erase", "Delete", UiAction::Erase);
            ui.separator();
            let kind = if v.one_instance == Some(true) { "Component" } else { "Group" };
            item(ui, v.one_instance.is_some(), &format!("Edit {kind}"), "", UiAction::EditGroup);
            item(ui, v.one_instance.is_some(), "Explode", "", UiAction::Explode);
            item(ui, v.one_instance == Some(true), "Make Unique", "", UiAction::MakeUnique);
            item(ui, v.editable && v.has_selection, "Make Group", "", UiAction::MakeGroup);
            item(ui, v.editable && v.has_selection, "Make Component", "G", UiAction::MakeComponent);
            if v.editing.is_some() {
                item(ui, true, "Close Group/Component", "Esc", UiAction::CloseGroup);
            }
            ui.separator();
            item(ui, true, "Select All", "Ctrl+A", UiAction::SelectAll);
            item(ui, true, "Zoom Extents", "Shift+Z", UiAction::ZoomExtents);
            });
        });
    });
    let clicked_away = !fresh && ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
    if clicked_away || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    (actions, open)
}

/// The Entity Info panel on the right (SketchUp's tray): what the selection is, and for an arc or circle its segments and
/// radius to change (applied on Enter).
fn tray(ctx: &egui::Context, v: &UiView, actions: &mut Vec<UiAction>) {
    // wider while Component Attributes is open: its rows have three fields
    let width = if v.sheet_open || v.stress_open { 360.0 } else if v.attributes_open { 340.0 } else { 230.0 };
    egui::SidePanel::right("tray").resizable(false).exact_width(width).show(ctx, |ui| {
        if v.stress_open {
            egui::ScrollArea::vertical().id_salt("stress").max_height(ui.available_height() * if v.sheet_open || v.info_open || v.outliner_open { 0.6 } else { 1.0 }).show(ui, |ui| {
                crate::stressui::panel(ui, v, actions);
            });
            ui.add_space(10.0);
        }
        if v.sheet_open {
            egui::ScrollArea::vertical().id_salt("sheet").max_height(ui.available_height() * if v.info_open || v.outliner_open { 0.7 } else { 1.0 }).show(ui, |ui| {
                crate::sheetui::panel(ui, v, actions);
            });
            ui.add_space(10.0);
        }
        if v.info_open {
            entity_info(ui, v, actions);
        }
        if v.outliner_open {
            if v.info_open {
                ui.add_space(10.0);
            }
            outliner(ui, v, actions);
        }
        egui::ScrollArea::vertical().id_salt("dc").show(ui, |ui| {
            if v.options_open {
                ui.add_space(10.0);
                component_options(ui, v, actions);
            }
            if v.attributes_open {
                ui.add_space(10.0);
                component_attributes(ui, v, actions);
            }
        });
    });
}

pub(crate) fn panel_head(ui: &mut egui::Ui, title: &str, close: UiAction, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        ui.strong(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("×").on_hover_text("Close").clicked() {
                actions.push(close);
            }
        });
    });
    ui.separator();
}

/// Component Options: the attributes the component offers its user (those with a label), each a field; Enter applies.
fn component_options(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    panel_head(ui, "Component Options", UiAction::ComponentOptions, actions);
    let Some(dc) = &v.dc else {
        ui.label(egui::RichText::new("Select one component").weak());
        return;
    };
    ui.label(&dc.name);
    let offered: Vec<usize> = (0..dc.attrs.len()).filter(|&i| dc.attrs[i].0.label.is_some()).collect();
    if offered.is_empty() {
        ui.label(egui::RichText::new("It offers no options (give an attribute a label in Component Attributes).").weak().small());
        return;
    }
    for i in offered {
        let (a, now) = &dc.attrs[i];
        let id = egui::Id::new(("dc-option", dc.key, i));
        let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| if a.formula.trim_start().starts_with('=') { now.clone() } else { a.formula.clone() });
        let mut done = false;
        ui.horizontal(|ui| {
            ui.label(a.label.as_deref().unwrap_or(&a.name));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.add_enabled(!a.formula.trim_start().starts_with('='), egui::TextEdit::singleline(&mut text).desired_width(100.0));
                done = r.lost_focus() && ui.input(|k| k.key_pressed(egui::Key::Enter));
            });
        });
        if done {
            let mut list: Vec<_> = dc.attrs.iter().map(|x| x.0.clone()).collect();
            list[i].formula = text.clone();
            *v.dc_out.borrow_mut() = Some((list, true));
            ui.data_mut(|d| d.remove::<String>(id));
        } else {
            ui.data_mut(|d| d.insert_temp(id, text));
        }
    }
}

/// Component Attributes: every attribute (name, value or formula, and the label it is offered to the user with), with
/// what it works out to; rows added and taken away, then Apply.
fn component_attributes(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    use daedalus_core::dynamic::Attribute;
    panel_head(ui, "Component Attributes", UiAction::ComponentAttributes, actions);
    let Some(dc) = &v.dc else {
        ui.label(egui::RichText::new("Select one component").weak());
        return;
    };
    ui.label(&dc.name);
    let id = egui::Id::new(("dc-rows", dc.key));
    let mut rows: Vec<(String, String, String)> = ui.data_mut(|d| d.get_temp(id)).unwrap_or_else(|| {
        dc.attrs.iter().map(|(a, _)| (a.name.clone(), a.formula.clone(), a.label.clone().unwrap_or_default())).collect()
    });
    let mut gone = None;
    egui::Grid::new(("dc-grid", dc.key)).num_columns(4).spacing([4.0, 3.0]).show(ui, |ui| {
        ui.label(egui::RichText::new("Name").weak().small());
        ui.label(egui::RichText::new("Value or =formula").weak().small());
        ui.label(egui::RichText::new("Option label").weak().small());
        ui.end_row();
        for (k, (name, formula, label)) in rows.iter_mut().enumerate() {
            ui.add(egui::TextEdit::singleline(name).desired_width(70.0));
            let r = ui.add(egui::TextEdit::singleline(formula).desired_width(130.0));
            if let Some((_, now)) = dc.attrs.get(k).filter(|(a, _)| a.formula.trim_start().starts_with('=')) {
                r.on_hover_text(format!("= {now}"));
            }
            ui.add(egui::TextEdit::singleline(label).desired_width(60.0)).on_hover_text("A label here shows it in Component Options");
            if ui.small_button("×").on_hover_text("Take it away").clicked() {
                gone = Some(k);
            }
            ui.end_row();
        }
    });
    if let Some(k) = gone {
        rows.remove(k);
    }
    ui.horizontal(|ui| {
        if ui.button("+ Attribute").clicked() {
            rows.push((String::new(), String::new(), String::new()));
        }
        ui.menu_button("+ Built-in", |ui| {
            for b in daedalus_core::dynamic::BUILT_INS.iter().chain(daedalus_core::dynamic::DRAWING.iter()).chain(&["onClick"]) {
                if ui.button(*b).clicked() {
                    rows.push((b.to_string(), String::new(), String::new()));
                    ui.close_menu();
                }
            }
        });
        if ui.button("Apply").clicked() {
            let list: Vec<Attribute> = rows
                .iter()
                .filter(|(n, _, _)| !n.trim().is_empty())
                .map(|(n, f, l)| Attribute { name: n.trim().to_string(), formula: f.trim().to_string(), label: Some(l.trim().to_string()).filter(|l| !l.is_empty()) })
                .collect();
            *v.dc_out.borrow_mut() = Some((list, false));
        }
    });
    ui.label(egui::RichText::new("X Y Z place it, LenX LenY LenZ size it, RotZ turns it; onClick: SET(\"Name\", value); ALTER(\"Name\", a, b)").weak().small());
    ui.data_mut(|d| d.insert_temp(id, rows));
}

/// The Outliner: every group and component, nested as they are; a click selects one.
fn outliner(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.strong("Outliner");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("×").on_hover_text("Close").clicked() {
                actions.push(UiAction::Outliner);
            }
        });
    });
    ui.separator();
    if v.outline.is_empty() {
        ui.label(egui::RichText::new("No groups or components").weak());
        return;
    }
    egui::ScrollArea::vertical().id_salt("outliner").max_height(ui.available_height()).show(ui, |ui| {
        for (i, (depth, name, component, selected, open)) in v.outline.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(*depth as f32 * 12.0);
                let mark = if *open { "▾" } else if *component { "◆" } else { "▪" };
                let text = format!("{mark} {name}");
                let label = if *open { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                if ui.selectable_label(*selected, label).clicked() {
                    actions.push(UiAction::OutlinerPick(i));
                }
            });
        }
    });
}

fn entity_info(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.strong("Entity Info");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").on_hover_text("Close").clicked() {
                    actions.push(UiAction::EntityInfo);
                }
            });
        });
        ui.separator();
        let row = |ui: &mut egui::Ui, label: &str, value: &str| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(value));
            });
        };
        match &v.info {
            None => {
                ui.label(egui::RichText::new("Nothing selected").weak());
            }
            Some(Info::Many(n)) => {
                ui.label(format!("{n} entities"));
            }
            Some(Info::Edge { length }) => {
                ui.label("Edge");
                row(ui, "Length", length);
            }
            Some(Info::Face { area, holes, texture }) => {
                ui.label("Face");
                row(ui, "Area", area);
                if *holes > 0 {
                    row(ui, "Holes", &holes.to_string());
                }
                if let Some((name, w, h, key)) = texture {
                    row(ui, "Texture", name);
                    let id = egui::Id::new(("texture-size", *key));
                    let (mut tw, mut th) = ui.data_mut(|d| d.get_temp::<(String, String)>(id)).unwrap_or_else(|| (w.clone(), h.clone()));
                    let mut done = false;
                    for (label, text) in [("Tile width", &mut tw), ("Tile height", &mut th)] {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(label).weak());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let r = ui.add(egui::TextEdit::singleline(text).desired_width(90.0));
                                done |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            });
                        });
                    }
                    if done {
                        match (daedalus_core::units::parse(&tw, v.units), daedalus_core::units::parse(&th, v.units)) {
                            (Some(a), Some(b)) if a > 0.0 && b > 0.0 => actions.push(UiAction::TextureSize(a, b)),
                            _ => (tw, th) = (w.clone(), h.clone()),
                        }
                    }
                    ui.data_mut(|d| d.insert_temp(id, (tw, th)));
                }
            }
            Some(Info::Group { name, component, glue, cut }) => {
                ui.label(if *component { "Component" } else { "Group" });
                row(ui, "Name", name);
                if *component {
                    let (mut g, mut c) = (*glue, *cut);
                    let a = ui.checkbox(&mut g, "Glue to faces").on_hover_text("Moved, it stays on the face it sits on").clicked();
                    let b = ui.checkbox(&mut c, "Cut opening").on_hover_text("Cuts a hole in that face where it touches it, as a window in a wall").clicked();
                    if a || b {
                        // cutting needs gluing; no glue, no cut
                        let (g, c) = if b { (g || c, c) } else { (g, c && g) };
                        actions.push(UiAction::GlueCut(g, c));
                    }
                }
            }
            Some(Info::Curve { arc, segments, radius, length, editable, key }) => {
                ui.label(if *arc { "Arc" } else { "Circle" });
                // the values being typed, kept while the same curve is selected
                let id = egui::Id::new(("curve-edit", *key));
                let (mut segs, mut rad) = ui.data_mut(|d| d.get_temp::<(String, String)>(id)).unwrap_or_else(|| (segments.to_string(), radius.clone()));
                let field = |ui: &mut egui::Ui, label: &str, text: &mut String| -> bool {
                    let mut done = false;
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(label).weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let r = ui.add_enabled(*editable, egui::TextEdit::singleline(text).desired_width(90.0));
                            done = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        });
                    });
                    done
                };
                if field(ui, "Radius", &mut rad) {
                    // in the units shown, unless a unit is written
                    match daedalus_core::units::parse(&rad, v.units) {
                        Some(r) if r > 0.0 => actions.push(UiAction::CurveRadius(r)),
                        _ => rad = radius.clone(),
                    }
                }
                if field(ui, "Segments", &mut segs) {
                    match segs.trim().parse::<usize>() {
                        Ok(n) => actions.push(UiAction::CurveSegments(n)),
                        Err(_) => segs = segments.to_string(),
                    }
                }
                ui.data_mut(|d| d.insert_temp(id, (segs, rad)));
                row(ui, "Length", length);
                if !*editable {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Pushed or pulled into walls: its segments and radius can no longer change.").weak().small());
                }
            }
        }
    }
}

pub fn draw(ctx: &egui::Context, v: &UiView) -> Vec<UiAction> {
    let mut actions = Vec::new();

    egui::TopBottomPanel::top("menu").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                for (label, keys, action) in [
                    ("New", "Ctrl+N", UiAction::New),
                    ("Open…", "Ctrl+O", UiAction::Open),
                    ("Save", "Ctrl+S", UiAction::Save),
                    ("Save As…", "Ctrl+Shift+S", UiAction::SaveAs),
                ] {
                    if ui.add(egui::Button::new(label).shortcut_text(keys)).clicked() {
                        actions.push(action);
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Import…").clicked() {
                    actions.push(UiAction::Import);
                    ui.close_menu();
                }
                if ui.button("Export…").clicked() {
                    actions.push(UiAction::Export);
                    ui.close_menu();
                }
                // not the same as saving as STEP (the whole model, for any CAD program): exact solids for a shop
                let fabrication = "Exact solids for a fabrication shop: sheet-metal parts with true bends, other closed groups and components as solids; open surfaces and loose lines are left out. Save (STEP) keeps the whole model.";
                if ui.button("Export STEP for Fabrication…").on_hover_text(fabrication).clicked() {
                    actions.push(UiAction::ExportStep);
                    ui.close_menu();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(v.can_undo, egui::Button::new("Undo").shortcut_text("Ctrl+Z")).clicked() {
                    actions.push(UiAction::Undo);
                    ui.close_menu();
                }
                if ui.add_enabled(v.can_redo, egui::Button::new("Redo").shortcut_text("Ctrl+Y")).clicked() {
                    actions.push(UiAction::Redo);
                    ui.close_menu();
                }
                ui.separator();
                if ui.add(egui::Button::new("Select All").shortcut_text("Ctrl+A")).clicked() {
                    actions.push(UiAction::SelectAll);
                    ui.close_menu();
                }
                ui.separator();
                if ui.add_enabled(v.editable && v.has_selection, egui::Button::new("Make Group")).clicked() {
                    actions.push(UiAction::MakeGroup);
                    ui.close_menu();
                }
                if ui.add_enabled(v.editable && v.has_selection, egui::Button::new("Make Component").shortcut_text("G")).clicked() {
                    actions.push(UiAction::MakeComponent);
                    ui.close_menu();
                }
                let kind = if v.one_instance == Some(true) { "Component" } else { "Group" };
                if ui.add_enabled(v.one_instance.is_some(), egui::Button::new(format!("Edit {kind}"))).clicked() {
                    actions.push(UiAction::EditGroup);
                    ui.close_menu();
                }
                if ui.add_enabled(v.one_instance.is_some(), egui::Button::new("Explode")).clicked() {
                    actions.push(UiAction::Explode);
                    ui.close_menu();
                }
                if ui.add_enabled(v.one_instance == Some(true), egui::Button::new("Make Unique")).clicked() {
                    actions.push(UiAction::MakeUnique);
                    ui.close_menu();
                }
                if ui.add_enabled(v.editing.is_some(), egui::Button::new("Close Group/Component")).clicked() {
                    actions.push(UiAction::CloseGroup);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Delete Guides").clicked() {
                    actions.push(UiAction::DeleteGuides);
                    ui.close_menu();
                }
                if ui.add_enabled(!v.labels.is_empty(), egui::Button::new("Delete Dimensions")).clicked() {
                    actions.push(UiAction::DeleteDimensions);
                    ui.close_menu();
                }
                if ui.add_enabled(v.section.is_some(), egui::Button::new("Delete Section Plane")).clicked() {
                    actions.push(UiAction::DeleteSection);
                    ui.close_menu();
                }
            });
            ui.menu_button("Camera", |ui| {
                if ui.add_enabled(v.can_go_back, egui::Button::new("Previous")).clicked() {
                    actions.push(UiAction::Previous);
                    ui.close_menu();
                }
                ui.menu_button("Standard Views", |ui| {
                    for (label, action) in [
                        ("Top", UiAction::Top),
                        ("Front", UiAction::Front),
                        ("Right", UiAction::Right),
                        ("Back", UiAction::Back),
                        ("Left", UiAction::Left),
                        ("Iso", UiAction::Iso),
                    ] {
                        if ui.button(label).clicked() {
                            actions.push(action);
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                let mut ortho = v.ortho;
                if ui.checkbox(&mut ortho, "Parallel Projection").clicked() {
                    actions.push(UiAction::ToggleOrtho);
                    ui.close_menu();
                }
                ui.separator();
                tool_items(ui, v, &[Tool::Orbit, Tool::Pan, Tool::Zoom, Tool::ZoomWindow], &mut actions);
                if ui.add(egui::Button::new("Zoom Extents").shortcut_text("Shift+Z")).clicked() {
                    actions.push(UiAction::ZoomExtents);
                    ui.close_menu();
                }
                ui.separator();
                tool_items(ui, v, &[Tool::PositionCamera, Tool::Walk, Tool::LookAround], &mut actions);
            });
            ui.menu_button("View", |ui| {
                let mut info = v.info_open;
                if ui.checkbox(&mut info, "Entity Info").clicked() {
                    actions.push(UiAction::EntityInfo);
                    ui.close_menu();
                }
                let mut outliner = v.outliner_open;
                if ui.checkbox(&mut outliner, "Outliner").clicked() {
                    actions.push(UiAction::Outliner);
                    ui.close_menu();
                }
                let mut options = v.options_open;
                if ui.checkbox(&mut options, "Component Options").clicked() {
                    actions.push(UiAction::ComponentOptions);
                    ui.close_menu();
                }
                let mut attrs = v.attributes_open;
                if ui.checkbox(&mut attrs, "Component Attributes").clicked() {
                    actions.push(UiAction::ComponentAttributes);
                    ui.close_menu();
                }
                ui.separator();
                ui.label(egui::RichText::new("Face Style").weak());
                for f in FaceStyle::ALL {
                    if ui.radio(v.style.faces == f, f.label()).clicked() {
                        actions.push(UiAction::FaceStyle(f));
                        ui.close_menu();
                    }
                }
                let mut back = v.style.back_edges;
                if ui.checkbox(&mut back, "Back Edges").clicked() {
                    actions.push(UiAction::BackEdges);
                    ui.close_menu();
                }
                if v.sections.len() > 1 {
                    ui.menu_button("Section Planes", |ui| {
                        if ui.radio(!v.sections.contains(&true), "None cuts").clicked() {
                            actions.push(UiAction::ActiveSection(None));
                            ui.close_menu();
                        }
                        for (i, &on) in v.sections.iter().enumerate() {
                            if ui.radio(on, format!("Section {}", i + 1)).clicked() {
                                actions.push(UiAction::ActiveSection(Some(i)));
                                ui.close_menu();
                            }
                        }
                    });
                }
                let mut cuts = v.section == Some(true);
                if ui.add_enabled(v.section.is_some(), egui::Checkbox::new(&mut cuts, "Section Cuts")).clicked() {
                    actions.push(UiAction::SectionCuts);
                    ui.close_menu();
                }
                ui.separator();
                ui.label(egui::RichText::new("Toolbar").weak());
                if ui.radio(!v.toolbar_left, "Across the top").clicked() {
                    actions.push(UiAction::ToolbarLeft(false));
                    ui.close_menu();
                }
                if ui.radio(v.toolbar_left, "Down the left side").clicked() {
                    actions.push(UiAction::ToolbarLeft(true));
                    ui.close_menu();
                }
            });
            ui.menu_button("Units", |ui| {
                for u in daedalus_core::units::ALL {
                    if ui.radio(v.units == u, u.name()).clicked() {
                        actions.push(UiAction::Units(u));
                        ui.close_menu();
                    }
                }
                ui.separator();
                ui.label(egui::RichText::new("Type any unit whatever is shown: 300mm, 1.2m, 4' 6\"").weak());
            });
            ui.menu_button("Draw", |ui| {
                tool_items(ui, v, &[Tool::Line, Tool::Freehand], &mut actions);
                ui.separator();
                tool_items(ui, v, &[Tool::Arc, Tool::TwoPointArc, Tool::ThreePointArc], &mut actions);
                ui.separator();
                tool_items(ui, v, &[Tool::Rectangle, Tool::RotatedRectangle, Tool::Circle, Tool::Polygon, Tool::Pie], &mut actions);
                ui.menu_button("Sandbox", |ui| {
                    if ui.add_enabled(v.editable && v.has_selection, egui::Button::new("From Contours")).clicked() {
                        actions.push(UiAction::FromContours);
                        ui.close_menu();
                    }
                    tool_items(ui, v, &[Tool::SandboxScratch], &mut actions);
                });
            });
            ui.menu_button("Tools", |ui| {
                tool_items(ui, v, &[Tool::Select, Tool::Eraser], &mut actions);
                ui.separator();
                tool_items(ui, v, &[Tool::Move, Tool::Rotate, Tool::Scale, Tool::PushPull, Tool::Offset, Tool::FollowMe], &mut actions);
                ui.separator();
                tool_items(ui, v, &[Tool::TapeMeasure, Tool::Protractor, Tool::Dimensions, Tool::Text, Tool::Axes, Tool::Text3D], &mut actions);
                ui.menu_button("Sandbox", |ui| {
                    tool_items(ui, v, &[Tool::Smoove, Tool::Stamp, Tool::Drape, Tool::FlipEdge], &mut actions);
                    if ui.add_enabled(v.editable && v.has_selection, egui::Button::new("Add Detail")).clicked() {
                        actions.push(UiAction::AddDetail);
                        ui.close_menu();
                    }
                });
                tool_items(ui, v, &[Tool::Interact], &mut actions);
                ui.menu_button("Sheet Metal", |ui| {
                    if ui.add_enabled(v.editable && v.face_for_part, egui::Button::new("Make Sheet Metal Part…")).on_hover_text("Select one face first").clicked() {
                        actions.push(UiAction::SheetMetal);
                        ui.close_menu();
                    }
                    tool_items(ui, v, &[Tool::Flange, Tool::Fold, Tool::Hole, Tool::SheetConvert], &mut actions);
                    if ui.button("Sheet Metal Panel").clicked() {
                        actions.push(UiAction::SheetMetal);
                        ui.close_menu();
                    }
                });
                tool_items(ui, v, &[Tool::Stress], &mut actions);
                ui.menu_button("Solid Tools", |ui| {
                    tool_items(ui, v, &[Tool::OuterShell, Tool::SolidIntersect, Tool::SolidUnion, Tool::SolidSubtract, Tool::SolidTrim, Tool::SolidSplit], &mut actions);
                });
                if ui.button("Reset Axes").clicked() {
                    actions.push(UiAction::ResetAxes);
                    ui.close_menu();
                }
                ui.separator();
                tool_items(ui, v, &[Tool::SectionPlane], &mut actions);
                ui.separator();
                if ui.add(egui::Button::new("Solid Inspector").shortcut_text("I")).clicked() {
                    actions.push(UiAction::Inspect);
                    ui.close_menu();
                }
                if ui.add_enabled(v.editable, egui::Button::new("Merge Coplanar Faces")).clicked() {
                    actions.push(UiAction::MergeCoplanar);
                    ui.close_menu();
                }
                if ui.button("Repair Solid").clicked() {
                    actions.push(UiAction::Repair);
                    ui.close_menu();
                }
            });
            if v.own_frame {
                title_bar(ui, v, &mut actions);
            }
        });
    });

    // the toolbar: icons in groups, across the top or down the left side (View > Toolbar)
    if v.toolbar_left {
        // SketchUp's Large Tool Set: two columns, in its order; views and styles below
        egui::SidePanel::left("toolbar").resizable(false).exact_width(2.0 * ICON + 22.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
                for (gi, group) in toolbar_groups().iter().enumerate() {
                    if gi > 0 {
                        ui.add_space(2.0);
                        ui.separator();
                        ui.add_space(2.0);
                    }
                    for pair in group.chunks(2) {
                        ui.horizontal(|ui| {
                            for &slot in pair {
                                toolbar_slot(ui, v, slot, &mut actions);
                            }
                        });
                    }
                }
            });
        });
    } else {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (gi, group) in toolbar_groups().iter().enumerate() {
                    if gi > 0 {
                        ui.add_space(3.0);
                        ui.separator();
                        ui.add_space(3.0);
                    }
                    for &slot in group.iter() {
                        if !matches!(slot, Slot::Empty) {
                            toolbar_slot(ui, v, slot, &mut actions);
                        }
                    }
                }
            });
        });
    }

    egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if let Some(path) = &v.editing {
                ui.label(RichText::new(format!("Editing {path}")).strong());
                ui.separator();
            }
            match v.message {
                Some(m) => ui.label(RichText::new(m).color(Color32::from_rgb(180, 40, 30))),
                None => ui.label(v.status),
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let text = if v.typing { format!("{}|", v.measure_value) } else { v.measure_value.to_string() };
                egui::Frame::new()
                    .stroke(Stroke::new(1.0_f32, Color32::GRAY))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.set_min_width(140.0);
                        ui.label(RichText::new(text).monospace());
                    });
                ui.label(v.measure_label);
            });
        });
    });

    // Paint Bucket's colours, while it is the tool
    if v.tool == Tool::PaintBucket {
        egui::Window::new("Colours")
            .anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0])
            .resizable(false)
            .collapsible(true)
            .show(ctx, |ui| {
                const SWATCHES: [[u8; 3]; 16] = [
                    [196, 92, 64], [214, 150, 60], [230, 200, 90], [120, 170, 80],
                    [60, 130, 90], [70, 150, 170], [70, 110, 190], [120, 90, 170],
                    [170, 90, 130], [140, 100, 70], [180, 150, 120], [230, 225, 215],
                    [160, 160, 160], [100, 100, 104], [60, 60, 64], [250, 250, 250],
                ];
                egui::Grid::new("swatches").spacing([4.0, 4.0]).show(ui, |ui| {
                    for (i, c) in SWATCHES.iter().enumerate() {
                        let (rect, r) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                        ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(c[0], c[1], c[2]));
                        if v.paint_color == Some(*c) {
                            ui.painter().rect_stroke(rect, 4.0, Stroke::new(2.0_f32, ui.visuals().strong_text_color()), egui::StrokeKind::Outside);
                        }
                        if r.clicked() {
                            actions.push(UiAction::PaintColor(Some(*c)));
                        }
                        if i % 4 == 3 {
                            ui.end_row();
                        }
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let mut rgb = v.paint_color.unwrap_or([200, 200, 205]);
                    if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                        actions.push(UiAction::PaintColor(Some(rgb)));
                    }
                    ui.label("Any colour");
                });
                if ui.radio(v.paint_color.is_none() && v.paint_texture.is_none(), "Default (takes paint off)").clicked() {
                    actions.push(UiAction::PaintColor(None));
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Textures").weak());
                for (i, name) in v.textures.iter().enumerate() {
                    if ui.radio(v.paint_texture == Some(i), name).clicked() {
                        actions.push(UiAction::PaintTexture(i));
                    }
                }
                if ui.button("Texture from file…").clicked() {
                    actions.push(UiAction::PickTexture);
                }
            });
    }

    // dimension labels, on a small plate so they read over the model
    if !v.labels.is_empty() {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("dimension-labels")));
        for (pos, text) in &v.labels {
            let galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(13.0), Color32::from_gray(30));
            let r = egui::Rect::from_center_size(*pos, galley.size() + egui::vec2(8.0, 4.0));
            painter.rect_filled(r, 3.0, Color32::from_rgba_unmultiplied(255, 255, 255, 220));
            painter.galley(r.min + egui::vec2(4.0, 2.0), galley, Color32::from_gray(30));
        }
    }

    // text, on a plate with a thin border (several lines allowed)
    if !v.texts.is_empty() {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("text-labels")));
        for (pos, text) in &v.texts {
            let galley = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(13.0), Color32::from_gray(25));
            let r = egui::Rect::from_center_size(*pos, galley.size() + egui::vec2(10.0, 6.0));
            painter.rect_filled(r, 2.0, Color32::from_rgba_unmultiplied(255, 255, 255, 235));
            painter.rect_stroke(r, 2.0, Stroke::new(1.0_f32, Color32::from_gray(120)), egui::StrokeKind::Inside);
            painter.galley(r.min + egui::vec2(5.0, 3.0), galley, Color32::from_gray(25));
        }
    }

    if let Some((pos, snap)) = v.marker {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("inference")));
        let color = match snap {
            Snap::Endpoint => Color32::from_rgb(20, 170, 20),
            Snap::Midpoint => Color32::from_rgb(0, 170, 200),
            Snap::OnFace => Color32::from_rgb(40, 60, 220),
            Snap::OnAxis(a) => {
                let c = AXIS_COLORS[a as usize];
                Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
            }
            Snap::InPlane | Snap::Ground => Color32::DARK_GRAY,
            Snap::OnGuide => Color32::from_rgb(90, 90, 100),
            Snap::Origin => Color32::from_rgb(200, 120, 20),
        };
        match snap {
            Snap::Endpoint | Snap::Midpoint | Snap::Origin => {
                painter.circle(pos, 5.0, Color32::TRANSPARENT, Stroke::new(2.0_f32, color));
            }
            _ => {
                painter.circle_filled(pos, 3.0, color);
            }
        }
        if !matches!(snap, Snap::InPlane | Snap::Ground) {
            painter.text(
                pos + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                snap.label(),
                egui::FontId::proportional(13.0),
                color,
            );
        }
    }
    if v.info_open || v.outliner_open || v.options_open || v.attributes_open || v.sheet_open || v.stress_open {
        tray(ctx, v, &mut actions);
    }
    actions
}

const BUTTON: f32 = 13.0; // diameter of a window button
const BUTTON_GAP: f32 = 8.0;

/// The rest of the menu bar: the window's title, a drag area (move; double-click maximizes) and
/// the window buttons, drawn as on the rest of the Yggdrasil desktop: round, in the Windows order
/// (minimize, maximize, close), the symbol showing while the pointer is over the group.
fn title_bar(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    let controls = 3.0 * BUTTON + 2.0 * BUTTON_GAP + 14.0;
    let height = ui.available_height().max(22.0);
    let (drag, resp) = ui.allocate_exact_size(egui::vec2((ui.available_width() - controls).max(0.0), height), egui::Sense::click_and_drag());
    if resp.drag_started_by(egui::PointerButton::Primary) {
        actions.push(UiAction::MoveWindow);
    }
    if resp.double_clicked() {
        actions.push(UiAction::ToggleMaximize);
    }
    let font = egui::FontId::proportional(13.0);
    let ink = ui.visuals().weak_text_color();
    let galley = ui.painter().layout_no_wrap(v.title.to_string(), font, ink);
    if galley.size().x + 24.0 < drag.width() {
        ui.painter().galley(egui::pos2(drag.center().x - galley.size().x / 2.0, drag.center().y - galley.size().y / 2.0), galley, ink);
    }

    let (area, _) = ui.allocate_exact_size(egui::vec2(controls, height), egui::Sense::hover());
    let hover = ui.rect_contains_pointer(area);
    let buttons = [
        (Color32::from_rgb(0xfe, 0xbc, 0x2e), "Minimize", UiAction::Minimize),
        (Color32::from_rgb(0x28, 0xc8, 0x40), if v.maximized { "Restore" } else { "Maximize" }, UiAction::ToggleMaximize),
        (Color32::from_rgb(0xff, 0x5f, 0x57), "Close", UiAction::Close),
    ];
    for (i, (colour, tip, action)) in buttons.into_iter().enumerate() {
        let c = egui::pos2(area.left() + 7.0 + BUTTON / 2.0 + i as f32 * (BUTTON + BUTTON_GAP), area.center().y);
        let rect = egui::Rect::from_center_size(c, egui::vec2(BUTTON, BUTTON));
        let r = ui.interact(rect.expand(3.0), ui.id().with(("window-button", i)), egui::Sense::click()).on_hover_text(tip);
        let p = ui.painter();
        p.circle_filled(c, BUTTON / 2.0, colour);
        p.circle_stroke(c, BUTTON / 2.0, Stroke::new(0.5_f32, Color32::from_black_alpha(60)));
        if hover {
            let ink = Stroke::new(1.4_f32, Color32::from_black_alpha(150));
            let d = BUTTON * 0.24;
            match i {
                0 => {
                    p.line_segment([c - egui::vec2(d, 0.0), c + egui::vec2(d, 0.0)], ink);
                }
                1 => {
                    p.line_segment([c - egui::vec2(d, 0.0), c + egui::vec2(d, 0.0)], ink);
                    p.line_segment([c - egui::vec2(0.0, d), c + egui::vec2(0.0, d)], ink);
                }
                _ => {
                    p.line_segment([c - egui::vec2(d, d), c + egui::vec2(d, d)], ink);
                    p.line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], ink);
                }
            }
        }
        if r.clicked() {
            actions.push(action);
        }
    }
}

/// The size of a toolbar icon, in points.
const ICON: f32 = 30.0;

/// One icon button: the picture, a highlight when it is the tool in use or under the pointer, its name on hover.
fn icon_button(ui: &mut egui::Ui, icon: Icon, selected: bool, enabled: bool) -> bool {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ICON, ICON), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let visuals = ui.visuals();
    if selected {
        ui.painter().rect_filled(rect, 5.0, visuals.selection.bg_fill.gamma_multiply(0.55));
        ui.painter().rect_stroke(rect, 5.0, Stroke::new(1.0_f32, visuals.selection.stroke.color), egui::StrokeKind::Inside);
    } else if r.hovered() && enabled {
        ui.painter().rect_filled(rect, 5.0, visuals.widgets.hovered.weak_bg_fill);
    }
    // line icons in the text colour: dark on the light style, light on the dark one
    let ink = visuals.strong_text_color();
    icons::paint(ui.painter(), rect.shrink(4.0), icon, !enabled, ink);
    let r = r.on_hover_text(icons::name(icon));
    enabled && r.clicked()
}

/// One place in the toolbar.
#[derive(Debug, Clone, Copy)]
enum Slot {
    Tool(Tool),
    Act(Icon, UiAction),
    Empty,
}

/// The toolbar, in groups. The first group is SketchUp's Large Tool Set in its order (pairs of two, left then right
/// column), with tools still to come greyed out in their place; then the camera views and the face styles.
fn toolbar_groups() -> [Vec<Slot>; 4] {
    use Slot::{Act, Empty};
    let t = Slot::Tool;
    [
        vec![
            t(Tool::Select),
            Act(Icon::Planned(Planned::MakeComponent), UiAction::MakeComponent),
            t(Tool::PaintBucket),
            t(Tool::Eraser),
            t(Tool::Line),
            t(Tool::Freehand),
            t(Tool::Rectangle),
            t(Tool::RotatedRectangle),
            t(Tool::Circle),
            t(Tool::Polygon),
            t(Tool::Arc),
            t(Tool::TwoPointArc),
            t(Tool::ThreePointArc),
            t(Tool::Pie),
            t(Tool::Move),
            t(Tool::PushPull),
            t(Tool::Rotate),
            t(Tool::FollowMe),
            t(Tool::Scale),
            t(Tool::Offset),
            t(Tool::TapeMeasure),
            t(Tool::Dimensions),
            t(Tool::Protractor),
            t(Tool::Text),
            t(Tool::Axes),
            t(Tool::Text3D),
            t(Tool::Orbit),
            t(Tool::Pan),
            t(Tool::Zoom),
            t(Tool::ZoomWindow),
            Act(Icon::ZoomExtents, UiAction::ZoomExtents),
            Act(Icon::Previous, UiAction::Previous),
            t(Tool::PositionCamera),
            t(Tool::Walk),
            t(Tool::LookAround),
            t(Tool::SectionPlane),
        ],
        vec![
            t(Tool::OuterShell),
            t(Tool::SolidIntersect),
            t(Tool::SolidUnion),
            t(Tool::SolidSubtract),
            t(Tool::SolidTrim),
            t(Tool::SolidSplit),
            Act(Icon::FromContours, UiAction::FromContours),
            t(Tool::SandboxScratch),
            t(Tool::Smoove),
            t(Tool::Stamp),
            t(Tool::Drape),
            Act(Icon::AddDetail, UiAction::AddDetail),
            t(Tool::FlipEdge),
            t(Tool::Interact),
            t(Tool::Flange),
            t(Tool::Fold),
            t(Tool::Hole),
            t(Tool::SheetConvert),
            t(Tool::Stress),
        ],
        vec![
            Act(Icon::Iso, UiAction::Iso),
            Act(Icon::Top, UiAction::Top),
            Act(Icon::Front, UiAction::Front),
            Act(Icon::Right, UiAction::Right),
            Act(Icon::Back, UiAction::Back),
            Act(Icon::Left, UiAction::Left),
            Act(Icon::Parallel, UiAction::ToggleOrtho),
            Empty,
        ],
        vec![
            Act(Icon::Style(FaceStyle::XRay), UiAction::FaceStyle(FaceStyle::XRay)),
            Act(Icon::BackEdges, UiAction::BackEdges),
            Act(Icon::Style(FaceStyle::Wireframe), UiAction::FaceStyle(FaceStyle::Wireframe)),
            Act(Icon::Style(FaceStyle::HiddenLine), UiAction::FaceStyle(FaceStyle::HiddenLine)),
            Act(Icon::Style(FaceStyle::Shaded), UiAction::FaceStyle(FaceStyle::Shaded)),
            Act(Icon::Style(FaceStyle::Textured), UiAction::FaceStyle(FaceStyle::Textured)),
            Act(Icon::Style(FaceStyle::Monochrome), UiAction::FaceStyle(FaceStyle::Monochrome)),
        ],
    ]
}

fn toolbar_slot(ui: &mut egui::Ui, v: &UiView, slot: Slot, actions: &mut Vec<UiAction>) {
    match slot {
        Slot::Tool(tool) => {
            if icon_button(ui, Icon::Tool(tool), v.tool == tool, v.editable || !tool.edits()) {
                actions.push(UiAction::Tool(tool));
            }
        }
        Slot::Act(icon, action) => {
            let selected = match action {
                UiAction::ToggleOrtho => v.ortho,
                UiAction::FaceStyle(f) => v.style.faces == f,
                UiAction::BackEdges => v.style.back_edges,
                _ => false,
            };
            let enabled = match action {
                UiAction::Previous => v.can_go_back,
                UiAction::MakeComponent => v.editable && v.has_selection,
                _ => true,
            };
            if icon_button(ui, icon, selected, enabled) {
                actions.push(action);
            }
        }
        Slot::Empty => {
            ui.allocate_exact_size(egui::vec2(ICON, ICON), egui::Sense::hover());
        }
    }
}

fn tool_items(ui: &mut egui::Ui, v: &UiView, tools: &[Tool], actions: &mut Vec<UiAction>) {
    for &tool in tools {
        let mut button = egui::Button::new(tool.label()).selected(v.tool == tool);
        if let Some(key) = tool.shortcut() {
            button = button.shortcut_text(key);
        }
        if ui.add_enabled(v.editable || !tool.edits(), button).clicked() {
            actions.push(UiAction::Tool(tool));
            ui.close_menu();
        }
    }
}
