//! SketchUp-style tools as a state machine over the [`Model`]. No GPU or window
//! code lives here: the viewer resolves the mouse into a [`Cursor`] (ray plus
//! snapped hit) and the tools turn clicks and typed values into model edits.

use daedalus_core::units::{self, Units};
use daedalus_core::sketch::{self, Plane, DEFAULT_ARC_SEGMENTS, DEFAULT_CIRCLE_SEGMENTS};
use daedalus_core::scene::{Axes, Dimension, Label, Section, Xform};
use daedalus_core::{HalfEdgeMesh, Model};

pub use daedalus_core::scene::Guide;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Rectangle,
    RotatedRectangle,
    Circle,
    Polygon,
    Arc,
    TwoPointArc,
    ThreePointArc,
    Pie,
    Freehand,
    PushPull,
    Offset,
    FollowMe,
    PaintBucket,
    SectionPlane,
    Dimensions,
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
    Text,
    Axes,
    Text3D,
}

impl Tool {
    pub const ALL: [Tool; 33] = [
        Tool::Select,
        Tool::Line,
        Tool::Rectangle,
        Tool::RotatedRectangle,
        Tool::Circle,
        Tool::Polygon,
        Tool::Arc,
        Tool::TwoPointArc,
        Tool::ThreePointArc,
        Tool::Pie,
        Tool::Freehand,
        Tool::PushPull,
        Tool::Offset,
        Tool::FollowMe,
        Tool::PaintBucket,
        Tool::SectionPlane,
        Tool::Dimensions,
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
        Tool::Text,
        Tool::Axes,
        Tool::Text3D,
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
            Tool::Arc => "Arc",
            Tool::TwoPointArc => "2 Point Arc",
            Tool::ThreePointArc => "3 Point Arc",
            Tool::Freehand => "Freehand",
            Tool::Offset => "Offset",
            Tool::PaintBucket => "Paint Bucket",
            Tool::FollowMe => "Follow Me",
            Tool::SectionPlane => "Section Plane",
            Tool::Dimensions => "Dimensions",
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
            Tool::Text => "Text",
            Tool::Axes => "Axes",
            Tool::Text3D => "3D Text",
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
            Tool::TwoPointArc => Some("A"),
            Tool::Offset => Some("F"),
            Tool::PaintBucket => Some("B"),
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
            | Tool::Arc
            | Tool::ThreePointArc
            | Tool::Freehand
            | Tool::FollowMe
            | Tool::SectionPlane
            | Tool::Dimensions
            | Tool::Protractor
            | Tool::ZoomWindow
            | Tool::PositionCamera
            | Tool::LookAround
            | Tool::Walk
            | Tool::Text
            | Tool::Axes
            | Tool::Text3D => None,
        }
    }

    pub fn from_key(key: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.shortcut().is_some_and(|s| s.eq_ignore_ascii_case(key)))
    }

    /// Tools that create or change geometry (disabled on read-only imports).
    pub fn edits(self) -> bool {
        !self.navigates()
            && !matches!(self, Tool::Select | Tool::TapeMeasure | Tool::Protractor | Tool::PositionCamera | Tool::SectionPlane | Tool::Dimensions | Tool::Text | Tool::Axes)
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
    pub point: [f64; 3],
    /// Plane a drawing started here would use (the face's plane, or the ground).
    pub plane: Plane,
    pub face: Option<usize>,
    /// A loose line close to the mouse (for the Eraser).
    pub line: Option<usize>,
    /// The group or component under the mouse (an instance of the open context); `face` is None then.
    pub instance: Option<usize>,
    pub snap: Snap,
}

/// What Move, Rotate and Scale work on: faces (and their vertices) and instances of the open context.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Target {
    pub faces: Vec<usize>,
    pub verts: Vec<u32>,
    pub inst: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    pub origin: [f64; 3],
    pub dir: [f64; 3],
    pub hit: Option<Hit>,
}

/// A value typed into the Measurements box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Entry {
    Value(f64),
    Pair(f64, f64),
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
    Line { plane: Plane, last: [f64; 3], cursor: Option<[f64; 3]>, axis: Option<u8> },
    Rect { plane: Plane, start: [f64; 3], cursor: Option<[f64; 3]> },
    Round { plane: Plane, center: [f64; 3], cursor: Option<[f64; 3]> },
    Push { face: usize, base: [f64; 3], normal: [f64; 3], distance: f64 },
    /// 2 Point Arc: the ends `a` and `b` (once set), then the bulge toward the cursor.
    Arc2 { plane: Plane, a: [f64; 3], b: Option<[f64; 3]>, cursor: Option<[f64; 3]> },
    /// 3 Point Arc: the start, a point the arc passes through (once set), then the end at the cursor.
    Arc3 { plane: Plane, a: [f64; 3], m: Option<[f64; 3]>, cursor: Option<[f64; 3]> },
    /// Offset: the face's outline (counter-clockwise about `normal`) and how far in to copy it.
    Offset { ring: Vec<[f64; 3]>, normal: [f64; 3], d: f64 },
    /// Dimensions: the two points measured (the second once set), then where the dimension line sits.
    Dim { a: [f64; 3], b: Option<[f64; 3]>, cursor: Option<[f64; 3]> },
    /// Follow Me: the profile face chosen; next, a line of the path.
    Follow { face: usize },
    /// Freehand: the points of the stroke while the button is held.
    Free { plane: Plane, points: Vec<[f64; 3]> },
    /// Rotated Rectangle: the first side from `a` to `b` (once set), then the height toward the cursor.
    RotRect { plane: Plane, a: [f64; 3], b: Option<[f64; 3]>, cursor: Option<[f64; 3]> },
    /// Pie: the centre, the start of the arc (once set, it gives the radius), then the angle.
    Pie { plane: Plane, center: [f64; 3], start: Option<[f64; 3]>, cursor: Option<[f64; 3]> },
    /// Move: these vertices go from `base` to the cursor.
    Move { t: Target, base: [f64; 3], cursor: Option<[f64; 3]>, axis: Option<u8> },
    /// Rotate: about the plane's normal through `center`, from the `start` direction (once set) by `angle` radians.
    Rotate { t: Target, plane: Plane, center: [f64; 3], start: Option<[f64; 3]>, angle: f64 },
    /// Scale: uniformly about `anchor` (the corner of the bounding box opposite the grip), `start` being the grip.
    Scale { t: Target, anchor: [f64; 3], start: [f64; 3], factor: f64 },
    /// Tape Measure: from `start` to the cursor.
    Tape { start: [f64; 3], cursor: Option<[f64; 3]> },
    /// Protractor: a guide through `center` turned `angle` radians from the `start` direction (once set).
    Protract { plane: Plane, center: [f64; 3], start: Option<[f64; 3]>, angle: f64 },
    /// Text: the point it is about and the words it starts with; next, where it is shown.
    Text { point: [f64; 3], text: String, cursor: Option<[f64; 3]> },
    /// Axes: the new origin, then the red direction (once set), then the green.
    Axes { origin: [f64; 3], red: Option<[f64; 3]>, cursor: Option<[f64; 3]> },
    /// 3D Text being placed: the words, how tall, how deep (None: flat), their outlines (for the preview), and where they
    /// would go now (on the plane under the mouse).
    Place3D { text: String, height: f64, depth: Option<f64>, letters: Vec<Vec<Vec<[f64; 2]>>>, at: Option<Plane> },
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
    /// Selected groups and components (instances of the open context), sorted.
    pub instances: Vec<usize>,
    /// Selected edges and loose lines of the open context, as vertex pairs (lower number first), sorted.
    pub edges: Vec<[u32; 2]>,
    /// Paint Bucket's colour (None: the default material, which takes paint off).
    pub paint_color: Option<[u8; 3]>,
    /// Move makes a copy (Ctrl toggles it while moving, as in SketchUp).
    pub copy: bool,
    /// The last copy made by Move (what was copied and how far), so `x5` or `/5` typed right after makes an array.
    last_copy: Option<(Target, [f64; 3])>,
    hover_face: Option<usize>,
    hover_instance: Option<usize>,
    /// The drawing axes (a copy of the model's, kept up to date by the viewer): axis locks follow them.
    pub axes: Axes,
    /// Text just placed (its number in the notes): the viewer opens it for typing.
    pub edit_label: Option<usize>,
    op: Op,
    /// Last push/pull (original face, distance), so a value typed right after
    /// it re-does it at that distance, as in SketchUp.
    last_push: Option<(usize, f64)>,
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
            instances: Vec::new(),
            edges: Vec::new(),
            paint_color: Some([196, 92, 64]),
            copy: false,
            last_copy: None,
            hover_face: None,
            hover_instance: None,
            axes: Axes::default(),
            edit_label: None,
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
        self.hover_instance = None;
        self.last_push = None;
        self.copy = false;
        self.last_copy = None;
    }

    /// Esc: abandons the current operation (or clears the selection).
    pub fn cancel(&mut self) {
        if matches!(self.op, Op::Idle) {
            self.selected.clear();
            self.instances.clear();
            self.edges.clear();
        }
        self.op = Op::Idle;
        self.copy = false;
    }

    /// Call after anything outside the tools changed the mesh (undo, repair):
    /// face indices may no longer mean the same faces.
    pub fn model_changed(&mut self) {
        self.op = Op::Idle;
        self.selected.clear();
        self.instances.clear();
        self.edges.clear();
        self.hover_face = None;
        self.hover_instance = None;
        self.last_push = None;
        self.last_copy = None;
    }

    /// Whether anything is selected (faces, edges or groups).
    pub fn has_selection(&self) -> bool {
        !self.selected.is_empty() || !self.instances.is_empty() || !self.edges.is_empty()
    }

    /// Select tool click on a curve: all its `edges` at once (replacing the selection; with `add` added; with `toggle` taken
    /// out if they were all in, else put in).
    pub fn select_edges(&mut self, edges: &[[u32; 2]], add: bool, toggle: bool) {
        let norm: Vec<[u32; 2]> = edges.iter().map(|&[a, b]| [a.min(b), a.max(b)]).collect();
        if !add && !toggle {
            self.selected.clear();
            self.instances.clear();
            self.edges.clear();
        }
        let all_in = norm.iter().all(|e| self.edges.binary_search(e).is_ok());
        if toggle && all_in {
            self.edges.retain(|e| !norm.contains(e));
            return;
        }
        self.edges.extend(norm);
        self.edges.sort_unstable();
        self.edges.dedup();
    }

    /// Select tool click on an edge or loose line (vertices `a`, `b` of the open context): it replaces the selection; with
    /// `add` (Ctrl) it is added, with `toggle` (Shift) it goes in or out.
    pub fn select_edge(&mut self, [a, b]: [u32; 2], add: bool, toggle: bool) {
        let e = [a.min(b), a.max(b)];
        if !add && !toggle {
            self.selected.clear();
            self.instances.clear();
            self.edges.clear();
        }
        match self.edges.binary_search(&e) {
            Ok(k) if toggle => {
                self.edges.remove(k);
            }
            Ok(_) => {}
            Err(k) => self.edges.insert(k, e),
        }
    }

    /// Select tool click: `face` replaces the selection; with `add` (Ctrl) it is added, with `toggle` (Shift) it goes in
    /// or out; `connected` (a triple-click) takes every face joined to it. Clicking empty space clears the selection.
    pub fn select(&mut self, model: &Model, face: Option<usize>, instance: Option<usize>, add: bool, toggle: bool, connected: bool) {
        if let (None, Some(i)) = (face, instance) {
            // a group or component is picked whole
            if !add && !toggle {
                self.selected.clear();
                self.instances.clear();
                self.edges.clear();
            }
            match self.instances.binary_search(&i) {
                Ok(k) if toggle => {
                    self.instances.remove(k);
                }
                Ok(_) => {}
                Err(k) => self.instances.insert(k, i),
            }
            return;
        }
        let Some(face) = face else {
            if !add && !toggle {
                self.selected.clear();
                self.instances.clear();
                self.edges.clear();
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
            self.instances.clear();
            self.edges.clear();
        }
        self.selected.extend(faces);
        self.selected.sort_unstable();
        self.selected.dedup();
    }

    /// Ctrl+A.
    pub fn select_all(&mut self, model: &Model) {
        self.selected = (0..model.mesh().face_count()).collect();
        self.instances = (0..model.instances().len()).collect();
    }

    /// What Move, Rotate and Scale work on: the selection, else the face or group under the mouse.
    fn target(&self, model: &Model, c: &Cursor) -> Option<Target> {
        let (faces, inst): (Vec<usize>, Vec<usize>) = if self.has_selection() {
            (self.selected.clone(), self.instances.clone())
        } else {
            (c.hit.and_then(|h| h.face).into_iter().collect(), c.hit.and_then(|h| h.instance).into_iter().collect())
        };
        let faces: Vec<usize> = faces.into_iter().filter(|&f| f < model.mesh().face_count()).collect();
        let inst: Vec<usize> = inst.into_iter().filter(|&i| i < model.instances().len()).collect();
        let mut verts: Vec<u32> = faces.iter().flat_map(|&f| model.mesh().face_vertices(f)).collect();
        if self.has_selection() {
            let n = model.mesh().vertex_count() as u32;
            verts.extend(self.edges.iter().flatten().copied().filter(|&v| v < n));
        }
        verts.sort_unstable();
        verts.dedup();
        (!verts.is_empty() || !inst.is_empty()).then_some(Target { faces, verts, inst })
    }

    /// Ends a Move, Rotate or Scale: the faces keep their numbers, so the selection stays. With `copy` (Move with Ctrl),
    /// a copy is moved instead and becomes the selection.
    fn transform(&mut self, model: &mut Model, t: &Target, f: impl Fn([f64; 3]) -> [f64; 3]) -> Result<(), String> {
        let xf = Xform::from_fn(f);
        if self.copy && self.tool == Tool::Move {
            let (faces, inst) = model.copy_all(&t.faces, &t.inst, &[xf]).map_err(|e| e.to_string())?;
            self.selected = faces;
            self.instances = inst;
            self.last_copy = Some((t.clone(), xf.origin()));
            self.copy = false;
        } else {
            model.transform_all(&t.verts, &t.inst, &xf).map_err(|e| e.to_string())?;
        }
        self.op = Op::Idle;
        Ok(())
    }

    /// 3D Text: the words chosen in its window; the mouse now places them.
    pub fn begin_text3d(&mut self, text: &str, height: f64, depth: Option<f64>) -> Result<(), String> {
        if !(height > 0.0) {
            return Err("the letters need a height".into());
        }
        let letters = crate::text3d::outlines(text, height).ok_or("the built-in font could not be read")?;
        if letters.is_empty() {
            return Err("there are no letters to make".into());
        }
        self.set_tool(Tool::Text3D);
        self.op = Op::Place3D { text: text.to_string(), height, depth, letters, at: None };
        Ok(())
    }

    /// Whether 3D Text is waiting for its words (the viewer shows its window).
    pub fn wants_text3d(&self) -> bool {
        self.tool == Tool::Text3D && matches!(self.op, Op::Idle)
    }

    /// Whether `x5` or `/5` typed now would make an array of the last copy.
    pub fn can_array(&self) -> bool {
        self.last_copy.is_some() && matches!(self.op, Op::Idle)
    }

    /// `x5` (or `5x`, `*5`): four more copies at the same spacing, five in all; `/5`: copies spaced evenly between the
    /// original and the copy, five spaces in all.
    fn array(&mut self, model: &mut Model, text: &str) -> Option<Result<(), String>> {
        let (src, d) = self.last_copy.clone()?;
        let t = text.trim();
        let (n, divide) = if let Some(n) = t.strip_prefix('/') {
            (n.trim().parse::<u32>().ok()?, true)
        } else {
            let n = t.strip_prefix(['x', 'X', '*']).or_else(|| t.strip_suffix(['x', 'X'])).map(str::trim)?;
            (n.parse::<u32>().ok()?, false)
        };
        if !(2..=1000).contains(&n) {
            return Some(Err("an array needs 2 to 1000 copies".into()));
        }
        let steps: Vec<Xform> = if divide {
            (1..n).map(|k| Xform::translation(scale(d, k as f64 / n as f64))).collect()
        } else {
            (2..=n).map(|k| Xform::translation(scale(d, k as f64))).collect()
        };
        Some(match model.copy_all(&src.faces, &src.inst, &steps) {
            Ok((faces, inst)) => {
                self.selected = faces;
                self.instances = inst;
                self.last_copy = None;
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        })
    }

    /// Move, Rotate or Scale live on groups: which instances, the transform so far, and whether they are copies.
    pub fn preview_instances(&self) -> Option<(Vec<usize>, Xform, bool)> {
        let (t, xf) = self.live_transform()?;
        (!t.inst.is_empty()).then(|| (t.inst.clone(), xf, self.copy && self.tool == Tool::Move))
    }

    /// The Move, Rotate or Scale under way, and the transform it would make now.
    fn live_transform(&self) -> Option<(&Target, Xform)> {
        match &self.op {
            Op::Move { t, base, cursor: Some(c), .. } => Some((t, Xform::translation(sub(*c, *base)))),
            Op::Rotate { t, plane, center, start: Some(_), angle } => {
                let (center, n, a) = (*center, plane.normal, *angle);
                Some((t, Xform::from_fn(|q| rotate_about(q, center, n, a))))
            }
            Op::Scale { t, anchor, factor, .. } => {
                let (anchor, f) = (*anchor, *factor);
                Some((t, Xform::from_fn(|q| scale_about(q, anchor, f))))
            }
            _ => None,
        }
    }

    /// The section plane's outline, drawn while it is placed: a square in its plane around its point, `size` across.
    pub fn section_outline(&self, model: &Model, size: f64) -> Vec<([f64; 3], [f64; 3], [f32; 4])> {
        let Some(Section { point: o, normal: n, .. }) = model.notes().section else { return Vec::new() };
        let Some(p) = Plane::from_point_normal(o, n) else { return Vec::new() };
        let h = size / 2.0;
        let corners = [p.to_world(-h, -h), p.to_world(h, -h), p.to_world(h, h), p.to_world(-h, h)];
        let orange = [0.9, 0.5, 0.15, 1.0];
        let mut out: Vec<_> = (0..4).map(|i| (corners[i], corners[(i + 1) % 4], orange)).collect();
        // an arrow the way it looks (into what is kept)
        out.push((o, sub(o, scale(n, h * 0.4)), orange));
        out
    }

    /// The placed dimensions' lines (extension lines and the dimension line), in `color`.
    pub fn dim_lines(&self, model: &Model, color: [f32; 4]) -> Vec<([f64; 3], [f64; 3], [f32; 4])> {
        model.notes().dims.iter().flat_map(|d| dim_lines(d.a, d.b, d.offset, color)).collect()
    }

    /// Each placed dimension's label: where it goes (the middle of its dimension line) and the length, in the units shown.
    pub fn dim_labels(&self, model: &Model) -> Vec<([f64; 3], String)> {
        model.notes().dims.iter().map(|d| (add(scale(add(d.a, d.b), 0.5), d.offset), fmt(dist(d.a, d.b), self.units))).collect()
    }

    #[cfg(test)]
    fn is_busy(&self) -> bool {
        !matches!(self.op, Op::Idle)
    }

    /// Points that are not in the model yet but should still be snap targets
    /// (the Line tool's own corners, so the loop can be closed).
    pub fn pending_points(&self) -> Vec<[f64; 3]> {
        match &self.op {
            Op::Line { last, .. } => vec![*last],
            Op::Rect { start, .. } => vec![*start],
            Op::Round { center, .. } | Op::Pie { center, .. } => vec![*center],
            Op::Arc2 { a, b, .. } => std::iter::once(*a).chain(*b).collect(),
            Op::Arc3 { a, m, .. } => std::iter::once(*a).chain(*m).collect(),
            Op::RotRect { a, b, .. } => std::iter::once(*a).chain(*b).collect(),
            Op::Move { base, .. } => vec![*base],
            Op::Tape { start, .. } => vec![*start],
            _ => Vec::new(),
        }
    }

    /// Faces to draw highlighted: the selection, or the face a click would act on.
    pub fn highlight_instances(&self) -> Vec<usize> {
        match (self.tool, &self.op) {
            (Tool::Eraser, Op::Idle) => self.hover_instance.into_iter().collect(),
            (t, Op::Idle) if t.transforms() && !self.has_selection() => self.hover_instance.into_iter().collect(),
            (t, _) if t.transforms() && !self.has_selection() => self.live_transform().map(|(t, _)| t.inst.clone()).unwrap_or_default(),
            _ => self.instances.clone(),
        }
    }

    pub fn highlight(&self) -> Vec<usize> {
        match (self.tool, &self.op) {
            (Tool::PushPull | Tool::Eraser, Op::Idle) => self.hover_face.into_iter().collect(),
            (Tool::PushPull, _) => Vec::new(),
            (t, Op::Idle) if t.transforms() && !self.has_selection() => self.hover_face.into_iter().collect(),
            _ => self.selected.clone(),
        }
    }

    /// Cursor position and snap after the tool's own inference (axis locks,
    /// keeping to the workplane), for the marker and tooltip.
    pub fn marker(&self, c: &Cursor) -> Option<([f64; 3], Snap)> {
        match &self.op {
            Op::Line { plane, last, .. } => {
                // a snap wins; then the red, green or blue axis through the last point; then the face or ground under the
                // mouse, locked onto an axis of its plane when close to one
                if let Some(h) = c.hit.filter(|h| matches!(h.snap, Snap::Endpoint | Snap::Midpoint | Snap::OnGuide | Snap::Origin)) {
                    return Some((h.point, h.snap));
                }
                if let Some((p, a)) = ray_axis_lock(*last, c, &self.axes.dirs) {
                    return Some((p, Snap::OnAxis(a)));
                }
                let (p, snap) = match c.hit {
                    Some(h) => (h.point, h.snap),
                    None => on_plane(&Plane { origin: *last, ..*plane }, c)?,
                };
                let level = Plane { origin: *last, ..Plane::from_point_normal(*last, plane.normal).unwrap_or(*plane) };
                if dot(sub(p, *last), level.normal).abs() <= 1e-4 * (1.0 + len(p)) {
                    let (q, axis) = axis_lock(&level, *last, p, &self.axes.dirs);
                    return Some((q, axis.map_or(snap, Snap::OnAxis)));
                }
                Some((p, snap))
            }
            Op::Rect { plane, .. } | Op::Round { plane, .. } | Op::RotRect { plane, .. } | Op::Pie { plane, .. } => on_plane(plane, c),
            Op::Arc2 { plane, .. } | Op::Arc3 { plane, .. } | Op::Free { plane, .. } => on_plane(plane, c),
            Op::Rotate { plane, .. } | Op::Protract { plane, .. } => on_plane(plane, c),
            Op::Move { base, .. } | Op::Tape { start: base, .. } | Op::Axes { origin: base, .. } => {
                // a snap wins; else the mouse locks onto an axis through the start point when it is close to one
                match c.hit {
                    Some(h) if matches!(h.snap, Snap::Endpoint | Snap::Midpoint | Snap::OnGuide) => Some((h.point, h.snap)),
                    _ => ray_axis_lock(*base, c, &self.axes.dirs).map(|(p, a)| (p, Snap::OnAxis(a))).or(c.hit.map(|h| (h.point, h.snap))),
                }
            }
            Op::Text { point, .. } => {
                // on what is under the mouse, else on a plane through the point facing the eye
                c.hit.map(|h| (h.point, h.snap)).or_else(|| on_plane(&Plane::from_point_normal(*point, scale(c.dir, -1.0))?, c))
            }
            Op::Push { .. } | Op::Scale { .. } | Op::Offset { .. } => None,
            Op::Follow { .. } | Op::Dim { .. } | Op::Place3D { .. } => c.hit.map(|h| (h.point, h.snap)),
            Op::Idle => c.hit.map(|h| (h.point, h.snap)),
        }
    }

    /// Mouse moved: updates the rubber band or the push/pull distance.
    pub fn hover(&mut self, c: &Cursor) {
        let marker = self.marker(c);
        self.hover_face = c.hit.and_then(|h| h.face);
        self.hover_instance = c.hit.and_then(|h| h.instance);
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
            Op::Tape { cursor, .. } | Op::Arc2 { cursor, .. } | Op::Arc3 { cursor, .. } | Op::Dim { cursor, .. } => *cursor = marker.map(|m| m.0),
            Op::Text { cursor, .. } | Op::Axes { cursor, .. } => *cursor = marker.map(|m| m.0),
            Op::Place3D { at, .. } => *at = c.hit.map(|h| Plane { origin: marker.map_or(h.point, |m| m.0), ..h.plane }),
            Op::Free { points, .. } => {
                // a new point once the mouse has moved a little way from the last
                if let (Some((p, _)), Some(&last)) = (marker, points.last()) {
                    let reach = points.first().map_or(1.0, |&f| dist(f, last)).max(0.02);
                    if dist(p, last) > reach * 0.02 && points.len() < 2000 {
                        points.push(p);
                    }
                }
            }
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
            Op::Offset { ring, normal, d } => {
                // how far the mouse is inside the outline (negative outside), in the face's plane
                if let Some(p) = Plane::from_point_normal(ring[0], *normal).and_then(|pl| pl.intersect_ray(c.origin, c.dir)) {
                    *d = inside_distance(ring, *normal, p);
                }
            }
            Op::Rotate { .. } | Op::Protract { .. } | Op::Follow { .. } | Op::Idle => {}
        }
    }

    /// Left click. Returns a status message for errors the user should see.
    pub fn click(&mut self, model: &mut Model, c: &Cursor) -> Result<(), String> {
        self.hover(c);
        let marker = self.marker(c);
        match (self.tool, std::mem::replace(&mut self.op, Op::Idle)) {
            (Tool::Select, _) => self.select(model, c.hit.and_then(|h| h.face), c.hit.and_then(|h| h.instance), false, false, false),
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
            (Tool::FollowMe, Op::Idle) => {
                let face = c.hit.and_then(|h| h.face).ok_or("click the face to sweep (the profile)")?;
                self.op = Op::Follow { face };
            }
            (Tool::FollowMe, Op::Follow { face }) => {
                let Some(line) = c.hit.and_then(|h| h.line) else {
                    self.op = Op::Follow { face };
                    return Err("now click a line of the path to follow".into());
                };
                let mesh = model.mesh();
                let centre = {
                    let vs = mesh.face_vertices(face);
                    let mut c = [0.0f64; 3];
                    for &v in &vs {
                        c = add(c, mesh.positions[v as usize]);
                    }
                    scale(c, 1.0 / vs.len().max(1) as f64)
                };
                let (path, closed) = path_through(&mesh.lines, &mesh.positions, line, centre).ok_or("that line is not part of a path")?;
                model.follow_me(face, &path, closed).map_err(|e| e.to_string())?;
                self.model_changed();
            }
            (Tool::Dimensions, Op::Idle) => {
                if let Some((p, _)) = marker {
                    self.op = Op::Dim { a: p, b: None, cursor: None };
                }
            }
            (Tool::Dimensions, Op::Dim { a, b, cursor }) => match (b, marker) {
                (None, Some((p, _))) if dist(a, p) > 1e-6 => self.op = Op::Dim { a, b: Some(p), cursor: None },
                (Some(b), Some((p, _))) => {
                    let offset = dim_offset(a, b, p);
                    model.edit_notes(|n| n.dims.push(Dimension { a, b, offset }));
                }
                (b, _) => self.op = Op::Dim { a, b, cursor },
            },
            (Tool::SectionPlane, _) => {
                // on the face clicked, cutting away what is in front of it
                let face = c.hit.and_then(|h| h.face).ok_or("click a face to place the section plane on it")?;
                let n = model.mesh().face_normal(face);
                if let (Some(h), true) = (c.hit, n != [0.0; 3]) {
                    model.edit_notes(|notes| notes.section = Some(Section { point: h.point, normal: n, active: true }));
                }
            }
            (Tool::PaintBucket, _) => {
                // a selected face paints the whole selection, as in SketchUp
                if let Some(face) = c.hit.and_then(|h| h.face) {
                    let faces = if self.selected.binary_search(&face).is_ok() { self.selected.clone() } else { vec![face] };
                    model.paint(&faces, self.paint_color).map_err(|e| e.to_string())?;
                }
            }
            (Tool::Offset, Op::Idle) => {
                let face = c.hit.and_then(|h| h.face).ok_or("click a face to offset its edges")?;
                let ring: Vec<[f64; 3]> = model.mesh().face_vertices(face).iter().map(|&v| model.mesh().positions[v as usize]).collect();
                let normal = model.mesh().face_normal(face);
                if normal != [0.0; 3] {
                    self.op = Op::Offset { ring, normal, d: 0.0 };
                    self.hover(c);
                }
            }
            (Tool::Offset, Op::Offset { ring, normal, d }) => self.offset(model, &ring, normal, d)?,
            (Tool::TwoPointArc, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Arc2 { plane: Plane { origin: h.point, ..h.plane }, a: h.point, b: None, cursor: None };
                }
            }
            (Tool::TwoPointArc, Op::Arc2 { plane, a, b, .. }) => match (b, marker) {
                (None, Some((p, _))) if dist(a, p) > 1e-6 => self.op = Op::Arc2 { plane, a, b: Some(p), cursor: None },
                (Some(b), Some((p, _))) => {
                    let bulge = bulge_toward(&plane, a, b, p);
                    self.draw_arc(model, sketch::arc_by_bulge(&plane, a, b, bulge, self.arc_segments))?
                }
                (b, _) => self.op = Op::Arc2 { plane, a, b, cursor: None },
            },
            (Tool::ThreePointArc, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Arc3 { plane: Plane { origin: h.point, ..h.plane }, a: h.point, m: None, cursor: None };
                }
            }
            (Tool::ThreePointArc, Op::Arc3 { plane, a, m, .. }) => match (m, marker) {
                (None, Some((p, _))) if dist(a, p) > 1e-6 => self.op = Op::Arc3 { plane, a, m: Some(p), cursor: None },
                (Some(m), Some((p, _))) => self.draw_arc(model, sketch::arc_three_points(&plane, a, m, p, self.arc_segments))?,
                (m, _) => self.op = Op::Arc3 { plane, a, m, cursor: None },
            },
            (Tool::Arc, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Pie { plane: Plane { origin: h.point, ..h.plane }, center: h.point, start: None, cursor: None };
                }
            }
            (Tool::Arc, Op::Pie { plane, center, start, .. }) => match (start, marker) {
                (None, Some((p, _))) if dist(center, p) > 1e-6 => self.op = Op::Pie { plane, center, start: Some(p), cursor: None },
                (Some(st), Some((p, _))) => {
                    let sweep = sketch::angle_between(&plane, center, st, p).map(snap_angle).unwrap_or(0.0);
                    self.draw_arc(model, sketch::arc_from_center(&plane, center, st, sweep, self.arc_segments))?
                }
                (start, _) => self.op = Op::Pie { plane, center, start, cursor: None },
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
                let t = self.target(model, c).ok_or("select something to move, or point at a face or group")?;
                if let Some(h) = c.hit {
                    self.op = Op::Move { t, base: h.point, cursor: None, axis: None };
                }
            }
            (Tool::Move, Op::Move { t, base, cursor, axis }) => match marker {
                Some((p, _)) => {
                    let d = sub(p, base);
                    self.transform(model, &t, |q| add(q, d))?;
                }
                None => self.op = Op::Move { t, base, cursor, axis },
            },
            (Tool::Rotate, Op::Idle) => {
                let t = self.target(model, c).ok_or("select something to rotate, or point at a face or group")?;
                if let Some(h) = c.hit {
                    self.op = Op::Rotate { t, plane: Plane { origin: h.point, ..h.plane }, center: h.point, start: None, angle: 0.0 };
                }
            }
            (Tool::Rotate, Op::Rotate { t, plane, center, start, angle }) => match (start, marker) {
                (None, Some((p, _))) if dist(center, p) > 1e-6 => self.op = Op::Rotate { t, plane, center, start: Some(p), angle: 0.0 },
                (Some(_), _) => {
                    let n = plane.normal;
                    self.transform(model, &t, |q| rotate_about(q, center, n, angle))?;
                }
                (start, _) => self.op = Op::Rotate { t, plane, center, start, angle },
            },
            (Tool::Scale, Op::Idle) => {
                let t = self.target(model, c).ok_or("select something to scale, or point at a face or group")?;
                if let Some(h) = c.hit {
                    let anchor = opposite_corner(&target_points(model, &t), h.point);
                    if dist(anchor, h.point) < 1e-6 {
                        return Err("pick a point away from the middle of what is scaled".into());
                    }
                    self.op = Op::Scale { t, anchor, start: h.point, factor: 1.0 };
                }
            }
            (Tool::Scale, Op::Scale { t, anchor, factor, .. }) => self.transform(model, &t, |q| scale_about(q, anchor, factor))?,
            (Tool::TapeMeasure, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Tape { start: h.point, cursor: None };
                }
            }
            (Tool::TapeMeasure, Op::Tape { start, cursor }) => match marker {
                Some((p, _)) => self.add_tape_guides(model, start, p),
                None => self.op = Op::Tape { start, cursor },
            },
            (Tool::Protractor, Op::Idle) => {
                if let Some(h) = c.hit {
                    self.op = Op::Protract { plane: Plane { origin: h.point, ..h.plane }, center: h.point, start: None, angle: 0.0 };
                }
            }
            (Tool::Protractor, Op::Protract { plane, center, start, angle }) => match (start, marker) {
                (None, Some((p, _))) if dist(center, p) > 1e-6 => self.op = Op::Protract { plane, center, start: Some(p), angle: 0.0 },
                (Some(st), _) => self.add_protractor_guide(model, &plane, center, st, angle),
                (start, _) => self.op = Op::Protract { plane, center, start, angle },
            },
            (Tool::Text3D, Op::Place3D { text, height, depth, letters, at }) => {
                let Some(plane) = at else {
                    self.op = Op::Place3D { text, height, depth, letters, at };
                    return Err("click a face or the ground to place the text".into());
                };
                let mesh = crate::text3d::geometry(&text, height, depth, &plane)?;
                let i = model.place_group(&mesh, false).map_err(|e| e.to_string())?;
                self.set_tool(Tool::Select);
                self.model_changed();
                self.instances = vec![i];
            }
            (Tool::Text, Op::Idle) => {
                let h = c.hit.ok_or("click a point, edge or face for the text to point at")?;
                self.op = Op::Text { point: h.point, text: self.default_text(model, &h), cursor: None };
            }
            (Tool::Text, Op::Text { point, text, cursor }) => {
                let at = marker.map(|m| m.0).or(cursor).unwrap_or(point);
                model.edit_notes(|n| n.labels.push(Label { point, at, text }));
                self.edit_label = Some(model.notes().labels.len() - 1);
            }
            (Tool::Axes, Op::Idle) => {
                if let Some((p, _)) = marker {
                    self.op = Op::Axes { origin: p, red: None, cursor: None };
                }
            }
            (Tool::Axes, Op::Axes { origin, red: None, cursor }) => match marker.and_then(|(p, _)| normalize(sub(p, origin))) {
                Some(d) => self.op = Op::Axes { origin, red: Some(d), cursor: None },
                None => self.op = Op::Axes { origin, red: None, cursor },
            },
            (Tool::Axes, Op::Axes { origin, red: Some(red), cursor }) => {
                // green: the way to the click, square to red; blue makes a right-handed set
                let to = marker.map(|m| sub(m.0, origin)).unwrap_or([0.0; 3]);
                match normalize(sub(to, scale(red, dot(to, red)))) {
                    Some(green) => {
                        let blue = cross(red, green);
                        model.edit_notes(|n| n.axes = Axes { origin, dirs: [red, green, blue] });
                        self.axes = model.notes().axes;
                        self.set_tool(Tool::Select);
                    }
                    None => {
                        self.op = Op::Axes { origin, red: Some(red), cursor };
                        return Err("pick the green direction away from the red axis".into());
                    }
                }
            }
            // the Eraser takes a line under the mouse before the face behind it
            (Tool::Eraser, _) if c.hit.and_then(|h| h.line).is_some() => {
                model.erase_line(c.hit.and_then(|h| h.line).expect("checked")).map_err(|e| e.to_string())?;
            }
            (Tool::Eraser, _) if c.hit.and_then(|h| h.instance).is_some() => {
                model.erase_instances(&[c.hit.and_then(|h| h.instance).expect("checked")]).map_err(|e| e.to_string())?;
                self.model_changed();
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
        if let Some(r) = self.array(model, text) {
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
            (Entry::Sides(n), Op::Idle) if matches!(self.tool, Tool::Pie | Tool::Arc | Tool::TwoPointArc | Tool::ThreePointArc) => {
                self.arc_segments = n
            }
            (Entry::Sides(n), op @ (Op::Arc2 { .. } | Op::Arc3 { .. })) => {
                self.arc_segments = n;
                self.op = op;
            }
            (Entry::Value(d), Op::Offset { ring, normal, d: shown }) => {
                // typed: in or out as the mouse is
                self.offset(model, &ring, normal, d.abs() * if shown < 0.0 { -1.0 } else { 1.0 })?
            }
            (Entry::Value(d), Op::Idle) if self.tool == Tool::SectionPlane && model.notes().section.is_some() => {
                // typed: the plane moves that far into the model
                model.edit_notes(|n| {
                    if let Some(s) = n.section.as_mut() {
                        s.point = sub(s.point, scale(s.normal, d));
                    }
                });
            }
            (Entry::Value(l), Op::Arc2 { plane, a, b: None, cursor }) => {
                let dir = cursor.and_then(|c| normalize(sub(c, a))).unwrap_or(plane.u);
                self.op = Op::Arc2 { plane, a, b: Some(add(a, scale(dir, l.abs()))), cursor };
            }
            (Entry::Value(h), Op::Arc2 { plane, a, b: Some(b), cursor }) => {
                // the bulge, to the side of the chord the mouse is on
                let side = cursor.map(|c| bulge_toward(&plane, a, b, c).signum()).unwrap_or(1.0);
                self.draw_arc(model, sketch::arc_by_bulge(&plane, a, b, h.abs() * side, self.arc_segments))?;
            }
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
            (Entry::Value(d), Op::Move { t, base, cursor, .. }) => {
                let dir = cursor.and_then(|c| normalize(sub(c, base))).ok_or("move the mouse to show the direction first")?;
                let v = scale(dir, d);
                self.transform(model, &t, |q| add(q, v))?;
            }
            (Entry::Value(d), Op::Tape { start, cursor }) => {
                let dir = cursor.and_then(|c| normalize(sub(c, start))).ok_or("move the mouse to show the direction first")?;
                self.add_tape_guides(model, start, add(start, scale(dir, d)));
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
        let n: f64 = t.parse().ok().filter(|n: &f64| n.is_finite())?;
        let rad = n.to_radians();
        Some(match std::mem::replace(&mut self.op, Op::Idle) {
            Op::Rotate { t, plane, center, start: Some(_), .. } => self.transform(model, &t, |q| rotate_about(q, center, plane.normal, rad)),
            Op::Protract { plane, center, start: Some(st), .. } => {
                self.add_protractor_guide(model, &plane, center, st, rad);
                Ok(())
            }
            Op::Pie { plane, center, start: Some(st), .. } if self.tool == Tool::Arc => {
                self.draw_arc(model, sketch::arc_from_center(&plane, center, st, rad, self.arc_segments))
            }
            Op::Pie { plane, center, start: Some(st), .. } => self.commit(model, sketch::pie(&plane, center, st, rad, self.arc_segments)),
            Op::Scale { t, anchor, .. } if n != 0.0 => self.transform(model, &t, |q| scale_about(q, anchor, n)),
            Op::Scale { .. } => Err("a scale of 0 would flatten it to a point".into()),
            op => {
                self.op = op;
                return None;
            }
        })
    }

    /// Offset: the face's outline copied `d` inside it, as lines (so the face gets an inner face and a ring around it).
    fn offset(&mut self, model: &mut Model, ring: &[[f64; 3]], normal: [f64; 3], d: f64) -> Result<(), String> {
        if d < 0.0 {
            self.op = Op::Idle;
            return Err("offsetting outward is not built yet; move the mouse inside the face".into());
        }
        let r = sketch::offset_ring(ring, normal, d).ok_or("that is too far in for this face")?;
        self.op = Op::Idle;
        self.commit(model, Some(r))
    }

    /// Draws an arc (open, as lines) into the model.
    fn draw_arc(&mut self, model: &mut Model, arc: Option<Vec<[f64; 3]>>) -> Result<(), String> {
        let arc = arc.ok_or("the arc has no size")?;
        model.add_polyline(&arc).map_err(|e| e.to_string())?;
        self.op = Op::Idle;
        Ok(())
    }

    /// Freehand: the button went down here.
    pub fn freehand_press(&mut self, c: &Cursor) {
        if self.tool == Tool::Freehand {
            if let Some(h) = c.hit {
                self.op = Op::Free { plane: Plane { origin: h.point, ..h.plane }, points: vec![h.point] };
            }
        }
    }

    /// Freehand: the button came up; the stroke goes into the model as lines.
    pub fn freehand_release(&mut self, model: &mut Model) -> Result<(), String> {
        let Op::Free { points, .. } = std::mem::replace(&mut self.op, Op::Idle) else { return Ok(()) };
        if points.len() < 2 {
            return Ok(());
        }
        model.add_polyline(&points).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn add_tape_guides(&mut self, model: &mut Model, start: [f64; 3], end: [f64; 3]) {
        if let Some(dir) = normalize(sub(end, start)) {
            model.edit_notes(|n| n.guides.extend([Guide::Line(start, dir), Guide::Point(end)]));
        }
        self.op = Op::Idle;
    }

    fn add_protractor_guide(&mut self, model: &mut Model, plane: &Plane, center: [f64; 3], start: [f64; 3], angle: f64) {
        if let Some(dir) = normalize(sub(start, center)) {
            let g = Guide::Line(center, rotate_about(dir, [0.0; 3], plane.normal, angle));
            model.edit_notes(|n| n.guides.push(g));
        }
        self.op = Op::Idle;
    }

    /// Mesh to draw instead of the model while push/pull, Move, Rotate or Scale is live.
    pub fn preview_mesh(&self, model: &Model) -> Option<HalfEdgeMesh> {
        let moved = |verts: &[u32], f: &dyn Fn([f64; 3]) -> [f64; 3]| {
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
            // Move with Ctrl: the faces stay, and a copy follows the mouse
            Op::Move { t, base, cursor: Some(c), .. } if self.copy && !t.faces.is_empty() => {
                let mesh = model.mesh().to_mesh();
                let (_, part) = mesh.extract(&t.faces);
                let mut both = mesh.clone();
                both.append(&part.transformed(&Xform::translation(sub(*c, *base))));
                HalfEdgeMesh::from_mesh(&both).ok()
            }
            Op::Move { t, base, cursor: Some(c), .. } if !t.verts.is_empty() && !self.copy => {
                let d = sub(*c, *base);
                moved(&t.verts, &|q| add(q, d))
            }
            Op::Rotate { t, plane, center, start: Some(_), angle } if *angle != 0.0 && !t.verts.is_empty() => {
                moved(&t.verts, &|q| rotate_about(q, *center, plane.normal, *angle))
            }
            Op::Scale { t, anchor, factor, .. } if *factor != 1.0 && !t.verts.is_empty() => moved(&t.verts, &|q| scale_about(q, *anchor, *factor)),
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
    pub fn preview_lines(&self) -> Vec<([f64; 3], [f64; 3], [f32; 4])> {
        let mut out = Vec::new();
        match &self.op {
            Op::Text { point, cursor: Some(c), .. } => out.push((*point, *c, RUBBER_BAND)),
            Op::Place3D { letters, at: Some(plane), .. } => {
                for ring in letters.iter().flatten() {
                    let r: Vec<[f64; 3]> = ring.iter().map(|p| plane.to_world(p[0], p[1])).collect();
                    ring_lines(&r, &mut out);
                }
            }
            Op::Axes { origin, red, cursor } => {
                let reach = cursor.map_or(1.0, |c| dist(*origin, c)).max(0.2);
                match (red, cursor) {
                    (None, Some(c)) => out.push((*origin, *c, AXIS_COLORS[0])),
                    (Some(r), c) => {
                        out.push((*origin, add(*origin, scale(*r, reach)), AXIS_COLORS[0]));
                        if let Some(c) = c {
                            let to = sub(*c, *origin);
                            if let Some(g) = normalize(sub(to, scale(*r, dot(to, *r)))) {
                                out.push((*origin, add(*origin, scale(g, reach)), AXIS_COLORS[1]));
                                out.push((*origin, add(*origin, scale(cross(*r, g), reach)), AXIS_COLORS[2]));
                            }
                        }
                    }
                    _ => {}
                }
            }
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
            Op::Arc2 { plane, a, b, cursor: Some(c) } => match b {
                None => out.push((*a, *c, RUBBER_BAND)),
                Some(b) => {
                    if let Some(arc) = sketch::arc_by_bulge(plane, *a, *b, bulge_toward(plane, *a, *b, *c), self.arc_segments) {
                        chain_lines(&arc, &mut out);
                    }
                }
            },
            Op::Arc3 { plane, a, m, cursor: Some(c) } => match m {
                None => out.push((*a, *c, RUBBER_BAND)),
                Some(m) => match sketch::arc_three_points(plane, *a, *m, *c, self.arc_segments) {
                    Some(arc) => chain_lines(&arc, &mut out),
                    None => out.push((*a, *c, RUBBER_BAND)),
                },
            },
            Op::Free { points, .. } => chain_lines(points, &mut out),
            Op::Dim { a, b, cursor: Some(c) } => match b {
                None => out.push((*a, *c, PUSH_GUIDE)),
                Some(b) => out.extend(dim_lines(*a, *b, dim_offset(*a, *b, *c), PUSH_GUIDE)),
            },
            Op::Offset { ring, normal, d } => {
                if let Some(r) = sketch::offset_ring(ring, *normal, *d) {
                    ring_lines(&r, &mut out);
                }
            }
            Op::Pie { plane, center, start: Some(st), cursor: Some(c) } if self.tool == Tool::Arc => {
                out.push((*center, *st, PUSH_GUIDE));
                let sweep = sketch::angle_between(plane, *center, *st, *c).map(snap_angle).unwrap_or(0.0);
                if let Some(arc) = sketch::arc_from_center(plane, *center, *st, sweep, self.arc_segments) {
                    chain_lines(&arc, &mut out);
                }
            }
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
                let ring: Vec<[f64; 3]> = (0..36)
                    .map(|i| {
                        let a = std::f64::consts::TAU * i as f64 / 36.0;
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
            (Tool::Select, _) => "Click to select. Ctrl adds, Shift toggles, a triple-click takes everything joined. Double-click a group to edit it.",
            (Tool::RotatedRectangle, Op::Idle) => "Select first corner.",
            (Tool::RotatedRectangle, Op::RotRect { b: None, .. }) => "Select second corner, or type a length. The first side may point any way.",
            (Tool::RotatedRectangle, _) => "Select the height, or type it.",
            (Tool::Pie | Tool::Arc, Op::Idle) => "Select center point. Type a number followed by s to set segments.",
            (Tool::Arc, Op::Pie { start: None, .. }) => "Select start of the arc, or type a radius.",
            (Tool::Arc, _) => "Select end of the arc, or type an angle in degrees.",
            (Tool::TwoPointArc, Op::Idle) => "Select start point. Type a number followed by s to set segments.",
            (Tool::TwoPointArc, Op::Arc2 { b: None, .. }) => "Select end point, or type the length.",
            (Tool::TwoPointArc, _) => "Move to bow the arc and click, or type the bulge.",
            (Tool::ThreePointArc, Op::Idle) => "Select start point.",
            (Tool::ThreePointArc, Op::Arc3 { m: None, .. }) => "Select a point the arc passes through.",
            (Tool::ThreePointArc, _) => "Select end point.",
            (Tool::Freehand, _) => "Hold the button and draw.",
            (Tool::Offset, Op::Idle) => "Click a face to offset its edges.",
            (Tool::FollowMe, Op::Idle) => "Click the face to sweep (the profile). Draw the path as lines first.",
            (Tool::FollowMe, _) => "Click a line of the path. The profile follows the whole chain of lines.",
            (Tool::Dimensions, Op::Idle) => "Click the first point to measure (an endpoint snaps).",
            (Tool::Dimensions, Op::Dim { b: None, .. }) => "Click the second point.",
            (Tool::Dimensions, _) => "Move out to place the dimension line and click.",
            (Tool::SectionPlane, _) => "Click a face: what is in front of it is cut away. Type a distance to move the plane in.",
            (Tool::PaintBucket, _) => "Click a face to paint it (a selected face paints the selection). Alt+click picks up a face's colour.",
            (Tool::Offset, _) => "Move in to set the distance and click, or type it.",
            (Tool::Pie, Op::Pie { start: None, .. }) => "Select start of the arc, or type a radius.",
            (Tool::Pie, _) => "Select end of the arc, or type an angle in degrees.",
            (Tool::Move, Op::Idle) => "Pick a point to move from. Moves the selection, or the face under the mouse.",
            (Tool::Move, _) if self.copy => "Copy: pick where the copy goes, or type a distance. Then type x5 for an array, /5 to divide.",
            (Tool::Move, _) => "Pick the point to move to, or type a distance. Close to an axis it locks onto it. Ctrl copies.",
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
            (Tool::Text, Op::Idle) => "Click what the text points at (a face shows its area, a corner its position), or empty space.",
            (Tool::Text, _) => "Click where the text goes. Then type the words and press Enter.",
            (Tool::Axes, Op::Idle) => "Pick the new origin of the drawing axes. Tools > Reset Axes puts them back.",
            (Tool::Axes, Op::Axes { red: None, .. }) => "Pick the direction of the red axis.",
            (Tool::Axes, _) => "Pick the direction of the green axis (blue follows).",
            (Tool::Text3D, Op::Idle) => "Type the words in the 3D Text window, then Place.",
            (Tool::Text3D, _) => "Click a face or the ground to place the text. It lies on the face and becomes a group.",
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
            (Tool::Eraser, _) => "Click an edge, line, face or group to erase it. Near an edge, the edge goes.",
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
            Op::Arc2 { a, b: None, cursor, .. } => ("Length", cursor.map(|c| fmt(dist(*a, c), self.units)).unwrap_or_default()),
            Op::Arc2 { plane, a, b: Some(b), cursor } => ("Bulge", cursor.map(|c| fmt(bulge_toward(plane, *a, *b, c).abs(), self.units)).unwrap_or_default()),
            Op::Arc3 { .. } | Op::Free { .. } | Op::Follow { .. } | Op::Text { .. } | Op::Axes { .. } | Op::Place3D { .. } => ("Measurements", String::new()),
            Op::Dim { a, b, cursor } => ("Length", b.or(*cursor).map(|b| fmt(dist(*a, b), self.units)).unwrap_or_default()),
            Op::Offset { d, .. } => ("Distance", fmt(d.abs(), self.units)),
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
                Tool::Pie | Tool::Arc | Tool::TwoPointArc | Tool::ThreePointArc => ("Segments", self.arc_segments.to_string()),
                Tool::PushPull => ("Distance", self.last_push.map(|(_, d)| fmt(d, self.units)).unwrap_or_default()),
                _ => ("Measurements", String::new()),
            },
        }
    }

    /// What new text says before it is typed over, as in SketchUp: a group's name, a corner's position, a face's area.
    fn default_text(&self, model: &Model, h: &Hit) -> String {
        if let Some(def) = h.instance.and_then(|i| model.instances().get(i)).and_then(|i| model.definition(i.def)) {
            return def.name.clone();
        }
        if matches!(h.snap, Snap::Endpoint | Snap::Origin | Snap::Midpoint) {
            let p = h.point;
            return format!("({}, {}, {})", fmt(p[0], self.units), fmt(p[1], self.units), fmt(p[2], self.units));
        }
        if let Some(f) = h.face.filter(|&f| f < model.mesh().face_count()) {
            let mesh = model.mesh();
            let ring: Vec<[f64; 3]> = mesh.face_vertices(f).iter().map(|&v| mesh.positions[v as usize]).collect();
            return units::format_area(polygon_area(&ring), self.units);
        }
        "Enter text".into()
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
    fn add_line_point(&mut self, model: &mut Model, plane: Plane, last: [f64; 3], p: [f64; 3]) -> Result<(), String> {
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

    fn commit(&mut self, model: &mut Model, outline: Option<Vec<[f64; 3]>>) -> Result<(), String> {
        let outline = outline.ok_or("the shape has no area")?;
        // drawn as lines, so a shape on a face splits it or cuts a hole in it, as in SketchUp
        model.add_outline(&outline).map_err(|e| e.to_string())?;
        self.model_changed();
        Ok(())
    }

    fn push(&mut self, model: &mut Model, face: usize, distance: f64) -> Result<(), String> {
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
fn on_plane(plane: &Plane, c: &Cursor) -> Option<([f64; 3], Snap)> {
    if let Some(h) = c.hit {
        let off = dot(sub(h.point, plane.origin), plane.normal).abs();
        if matches!(h.snap, Snap::Endpoint | Snap::Midpoint) && off <= 1e-4 * (1.0 + len(h.point)) {
            return Some((h.point, h.snap));
        }
    }
    plane.intersect_ray(c.origin, c.dir).map(|p| (p, Snap::InPlane))
}

/// Locks `p` onto the plane axis through `from` when within about 3 degrees of it.
fn axis_lock(plane: &Plane, from: [f64; 3], p: [f64; 3], axes: &[[f64; 3]; 3]) -> ([f64; 3], Option<u8>) {
    let tol = 3f64.to_radians().tan();
    let d = sub(p, from);
    for axis in [plane.u, plane.v] {
        let along = dot(d, axis);
        let across = len(sub(d, scale(axis, along)));
        if along.abs() > 1e-9 && across <= tol * along.abs() {
            let world = (0..3).find(|&i| dot(axis, axes[i]).abs() > 0.999).map_or(3, |i| i as u8);
            return (add(from, scale(axis, along)), Some(world));
        }
    }
    (p, None)
}

/// Rounds an angle to a multiple of 15 degrees when within 2 degrees of it, as the protractor does in SketchUp.
fn snap_angle(a: f64) -> f64 {
    let step = 15f64.to_radians();
    let near = (a / step).round() * step;
    if (a - near).abs() <= 2f64.to_radians() {
        near
    } else {
        a
    }
}

/// `p` turned `angle` radians about the line through `center` along unit `axis` (right-handed).
fn rotate_about(p: [f64; 3], center: [f64; 3], axis: [f64; 3], angle: f64) -> [f64; 3] {
    let v = sub(p, center);
    let (s, c) = angle.sin_cos();
    // Rodrigues: v cos + (k x v) sin + k (k . v)(1 - cos)
    let r = add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)));
    add(center, r)
}

fn scale_about(p: [f64; 3], anchor: [f64; 3], factor: f64) -> [f64; 3] {
    add(anchor, scale(sub(p, anchor), factor))
}

/// The corner of the vertices' bounding box opposite `grip`: on each axis, the far side from it (or the grip's own
/// coordinate when the box has no size that way, so a flat shape scales in its plane).
fn opposite_corner(pts: &[[f64; 3]], grip: [f64; 3]) -> [f64; 3] {
    let mut out = grip;
    for i in 0..3 {
        let lo = pts.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min);
        let hi = pts.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max);
        if hi - lo > 1e-6 {
            out[i] = if (grip[i] - lo).abs() < (grip[i] - hi).abs() { hi } else { lo };
        }
    }
    out
}

/// The points of what is transformed: its vertices, and the corners of its groups as they stand in the model.
fn target_points(model: &Model, t: &Target) -> Vec<[f64; 3]> {
    let mut pts: Vec<[f64; 3]> = t.verts.iter().filter_map(|&v| model.mesh().positions.get(v as usize).copied()).collect();
    let (inside, _) = if t.inst.is_empty() { (Vec::new(), daedalus_core::Mesh::new()) } else { model.parts() };
    for (i, m) in inside {
        if t.inst.contains(&i) {
            pts.extend(m.positions);
        }
    }
    pts
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
fn ray_axis_lock(from: [f64; 3], c: &Cursor, axes: &[[f64; 3]; 3]) -> Option<([f64; 3], u8)> {
    let tol = 1.5f64.to_radians().tan();
    let mut best: Option<([f64; 3], u8, f64)> = None;
    for (i, &axis) in axes.iter().enumerate() {
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
fn closest_on_line(base: [f64; 3], dir_line: [f64; 3], origin: [f64; 3], dir: [f64; 3]) -> Option<f64> {
    let w0 = sub(base, origin);
    let b = dot(dir_line, dir);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    Some((b * dot(dir, w0) - dot(dir_line, w0)) / denom)
}

/// Follow Me's path: the chain of loose lines through line `line` (as far as it runs without branching), as points in
/// order starting from the end nearest `near` (the profile), and whether it closes on itself.
fn path_through(lines: &[[u32; 2]], positions: &[[f64; 3]], line: usize, near: [f64; 3]) -> Option<(Vec<[f64; 3]>, bool)> {
    let &[a, b] = lines.get(line)?;
    let mut by_vertex: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
    for (i, &[x, y]) in lines.iter().enumerate() {
        by_vertex.entry(x).or_default().push(i);
        by_vertex.entry(y).or_default().push(i);
    }
    // from `start` (an end of the clicked line), on along the chain while it does not branch; `back` is the clicked
    // line's other end: reaching it means the chain closes
    let walk = |start: u32, back: u32| -> (Vec<u32>, bool) {
        let (mut v, mut l, mut out) = (start, line, vec![]);
        loop {
            let others: Vec<usize> = by_vertex[&v].iter().copied().filter(|&o| o != l).collect();
            if others.len() != 1 || out.len() > lines.len() {
                return (out, false);
            }
            l = others[0];
            let [x, y] = lines[l];
            let next = if x == v { y } else { x };
            if next == back {
                return (out, true);
            }
            out.push(next);
            v = next;
        }
    };
    let (ahead, closed) = walk(b, a);
    if closed {
        let mut pts: Vec<[f64; 3]> = [a, b].iter().chain(ahead.iter()).map(|&v| positions[v as usize]).collect();
        // start at the corner nearest the profile
        let start = (0..pts.len()).min_by(|&i, &j| dist(pts[i], near).total_cmp(&dist(pts[j], near)))?;
        pts.rotate_left(start);
        return Some((pts, true));
    }
    let (behind, _) = walk(a, b);
    let mut chain: Vec<u32> = behind.into_iter().rev().collect();
    chain.push(a);
    chain.push(b);
    chain.extend(ahead);
    let mut pts: Vec<[f64; 3]> = chain.iter().map(|&v| positions[v as usize]).collect();
    if dist(*pts.last()?, near) < dist(pts[0], near) {
        pts.reverse();
    }
    Some((pts, false))
}

/// Where a dimension's line sits: the part of `p - a` square to the measured line `a`-`b`.
fn dim_offset(a: [f64; 3], b: [f64; 3], p: [f64; 3]) -> [f64; 3] {
    let v = sub(p, a);
    match normalize(sub(b, a)) {
        Some(d) => sub(v, scale(d, dot(v, d))),
        None => v,
    }
}

/// A dimension's lines: one out from each point, and the dimension line between them with small ticks at its ends.
fn dim_lines(a: [f64; 3], b: [f64; 3], off: [f64; 3], color: [f32; 4]) -> Vec<([f64; 3], [f64; 3], [f32; 4])> {
    let (a2, b2) = (add(a, off), add(b, off));
    let mut out = vec![(a, a2, color), (b, b2, color), (a2, b2, color)];
    if let (Some(d), Some(o)) = (normalize(sub(b, a)), normalize(off)) {
        let tick = dist(a, b) * 0.03;
        for p in [a2, b2] {
            out.push((sub(p, scale(add(d, o), tick)), add(p, scale(add(d, o), tick)), color));
        }
    }
    out
}

/// How far `p` lies inside the outline `ring` (in its plane): the distance to the nearest side, negative outside.
fn inside_distance(ring: &[[f64; 3]], normal: [f64; 3], p: [f64; 3]) -> f64 {
    let Some(plane) = Plane::from_point_normal(ring[0], normal) else { return 0.0 };
    let pts: Vec<[f64; 2]> = ring.iter().map(|&q| plane.to_local(q)).collect();
    let q = plane.to_local(p);
    let n = pts.len();
    let mut nearest = f64::INFINITY;
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 { (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        nearest = nearest.min((q[0] - a[0] - t * dx).hypot(q[1] - a[1] - t * dy));
        if (a[1] > q[1]) != (b[1] > q[1]) && q[0] < a[0] + (q[1] - a[1]) / (b[1] - a[1]) * dx {
            inside = !inside;
        }
    }
    if inside {
        nearest
    } else {
        -nearest
    }
}

/// The area of a flat polygon.
fn polygon_area(ring: &[[f64; 3]]) -> f64 {
    let mut n = [0.0f64; 3];
    for i in 0..ring.len() {
        n = add(n, cross(ring[i], ring[(i + 1) % ring.len()]));
    }
    len(n) / 2.0
}

fn chain_lines(r: &[[f64; 3]], out: &mut Vec<([f64; 3], [f64; 3], [f32; 4])>) {
    for w in r.windows(2) {
        out.push((w[0], w[1], RUBBER_BAND));
    }
}

/// The bulge of a 2 Point Arc from `a` to `b` toward `p`: how far `p` is from the chord, positive on its left.
fn bulge_toward(plane: &Plane, a: [f64; 3], b: [f64; 3], p: [f64; 3]) -> f64 {
    let (la, lb, lp) = (plane.to_local(a), plane.to_local(b), plane.to_local(p));
    let (dx, dy) = (lb[0] - la[0], lb[1] - la[1]);
    let l = dx.hypot(dy);
    if l < 1e-9 {
        return 0.0;
    }
    ((lp[0] - la[0]) * -dy + (lp[1] - la[1]) * dx) / l
}

fn ring_lines(r: &[[f64; 3]], out: &mut Vec<([f64; 3], [f64; 3], [f32; 4])>) {
    for i in 0..r.len() {
        out.push((r[i], r[(i + 1) % r.len()], RUBBER_BAND));
    }
}

fn fmt(v: f64, u: Units) -> String {
    units::format(v, u)
}

fn sign(v: f64) -> f64 {
    if v < 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    len(sub(a, b))
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = len(a);
    (l > 1e-9).then(|| scale(a, 1.0 / l))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cursor looking straight down at (x, y) on the ground.
    fn at(x: f64, y: f64) -> Cursor {
        ground_hit([x, y, 0.0], Snap::Ground)
    }

    fn ground_hit(point: [f64; 3], snap: Snap) -> Cursor {
        Cursor {
            origin: [point[0], point[1], 10.0],
            dir: [0.0, 0.0, -1.0],
            hit: Some(Hit { point, plane: Plane::ground(), face: None, line: None, instance: None, snap }),
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
            hit: Some(Hit { point: [1.0, 1.0, 2.0], plane: Plane::from_point_normal([1.0, 1.0, 2.0], [0.0, 0.0, 1.0]).unwrap(), face: Some(top), line: None, instance: None, snap: Snap::OnFace }),
        };
        (m, c)
    }

    fn bounds(m: &Model) -> ([f64; 3], [f64; 3]) {
        let mesh = m.mesh().to_mesh();
        mesh.bounds().unwrap()
    }

    #[test]
    fn edges_are_selected_and_erased_like_faces() {
        let (mut m, top) = box_model();
        let mut t = Tools::default();
        // a top edge: two corners of the top face that follow each other round it
        let ring = m.mesh().face_vertices(top.hit.unwrap().face.unwrap());
        let edge = [ring[1], ring[0]];
        t.select_edge(edge, false, false);
        assert_eq!(t.edges, vec![[ring[0].min(ring[1]), ring[0].max(ring[1])]]);
        assert!(t.has_selection());
        // a face click replaces it; Ctrl adds it back; Shift takes it out again
        t.select(&m, top.hit.unwrap().face, None, false, false, false);
        assert!(t.edges.is_empty());
        t.select_edge(edge, true, false);
        assert_eq!((t.selected.len(), t.edges.len()), (1, 1));
        t.select_edge(edge, false, true);
        assert!(t.edges.is_empty());
        // Delete on the edge alone: the two faces on it go (box edges are not between faces in one plane)
        t.select_edge(edge, false, false);
        assert_eq!(m.erase_all(&t.selected, &t.edges, &t.instances), Ok(0));
        assert_eq!(m.mesh().face_count(), 4);
        assert!(m.undo());
        assert_eq!(m.mesh().face_count(), 6);
    }

    #[test]
    fn triple_click_selects_the_whole_box_and_move_carries_it() {
        let (mut m, top) = box_model();
        let mut t = Tools::default();
        t.select(&m, top.hit.unwrap().face, None, false, false, true);
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
        t.select(&m, top.hit.unwrap().face, None, false, false, true);
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
        assert!((snap_angle(44.0f64.to_radians()) - 45f64.to_radians()).abs() < 1e-6);
        assert!((snap_angle(40.0f64.to_radians()) - 40f64.to_radians()).abs() < 1e-6);
    }

    #[test]
    fn scale_by_a_factor_from_the_opposite_corner() {
        let (mut m, _) = box_model();
        let mut t = Tools::default();
        t.select_all(&m);
        t.set_tool(Tool::Scale);
        // grip the top corner at (2,2,2): the anchor is the corner at the origin
        let grip = Cursor { origin: [2.0, 2.0, 10.0], dir: [0.0, 0.0, -1.0], hit: Some(Hit { point: [2.0, 2.0, 2.0], plane: Plane::ground(), face: None, line: None, instance: None, snap: Snap::Endpoint }) };
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
        assert_eq!(m.notes().guides.len(), 2);
        match m.notes().guides[1] {
            Guide::Point(p) => assert!((p[0] - 6.0).abs() < 1e-5 && (p[1] - 8.0).abs() < 1e-5),
            g => panic!("{g:?}"),
        }
        assert_eq!(m.mesh().face_count(), 0, "guides are not geometry");
        m.edit_notes(|n| n.guides.clear());
        assert!(m.notes().guides.is_empty());
    }

    #[test]
    fn protractor_guide_at_a_typed_angle() {
        let (mut t, mut m) = with(Tool::Protractor);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.click(&mut m, &at(1.0, 0.0)).unwrap();
        t.type_entry(&mut m, "30").unwrap();
        match m.notes().guides[0] {
            Guide::Line(p, d) => {
                assert_eq!(p, [0.0; 3]);
                assert!((d[0] - 30f64.to_radians().cos()).abs() < 1e-5 && (d[1] - 0.5).abs() < 1e-5);
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
            point: [0.5, 0.5, 0.0], plane: Plane::ground(), face: Some(0), line: None, instance: None, snap: Snap::OnFace,
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
        let c = at(5.0, 5.0 * 1f64.to_radians().tan());
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
            point: [1.0, 1.0, 0.0], plane: Plane::ground(), face: Some(0), line: None, instance: None, snap: Snap::OnFace,
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

    #[test]
    fn an_arc_and_a_line_make_a_half_disc() {
        let (mut t, mut m) = with(Tool::TwoPointArc);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.click(&mut m, &at(2.0, 0.0)).unwrap();
        t.hover(&at(1.0, 0.5));
        t.type_entry(&mut m, "1").unwrap();
        assert_eq!(m.mesh().lines.len(), 12, "a 12-piece arc");
        let mesh = m.mesh().to_mesh();
        assert!(mesh.positions.iter().any(|p| (p[0] - 1.0).abs() < 1e-5 && (p[1] - 1.0).abs() < 1e-5), "bowed toward the mouse");
        // close it with a line along the chord
        t.set_tool(Tool::Line);
        t.click(&mut m, &ground_hit([2.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        t.click(&mut m, &ground_hit([0.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        assert_eq!(m.mesh().face_count(), 1);
        m.push_pull(0, 1.0).unwrap();
        let half_disc = 0.5 * 12.0 * (std::f64::consts::PI / 12.0).sin();
        assert!((m.mesh().signed_volume() - half_disc).abs() < 1e-3, "{}", m.mesh().signed_volume());
    }

    #[test]
    fn three_point_arc_and_freehand() {
        let (mut t, mut m) = with(Tool::ThreePointArc);
        for (x, y) in [(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)] {
            t.click(&mut m, &at(x, y)).unwrap();
        }
        assert_eq!(m.mesh().lines.len(), 12);

        let (mut t, mut m) = with(Tool::Freehand);
        t.freehand_press(&at(0.0, 0.0));
        for i in 1..=50 {
            let x = i as f64 * 0.05;
            t.hover(&at(x, (x * 3.0).sin()));
        }
        t.freehand_release(&mut m).unwrap();
        assert!(m.mesh().lines.len() > 10, "a stroke of many short lines: {}", m.mesh().lines.len());
        assert!(!t.is_busy());
    }

    #[test]
    fn offset_a_face_then_push_the_middle_up() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]).unwrap();
        let mut t = Tools::default();
        t.set_tool(Tool::Offset);
        let mut c = at(2.0, 2.0);
        c.hit.as_mut().unwrap().face = Some(0);
        t.click(&mut m, &c).unwrap();
        // the mouse 1 m in from the left side
        t.hover(&at(1.0, 2.0));
        assert_eq!(t.measurement(), ("Distance", "1 m".into()));
        t.click(&mut m, &at(1.0, 2.0)).unwrap();
        assert_eq!(m.mesh().face_count(), 2, "the middle, and the ring around it (one face with a hole)");
        assert!(m.mesh().has_holes(0) || m.mesh().has_holes(1));
        let middle = (0..2).find(|&f| m.mesh().face_vertices(f).len() == 4 && {
            let p = m.mesh().positions[m.mesh().face_vertices(f)[0] as usize];
            (1.0..=3.0).contains(&p[0]) && (1.0..=3.0).contains(&p[1])
        });
        let middle = middle.expect("the 2 x 2 middle face");
        m.push_pull(middle, 1.0).unwrap();
        // the ring stays flat; the middle is a 2 x 2 x 1 block standing on it
        let up: f64 = m.mesh().positions.iter().filter(|p| p[2] > 0.5).count() as f64;
        assert_eq!(up, 4.0);
        // too far in
        t.click(&mut m, &c).unwrap();
        t.hover(&at(2.0, 2.0));
        assert!(t.type_entry(&mut m, "3").is_err());
    }

    #[test]
    fn follow_me_sweeps_a_profile_along_drawn_lines() {
        let mut m = Model::new();
        // the profile: a 1 x 1 square standing at the start of the path, across it
        m.add_rectangle([0.0, -0.5, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        // the path, drawn with the Line tool: 3 m along +X, then 2 m along +Y
        let mut t = Tools::default();
        t.set_tool(Tool::Line);
        t.click(&mut m, &at(0.0, 0.0)).unwrap();
        t.click(&mut m, &at(3.0, 0.0)).unwrap();
        t.click(&mut m, &at(3.0, 2.0)).unwrap();
        t.cancel();
        assert_eq!(m.mesh().lines.len(), 2);
        t.set_tool(Tool::FollowMe);
        let mut c = at(0.0, 0.0);
        c.hit.as_mut().unwrap().face = Some(0);
        t.click(&mut m, &c).unwrap();
        // the second line of the path: the whole chain is followed
        let mut c = at(3.0, 1.0);
        c.hit.as_mut().unwrap().line = Some(1);
        t.click(&mut m, &c).unwrap();
        assert!(m.mesh().is_closed());
        assert!((m.mesh().signed_volume() - 5.0).abs() < 1e-3, "{}", m.mesh().signed_volume());
    }

    #[test]
    fn a_section_plane_on_a_face_moves_in_by_a_typed_distance() {
        let (mut m, top) = box_model();
        let mut t = Tools::default();
        t.set_tool(Tool::SectionPlane);
        t.click(&mut m, &top).unwrap();
        let (o, n) = { let s = m.notes().section.unwrap(); (s.point, s.normal) };
        assert_eq!(n, [0.0, 0.0, 1.0], "it cuts away what is above the top");
        assert!((o[2] - 2.0).abs() < 1e-6);
        t.type_entry(&mut m, "0.5").unwrap();
        assert!((m.notes().section.unwrap().point[2] - 1.5).abs() < 1e-6, "half a metre down into the box");
        assert_eq!(t.section_outline(&m, 1.0).len(), 5);
        assert_eq!(m.mesh().face_count(), 6, "the model is not changed");
    }

    #[test]
    fn a_dimension_between_two_points() {
        let (mut t, mut m) = with(Tool::Dimensions);
        t.click(&mut m, &ground_hit([0.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        t.click(&mut m, &ground_hit([3.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        // placed 1 m out to the side (the mouse a little along the line too: only the square part counts)
        t.click(&mut m, &at(2.0, -1.0)).unwrap();
        assert_eq!(m.notes().dims.len(), 1);
        assert_eq!(m.notes().dims[0].offset, [0.0, -1.0, 0.0]);
        let labels = t.dim_labels(&m);
        assert_eq!(labels[0], ([1.5, -1.0, 0.0], "3 m".to_string()));
        t.units = Units::Millimetres;
        assert_eq!(t.dim_labels(&m)[0].1, "3000 mm", "it reads in the units shown");
        assert_eq!(m.mesh().face_count() + m.mesh().lines.len(), 0, "not geometry");
    }

    /// The 2 m box of `box_model`, made into a component and selected.
    fn grouped_box() -> (Tools, Model) {
        let (mut m, top) = box_model();
        let mut t = Tools::default();
        t.select(&m, top.hit.unwrap().face, None, false, false, true);
        let g = m.make_group(&t.selected.clone(), &[], true).unwrap();
        t.model_changed();
        t.instances = vec![g];
        (t, m)
    }

    #[test]
    fn move_carries_a_group_whole() {
        let (mut t, mut m) = grouped_box();
        t.set_tool(Tool::Move);
        t.click(&mut m, &ground_hit([0.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        t.hover(&ground_hit([0.0, 5.0, 0.0], Snap::Endpoint));
        let (ids, xf, copy) = t.preview_instances().unwrap();
        assert_eq!((ids, xf.origin(), copy), (vec![0], [0.0, 5.0, 0.0], false));
        t.click(&mut m, &ground_hit([0.0, 5.0, 0.0], Snap::Endpoint)).unwrap();
        let (lo, hi) = m.flatten().bounds().unwrap();
        assert_eq!((lo[1], hi[1]), (5.0, 7.0));
        assert_eq!(t.instances, vec![0], "it stays selected");
    }

    #[test]
    fn ctrl_move_copies_then_x4_makes_an_array() {
        let (mut t, mut m) = grouped_box();
        t.set_tool(Tool::Move);
        t.click(&mut m, &ground_hit([0.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        t.copy = true;
        t.click(&mut m, &ground_hit([3.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        assert_eq!(m.instances().len(), 2);
        assert_eq!(t.instances, vec![1], "the copy is selected");
        assert!(t.can_array());
        t.type_entry(&mut m, "x4").unwrap();
        assert_eq!(m.instances().len(), 5, "the original and four copies, as SketchUp counts x4");
        let (_, hi) = m.flatten().bounds().unwrap();
        assert!((hi[0] - 14.0).abs() < 1e-4, "{hi:?}");
        m.undo();
        assert_eq!(m.instances().len(), 2, "the array is one undo step");
    }

    #[test]
    fn divide_puts_copies_between() {
        let (mut t, mut m) = grouped_box();
        t.set_tool(Tool::Move);
        t.click(&mut m, &ground_hit([0.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        t.copy = true;
        t.click(&mut m, &ground_hit([9.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        t.type_entry(&mut m, "/3").unwrap();
        let mut xs: Vec<f64> = m.instances().iter().map(|i| i.xf.origin()[0]).collect();
        xs.sort_by(f64::total_cmp);
        assert_eq!(xs, vec![0.0, 3.0, 6.0, 9.0]);
    }

    #[test]
    fn copying_loose_faces_with_ctrl() {
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        let mut t = Tools::default();
        t.selected = vec![f];
        t.set_tool(Tool::Move);
        t.click(&mut m, &ground_hit([0.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        t.copy = true;
        t.hover(&ground_hit([2.0, 0.0, 0.0], Snap::Endpoint));
        assert_eq!(t.preview_mesh(&m).unwrap().face_count(), 2, "the copy is shown beside the original");
        t.click(&mut m, &ground_hit([2.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        assert_eq!(m.mesh().face_count(), 2);
        assert_eq!(t.selected, vec![1]);
    }

    #[test]
    fn a_group_is_picked_whole_and_erased() {
        let (mut t, mut m) = grouped_box();
        t.model_changed();
        let on_group = Cursor { hit: Some(Hit { point: [1.0, 1.0, 2.0], plane: Plane::ground(), face: None, line: None, instance: Some(0), snap: Snap::OnFace }), ..ground_hit([1.0, 1.0, 2.0], Snap::OnFace) };
        t.click(&mut m, &on_group).unwrap();
        assert_eq!((t.selected.len(), t.instances.clone()), (0, vec![0]));
        t.set_tool(Tool::Eraser);
        t.click(&mut m, &on_group).unwrap();
        assert!(m.instances().is_empty());
    }

    #[test]
    fn text_on_a_face_starts_with_its_area() {
        let (mut m, top) = box_model();
        let (mut t, _) = with(Tool::Text);
        t.click(&mut m, &top).unwrap();
        t.click(&mut m, &ground_hit([4.0, 4.0, 0.0], Snap::Ground)).unwrap();
        let l = &m.notes().labels[0];
        assert_eq!(l.text, "4.00 m²");
        assert_eq!((l.point, l.at), ([1.0, 1.0, 2.0], [4.0, 4.0, 0.0]));
        assert_eq!(t.edit_label, Some(0), "the viewer opens it for typing");
        // at a corner it gives the position
        t.click(&mut m, &ground_hit([2.0, 0.0, 0.0], Snap::Endpoint)).unwrap();
        t.click(&mut m, &ground_hit([3.0, 0.0, 0.0], Snap::Ground)).unwrap();
        assert_eq!(m.notes().labels[1].text, "(2 m, 0 m, 0 m)");
    }

    #[test]
    fn three_d_text_is_placed_as_a_solid_group() {
        let mut m = Model::new();
        let mut t = Tools::default();
        t.begin_text3d("Hi", 0.3, Some(0.05)).unwrap();
        t.hover(&ground_hit([1.0, 1.0, 0.0], Snap::Ground));
        assert!(!t.preview_lines().is_empty(), "the outlines follow the mouse");
        t.click(&mut m, &ground_hit([1.0, 1.0, 0.0], Snap::Ground)).unwrap();
        assert_eq!(m.instances().len(), 1);
        assert_eq!(t.instances, vec![0]);
        let flat = m.flatten();
        let (lo, hi) = flat.bounds().unwrap();
        assert!((lo[2] - 0.0).abs() < 1e-5 && (hi[2] - 0.05).abs() < 1e-5, "{lo:?} {hi:?}");
        assert!(lo[0] >= 0.99 && lo[1] >= 0.99, "it starts at the point clicked");
        assert!(daedalus_core::HalfEdgeMesh::from_mesh(&flat).unwrap().is_closed());
        assert!(t.begin_text3d("   ", 0.3, None).is_err());
    }

    #[test]
    fn the_axes_tool_turns_the_drawing_axes() {
        let mut m = Model::new();
        let (mut t, _) = with(Tool::Axes);
        t.click(&mut m, &ground_hit([1.0, 1.0, 0.0], Snap::Endpoint)).unwrap();
        t.click(&mut m, &ground_hit([1.0, 3.0, 0.0], Snap::Endpoint)).unwrap();
        t.click(&mut m, &ground_hit([0.0, 1.5, 0.0], Snap::Ground)).unwrap();
        let a = m.notes().axes;
        assert_eq!(a.origin, [1.0, 1.0, 0.0]);
        assert_eq!(a.dirs[0], [0.0, 1.0, 0.0], "red along the old green");
        assert!((a.dirs[1][0] + 1.0).abs() < 1e-6, "green square to it, toward the click");
        assert!((a.dirs[2][2] - 1.0).abs() < 1e-6, "blue still up");
        assert_eq!(t.tool, Tool::Select);
        // a move now locks onto the new red axis
        t.axes = a;
        let lock = ray_axis_lock([1.0, 1.0, 0.0], &Cursor { origin: [1.0, 4.0, 10.0], dir: [0.0, 0.0, -1.0], hit: None }, &a.dirs);
        assert_eq!(lock.map(|l| l.1), Some(0));
        m.undo();
        assert!(m.notes().axes.is_default(), "placing the axes is undone like an edit");
    }
}
