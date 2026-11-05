//! The toolbar's icons, drawn as vector shapes in SketchUp's manner: a coloured picture of what the tool does (a red box with a
//! push arrow, a pencil, a pink eraser, a magnifier), dark outlines, so they read at a glance and stay sharp at any size. No
//! image files to ship.

use crate::tools::Tool;
use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Tool(Tool),
    ZoomExtents,
    Top,
    Iso,
    Parallel,
}

const INK: Color32 = Color32::from_rgb(40, 40, 46);
const RED: Color32 = Color32::from_rgb(214, 70, 52);
const GREEN: Color32 = Color32::from_rgb(52, 160, 72);
const BLUE: Color32 = Color32::from_rgb(56, 110, 214);
const SKY: Color32 = Color32::from_rgb(150, 190, 240);
const SAND: Color32 = Color32::from_rgb(232, 210, 160);
const PINK: Color32 = Color32::from_rgb(236, 140, 160);
const WOOD: Color32 = Color32::from_rgb(232, 180, 90);
const GREY: Color32 = Color32::from_rgb(196, 198, 204);

/// Draws `icon` into `rect` (a square). `dim` greys it out (a tool that cannot be used now).
pub fn paint(p: &Painter, rect: Rect, icon: Icon, dim: bool) {
    let w = rect.width();
    let at = |x: f32, y: f32| pos2(rect.left() + x * w, rect.top() + y * w);
    let fade = |c: Color32| if dim { c.gamma_multiply(0.35) } else { c };
    let ink = Stroke::new((w / 16.0).max(1.2), fade(INK));
    let thin = Stroke::new((w / 24.0).max(1.0), fade(INK));
    let poly = |pts: Vec<Pos2>, fill: Color32, s: Stroke| p.add(Shape::convex_polygon(pts, fade(fill), s));
    let line = |pts: Vec<Pos2>, s: Stroke| p.add(Shape::line(pts, s));
    // a small isometric box: top, left and right faces
    let cube = |top: Color32, left: Color32, right: Color32, s: Stroke| {
        poly(vec![at(0.5, 0.18), at(0.86, 0.36), at(0.5, 0.54), at(0.14, 0.36)], top, s);
        poly(vec![at(0.14, 0.36), at(0.5, 0.54), at(0.5, 0.9), at(0.14, 0.72)], left, s);
        poly(vec![at(0.5, 0.54), at(0.86, 0.36), at(0.86, 0.72), at(0.5, 0.9)], right, s);
    };
    match icon {
        Icon::Tool(Tool::Select) => {
            // the arrow pointer
            poly(vec![at(0.28, 0.12), at(0.28, 0.78), at(0.44, 0.63), at(0.28, 0.12)], Color32::WHITE, Stroke::NONE);
            p.add(Shape::closed_line(vec![at(0.28, 0.12), at(0.28, 0.78), at(0.43, 0.64), at(0.55, 0.9), at(0.65, 0.85), at(0.53, 0.6), at(0.74, 0.6)], ink));
        }
        Icon::Tool(Tool::Eraser) => {
            // a tilted pink eraser with a white sleeve
            poly(vec![at(0.18, 0.62), at(0.5, 0.3), at(0.7, 0.5), at(0.38, 0.82)], PINK, ink);
            poly(vec![at(0.5, 0.3), at(0.66, 0.14), at(0.86, 0.34), at(0.7, 0.5)], Color32::WHITE, ink);
            line(vec![at(0.12, 0.9), at(0.62, 0.9)], thin);
        }
        Icon::Tool(Tool::Line) => {
            // a pencil drawing a line
            line(vec![at(0.1, 0.9), at(0.42, 0.9)], Stroke::new(w / 14.0, fade(BLUE)));
            poly(vec![at(0.2, 0.8), at(0.32, 0.84), at(0.84, 0.32), at(0.72, 0.2)], WOOD, ink);
            poly(vec![at(0.2, 0.8), at(0.14, 0.9), at(0.32, 0.84)], INK, Stroke::NONE);
            poly(vec![at(0.72, 0.2), at(0.84, 0.32), at(0.9, 0.26), at(0.78, 0.14)], PINK, ink);
        }
        Icon::Tool(Tool::Rectangle) => {
            p.rect(Rect::from_min_max(at(0.12, 0.26), at(0.88, 0.78)), 0.0, fade(SKY), ink, egui::StrokeKind::Middle);
            p.circle_filled(at(0.12, 0.26), w / 14.0, fade(RED));
            p.circle_filled(at(0.88, 0.78), w / 14.0, fade(RED));
        }
        Icon::Tool(Tool::Circle) => {
            p.circle(at(0.5, 0.52), w * 0.36, fade(SKY), ink);
            line(vec![at(0.5, 0.52), at(0.86, 0.52)], thin);
            p.circle_filled(at(0.5, 0.52), w / 16.0, fade(RED));
        }
        Icon::Tool(Tool::Polygon) => {
            let c = at(0.5, 0.52);
            let hex: Vec<Pos2> = (0..6).map(|i| {
                let a = std::f32::consts::PI / 3.0 * i as f32 + std::f32::consts::PI / 6.0;
                c + vec2(a.cos(), a.sin()) * w * 0.38
            }).collect();
            poly(hex, SKY, ink);
        }
        Icon::Tool(Tool::PushPull) => {
            // a box with its top pushed up: the arrow rises from the top face
            cube(SAND, RED, Color32::from_rgb(170, 50, 40), ink);
            line(vec![at(0.5, 0.36), at(0.5, 0.04)], Stroke::new(w / 12.0, fade(INK)));
            poly(vec![at(0.36, 0.16), at(0.5, 0.0), at(0.64, 0.16)], INK, Stroke::NONE);
        }
        Icon::Tool(Tool::Orbit) => {
            // two arrows chasing round: red and green, as SketchUp's orbit
            let arc = |from: f32, to: f32| -> Vec<Pos2> {
                (0..=12).map(|i| {
                    let a = (from + (to - from) * i as f32 / 12.0).to_radians();
                    at(0.5, 0.5) + vec2(a.cos(), -a.sin()) * w * 0.34
                }).collect()
            };
            line(arc(20.0, 160.0), Stroke::new(w / 10.0, fade(RED)));
            line(arc(200.0, 340.0), Stroke::new(w / 10.0, fade(GREEN)));
            poly(vec![at(0.06, 0.48), at(0.26, 0.48), at(0.16, 0.62)], RED, Stroke::NONE);
            poly(vec![at(0.74, 0.52), at(0.94, 0.52), at(0.84, 0.38)], GREEN, Stroke::NONE);
        }
        Icon::Tool(Tool::Pan) => {
            // an open hand
            let palm = Rect::from_min_max(at(0.28, 0.46), at(0.74, 0.86));
            p.rect(palm, w * 0.12, fade(SAND), ink, egui::StrokeKind::Middle);
            for (i, x) in [0.32, 0.43, 0.54, 0.65].iter().enumerate() {
                let top = if i == 1 || i == 2 { 0.14 } else { 0.22 };
                p.rect(Rect::from_min_max(at(*x, top), at(x + 0.09, 0.56)), w * 0.05, fade(SAND), ink, egui::StrokeKind::Middle);
            }
            p.rect(Rect::from_min_max(at(0.14, 0.5), at(0.32, 0.6)), w * 0.05, fade(SAND), ink, egui::StrokeKind::Middle);
        }
        Icon::Tool(Tool::Zoom) => {
            p.circle(at(0.42, 0.42), w * 0.26, fade(SKY), ink);
            line(vec![at(0.6, 0.6), at(0.88, 0.88)], Stroke::new(w / 8.0, fade(INK)));
        }
        Icon::ZoomExtents => {
            // a magnifier over a frame with corner marks
            for (a, b, c) in [((0.08, 0.3), (0.08, 0.08), (0.3, 0.08)), ((0.7, 0.08), (0.92, 0.08), (0.92, 0.3)), ((0.08, 0.7), (0.08, 0.92), (0.3, 0.92)), ((0.92, 0.7), (0.92, 0.92), (0.7, 0.92))] {
                line(vec![at(a.0, a.1), at(b.0, b.1), at(c.0, c.1)], Stroke::new(w / 14.0, fade(RED)));
            }
            p.circle(at(0.44, 0.44), w * 0.2, fade(SKY), ink);
            line(vec![at(0.58, 0.58), at(0.78, 0.78)], Stroke::new(w / 9.0, fade(INK)));
        }
        Icon::Top => {
            // the box seen from above: its top face filled
            p.rect(Rect::from_min_max(at(0.16, 0.16), at(0.84, 0.84)), 0.0, fade(SAND), ink, egui::StrokeKind::Middle);
            line(vec![at(0.5, 0.3), at(0.5, 0.7)], Stroke::new(w / 16.0, fade(GREEN)));
            line(vec![at(0.3, 0.5), at(0.7, 0.5)], Stroke::new(w / 16.0, fade(RED)));
        }
        Icon::Iso => cube(SAND, GREY, Color32::from_rgb(160, 162, 170), ink),
        Icon::Parallel => {
            // a box drawn with parallel edges (no perspective), next to its parallel guide lines
            cube(Color32::WHITE, GREY, Color32::from_rgb(160, 162, 170), thin);
            line(vec![at(0.04, 0.2), at(0.36, 0.04)], Stroke::new(w / 16.0, fade(BLUE)));
            line(vec![at(0.64, 0.96), at(0.96, 0.8)], Stroke::new(w / 16.0, fade(BLUE)));
        }
    }
}

/// The name shown on hover.
pub fn name(icon: Icon) -> String {
    match icon {
        Icon::Tool(t) => match t.shortcut() {
            Some(k) => format!("{} ({k})", t.label()),
            None => t.label().to_string(),
        },
        Icon::ZoomExtents => "Zoom Extents (Shift+Z)".into(),
        Icon::Top => "Top view".into(),
        Icon::Iso => "Iso view".into(),
        Icon::Parallel => "Parallel projection".into(),
    }
}
