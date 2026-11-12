//! The Sheet Metal panel (in the tray on the right): making a part of a face, and the part's editor in three tabs, as
//! CADCraft lays it out: Drawing (the outline it was made from, its material and its flanges), 3D (the part bent or flat in
//! the model, and the Flange tool) and Flat pattern (the blank with its bend lines and bend table, for the cutting shop).
//! Lengths are in mm, as fabricators work; a typed length may carry its own unit (`0.125"`).

use crate::tools::Tool;
use crate::ui::{panel_head, Legs, SheetView, UiAction, UiView};
use daedalus_core::units::{self, Units};
use daedalus_sheet::part::{parse_legs, FlangePosition, FoldPosition};
use egui::{pos2, vec2, Color32, Rect, RichText, Sense, Shape, Stroke};

const TABS: [&str; 3] = ["Drawing", "3D", "Flat pattern"];

pub fn panel(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    ui.add_space(6.0);
    panel_head(ui, "Sheet Metal", UiAction::SheetMetal, actions);
    match (&v.sheet, &v.converting) {
        (Some(s), _) => part(ui, v, s, actions),
        (None, Some(c)) => convert_form(ui, v, c, actions),
        (None, None) => make_form(ui, v, actions),
    }
}

/// Convert to Sheet Metal under way: what is picked, the material, Convert.
fn convert_form(ui: &mut egui::Ui, v: &UiView, (name, fixed, bends): &(String, bool, usize), actions: &mut Vec<UiAction>) {
    ui.label(format!("Convert {name} to sheet metal"));
    ui.label(RichText::new(if *fixed { "✔ the face that stays put" } else { "Click the face that stays put" }).weak());
    ui.label(
        RichText::new(match bends {
            0 => "Click each edge that becomes a bend".to_string(),
            1 => "✔ 1 bend edge (click more, or Convert)".to_string(),
            n => format!("✔ {n} bend edges (click more, or Convert)"),
        })
        .weak(),
    );
    ui.label(
        RichText::new("A solid with thin walls keeps their thickness; any other gets the thickness below inside its outer faces.")
            .weak()
            .small(),
    );
    ui.add_space(4.0);
    let first = &v.materials[0];
    let (values, preset, _) = material_fields(ui, v, egui::Id::new("sheet-convert"), (0, first.1, first.2, first.3));
    ui.horizontal(|ui| {
        let ready = v.editable && *fixed && *bends > 0 && values.is_some();
        if ui.add_enabled(ready, egui::Button::new("Convert")).clicked() {
            if let Some((t, k, r)) = values {
                actions.push(UiAction::ConvertSheet(t, k, r, preset));
            }
        }
        if ui.button("Start over").clicked() {
            actions.push(UiAction::ConvertReset);
        }
    });
}

/// A length typed in mm (or with its own unit), as mm.
fn mm(text: &str) -> Option<f64> {
    units::parse(text, Units::Millimetres).map(|m| m * 1000.0).filter(|x| x.is_finite())
}

fn num(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|x| x.is_finite())
}

fn fmt(x: f64) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// A row: a label on the left, a text field on the right. Returns true when Enter was pressed in it.
fn field(ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str) -> bool {
    let mut enter = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(110.0)).on_hover_text(hint);
            enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        });
    });
    enter
}

/// The material picker and the three values it fills in (kept per `id` while being typed). Returns the values when they
/// read, and the preset (`usize::MAX`: typed in).
fn material_fields(ui: &mut egui::Ui, v: &UiView, id: egui::Id, start: (usize, f64, f64, f64)) -> (Option<(f64, f64, f64)>, usize, bool) {
    let (mut preset, mut t, mut k, mut r) = ui
        .data_mut(|d| d.get_temp::<(usize, String, String, String)>(id))
        .unwrap_or_else(|| (start.0, fmt(start.1), fmt(start.2), fmt(start.3)));
    let shown = v.materials.get(preset).map(|m| m.0.as_str()).unwrap_or("Typed in");
    let first_table = v.materials.len() - v.table_rows;
    egui::ComboBox::from_id_salt((id, "material")).selected_text(shown).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
        for (i, m) in v.materials.iter().enumerate() {
            if i == first_table {
                ui.separator();
                ui.label(RichText::new("From my bend table").weak().small());
            }
            if ui.selectable_label(preset == i, &m.0).clicked() {
                preset = i;
                (t, k, r) = (fmt(m.1), fmt(m.2), fmt(m.3));
            }
        }
        if first_table == v.materials.len() {
            ui.separator();
        }
        if ui.selectable_label(preset == usize::MAX, "Typed in").clicked() {
            preset = usize::MAX;
        }
    });
    let (t0, k0, r0) = (t.clone(), k.clone(), r.clone());
    let mut enter = field(ui, "Thickness", &mut t, "Sheet thickness, mm (or with a unit: 0.125\")");
    enter |= field(ui, "K-factor", &mut k, "Where the neutral line sits in the bend, 0 to 1: use your fabricator's value");
    enter |= field(ui, "Bend radius", &mut r, "Inside bend radius, mm: use your fabricator's value");
    if (&t, &k, &r) != (&t0, &k0, &r0) {
        preset = usize::MAX; // a value typed over the preset's
    }
    let values = match (mm(&t), num(&k), mm(&r)) {
        (Some(t), Some(k), Some(r)) if t > 0.0 && (0.0..=1.0).contains(&k) && r >= 0.0 => Some((t, k, r)),
        _ => None,
    };
    ui.data_mut(|d| d.insert_temp(id, (preset, t, k, r)));
    (values, preset, enter)
}

/// No part selected: make one of the selected face.
fn make_form(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    if !v.face_for_part {
        ui.label(RichText::new("Select a sheet-metal part to edit it, or one face to make a part of it.").weak());
        ui.label(
            RichText::new("The face's outline and holes become the flat part; it grows out of the face's front by the thickness.")
                .weak()
                .small(),
        );
        ui.add_space(4.0);
        if ui
            .add_enabled(v.editable, egui::Button::new("Convert a solid…").selected(v.tool == Tool::SheetConvert))
            .on_hover_text("A solid group or component you modelled (say, with 2 mm walls) into a part with bends")
            .clicked()
        {
            actions.push(UiAction::Tool(Tool::SheetConvert));
        }
        return;
    }
    ui.label("Make a sheet-metal part of the selected face");
    ui.add_space(4.0);
    let first = &v.materials[0];
    let (values, preset, enter) = material_fields(ui, v, egui::Id::new("sheet-make"), (0, first.1, first.2, first.3));
    ui.add_space(4.0);
    ui.label(RichText::new("Presets are typical values only: check your fabricator's K-factor and bend radius.").weak().small());
    let clicked = ui.add_enabled(v.editable && values.is_some(), egui::Button::new("Make Part")).clicked();
    if let Some((t, k, r)) = values.filter(|_| clicked || enter) {
        actions.push(UiAction::MakeSheetPart(t, k, r, preset));
    }
}

fn part(ui: &mut egui::Ui, v: &UiView, s: &SheetView, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        ui.strong(&s.name);
        ui.label(RichText::new(&s.material).weak());
    });
    ui.horizontal(|ui| {
        for (i, name) in TABS.iter().enumerate() {
            if ui.selectable_label(v.sheet_tab == i, *name).clicked() {
                actions.push(UiAction::SheetTab(i));
            }
        }
    });
    ui.separator();
    match v.sheet_tab {
        0 => drawing_tab(ui, v, s, actions),
        1 => model_tab(ui, v, s, actions),
        _ => flat_tab(ui, s, actions),
    }
}

/// Drawing: the outline the part was made from (with its size), its material, and its flanges.
fn drawing_tab(ui: &mut egui::Ui, v: &UiView, s: &SheetView, actions: &mut Vec<UiAction>) {
    drawing(ui, &s.base, &[], 200.0);
    for n in &s.notes {
        ui.label(RichText::new(format!("Note: {n}")).weak().small());
    }
    ui.add_space(6.0);
    ui.strong("Material");
    let preset = v.materials.iter().position(|m| m.0 == s.material).unwrap_or(usize::MAX);
    let (values, preset, enter) =
        material_fields(ui, v, egui::Id::new(("sheet-rule", s.key)), (preset, s.thickness, s.k_factor, s.bend_radius));
    let clicked = ui
        .add_enabled(v.editable && values.is_some(), egui::Button::new("Apply"))
        .on_hover_text("The part is made again with these values; its flanges stay")
        .clicked();
    if let Some((t, k, r)) = values.filter(|_| clicked || enter) {
        actions.push(UiAction::SheetRule(t, k, r, preset));
    }
    bend_table(ui, v, actions);
    ui.add_space(6.0);
    reliefs(ui, v, s, actions);
    ui.add_space(6.0);
    ui.strong("Flanges, folds, jogs and holes");
    if s.flanges.is_empty() {
        ui.label(RichText::new("None yet: the Flange, Fold and Hole tools (3D tab) add them.").weak());
        return;
    }
    egui::Grid::new(("sheet-flanges", s.key)).num_columns(6).spacing([4.0, 3.0]).show(ui, |ui| {
        ui.label(RichText::new("#").weak().small());
        ui.label(RichText::new("Height").weak().small());
        ui.label(RichText::new("Angle").weak().small());
        ui.label(RichText::new("Position").weak().small());
        ui.label(RichText::new("Bent").weak().small());
        ui.end_row();
        for (i, &(h, a, pos, down, kind)) in s.flanges.iter().enumerate() {
            let (hem, fold, jog) = (kind == 1, kind == 2, kind == 3);
            if kind == 4 || kind == 5 {
                let text = s.step_text.get(i).cloned().unwrap_or_default();
                text_row(ui, v, s.key, i, kind == 4, &text, actions);
                continue;
            }
            let id = egui::Id::new(("sheet-flange", s.key, i));
            let (mut th, mut ta, mut tp, mut td) =
                ui.data_mut(|d| d.get_temp::<(String, String, usize, bool)>(id)).unwrap_or_else(|| (fmt(h), fmt(a), pos, down));
            ui.label(format!("{}", i + 1));
            let mut changed = false;
            let r1 = if fold {
                ui.label(RichText::new("fold").weak());
                None
            } else {
                let hint = if hem {
                    "Hem length to its outer face, mm"
                } else if jog {
                    "The jog's step, face to face, mm"
                } else {
                    "Height from the outside of the bend, mm"
                };
                Some(ui.add(egui::TextEdit::singleline(&mut th).desired_width(56.0)).on_hover_text(hint))
            };
            let mut r2 = None;
            if hem {
                ui.label(RichText::new("hem").weak());
                ui.label("");
            } else {
                r2 =
                    Some(ui.add(egui::TextEdit::singleline(&mut ta).desired_width(40.0)).on_hover_text("Bend angle, degrees (90: square)"));
                if jog {
                    ui.label(RichText::new("jog").weak());
                } else if fold {
                    egui::ComboBox::from_id_salt((id, "pos")).selected_text(if tp == 1 { "Mould" } else { "Centre" }).width(64.0).show_ui(
                        ui,
                        |ui| {
                            for (k, p) in FoldPosition::ALL.iter().enumerate() {
                                if ui.selectable_label(tp == k, p.label()).clicked() {
                                    tp = k;
                                    changed = true;
                                }
                            }
                        },
                    );
                } else {
                    egui::ComboBox::from_id_salt((id, "pos")).selected_text(position_short(tp)).width(64.0).show_ui(ui, |ui| {
                        for (k, p) in FlangePosition::ALL.iter().enumerate() {
                            if ui.selectable_label(tp == k, p.label()).clicked() {
                                tp = k;
                                changed = true;
                            }
                        }
                    });
                }
            }
            if ui.selectable_label(td, if td { "down" } else { "up" }).on_hover_text("Click to bend it the other way").clicked() {
                td = !td;
                changed = true;
            }
            if ui.small_button("×").on_hover_text("Take it away").clicked() {
                actions.push(UiAction::SheetRemoveFlange(i));
            }
            let enter = (r1.as_ref().is_some_and(|r| r.lost_focus()) || r2.as_ref().is_some_and(|r| r.lost_focus()))
                && ui.input(|k| k.key_pressed(egui::Key::Enter));
            if enter || changed {
                // a hem keeps its 180 degrees
                let a = if hem { Some(180.0) } else { num(&ta) };
                if fold || jog {
                    if let (Some(step), Some(a)) = (if jog { mm(&th) } else { Some(0.0) }, a) {
                        actions.push(UiAction::SheetFold(i, step, a, tp, td));
                    }
                } else if let (Some(h), Some(a)) = (mm(&th), a) {
                    actions.push(UiAction::SheetFlange(i, h, a, tp, td));
                }
            }
            ui.data_mut(|d| d.insert_temp(id, (th, ta, tp, td)));
            ui.end_row();
        }
    });
    ui.label(RichText::new("Enter applies a change.").weak().small());
}

/// The reliefs: what is cut where two flanges meet at a corner, and where a bend ends short of an edge.
fn reliefs(ui: &mut egui::Ui, v: &UiView, s: &SheetView, actions: &mut Vec<UiAction>) {
    const SHAPES: [(&str, &str); 4] = [
        ("None", "Nothing cut: the bends end square at the corner"),
        ("Round", "A round cut about the corner of the bends"),
        ("Square", "A square cut on the corner of the bends (what cutting shops such as SendCutSend ask for)"),
        ("Tear", "Both bends run to the corner; each end is cut straight from there"),
    ];
    ui.strong("Reliefs");
    let id = egui::Id::new(("sheet-relief", s.key));
    let (shape0, c0, w0, d0) = s.relief;
    let (mut shape, mut tc, mut tw, mut td) =
        ui.data_mut(|d| d.get_temp::<(usize, String, String, String)>(id)).unwrap_or_else(|| (shape0, fmt(c0), fmt(w0), fmt(d0)));
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Corners").weak());
        for (k, (name, hint)) in SHAPES.iter().enumerate() {
            if ui.selectable_label(shape == k, *name).on_hover_text(*hint).clicked() {
                shape = k;
                changed = true;
            }
        }
    });
    changed |= field(ui, "Corner size", &mut tc, "How far across the corner relief is, mm (Enter sets it)");
    changed |= field(ui, "Bend relief width", &mut tw, "Width of the cut where a bend stops short of an edge, mm (Enter sets it)");
    changed |= field(ui, "Bend relief depth", &mut td, "How far that cut runs past the bend, mm (Enter sets it)");
    if let (Some(c), Some(w), Some(d)) = (mm(&tc), mm(&tw), mm(&td)) {
        if changed && v.editable && (shape, c, w, d) != s.relief && c >= 0.0 && w >= 0.0 && d >= 0.0 {
            actions.push(UiAction::SheetRelief(shape, c, w, d));
        }
    }
    ui.data_mut(|d| d.insert_temp(id, (shape, tc, tw, td)));
}

fn position_short(i: usize) -> &'static str {
    match FlangePosition::ALL.get(i) {
        Some(FlangePosition::Inside) => "Inside",
        Some(FlangePosition::Outside) => "Outside",
        _ => "Middle",
    }
}

/// 3D: the part in the model bent or flat, the Flange tool and what it adds, STEP of the part.
fn model_tab(ui: &mut egui::Ui, v: &UiView, s: &SheetView, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Show it").weak());
        if ui.selectable_label(!s.flat, "Bent").clicked() && s.flat {
            actions.push(UiAction::SheetFlat(false));
        }
        if ui.selectable_label(s.flat, "Flat").clicked() && !s.flat {
            actions.push(UiAction::SheetFlat(true));
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(v.editable, egui::Button::new("Flange tool").selected(v.tool == Tool::Flange)).clicked() {
            actions.push(UiAction::Tool(Tool::Flange));
        }
        ui.label(RichText::new("then click an edge of the part").weak());
    });
    ui.label(RichText::new("New flanges:").weak());
    let id = egui::Id::new("flange-defaults");
    let (h0, a0, p0, d0, hem0) = v.flange_defaults;
    let (mut th, mut ta, mut tp, mut td, mut thm) =
        ui.data_mut(|d| d.get_temp::<(String, String, usize, bool, bool)>(id)).unwrap_or_else(|| (fmt(h0), fmt(a0), p0, d0, hem0));
    let mut changed = false;
    let path_id = egui::Id::new("path-defaults");
    let (on0, legs0) = v.path_defaults.clone();
    let (mut on, mut tl) =
        ui.data_mut(|d| d.get_temp::<(bool, String)>(path_id)).unwrap_or_else(|| (on0, daedalus_sheet::part::legs_text(&legs0)));
    let mut path_changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Kind").weak());
        for (hem, path, name, hint) in [
            (false, false, "Flange", "Bent up to the angle"),
            (true, false, "Hem", "Folded right back onto the sheet"),
            (false, true, "Path", "Several flanges, one on the next: a lip, a channel, a Z"),
        ] {
            if ui.selectable_label(thm == hem && on == path, name).on_hover_text(hint).clicked() {
                (thm, on) = (hem, path);
                changed = true;
                path_changed = true;
            }
        }
        if !on {
            ui.label(RichText::new("  Bent").weak());
            for (down, name) in [(false, "Up"), (true, "Down")] {
                if ui.selectable_label(td == down, name).on_hover_text("Up: toward the face the part was made from").clicked() {
                    td = down;
                    changed = true;
                }
            }
        }
    });
    if on {
        path_changed |= field(ui, "Legs", &mut tl, "Each leg's length (mm, from the outside of its bend) and turn (degrees), legs apart by commas: 20 90, 12 90 is a lip turned back over the part; 12 -90 turns it out. The first turn is from the sheet (positive: toward its top face), each after it from the leg before (Enter sets it)");
        let legs = parse_legs(&tl).and_then(|l| Legs::new(&l));
        if legs.is_none() {
            ui.label(RichText::new(format!("Type each leg as a length and a turn, at most {}: 20 90, 12 90", Legs::MAX)).weak().small());
        }
        if let Some(legs) = legs.filter(|l| path_changed && (true, l.get()) != (on0, &legs0[..])) {
            actions.push(UiAction::PathDefaults(true, legs));
        }
        ui.data_mut(|d| d.insert_temp(path_id, (on, tl)));
        ui.data_mut(|d| d.insert_temp(id, (th, ta, tp, td, thm)));
        ui.add_space(8.0);
        fold_section(ui, v, actions);
        ui.add_space(8.0);
        hole_section(ui, v, actions);
        ui.add_space(8.0);
        step_button(ui, actions);
        return;
    }
    if let Some(legs) = Legs::new(&legs0).filter(|_| path_changed && on0) {
        actions.push(UiAction::PathDefaults(false, legs));
    }
    ui.data_mut(|d| d.insert_temp(path_id, (on, tl)));
    changed |= field(ui, if thm { "Length" } else { "Height" }, &mut th, "From the outside of the bend, mm (Enter sets it)");
    if !thm {
        changed |= field(ui, "Angle", &mut ta, "Bend angle, degrees, 90 is square (Enter sets it)");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Position").weak());
            for (k, p) in FlangePosition::ALL.iter().enumerate() {
                if ui.selectable_label(tp == k, position_short(k)).on_hover_text(p.label()).clicked() {
                    tp = k;
                    changed = true;
                }
            }
        });
    }
    if let (Some(h), Some(a)) = (mm(&th), num(&ta)) {
        if changed && (h, a, tp, td, thm) != (h0, a0, p0, d0, hem0) && h > 0.0 && a > 0.0 && a <= 180.0 {
            actions.push(UiAction::FlangeDefaults(h, a, tp, td, thm));
        }
    }
    ui.data_mut(|d| d.insert_temp(id, (th, ta, tp, td, thm)));
    ui.add_space(8.0);
    fold_section(ui, v, actions);
    ui.add_space(8.0);
    hole_section(ui, v, actions);
    ui.add_space(8.0);
    step_button(ui, actions);
}

fn step_button(ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
    if ui
        .button("Export part as STEP…")
        .on_hover_text("The bent part as an exact solid, for the fabricator or another CAD program")
        .clicked()
    {
        actions.push(UiAction::ExportPartStep);
    }
}

/// The Hole tool and what it cuts: a round hole, or a rectangle lined up with the bend of the panel it is on.
fn hole_section(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        if ui.add_enabled(v.editable, egui::Button::new("Hole tool").selected(v.tool == Tool::Hole)).clicked() {
            actions.push(UiAction::Tool(Tool::Hole));
        }
        ui.label(RichText::new("then click a flat face, bent or not").weak());
    });
    let id = egui::Id::new("hole-defaults");
    let (r0, s0) = v.hole_defaults;
    let (mut round, mut tw, mut th) =
        ui.data_mut(|d| d.get_temp::<(bool, String, String)>(id)).unwrap_or_else(|| (r0, fmt(s0[0]), fmt(s0[1])));
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Shape").weak());
        for (r, name, hint) in
            [(true, "Round", "A round hole"), (false, "Rectangle", "A rectangular cut-out, its sides along and across the panel's bend")]
        {
            if ui.selectable_label(round == r, name).on_hover_text(hint).clicked() {
                round = r;
                changed = true;
            }
        }
    });
    if round {
        changed |= field(ui, "Diameter", &mut tw, "Hole diameter, mm (Enter sets it)");
    } else {
        changed |= field(ui, "Along the bend", &mut tw, "The cut-out's width along the panel's bend, mm (Enter sets it)");
        changed |= field(ui, "Across", &mut th, "The cut-out's height across the panel, mm (Enter sets it)");
    }
    if let (Some(w), Some(h)) = (mm(&tw), mm(&th)) {
        let size = [w, if round { w } else { h }];
        if changed && (round, size) != (r0, s0) && w > 0.0 && size[1] > 0.0 {
            actions.push(UiAction::HoleDefaults(round, size));
        }
    }
    ui.data_mut(|d| d.insert_temp(id, (round, tw, th)));
}

/// The bend table: how many materials it adds, a way to choose the fabricator's file, and what in it did not read.
fn bend_table(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        let text = match v.table_rows {
            0 => "No bend table yet".to_string(),
            1 => "1 material from my bend table".to_string(),
            n => format!("{n} materials from my bend table"),
        };
        ui.label(RichText::new(text).weak().small());
        if ui
            .small_button("Bend table…")
            .on_hover_text("Choose your fabricator's bend table: a CSV with one row per material, thickness, inside radius and K-factor (or BD for the 90 degree bend deduction, or BA for its allowance). Lengths in mm, or with a unit: 0.060\". Its rows join the materials above; the file is kept in ~/.config/daedalus/bend-table.csv.")
            .clicked()
        {
            actions.push(UiAction::ImportBendTable);
        }
    });
    for n in v.table_note.iter().take(3) {
        ui.label(RichText::new(format!("Bend table {n}")).weak().small());
    }
}

/// A hole's or a path flange's row in the Drawing tab's list: its size (or legs) and, for a hole, where it sits on which
/// panel; Enter applies, × takes it away.
fn text_row(
    ui: &mut egui::Ui,
    v: &UiView,
    key: u64,
    i: usize,
    hole: bool,
    (first, second, on): &(String, String, String),
    actions: &mut Vec<UiAction>,
) {
    let id = egui::Id::new(("sheet-step-text", key, i));
    let (mut ta, mut tb) = ui.data_mut(|d| d.get_temp::<(String, String)>(id)).unwrap_or_else(|| (first.clone(), second.clone()));
    ui.label(format!("{}", i + 1));
    let hint = if hole {
        "Size, mm: a diameter (6) or along the bend x across (10 x 5)"
    } else {
        "Legs: length (mm) and turn (degrees) each, apart by commas"
    };
    let r1 = ui.add(egui::TextEdit::singleline(&mut ta).desired_width(if hole { 56.0 } else { 140.0 })).on_hover_text(hint);
    let mut r2 = None;
    if hole {
        r2 = Some(ui.add(egui::TextEdit::singleline(&mut tb).desired_width(64.0)).on_hover_text(format!(
            "Its centre on {on}, mm: along the bend, then across the panel from where the bend ends (on the base: the flat's own x, y)"
        )));
        ui.label(RichText::new(format!("hole, {on}")).weak().small());
    } else {
        ui.label(RichText::new("path").weak());
        ui.label("");
    }
    if ui.add_enabled(v.editable, egui::Button::new("×").small()).on_hover_text("Take it away").clicked() {
        actions.push(UiAction::SheetRemoveFlange(i));
    }
    let enter = (r1.lost_focus() || r2.as_ref().is_some_and(|r| r.lost_focus())) && ui.input(|k| k.key_pressed(egui::Key::Enter));
    if enter && v.editable {
        if hole {
            if let (Some((round, size)), Some(at)) = (hole_size(&ta), pair(&tb)) {
                actions.push(UiAction::SheetHole(i, at, round, size));
            }
        } else if let Some(legs) = parse_legs(&ta).and_then(|l| Legs::new(&l)) {
            actions.push(UiAction::SheetPath(i, legs));
        }
    }
    ui.data_mut(|d| d.insert_temp(id, (ta, tb)));
    ui.end_row();
}

/// A hole's size as typed: one length (round) or two apart by `x` (a cut-out).
fn hole_size(text: &str) -> Option<(bool, [f64; 2])> {
    let parts: Vec<&str> = text.split(['x', 'X', '×']).map(str::trim).collect();
    match parts[..] {
        [d] => mm(d).filter(|d| *d > 0.0).map(|d| (true, [d, d])),
        [w, h] => match (mm(w), mm(h)) {
            (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((false, [w, h])),
            _ => None,
        },
        _ => None,
    }
}

/// Two lengths apart by a comma.
fn pair(text: &str) -> Option<[f64; 2]> {
    let (a, b) = text.split_once(',')?;
    Some([mm(a.trim())?, mm(b.trim())?])
}

/// The Fold tool and what it adds: a fold (one bend along the line) or a jog (two, the far side stepped aside and parallel).
fn fold_section(ui: &mut egui::Ui, v: &UiView, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        if ui.add_enabled(v.editable, egui::Button::new("Fold tool").selected(v.tool == Tool::Fold)).clicked() {
            actions.push(UiAction::Tool(Tool::Fold));
        }
        ui.label(
            RichText::new(if v.tool == Tool::Fold && v.fold_first {
                "now the line's second point"
            } else {
                "then two points on a face: the line"
            })
            .weak(),
        );
    });
    let id = egui::Id::new("fold-defaults");
    let (j0, a0, s0, p0, d0) = v.fold_defaults;
    let (mut tj, mut ta, mut ts, mut tp, mut td) =
        ui.data_mut(|d| d.get_temp::<(bool, String, String, usize, bool)>(id)).unwrap_or_else(|| (j0, fmt(a0), fmt(s0), p0, d0));
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Kind").weak());
        for (jog, name, hint) in [
            (false, "Fold", "One bend along the line"),
            (true, "Jog", "Two bends the opposite way: the far side steps aside and stays parallel"),
        ] {
            if ui.selectable_label(tj == jog, name).on_hover_text(hint).clicked() {
                tj = jog;
                changed = true;
            }
        }
        ui.label(RichText::new("  Bent").weak());
        for (down, name) in [(false, "Up"), (true, "Down")] {
            if ui.selectable_label(td == down, name).on_hover_text("Up: toward the face the part was made from").clicked() {
                td = down;
                changed = true;
            }
        }
    });
    changed |= field(ui, "Angle", &mut ta, "Bend angle, degrees, 90 is square (Enter sets it)");
    if tj {
        changed |= field(ui, "Step", &mut ts, "How far the far side moves, face to face, mm (Enter sets it)");
    } else {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Line").weak());
            for (k, p) in FoldPosition::ALL.iter().enumerate() {
                if ui.selectable_label(tp == k, if k == 0 { "Centre" } else { "Mould" }).on_hover_text(p.label()).clicked() {
                    tp = k;
                    changed = true;
                }
            }
        });
    }
    if let (Some(a), Some(st)) = (num(&ta), mm(&ts)) {
        if changed && (tj, a, st, tp, td) != (j0, a0, s0, p0, d0) && a > 0.0 && a <= 180.0 && st > 0.0 {
            actions.push(UiAction::FoldDefaults(tj, a, st, tp, td));
        }
    }
    ui.data_mut(|d| d.insert_temp(id, (tj, ta, ts, tp, td)));
}

/// Flat pattern: the blank, its bend lines and bend table, and DXF for the cutting shop.
fn flat_tab(ui: &mut egui::Ui, s: &SheetView, actions: &mut Vec<UiAction>) {
    drawing(ui, &s.flat_loops, &s.bend_lines, 220.0);
    if let Some((w, h)) = s.flat_size {
        ui.label(format!("Blank {} × {} mm, {} mm thick", fmt(w), fmt(h), fmt(s.thickness)));
    }
    if !s.bends.is_empty() {
        ui.add_space(4.0);
        egui::Grid::new(("bend-table", s.key)).num_columns(5).spacing([10.0, 2.0]).striped(true).show(ui, |ui| {
            for head in ["Bend", "Angle", "Included", "Radius", "Length"] {
                ui.label(RichText::new(head).weak().small());
            }
            ui.end_row();
            for (i, &(a, r, l)) in s.bends.iter().enumerate() {
                ui.label(format!("{}", i + 1));
                ui.label(format!("{}°", fmt(a)));
                ui.label(format!("{}°", fmt(180.0 - a)));
                ui.label(fmt(r));
                ui.label(fmt(l));
                ui.end_row();
            }
        });
    }
    ui.add_space(6.0);
    ui.strong("Shop checks");
    if s.checks.is_empty() {
        ui.label(RichText::new("Nothing a shop would usually query.").weak());
    } else {
        for c in &s.checks {
            ui.label(RichText::new(format!("⚠ {c}")).color(Color32::from_rgb(214, 140, 30)));
        }
    }
    ui.label(RichText::new("Rules of thumb only: your shop's own limits decide.").weak().small());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("DXF (mm)…").on_hover_text("Outline and holes on layer CUT, bend centre lines on BEND, 1:1").clicked() {
            actions.push(UiAction::ExportFlatDxf(false));
        }
        if ui.button("DXF (inches)…").clicked() {
            actions.push(UiAction::ExportFlatDxf(true));
        }
    });
    ui.label(RichText::new("Check with your shop whether bend lines go in the cut file or are left out.").weak().small());
}

/// A 2D drawing fitted to the panel's width: `loops` in ink, `bends` as dashed red lines, the overall size along the bottom
/// and the right side (mm, y up).
fn drawing(ui: &mut egui::Ui, loops: &[Vec<[f64; 2]>], bends: &[([f64; 2], [f64; 2])], height: f32) {
    /// Room kept right of the drawing for its height (points).
    const RIGHT: f64 = 52.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    let pts = loops.iter().flatten().chain(bends.iter().flat_map(|(a, b)| [a, b]));
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if !(hi[0] > lo[0] && hi[1] > lo[1]) {
        return;
    }
    let inner = rect.shrink2(vec2(18.0, 18.0));
    let scale = ((inner.width() as f64 - RIGHT) / (hi[0] - lo[0])).min((inner.height() as f64 - 16.0) / (hi[1] - lo[1]));
    let (w, h) = ((hi[0] - lo[0]) * scale, (hi[1] - lo[1]) * scale);
    let origin = pos2(
        inner.left() + ((inner.width() as f64 - RIGHT - w) / 2.0) as f32,
        inner.top() + ((inner.height() as f64 - 16.0 - h) / 2.0) as f32,
    );
    let at = |p: &[f64; 2]| pos2(origin.x + ((p[0] - lo[0]) * scale) as f32, origin.y + ((hi[1] - p[1]) * scale) as f32);
    let ink = visuals.text_color();
    for lp in loops {
        let mut pts: Vec<_> = lp.iter().map(at).collect();
        if let Some(&first) = pts.first() {
            pts.push(first);
        }
        painter.add(Shape::line(pts, Stroke::new(1.3_f32, ink)));
    }
    let red = Color32::from_rgb(220, 60, 50);
    for (a, b) in bends {
        painter.extend(Shape::dashed_line(&[at(a), at(b)], Stroke::new(1.0_f32, red), 6.0, 4.0));
    }
    // overall size
    let font = egui::FontId::proportional(11.0);
    let dim = visuals.weak_text_color();
    let box_ = Rect::from_min_size(origin, vec2(w as f32, h as f32));
    let y = box_.bottom() + 8.0;
    painter.line_segment([pos2(box_.left(), y), pos2(box_.right(), y)], Stroke::new(0.8_f32, dim));
    painter.text(pos2(box_.center().x, y + 1.0), egui::Align2::CENTER_TOP, format!("{} mm", fmt(hi[0] - lo[0])), font.clone(), dim);
    let x = box_.right() + 8.0;
    painter.line_segment([pos2(x, box_.top()), pos2(x, box_.bottom())], Stroke::new(0.8_f32, dim));
    painter.text(pos2(x + 2.0, box_.center().y), egui::Align2::LEFT_CENTER, fmt(hi[1] - lo[1]), font, dim);
}
