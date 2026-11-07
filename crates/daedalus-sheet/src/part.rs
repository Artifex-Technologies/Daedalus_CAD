//! A sheet-metal part in a Daedalus model. The part is a component: its definition keeps the part's own description
//! ([`SheetPart`], as text in `Definition::part`) and, as its faces, the part folded (or flat), made from that description.
//! The part's coordinates are millimetres in its own frame (the base panel's bottom face on the XY plane, the sheet growing
//! up +Z); the definition's faces are the same in metres, and the component's placement puts the part in the model.

use crate::kernel::{self, MeshTolerance};
use crate::sheet::{self, RuleValues, SheetBody};
use crate::util::{DocError, Result};
use daedalus_core::scene::Xform;
use daedalus_core::Mesh;
use daedalus_io::dxf2d::{self, Entity, Shape, Units};
use serde::{Deserialize, Serialize};
use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::prelude::*;

/// Millimetres per metre: parts are in mm, the model in metres.
const MM: f64 = 1000.0;

/// How finely a part's curved faces are drawn: no facet more than this far off the true surface (mm).
const DRAW_CHORD: f64 = 0.02;

/// A shop material: its name and the sheet-metal values for it. These are starting points only; a part is made with the
/// fabricator's own K-factor and bend radius, which the user types in.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: &'static str,
    /// Thickness (mm).
    pub thickness: f64,
    pub k_factor: f64,
    /// Inside bend radius (mm).
    pub bend_radius: f64,
}

/// Common sheet materials. K-factor 0.44 and a bend radius about the thickness are typical air-bending values, not any
/// shop's own: check the fabricator's table before ordering.
pub const MATERIALS: &[Material] = &[
    Material { name: "Mild steel 16 ga (1.52 mm)", thickness: 1.52, k_factor: 0.44, bend_radius: 1.52 },
    Material { name: "Mild steel 14 ga (1.90 mm)", thickness: 1.90, k_factor: 0.44, bend_radius: 1.90 },
    Material { name: "Mild steel 11 ga (3.04 mm)", thickness: 3.04, k_factor: 0.44, bend_radius: 3.04 },
    Material { name: "Stainless 304 16 ga (1.52 mm)", thickness: 1.52, k_factor: 0.45, bend_radius: 1.52 },
    Material { name: "Aluminium 5052 0.080\" (2.03 mm)", thickness: 2.03, k_factor: 0.43, bend_radius: 2.03 },
    Material { name: "Aluminium 5052 0.125\" (3.18 mm)", thickness: 3.18, k_factor: 0.43, bend_radius: 4.76 },
];

impl Material {
    /// The rule values for this material: reliefs and gaps from the thickness (SolidCraft's defaults).
    pub fn rule(&self) -> RuleValues {
        RuleValues { k_factor: self.k_factor, bend_radius: self.bend_radius, ..RuleValues::steel(self.thickness) }
    }
}

/// Where an edge flange's bend sits against the edge it is added on (SolidCraft's names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlangePosition {
    /// The flange's outside face flush with the edge: the part keeps its outer size.
    Inside,
    /// The bend starts at the edge: the flange is added beyond it.
    Outside,
    /// The flange's middle on the edge.
    Middle,
}

impl FlangePosition {
    pub const ALL: [FlangePosition; 3] = [FlangePosition::Inside, FlangePosition::Outside, FlangePosition::Middle];

    fn key(self) -> &'static str {
        match self {
            FlangePosition::Inside => "inside",
            FlangePosition::Outside => "outside",
            FlangePosition::Middle => "middle",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FlangePosition::Inside => "Inside (keeps the outer size)",
            FlangePosition::Outside => "Outside (added beyond the edge)",
            FlangePosition::Middle => "Middle",
        }
    }
}

/// A sheet-metal part: its material and the sheet it is made of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetPart {
    /// Saved with the part so a newer description can be told apart.
    pub kind: String,
    pub material: String,
    pub rule: RuleValues,
    pub sheet: SheetBody,
    /// Edge flanges as they were asked for, in order (to list and undo them; the sheet holds what they made).
    #[serde(default)]
    pub flanges: Vec<FlangeAsked>,
}

/// An edge flange as the user asked for it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlangeAsked {
    /// The point picked on the edge (part mm).
    pub pick: [f64; 3],
    pub height: f64,
    pub angle_deg: f64,
    pub position: String,
}

/// The kind a part's text names, so `Definition::part` can hold other kinds later.
pub const KIND: &str = "sheet-metal 1";

/// Whether a definition's part text is a sheet-metal part.
pub fn is_sheet(text: &str) -> bool {
    text.contains(KIND)
}

fn invalid<T>(m: impl Into<String>) -> Result<T> {
    Err(DocError::Invalid(m.into()))
}

impl SheetPart {
    /// A flat part from an outline and its holes (part mm, the outline counter-clockwise), `rule`'s thickness up +Z.
    pub fn from_region(material: &str, rule: RuleValues, region: Region2) -> Result<SheetPart> {
        let rule = rule.checked()?;
        let sheet = sheet::base_flange("Part", material, &rule, Plane::XY, region);
        let part = SheetPart { kind: KIND.into(), material: material.into(), rule, sheet, flanges: Vec::new() };
        part.sheet.solid()?;
        Ok(part)
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string(self).expect("a sheet part always writes")
    }

    pub fn from_text(text: &str) -> Result<SheetPart> {
        let p: SheetPart = serde_json::from_str(text).map_err(|e| DocError::Invalid(format!("the sheet-metal part does not read: {e}")))?;
        if p.kind != KIND {
            return invalid(format!("a part of kind {:?}, made by a newer Daedalus", p.kind));
        }
        Ok(p)
    }

    /// Thickness (mm).
    pub fn thickness(&self) -> f64 {
        self.sheet.t
    }

    /// Adds an edge flange on the part's edge nearest `pick` (part mm): `height` from the outer virtual sharp (mm),
    /// `angle_deg` bend angle (0…180), radius from the part's rule. The part is unchanged if it fails.
    pub fn add_flange(&mut self, pick: [f64; 3], height: f64, angle_deg: f64, position: FlangePosition) -> Result<()> {
        if !(height > 0.0 && height.is_finite()) {
            return invalid("the flange height must be more than 0");
        }
        if !(angle_deg > 0.0 && angle_deg <= 180.0) {
            return invalid("the bend angle must be more than 0 and at most 180 degrees");
        }
        let mut sheet = self.sheet.clone();
        sheet.add_flange(Vec3::new(pick[0], pick[1], pick[2]), height, angle_deg.to_radians(), self.rule.bend_radius, position.key(), false)?;
        sheet.solid()?;
        self.sheet = sheet;
        self.flanges.push(FlangeAsked { pick, height, angle_deg, position: position.key().into() });
        Ok(())
    }

    /// The part as Daedalus faces, in metres in the part's frame: `flat` unfolded, else folded. Flat faces of the exact solid
    /// stay whole (with their holes); curved ones (the bends, round holes' walls) are fine facets with soft edges between.
    pub fn faces(&self, flat: bool) -> Result<Mesh> {
        let mut sheet = self.sheet.clone();
        sheet.flat = flat;
        let body = sheet.solid()?;
        body_faces(&body)
    }

    /// The bends as the flat pattern's bend table: (angle degrees, inside radius mm, length mm).
    pub fn bend_table(&self) -> Vec<(f64, f64, f64)> {
        self.sheet.bends().iter().map(|b| (b.angle_deg, b.radius, b.length)).collect()
    }

    /// The flat blank's size (mm).
    pub fn flat_size(&self) -> Result<(f64, f64)> {
        self.sheet.flat_size()
    }

    /// The flat pattern's outline, holes and bend centre lines (part mm, flat frame), for drawing.
    pub fn flat_drawing(&self) -> Result<(Vec<Vec<[f64; 2]>>, Vec<([f64; 2], [f64; 2])>)> {
        let mut loops = Vec::new();
        for r in self.sheet.flat_regions()? {
            for lp in std::iter::once(&r.outer).chain(&r.holes) {
                loops.push(sample_loop(lp));
            }
        }
        let bends = self.sheet.bends().iter().map(|b| ([b.start.x, b.start.y], [b.end.x, b.end.y])).collect();
        Ok((loops, bends))
    }

    /// The flat pattern as DXF for a cutting service: outline and holes on `CUT` (lines and true arcs), a line at the centre
    /// of each bend on `BEND`. 1:1 in `units`.
    pub fn flat_dxf(&self, units: Units) -> Result<String> {
        let k = match units {
            Units::Millimeters => 1.0,
            Units::Inches => 1.0 / 25.4,
        };
        let p = |v: Vec2| [v.x * k, v.y * k];
        let mut out = Vec::new();
        for r in self.sheet.flat_regions()? {
            for lp in std::iter::once(&r.outer).chain(&r.holes) {
                for s in &joined_arcs(&lp.segs) {
                    match *s {
                        Seg2::Line { a, b } => out.push(Entity { layer: dxf2d::CUT_LAYER, shape: Shape::Line { a: p(a), b: p(b) } }),
                        Seg2::Arc { center, radius, start, sweep } => {
                            // DXF arcs run counter-clockwise: a clockwise one is written from its end
                            let (a0, a1) = if sweep >= 0.0 { (start, start + sweep) } else { (start + sweep, start) };
                            out.push(Entity {
                                layer: dxf2d::CUT_LAYER,
                                shape: Shape::Arc { center: p(center), radius: radius * k, start: a0.to_degrees(), end: a1.to_degrees() },
                            });
                        }
                        _ => {
                            let pts = sample_seg(s);
                            for w in pts.windows(2) {
                                out.push(Entity { layer: dxf2d::CUT_LAYER, shape: Shape::Line { a: [w[0][0] * k, w[0][1] * k], b: [w[1][0] * k, w[1][1] * k] } });
                            }
                        }
                    }
                }
            }
        }
        for b in self.sheet.bends() {
            out.push(Entity { layer: dxf2d::BEND_LAYER, shape: Shape::Line { a: p(b.start), b: p(b.end) } });
        }
        Ok(dxf2d::write_entities(&out, units))
    }

    /// The folded part as an exact solid, placed by `place` (model metres) and given in mm, as STEP files want.
    pub fn solid_placed(&self, place: &Xform) -> Result<kernel::Body> {
        let body = self.sheet.solid()?;
        let m = place.0;
        // part mm -> model mm: the placement's rotation as it is, its translation in mm
        let mat: crate::util::Mat = [
            [m[0][0], m[1][0], m[2][0], 0.0],
            [m[0][1], m[1][1], m[2][1], 0.0],
            [m[0][2], m[1][2], m[2][2], 0.0],
            [m[0][3] * MM, m[1][3] * MM, m[2][3] * MM, 1.0],
        ];
        if crate::util::is_identity(&mat) {
            return Ok(body);
        }
        Ok(kernel::transform_matrix(&body, mat)?)
    }
}

/// STEP (AP242) of exact solids, for a fabricator or another CAD program.
pub fn step(bodies: &[kernel::Body]) -> Result<String> {
    let refs: Vec<&kernel::Body> = bodies.iter().collect();
    Ok(kernel::step_export(&refs, "Daedalus CAD")?)
}

/// Arcs that follow on from each other round one circle, joined into one (a circle split in halves is one circle again).
fn joined_arcs(segs: &[Seg2]) -> Vec<Seg2> {
    let mut out: Vec<Seg2> = Vec::new();
    for s in segs {
        if let (Some(Seg2::Arc { center: c0, radius: r0, start: s0, sweep: w0 }), Seg2::Arc { center, radius, start, sweep }) = (out.last().copied(), *s) {
            let tol = 1e-7 * r0.max(1.0);
            let follows = ((s0 + w0 - start + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI).abs() < 1e-9;
            if c0.dist(center) < tol && (r0 - radius).abs() < tol && w0.signum() == sweep.signum() && follows && (w0 + sweep).abs() <= std::f64::consts::TAU + 1e-9 {
                *out.last_mut().unwrap() = Seg2::Arc { center: c0, radius: r0, start: s0, sweep: w0 + sweep };
                continue;
            }
        }
        out.push(*s);
    }
    out
}

/// A loop's points, curves sampled (for drawing).
fn sample_loop(lp: &Loop2) -> Vec<[f64; 2]> {
    let mut pts: Vec<[f64; 2]> = Vec::new();
    for s in &lp.segs {
        let mut sp = sample_seg(s);
        if !pts.is_empty() {
            sp.remove(0);
        }
        pts.extend(sp);
    }
    pts
}

fn sample_seg(s: &Seg2) -> Vec<[f64; 2]> {
    let n = match s {
        Seg2::Line { .. } => 1,
        _ => ((s.length() / 0.5).ceil() as usize).clamp(8, 256),
    };
    (0..=n).map(|i| s.point_at(i as f64 / n as f64)).map(|v| [v.x, v.y]).collect()
}

/// An exact solid's faces as a Daedalus mesh (metres): its triangles welded, the edges inside each exact face made soft,
/// then the pieces of each flat face joined back into one face (with holes).
fn body_faces(body: &kernel::Body) -> Result<Mesh> {
    let tm = body.mesh(&MeshTolerance { chord: DRAW_CHORD, angle: Some(10f64.to_radians()) })?;
    let mut mesh = Mesh::new();
    let mut weld: std::collections::HashMap<[i64; 3], u32> = std::collections::HashMap::new();
    let q = |v: f64| (v * 1e6).round() as i64; // nanometre grid (mm * 1e6)
    let ids: Vec<u32> = tm
        .positions
        .iter()
        .map(|p| *weld.entry([q(p.x), q(p.y), q(p.z)]).or_insert_with(|| mesh.add_vertex([p.x / MM, p.y / MM, p.z / MM])))
        .collect();
    let mut owner: std::collections::HashMap<(u32, u32), Vec<u32>> = std::collections::HashMap::new();
    for (t, tri) in tm.triangles.iter().enumerate() {
        let [a, b, c] = tri.map(|i| ids[i as usize]);
        if a == b || b == c || a == c {
            continue;
        }
        mesh.add_face(vec![a, b, c]);
        let bface = tm.tri_face.get(t).copied().unwrap_or(u32::MAX);
        for (x, y) in [(a, b), (b, c), (c, a)] {
            owner.entry((x.min(y), x.max(y))).or_default().push(bface);
        }
    }
    for ((x, y), faces) in owner {
        if faces.len() == 2 && faces[0] == faces[1] {
            mesh.seams.push([x, y]);
        }
    }
    mesh.join_seamed_faces();
    Ok(mesh)
}

/// A frame for a face of the model: origin at its first corner, X along its first edge, Z its normal (all model metres).
fn face_frame(mesh: &Mesh, f: usize) -> Option<([f64; 3], [f64; 3], [f64; 3], [f64; 3])> {
    let (n, _) = daedalus_core::imprint::face_plane(mesh, &mesh.faces[f])?;
    let face = &mesh.faces[f];
    let o = mesh.positions[face[0] as usize];
    let p1 = mesh.positions[face[1] as usize];
    let d = [p1[0] - o[0], p1[1] - o[1], p1[2] - o[2]];
    let dn = d[0] * n[0] + d[1] * n[1] + d[2] * n[2];
    let x = [d[0] - dn * n[0], d[1] - dn * n[1], d[2] - dn * n[2]];
    let l = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
    if !(l > 0.0) {
        return None;
    }
    let x = [x[0] / l, x[1] / l, x[2] / l];
    let y = [n[1] * x[2] - n[2] * x[1], n[2] * x[0] - n[0] * x[2], n[0] * x[1] - n[1] * x[0]];
    Some((o, x, y, n))
}

/// A sheet-metal part made from face `f` of `mesh`: the face's outline and holes (circles and arcs drawn in Daedalus become
/// true arcs), the sheet `rule`'s thickness growing out of the face's front. Returns the part and its placement in the model.
pub fn part_from_face(mesh: &Mesh, f: usize, material: &str, rule: RuleValues) -> Result<(SheetPart, Xform)> {
    let Some((o, x, y, n)) = face_frame(mesh, f) else { return invalid("that face has no plane") };
    let to2 = |p: [f64; 3]| {
        let d = [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
        Vec2::new((d[0] * x[0] + d[1] * x[1] + d[2] * x[2]) * MM, (d[0] * y[0] + d[1] * y[1] + d[2] * y[2]) * MM)
    };
    let loop2 = |lp: &[u32]| -> Loop2 { Loop2 { segs: loop_segments(mesh, lp, &to2) } };
    let outer = loop2(&mesh.faces[f]);
    let holes: Vec<Loop2> = mesh.holes_of(f).iter().map(|h| loop2(h)).collect();
    let part = SheetPart::from_region(material, rule, Region2 { outer, holes })?;
    let place = Xform([[x[0], y[0], n[0], o[0]], [x[1], y[1], n[1], o[1]], [x[2], y[2], n[2], o[2]]]);
    Ok((part, place))
}

/// A face loop's segments in the face's plane (mm): runs of edges on one recognised circle or arc become one exact arc.
fn loop_segments(mesh: &Mesh, lp: &[u32], to2: &dyn Fn([f64; 3]) -> Vec2) -> Vec<Seg2> {
    let n = lp.len();
    let pt = |i: usize| to2(mesh.positions[lp[i % n] as usize]);
    // which curve each edge (i, i+1) lies on, by its centre and radius
    let on: Vec<Option<(Vec2, f64)>> = (0..n)
        .map(|i| daedalus_core::curve::curve_through(mesh, lp[i], lp[(i + 1) % n]).map(|c| (to2(c.center), c.radius * MM)))
        .collect();
    let same = |a: Option<(Vec2, f64)>, b: Option<(Vec2, f64)>| match (a, b) {
        (Some((c1, r1)), Some((c2, r2))) => c1.dist(c2) < 1e-6 * r1.max(1.0) && (r1 - r2).abs() < 1e-6 * r1.max(1.0),
        _ => false,
    };
    // a whole loop on one circle
    if n >= 3 && on.iter().all(|c| same(*c, on[0])) {
        let (c, r) = on[0].unwrap();
        let start = (pt(0) - c).angle();
        let turn = signed_area(&(0..n).map(pt).collect::<Vec<_>>()).signum();
        return vec![Seg2::Arc { center: c, radius: r, start, sweep: std::f64::consts::TAU * turn }];
    }
    // start where the curve changes, so a run never wraps round the loop's start
    let first = (0..n).find(|&i| !same(on[i], on[(i + n - 1) % n])).unwrap_or(0);
    let mut segs = Vec::new();
    let mut i = 0;
    while i < n {
        let k = (first + i) % n;
        let Some((c, r)) = on[k] else {
            segs.push(Seg2::Line { a: pt(k), b: pt(k + 1) });
            i += 1;
            continue;
        };
        let mut j = i + 1;
        while j < n && same(on[(first + j) % n], on[k]) {
            j += 1;
        }
        // the sweep: the turns between successive points along the run, each the short way round
        let a = pt(k);
        let sweep = wrap_sum(&(i..=j).map(|m| (pt(first + m) - c).angle()).collect::<Vec<_>>());
        segs.push(Seg2::Arc { center: c, radius: r, start: (a - c).angle(), sweep });
        i = j;
    }
    segs
}

/// The total turn through a list of angles, each step taken the short way round.
fn wrap_sum(angles: &[f64]) -> f64 {
    use std::f64::consts::{PI, TAU};
    angles.windows(2).map(|w| (w[1] - w[0] + PI).rem_euclid(TAU) - PI).sum()
}

fn signed_area(p: &[Vec2]) -> f64 {
    (0..p.len()).map(|i| p[i].x * p[(i + 1) % p.len()].y - p[(i + 1) % p.len()].x * p[i].y).sum::<f64>() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100 x 50 mm plate on the ground (model metres), facing up, with a 20 mm circle of 24 segments drawn in it as a hole.
    fn plate_face() -> Mesh {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.1, 0.05, 0.0], [0.0, 0.05, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        let hole: Vec<u32> = (0..24)
            .rev()
            .map(|i| {
                let a = i as f64 / 24.0 * std::f64::consts::TAU;
                m.add_vertex([0.03 + 0.01 * a.cos(), 0.025 + 0.01 * a.sin(), 0.0])
            })
            .collect();
        m.holes = vec![vec![hole]];
        m
    }

    fn steel2() -> RuleValues {
        RuleValues { k_factor: 0.44, bend_radius: 2.0, ..RuleValues::steel(2.0) }
    }

    fn size(m: &Mesh) -> [f64; 3] {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in &m.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
    }

    #[test]
    fn a_face_with_a_drawn_circle_becomes_a_part_with_a_true_round_hole() {
        let (part, place) = part_from_face(&plate_face(), 0, "Steel 2 mm", steel2()).unwrap();
        assert_eq!(place.apply([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        assert_eq!(part.sheet.holes.len(), 1);
        let segs = &part.sheet.holes[0].outer.segs;
        assert_eq!(segs.len(), 1, "the 24 segments are one circle: {segs:?}");
        let Seg2::Arc { center, radius, sweep, .. } = segs[0] else { panic!("{segs:?}") };
        assert!((radius - 10.0).abs() < 1e-6 && center.dist(Vec2::new(30.0, 25.0)) < 1e-6 && (sweep.abs() - std::f64::consts::TAU).abs() < 1e-9);
        // its faces: 100 x 50 x 2 mm, in metres; the top and bottom whole, each with the hole
        let faces = part.faces(false).unwrap();
        let s = size(&faces);
        assert!((s[0] - 0.1).abs() < 1e-9 && (s[1] - 0.05).abs() < 1e-9 && (s[2] - 0.002).abs() < 1e-9, "{s:?}");
        let holed = (0..faces.faces.len()).filter(|&f| faces.holes_of(f).len() == 1).count();
        assert_eq!(holed, 2, "top and bottom are one face each with the hole in it");
        // closed: every edge between exactly two faces
        let mut uses: std::collections::HashMap<(u32, u32), i32> = std::collections::HashMap::new();
        for (_, a, b) in faces.face_edges() {
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
        assert!(uses.values().all(|&n| n == 2), "watertight");
        // the text round trip
        assert_eq!(SheetPart::from_text(&part.to_text()).unwrap(), part);
        assert!(is_sheet(&part.to_text()));
    }

    #[test]
    fn a_flange_bends_up_and_the_flat_pattern_goes_out_with_arcs_and_a_bend_line() {
        let (mut part, _) = part_from_face(&plate_face(), 0, "Steel 2 mm", steel2()).unwrap();
        part.add_flange([50.0, 0.0, 2.0], 30.0, 90.0, FlangePosition::Inside).unwrap();
        assert_eq!(part.flanges.len(), 1);
        let s = size(&part.faces(false).unwrap());
        assert!((s[0] - 0.1).abs() < 1e-9 && (s[1] - 0.05).abs() < 1e-9 && (s[2] - 0.03).abs() < 1e-6, "folded {s:?}");
        // flat: 100 wide, (50 - 4) + (30 - 4) + BA long (outside setback T + R = 4 mm)
        let ba = std::f64::consts::FRAC_PI_2 * (2.0 + 0.44 * 2.0);
        let (w, h) = part.flat_size().unwrap();
        assert!((w - 100.0).abs() < 1e-9 && (h - (46.0 + 26.0 + ba)).abs() < 1e-6, "{w} x {h}");
        let flat = size(&part.faces(true).unwrap());
        assert!((flat[2] - 0.002).abs() < 1e-9 && (flat[1] * 1000.0 - h).abs() < 1e-6, "flat {flat:?}");
        assert_eq!(part.bend_table().len(), 1);
        let dxf = part.flat_dxf(Units::Millimeters).unwrap();
        let count = |n: &str| dxf.lines().filter(|l| *l == n).count();
        assert_eq!(count("CIRCLE"), 1, "the hole is one true circle");
        assert_eq!(count("LINE"), 4 + 1, "four sides and the bend line");
        assert_eq!(count("BEND"), 1 + 1, "the bend line and its layer");
        // a bad flange leaves the part as it was
        let before = part.clone();
        assert!(part.add_flange([50.0, 0.0, 2.0], -5.0, 90.0, FlangePosition::Inside).is_err());
        assert!(part.add_flange([50.0, 0.0, 2.0], 10.0, 200.0, FlangePosition::Inside).is_err());
        assert_eq!(part, before);
    }

    #[test]
    fn a_placed_part_goes_out_as_one_exact_step_solid_in_mm() {
        let (mut part, _) = part_from_face(&plate_face(), 0, "Steel 2 mm", steel2()).unwrap();
        part.add_flange([50.0, 0.0, 2.0], 30.0, 90.0, FlangePosition::Inside).unwrap();
        // placed 1 m along X and turned a quarter about Z
        let place = Xform([[0.0, -1.0, 0.0, 1.0], [1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]]);
        let body = part.solid_placed(&place).unwrap();
        let bb = kernel::measure(&body).unwrap().bbox;
        assert!((bb.min.x - 950.0).abs() < 1e-6 && (bb.max.x - 1000.0).abs() < 1e-6 && (bb.max.y - 100.0).abs() < 1e-6, "{bb:?}");
        let text = step(&[body]).unwrap();
        assert_eq!(text.matches("MANIFOLD_SOLID_BREP").count(), 1);
        assert!(text.contains("CYLINDRICAL_SURFACE"));
    }

    #[test]
    fn a_rounded_corner_drawn_in_segments_becomes_one_exact_arc() {
        // 60 x 40 mm, the corner at (60, 40) rounded with radius 10 in 6 segments, counter-clockwise seen from above
        let mut m = Mesh::new();
        let mut ring = vec![m.add_vertex([0.0, 0.0, 0.0]), m.add_vertex([0.06, 0.0, 0.0])];
        for i in 0..=6 {
            let a = i as f64 / 6.0 * std::f64::consts::FRAC_PI_2;
            ring.push(m.add_vertex([0.05 + 0.01 * a.cos(), 0.03 + 0.01 * a.sin(), 0.0]));
        }
        ring.push(m.add_vertex([0.0, 0.04, 0.0]));
        m.add_face(ring);
        let (part, _) = part_from_face(&m, 0, "Steel 2 mm", steel2()).unwrap();
        let segs = &part.sheet.base.outer.segs;
        let arcs: Vec<_> = segs.iter().filter_map(|s| if let Seg2::Arc { radius, sweep, .. } = s { Some((*radius, *sweep)) } else { None }).collect();
        assert_eq!(arcs.len(), 1, "{segs:?}");
        assert!((arcs[0].0 - 10.0).abs() < 1e-6 && (arcs[0].1 - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "{arcs:?}");
        assert_eq!(segs.len(), 5, "four lines and the arc");
        // the segments join up end to end
        for w in 0..segs.len() {
            assert!(segs[w].end().dist(segs[(w + 1) % segs.len()].start()) < 1e-6, "{segs:?}");
        }
    }
}
