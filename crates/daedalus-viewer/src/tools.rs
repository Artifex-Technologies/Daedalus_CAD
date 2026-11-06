//! SketchUp-style tools as a state machine over the [`Model`]. No GPU or window
//! code lives here: the viewer resolves the mouse into a [`Cursor`] (ray plus
//! snapped hit) and the tools turn clicks and typed values into model edits.

use daedalus_core::units::{self, Units};
use daedalus_core::sketch::{self, Plane, DEFAULT_ARC_SEGMENTS, DEFAULT_CIRCLE_SEGMENTS};
use daedalus_core::{HalfEdgeMesh, Model};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Rectangle,
    RotatedRectangle,
    Circle,
    Polygon,
    Pie,
    PushPull,
    Move,
    Rotate,
    Scale,
    Eraser,
    TapeMeasure,
    Protractor,
    Orbit,
    Pan,
    Zoom,
    ZoomWindow,
    PositionCamera,
    LookAround,
    Walk,
}

impl Tool {
    pub const ALL: [Tool; 21] = [
        Tool::Select,
        Tool::Line,
        Tool::Rectangle,
        Tool::RotatedRectangle,
        Tool::Circle,
        Tool::Polygon,
        Tool::Pie,
        Tool::PushPull,
        Tool::Move,
        Tool::Rotate,
        Tool::Scale,
        Tool::Eraser,
        Tool::TapeMeasure,
        Tool::Protractor,
        Tool::Orbit,
        Tool::Pan,
        Tool::Zoom,
        Tool::ZoomWindow,
        Tool::PositionCamera,
        Tool::LookAround,
        Tool::Walk,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => "Line",
            Tool::Rectangle => "Rectangle",
            Tool::RotatedRectangle => "Rotated Rectangle",
            Tool::Circle => "Circle",
            Tool::Polygon => "Polygon",
            Tool::Pie => "Pie",
            Tool::PushPull => "Push/Pull",
            Tool::Move => "Move",
            Tool::Rotate => "Rotate",
            Tool::Scale => "Scale",
            Tool::Eraser => "Eraser",
            Tool::TapeMeasure => "Tape Measure",
            Tool::Protractor => "Protractor",
            Tool::Orbit => "Orbit",
            Tool::Pan => "Pan (Hand)",
            Tool::Zoom => "Zoom",
            Tool::ZoomWindow => "Zoom Window",
            Tool::PositionCamera => "Position Camera",
            Tool::LookAround => "Look Around",
            Tool::Walk => "Walk",
        }
    }

    /// SketchUp's default single-key shortcut, if it has one.
    pub fn shortcut(self) -> Option<&'static str> {
        match self {
            Tool::Select => Some("Space"),
            Tool::Line => Some("L"),
            Tool::Rectangle => Some("R"),
            Tool::Circle => Some("C"),
            Tool::PushPull => Some("P"),
            Tool::Move => Some("M"),
            Tool::Rotate => Some("Q"),
            Tool::Scale => Some("S"),
            Tool::Eraser => Some("E"),
            Tool::TapeMeasure => Some("T"),
            Tool::Orbit => Some("O"),
            Tool::Pan => Some("H"),
            Tool::Zoom => Some("Z"),
            Tool::Polygon
            | Tool::RotatedRectangle
            | Tool::Pie
            | Tool::Protractor
            | Tool::ZoomWindow
            | Tool::PositionCamera
            | Tool::LookAround
            | Tool::Walk => None,
        }
    }

    pub fn from_key(key: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.shortcut().is_some_and(|s| s.eq_ignore_ascii_case(key)))
    }

    /// Tools that create or change geometry (disabled on read-only imports).
    pub fn edits(self) -> bool {
        !self.navigates() && !matches!(self, Tool::Select | Tool::TapeMeasure | Tool::Protractor | Tool::PositionCamera)
    }

    /// Tools where a left-drag moves the camera instead of picking.
    pub fn navigates(self) -> bool {
        matches!(self, Tool::Orbit | Tool::Pan | Tool::Zoom | Tool::ZoomWindow | Tool::LookAround | Tool::Walk)
    }

    /// Tools that work on what is selected (or, with nothing selected, on the face under the mouse).
    pub fn transforms(self) -> bool {
        matches!(self, Tool::Move | Tool::Rotate | Tool::Scale)
    }
}

/// What the cursor point was inferred from, shown as a tooltip and marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snap {
    Endpoint,
    Midpoint,
    OnFace,
    /// Locked parallel to an axis: 0 red (X), 1 green (Y), 2 blue (Z), 3 a workplane axis.
    OnAxis(u8),
    InPlane,
    Ground,
    /// On a guide line or at a guide point (Tape Measure, Protractor).
    OnGuide,
    /// The origin, where the three axes meet.
    Origin,
}

/// A construction guide: drawn dashed, snapped to, never part of the model (SketchUp's guide lines and points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    Point([f32; 3]),
    /// Through a point, along a unit direction, endless.
    Line([f32; 3], [f32; 3]),
}

impl Snap {
    pub fn label(self) -> &'static str {
        match self {
            Snap::Endpoint => "Endpoint",
            Snap::Midpoint => "Midpoint",
            Snap::OnFace => "On Face",
            Snap::OnAxis(0) => "On Red Axis",
            Snap::OnAxis(1) => "On Green Axis",
            Snap::OnAxis(2) => "On Blue Axis",
            Snap::OnAxis(_) => "Parallel to Plane Axis",
            Snap::InPlane => "In Plane",
            Snap::Ground => "On Ground",
            Snap::OnGuide => "On Guide",
            Snap::Origin => "Origin",
        }
    }
}

pub const AXIS_COLORS: [[f32; 4]; 4] =
    [[0.85, 0.1, 0.1, 1.0], [0.1, 0.65, 0.1, 1.0], [0.1, 0.2, 0.9, 1.0], [0.7, 0.1, 0.7, 1.0]];
const RUBBER_BAND: [f32; 4] = [0.05, 0.05, 0.05, 1.0];
const PUSH_GUIDE: [f32; 4] = [0.2, 0.4, 0.9, 1.0];
const PROTRACTOR: [f32; 4] = [0.3, 0.3, 0.75, 1.0];

/// The mouse resolved by the viewer: hits on geometry, snaps, or the ground.
#[derive(Debug, Clone, Copy)]
pub struct Hit {
    pub point: [f32; 3],
    /// Plane a drawing started here would use (the face's plane, or the ground).
    pub plane: Plane,
    pub face: Option<usize>,
    /// A loose line close to the mouse (for the Eraser).
    pub line: Option<usize>,
    pub snap: Snap,
}

#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
    pub hit: Option<Hit>,
}

/// A value typed into the Measurements box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Entry {
    Value(f32),
    Pair(f32, f32),
    Sides(u32),
}

/// Parses `2.5`, `3,2` (or `3;2`) and `24s`. Lengths are in `units` unless a unit is written (`300mm`, `4' 6"`); they come
/// back in metres, the model's unit.
pub fn parse_entry(text: &str, units: Units) -> Option<Entry> {
    let t = text.trim();
    if let Some(n) = t.strip_suffix(['s', 'S']) {
        if let Ok(n) = n.trim().parse::<u32>() {
            return (n >= 3).then_some(Entry::Sides(n));
        }
    }
    if let Some((a, b)) = t.split_once([',', ';']) {
        return Some(Entry::Pair(units::parse(a, units)?, units::parse(b, units)?));
    }
    units::parse(t, units).map(Entry::Value)
}

#[derive(Debug, Clone)]
enum Op {
    Idle,
    /// Line: each click adds a loose line from `last` to the new point (in the model at once, so Esc keeps what is drawn).
    Line { plane: Plane, last: [f32; 3], cursor: Option<[f32; 3]>, axis: Option<u8> },
    Rect { plane: Plane, start: [f32; 3], cursor: Option<[f32; 3]> },
    Round { plane: Plane, center: [f32; 3], cursor: Option<[f32; 3]> },
    Push { face: usize, base: [f32; 3], normal: [f32; 3], distance: f32 },
    /// Rotated Rectangle: the first side from `a` to `b` (once set), then the height toward the cursor.
    RotRect { plane: Plane, a: [f32; 3], b: Option<[f32; 3]>, cursor: Option<[f32; 3]> },
    /// Pie: the centre, the start of the arc (once set, it gives the radius), then the angle.
    Pie { plane: Plane, center: [f32; 3], start: Option<[f32; 3]>, cursor: Option<[f32; 3]> },
    /// Move: these vertices go from `base` to the cursor.
    Move { verts: Vec<u32>, base: [f32; 3], cursor: Option<[f32; 3]>, axis: Option<u8> },
    /// Rotate: about the plane's normal through `center`, from the `start` direction (once set) by `angle` radians.
    Rotate { verts: Vec<u32>, plane: Plane, center: [f32; 3], start: Option<[f32; 3]>, angle: f32 },
    /// Scale: uniformly about `anchor` (the corner of the bounding box opposite the grip), `start` being the grip.
    Scale { verts: Vec<u32>, anchor: [f32; 3], start: [f32; 3], factor: f32 },
    /// Tape Measure: from `start` to the cursor.
    Tape { start: [f32; 3], cursor: Option<[f32; 3]> },
    /// Protractor: a guide through `center` turned `angle` radians from the `start` direction (once set).
    Protract { plane: Plane, center: [f32; 3], start: Option<[f32; 3]>, angle: f32 },
}

pub struct Tools {
    pub tool: Tool,
    pub circle_segments: u32,
    pub polygon_sides: u32,
    /// What lengths are shown and typed in (the model itself is in metres).
    pub units: Units,
    /// Selected faces, sorted.
    pub selected: Vec<usize>,
    /// Segments in a pie's arc.
    pub arc_segments: u32,
    /// Construction guides from Tape Measure and Protractor.
    pub guides: Vec<Guide>,
    hover_face: Option<usize>,
    op: Op,
    /// Last push/pull (original face, distance), so a value typed right after
    /// it re-does it at that distance, as in SketchUp.
    last_push: Option<(usize, f32)>,
}

impl Default for Tools {
    fn default() -> Self {
        Tools {
            tool: Tool::Select,
            circle_segments: DEFAULT_CIRCLE_SEGMENTS,
            polygon_sides: 6,
            units: Units::Metres,
            selected: Vec::new(),
            arc_segments: DEFAULT_ARC_SEGMENTS,
            guides: Vec::new(),
            hover_face: None,
            op: Op::Idle,
            last_push: None,
        }
    }
}

impl Tools {
    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.op = Op::Idle;
        self.hover_face = None;
        self.last_push = None;
    }

    /// Esc: abandons the current operation (or clears the selection).
    pub fn cancel(&mut self) {
        if matches!(self.op, Op::Idle) {
            self.selected.clear();
        }
        self.op = Op::Idle;
    }

    /// Call after anything outside the tools changed the mesh (undo, repair):
    /// face indices may no longer mean the same faces.
    pub fn model_changed(&mut self) {
        self.op = Op::Idle;
        self.selected.clear();
        self.hover_face = None;
        self.last_push = None;
    }

    /// Select tool click: `face` replaces the selection; with `add` (Ctrl) it is added, with `toggle` (Shift) it goes in
    /// or out; `connected` (a triple-click) takes every face joined to it. Clicking empty space clears the selection.
    pub fn select(&mut self, model: &Model, face: Option<usize>, add: bool, toggle: bool, connected: bool) {
        let Some(face) = face else {
            if !add && !toggle {
                self.selected.clear();
            }
            return;
        };
        let faces = if connected { connected_faces(model.mesh(), face) } else { vec![face] };
        if toggle && !connected {
            match self.selected.binary_search(&face) {
                Ok(i) => {
                    self.selected.remove(i);
                }
                Err(i) => self.selected.insert(i, face),
            }
            return;
        }
        if !add && !toggle {
            self.selected.clear();
        }
        self.selected.extend(faces);
        self.selected.sort_unstable();
        self.selected.dedup();
    }

    /// Ctrl+A.
    pub fn select_all(&mut self, model: &Model) {
        self.selected = (0..model.mesh().face_count()).collect();
    }

    /// The vertices Move, Rotate and Scale work on: the selection's, else the face under the mouse.
    fn target_verts(&self, model: &Model, c: &Cursor) -> Option<Vec<u32>> {
        let faces: Vec<usize> = if self.selected.is_empty() { c.hit.and_then(|h| h.face).into_iter().collect() } else { self.selected.clone() };
        let mut v: Vec<u32> = faces.iter().filter(|&&f| f < model.mesh().face_count()).flat_map(|&f| model.mesh().face_vertices(f)).collect();
        v.sort_unstable();
        v.dedup();
        (!v.is_empty()).then_some(v)
    }

    /// Ends a Move, Rotate or Scale: the faces keep their numbers, so the selection stays.
    fn transform(&mut self, model: &mut Model, verts: &[u32], f: impl Fn([f32; 3]) -> [f32; 3]) -> Result<(), String> {
        model.transform_vertices(verts, f).map_err(|e| e.to_string())?;
        self.op = Op::Idle;
        Ok(())
    }

    /// Delete Guides (Edit menu).
    pub fn delete_guides(&mut self) {
        self.guides.clear();
    }

    #[cfg(test)]
    fn is_busy(&self) -> bool {
        !matches!(self.op, Op::Idle)
    }

    /// Points that are not in the model yet but should still be snap targets
    /// (the Line tool's own corners, so the loop can be closed).
    pub fn pending_points(&self) -> Vec<[f32; 3]> {
        match &self.op {
            Op::Line { last, .. } => vec![*last],
            Op::Rect { start, .. } => vec![*start],
            Op::Round { center, .. } | Op::Pie { center, .. } => vec![*center],
            Op::RotRect { a, b, .. } => std::iter::once(*a).chain(*b).collect(),
            Op::Move { base, .. } => vec![*base],
            Op::Tape { start, .. } => vec![*start],
            _ => Vec::new(),
        }
    }

    /// Faces to draw highlighted: the selection, or the face a click would act on.
    pub fn highlight(&self) -> Vec<usize> {
        match (self.tool, &self.op) {
            (Tool::PushPull | Tool::Eraser, Op::Idle) => self.hover_face.into_iter().collect(),
            (Tool::PushPull, _) => Vec::new(),
            (t, Op::Idle) if t.transforms() && self.selected.is_empty() => self.hover_face.into_iter().collect(),
            _ => self.selected.clone(),
        }
    }

    /// Cursor position and snap after the tool's own inference (axis locks,
    /// keeping to the workplane), for the marker and tooltip.
    pub fn marker(&self, c: &Cursor) -> Option<([f32; 3], Snap)> {
        match &self.op {
            Op::Line { plane, last, .. } => {
                // a snap wins; then the red, green or blue axis through the last point; then the face or ground under the
                // mouse, locked onto an axis of its plane when close to one
                if let Some(h) = c.hit.filter(|h| matches!(h.snap, Snap::Endpoint | Snap::Midpoint | Snap::OnGuide | Snap::Origin)) {
                    return Some((h.point, h.snap));
                }
                if let Some((p, a)) = ray_axis_lock(*last, c) {
                    return Some((p, Snap::OnAxis(a)));
                }
                let (p, snap) = match c.hit {
                    Some(h) => (h.point, h.snap),
                    None => on_plane(&Plane { origin: *last, ..*plane }, c)?,
                };
                let level = Plane { origin: *last, ..Plane::from_point_normal(*last, plane.normal).unwrap_or(*plane) };
                if dot(sub(p, *last), level.normal).abs() <= 1e-4 * (1.0 + len(p)) {
                    let (q, axis) = axis_lock(&level, *last, p);
                    return Some((q, axis.map_or(snap, Snap::OnAxis)));
                }
                Some((p, snap))
            }
            Op::Rect { plane, .. } | Op::Round { plane, .. } | Op::RotRect { plane, .. } | Op::Pie { plane, .. } => on_plane(plane, c),
            Op::Rotate { plane, .. } | Op::Protract { plane, .. } => on_plane(plane, c),
            Op::Move { base, .. } | Op::Tape { start: base, .. } => {
                // a snap wins; else the mouse locks onto an axis through the start point when it is close to one
                match c.hit {
                    Some(h) if matches!(h.snap, Snap::Endpoint | Snap::Midpoint | Snap::OnGuide) => Some((h.point, h.snap)),
                    _ => ray_axis_lock(*base, c).map(|(p, a)| (p, Snap::OnAxis(a))).or(c.hit.map(|h| (h.point, h.snap))),
                }
            }
            Op::Push { .. } | Op::Scale { .. } => None,
            Op::Idle => c.hit.map(|h| (h.point, h.snap)),
        }
    }

    /// Mouse moved: updates the rubber band or the push/pull distance.
    pub fn hover(&mut self, c: &Cursor) {
        let marker = self.marker(c);
        self.hover_face = c.hit.and_then(|h| h.face);
        match &mut self.op {
            Op::Line { cursor, axis, .. } => {
                *cursor = marker.map(|m| m.0);
                *axis = match marker {
                    Some((_, Snap::OnAxis(a))) => Some(a),
                    _ => None,
                };
            }
            Op::Rect { cursor, .. } | Op::Round { cursor, .. } | Op::RotRect { cursor, .. } | Op::Pie { cursor, .. } => {
                *cursor = marker.map(|m| m.0)
            }
            Op::Tape { cursor, .. } => *cursor = marker.map(|m| m.0),
            Op::Move { cursor, axis, .. } => {
                *cursor = marker.map(|m| m.0);
                *axis = match marker {
                    Some((_, Snap::OnAxis(a))) => Some(a),
                    _ => None,
                };
            }
            Op::Rotate { plane, center, start: Some(start), angle, .. } | Op::Protract { plane, center, start: Some(start), angle } => {
                if let Some(a) = marker.and_then(|(p, _)| sketch::angle_between(plane, *center, *start, p)) {
                    *angle = snap_angle(a);
                }
            }
            Op::Scale { anchor, start, factor, .. } => {
                let axis = sub(*start, *anchor);
                let l = len(axis);
                if let (Some(dir), true) = (normalize(axis), l > 1e-9) {
                    if let Some(t) = closest_on_line(*anchor, dir, c.origin, c.dir) {
                        if t > 1e-6 {
                            *factor = t / l;
                        }
                    }
                }
            }
            Op::Push { base, normal, distance, .. } => {
                if let Some(d) = closest_on_line(*base, *normal, c.origin, c.dir) {
                    *distance = d;
                }
            }
            Op::Rotate { .. } | Op::Protract { .. } | Op::Idle => {}
        }
    }

    /// Left click. Returns a status message for errors the user should see.
    pub fn click(&mut self, model: &mut Model, c: &Cursor) -> Result<(), String> {
        self.hover(c);
        let marker = self.marker(c);
        match (self.tool, std::mem::replace(&mut self.op, Op::Idle)) {
            (Tool::Select, _) => self.select(model, c.hit.and_then(|h| h.face), false, false, false),
            (Tool::RotatedRectangle, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::RotRect { plane: h.plane, a: h.point, b: None, cursor: None };
                }
            }
            (Tool::RotatedRectangle, Op::RotRect { plane, a, b, .. }) => match (b, marker) {
                (None, Some((p, _))) if dist(a, p) > 1e-6 => self.op = Op::RotRect { plane, a, b: Some(p), cursor: None },
                (Some(b), Some((p, _))) => self.commit(model, sketch::rotated_rectangle(&plane, a, b, p))?,
                (b, _) => self.op = Op::RotRect { plane, a, b, cursor: None },
            },
            (Tool::Pie, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Pie { plane: h.plane, center: h.point, start: None, cursor: None };
                }
            }
            (Tool::Pie, Op::Pie { plane, center, start, .. }) => match (start, marker) {
                (None, Some((p, _))) if dist(center, p) > 1e-6 => self.op = Op::Pie { plane, center, start: Some(p), cursor: None },
                (Some(st), Some((p, _))) => {
                    let sweep = sketch::angle_between(&plane, center, st, p).map(snap_angle).unwrap_or(0.0);
                    self.commit(model, sketch::pie(&plane, center, st, sweep, self.arc_segments))?
                }
                (start, _) => self.op = Op::Pie { plane, center, start, cursor: None },
            },
            (Tool::Move, Op::Idle) => {
                let verts = self.target_verts(model, c).ok_or("select something to move, or point at a face")?;
                if let Some(h) = c.hit {
                    self.op = Op::Move { verts, base: h.point, cursor: None, axis: None };
                }
            }
            (Tool::Move, Op::Move { verts, base, cursor, axis }) => match marker {
                Some((p, _)) => {
                    let d = sub(p, base);
                    self.transform(model, &verts, |q| add(q, d))?;
                }
                None => self.op = Op::Move { verts, base, cursor, axis },
            },
            (Tool::Rotate, Op::Idle) => {
                let verts = self.target_verts(model, c).ok_or("select something to rotate, or point at a face")?;
                if let Some(h) = c.hit {
                    self.op = Op::Rotate { verts, plane: Plane { origin: h.point, ..h.plane }, center: h.point, start: None, angle: 0.0 };
                }
            }
            (Tool::Rotate, Op::Rotate { verts, plane, center, start, angle }) => match (start, marker) {
                (None, Some((p, _))) if dist(center, p) > 1e-6 => self.op = Op::Rotate { verts, plane, center, start: Some(p), angle: 0.0 },
                (Some(_), _) => {
                    let n = plane.normal;
                    self.transform(model, &verts, |q| rotate_about(q, center, n, angle))?;
                }
                (start, _) => self.op = Op::Rotate { verts, plane, center, start, angle },
            },
            (Tool::Scale, Op::Idle) => {
                let verts = self.target_verts(model, c).ok_or("select something to scale, or point at a face")?;
                if let Some(h) = c.hit {
                    let anchor = opposite_corner(model, &verts, h.point);
                    if dist(anchor, h.point) < 1e-6 {
                        return Err("pick a point away from the middle of what is scaled".into());
                    }
                    self.op = Op::Scale { verts, anchor, start: h.point, factor: 1.0 };
                }
            }
            (Tool::Scale, Op::Scale { verts, anchor, factor, .. }) => self.transform(model, &verts, |q| scale_about(q, anchor, factor))?,
            (Tool::TapeMeasure, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Tape { start: h.point, cursor: None };
                }
            }
            (Tool::TapeMeasure, Op::Tape { start, cursor }) => match marker {
                Some((p, _)) => self.add_tape_guides(start, p),
                None => self.op = Op::Tape { start, cursor },
            },
            (Tool::Protractor, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Protract { plane: Plane { origin: h.point, ..h.plane }, center: h.point, start: None, angle: 0.0 };
                }
            }
            (Tool::Protractor, Op::Protract { plane, center, start, angle }) => match (start, marker) {
                (None, Some((p, _))) if dist(center, p) > 1e-6 => self.op = Op::Protract { plane, center, start: Some(p), angle: 0.0 },
                (Some(st), _) => self.add_protractor_guide(&plane, center, st, angle),
                (start, _) => self.op = Op::Protract { plane, center, start, angle },
            },
            // the Eraser takes a line under the mouse before the face behind it
            (Tool::Eraser, _) if c.hit.and_then(|h| h.line).is_some() => {
                model.erase_line(c.hit.and_then(|h| h.line).expect("checked")).map_err(|e| e.to_string())?;
            }
            (Tool::Eraser, _) => {
                if let Some(face) = c.hit.and_then(|h| h.face) {
                    model.erase_face(face).map_err(|e| e.to_string())?;
                    self.model_changed();
                }
            }
            (Tool::Line, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Line { plane: h.plane, last: h.point, cursor: None, axis: None };
                }
            }
            (Tool::Line, Op::Line { plane, last, .. }) => match marker {
                Some((p, _)) => self.add_line_point(model, plane, last, p)?,
                None => self.op = Op::Line { plane, last, cursor: None, axis: None },
            },
            (Tool::Rectangle, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Rect { plane: h.plane, start: h.point, cursor: None };
                }
            }
            (Tool::Rectangle, Op::Rect { plane, start, .. }) => match marker {
                Some((p, _)) => self.commit(model, sketch::rectangle(&plane, start, p))?,
                None => self.op = Op::Rect { plane, start, cursor: None },
            },
            (Tool::Circle | Tool::Polygon, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Round { plane: h.plane, center: h.point, cursor: None };
                }
            }
            (Tool::Circle | Tool::Polygon, Op::Round { plane, center, .. }) => match marker {
                Some((p, _)) => self.commit(model, sketch::regular_polygon(&plane, center, p, self.sides()))?,
                None => self.op = Op::Round { plane, center, cursor: None },
            },
            (Tool::PushPull, Op::Idle) => {
                if let Some(Hit { face: Some(face), point, .. }) = c.hit {
                    let normal = model.mesh().face_normal(face);
                    if normal != [0.0; 3] {
                        self.op = Op::Push { face, base: point, normal, distance: 0.0 };
                    }
                }
            }
            (Tool::PushPull, Op::Push { face, distance, .. }) => self.push(model, face, distance)?,
            (_, op) => self.op = op,
        }
        Ok(())
    }

    /// Enter in the Measurements box.
    pub fn type_entry(&mut self, model: &mut Model, text: &str) -> Result<(), String> {
        // angles (Rotate, Protractor, a pie's sweep) and scale factors are plain numbers, not lengths
        if let Some(r) = self.type_number(model, text) {
            return r;
        }
        let entry = parse_entry(text, self.units).ok_or_else(|| format!("cannot read \"{text}\""))?;
        match (entry, std::mem::replace(&mut self.op, Op::Idle)) {
            (Entry::Sides(n), op) if matches!(self.tool, Tool::Circle | Tool::Polygon) => {
                match self.tool {
                    Tool::Circle => self.circle_segments = n,
                    _ => self.polygon_sides = n,
                }
                self.op = op;
            }
            (Entry::Value(len), Op::Line { plane, last, cursor, axis }) => {
                let dir = cursor.and_then(|c| normalize(sub(c, last)));
                let Some(dir) = dir else {
                    self.op = Op::Line { plane, last, cursor, axis };
                    return Err("move the mouse to show the direction first".into());
                };
                self.add_line_point(model, plane, last, add(last, scale(dir, len)))?;
            }
            (Entry::Pair(w, h), Op::Rect { plane, start, cursor }) => {
                // Typed sizes follow the side of the start point the mouse is on.
                let (sx, sy) = match cursor {
                    Some(c) => {
                        let (a, b) = (plane.to_local(start), plane.to_local(c));
                        (sign(b[0] - a[0]), sign(b[1] - a[1]))
                    }
                    None => (1.0, 1.0),
                };
                let a = plane.to_local(start);
                let corner = plane.to_world(a[0] + sx * w.abs(), a[1] + sy * h.abs());
                self.commit(model, sketch::rectangle(&plane, start, corner))?;
            }
            (Entry::Value(r), Op::Round { plane, center, cursor }) => {
                let dir = cursor.and_then(|c| normalize(sub(c, center))).unwrap_or(plane.u);
                let rim = add(center, scale(dir, r.abs()));
                self.commit(model, sketch::regular_polygon(&plane, center, rim, self.sides()))?;
            }
            (Entry::Value(d), Op::Push { face, .. }) => self.push(model, face, d)?,
            (Entry::Sides(n), op @ Op::Pie { .. }) => {
                self.arc_segments = n;
                self.op = op;
            }
            (Entry::Sides(n), Op::Idle) if self.tool == Tool::Pie => self.arc_segments = n,
            (Entry::Value(r), Op::Pie { plane, center, start: None, cursor }) => {
                let dir = cursor.and_then(|c| normalize(sub(c, center))).unwrap_or(plane.u);
                self.op = Op::Pie { plane, center, start: Some(add(center, scale(dir, r.abs()))), cursor };
            }
            (Entry::Value(l), Op::RotRect { plane, a, b: None, cursor }) => {
                let dir = cursor.and_then(|c| normalize(sub(c, a))).unwrap_or(plane.u);
                self.op = Op::RotRect { plane, a, b: Some(add(a, scale(dir, l.abs()))), cursor };
            }
            (Entry::Value(h), Op::RotRect { plane, a, b: Some(b), cursor }) => {
                // the height goes to the side of the first edge the mouse is on
                let along = normalize(sub(b, a)).ok_or("the first side has no length")?;
                let mut side = cross(plane.normal, along);
                if let Some(c) = cursor {
                    if dot(sub(c, a), side) < 0.0 {
                        side = scale(side, -1.0);
                    }
                }
                self.commit(model, sketch::rotated_rectangle(&plane, a, b, add(a, scale(side, h.abs()))))?;
            }
            (Entry::Value(d), Op::Move { verts, base, cursor, .. }) => {
                let dir = cursor.and_then(|c| normalize(sub(c, base))).ok_or("move the mouse to show the direction first")?;
                let v = scale(dir, d);
                self.transform(model, &verts, |q| add(q, v))?;
            }
            (Entry::Value(d), Op::Tape { start, cursor }) => {
                let dir = cursor.and_then(|c| normalize(sub(c, start))).ok_or("move the mouse to show the direction first")?;
                self.add_tape_guides(start, add(start, scale(dir, d)));
            }
            (Entry::Value(d), Op::Idle) if self.tool == Tool::PushPull && self.last_push.is_some() => {
                let (face, _) = self.last_push.expect("checked");
                model.undo();
                self.push(model, face, d)?;
            }
            (_, op) => {
                self.op = op;
                return Err(format!("\"{text}\" does not apply here"));
            }
        }
        Ok(())
    }

    /// A plain number typed while it means an angle in degrees (Rotate, Protractor, a pie's sweep) or a scale factor.
    /// None when the text is not that, so it is read as a length.
    fn type_number(&mut self, model: &mut Model, text: &str) -> Option<Result<(), String>> {
        let t = text.trim().trim_end_matches('°');
        let n: f32 = t.parse().ok().filter(|n: &f32| n.is_finite())?;
        let rad = n.to_radians();
        Some(match std::mem::replace(&mut self.op, Op::Idle) {
            Op::Rotate { verts, plane, center, start: Some(_), .. } => self.transform(model, &verts, |q| rotate_about(q, center, plane.normal, rad)),
            Op::Protract { plane, center, start: Some(st), .. } => {
                self.add_protractor_guide(&plane, center, st, rad);
                Ok(())
            }
            Op::Pie { plane, center, start: Some(st), .. } => self.commit(model, sketch::pie(&plane, center, st, rad, self.arc_segments)),
            Op::Scale { verts, anchor, .. } if n != 0.0 => self.transform(model, &verts, |q| scale_about(q, anchor, n)),
            Op::Scale { .. } => Err("a scale of 0 would flatten it to a point".into()),
            op => {
                self.op = op;
                return None;
            }
        })
    }

    fn add_tape_guides(&mut self, start: [f32; 3], end: [f32; 3]) {
        if let Some(dir) = normalize(sub(end, start)) {
            self.guides.push(Guide::Line(start, dir));
            self.guides.push(Guide::Point(end));
        }
        self.op = Op::Idle;
    }

    fn add_protractor_guide(&mut self, plane: &Plane, center: [f32; 3], start: [f32; 3], angle: f32) {
        if let Some(dir) = normalize(sub(start, center)) {
            self.guides.push(Guide::Line(center, rotate_about(dir, [0.0; 3], plane.normal, angle)));
        }
        self.op = Op::Idle;
    }

    /// Mesh to draw instead of the model while push/pull, Move, Rotate or Scale is live.
    pub fn preview_mesh(&self, model: &Model) -> Option<HalfEdgeMesh> {
        let moved = |verts: &[u32], f: &dyn Fn([f32; 3]) -> [f32; 3]| {
            let mut m = model.mesh().clone();
            for &v in verts {
                if let Some(p) = m.positions.get_mut(v as usize) {
                    *p = f(*p);
                }
            }
            Some(m)
        };
        match &self.op {
            Op::Push { face, distance, .. } if *distance != 0.0 => {
                let mut m = model.mesh().clone();
                m.push_pull(*face, *distance).ok()?;
                Some(m)
            }
            Op::Move { verts, base, cursor: Some(c), .. } => {
                let d = sub(*c, *base);
                moved(verts, &|q| add(q, d))
            }
            Op::Rotate { verts, plane, center, start: Some(_), angle } if *angle != 0.0 => {
                moved(verts, &|q| rotate_about(q, *center, plane.normal, *angle))
            }
            Op::Scale { verts, anchor, factor, .. } if *factor != 1.0 => moved(verts, &|q| scale_about(q, *anchor, *factor)),
            _ => None,
        }
    }

    /// Guides to draw: the ones placed, plus the one Tape Measure or Protractor would place now.
    pub fn guide_preview(&self) -> Option<Guide> {
        match &self.op {
            Op::Protract { plane, center, start: Some(st), angle } => {
                normalize(sub(*st, *center)).map(|d| Guide::Line(*center, rotate_about(d, [0.0; 3], plane.normal, *angle)))
            }
            _ => None,
        }
    }

    /// Rubber-band lines drawn over the scene.
    pub fn preview_lines(&self) -> Vec<([f32; 3], [f32; 3], [f32; 4])> {
        let mut out = Vec::new();
        match &self.op {
            Op::Line { last, cursor: Some(c), axis, .. } => {
                out.push((*last, *c, axis.map_or(RUBBER_BAND, |a| AXIS_COLORS[a as usize])));
            }
            Op::Rect { plane, start, cursor: Some(c) } => {
                if let Some(r) = sketch::rectangle(plane, *start, *c) {
                    ring_lines(&r, &mut out);
                }
            }
            Op::Round { plane, center, cursor: Some(c) } => {
                out.push((*center, *c, RUBBER_BAND));
                if let Some(r) = sketch::regular_polygon(plane, *center, *c, self.sides()) {
                    ring_lines(&r, &mut out);
                }
            }
            Op::Push { base, normal, distance, .. } => {
                out.push((*base, add(*base, scale(*normal, *distance)), PUSH_GUIDE));
            }
            Op::RotRect { plane, a, b, cursor: Some(c) } => match b {
                None => out.push((*a, *c, RUBBER_BAND)),
                Some(b) => {
                    out.push((*a, *b, RUBBER_BAND));
                    if let Some(r) = sketch::rotated_rectangle(plane, *a, *b, *c) {
                        ring_lines(&r, &mut out);
                    }
                }
            },
            Op::Pie { plane, center, start, cursor: Some(c) } => match start {
                None => out.push((*center, *c, RUBBER_BAND)),
                Some(st) => {
                    let sweep = sketch::angle_between(plane, *center, *st, *c).map(snap_angle).unwrap_or(0.0);
                    match sketch::pie(plane, *center, *st, sweep, self.arc_segments) {
                        Some(r) => ring_lines(&r, &mut out),
                        None => out.push((*center, *st, RUBBER_BAND)),
                    }
                }
            },
            Op::Move { base, cursor: Some(c), axis, .. } => {
                out.push((*base, *c, axis.map_or(PUSH_GUIDE, |a| AXIS_COLORS[a as usize])));
            }
            Op::Tape { start, cursor: Some(c) } => out.push((*start, *c, PUSH_GUIDE)),
            Op::Rotate { plane, center, start, angle, .. } | Op::Protract { plane, center, start, angle } => {
                // the protractor: a circle in the plane, the start direction and the turned one
                let r = start.map(|s| dist(s, *center)).unwrap_or(0.25).max(0.05);
                let ring: Vec<[f32; 3]> = (0..36)
                    .map(|i| {
                        let a = std::f32::consts::TAU * i as f32 / 36.0;
                        add(*center, add(scale(plane.u, r * a.cos()), scale(plane.v, r * a.sin())))
                    })
                    .collect();
                for i in 0..ring.len() {
                    out.push((ring[i], ring[(i + 1) % ring.len()], PROTRACTOR));
                }
                if let Some(st) = start {
                    out.push((*center, *st, RUBBER_BAND));
                    if let Some(d) = normalize(sub(*st, *center)) {
                        out.push((*center, add(*center, scale(rotate_about(d, [0.0; 3], plane.normal, *angle), r)), PUSH_GUIDE));
                    }
                }
            }
            Op::Scale { anchor, start, factor, .. } => {
                out.push((*anchor, add(*anchor, scale(sub(*start, *anchor), *factor)), PUSH_GUIDE));
            }
            _ => {}
        }
        out
    }

    /// SketchUp-style hint for the status bar.
    pub fn status(&self) -> &'static str {
        match (self.tool, &self.op) {
            (Tool::Select, _) => "Click a face to select it. Ctrl adds, Shift toggles, a triple-click takes everything joined to it.",
            (Tool::RotatedRectangle, Op::Idle) => "Select first corner.",
            (Tool::RotatedRectangle, Op::RotRect { b: None, .. }) => "Select second corner, or type a length. The first side may point any way.",
            (Tool::RotatedRectangle, _) => "Select the height, or type it.",
            (Tool::Pie, Op::Idle) => "Select center point. Type a number followed by s to set segments.",
            (Tool::Pie, Op::Pie { start: None, .. }) => "Select start of the arc, or type a radius.",
            (Tool::Pie, _) => "Select end of the arc, or type an angle in degrees.",
            (Tool::Move, Op::Idle) => "Pick a point to move from. Moves the selection, or the face under the mouse.",
            (Tool::Move, _) => "Pick the point to move to, or type a distance. Close to an axis it locks onto it.",
            (Tool::Rotate, Op::Idle) => "Pick the center of rotation. Turns the selection, or the face under the mouse.",
            (Tool::Rotate, Op::Rotate { start: None, .. }) => "Pick a point to turn from.",
            (Tool::Rotate, _) => "Turn to the angle and click, or type degrees. It snaps every 15 degrees.",
            (Tool::Scale, Op::Idle) => "Pick a grip: a corner or edge point. It scales from the opposite corner.",
            (Tool::Scale, _) => "Move to scale and click, or type a factor (2 doubles it) or a length.",
            (Tool::TapeMeasure, Op::Idle) => "Pick a point to measure from.",
            (Tool::TapeMeasure, _) => "Pick the point to measure to, or type a length. Leaves a guide line and a guide point.",
            (Tool::Protractor, Op::Idle) => "Pick the protractor's center.",
            (Tool::Protractor, Op::Protract { start: None, .. }) => "Pick a point to measure the angle from.",
            (Tool::Protractor, _) => "Turn to the angle and click, or type degrees. Leaves a guide line.",
            (Tool::ZoomWindow, _) => "Drag a rectangle around what to zoom in on.",
            (Tool::PositionCamera, _) => "Click where to stand. The eye goes 1.68 m above it.",
            (Tool::LookAround, _) => "Drag to look around. The eye stays where it is.",
            (Tool::Walk, _) => "Drag up or down to walk, left or right to turn. Arrow keys work too.",
            (Tool::Line, Op::Idle) => "Select start point.",
            (Tool::Line, _) => "Select end point, or type a length. Esc stops; what is drawn stays. A closed outline becomes a face.",
            (Tool::Rectangle, Op::Idle) => "Select first corner.",
            (Tool::Rectangle, _) => "Select opposite corner, or type width,height.",
            (Tool::Circle, Op::Idle) => "Select center point. Type a number followed by s to set segments.",
            (Tool::Polygon, Op::Idle) => "Select center point. Type a number followed by s to set sides.",
            (Tool::Circle | Tool::Polygon, _) => "Select radius point, or type a radius.",
            (Tool::PushPull, Op::Idle) => "Pick a face to push or pull.",
            (Tool::PushPull, _) => "Move to set the distance; click or type a value.",
            (Tool::Eraser, _) => "Click a face to erase it.",
            (Tool::Orbit, _) => "Drag to orbit.",
            (Tool::Pan, _) => "Drag to pan.",
            (Tool::Zoom, _) => "Drag up or down to zoom.",
        }
    }

    /// Label and current value for the Measurements box.
    pub fn measurement(&self) -> (&'static str, String) {
        match &self.op {
            Op::Line { last, cursor: Some(c), .. } => ("Length", fmt(dist(*last, *c), self.units)),
            Op::Line { .. } => ("Length", String::new()),
            Op::Rect { plane, start, cursor: Some(c) } => {
                let (a, b) = (plane.to_local(*start), plane.to_local(*c));
                ("Dimensions", format!("{}, {}", fmt((b[0] - a[0]).abs(), self.units), fmt((b[1] - a[1]).abs(), self.units)))
            }
            Op::Rect { .. } => ("Dimensions", String::new()),
            Op::Round { center, cursor, .. } => ("Radius", cursor.map(|c| fmt(dist(*center, c), self.units)).unwrap_or_default()),
            Op::Push { distance, .. } => ("Distance", fmt(*distance, self.units)),
            Op::RotRect { a, b: None, cursor, .. } => ("Length", cursor.map(|c| fmt(dist(*a, c), self.units)).unwrap_or_default()),
            Op::RotRect { plane, a, b: Some(b), cursor } => (
                "Height",
                cursor
                    .and_then(|c| normalize(sub(*b, *a)).map(|d| dot(sub(c, *a), cross(plane.normal, d)).abs()))
                    .map(|h| fmt(h, self.units))
                    .unwrap_or_default(),
            ),
            Op::Pie { center, start: None, cursor, .. } => ("Radius", cursor.map(|c| fmt(dist(*center, c), self.units)).unwrap_or_default()),
            Op::Pie { plane, center, start: Some(st), cursor } => (
                "Angle",
                cursor
                    .and_then(|c| sketch::angle_between(plane, *center, *st, c))
                    .map(|a| format!("{:.1}°", snap_angle(a).to_degrees()))
                    .unwrap_or_default(),
            ),
            Op::Move { base, cursor, .. } => ("Length", cursor.map(|c| fmt(dist(*base, c), self.units)).unwrap_or_default()),
            Op::Tape { start, cursor } => ("Length", cursor.map(|c| fmt(dist(*start, c), self.units)).unwrap_or_default()),
            Op::Rotate { angle, .. } | Op::Protract { angle, .. } => ("Angle", format!("{:.1}°", angle.to_degrees())),
            Op::Scale { factor, .. } => ("Scale", format!("{factor:.3}")),
            Op::Idle => match self.tool {
                Tool::Circle => ("Segments", self.circle_segments.to_string()),
                Tool::Polygon => ("Sides", self.polygon_sides.to_string()),
                Tool::Pie => ("Segments", self.arc_segments.to_string()),
                Tool::PushPull => ("Distance", self.last_push.map(|(_, d)| fmt(d, self.units)).unwrap_or_default()),
                _ => ("Measurements", String::new()),
            },
        }
    }

    fn sides(&self) -> u32 {
        if self.tool == Tool::Circle {
            self.circle_segments
        } else {
            self.polygon_sides
        }
    }

    /// Draws the line from `last` to `p` into the model and goes on from `p`; when it closes a face the chain ends, as
    /// in SketchUp.
    fn add_line_point(&mut self, model: &mut Model, plane: Plane, last: [f32; 3], p: [f32; 3]) -> Result<(), String> {
        if dist(last, p) <= 1e-6 * (1.0 + len(p)) {
            self.op = Op::Line { plane, last, cursor: None, axis: None };
            return Ok(());
        }
        match model.add_line(last, p) {
            Ok(Some(_)) => self.op = Op::Idle,
            Ok(None) => self.op = Op::Line { plane, last: p, cursor: None, axis: None },
            Err(e) => {
                self.op = Op::Line { plane, last, cursor: None, axis: None };
                return Err(e.to_string());
            }
        }
        Ok(())
    }

    fn commit(&mut self, model: &mut Model, outline: Option<Vec<[f32; 3]>>) -> Result<(), String> {
        let outline = outline.ok_or("the shape has no area")?;
        model.add_polygon(&outline).map_err(|e| e.to_string())?;
        self.model_changed();
        Ok(())
    }

    fn push(&mut self, model: &mut Model, face: usize, distance: f32) -> Result<(), String> {
        if distance == 0.0 {
            return Ok(());
        }
        model.push_pull(face, distance).map_err(|e| e.to_string())?;
        self.model_changed();
        self.last_push = Some((face, distance));
        Ok(())
    }
}

/// The cursor on a workplane: a snap that lies on it, else the ray's crossing.
fn on_plane(plane: &Plane, c: &Cursor) -> Option<([f32; 3], Snap)> {
    if let Some(h) = c.hit {
        let off = dot(sub(h.point, plane.origin), plane.normal).abs();
        if matches!(h.snap, Snap::Endpoint | Snap::Midpoint) && off <= 1e-4 * (1.0 + len(h.point)) {
            return Some((h.point, h.snap));
        }
    }
    plane.intersect_ray(c.origin, c.dir).map(|p| (p, Snap::InPlane))
}

/// Locks `p` onto the plane axis through `from` when within about 3 degrees of it.
fn axis_lock(plane: &Plane, from: [f32; 3], p: [f32; 3]) -> ([f32; 3], Option<u8>) {
    let tol = 3f32.to_radians().tan();
    let d = sub(p, from);
    for axis in [plane.u, plane.v] {
        let along = dot(d, axis);
        let across = len(sub(d, scale(axis, along)));
        if along.abs() > 1e-9 && across <= tol * along.abs() {
            let world = (0..3).find(|&i| axis[i].abs() > 0.999).map_or(3, |i| i as u8);
            return (add(from, scale(axis, along)), Some(world));
        }
    }
    (p, None)
}

/// Rounds an angle to a multiple of 15 degrees when within 2 degrees of it, as the protractor does in SketchUp.
fn snap_angle(a: f32) -> f32 {
    let step = 15f32.to_radians();
    let near = (a / step).round() * step;
    if (a - near).abs() <= 2f32.to_radians() {
        near
    } else {
        a
    }
}

/// `p` turned `angle` radians about the line through `center` along unit `axis` (right-handed).
fn rotate_about(p: [f32; 3], center: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    let v = sub(p, center);
    let (s, c) = angle.sin_cos();
    // Rodrigues: v cos + (k x v) sin + k (k . v)(1 - cos)
    let r = add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)));
    add(center, r)
}

fn scale_about(p: [f32; 3], anchor: [f32; 3], factor: f32) -> [f32; 3] {
    add(anchor, scale(sub(p, anchor), factor))
}

/// The corner of the vertices' bounding box opposite `grip`: on each axis, the far side from it (or the grip's own
/// coordinate when the box has no size that way, so a flat shape scales in its plane).
fn opposite_corner(model: &Model, verts: &[u32], grip: [f32; 3]) -> [f32; 3] {
    let pts: Vec<[f32; 3]> = verts.iter().filter_map(|&v| model.mesh().positions.get(v as usize).copied()).collect();
    let mut out = grip;
    for i in 0..3 {
        let lo = pts.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min);
        let hi = pts.iter().map(|p| p[i]).fold(f32::NEG_INFINITY, f32::max);
        if hi - lo > 1e-6 {
            out[i] = if (grip[i] - lo).abs() < (grip[i] - hi).abs() { hi } else { lo };
        }
    }
    out
}

/// Every face joined to `face` through shared vertices.
fn connected_faces(mesh: &HalfEdgeMesh, face: usize) -> Vec<usize> {
    let n = mesh.face_count();
    if face >= n {
        return Vec::new();
    }
    let verts: Vec<Vec<u32>> = (0..n).map(|f| mesh.face_vertices(f)).collect();
    let mut by_vertex: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
    for (f, vs) in verts.iter().enumerate() {
        for &v in vs {
            by_vertex.entry(v).or_default().push(f);
        }
    }
    let mut seen = vec![false; n];
    let mut stack = vec![face];
    seen[face] = true;
    while let Some(f) = stack.pop() {
        for v in &verts[f] {
            for &g in &by_vertex[v] {
                if !seen[g] {
                    seen[g] = true;
                    stack.push(g);
                }
            }
        }
    }
    (0..n).filter(|&f| seen[f]).collect()
}

/// Locks the mouse onto the red, green or blue axis through `from` when the ray passes close to it (within about 1.5
/// degrees as seen from the eye): the point on the axis, and which axis.
fn ray_axis_lock(from: [f32; 3], c: &Cursor) -> Option<([f32; 3], u8)> {
    let tol = 1.5f32.to_radians().tan();
    let mut best: Option<([f32; 3], u8, f32)> = None;
    for (i, axis) in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]].into_iter().enumerate() {
        let Some(t) = closest_on_line(from, axis, c.origin, c.dir) else { continue };
        let p = add(from, scale(axis, t));
        let along = dot(sub(p, c.origin), c.dir);
        if along <= 0.0 || t.abs() < 1e-6 {
            continue;
        }
        let gap = len(sub(sub(p, c.origin), scale(c.dir, along)));
        let ratio = gap / along;
        if ratio <= tol && best.is_none_or(|b| ratio < b.2) {
            best = Some((p, i as u8, ratio));
        }
    }
    best.map(|(p, a, _)| (p, a))
}

/// Parameter along the line `base + s * dir_line` closest to the ray.
fn closest_on_line(base: [f32; 3], dir_line: [f32; 3], origin: [f32; 3], dir: [f32; 3]) -> Option<f32> {
    let w0 = sub(base, origin);
    let b = dot(dir_line, dir);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    Some((b * dot(dir, w0) - dot(dir_line, w0)) / denom)
}

fn ring_lines(r: &[[f32; 3]], out: &mut Vec<([f32; 3], [f32; 3], [f32; 4])>) {
    for i in 0..r.len() {
        out.push((r[i], r[(i + 1) % r.len()], RUBBER_BAND));
    }
}

fn fmt(v: f32, u: Units) -> String {
    units::format(v, u)
}

fn sign(v: f32) -> f32 {
    if v < 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    len(sub(a, b))
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(a: [f32; 3]) -> Option<[f32; 3]> {
    let l = len(a);
    (l > 1e-9).then(|| scale(a, 1.0 / l))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cursor looking straight down at (x, y) on the ground.
    fn at(x: f32, y: f32) -> Cursor {
        ground_hit([x, y, 0.0], Snap::Ground)
    }

    fn ground_hit(point: [f32; 3], snap: Snap) -> Cursor {
        Cursor {
            origin: [point[0], point[1], 10.0],
            dir: [0.0, 0.0, -1.0],
            hit: Some(Hit { point, plane: Plane::ground(), face: None, line: None, snap }),
        }
    }

    fn with(tool: Tool) -> (Tools, Model) {
        let mut t = Tools::default();
        t.set_tool(tool);
        (t, Model::new())
    }

    #[test]
    fn parses_measurement_entries() {
        let m = Units::Metres;
        assert_eq!(parse_entry(" 2.5 ", m), Some(Entry::Value(2.5)));
        assert_eq!(parse_entry("3,2", m), Some(Entry::Pair(3.0, 2.0)));
        assert_eq!(parse_entry("3; -2", m), Some(Entry::Pair(3.0, -2.0)));
        assert_eq!(parse_entry("24s", m), Some(Entry::Sides(24)));
        assert_eq!(parse_entry("2s", m), None, "fewer than 3 sides");
        assert_eq!(parse_entry("abc", m), None);
        assert_eq!(parse_entry("inf", m), None);
        // units: a bare number in the chosen unit, a written one wins
        assert_eq!(parse_entry("300", Units::Millimetres), Some(Entry::Value(0.3)));
        assert_eq!(parse_entry("300mm, 2m", Units::Inches), Some(Entry::Pair(0.3, 2.0)));
        match parse_entry("4'6\"", Units::Metres) {
            Some(Entry::Value(v)) => assert!((v - 54.0 * 0.0254).abs() < 1e-6),
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn shortcuts_match_sketchup() {
        assert_eq!(Tool::from_key("r"), Some(Tool::Rectangle));
        assert_eq!(Tool::from_key("P"), Some(Tool::PushPull));
        assert_eq!(Tool::from_key("space"), Some(Tool::Select));
        assert_eq!(Tool::from_key("q"), Some(Tool::Rotate));
        assert_eq!(Tool::from_key("m"), Some(Tool::Move));
        assert_eq!(Tool::from_key("S"), Some(Tool::Scale));
        assert_eq!(Tool::from_key("t"), Some(Tool::TapeMeasure));
        assert_eq!(Tool::from_key("x"), None);
    }

    /// A 2 x 2 x 2 box standing on the ground at the origin, and the cursor on its top face.
    fn box_model() -> (Model, Cursor) {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        m.push_pull(0, 2.0).unwrap();
        let top = (0..m.mesh().face_count()).find(|&f| m.mesh().face_normal(f)[2] > 0.9).unwrap();
        let c = Cursor {
            origin: [1.0, 1.0, 10.0],
            dir: [0.0, 0.0, -1.0],
            hit: Some(Hit { point: [1.0, 1.0, 2.0], plane: Plane::from_point_normal([1.0, 1.0, 2.0], [0.0, 0.0, 1.0]).unwrap(), face: Some(top), line: None, snap: Snap::OnFace }),
        };
        (m, c)
    }

    fn bounds(m: &Model) -> ([f32; 3], [f32; 3]) {
        let mesh = m.mesh().to_mesh();
        mesh.bounds().unwrap()
    }

    #[test]
    fn triple_click_selects_the_whole_box_and_move_carries_it() {
        let (mut m, top) = box_model();
        let mut t = Tools::default();
        t.select(&m, top.hit.unwrap().face, false, false, true);
        assert_eq!(t.selected.len(), 6, "every face of the box");
        t.set_tool(Tool::Move);
        t.click(&mut m, &top).unwrap();
        // the mouse off to +X, then 3 m typed
        t.hover(&ground_hit([5.0, 1.0, 0.0], Snap::Ground));
        t.type_entry(&mut m, "3").unwrap();
        let (lo, hi) = bounds(&m);
        let v = (m.mesh().signed_volume() - 8.0).abs();
        assert!(v < 1e-4, "still 8 cubic metres");
        // the direction was from (1,1,2) toward (5,1,0): mostly +X, a little down
        assert!(lo[0] > 2.0 && hi[0] > 4.5, "moved toward +X: {lo:?} {hi:?}");
        assert_eq!(t.selected.len(), 6, "the selection stays after a move");
        assert!(m.undo());
        assert_eq!(bounds(&m), ([0.0; 3], [2.0, 2.0, 2.0]));
    }

    #[test]
    fn move_locks_onto_the_blue_axis() {
        let (mut m, top) = box_model();
        let mut t = Tools::default();
        t.set_tool(Tool::Move);
        t.click(&mut m, &top).unwrap();
        // a ray passing just beside the vertical line through (1,1,2), at height 5
        let c = Cursor { origin: [1.0, -20.0, 5.0], dir: [0.0, 1.0, 0.0], hit: None };
        t.hover(&c);
        let (p, snap) = t.marker(&c).unwrap();
        assert_eq!(snap, Snap::OnAxis(2));
        assert!((p[2] - 5.0).abs() < 1e-4 && (p[0] - 1.0).abs() < 1e-5);
        t.click(&mut m, &c).unwrap();
        // nothing selected: only the top face moved, the box is now 5 m tall
        let (_, hi) = bounds(&m);
        assert!((hi[2] - 5.0).abs() < 1e-4);
        assert!((m.mesh().signed_volume() - 20.0).abs() < 1e-3);
    }

    #[test]
    fn rotate_by_typed_degrees() {
        let (mut m, top) = box_model();
        let mut t = Tools::default();
        t.select(&m, top.hit.unwrap().face, false, false, true);
        t.set_tool(Tool::Rotate);
        // centre at the origin corner on the ground, start along +X
        t.click(&mut m, &ground_hit([0.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        t.click(&mut m, &ground_hit([1.0, 0.0, 0.0], Snap::Ground)).unwrap();
        t.type_entry(&mut m, "90").unwrap();
        let (lo, hi) = bounds(&m);
        assert!((lo[0] + 2.0).abs() < 1e-4 && hi[0].abs() < 1e-4, "turned onto -X: {lo:?} {hi:?}");
        assert!((m.mesh().signed_volume() - 8.0).abs() < 1e-3, "a turn keeps the volume and the winding");
    }

    #[test]
    fn rotate_snaps_to_15_degrees() {
        assert!((snap_angle(44.0f32.to_radians()) - 45f32.to_radians()).abs() < 1e-6);
        assert!((snap_angle(40.0f32.to_radians()) - 40f32.to_radians()).abs() < 1e-6);
    }

    #[test]
    fn scale_by_a_factor_from_the_opposite_corner() {
        let (mut m, _) = box_model();
        let mut t = Tools::default();
        t.select_all(&m);
        t.set_tool(Tool::Scale);
        // grip the top corner at (2,2,2): the anchor is the corner at the origin
        let grip = Cursor { origin: [2.0, 2.0, 10.0], dir: [0.0, 0.0, -1.0], hit: Some(Hit { point: [2.0, 2.0, 2.0], plane: Plane::ground(), face: None, line: None, snap: Snap::Endpoint }) };
        t.click(&mut m, &grip).unwrap();
        t.type_entry(&mut m, "1.5").unwrap();
        assert_eq!(bounds(&m), ([0.0; 3], [3.0, 3.0, 3.0]));
        assert!((m.mesh().signed_volume() - 27.0).abs() < 1e-3);
    }

    #[test]
    fn tape_measure_leaves_a_guide_line_and_point() {
        let (mut t, mut m) = with(Tool::TapeMeasure);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.hover(&at(3.0, 4.0));
        assert_eq!(t.measurement(), ("Length", "5 m".into()));
        t.type_entry(&mut m, "10").unwrap();
        assert_eq!(t.guides.len(), 2);
        match t.guides[1] {
            Guide::Point(p) => assert!((p[0] - 6.0).abs() < 1e-5 && (p[1] - 8.0).abs() < 1e-5),
            g => panic!("{g:?}"),
        }
        assert_eq!(m.mesh().face_count(), 0, "guides are not geometry");
        t.delete_guides();
        assert!(t.guides.is_empty());
    }

    #[test]
    fn protractor_guide_at_a_typed_angle() {
        let (mut t, mut m) = with(Tool::Protractor);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.click(&mut m, &at(1.0, 0.0)).unwrap();
        t.type_entry(&mut m, "30").unwrap();
        match t.guides[0] {
            Guide::Line(p, d) => {
                assert_eq!(p, [0.0; 3]);
                assert!((d[0] - 30f32.to_radians().cos()).abs() < 1e-5 && (d[1] - 0.5).abs() < 1e-5);
            }
            g => panic!("{g:?}"),
        }
    }

    #[test]
    fn rotated_rectangle_and_pie_by_clicks_and_typing() {
        let (mut t, mut m) = with(Tool::RotatedRectangle);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.click(&mut m, &at(3.0, 4.0)).unwrap();
        t.hover(&at(-4.0, 3.0));
        t.type_entry(&mut m, "2").unwrap();
        m.push_pull(0, 1.0).unwrap();
        assert!((m.mesh().signed_volume().abs() - 10.0).abs() < 1e-3, "5 x 2 x 1");

        let (mut t, mut m) = with(Tool::Pie);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.hover(&at(1.0, 0.0));
        t.type_entry(&mut m, "2").unwrap();
        t.type_entry(&mut m, "90").unwrap();
        assert_eq!(m.mesh().face_count(), 1);
        assert_eq!(m.mesh().face_vertices(0).len(), 14);
    }

    #[test]
    fn rectangle_by_two_clicks() {
        let (mut t, mut m) = with(Tool::Rectangle);
        t.click(&mut m, &at(1.0, 1.0)).unwrap();
        t.hover(&at(4.0, 3.0));
        assert_eq!(t.measurement(), ("Dimensions", "3 m, 2 m".into()));
        t.units = Units::Millimetres;
        assert_eq!(t.measurement(), ("Dimensions", "3000 mm, 2000 mm".into()));
        t.units = Units::Metres;
        t.click(&mut m, &at(4.0, 3.0)).unwrap();
        assert_eq!(m.mesh().face_count(), 1);
        assert_eq!(m.mesh().face_normal(0), [0.0, 0.0, 1.0]);
        assert!(!t.is_busy());
    }

    #[test]
    fn typed_rectangle_follows_the_mouse_side() {
        let (mut t, mut m) = with(Tool::Rectangle);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.hover(&at(-1.0, 1.0));
        t.type_entry(&mut m, "3,2").unwrap();
        let (lo, hi) = m.mesh().to_mesh().bounds().unwrap();
        assert_eq!((lo, hi), ([-3.0, 0.0, 0.0], [0.0, 2.0, 0.0]));
    }

    #[test]
    fn line_tool_closes_an_l_shaped_room() {
        let (mut t, mut m) = with(Tool::Line);
        for (x, y) in [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0), (1.0, 2.0), (0.0, 2.0)] {
            t.click(&mut m, &at(x, y)).unwrap();
        }
        assert_eq!(m.mesh().face_count(), 0, "still open");
        assert_eq!(m.mesh().lines.len(), 5, "every segment is in the model already");
        // the viewer snaps onto the start point (an endpoint of the first line)
        t.click(&mut m, &ground_hit([0.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        assert_eq!(m.mesh().face_count(), 1);
        assert_eq!(m.mesh().face_vertices(0).len(), 6);
        assert!(m.mesh().lines.is_empty());
        assert!(!t.is_busy(), "closing a face ends the chain");

        // Pull it into an L-shaped solid of area 3 x height 2.
        let (mut p, _) = with(Tool::PushPull);
        let down = Cursor { origin: [0.5, 0.5, 10.0], dir: [0.0, 0.0, -1.0], hit: Some(Hit {
            point: [0.5, 0.5, 0.0], plane: Plane::ground(), face: Some(0), line: None, snap: Snap::OnFace,
        }) };
        p.click(&mut m, &down).unwrap();
        p.type_entry(&mut m, "2").unwrap();
        assert!(m.mesh().is_closed());
        assert!((m.mesh().signed_volume() - 6.0).abs() < 1e-4);
    }

    #[test]
    fn line_axis_lock_and_typed_length() {
        let (mut t, mut m) = with(Tool::Line);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        // 1 degree off the red axis locks onto it.
        let c = at(5.0, 5.0 * 1f32.to_radians().tan());
        assert_eq!(t.marker(&c).unwrap(), ([5.0, 0.0, 0.0], Snap::OnAxis(0)));
        t.hover(&c);
        t.type_entry(&mut m, "2").unwrap();
        assert_eq!(t.pending_points(), vec![[2.0, 0.0, 0.0]], "going on from the end of the typed line");
        assert_eq!(m.mesh().lines.len(), 1);
        // Far off any axis: no lock.
        assert_eq!(t.marker(&at(3.0, 2.0)).unwrap().1, Snap::Ground);
    }

    #[test]
    fn esc_keeps_the_lines_already_drawn() {
        let (mut t, mut m) = with(Tool::Line);
        for (x, y) in [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0)] {
            t.click(&mut m, &at(x, y)).unwrap();
        }
        t.cancel();
        assert!(!t.is_busy());
        assert_eq!(m.mesh().lines.len(), 2, "Esc stops the next line only");
    }

    #[test]
    fn a_line_goes_up_the_blue_axis() {
        let (mut t, mut m) = with(Tool::Line);
        t.click(&mut m, &at(1.0, 1.0)).unwrap();
        // looking from the front at height 2.5, just beside the upright line through (1, 1, 0)
        let c = Cursor { origin: [1.02, -20.0, 2.5], dir: [0.0, 1.0, 0.0], hit: None };
        let (p, snap) = t.marker(&c).unwrap();
        assert_eq!(snap, Snap::OnAxis(2));
        assert!((p[0] - 1.0).abs() < 1e-5 && (p[1] - 1.0).abs() < 1e-5 && (p[2] - 2.5).abs() < 1e-4);
        t.click(&mut m, &c).unwrap();
        let mesh = m.mesh().to_mesh();
        let [a, b] = mesh.lines[0];
        assert!((mesh.positions[b as usize][2] - mesh.positions[a as usize][2] - 2.5).abs() < 1e-4);
    }

    #[test]
    fn the_eraser_takes_a_line() {
        let (mut t, mut m) = with(Tool::Line);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.click(&mut m, &at(2.0, 0.0)).unwrap();
        t.set_tool(Tool::Eraser);
        let mut c = at(1.0, 0.0);
        c.hit.as_mut().unwrap().line = Some(0);
        t.click(&mut m, &c).unwrap();
        assert!(m.mesh().lines.is_empty());
    }

    #[test]
    fn circle_with_typed_segments_and_radius() {
        let (mut t, mut m) = with(Tool::Circle);
        t.type_entry(&mut m, "12s").unwrap();
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.type_entry(&mut m, "2").unwrap();
        assert_eq!(m.mesh().face_vertices(0).len(), 12);
        assert_eq!(t.circle_segments, 12);
    }

    #[test]
    fn push_pull_by_mouse_then_retype_distance() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        let (mut t, _) = with(Tool::PushPull);
        let face_hit = Cursor { origin: [1.0, -10.0, 0.0], dir: [0.0, 1.0, 0.0], hit: Some(Hit {
            point: [1.0, 1.0, 0.0], plane: Plane::ground(), face: Some(0), line: None, snap: Snap::OnFace,
        }) };
        t.click(&mut m, &face_hit).unwrap();
        // A horizontal ray at height 3 seen from the side sets distance 3.
        let side = Cursor { origin: [1.0, -10.0, 3.0], dir: [0.0, 1.0, 0.0], hit: None };
        t.hover(&side);
        assert_eq!(t.measurement(), ("Distance", "3 m".into()));
        assert!(t.preview_mesh(&m).is_some());
        assert_eq!(m.mesh().face_count(), 1, "preview does not touch the model");
        t.click(&mut m, &side).unwrap();
        assert!((m.mesh().signed_volume() - 12.0).abs() < 1e-4);

        // Typing right after replaces the last push/pull in one undo step.
        t.type_entry(&mut m, "1").unwrap();
        assert!((m.mesh().signed_volume() - 4.0).abs() < 1e-4);
        assert!(m.undo() && m.undo() && !m.can_undo());
    }

    #[test]
    fn eraser_and_errors_leave_tools_usable() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        let (mut t, _) = with(Tool::Eraser);
        let mut c = at(0.5, 0.5);
        c.hit.as_mut().unwrap().face = Some(0);
        t.click(&mut m, &c).unwrap();
        assert_eq!(m.mesh().face_count(), 0);

        let (mut r, mut m) = with(Tool::Rectangle);
        assert!(r.type_entry(&mut m, "3,2").is_err(), "no first corner yet");
        r.click(&mut m, &at(0.0, 0.0)).unwrap();
        assert!(r.click(&mut m, &at(2.0, 0.0)).is_err(), "zero height");
        assert!(!r.is_busy());
        assert_eq!(m.mesh().face_count(), 0);
    }
}
