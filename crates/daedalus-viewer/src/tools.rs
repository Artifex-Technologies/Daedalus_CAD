//! SketchUp-style tools as a state machine over the [`Model`]. No GPU or window
//! code lives here: the viewer resolves the mouse into a [`Cursor`] (ray plus
//! snapped hit) and the tools turn clicks and typed values into model edits.

use daedalus_core::units::{self, Units};
use daedalus_core::sketch::{self, Plane, DEFAULT_CIRCLE_SEGMENTS};
use daedalus_core::{HalfEdgeMesh, Model};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Rectangle,
    Circle,
    Polygon,
    PushPull,
    Eraser,
    Orbit,
    Pan,
    Zoom,
}

impl Tool {
    pub const ALL: [Tool; 10] = [
        Tool::Select,
        Tool::Line,
        Tool::Rectangle,
        Tool::Circle,
        Tool::Polygon,
        Tool::PushPull,
        Tool::Eraser,
        Tool::Orbit,
        Tool::Pan,
        Tool::Zoom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => "Line",
            Tool::Rectangle => "Rectangle",
            Tool::Circle => "Circle",
            Tool::Polygon => "Polygon",
            Tool::PushPull => "Push/Pull",
            Tool::Eraser => "Eraser",
            Tool::Orbit => "Orbit",
            Tool::Pan => "Pan (Hand)",
            Tool::Zoom => "Zoom",
        }
    }

    /// SketchUp's default single-key shortcut, if it has one.
    pub fn shortcut(self) -> Option<&'static str> {
        match self {
            Tool::Select => Some("Space"),
            Tool::Line => Some("L"),
            Tool::Rectangle => Some("R"),
            Tool::Circle => Some("C"),
            Tool::Polygon => None,
            Tool::PushPull => Some("P"),
            Tool::Eraser => Some("E"),
            Tool::Orbit => Some("O"),
            Tool::Pan => Some("H"),
            Tool::Zoom => Some("Z"),
        }
    }

    pub fn from_key(key: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.shortcut().is_some_and(|s| s.eq_ignore_ascii_case(key)))
    }

    /// Tools that create or change geometry (disabled on read-only imports).
    pub fn edits(self) -> bool {
        !matches!(self, Tool::Select | Tool::Orbit | Tool::Pan | Tool::Zoom)
    }

    /// Tools where a left-drag moves the camera instead of picking.
    pub fn navigates(self) -> bool {
        matches!(self, Tool::Orbit | Tool::Pan | Tool::Zoom)
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
        }
    }
}

pub const AXIS_COLORS: [[f32; 4]; 4] =
    [[0.85, 0.1, 0.1, 1.0], [0.1, 0.65, 0.1, 1.0], [0.1, 0.2, 0.9, 1.0], [0.7, 0.1, 0.7, 1.0]];
const RUBBER_BAND: [f32; 4] = [0.05, 0.05, 0.05, 1.0];
const PUSH_GUIDE: [f32; 4] = [0.2, 0.4, 0.9, 1.0];

/// The mouse resolved by the viewer: hits on geometry, snaps, or the ground.
#[derive(Debug, Clone, Copy)]
pub struct Hit {
    pub point: [f32; 3],
    /// Plane a drawing started here would use (the face's plane, or the ground).
    pub plane: Plane,
    pub face: Option<usize>,
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
    Line { plane: Plane, points: Vec<[f32; 3]>, cursor: Option<[f32; 3]>, axis: Option<u8> },
    Rect { plane: Plane, start: [f32; 3], cursor: Option<[f32; 3]> },
    Round { plane: Plane, center: [f32; 3], cursor: Option<[f32; 3]> },
    Push { face: usize, base: [f32; 3], normal: [f32; 3], distance: f32 },
}

pub struct Tools {
    pub tool: Tool,
    pub circle_segments: u32,
    pub polygon_sides: u32,
    /// What lengths are shown and typed in (the model itself is in metres).
    pub units: Units,
    pub selected: Option<usize>,
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
            selected: None,
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
            self.selected = None;
        }
        self.op = Op::Idle;
    }

    /// Call after anything outside the tools changed the mesh (undo, repair):
    /// face indices may no longer mean the same faces.
    pub fn model_changed(&mut self) {
        self.op = Op::Idle;
        self.selected = None;
        self.hover_face = None;
        self.last_push = None;
    }

    #[cfg(test)]
    fn is_busy(&self) -> bool {
        !matches!(self.op, Op::Idle)
    }

    /// Points that are not in the model yet but should still be snap targets
    /// (the Line tool's own corners, so the loop can be closed).
    pub fn pending_points(&self) -> Vec<[f32; 3]> {
        match &self.op {
            Op::Line { points, .. } => points.clone(),
            Op::Rect { start, .. } => vec![*start],
            Op::Round { center, .. } => vec![*center],
            _ => Vec::new(),
        }
    }

    /// Face to draw highlighted: the selection, or the face a click would act on.
    pub fn highlight(&self) -> Option<usize> {
        match (self.tool, &self.op) {
            (Tool::PushPull | Tool::Eraser, Op::Idle) => self.hover_face,
            (Tool::PushPull, _) => None,
            _ => self.selected,
        }
    }

    /// Cursor position and snap after the tool's own inference (axis locks,
    /// keeping to the workplane), for the marker and tooltip.
    pub fn marker(&self, c: &Cursor) -> Option<([f32; 3], Snap)> {
        match &self.op {
            Op::Line { plane, points, .. } => {
                let (p, snap) = on_plane(plane, c)?;
                Some(match points.last() {
                    Some(&last) if !matches!(snap, Snap::Endpoint | Snap::Midpoint) => {
                        let (q, axis) = axis_lock(plane, last, p);
                        (q, axis.map_or(snap, Snap::OnAxis))
                    }
                    _ => (p, snap),
                })
            }
            Op::Rect { plane, .. } | Op::Round { plane, .. } => on_plane(plane, c),
            Op::Push { .. } => None,
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
            Op::Rect { cursor, .. } | Op::Round { cursor, .. } => *cursor = marker.map(|m| m.0),
            Op::Push { base, normal, distance, .. } => {
                if let Some(d) = closest_on_line(*base, *normal, c.origin, c.dir) {
                    *distance = d;
                }
            }
            Op::Idle => {}
        }
    }

    /// Left click. Returns a status message for errors the user should see.
    pub fn click(&mut self, model: &mut Model, c: &Cursor) -> Result<(), String> {
        self.hover(c);
        let marker = self.marker(c);
        match (self.tool, std::mem::replace(&mut self.op, Op::Idle)) {
            (Tool::Select, _) => self.selected = c.hit.and_then(|h| h.face),
            (Tool::Eraser, _) => {
                if let Some(face) = c.hit.and_then(|h| h.face) {
                    model.erase_face(face).map_err(|e| e.to_string())?;
                    self.model_changed();
                }
            }
            (Tool::Line, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Line { plane: h.plane, points: vec![h.point], cursor: None, axis: None };
                }
            }
            (Tool::Line, Op::Line { plane, mut points, .. }) => {
                let Some((p, _)) = marker else {
                    self.op = Op::Line { plane, points, cursor: None, axis: None };
                    return Ok(());
                };
                self.op = Op::Line { plane, points: Vec::new(), cursor: None, axis: None };
                self.add_line_point(model, plane, &mut points, p)?;
            }
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
        let entry = parse_entry(text, self.units).ok_or_else(|| format!("cannot read \"{text}\""))?;
        match (entry, std::mem::replace(&mut self.op, Op::Idle)) {
            (Entry::Sides(n), op) if matches!(self.tool, Tool::Circle | Tool::Polygon) => {
                match self.tool {
                    Tool::Circle => self.circle_segments = n,
                    _ => self.polygon_sides = n,
                }
                self.op = op;
            }
            (Entry::Value(len), Op::Line { plane, mut points, cursor, axis }) => {
                let last = *points.last().expect("a line op always has a start point");
                let dir = cursor.and_then(|c| normalize(sub(c, last)));
                let Some(dir) = dir else {
                    self.op = Op::Line { plane, points, cursor, axis };
                    return Err("move the mouse to show the direction first".into());
                };
                self.op = Op::Line { plane, points: Vec::new(), cursor, axis };
                self.add_line_point(model, plane, &mut points, add(last, scale(dir, len)))?;
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

    /// Mesh to draw instead of the model while push/pull is live.
    pub fn preview_mesh(&self, model: &Model) -> Option<HalfEdgeMesh> {
        let Op::Push { face, distance, .. } = self.op else { return None };
        if distance == 0.0 {
            return None;
        }
        let mut m = model.mesh().clone();
        m.push_pull(face, distance).ok()?;
        Some(m)
    }

    /// Rubber-band lines drawn over the scene.
    pub fn preview_lines(&self) -> Vec<([f32; 3], [f32; 3], [f32; 4])> {
        let mut out = Vec::new();
        match &self.op {
            Op::Line { points, cursor, axis, .. } => {
                for w in points.windows(2) {
                    out.push((w[0], w[1], RUBBER_BAND));
                }
                if let (Some(&last), Some(c)) = (points.last(), cursor) {
                    out.push((last, *c, axis.map_or(RUBBER_BAND, |a| AXIS_COLORS[a as usize])));
                }
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
            _ => {}
        }
        out
    }

    /// SketchUp-style hint for the status bar.
    pub fn status(&self) -> &'static str {
        match (self.tool, &self.op) {
            (Tool::Select, _) => "Click a face to select it.",
            (Tool::Line, Op::Idle) => "Select start point.",
            (Tool::Line, _) => "Select end point, or type a length. Click the start point to close the face.",
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
            Op::Line { points, cursor: Some(c), .. } => ("Length", fmt(dist(*points.last().unwrap(), *c), self.units)),
            Op::Line { .. } => ("Length", String::new()),
            Op::Rect { plane, start, cursor: Some(c) } => {
                let (a, b) = (plane.to_local(*start), plane.to_local(*c));
                ("Dimensions", format!("{}, {}", fmt((b[0] - a[0]).abs(), self.units), fmt((b[1] - a[1]).abs(), self.units)))
            }
            Op::Rect { .. } => ("Dimensions", String::new()),
            Op::Round { center, cursor, .. } => ("Radius", cursor.map(|c| fmt(dist(*center, c), self.units)).unwrap_or_default()),
            Op::Push { distance, .. } => ("Distance", fmt(*distance, self.units)),
            Op::Idle => match self.tool {
                Tool::Circle => ("Segments", self.circle_segments.to_string()),
                Tool::Polygon => ("Sides", self.polygon_sides.to_string()),
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

    /// Adds `p` to the line chain; closing on the start point makes the face.
    fn add_line_point(
        &mut self,
        model: &mut Model,
        plane: Plane,
        points: &mut Vec<[f32; 3]>,
        p: [f32; 3],
    ) -> Result<(), String> {
        let same = |a: [f32; 3], b: [f32; 3]| dist(a, b) <= 1e-6 * (1.0 + len(a));
        if points.len() >= 3 && same(p, points[0]) {
            return self.commit(model, Some(std::mem::take(points)));
        }
        if !points.last().is_some_and(|&l| same(l, p)) {
            points.push(p);
        }
        self.op = Op::Line { plane, points: std::mem::take(points), cursor: None, axis: None };
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
            hit: Some(Hit { point, plane: Plane::ground(), face: None, snap }),
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
        assert_eq!(Tool::from_key("q"), None);
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
        // The viewer snaps onto the start point because it is a pending point.
        assert!(t.pending_points().contains(&[0.0, 0.0, 0.0]));
        t.click(&mut m, &ground_hit([0.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        assert_eq!(m.mesh().face_count(), 1);
        assert_eq!(m.mesh().face_vertices(0).len(), 6);

        // Pull it into an L-shaped solid of area 3 x height 2.
        let (mut p, _) = with(Tool::PushPull);
        let down = Cursor { origin: [0.5, 0.5, 10.0], dir: [0.0, 0.0, -1.0], hit: Some(Hit {
            point: [0.5, 0.5, 0.0], plane: Plane::ground(), face: Some(0), snap: Snap::OnFace,
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
        assert_eq!(t.pending_points(), vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]]);
        // Far off any axis: no lock.
        assert_eq!(t.marker(&at(3.0, 2.0)).unwrap().1, Snap::InPlane);
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
            point: [1.0, 1.0, 0.0], plane: Plane::ground(), face: Some(0), snap: Snap::OnFace,
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
