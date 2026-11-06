//! The toolbar's icons, drawn as vector shapes in SketchUp's manner: a coloured picture of what the tool does (a red box with a
//! push arrow, a pencil, a pink eraser, a magnifier), dark outlines, so they read at a glance and stay sharp at any size. No
//! image files to ship.

use crate::render::FaceStyle;
use crate::tools::Tool;
use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Tool(Tool),
    ZoomExtents,
    Previous,
    Top,
    Iso,
    Front,
    Back,
    Right,
    Left,
    Parallel,
    Style(FaceStyle),
    BackEdges,
    /// A SketchUp tool that has its place in the toolbar but is not built yet (shown greyed out).
    Planned(Planned),
}

/// Tools of SketchUp's Large Tool Set still to come; see PROGRESS.md for the order they are built in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Planned {
    MakeComponent,
    PaintBucket,
    Freehand,
    Arc,
    TwoPointArc,
    ThreePointArc,
    FollowMe,
    Offset,
    Dimensions,
    Text,
    Axes,
    Text3D,
    SectionPlane,
}

impl Planned {
    pub fn label(self) -> &'static str {
        match self {
            Planned::MakeComponent => "Make Component",
            Planned::PaintBucket => "Paint Bucket",
            Planned::Freehand => "Freehand",
            Planned::Arc => "Arc",
            Planned::TwoPointArc => "2 Point Arc",
            Planned::ThreePointArc => "3 Point Arc",
            Planned::FollowMe => "Follow Me",
            Planned::Offset => "Offset",
            Planned::Dimensions => "Dimensions",
            Planned::Text => "Text",
            Planned::Axes => "Axes",
            Planned::Text3D => "3D Text",
            Planned::SectionPlane => "Section Plane",
        }
    }
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
    let arrow_head = |tip: Pos2, dir: Vec2, size: f32, c: Color32| {
        let d = dir.normalized();
        let n = vec2(-d.y, d.x);
        poly(vec![tip, tip - d * size + n * size * 0.6, tip - d * size - n * size * 0.6], c, Stroke::NONE);
    };
    let arc_pts = |c: Pos2, r: f32, from: f32, to: f32| -> Vec<Pos2> {
        (0..=16).map(|i| {
            let a = (from + (to - from) * i as f32 / 16.0).to_radians();
            c + vec2(a.cos(), -a.sin()) * r
        }).collect()
    };
    // a little house, for the standard views
    let house = |front: bool, side: f32| {
        let (l, r) = (0.18 + side * 0.08, 0.82 + side * 0.08);
        let body = vec![at(l, 0.48), at(r, 0.48), at(r, 0.88), at(l, 0.88)];
        poly(body, if front { SAND } else { Color32::WHITE }, ink);
        poly(vec![at(l - 0.06, 0.5), at(0.5 + side * 0.08, 0.14), at(r + 0.06, 0.5)], GREY, ink);
        if front {
            p.rect(Rect::from_min_max(at(0.42 + side * 0.08, 0.62), at(0.58 + side * 0.08, 0.88)), 0.0, fade(INK), Stroke::NONE, egui::StrokeKind::Middle);
        }
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
        Icon::Tool(Tool::RotatedRectangle) => {
            poly(vec![at(0.1, 0.56), at(0.5, 0.16), at(0.9, 0.5), at(0.5, 0.9)], SKY, ink);
            for (x, y) in [(0.1, 0.56), (0.5, 0.16), (0.9, 0.5)] {
                p.circle_filled(at(x, y), w / 14.0, fade(RED));
            }
        }
        Icon::Tool(Tool::Pie) => {
            let c = at(0.3, 0.72);
            let mut pts = vec![c];
            pts.extend(arc_pts(c, w * 0.6, 0.0, 70.0));
            poly(pts.clone(), SKY, ink);
            p.circle_filled(c, w / 14.0, fade(RED));
        }
        Icon::Tool(Tool::Move) => {
            // four red arrows from the middle
            let c = at(0.5, 0.5);
            for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
                line(vec![c, c + d * w * 0.3], Stroke::new(w / 9.0, fade(RED)));
                arrow_head(c + d * w * 0.44, d, w * 0.16, RED);
            }
        }
        Icon::Tool(Tool::Rotate) => {
            line(arc_pts(at(0.5, 0.5), w * 0.32, 30.0, 170.0), Stroke::new(w / 9.0, fade(RED)));
            line(arc_pts(at(0.5, 0.5), w * 0.32, 210.0, 350.0), Stroke::new(w / 9.0, fade(RED)));
            arrow_head(at(0.2, 0.42), vec2(-0.2, 1.0), w * 0.18, RED);
            arrow_head(at(0.8, 0.58), vec2(0.2, -1.0), w * 0.18, RED);
        }
        Icon::Tool(Tool::Scale) => {
            p.rect(Rect::from_min_max(at(0.1, 0.4), at(0.6, 0.9)), 0.0, fade(SAND), ink, egui::StrokeKind::Middle);
            p.rect(Rect::from_min_max(at(0.1, 0.1), at(0.9, 0.9)), 0.0, Color32::TRANSPARENT, Stroke::new(w / 20.0, fade(RED)), egui::StrokeKind::Middle);
            line(vec![at(0.5, 0.5), at(0.78, 0.22)], Stroke::new(w / 10.0, fade(RED)));
            arrow_head(at(0.88, 0.12), vec2(1.0, -1.0), w * 0.2, RED);
        }
        Icon::Tool(Tool::TapeMeasure) => {
            p.circle(at(0.62, 0.4), w * 0.28, fade(Color32::from_rgb(240, 200, 40)), ink);
            p.circle(at(0.62, 0.4), w * 0.1, fade(GREY), thin);
            poly(vec![at(0.4, 0.58), at(0.1, 0.88), at(0.18, 0.94), at(0.48, 0.66)], Color32::from_rgb(240, 200, 40), thin);
        }
        Icon::Tool(Tool::Protractor) => {
            let c = at(0.5, 0.76);
            let mut half = arc_pts(c, w * 0.42, 0.0, 180.0);
            half.push(c);
            poly(half, Color32::from_rgb(240, 200, 40), ink);
            line(vec![c, at(0.78, 0.34)], Stroke::new(w / 16.0, fade(INK)));
        }
        Icon::Tool(Tool::ZoomWindow) => {
            p.rect(Rect::from_min_max(at(0.06, 0.06), at(0.7, 0.7)), 0.0, Color32::TRANSPARENT, Stroke::new(w / 18.0, fade(RED)), egui::StrokeKind::Middle);
            p.circle(at(0.5, 0.5), w * 0.2, fade(SKY), ink);
            line(vec![at(0.64, 0.64), at(0.9, 0.9)], Stroke::new(w / 9.0, fade(INK)));
        }
        Icon::Tool(Tool::PositionCamera) => {
            // a standing figure over a red cross on the ground
            p.circle(at(0.5, 0.16), w * 0.09, fade(Color32::WHITE), thin);
            line(vec![at(0.5, 0.26), at(0.5, 0.6)], ink);
            line(vec![at(0.32, 0.4), at(0.68, 0.4)], ink);
            line(vec![at(0.36, 0.84), at(0.5, 0.6), at(0.64, 0.84)], ink);
            line(vec![at(0.3, 0.8), at(0.7, 0.96)], Stroke::new(w / 16.0, fade(RED)));
            line(vec![at(0.3, 0.96), at(0.7, 0.8)], Stroke::new(w / 16.0, fade(RED)));
        }
        Icon::Tool(Tool::LookAround) => {
            // an eye
            let top = arc_pts(at(0.5, 0.82), w * 0.5, 40.0, 140.0);
            let bottom = arc_pts(at(0.5, 0.18), w * 0.5, 220.0, 320.0);
            let mut outline = top.clone();
            outline.extend(bottom);
            p.add(Shape::closed_line(outline, ink));
            p.circle(at(0.5, 0.5), w * 0.16, fade(BLUE), thin);
            p.circle_filled(at(0.5, 0.5), w * 0.07, fade(INK));
        }
        Icon::Tool(Tool::Walk) => {
            // two footprints
            for (x, y) in [(0.32, 0.6), (0.66, 0.36)] {
                p.circle_filled(at(x, y), w * 0.13, fade(INK));
                p.circle_filled(at(x, y + 0.22), w * 0.08, fade(INK));
            }
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
        Icon::Previous => {
            p.circle(at(0.5, 0.56), w * 0.24, fade(SKY), ink);
            line(arc_pts(at(0.38, 0.4), w * 0.28, 60.0, 200.0), Stroke::new(w / 11.0, fade(BLUE)));
            arrow_head(at(0.16, 0.56), vec2(-0.2, 1.0), w * 0.18, BLUE);
        }
        Icon::Front => house(true, 0.0),
        Icon::Back => house(false, 0.0),
        Icon::Right => house(false, 0.06),
        Icon::Left => house(false, -0.06),
        Icon::Style(st) => {
            let (fill, s) = match st {
                FaceStyle::Wireframe => (Color32::TRANSPARENT, thin),
                FaceStyle::HiddenLine => (Color32::WHITE, ink),
                FaceStyle::Shaded => (SAND, ink),
                FaceStyle::XRay => (SKY.gamma_multiply(0.6), thin),
            };
            cube(fill, if fill == Color32::TRANSPARENT { fill } else if st == FaceStyle::Shaded { GREY } else { fill }, if st == FaceStyle::Shaded { Color32::from_rgb(160, 162, 170) } else { fill }, s);
            if matches!(st, FaceStyle::Wireframe | FaceStyle::XRay) {
                // the hidden corner's edges
                line(vec![at(0.14, 0.72), at(0.5, 0.54), at(0.86, 0.72)], thin);
                line(vec![at(0.5, 0.54), at(0.5, 0.18)], thin);
            }
        }
        Icon::BackEdges => {
            cube(Color32::WHITE, Color32::WHITE, Color32::WHITE, ink);
            let dash = Stroke::new(w / 22.0, fade(Color32::from_gray(120)));
            for (a, b) in [((0.14, 0.72), (0.32, 0.63)), ((0.5, 0.54), (0.5, 0.36))] {
                line(vec![at(a.0, a.1), at(b.0, b.1)], dash);
            }
        }
        Icon::Planned(pl) => planned(p, rect, pl),
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
        Icon::Previous => "Previous view".into(),
        Icon::Top => "Top view".into(),
        Icon::Iso => "Iso view".into(),
        Icon::Front => "Front view".into(),
        Icon::Back => "Back view".into(),
        Icon::Right => "Right view".into(),
        Icon::Left => "Left view".into(),
        Icon::Parallel => "Parallel projection".into(),
        Icon::Style(s) => s.label().into(),
        Icon::BackEdges => "Back Edges".into(),
        Icon::Planned(pl) => format!("{} (not built yet)", pl.label()),
    }
}

/// The pictures for tools not built yet; always drawn greyed out.
fn planned(p: &Painter, rect: Rect, pl: Planned) {
    let w = rect.width();
    let at = |x: f32, y: f32| pos2(rect.left() + x * w, rect.top() + y * w);
    let c = |c: Color32| c.gamma_multiply(0.35);
    let ink = Stroke::new((w / 16.0).max(1.2), c(INK));
    let red = Stroke::new(w / 12.0, c(RED));
    let line = |pts: Vec<Pos2>, s: Stroke| p.add(Shape::line(pts, s));
    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| -> Vec<Pos2> {
        (0..=16).map(|i| {
            let a = (from + (to - from) * i as f32 / 16.0).to_radians();
            at(cx, cy) + vec2(a.cos(), -a.sin()) * r * w
        }).collect()
    };
    match pl {
        Planned::MakeComponent | Planned::Axes => {
            let tl = |x: f32, y: f32| at(x, y);
            p.add(Shape::convex_polygon(vec![tl(0.5, 0.18), tl(0.86, 0.36), tl(0.5, 0.54), tl(0.14, 0.36)], c(SAND), ink));
            p.add(Shape::convex_polygon(vec![tl(0.14, 0.36), tl(0.5, 0.54), tl(0.5, 0.9), tl(0.14, 0.72)], c(GREY), ink));
            p.add(Shape::convex_polygon(vec![tl(0.5, 0.54), tl(0.86, 0.36), tl(0.86, 0.72), tl(0.5, 0.9)], c(GREY), ink));
            if pl == Planned::MakeComponent {
                p.rect(Rect::from_min_max(at(0.06, 0.08), at(0.94, 0.94)), 0.0, Color32::TRANSPARENT, red, egui::StrokeKind::Middle);
            }
        }
        Planned::PaintBucket => {
            p.add(Shape::convex_polygon(vec![at(0.2, 0.4), at(0.7, 0.3), at(0.8, 0.8), at(0.3, 0.9)], c(GREY), ink));
            p.circle_filled(at(0.24, 0.3), w * 0.14, c(WOOD));
        }
        Planned::Freehand => {
            line(vec![at(0.1, 0.8), at(0.3, 0.3), at(0.5, 0.7), at(0.7, 0.2), at(0.9, 0.6)], red);
        }
        Planned::Arc | Planned::TwoPointArc | Planned::ThreePointArc => {
            line(arc(0.5, 0.8, 0.4, 0.0, 180.0), ink);
            let n = match pl { Planned::Arc => 1, Planned::TwoPointArc => 2, _ => 3 };
            for (i, (x, y)) in [(0.1, 0.8), (0.9, 0.8), (0.5, 0.4)].iter().enumerate().take(n.max(2)) {
                if i < n || n == 1 {
                    p.circle_filled(at(*x, *y), w / 14.0, c(RED));
                }
            }
        }
        Planned::FollowMe => {
            line(arc(0.5, 0.5, 0.34, 200.0, 520.0), Stroke::new(w / 7.0, c(RED)));
        }
        Planned::Offset => {
            p.rect(Rect::from_min_max(at(0.1, 0.1), at(0.9, 0.9)), 0.0, c(SKY), ink, egui::StrokeKind::Middle);
            p.rect(Rect::from_min_max(at(0.28, 0.28), at(0.72, 0.72)), 0.0, Color32::TRANSPARENT, red, egui::StrokeKind::Middle);
        }
        Planned::Dimensions => {
            line(vec![at(0.12, 0.3), at(0.12, 0.7)], ink);
            line(vec![at(0.88, 0.3), at(0.88, 0.7)], ink);
            line(vec![at(0.12, 0.5), at(0.88, 0.5)], ink);
            p.text(at(0.5, 0.32), egui::Align2::CENTER_CENTER, "5", egui::FontId::proportional(w * 0.32), c(INK));
        }
        Planned::Text | Planned::Text3D => {
            p.rect(Rect::from_min_max(at(0.3, 0.14), at(0.94, 0.54)), 0.0, c(Color32::WHITE), ink, egui::StrokeKind::Middle);
            p.text(at(0.62, 0.34), egui::Align2::CENTER_CENTER, if pl == Planned::Text { "A1" } else { "A" }, egui::FontId::proportional(w * 0.3), c(INK));
            line(vec![at(0.3, 0.54), at(0.1, 0.9)], ink);
        }
        Planned::SectionPlane => {
            p.circle(at(0.5, 0.5), w * 0.3, Color32::TRANSPARENT, ink);
            line(vec![at(0.06, 0.5), at(0.94, 0.5)], ink);
            p.text(at(0.5, 0.36), egui::Align2::CENTER_CENTER, "C", egui::FontId::proportional(w * 0.22), c(INK));
        }
    }
}
