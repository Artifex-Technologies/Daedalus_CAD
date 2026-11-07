//! The toolbar's icons: thin line drawings in one colour that follows the light or dark style (dark ink on a light
//! toolbar, light on a dark one), with an orange accent here and there, in the manner of current SketchUp-like
//! modelers. Drawn as vector shapes, so they stay sharp at any size and there are no image files to ship.

use crate::render::FaceStyle;
use crate::tools::Tool;
use egui::{pos2, Color32, Painter, Pos2, Rect, Shape, Stroke};

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
    /// Sandbox buttons that act on the selection: From Contours, Add Detail.
    FromContours,
    AddDetail,
    /// A SketchUp tool that has its place in the toolbar but is not built yet (shown greyed out).
    Planned(Planned),
}

/// Tools of SketchUp's Large Tool Set still to come; see PROGRESS.md for the order they are built in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Planned {
    MakeComponent,
    Text,
    Axes,
    Text3D,
}

impl Planned {
    pub fn label(self) -> &'static str {
        match self {
            Planned::MakeComponent => "Make Component",
            Planned::Text => "Text",
            Planned::Axes => "Axes",
            Planned::Text3D => "3D Text",
        }
    }
}

/// The accent for the few icons that carry one (paint, 3D text, the section plane, axes).
const ACCENT: Color32 = Color32::from_rgb(232, 120, 40);

/// Draws `icon` into `rect` (a square) in `ink`. `dim` greys it out (a tool that cannot be used now, or not built yet).
pub fn paint(p: &Painter, rect: Rect, icon: Icon, dim: bool, ink: Color32) {
    p.extend(shapes(rect, icon, dim, ink));
}

/// The shapes `icon` is drawn with, in `rect` (for the toolbar, and for the tools' mouse cursors, see cursors.rs).
pub fn shapes(rect: Rect, icon: Icon, dim: bool, ink: Color32) -> Vec<Shape> {
    let fade = |c: Color32| if dim { c.gamma_multiply(0.35) } else { c };
    let d = Draw { out: std::cell::RefCell::new(Vec::new()), rect, w: rect.width(), ink: fade(ink), accent: fade(ACCENT), dim };
    d.icon(icon);
    d.out.into_inner()
}

struct Draw {
    out: std::cell::RefCell<Vec<Shape>>,
    rect: Rect,
    w: f32,
    ink: Color32,
    accent: Color32,
    dim: bool,
}

impl Draw {
    fn add(&self, s: Shape) {
        self.out.borrow_mut().push(s);
    }

    fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.rect.left() + x * self.w, self.rect.top() + y * self.w)
    }

    fn pts(&self, v: &[(f32, f32)]) -> Vec<Pos2> {
        v.iter().map(|&(x, y)| self.at(x, y)).collect()
    }

    fn stroke(&self) -> Stroke {
        Stroke::new((self.w / 15.0).max(1.2), self.ink)
    }

    fn thin(&self) -> Stroke {
        Stroke::new((self.w / 24.0).max(1.0), self.ink)
    }

    fn line(&self, v: &[(f32, f32)]) {
        self.add(Shape::line(self.pts(v), self.stroke()));
    }

    fn line_with(&self, v: &[(f32, f32)], s: Stroke) {
        self.add(Shape::line(self.pts(v), s));
    }

    fn closed(&self, v: &[(f32, f32)]) {
        self.add(Shape::closed_line(self.pts(v), self.stroke()));
    }

    fn fill(&self, v: &[(f32, f32)], c: Color32) {
        self.add(Shape::convex_polygon(self.pts(v), c, Stroke::NONE));
    }

    fn dot(&self, x: f32, y: f32) {
        self.add(Shape::circle_filled(self.at(x, y), self.w * 0.06, self.ink));
    }

    fn circle(&self, x: f32, y: f32, r: f32) {
        self.add(Shape::circle_stroke(self.at(x, y), r * self.w, self.stroke()));
    }

    /// Points on a circle from `from` to `to` degrees (counter-clockwise, 0 to the right).
    fn arc(&self, cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
        (0..=20)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / 20.0).to_radians();
                (cx + r * a.cos(), cy - r * a.sin())
            })
            .collect()
    }

    /// A filled arrowhead with its tip at (x, y) pointing along (dx, dy).
    fn head(&self, x: f32, y: f32, dx: f32, dy: f32, size: f32, c: Color32) {
        let l = (dx * dx + dy * dy).sqrt().max(1e-6);
        let (ux, uy) = (dx / l, dy / l);
        let (nx, ny) = (-uy, ux);
        let b = (x - ux * size, y - uy * size);
        self.fill(&[(x, y), (b.0 + nx * size * 0.55, b.1 + ny * size * 0.55), (b.0 - nx * size * 0.55, b.1 - ny * size * 0.55)], c);
    }

    fn dashed(&self, a: (f32, f32), b: (f32, f32)) {
        let n = 5;
        for i in (0..n).step_by(2) {
            let t0 = i as f32 / n as f32;
            let t1 = (i + 1) as f32 / n as f32;
            self.line_with(&[(a.0 + (b.0 - a.0) * t0, a.1 + (b.1 - a.1) * t0), (a.0 + (b.0 - a.0) * t1, a.1 + (b.1 - a.1) * t1)], self.thin());
        }
    }

    /// An isometric box: the six visible edges, the hidden ones drawn as `hidden` says.
    fn cube(&self, hidden: Hidden, fill: Option<Color32>) {
        let top = [(0.5, 0.14), (0.86, 0.32), (0.5, 0.5), (0.14, 0.32)];
        let left = [(0.14, 0.32), (0.5, 0.5), (0.5, 0.88), (0.14, 0.7)];
        let right = [(0.5, 0.5), (0.86, 0.32), (0.86, 0.7), (0.5, 0.88)];
        if let Some(c) = fill {
            self.fill(&top, c.gamma_multiply(0.35));
            self.fill(&left, c.gamma_multiply(0.6));
            self.fill(&right, c.gamma_multiply(0.85));
        }
        self.closed(&[(0.5, 0.14), (0.86, 0.32), (0.86, 0.7), (0.5, 0.88), (0.14, 0.7), (0.14, 0.32)]);
        self.line(&[(0.14, 0.32), (0.5, 0.5), (0.86, 0.32)]);
        self.line(&[(0.5, 0.5), (0.5, 0.88)]);
        let back = (0.5, 0.52);
        match hidden {
            Hidden::None => {}
            Hidden::Solid => {
                self.line_with(&[(0.14, 0.7), back, (0.86, 0.7)], self.thin());
                self.line_with(&[back, (0.5, 0.14)], self.thin());
            }
            Hidden::Dashed => {
                self.dashed((0.14, 0.7), back);
                self.dashed(back, (0.86, 0.7));
                self.dashed(back, (0.5, 0.14));
            }
        }
    }

    /// A house seen square on, for the standard views: `door` for the front, shifted sideways for the sides.
    fn house(&self, door: bool, shift: f32) {
        let (l, r) = (0.2 + shift, 0.8 + shift);
        let mid = 0.5 + shift;
        self.closed(&[(l, 0.48), (mid, 0.16), (r, 0.48), (r, 0.86), (l, 0.86)]);
        if door {
            self.closed(&[(mid - 0.08, 0.86), (mid - 0.08, 0.62), (mid + 0.08, 0.62), (mid + 0.08, 0.86)]);
        }
        if shift != 0.0 {
            // the side wall going back
            let s = if shift > 0.0 { l } else { r };
            let o = if shift > 0.0 { -0.12 } else { 0.12 };
            self.line(&[(s, 0.48), (s + o, 0.4), (s + o, 0.8), (s, 0.86)]);
        }
    }

    fn magnifier(&self, x: f32, y: f32, r: f32) {
        self.circle(x, y, r);
        let k = r * 0.72;
        self.line_with(&[(x + k, y + k), (x + k + 0.26, y + k + 0.26)], Stroke::new(self.w / 9.0, self.ink));
    }

    fn icon(&self, icon: Icon) {
        let ink = self.ink;
        match icon {
            Icon::Tool(Tool::Select) => {
                self.fill(&[(0.3, 0.1), (0.3, 0.78), (0.74, 0.62)], ink);
                self.fill(&[(0.42, 0.64), (0.56, 0.92), (0.68, 0.86), (0.55, 0.6)], ink);
            }
            Icon::Tool(Tool::Eraser) => {
                self.closed(&[(0.14, 0.62), (0.5, 0.26), (0.84, 0.58), (0.48, 0.94)]);
                self.line(&[(0.32, 0.44), (0.66, 0.78)]);
                self.fill(&[(0.5, 0.26), (0.84, 0.58), (0.66, 0.76), (0.32, 0.44)], ink.gamma_multiply(0.35));
            }
            Icon::Tool(Tool::Line) => {
                self.line(&[(0.16, 0.84), (0.84, 0.16)]);
                self.dot(0.16, 0.84);
                self.dot(0.84, 0.16);
            }
            Icon::Tool(Tool::Rectangle) => self.closed(&[(0.14, 0.24), (0.86, 0.24), (0.86, 0.76), (0.14, 0.76)]),
            Icon::Tool(Tool::RotatedRectangle) => self.closed(&[(0.5, 0.12), (0.88, 0.5), (0.5, 0.88), (0.12, 0.5)]),
            Icon::Tool(Tool::Circle) => {
                self.circle(0.5, 0.5, 0.36);
                self.dot(0.5, 0.5);
            }
            Icon::Tool(Tool::Polygon) => {
                let hex: Vec<(f32, f32)> = (0..6)
                    .map(|i| {
                        let a = std::f32::consts::PI / 3.0 * i as f32 + std::f32::consts::PI / 6.0;
                        (0.5 + 0.38 * a.cos(), 0.5 + 0.38 * a.sin())
                    })
                    .collect();
                self.closed(&hex);
            }
            Icon::Tool(Tool::Freehand) => self.line(&[(0.1, 0.8), (0.24, 0.4), (0.38, 0.72), (0.56, 0.24), (0.72, 0.62), (0.9, 0.3)]),
            Icon::Tool(Tool::Arc) => {
                self.line(&self.arc(0.5, 0.78, 0.38, 0.0, 180.0));
                self.dot(0.5, 0.78);
                self.dot(0.88, 0.78);
            }
            Icon::Tool(Tool::TwoPointArc) => {
                self.line(&self.arc(0.5, 0.78, 0.38, 0.0, 180.0));
                self.dot(0.12, 0.78);
                self.dot(0.88, 0.78);
            }
            Icon::Tool(Tool::ThreePointArc) => {
                self.line(&self.arc(0.5, 0.78, 0.38, 0.0, 180.0));
                self.dot(0.12, 0.78);
                self.dot(0.5, 0.4);
                self.dot(0.88, 0.78);
            }
            Icon::Tool(Tool::Offset) => {
                self.closed(&[(0.1, 0.1), (0.9, 0.1), (0.9, 0.9), (0.1, 0.9)]);
                self.closed(&[(0.3, 0.3), (0.7, 0.3), (0.7, 0.7), (0.3, 0.7)]);
            }
            Icon::Tool(Tool::PaintBucket) => {
                self.closed(&[(0.16, 0.42), (0.56, 0.22), (0.78, 0.66), (0.38, 0.86)]);
                self.line(&self.arc(0.36, 0.32, 0.2, 60.0, 200.0));
                self.fill(&[(0.82, 0.56), (0.9, 0.74), (0.82, 0.84), (0.74, 0.74)], self.accent);
            }
            Icon::Tool(Tool::FollowMe) => {
                self.line(&self.arc(0.5, 0.56, 0.34, 180.0, 360.0));
                self.closed(&[(0.06, 0.4), (0.26, 0.4), (0.26, 0.6), (0.06, 0.6)]);
                self.head(0.84, 0.48, 0.0, -1.0, 0.16, ink);
            }
            Icon::Tool(Tool::SectionPlane) => {
                self.closed(&[(0.2, 0.2), (0.7, 0.08), (0.7, 0.8), (0.2, 0.92)]);
                self.line_with(&[(0.45, 0.5), (0.92, 0.5)], Stroke::new(self.w / 15.0, self.accent));
                self.head(0.98, 0.5, 1.0, 0.0, 0.14, self.accent);
            }
            Icon::Tool(Tool::Dimensions) => {
                self.line(&[(0.12, 0.3), (0.12, 0.78)]);
                self.line(&[(0.88, 0.3), (0.88, 0.78)]);
                self.line(&[(0.2, 0.54), (0.8, 0.54)]);
                self.head(0.14, 0.54, -1.0, 0.0, 0.14, ink);
                self.head(0.86, 0.54, 1.0, 0.0, 0.14, ink);
            }
            Icon::Tool(Tool::Pie) => {
                let mut v = vec![(0.14, 0.86)];
                v.extend(self.arc(0.14, 0.86, 0.72, 0.0, 90.0));
                self.closed(&v);
            }
            Icon::Tool(Tool::PushPull) => {
                // a slab with an arrow rising from its top
                self.closed(&[(0.1, 0.66), (0.5, 0.5), (0.9, 0.66), (0.5, 0.82)]);
                self.line(&[(0.1, 0.66), (0.1, 0.76), (0.5, 0.92), (0.9, 0.76), (0.9, 0.66)]);
                self.line(&[(0.5, 0.82), (0.5, 0.92)]);
                self.line(&[(0.5, 0.66), (0.5, 0.24)]);
                self.head(0.5, 0.08, 0.0, -1.0, 0.2, ink);
            }
            Icon::Tool(Tool::Move) => {
                self.line(&[(0.18, 0.5), (0.82, 0.5)]);
                self.line(&[(0.5, 0.18), (0.5, 0.82)]);
                for (x, y, dx, dy) in [(0.94, 0.5, 1.0, 0.0), (0.06, 0.5, -1.0, 0.0), (0.5, 0.06, 0.0, -1.0), (0.5, 0.94, 0.0, 1.0)] {
                    self.head(x, y, dx, dy, 0.18, ink);
                }
            }
            Icon::Tool(Tool::Rotate) => {
                self.line(&self.arc(0.5, 0.5, 0.34, 30.0, 170.0));
                self.line(&self.arc(0.5, 0.5, 0.34, 210.0, 350.0));
                self.head(0.17, 0.5, -0.15, 1.0, 0.18, ink);
                self.head(0.83, 0.5, 0.15, -1.0, 0.18, ink);
            }
            Icon::Tool(Tool::Scale) => {
                self.closed(&[(0.12, 0.5), (0.5, 0.5), (0.5, 0.88), (0.12, 0.88)]);
                self.dashed((0.12, 0.12), (0.88, 0.12));
                self.dashed((0.88, 0.12), (0.88, 0.88));
                self.dashed((0.12, 0.12), (0.12, 0.5));
                self.dashed((0.5, 0.88), (0.88, 0.88));
                self.line(&[(0.5, 0.5), (0.76, 0.24)]);
                self.head(0.86, 0.14, 1.0, -1.0, 0.18, ink);
            }
            Icon::Tool(Tool::TapeMeasure) => {
                self.circle(0.6, 0.4, 0.28);
                self.circle(0.6, 0.4, 0.08);
                self.line_with(&[(0.4, 0.6), (0.1, 0.9)], Stroke::new(self.w / 9.0, ink));
            }
            Icon::Tool(Tool::Protractor) => {
                let mut v = self.arc(0.5, 0.78, 0.42, 0.0, 180.0);
                v.push((0.5, 0.78));
                self.closed(&v);
                self.line(&[(0.5, 0.78), (0.76, 0.42)]);
                self.dot(0.5, 0.78);
            }
            Icon::Tool(Tool::Orbit) => {
                // going round an upright axis: the axis, and a flat ring of arrows about it
                self.line(&[(0.5, 0.06), (0.5, 0.94)]);
                let e: Vec<(f32, f32)> = (0..=20)
                    .map(|i| {
                        let a = (200.0 + 300.0 * i as f32 / 20.0f32).to_radians();
                        (0.5 + 0.42 * a.cos(), 0.56 - 0.18 * a.sin())
                    })
                    .collect();
                self.line(&e);
                self.head(0.5 + 0.42 * 140f32.to_radians().cos(), 0.56 - 0.18 * 140f32.to_radians().sin(), -1.0, -0.35, 0.16, ink);
            }
            Icon::Tool(Tool::Pan) => {
                // an open hand: palm and four fingers
                self.closed(&[(0.26, 0.5), (0.26, 0.76), (0.4, 0.92), (0.7, 0.92), (0.78, 0.7), (0.78, 0.46)]);
                for (x, top) in [(0.3, 0.26), (0.44, 0.14), (0.58, 0.16), (0.72, 0.3)] {
                    self.line(&[(x, 0.52), (x, top)]);
                }
                self.line(&[(0.26, 0.62), (0.12, 0.5)]);
            }
            Icon::Tool(Tool::Zoom) => self.magnifier(0.42, 0.42, 0.26),
            Icon::Tool(Tool::ZoomWindow) => {
                self.dashed((0.08, 0.08), (0.7, 0.08));
                self.dashed((0.08, 0.08), (0.08, 0.7));
                self.dashed((0.7, 0.08), (0.7, 0.32));
                self.dashed((0.08, 0.7), (0.32, 0.7));
                self.magnifier(0.52, 0.52, 0.2);
            }
            Icon::ZoomExtents => {
                for &(a, b, c) in &[((0.1, 0.32), (0.1, 0.1), (0.32, 0.1)), ((0.68, 0.1), (0.9, 0.1), (0.9, 0.32)), ((0.1, 0.68), (0.1, 0.9), (0.32, 0.9)), ((0.9, 0.68), (0.9, 0.9), (0.68, 0.9))] {
                    self.line(&[a, b, c]);
                }
                self.closed(&[(0.32, 0.32), (0.68, 0.32), (0.68, 0.68), (0.32, 0.68)]);
            }
            Icon::Previous => {
                self.magnifier(0.52, 0.5, 0.22);
                self.line(&self.arc(0.4, 0.42, 0.3, 100.0, 200.0));
                self.head(0.12, 0.5, 0.1, 1.0, 0.16, ink);
            }
            Icon::Tool(Tool::PositionCamera) => {
                self.circle(0.5, 0.16, 0.08);
                self.line(&[(0.5, 0.25), (0.5, 0.58)]);
                self.line(&[(0.3, 0.38), (0.7, 0.38)]);
                self.line(&[(0.36, 0.8), (0.5, 0.58), (0.64, 0.8)]);
                self.line_with(&[(0.32, 0.84), (0.68, 0.96)], Stroke::new(self.w / 15.0, self.accent));
                self.line_with(&[(0.32, 0.96), (0.68, 0.84)], Stroke::new(self.w / 15.0, self.accent));
            }
            Icon::Tool(Tool::LookAround) => {
                let mut eye = self.arc(0.5, 0.86, 0.52, 35.0, 145.0);
                eye.extend(self.arc(0.5, 0.14, 0.52, 215.0, 325.0));
                self.closed(&eye);
                self.circle(0.5, 0.5, 0.14);
                self.dot(0.5, 0.5);
            }
            Icon::Tool(Tool::Text) => self.planned(Planned::Text),
            Icon::Tool(Tool::Axes) => self.planned(Planned::Axes),
            Icon::Tool(Tool::Text3D) => self.planned(Planned::Text3D),
            Icon::Tool(t @ (Tool::SandboxScratch | Tool::Smoove | Tool::Stamp | Tool::Drape | Tool::FlipEdge)) => self.sandbox(Some(t)),
            Icon::FromContours => self.sandbox(None),
            Icon::Tool(Tool::Interact) => {
                // a pointing hand: the finger up, the fist below, a spark where it touches
                self.closed(&[(0.42, 0.18), (0.5, 0.12), (0.58, 0.18), (0.58, 0.45), (0.78, 0.5), (0.8, 0.78), (0.62, 0.92), (0.38, 0.92), (0.24, 0.72), (0.3, 0.55), (0.42, 0.6)]);
                self.line_with(&[(0.58, 0.45), (0.58, 0.62)], self.thin());
                self.line_with(&[(0.68, 0.48), (0.68, 0.64)], self.thin());
                self.line_with(&[(0.32, 0.1), (0.24, 0.04)], Stroke::new(1.5_f32, self.accent));
                self.line_with(&[(0.68, 0.1), (0.76, 0.04)], Stroke::new(1.5_f32, self.accent));
            }
            Icon::Tool(Tool::Flange) => {
                // a sheet seen from its end: the flat part along the bottom, bent up round a radius into the flange; an
                // orange arrow for the fold
                let mut outer = vec![(0.1, 0.84), (0.56, 0.84)];
                outer.extend(self.arc(0.56, 0.64, 0.2, 270.0, 360.0));
                outer.push((0.76, 0.14));
                let mut inner = vec![(0.1, 0.74), (0.56, 0.74)];
                inner.extend(self.arc(0.56, 0.64, 0.1, 270.0, 360.0));
                inner.push((0.66, 0.14));
                self.line(&outer);
                self.line(&inner);
                self.line(&[(0.1, 0.84), (0.1, 0.74)]);
                self.line(&[(0.76, 0.14), (0.66, 0.14)]);
                let fold = self.arc(0.32, 0.62, 0.2, 180.0, 95.0);
                self.line_with(&fold, Stroke::new(1.5_f32, self.accent));
                self.head(0.33, 0.42, 1.0, 0.0, 0.1, self.accent);
            }
            Icon::Tool(Tool::SheetConvert) => {
                // a block on the left, an arrow, the same unfolded into an L of sheet on the right
                self.closed(&[(0.08, 0.5), (0.36, 0.5), (0.36, 0.86), (0.08, 0.86)]);
                self.line(&[(0.08, 0.5), (0.18, 0.4), (0.46, 0.4), (0.36, 0.5)]);
                self.line(&[(0.46, 0.4), (0.46, 0.76), (0.36, 0.86)]);
                self.line(&[(0.56, 0.86), (0.92, 0.86), (0.92, 0.8), (0.62, 0.8), (0.62, 0.3), (0.56, 0.3), (0.56, 0.86)]);
                let accent = Stroke::new(1.5_f32, self.accent);
                self.line_with(&[(0.2, 0.28), (0.44, 0.16)], accent);
                self.head(0.5, 0.13, 1.0, -0.5, 0.09, self.accent);
            }
            Icon::Tool(Tool::Fold) => {
                // a plate with a dashed fold line across it, the far part turned up along it
                self.closed(&[(0.08, 0.62), (0.52, 0.62), (0.62, 0.86), (0.18, 0.86)]);
                self.dashed((0.52, 0.62), (0.62, 0.86));
                self.closed(&[(0.52, 0.62), (0.62, 0.86), (0.8, 0.5), (0.7, 0.26)]);
                let accent = Stroke::new(1.5_f32, self.accent);
                let turn = self.arc(0.72, 0.62, 0.2, 10.0, 80.0);
                self.line_with(&turn, accent);
                self.head(0.75, 0.42, -0.4, -1.0, 0.09, self.accent);
            }
            Icon::Tool(Tool::Hole) => {
                // a plate seen at a slant with a round hole and a slot through it, an orange arrow going in
                self.closed(&[(0.06, 0.62), (0.66, 0.62), (0.94, 0.88), (0.34, 0.88)]);
                self.circle(0.42, 0.74, 0.07);
                self.closed(&[(0.6, 0.7), (0.74, 0.7), (0.8, 0.78), (0.66, 0.78)]);
                self.line_with(&[(0.42, 0.08), (0.42, 0.5)], Stroke::new(1.5_f32, self.accent));
                self.head(0.42, 0.58, 0.0, 1.0, 0.1, self.accent);
            }
            Icon::Tool(Tool::Stress) => {
                // a bracket held at its foot (hatching), pushed down at its tip (arrow), bending
                self.line(&[(0.14, 0.86), (0.14, 0.3)]);
                for k in 0..4 {
                    let y = 0.36 + k as f32 * 0.14;
                    self.line_with(&[(0.14, y), (0.06, y + 0.08)], self.thin());
                }
                let beam = self.arc(0.14, 1.9, 1.6, 90.0, 66.0);
                self.line(&beam);
                let under = self.arc(0.14, 2.02, 1.6, 90.0, 66.0);
                self.line(&under);
                self.head(0.8, 0.42, 0.0, 1.0, 0.1, self.accent);
                self.line_with(&[(0.8, 0.1), (0.8, 0.34)], Stroke::new(1.5_f32, self.accent));
            }
            Icon::AddDetail => {
                self.sandbox_grid();
                self.dot(0.5, 0.55);
                self.line_with(&[(0.3, 0.7), (0.5, 0.55), (0.7, 0.7)], self.thin());
                self.line_with(&[(0.5, 0.55), (0.5, 0.35)], self.thin());
            }
            Icon::Tool(t @ (Tool::OuterShell | Tool::SolidIntersect | Tool::SolidUnion | Tool::SolidSubtract | Tool::SolidTrim | Tool::SolidSplit)) => self.solid(t),
            Icon::Tool(Tool::Walk) => {
                // two footprints: a sole and a heel each
                for (x, y) in [(0.3, 0.42), (0.68, 0.22)] {
                    let sole: Vec<(f32, f32)> = (0..16)
                        .map(|i| {
                            let a = std::f32::consts::TAU * i as f32 / 16.0;
                            (x + 0.11 * a.cos(), y + 0.17 * a.sin())
                        })
                        .collect();
                    self.fill(&sole, ink);
                    self.add(Shape::circle_filled(self.at(x, y + 0.31), self.w * 0.085, ink));
                }
            }
            Icon::Top => {
                self.closed(&[(0.16, 0.16), (0.84, 0.16), (0.84, 0.84), (0.16, 0.84)]);
                self.line(&[(0.16, 0.16), (0.5, 0.5), (0.84, 0.16)]);
                self.line(&[(0.16, 0.84), (0.5, 0.5), (0.84, 0.84)]);
            }
            Icon::Iso => self.cube(Hidden::None, None),
            Icon::Front => self.house(true, 0.0),
            Icon::Back => self.house(false, 0.0),
            Icon::Right => self.house(false, 0.08),
            Icon::Left => self.house(false, -0.08),
            Icon::Parallel => {
                self.cube(Hidden::None, None);
                self.line_with(&[(0.02, 0.26), (0.3, 0.12)], self.thin());
                self.line_with(&[(0.7, 0.98), (0.98, 0.84)], self.thin());
            }
            Icon::Style(FaceStyle::Wireframe) => self.cube(Hidden::Solid, None),
            Icon::Style(FaceStyle::HiddenLine) => self.cube(Hidden::None, None),
            Icon::Style(FaceStyle::Shaded) => self.cube(Hidden::None, Some(self.accent)),
            Icon::Style(FaceStyle::Monochrome) => self.cube(Hidden::None, Some(ink)),
            Icon::Style(FaceStyle::Textured) => {
                // a shaded cube with brick courses on its faces
                self.cube(Hidden::None, Some(self.accent));
                let t = self.thin();
                for (a, b) in [((0.14, 0.45), (0.5, 0.63)), ((0.14, 0.58), (0.5, 0.76)), ((0.5, 0.63), (0.86, 0.45)), ((0.5, 0.76), (0.86, 0.58))] {
                    self.line_with(&[a, b], t);
                }
                for (a, b) in [((0.32, 0.54), (0.32, 0.67)), ((0.68, 0.54), (0.68, 0.67))] {
                    self.line_with(&[a, b], t);
                }
            }
            Icon::Style(FaceStyle::XRay) => {
                self.cube(Hidden::Solid, Some(ink.gamma_multiply(0.5)));
            }
            Icon::BackEdges => self.cube(Hidden::Dashed, None),
            Icon::Planned(pl) => self.planned(pl),
        }
    }

    /// A small terrain grid seen at an angle.
    fn sandbox_grid(&self) {
        let t = self.thin();
        for k in 0..=3 {
            let f = k as f32 / 3.0;
            self.line_with(&[(0.1 + 0.3 * f, 0.62 - 0.22 * f), (0.6 + 0.3 * f, 0.88 - 0.22 * f)], t);
            self.line_with(&[(0.1 + 0.5 * f, 0.62 + 0.26 * f), (0.4 + 0.5 * f, 0.4 + 0.26 * f)], t);
        }
    }

    /// The Sandbox tools (None: From Contours).
    fn sandbox(&self, t: Option<Tool>) {
        match t {
            None => {
                // contour rings, one inside another
                self.closed(&[(0.1, 0.6), (0.4, 0.35), (0.85, 0.42), (0.9, 0.7), (0.5, 0.85), (0.15, 0.78)]);
                self.closed(&[(0.3, 0.6), (0.48, 0.48), (0.7, 0.52), (0.72, 0.66), (0.5, 0.72)]);
                self.dot(0.52, 0.6);
            }
            Some(Tool::SandboxScratch) => self.sandbox_grid(),
            Some(Tool::Smoove) => {
                self.sandbox_grid();
                self.line(&[(0.15, 0.55), (0.35, 0.5), (0.5, 0.22), (0.65, 0.5), (0.85, 0.55)]);
                self.head(0.5, 0.12, 0.0, -1.0, 0.12, self.accent);
            }
            Some(Tool::Stamp) => {
                self.sandbox_grid();
                self.fill(&[(0.35, 0.18), (0.65, 0.18), (0.65, 0.42), (0.35, 0.42)], self.accent.gamma_multiply(0.7));
                self.closed(&[(0.35, 0.18), (0.65, 0.18), (0.65, 0.42), (0.35, 0.42)]);
            }
            Some(Tool::Drape) => {
                self.sandbox_grid();
                self.dashed((0.3, 0.12), (0.3, 0.6));
                self.dashed((0.7, 0.12), (0.7, 0.6));
                self.line_with(&[(0.2, 0.12), (0.8, 0.12)], Stroke::new(2.0_f32, self.accent));
            }
            _ => {
                // Flip Edge: a quad, its diagonal, a turning arrow
                self.closed(&[(0.15, 0.5), (0.5, 0.2), (0.85, 0.5), (0.5, 0.8)]);
                self.line(&[(0.5, 0.2), (0.5, 0.8)]);
                self.line_with(&[(0.15, 0.5), (0.85, 0.5)], Stroke::new(1.5_f32, self.accent));
            }
        }
    }

    /// The Solid Tools: two overlapping squares, what each keeps filled.
    fn solid(&self, t: Tool) {
        let a = [(0.12, 0.3), (0.62, 0.3), (0.62, 0.8), (0.12, 0.8)];
        let b = [(0.38, 0.12), (0.88, 0.12), (0.88, 0.62), (0.38, 0.62)];
        let both = [(0.38, 0.3), (0.62, 0.3), (0.62, 0.62), (0.38, 0.62)];
        let fill = self.accent.gamma_multiply(0.7);
        match t {
            Tool::OuterShell => {
                let outline = [(0.12, 0.3), (0.38, 0.3), (0.38, 0.12), (0.88, 0.12), (0.88, 0.62), (0.62, 0.62), (0.62, 0.8), (0.12, 0.8)];
                self.fill(&a, fill);
                self.fill(&b, fill);
                self.closed(&outline);
                return;
            }
            Tool::SolidUnion => {
                self.fill(&a, fill);
                self.fill(&b, fill);
            }
            Tool::SolidIntersect => self.fill(&both, fill),
            Tool::SolidSubtract | Tool::SolidTrim => {
                // what is left of the second (the back square) once the first is taken out
                self.fill(&[(0.62, 0.12), (0.88, 0.12), (0.88, 0.62), (0.62, 0.62)], fill);
                self.fill(&[(0.38, 0.12), (0.62, 0.12), (0.62, 0.3), (0.38, 0.3)], fill);
                if t == Tool::SolidTrim {
                    // the first stays
                    self.fill(&a, self.accent.gamma_multiply(0.25));
                }
            }
            _ => {
                // Split: the three parts, apart
                self.fill(&both, fill);
                self.line(&[(0.38, 0.3), (0.38, 0.62)]);
                self.line(&[(0.62, 0.3), (0.62, 0.62)]);
            }
        }
        self.closed(&a);
        self.closed(&b);
    }

    fn planned(&self, pl: Planned) {
        match pl {
            Planned::MakeComponent => {
                self.cube(Hidden::None, None);
                self.dashed((0.04, 0.04), (0.96, 0.04));
                self.dashed((0.96, 0.04), (0.96, 0.96));
                self.dashed((0.96, 0.96), (0.04, 0.96));
                self.dashed((0.04, 0.96), (0.04, 0.04));
            }
            Planned::Text => {
                self.closed(&[(0.3, 0.12), (0.92, 0.12), (0.92, 0.52), (0.3, 0.52)]);
                self.line(&[(0.4, 0.26), (0.82, 0.26)]);
                self.line(&[(0.4, 0.38), (0.7, 0.38)]);
                self.line(&[(0.3, 0.52), (0.1, 0.9)]);
            }
            Planned::Axes => {
                let o = (0.36, 0.64);
                for (end, c) in [((0.9, 0.84), Color32::from_rgb(214, 70, 52)), ((0.86, 0.36), Color32::from_rgb(52, 160, 72)), ((0.36, 0.08), Color32::from_rgb(70, 120, 220))] {
                    self.line_with(&[o, end], Stroke::new(self.w / 15.0, if self.dim { c.gamma_multiply(0.35) } else { c }));
                }
            }
            Planned::Text3D => {
                self.line_with(&[(0.16, 0.9), (0.5, 0.1), (0.84, 0.9)], Stroke::new(self.w / 9.0, self.accent));
                self.line_with(&[(0.3, 0.6), (0.7, 0.6)], Stroke::new(self.w / 11.0, self.accent));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Hidden {
    None,
    Solid,
    Dashed,
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
        Icon::FromContours => "From Contours (select contour lines first)".into(),
        Icon::AddDetail => "Add Detail (select ground faces first)".into(),
        Icon::Planned(Planned::MakeComponent) => "Make Component (G)".into(),
        Icon::Planned(pl) => format!("{} (not built yet)", pl.label()),
    }
}

