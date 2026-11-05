//! The window chrome, laid out like SketchUp: menu bar and toolbar on top, a
//! status bar with hints on the left and the Measurements box on the right.

use crate::tools::{Snap, Tool, AXIS_COLORS};
use egui::{Align, Color32, Layout, Pos2, RichText, Stroke};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    Tool(Tool),
    ZoomExtents,
    Top,
    Iso,
    ToggleOrtho,
    Undo,
    Redo,
    Inspect,
    Repair,
    MergeCoplanar,
    New,
    Open,
    Save,
    SaveAs,
    Import,
    Export,
}

/// Everything the chrome shows, gathered by the viewer each frame.
pub struct UiView<'a> {
    pub tool: Tool,
    pub editable: bool,
    pub ortho: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub status: &'a str,
    /// Last error or note, shown instead of the hint until the next action.
    pub message: Option<&'a str>,
    pub measure_label: &'a str,
    pub measure_value: &'a str,
    /// True while the user is typing into the Measurements box.
    pub typing: bool,
    /// Inference marker in egui points, with what it snapped to.
    pub marker: Option<(Pos2, Snap)>,
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
            });
            ui.menu_button("Camera", |ui| {
                for (label, action) in [("Top", UiAction::Top), ("Iso", UiAction::Iso)] {
                    if ui.button(label).clicked() {
                        actions.push(action);
                        ui.close_menu();
                    }
                }
                ui.separator();
                let mut ortho = v.ortho;
                if ui.checkbox(&mut ortho, "Parallel Projection").clicked() {
                    actions.push(UiAction::ToggleOrtho);
                    ui.close_menu();
                }
                if ui.add(egui::Button::new("Zoom Extents").shortcut_text("Shift+Z")).clicked() {
                    actions.push(UiAction::ZoomExtents);
                    ui.close_menu();
                }
            });
            ui.menu_button("Draw", |ui| tool_items(ui, v, &[Tool::Line, Tool::Rectangle, Tool::Circle, Tool::Polygon], &mut actions));
            ui.menu_button("Tools", |ui| {
                tool_items(ui, v, &[Tool::Select, Tool::Eraser, Tool::PushPull], &mut actions);
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
        });
    });

    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            for group in [
                &[Tool::Select, Tool::Eraser][..],
                &[Tool::Line, Tool::Rectangle, Tool::Circle, Tool::Polygon],
                &[Tool::PushPull],
                &[Tool::Orbit, Tool::Pan, Tool::Zoom],
            ] {
                for &tool in group {
                    let enabled = v.editable || !tool.edits();
                    let button = egui::SelectableLabel::new(v.tool == tool, tool.label());
                    let r = ui.add_enabled(enabled, button);
                    let r = match tool.shortcut() {
                        Some(key) => r.on_hover_text(format!("{} ({key})", tool.label())),
                        None => r,
                    };
                    if r.clicked() {
                        actions.push(UiAction::Tool(tool));
                    }
                }
                ui.separator();
            }
            if ui.button("Zoom Extents").on_hover_text("Shift+Z").clicked() {
                actions.push(UiAction::ZoomExtents);
            }
            if ui.button("Top").clicked() {
                actions.push(UiAction::Top);
            }
            if ui.button("Iso").clicked() {
                actions.push(UiAction::Iso);
            }
            if ui.selectable_label(v.ortho, "Parallel").on_hover_text("Parallel projection").clicked() {
                actions.push(UiAction::ToggleOrtho);
            }
        });
    });

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
        };
        match snap {
            Snap::Endpoint | Snap::Midpoint => {
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
