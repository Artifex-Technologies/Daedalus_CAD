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

    pub fn from_key(k: &str) -> Option<FlangePosition> {
        FlangePosition::ALL.into_iter().find(|p| p.key() == k)
    }

    pub fn label(self) -> &'static str {
        match self {
            FlangePosition::Inside => "Inside (keeps the outer size)",
            FlangePosition::Outside => "Outside (added beyond the edge)",
            FlangePosition::Middle => "Middle",
        }
    }
}

/// A sheet-metal part: its material, the outline it was made from, and its flanges in the order they were added. The sheet
/// itself is built from those (again whenever one changes), so a flange can be changed or taken away later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetPart {
    /// Saved with the part so a newer description can be told apart.
    pub kind: String,
    pub material: String,
    pub rule: RuleValues,
    /// The base panel's outline and holes (part mm).
    pub region: Region2,
    pub flanges: Vec<FlangeAsked>,
    /// The model shows it unfolded.
    #[serde(default)]
    pub shown_flat: bool,
    /// Made by converting a solid (then `region` is unused): the solid, the face that stays put and the bend edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid: Option<SolidSource>,
    /// What the conversion could not finish as the rule says (corners, bend ends), to show the person.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// What the above make (kept so a part reads back without rebuilding; rebuilt on every change).
    pub sheet: SheetBody,
}

/// A solid a part was converted from (part mm): its flat faces (each loops of points, the outline first), a point on the
/// face that stays put, and a point on each edge that becomes a bend.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolidSource {
    pub faces: Vec<Vec<Vec<[f64; 3]>>>,
    pub fixed: [f64; 3],
    pub bends: Vec<[f64; 3]>,
}

/// An edge flange, a hem, a fold or a jog as the user asked for it, in the order they were added (the part is built again
/// from them in this order).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlangeAsked {
    /// The point picked on the edge (part mm).
    pub pick: [f64; 3],
    /// A flange's height, or a hem's length to its outer face (mm).
    pub height: f64,
    pub angle_deg: f64,
    pub position: String,
    /// Bent down, away from the face the part was made from (the other way round from the default).
    #[serde(default)]
    pub down: bool,
    /// A flat hem (folded right back onto the sheet; the angle and position do not apply).
    #[serde(default)]
    pub hem: bool,
    /// A fold along a line across a panel, or a jog (then `height` is its step and `position` is not used); `pick` is unused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fold: Option<FoldLine>,
}

/// Where a fold or jog runs: two points on its line, on a face of the panel it folds (part mm).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FoldLine {
    pub a: [f64; 3],
    pub b: [f64; 3],
    /// Two folds the opposite way round, so the far side stays parallel and steps aside by `height` (a joggle).
    #[serde(default)]
    pub jog: bool,
}

/// Where a fold's bend sits against the line drawn (SolidCraft's names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldPosition {
    /// The middle of the bend on the line: the bend line a shop's DXF expects.
    Centre,
    /// Where the outer faces would meet (the mould line): the side that stays measures the same to it folded as flat.
    Mould,
}

impl FoldPosition {
    pub const ALL: [FoldPosition; 2] = [FoldPosition::Centre, FoldPosition::Mould];

    pub fn key(self) -> &'static str {
        match self {
            FoldPosition::Centre => "centerline",
            FoldPosition::Mould => "mould",
        }
    }

    pub fn from_key(k: &str) -> Option<FoldPosition> {
        FoldPosition::ALL.into_iter().find(|p| p.key() == k)
    }

    pub fn label(self) -> &'static str {
        match self {
            FoldPosition::Centre => "Centre line on the line drawn",
            FoldPosition::Mould => "Mould line on the line drawn",
        }
    }
}

/// What a step is: a flange, a hem, a fold or a jog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Flange,
    Hem,
    Fold,
    Jog,
}

impl FlangeAsked {
    pub fn kind(&self) -> StepKind {
        match &self.fold {
            Some(f) if f.jog => StepKind::Jog,
            Some(_) => StepKind::Fold,
            None if self.hem => StepKind::Hem,
            None => StepKind::Flange,
        }
    }
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

/// The sheet a solid converts to under `rule` (the solid's own thickness when it is a shell), and the notes on what was
/// not finished as the rule says.
fn converted(src: &SolidSource, material: &str, rule: &RuleValues) -> Result<(SheetBody, Vec<String>)> {
    let body = crate::export::sew_polygons(&src.faces)?;
    let mb = sheet::ModelBody::new("Part", body)?;
    let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
    let rule = *rule;
    let values = move |t: Option<f64>| RuleValues { thickness: t.unwrap_or(rule.thickness), ..rule }.checked();
    let bends: Vec<Vec3> = src.bends.iter().map(|&p| v(p)).collect();
    sheet::convert_body(&mb, material, &values, v(src.fixed), &bends, &crate::util::IDENTITY)
}

/// A sheet-metal part converted from a closed solid (`mesh`, in metres, the part's coordinates): `fixed` a point on the
/// face that stays put, `bends` a point on each edge that becomes a bend (metres). A shelled solid keeps its walls'
/// thickness; any other takes the rule's thickness inside its outer faces. Returns the part and its notes.
pub fn part_from_solid(mesh: &Mesh, fixed: [f64; 3], bends: &[[f64; 3]], material: &str, rule: RuleValues) -> Result<SheetPart> {
    let mm = |p: [f64; 3]| p.map(|x| x * MM);
    let src = SolidSource { faces: crate::export::polygons_mm(mesh, &Xform::IDENTITY), fixed: mm(fixed), bends: bends.iter().map(|&p| mm(p)).collect() };
    let rule = rule.checked()?;
    let (sheet, notes) = converted(&src, material, &rule)?;
    let mut part = SheetPart { kind: KIND.into(), material: material.into(), rule, region: Region2::default(), flanges: Vec::new(), shown_flat: false, solid: Some(src), notes, sheet };
    part.rule.thickness = part.sheet.t;
    part.sheet.solid()?;
    Ok(part)
}

/// A flange or hem as asked, added to `sheet`.
fn apply(sheet: &mut SheetBody, f: &FlangeAsked, rule: &RuleValues) -> Result<()> {
    let pick = Vec3::new(f.pick[0], f.pick[1], f.pick[2]);
    if let Some(line) = &f.fold {
        let (a, b) = (Vec3::new(line.a[0], line.a[1], line.a[2]), Vec3::new(line.b[0], line.b[1], line.b[2]));
        if line.jog {
            return jog(sheet, a, b, f.height, f.angle_deg.to_radians(), rule.bend_radius, f.down);
        }
        let mut warning = None;
        sheet.add_fold(a, b, f.angle_deg.to_radians(), rule.bend_radius, &f.position, f.down, None, &mut warning)?;
        return Ok(());
    }
    if f.hem {
        sheet.add_hem(pick, f.height, rule.hem_gap, f.down)?;
    } else {
        let position = FlangePosition::from_key(&f.position).unwrap_or(FlangePosition::Inside);
        sheet.add_flange(pick, f.height, f.angle_deg.to_radians(), rule.bend_radius, position.key(), f.down)?;
    }
    Ok(())
}

/// The second fold of a jog: `s` (flat mm) from where the first one's bend starts, the other way round. Returns the sheet, the
/// far side's step from the base (top face to top face, along the base's normal) and how parallel it is to the base (1: it is).
fn jog_second(first: &SheetBody, i: usize, s: f64, angle: f64, radius: f64, flip: bool) -> Result<(SheetBody, f64, f64)> {
    use crate::util::apply_point;
    let f = first.flanges[i].clone();
    let t = first.t;
    let flat3 = |q: Vec2, z: f64| Vec3::new(q.x, q.y, z);
    let c = f.p + f.n * s;
    let big = 1.0e4;
    let m = first.world(Some(i));
    let (wa, wb) = (apply_point(&m, flat3(c - f.d * big, t)), apply_point(&m, flat3(c + f.d * big, t)));
    let mut next = first.clone();
    let mut warning = None;
    let j = next.add_fold(wa, wb, angle, radius, "centerline", flip, None, &mut warning)?;
    let g = next.flanges[j].clone();
    let q = g.p + g.n * (next.allowance(&g) + 1.0);
    let (far, base) = (next.world(Some(j)), next.world(None));
    let normal = |m: &crate::util::Mat| {
        let v = apply_point(m, Vec3::Z) - apply_point(m, Vec3::ZERO);
        v.normalized().unwrap_or(Vec3::Z)
    };
    let base_n = normal(&base);
    let step = (apply_point(&far, flat3(q, t)) - apply_point(&base, flat3(q, t))).dot(base_n);
    Ok((next, step, normal(&far).dot(base_n)))
}

/// A jog on the panel the line `a`-`b` lies on: a fold of `angle` there (up, or `down`), then one the other way round further
/// on, placed so the far side, parallel to the panel again, has stepped `step` mm aside (face to face). The flat between the
/// two bends follows from the step; a step too small for the bends (they would overlap) is refused, saying the least.
fn jog(sheet: &mut SheetBody, a: Vec3, b: Vec3, step: f64, angle: f64, radius: f64, down: bool) -> Result<()> {
    if !(step > 0.0 && step.is_finite()) {
        return invalid("the jog's step must be more than 0");
    }
    let mut first = sheet.clone();
    let mut warning = None;
    let i = first.add_fold(a, b, angle, radius, "centerline", down, None, &mut warning)?;
    let ba = first.allowance(&first.flanges[i]);
    // the second bend starts where the first ends at the earliest (no flat between them)
    let s0 = ba * 1.5 + 1e-3;
    // which way round makes the far side parallel (a Z, not a U)
    let flip = [!down, down]
        .into_iter()
        .find(|&fl| jog_second(&first, i, s0, angle, radius, fl).is_ok_and(|(_, _, par)| par > 0.999))
        .ok_or_else(|| DocError::Invalid("the jog's second bend could not be made".into()))?;
    let (_, o1, _) = jog_second(&first, i, s0, angle, radius, flip)?;
    let (_, o2, _) = jog_second(&first, i, s0 + 5.0, angle, radius, flip)?;
    let slope = (o2.abs() - o1.abs()) / 5.0;
    if !(slope > 1e-9) {
        return invalid("the jog's step does not grow with the flat between its bends (is the angle 0?)");
    }
    if step < o1.abs() - 1e-9 {
        return invalid(format!("a jog with these bends steps at least {} mm (a smaller radius or angle allows less)", fmt_mm(o1.abs())));
    }
    let s = s0 + (step - o1.abs()) / slope;
    let (next, o, _) = jog_second(&first, i, s, angle, radius, flip)?;
    if (o.abs() - step).abs() > 1e-6 {
        return invalid(format!("the jog came out {} mm instead of {} mm", fmt_mm(o.abs()), fmt_mm(step)));
    }
    *sheet = next;
    Ok(())
}

fn check_flange(height: f64, angle_deg: f64) -> Result<()> {
    if !(height > 0.0 && height.is_finite()) {
        return invalid("the flange height must be more than 0");
    }
    if !(angle_deg > 0.0 && angle_deg <= 180.0) {
        return invalid("the bend angle must be more than 0 and at most 180 degrees");
    }
    Ok(())
}

impl SheetPart {
    /// A flat part from an outline and its holes (part mm, the outline counter-clockwise), `rule`'s thickness up +Z.
    pub fn from_region(material: &str, rule: RuleValues, region: Region2) -> Result<SheetPart> {
        let rule = rule.checked()?;
        let sheet = sheet::base_flange("Part", material, &rule, Plane::XY, region.clone());
        let part = SheetPart { kind: KIND.into(), material: material.into(), rule, region, flanges: Vec::new(), shown_flat: false, solid: None, notes: Vec::new(), sheet };
        part.sheet.solid()?;
        Ok(part)
    }

    /// The sheet built again from the outline, the rule and the flanges. Fails (and changes nothing) if a flange no longer
    /// finds its edge or the part cannot be made.
    pub fn rebuild(&mut self) -> Result<()> {
        let rule = self.rule.checked()?;
        let (mut sheet, notes) = match &self.solid {
            Some(src) => converted(src, &self.material, &rule)?,
            None => (sheet::base_flange("Part", &self.material, &rule, Plane::XY, self.region.clone()), Vec::new()),
        };
        self.notes = notes;
        for (n, f) in self.flanges.iter().enumerate() {
            apply(&mut sheet, f, &rule).map_err(|e| DocError::Invalid(format!("flange {}: {e}", n + 1)))?;
        }
        sheet.solid()?;
        self.sheet = sheet;
        Ok(())
    }

    /// A new material or rule (thickness, K-factor, bend radius), the flanges kept. The part is unchanged if it fails.
    pub fn set_rule(&mut self, material: &str, rule: RuleValues) -> Result<()> {
        let mut next = self.clone();
        next.material = material.into();
        next.rule = rule.checked()?;
        next.rebuild()?;
        *self = next;
        Ok(())
    }

    /// Flange `i` changed (its height, angle, position or direction). The part is unchanged if it fails.
    pub fn set_flange(&mut self, i: usize, height: f64, angle_deg: f64, position: FlangePosition, down: bool) -> Result<()> {
        check_flange(height, angle_deg)?;
        let mut next = self.clone();
        let Some(f) = next.flanges.get_mut(i) else { return invalid("no such flange") };
        (f.height, f.angle_deg, f.position, f.down) = (height, angle_deg, position.key().into(), down);
        next.rebuild()?;
        *self = next;
        Ok(())
    }

    /// Fold or jog `i` changed: its angle (degrees), a jog's step (mm; ignored for a fold), a fold's position, its direction.
    /// The part is unchanged if it fails.
    pub fn set_fold(&mut self, i: usize, step: f64, angle_deg: f64, position: FoldPosition, down: bool) -> Result<()> {
        let mut next = self.clone();
        let Some(f) = next.flanges.get_mut(i) else { return invalid("no such fold") };
        let jog = match &f.fold {
            Some(line) => line.jog,
            None => return invalid("that is not a fold or a jog"),
        };
        if jog {
            check_flange(step, angle_deg)?;
            f.height = step;
        } else {
            check_flange(1.0, angle_deg)?;
            f.position = position.key().into();
        }
        (f.angle_deg, f.down) = (angle_deg, down);
        next.rebuild()?;
        *self = next;
        Ok(())
    }

    /// Flange `i` taken away (and any added after it that no longer finds its edge stops the change).
    pub fn remove_flange(&mut self, i: usize) -> Result<()> {
        let mut next = self.clone();
        if i >= next.flanges.len() {
            return invalid("no such flange");
        }
        next.flanges.remove(i);
        next.rebuild()?;
        *self = next;
        Ok(())
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
    pub fn add_flange(&mut self, pick: [f64; 3], height: f64, angle_deg: f64, position: FlangePosition, down: bool) -> Result<()> {
        check_flange(height, angle_deg)?;
        self.add(FlangeAsked { pick, height, angle_deg, position: position.key().into(), down, hem: false, fold: None })
    }

    /// Adds a flat hem on the part's edge nearest `pick` (part mm): folded right back onto the sheet, `length` to its outer
    /// face, the rule's hem gap inside. The part is unchanged if it fails.
    pub fn add_hem(&mut self, pick: [f64; 3], length: f64, down: bool) -> Result<()> {
        check_flange(length, 180.0)?;
        self.add(FlangeAsked { pick, height: length, angle_deg: 180.0, position: FlangePosition::Inside.key().into(), down, hem: true, fold: None })
    }

    /// Adds a fold along the line through `a` and `b` (part mm, on a face of the panel to fold): `angle_deg` (0…180), up or
    /// `down`, the bend centred on the line or its mould line there. The side away from the larger part moves. The part is
    /// unchanged if it fails.
    pub fn add_fold(&mut self, a: [f64; 3], b: [f64; 3], angle_deg: f64, position: FoldPosition, down: bool) -> Result<()> {
        check_flange(1.0, angle_deg)?;
        let fold = Some(FoldLine { a, b, jog: false });
        self.add(FlangeAsked { pick: a, height: 0.0, angle_deg, position: position.key().into(), down, hem: false, fold })
    }

    /// Adds a jog (a joggle) along the line through `a` and `b`: the side away from the larger part steps `step` mm aside
    /// (face to face) through two bends of `angle_deg`, up or `down`, and stays parallel. The part is unchanged if it fails.
    pub fn add_jog(&mut self, a: [f64; 3], b: [f64; 3], step: f64, angle_deg: f64, down: bool) -> Result<()> {
        check_flange(step, angle_deg)?;
        if angle_deg >= 179.0 {
            return invalid("a jog's bends must be less than 179 degrees");
        }
        let fold = Some(FoldLine { a, b, jog: true });
        self.add(FlangeAsked { pick: a, height: step, angle_deg, position: "centerline".into(), down, hem: false, fold })
    }

    fn add(&mut self, f: FlangeAsked) -> Result<()> {
        let mut sheet = self.sheet.clone();
        apply(&mut sheet, &f, &self.rule)?;
        sheet.solid()?;
        self.sheet = sheet;
        self.flanges.push(f);
        Ok(())
    }

    /// The part as Daedalus faces, in metres in the part's frame: `flat` unfolded, else folded. Flat faces of the exact solid
    /// stay whole (with their holes); curved ones (the bends, round holes' walls) are fine facets with soft edges between.
    pub fn faces(&self, flat: bool) -> Result<Mesh> {
        let mut sheet = self.sheet.clone();
        sheet.flat = flat;
        let body = sheet.solid()?;
        let mut mesh = body_faces(&body)?;
        if flat {
            // the bend centre lines drawn on both faces of the blank, as the Flat pattern tab shows them
            for b in self.sheet.bends() {
                for z in [0.0, self.sheet.t] {
                    let a = mesh.add_vertex([b.start.x / MM, b.start.y / MM, z / MM]);
                    let c = mesh.add_vertex([b.end.x / MM, b.end.y / MM, z / MM]);
                    mesh.lines.push([a, c]);
                }
            }
        }
        Ok(mesh)
    }

    /// The bends as the flat pattern's bend table: (angle degrees, inside radius mm, length mm).
    pub fn bend_table(&self) -> Vec<(f64, f64, f64)> {
        self.sheet.bends().iter().map(|b| (b.angle_deg, b.radius, b.length)).collect()
    }

    /// What a bending shop would likely query, by common rules of thumb (not any shop's own limits): a hole closer to a
    /// bend than 2.5 x thickness + radius (it stretches out of round), a flange shorter than 4 x thickness (the press brake
    /// cannot grip it), a bend radius tighter than the thickness (some materials crack).
    pub fn shop_checks(&self) -> Vec<String> {
        let (t, r) = (self.sheet.t, self.rule.bend_radius);
        let mut out = Vec::new();
        if r < t {
            out.push(format!("Bend radius {} mm is tighter than the {} mm thickness: some materials crack.", fmt_mm(r), fmt_mm(t)));
        }
        for (i, f) in self.flanges.iter().enumerate() {
            if !f.hem && f.height < 4.0 * t {
                out.push(format!("Flange {} is {} mm high, under 4 x thickness ({} mm): it may be too short to bend.", i + 1, fmt_mm(f.height), fmt_mm(4.0 * t)));
            }
        }
        let Ok(regions) = self.sheet.flat_regions() else { return out };
        let bends = self.sheet.bends();
        let near = 2.5 * t + r;
        for region in &regions {
            for (h, hole) in region.holes.iter().enumerate() {
                let pts = sample_loop(hole);
                for (k, b) in bends.iter().enumerate() {
                    // from the hole's edge to the bend zone's edge
                    let d = pts.iter().map(|p| dist_to_segment(*p, [b.start.x, b.start.y], [b.end.x, b.end.y])).fold(f64::INFINITY, f64::min) - b.allowance / 2.0;
                    if d < near {
                        out.push(format!("Hole {} is {} mm from bend {}: under 2.5 x thickness + radius ({} mm), it may pull out of shape.", h + 1, fmt_mm(d.max(0.0)), k + 1, fmt_mm(near)));
                    }
                }
            }
        }
        out
    }

    /// The outline the part was made from, and its holes (part mm, flat), for the Drawing tab: the face's for a part made
    /// of a face, the base panel's for a converted solid.
    pub fn outline(&self) -> Vec<Vec<[f64; 2]>> {
        let base: Vec<&Loop2> = match &self.solid {
            Some(_) => std::iter::once(&self.sheet.base.outer).chain(&self.sheet.base.holes).chain(self.sheet.holes.iter().map(|h| &h.outer)).collect(),
            None => std::iter::once(&self.region.outer).chain(&self.region.holes).collect(),
        };
        base.into_iter().map(sample_loop).collect()
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
pub fn loop_points(lp: &Loop2) -> Vec<[f64; 2]> {
    sample_loop(lp)
}

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

fn fmt_mm(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn dist_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
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

    /// A 120 x 60 mm steel plate, 2 mm (no holes), as a part.
    fn plain_plate() -> SheetPart {
        let outer = Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(120.0, 0.0), Vec2::new(120.0, 60.0), Vec2::new(0.0, 60.0)]).ccw();
        SheetPart::from_region("Steel 2 mm", steel2(), Region2 { outer, holes: Vec::new() }).unwrap()
    }

    /// The top face's height (mm, along +Z) of the part's piece at flat point (x, y), folded.
    fn top_at(p: &SheetPart, x: f64, y: f64) -> f64 {
        let s = &p.sheet;
        // the piece that holds the point in the flat: the flange whose panel starts last before it, else the base
        let mut best: (Option<usize>, f64) = (None, f64::NEG_INFINITY);
        for (i, f) in s.flanges.iter().enumerate() {
            let along = (Vec2::new(x, y) - f.p).dot(f.n) - s.allowance(f);
            if along > 0.0 && along < 1e5 && f.p.dot(f.n) > best.1 {
                best = (Some(i), f.p.dot(f.n));
            }
        }
        crate::util::apply_point(&s.world(best.0), Vec3::new(x, y, s.t)).z
    }

    #[test]
    fn a_fold_across_the_plate_bends_it_and_the_blank_keeps_its_size() {
        let mut p = plain_plate();
        p.add_fold([80.0, -10.0, 2.0], [80.0, 70.0, 2.0], 90.0, FoldPosition::Centre, false).unwrap();
        assert_eq!(p.flanges[0].kind(), StepKind::Fold);
        let table = p.bend_table();
        assert_eq!(table.len(), 1);
        assert!((table[0].0 - 90.0).abs() < 1e-9 && (table[0].2 - 60.0).abs() < 1e-6, "{table:?}");
        let (w, h) = p.sheet.flat_size().unwrap();
        assert!((w - 120.0).abs() < 1e-6 && (h - 60.0).abs() < 1e-6, "the fold takes its bend out of the sheet: {w} x {h}");
        // bent: the short side stands up, about 40 mm less half the bend
        let bent = p.faces(false).unwrap();
        let [sx, _, sz] = size(&bent);
        assert!(sx < 0.1 && sz > 0.03, "{sx} x {sz} m");
        // turned, then taken away: the plate again
        p.set_fold(0, 0.0, 45.0, FoldPosition::Mould, true).unwrap();
        assert!((p.bend_table()[0].0 - 45.0).abs() < 1e-9);
        p.remove_flange(0).unwrap();
        assert!(p.bend_table().is_empty());
    }

    #[test]
    fn a_jog_steps_the_far_side_aside_by_what_was_asked_and_keeps_it_parallel() {
        for (step, angle, down) in [(10.0, 90.0, false), (4.0, 45.0, false), (6.0, 60.0, true)] {
            let what = format!("{step} mm at {angle} degrees{}", if down { ", down" } else { "" });
            let mut p = plain_plate();
            p.add_jog([80.0, -10.0, 2.0], [80.0, 70.0, 2.0], step, angle, down).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(p.flanges[0].kind(), StepKind::Jog);
            let table = p.bend_table();
            assert_eq!(table.len(), 2, "{what}: two bends");
            assert!(table.iter().all(|b| (b.0 - angle).abs() < 1e-6), "{what}: {table:?}");
            // the far end's top face against the base's
            let moved = top_at(&p, 118.0, 30.0) - top_at(&p, 5.0, 30.0);
            assert!((moved.abs() - step).abs() < 1e-6, "{what}: stepped {moved} mm");
            assert_eq!(moved < 0.0, down, "{what}: the way asked");
            let (w, _) = p.sheet.flat_size().unwrap();
            assert!((w - 120.0).abs() < 1e-6, "{what}: the blank keeps its length ({w})");
        }
        // too small for the bends: refused, saying the least
        let mut p = plain_plate();
        let e = p.add_jog([80.0, -10.0, 2.0], [80.0, 70.0, 2.0], 0.5, 90.0, false).unwrap_err().to_string();
        assert!(e.contains("at least"), "{e}");
        assert!(p.flanges.is_empty(), "unchanged");
    }

    #[test]
    fn every_corner_relief_shape_makes_a_part_with_two_flanges_at_a_corner() {
        use crate::sheet::ReliefShape;
        for shape in [ReliefShape::Notch, ReliefShape::Round, ReliefShape::Rectangular, ReliefShape::Tear] {
            let outer = Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(120.0, 0.0), Vec2::new(120.0, 60.0), Vec2::new(0.0, 60.0)]).ccw();
            let rule = RuleValues { corner_relief_shape: shape, corner_relief: 6.0, ..steel2() };
            let mut p = SheetPart::from_region("Steel 2 mm", rule, Region2 { outer, holes: Vec::new() }).unwrap();
            p.add_flange([60.0, 0.0, 0.0], 20.0, 90.0, FlangePosition::Inside, false).unwrap_or_else(|e| panic!("{shape:?}: {e}"));
            p.add_flange([120.0, 30.0, 0.0], 20.0, 90.0, FlangePosition::Inside, false).unwrap_or_else(|e| panic!("{shape:?}: {e}"));
            p.faces(true).unwrap_or_else(|e| panic!("{shape:?}: flat: {e}"));
            p.faces(false).unwrap_or_else(|e| panic!("{shape:?}: bent: {e}"));
            assert_eq!(p.bend_table().len(), 2, "{shape:?}");
        }
    }

    #[test]
    fn parts_without_folds_read_as_before_and_folds_read_back() {
        let mut p = plain_plate();
        p.add_flange([0.0, 30.0, 0.0], 20.0, 90.0, FlangePosition::Inside, false).unwrap();
        let old = p.to_text();
        assert!(!old.contains("\"fold\""), "a flange writes no fold");
        assert_eq!(SheetPart::from_text(&old).unwrap().flanges[0].kind(), StepKind::Flange);
        p.add_jog([80.0, -10.0, 2.0], [80.0, 70.0, 2.0], 8.0, 90.0, false).unwrap();
        let mut back = SheetPart::from_text(&p.to_text()).unwrap();
        assert_eq!(back.flanges[1].kind(), StepKind::Jog);
        back.rebuild().unwrap();
        assert_eq!(back.bend_table().len(), 3);
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
        part.add_flange([50.0, 0.0, 2.0], 30.0, 90.0, FlangePosition::Inside, false).unwrap();
        assert_eq!(part.flanges.len(), 1);
        let s = size(&part.faces(false).unwrap());
        assert!((s[0] - 0.1).abs() < 1e-9 && (s[1] - 0.05).abs() < 1e-9 && (s[2] - 0.03).abs() < 1e-6, "folded {s:?}");
        // flat: 100 wide, (50 - 4) + (30 - 4) + BA long (outside setback T + R = 4 mm)
        let ba = std::f64::consts::FRAC_PI_2 * (2.0 + 0.44 * 2.0);
        let (w, h) = part.flat_size().unwrap();
        assert!((w - 100.0).abs() < 1e-9 && (h - (46.0 + 26.0 + ba)).abs() < 1e-6, "{w} x {h}");
        let flat_faces = part.faces(true).unwrap();
        let flat = size(&flat_faces);
        assert!((flat[2] - 0.002).abs() < 1e-9 && (flat[1] * 1000.0 - h).abs() < 1e-6, "flat {flat:?}");
        // the bend line on top and bottom, across the whole blank, inside it
        assert_eq!(flat_faces.lines.len(), 2);
        let [a, b] = flat_faces.lines[0];
        let (pa, pb) = (flat_faces.positions[a as usize], flat_faces.positions[b as usize]);
        // inside position: the panel edge 4 mm in (T + R), the bend's centre half the allowance past it
        assert!(((pb[0] - pa[0]).abs() - 0.1).abs() < 1e-9 && pa[1] == pb[1] && (pa[1] * 1000.0 - (4.0 - ba / 2.0)).abs() < 1e-9, "{pa:?} {pb:?}");
        assert_eq!(part.bend_table().len(), 1);
        let dxf = part.flat_dxf(Units::Millimeters).unwrap();
        let count = |n: &str| dxf.lines().filter(|l| *l == n).count();
        assert_eq!(count("CIRCLE"), 1, "the hole is one true circle");
        assert_eq!(count("LINE"), 4 + 1, "four sides and the bend line");
        assert_eq!(count("BEND"), 1 + 1, "the bend line and its layer");
        // a bad flange leaves the part as it was
        let before = part.clone();
        assert!(part.add_flange([50.0, 0.0, 2.0], -5.0, 90.0, FlangePosition::Inside, false).is_err());
        assert!(part.add_flange([50.0, 0.0, 2.0], 10.0, 200.0, FlangePosition::Inside, false).is_err());
        assert_eq!(part, before);
    }

    #[test]
    fn a_placed_part_goes_out_as_one_exact_step_solid_in_mm() {
        let (mut part, _) = part_from_face(&plate_face(), 0, "Steel 2 mm", steel2()).unwrap();
        part.add_flange([50.0, 0.0, 2.0], 30.0, 90.0, FlangePosition::Inside, false).unwrap();
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

    #[test]
    fn flanges_change_and_go_and_a_new_rule_keeps_them() {
        let (mut part, _) = part_from_face(&plate_face(), 0, "Steel 2 mm", steel2()).unwrap();
        part.add_flange([50.0, 0.0, 2.0], 30.0, 90.0, FlangePosition::Inside, false).unwrap();
        part.add_flange([50.0, 50.0, 2.0], 20.0, 90.0, FlangePosition::Inside, false).unwrap();
        let h = |p: &SheetPart| p.flat_size().unwrap().1;
        let ba = std::f64::consts::FRAC_PI_2 * (2.0 + 0.44 * 2.0);
        assert!((h(&part) - (26.0 + 42.0 + 16.0 + 2.0 * ba)).abs() < 1e-6, "{}", h(&part));
        // the first flange 40 high: 10 more in the blank
        part.set_flange(0, 40.0, 90.0, FlangePosition::Inside, false).unwrap();
        assert!((h(&part) - (36.0 + 42.0 + 16.0 + 2.0 * ba)).abs() < 1e-6, "{}", h(&part));
        // a softer bend radius changes both bends, the flanges stay
        part.set_rule("Steel 2 mm R3", RuleValues { bend_radius: 3.0, ..part.rule }).unwrap();
        let ba3 = std::f64::consts::FRAC_PI_2 * (3.0 + 0.44 * 2.0);
        assert!((h(&part) - (35.0 + 40.0 + 15.0 + 2.0 * ba3)).abs() < 1e-6, "{}", h(&part));
        assert_eq!(part.flanges.len(), 2);
        // the second goes: one bend left
        part.remove_flange(1).unwrap();
        assert_eq!(part.bend_table().len(), 1);
        // bad changes leave it alone
        let before = part.clone();
        assert!(part.set_flange(0, 0.0, 90.0, FlangePosition::Inside, false).is_err());
        assert!(part.set_rule("x", RuleValues { k_factor: 2.0, ..part.rule }).is_err());
        assert!(part.remove_flange(5).is_err());
        assert_eq!(part, before);
        // the text round trip rebuilds the same
        let mut back = SheetPart::from_text(&part.to_text()).unwrap();
        back.rebuild().unwrap();
        assert_eq!(back.sheet, part.sheet);
    }

    #[test]
    fn shop_checks_name_a_hole_near_a_bend_a_short_flange_and_a_tight_radius() {
        // the 20 mm hole is centred 25 mm in from the y = 0 edge: its edge 15 mm from that edge
        let (mut part, _) = part_from_face(&plate_face(), 0, "Steel 2 mm", steel2()).unwrap();
        assert!(part.shop_checks().is_empty(), "{:?}", part.shop_checks());
        part.add_flange([50.0, 0.0, 2.0], 30.0, 90.0, FlangePosition::Inside, false).unwrap();
        assert!(part.shop_checks().is_empty(), "15 mm is far enough from the bend: {:?}", part.shop_checks());
        // a flange on the x = 0 edge: the hole's edge is 20 mm from it, minus the inside setback
        let mut near = part.clone();
        near.add_flange([0.0, 25.0, 2.0], 30.0, 90.0, FlangePosition::Inside, false).unwrap();
        assert!(near.shop_checks().is_empty(), "{:?}", near.shop_checks());
        // a 6 mm flange is under 4 x 2 mm
        let mut short = part.clone();
        short.add_flange([50.0, 50.0, 2.0], 6.0, 90.0, FlangePosition::Inside, false).unwrap();
        let c = short.shop_checks();
        assert!(c.len() == 1 && c[0].contains("Flange 2 is 6 mm high"), "{c:?}");
        // a hole 3 mm from the plate's edge, then a flange there
        let mut m = plate_face();
        for p in &mut m.positions[4..] {
            p[0] += 0.0;
            p[1] -= 0.012; // centre at y = 13 mm: its edge 3 mm in
        }
        let (mut close, _) = part_from_face(&m, 0, "Steel 2 mm", steel2()).unwrap();
        close.add_flange([50.0, 0.0, 2.0], 30.0, 90.0, FlangePosition::Outside, false).unwrap();
        let c = close.shop_checks();
        assert!(c.iter().any(|s| s.starts_with("Hole 1 is") && s.contains("bend 1")), "{c:?}");
        // a radius under the thickness
        close.set_rule("Tight", RuleValues { bend_radius: 1.0, ..close.rule }).unwrap();
        assert!(close.shop_checks().iter().any(|s| s.starts_with("Bend radius 1 mm")), "{:?}", close.shop_checks());
    }

    #[test]
    fn a_flange_bends_down_and_a_hem_folds_flat() {
        let (mut part, _) = part_from_face(&plate_face(), 0, "Steel 2 mm", steel2()).unwrap();
        part.add_flange([50.0, 0.0, 2.0], 30.0, 90.0, FlangePosition::Inside, true).unwrap();
        let m = part.faces(false).unwrap();
        let z: Vec<f64> = m.positions.iter().map(|p| p[2]).collect();
        let (lo, hi) = (z.iter().cloned().fold(f64::MAX, f64::min), z.iter().cloned().fold(f64::MIN, f64::max));
        assert!((hi - 0.002).abs() < 1e-9 && (lo + 0.028).abs() < 1e-6, "down: the flange hangs below, {lo} .. {hi}");
        // a hem on the far edge: folded back, the part no taller than two sheets and the gap
        part.add_hem([50.0, 50.0, 2.0], 10.0, false).unwrap();
        assert_eq!(part.bend_table().len(), 2);
        let m = part.faces(false).unwrap();
        let hi = m.positions.iter().map(|p| p[2]).fold(f64::MIN, f64::max);
        assert!(hi <= 0.004 + 0.0001 + 1e-9, "the hem lies flat on the sheet: top at {hi}");
        // the hem changed through the same path as a flange: longer, and still a hem
        part.set_flange(1, 12.0, 180.0, FlangePosition::Inside, false).unwrap();
        assert!(part.flanges[1].hem && part.flanges[1].height == 12.0);
        assert_eq!(part.bend_table().len(), 2);
        // both survive a rebuild (as after reading the file back)
        let mut back = SheetPart::from_text(&part.to_text()).unwrap();
        back.rebuild().unwrap();
        assert_eq!(back.sheet, part.sheet);
    }

    /// A profile in the XZ plane (metres, counter-clockwise seen from -Y) pushed `depth` along +Y: a closed solid.
    fn extruded(profile: &[[f64; 2]], depth: f64) -> Mesh {
        let mut m = Mesh::new();
        let n = profile.len() as u32;
        for y in [0.0, depth] {
            for p in profile {
                m.add_vertex([p[0], y, p[1]]);
            }
        }
        // the end at y = 0 faces -Y: the profile's order seen from -Y is counter-clockwise
        m.add_face((0..n).collect());
        m.add_face((0..n).rev().map(|i| i + n).collect());
        for i in 0..n {
            let j = (i + 1) % n;
            m.add_face(vec![i, i + n, j + n, j]);
        }
        m
    }

    #[test]
    fn an_l_shaped_solid_with_2mm_walls_converts_to_a_part_with_one_bend() {
        // base 40 long, upright 30 high, 2 thick, 50 deep; the outside corner at the origin, sharp
        let l = extruded(&[[0.0, 0.0], [0.04, 0.0], [0.04, 0.002], [0.002, 0.002], [0.002, 0.03], [0.0, 0.03]], 0.05);
        assert!(daedalus_core::inspect::inspect(&l, l.tolerance()).is_solid(), "the test solid is closed");
        let rule = RuleValues { k_factor: 0.44, bend_radius: 2.0, ..RuleValues::steel(2.0) };
        // the bottom stays put; the outside corner edge (x = 0, z = 0) is the bend
        let part = part_from_solid(&l, [0.02, 0.025, 0.0], &[[0.0, 0.025, 0.0]], "Steel 2 mm", rule).unwrap();
        assert!((part.thickness() - 2.0).abs() < 1e-9, "the walls' own thickness: {}", part.thickness());
        let bends = part.bend_table();
        assert_eq!(bends.len(), 1, "{:?}", part.notes);
        assert!((bends[0].0 - 90.0).abs() < 1e-6 && (bends[0].1 - 2.0).abs() < 1e-9 && (bends[0].2 - 50.0).abs() < 1e-6, "{bends:?}");
        // outer sizes kept: flat = (40 - 4) + (30 - 4) + BA
        let ba = std::f64::consts::FRAC_PI_2 * (2.0 + 0.44 * 2.0);
        let (w, h) = part.flat_size().unwrap();
        let long = w.max(h);
        assert!((long - (36.0 + 26.0 + ba)).abs() < 1e-6 && (w.min(h) - 50.0).abs() < 1e-6, "{w} x {h}");
        // folded it fills the solid's box
        let m = part.faces(false).unwrap();
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in &m.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        assert!((hi[0] - lo[0] - 0.04).abs() < 1e-6 && (hi[1] - lo[1] - 0.05).abs() < 1e-6 && (hi[2] - lo[2] - 0.03).abs() < 1e-6, "{lo:?} {hi:?}");
        // a new radius rebuilds it from the solid; the text round trip too
        let mut softer = part.clone();
        softer.set_rule("Steel R3", RuleValues { bend_radius: 3.0, ..softer.rule }).unwrap();
        assert!((softer.bend_table()[0].1 - 3.0).abs() < 1e-9);
        let mut back = SheetPart::from_text(&part.to_text()).unwrap();
        back.rebuild().unwrap();
        assert_eq!(back.sheet, part.sheet);
        // a flange added on the converted part's far edge
        let mut more = part.clone();
        more.add_flange([40.0, 25.0, 2.0], 15.0, 90.0, FlangePosition::Inside, false).unwrap();
        assert_eq!(more.bend_table().len(), 2);
    }
}
