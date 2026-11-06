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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    SelectAll,
    DeleteGuides,
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
    /// The window frame (Linux): start moving the window, maximize or restore, minimize, close.
    MoveWindow,
    ToggleMaximize,
    Minimize,
    Close,
}

/// Everything the chrome shows, gathered by the viewer each frame.
pub struct UiView<'a> {
    pub tool: Tool,
    pub units: daedalus_core::units::Units,
    pub toolbar_left: bool,
    pub editable: bool,
    pub ortho: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub style: Style,
    /// Camera > Previous has a view to go back to.
    pub can_go_back: bool,
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
                if ui.button("Delete Guides").clicked() {
                    actions.push(UiAction::DeleteGuides);
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
            });
            ui.menu_button("Tools", |ui| {
                tool_items(ui, v, &[Tool::Select, Tool::Eraser], &mut actions);
                ui.separator();
                tool_items(ui, v, &[Tool::Move, Tool::Rotate, Tool::Scale, Tool::PushPull], &mut actions);
                ui.separator();
                tool_items(ui, v, &[Tool::TapeMeasure, Tool::Protractor], &mut actions);
                ui.separator();
                if ui.add(egui::Button::new("Solid Inspector").shortcut_text("I")).clicked() {
                    actions.push(UiAction::Inspect);
                    ui.close_menu();
                }
                if ui.add_enabled(v.editable, egui::Button::new("Merge Coplanar Faces")).clicked() {
                    actions.push(UiAction::MergeCoplanar);
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Repair Solid").shortcut_text("F")).clicked() {
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
    Planned(Planned),
    Empty,
}

/// The toolbar, in groups. The first group is SketchUp's Large Tool Set in its order (pairs of two, left then right
/// column), with tools still to come greyed out in their place; then the camera views and the face styles.
fn toolbar_groups() -> [Vec<Slot>; 3] {
    use Slot::{Act, Empty, Planned as P};
    let t = Slot::Tool;
    [
        vec![
            t(Tool::Select),
            P(Planned::MakeComponent),
            P(Planned::PaintBucket),
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
            P(Planned::FollowMe),
            t(Tool::Scale),
            P(Planned::Offset),
            t(Tool::TapeMeasure),
            P(Planned::Dimensions),
            t(Tool::Protractor),
            P(Planned::Text),
            P(Planned::Axes),
            P(Planned::Text3D),
            t(Tool::Orbit),
            t(Tool::Pan),
            t(Tool::Zoom),
            t(Tool::ZoomWindow),
            Act(Icon::ZoomExtents, UiAction::ZoomExtents),
            Act(Icon::Previous, UiAction::Previous),
            t(Tool::PositionCamera),
            t(Tool::Walk),
            t(Tool::LookAround),
            P(Planned::SectionPlane),
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
            Empty,
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
            let enabled = action != UiAction::Previous || v.can_go_back;
            if icon_button(ui, icon, selected, enabled) {
                actions.push(action);
            }
        }
        Slot::Planned(p) => {
            icon_button(ui, Icon::Planned(p), false, false);
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
