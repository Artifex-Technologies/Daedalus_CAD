//! STEP (ISO 10303-21) in AP214, "automotive design", the schema every CAD program and fabrication service reads; to be the
//! format new models are saved in (decided 2026-11-08), so a model never needs Daedalus to be opened. Written from the
//! standard (Part 21 for the file, Part 42 for geometry and topology, AP214 for products, assemblies and colours) with no
//! geometry kernel: every Daedalus face is flat, so it is a PLANE bounded by straight edges.
//!
//! How a model is written:
//! - each face an ADVANCED_FACE on a PLANE, its outline a FACE_OUTER_BOUND and each hole a FACE_BOUND, every bound an
//!   EDGE_LOOP of ORIENTED_EDGEs over EDGE_CURVEs (LINEs) between VERTEX_POINTs; an edge two faces share is written once;
//! - faces joined edge to edge into a closed surface round a volume, facing out, are a MANIFOLD_SOLID_BREP (CLOSED_SHELL); the
//!   rest (SketchUp-style models are often not closed) a SHELL_BASED_SURFACE_MODEL of OPEN_SHELLs;
//! - the model is a PRODUCT, and so is every group and component, each placed in the one holding it by a
//!   NEXT_ASSEMBLY_USAGE_OCCURRENCE with its copy's placement: a component used five times is written once. STEP places a
//!   part by turning and moving it only, so a scaled or mirrored copy becomes a part of its own, already scaled; a Dynamic
//!   Component is written as it is drawn (Hidden, Copies, Material);
//! - paint is face colour (STYLED_ITEM, SURFACE_STYLE_USAGE, COLOUR_RGB; a texture leaves its colour);
//! - lengths are in the units asked for (millimetres when unsure), with the model's tolerance as the uncertainty;
//! - a face a few micrometres off flat (as converted SketchUp faces are) is still one PLANE; one further off is written as
//!   its triangles (see `flatness`), the edges between them soft;
//! - loose lines are wireframe, as other programs show them: POLYLINEs (chained where they meet end to end) in a
//!   GEOMETRIC_CURVE_SET of each part's GEOMETRICALLY_BOUNDED_WIREFRAME_SHAPE_REPRESENTATION;
//! - soft edges (drawn smooth, so curved surfaces show no facets) are the EDGE_CURVEs of a PRESENTATION_LAYER_ASSIGNMENT
//!   named "Soft edges", made invisible (INVISIBILITY): edges only, never faces.
//!
//! What STEP has no place for (notes, textures, Dynamic Component formulas, sheet-metal steps) is listed in
//! [`Written::left_out`], for the caller to tell the person.
//!
//! Reading takes what this writer writes and flat-faced STEP from other programs: ADVANCED_FACE or FACE_SURFACE on a PLANE
//! bounded by lines, polylines, circles or ellipses (curved edges in segments, as Daedalus draws circles), faceted B-reps,
//! assemblies with their placements (MAPPED_ITEM copies too), colours, units, wireframe curves as loose lines, and edges in
//! the "Soft edges" layer or any layer made invisible as soft edges. Faces on curved surfaces (cylinders, cones,
//! spheres, tori, B-splines) are not read yet: [`read`] names them in [`Read::skipped`] and reads the rest, and
//! [`read_scene`] refuses such a file, naming them, rather than open it with faces missing.

use crate::{IoError, MeshExporter, MeshImporter};
use daedalus_core::scene::{Context, Definition, Instance, Scene, Xform};
use daedalus_core::units::Units;
use daedalus_core::{Mesh, Paint};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

/// The schema written: AP214's first edition, which every STEP reader takes.
const SCHEMA: &str = "AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }";
/// Segments in a whole circle or ellipse read from a file: as many as Daedalus draws a circle with.
const CIRCLE_SEGMENTS: f64 = 24.0;

/// STEP for [`crate::import_mesh`] and [`crate::export_mesh`]; a whole model goes through [`write`] and [`read`].
pub struct Step;

impl MeshImporter for Step {
    fn extensions(&self) -> &'static [&'static str] {
        &["step", "stp"]
    }

    /// The file's geometry as one mesh, every part put in place.
    fn import(&self, bytes: &[u8]) -> Result<Mesh, IoError> {
        Ok(read_scene(bytes)?.flatten())
    }
}

impl MeshExporter for Step {
    fn extensions(&self) -> &'static [&'static str] {
        &["step", "stp"]
    }

    fn export(&self, mesh: &Mesh) -> Result<Vec<u8>, IoError> {
        write_scene(&Scene::from_mesh(mesh.clone()))
    }
}

type V3 = [f64; 3];
const X: V3 = [1.0, 0.0, 0.0];
const Z: V3 = [0.0, 0.0, 1.0];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn plus(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn times(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: V3) -> Option<V3> {
    let l = length(a);
    (l > 0.0 && l.is_finite()).then(|| times(a, 1.0 / l))
}

fn column(xf: &Xform, c: usize) -> V3 {
    [xf.0[0][c], xf.0[1][c], xf.0[2][c]]
}

/// The part of `d` at right angles to the unit vector `n`, unit length (any such direction when `d` has none).
fn across(d: V3, n: V3) -> V3 {
    unit(sub(d, times(n, dot(d, n)))).unwrap_or_else(|| {
        let other = if n[0].abs() < 0.9 { X } else { [0.0, 1.0, 0.0] };
        unit(cross(n, other)).unwrap_or(X)
    })
}

/// Twice a loop's area, as a vector along its normal (counter-clockwise about it), from a fan round its first corner.
fn area_normal(pos: &[V3], ring: &[u32]) -> V3 {
    let Some(&first) = ring.first() else { return [0.0; 3] };
    let o = pos[first as usize];
    let mut n = [0.0; 3];
    for i in 1..ring.len().saturating_sub(1) {
        n = plus(n, cross(sub(pos[ring[i] as usize], o), sub(pos[ring[i + 1] as usize], o)));
    }
    n
}

/// The diagonal of a loop's bounding box.
fn extent(pos: &[V3], ring: &[u32]) -> f64 {
    let Some(&first) = ring.first() else { return 0.0 };
    let (mut lo, mut hi) = (pos[first as usize], pos[first as usize]);
    for &v in ring {
        let p = pos[v as usize];
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    length(sub(hi, lo))
}

/// A length unit: a number in it is `number * mul / div` metres (a millimetre is 1 / 1000, exactly).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Scale {
    mul: f64,
    div: f64,
}

impl Scale {
    const MM: Scale = Scale { mul: 1.0, div: 1000.0 };

    fn of(units: Units) -> Scale {
        match units {
            Units::Millimetres => Scale::MM,
            Units::Centimetres => Scale { mul: 1.0, div: 100.0 },
            Units::Metres => Scale { mul: 1.0, div: 1.0 },
            Units::Inches | Units::FeetInches => Scale { mul: 25.4, div: 1000.0 },
        }
    }

    fn metres(self, number: f64) -> f64 {
        number * self.mul / self.div
    }

    fn number(self, metres: f64) -> f64 {
        metres * self.div / self.mul
    }

    /// The Daedalus units this is, if it is one of them (feet: feet and inches).
    fn units(self) -> Option<Units> {
        let m = self.mul / self.div;
        [(0.001, Units::Millimetres), (0.01, Units::Centimetres), (1.0, Units::Metres), (0.0254, Units::Inches), (0.3048, Units::FeetInches)]
            .into_iter()
            .find(|(k, _)| (m / k - 1.0).abs() < 1e-9)
            .map(|(_, u)| u)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Writing

/// How [`write`] writes.
#[derive(Debug, Clone)]
pub struct WriteOptions {
    /// The unit the file's lengths are in: the units the model is shown in (millimetres when unsure). Feet and inches are
    /// written in inches.
    pub units: Units,
    /// The file's name, for its header (FILE_NAME); the top product, the whole model, is named after it. May be empty.
    pub name: String,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions { units: Units::Millimetres, name: String::new() }
    }
}

/// A STEP file as written, and what went into it.
#[derive(Debug, Clone, Default)]
pub struct Written {
    pub text: String,
    /// PRODUCTs: the model, one per group or component, and one per scaled or mirrored copy or Dynamic Component copy.
    pub products: usize,
    /// Closed solids (MANIFOLD_SOLID_BREP), the other shells (in SHELL_BASED_SURFACE_MODELs), and faces, each part once.
    pub solids: usize,
    pub shells: usize,
    pub faces: usize,
    /// What the file could not hold, one line each ("3 guides", "12 loose lines"); empty when nothing was lost.
    pub left_out: Vec<String>,
}

/// Writes a whole model to STEP in millimetres (see [`write`]).
pub fn write_scene(scene: &Scene) -> Result<Vec<u8>, IoError> {
    Ok(write(scene, &WriteOptions::default())?.text.into_bytes())
}

/// What a part is in Daedalus, kept in its PRODUCT's description so it comes back the same (other programs show the words).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Model,
    Group,
    Component { glue: bool, cut: bool },
}

impl Kind {
    fn of(d: &Definition) -> Kind {
        if d.component {
            Kind::Component { glue: d.glue, cut: d.cut }
        } else {
            Kind::Group
        }
    }

    fn description(self) -> String {
        match self {
            Kind::Model => "Daedalus model".into(),
            Kind::Group => "Daedalus group".into(),
            Kind::Component { glue, cut } => format!("Daedalus component{}{}", if glue { " glue" } else { "" }, if cut { " cut" } else { "" }),
        }
    }

    fn word(self) -> &'static str {
        match self {
            Kind::Model => "Model",
            Kind::Group => "Group",
            Kind::Component { .. } => "Component",
        }
    }
}

/// One PRODUCT to write: a definition (as it is, or scaled for a copy STEP cannot place), or the model itself.
struct Part<'s> {
    name: String,
    kind: Kind,
    /// Its own faces, in its own coordinates (metres).
    mesh: Cow<'s, Mesh>,
    /// What is placed in it: another part, and where (a turn and a move only).
    children: Vec<(usize, Xform)>,
}

/// The parts a scene is written as. Part 0 is the model.
struct Plan<'s> {
    scene: &'s Scene,
    parts: Vec<Part<'s>>,
    /// The part made for a definition, as it is (None) or with a scale built in (that linear map's bits).
    made: HashMap<(usize, Option<[u64; 9]>), usize>,
    building: Vec<(usize, Option<[u64; 9]>)>,
    reached: Vec<bool>,
    /// Copies placed with a scale or mirror, Dynamic Component copies, and copies of what is missing or holds itself.
    reshaped: usize,
    dynamic: usize,
    broken: usize,
}

impl<'s> Plan<'s> {
    fn new(scene: &'s Scene, name: String) -> Plan<'s> {
        let mut plan = Plan { scene, parts: Vec::new(), made: HashMap::new(), building: Vec::new(), reached: vec![false; scene.defs.len()], reshaped: 0, dynamic: 0, broken: 0 };
        plan.parts.push(Part { name, kind: Kind::Model, mesh: Cow::Borrowed(&scene.root.mesh), children: Vec::new() });
        let children = plan.children(&scene.root, None, 0);
        plan.parts[0].children = children;
        plan
    }

    /// The parts placed in a context, where (in the coordinates of the part holding them, which has `scale` built in).
    fn children(&mut self, ctx: &'s Context, scale: Option<&Xform>, depth: usize) -> Vec<(usize, Xform)> {
        let scene = self.scene;
        let mut out = Vec::new();
        for inst in &ctx.instances {
            let Some(d) = scene.defs.get(inst.def) else {
                self.broken += 1;
                continue;
            };
            self.reached[inst.def] = true;
            if !d.attributes.is_empty() {
                // a Dynamic Component: as it is drawn now (its copies, hidden or not, its material), a part of its own
                let mut mesh = Mesh::new();
                scene.flatten_instance(inst, &scale.copied().unwrap_or(Xform::IDENTITY), &mut mesh);
                self.dynamic += 1;
                self.parts.push(Part { name: d.name.clone(), kind: Kind::of(d), mesh: Cow::Owned(mesh), children: Vec::new() });
                out.push((self.parts.len() - 1, Xform::IDENTITY));
                continue;
            }
            let xf = match scale {
                Some(s) => s.after(&inst.xf),
                None => inst.xf,
            };
            let (place, rest) = split(&xf);
            if rest.is_some() {
                self.reshaped += 1;
            }
            match self.part(inst.def, rest, depth + 1) {
                Some(p) => out.push((p, place)),
                None => self.broken += 1,
            }
        }
        out
    }

    /// The part for definition `def` with `scale` built into its geometry (None: as it is), made the first time it is asked for.
    fn part(&mut self, def: usize, scale: Option<Xform>, depth: usize) -> Option<usize> {
        let key = (def, scale.map(|s| linear_bits(&s)));
        if let Some(&p) = self.made.get(&key) {
            return Some(p);
        }
        // a definition that holds itself (only in a damaged model) is not followed round
        if depth > 64 || self.building.contains(&key) {
            return None;
        }
        let scene = self.scene;
        let d = &scene.defs[def];
        self.building.push(key);
        let children = self.children(&d.ctx, scale.as_ref(), depth);
        self.building.pop();
        let mesh = match &scale {
            None => Cow::Borrowed(&d.ctx.mesh),
            Some(s) => Cow::Owned(d.ctx.mesh.transformed(s)),
        };
        self.parts.push(Part { name: d.name.clone(), kind: Kind::of(d), mesh, children });
        self.made.insert(key, self.parts.len() - 1);
        Some(self.parts.len() - 1)
    }

    /// The order the parts are written in: the model, the definitions as they are in the scene's order (so a file read back
    /// numbers them the same), then the scaled ones and Dynamic Component copies.
    fn order(&self) -> Vec<usize> {
        let mut plain: Vec<(usize, usize)> = self.made.iter().filter(|((_, s), _)| s.is_none()).map(|(&(d, _), &p)| (d, p)).collect();
        plain.sort_unstable();
        let mut order = vec![0];
        order.extend(plain.iter().map(|&(_, p)| p));
        let mut taken = vec![false; self.parts.len()];
        for &p in &order {
            taken[p] = true;
        }
        order.extend((0..self.parts.len()).filter(|&p| !taken[p]));
        order
    }
}

fn linear_bits(xf: &Xform) -> [u64; 9] {
    let m = &xf.0;
    [m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2]].map(f64::to_bits)
}

/// A copy's placement as STEP can place it, a turn and a move, and what is left to build into the part: its scale and
/// mirror (along the part's own axes), or the whole linear map if it slants (None: the placement is all of it).
fn split(xf: &Xform) -> (Xform, Option<Xform>) {
    let c = [column(xf, 0), column(xf, 1), column(xf, 2)];
    let t = xf.origin();
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let rigid = (0..3).all(|i| (0..3).all(|j| near(dot(c[i], c[j]), if i == j { 1.0 } else { 0.0 }))) && xf.det() > 0.0;
    if rigid {
        return (*xf, None);
    }
    let l = c.map(length);
    let square = l.iter().all(|&x| x > 1e-12) && (0..3).all(|i| (i + 1..3).all(|j| dot(c[i], c[j]).abs() < 1e-9 * l[i] * l[j]));
    if square {
        // turned, then scaled along its own axes, and mirrored across its third one when the determinant is negative
        let flip = if xf.det() < 0.0 { -1.0 } else { 1.0 };
        let k = [l[0], l[1], flip * l[2]];
        let mut place = [[0.0; 4]; 3];
        let mut rest = [[0.0; 4]; 3];
        for r in 0..3 {
            for j in 0..3 {
                place[r][j] = c[j][r] / k[j];
            }
            place[r][3] = t[r];
            rest[r][r] = k[r];
        }
        return (Xform(place), Some(Xform(rest)));
    }
    let mut rest = *xf;
    for row in &mut rest.0 {
        row[3] = 0.0;
    }
    (Xform::translation(t), Some(rest))
}

/// A face ready to write: its loops (the outline first; vertex numbers of its part's mesh), its unit normal, its colour.
struct Poly {
    loops: Vec<Vec<u32>>,
    normal: V3,
    rgb: Option<[u8; 3]>,
}

/// Faces joined edge to edge, written as one shell.
struct Shell {
    faces: Vec<usize>,
    /// Every edge shared by two of its faces, used once each way.
    closed: bool,
    /// Closed, one fan of faces round every corner, and round a volume facing out: a solid.
    solid: bool,
}

#[derive(Default)]
struct Tally {
    faces: usize,
    solids: usize,
    shells: usize,
    /// Faces with no area, holes with none, faces not flat (written as their triangles), loose lines of no length.
    degenerate: usize,
    holes_dropped: usize,
    split: usize,
    lines_dropped: usize,
}

/// A loop without repeated corners (a corner at the same place as the one before it), or None if fewer than three are left
/// or it names a vertex the mesh does not have.
fn clean(mesh: &Mesh, ring: &[u32]) -> Option<Vec<u32>> {
    let pos = &mesh.positions;
    if ring.iter().any(|&v| v as usize >= pos.len()) {
        return None;
    }
    let mut out: Vec<u32> = Vec::with_capacity(ring.len());
    for &v in ring {
        if out.last().is_some_and(|&l| pos[l as usize] == pos[v as usize]) {
            continue;
        }
        out.push(v);
    }
    while out.len() > 1 && pos[out[0] as usize] == pos[out[out.len() - 1] as usize] {
        out.pop();
    }
    (out.len() >= 3).then_some(out)
}

/// How far a face's corners may stand off its plane and it still be written as one flat face. A model converted from
/// SketchUp has faces a few micrometres off flat (its points rounded on the way), which are flat as drawn; so a thousand
/// times the model's tolerance, but no more than 0.1 mm unless a hundred times the tolerance is (a 10 m house or a 1 m
/// chair: 0.1 mm; a 100 mm part: 10 um).
fn flatness(mesh: &Mesh) -> f64 {
    let tol = mesh.tolerance();
    (tol * 1000.0).min(1e-4).max(tol * 100.0)
}

/// A mesh's faces ready to write, and the edges inside faces written as triangles. A face whose corners are not in one plane
/// is written as its triangles, so the file stays exact, the edges between them soft so it still looks one face; faces with
/// no area are left out. A face faces the way its loops together wind, and the loop round the most area is its outline (a
/// converted face may list a hole first).
fn polys(mesh: &Mesh, tally: &mut Tally) -> (Vec<Poly>, Vec<(u32, u32)>) {
    let flat = flatness(mesh);
    let pos = &mesh.positions;
    let mut out = Vec::with_capacity(mesh.faces.len());
    let mut inner = Vec::new();
    for f in 0..mesh.faces.len() {
        let rgb = mesh.color_of(f).map(|c| c.rgb);
        let Some(outline) = clean(mesh, &mesh.faces[f]) else {
            tally.degenerate += 1;
            continue;
        };
        let mut loops = vec![outline];
        for h in mesh.holes_of(f) {
            match clean(mesh, h) {
                Some(h) => loops.push(h),
                None => tally.holes_dropped += 1,
            }
        }
        let areas: Vec<V3> = loops.iter().map(|l| area_normal(pos, l)).collect();
        let n = areas.iter().fold([0.0; 3], |s, &a| plus(s, a));
        let size = loops.iter().map(|l| extent(pos, l)).fold(0.0, f64::max);
        let Some(u) = unit(n).filter(|_| length(n) > 1e-12 * size * size) else {
            tally.degenerate += 1;
            continue;
        };
        let outer = (0..loops.len()).max_by(|&i, &j| dot(areas[i], u).abs().total_cmp(&dot(areas[j], u).abs())).unwrap_or(0);
        loops.swap(0, outer);
        let at = pos[loops[0][0] as usize];
        let (lo, hi) = loops.iter().flatten().map(|&v| dot(sub(pos[v as usize], at), u)).fold((0.0f64, 0.0f64), |(lo, hi), d| (lo.min(d), hi.max(d)));
        if (hi - lo) / 2.0 > flat {
            tally.split += 1;
            let key = |a: u32, b: u32| (a.min(b), a.max(b));
            let own: HashSet<(u32, u32)> = loops.iter().flat_map(|l| (0..l.len()).map(move |i| key(l[i], l[(i + 1) % l.len()]))).collect();
            for t in mesh.triangulate_face(f) {
                let t = t.to_vec();
                let n = area_normal(pos, &t);
                if let Some(u) = unit(n).filter(|_| length(n) > 1e-12 * size * size) {
                    inner.extend((0..3).map(|i| key(t[i], t[(i + 1) % 3])).filter(|e| !own.contains(e)));
                    out.push(Poly { loops: vec![t], normal: u, rgb });
                }
            }
            continue;
        }
        out.push(Poly { loops, normal: u, rgb });
    }
    (out, inner)
}

/// Loose lines as chains, each from a corner where one must start (an end, or where more than two lines meet) through the
/// corners where just two meet, so a polyline drawn comes back whole; a closed ring of lines goes round once.
fn chains(lines: &[[u32; 2]]) -> Vec<Vec<u32>> {
    let mut at: HashMap<u32, Vec<usize>> = HashMap::new();
    for (k, &[a, b]) in lines.iter().enumerate() {
        at.entry(a).or_default().push(k);
        at.entry(b).or_default().push(k);
    }
    let walk = |start: u32, used: &mut [bool]| -> Option<Vec<u32>> {
        let mut chain = vec![start];
        let mut v = start;
        while let Some(&k) = at[&v].iter().find(|&&k| !used[k]) {
            used[k] = true;
            let [a, b] = lines[k];
            v = if a == v { b } else { a };
            chain.push(v);
            if at[&v].len() != 2 || v == start {
                break;
            }
        }
        (chain.len() > 1).then_some(chain)
    };
    let mut used = vec![false; lines.len()];
    let mut starts: Vec<u32> = at.iter().filter(|(_, ks)| ks.len() != 2).map(|(&v, _)| v).collect();
    starts.sort_unstable();
    let mut out = Vec::new();
    for s in starts {
        while let Some(c) = walk(s, &mut used) {
            out.push(c);
        }
    }
    for k in 0..lines.len() {
        if !used[k] {
            out.extend(walk(lines[k][0], &mut used));
        }
    }
    out
}

/// Faces in shells: two faces sharing an edge, each the other way along it, are in one shell; a shell whose every edge is
/// shared so is closed, and a solid if it also has one fan of faces round each corner and faces out round a volume.
fn shells(mesh: &Mesh, polys: &[Poly]) -> Vec<Shell> {
    let mut uses: HashMap<(u32, u32), Vec<(usize, bool)>> = HashMap::new();
    for (k, p) in polys.iter().enumerate() {
        for l in &p.loops {
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                uses.entry((a.min(b), a.max(b))).or_default().push((k, a < b));
            }
        }
    }
    let good = |u: &[(usize, bool)]| u.len() == 2 && u[0].1 != u[1].1;
    let mut parent: Vec<usize> = (0..polys.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for u in uses.values().filter(|u| good(u)) {
        let (a, b) = (find(&mut parent, u[0].0), find(&mut parent, u[1].0));
        // the lower number leads, so the shells come out the same whatever order the edges are visited in
        parent[a.max(b)] = a.min(b);
    }
    let mut slot: HashMap<usize, usize> = HashMap::new();
    let mut out: Vec<Shell> = Vec::new();
    let mut shell_of = vec![0; polys.len()];
    for (k, of) in shell_of.iter_mut().enumerate() {
        let r = find(&mut parent, k);
        let s = *slot.entry(r).or_insert_with(|| {
            out.push(Shell { faces: Vec::new(), closed: true, solid: false });
            out.len() - 1
        });
        out[s].faces.push(k);
        *of = s;
    }
    for u in uses.values().filter(|u| !good(u)) {
        for &(k, _) in u {
            out[shell_of[k]].closed = false;
        }
    }
    for s in &mut out {
        s.solid = s.closed && one_fan_per_corner(polys, &s.faces) && encloses(mesh, polys, &s.faces);
    }
    out
}

/// Whether a closed shell's faces make one fan round each of its corners (not two cones meeting at a point).
fn one_fan_per_corner(polys: &[Poly], faces: &[usize]) -> bool {
    // at each corner: the vertex and the one before it, to the one after it
    let mut next: HashMap<(u32, u32), u32> = HashMap::new();
    let mut corners: HashMap<u32, (u32, usize)> = HashMap::new();
    for &k in faces {
        for l in &polys[k].loops {
            let n = l.len();
            for i in 0..n {
                let (before, v, after) = (l[(i + n - 1) % n], l[i], l[(i + 1) % n]);
                if next.insert((v, before), after).is_some() {
                    return false;
                }
                corners.entry(v).or_insert((before, 0)).1 += 1;
            }
        }
    }
    // round a vertex from one corner to the next across the edges between them: one fan comes back after all of them
    corners.iter().all(|(&v, &(start, count))| {
        let (mut at, mut steps) = (start, 0);
        loop {
            let Some(&after) = next.get(&(v, at)) else { return false };
            steps += 1;
            at = after;
            if at == start || steps > count {
                return steps == count;
            }
        }
    })
}

/// Whether a closed shell faces out round a volume (an inside-out or flat one is no solid).
fn encloses(mesh: &Mesh, polys: &[Poly], faces: &[usize]) -> bool {
    let pos = &mesh.positions;
    let o = pos[polys[faces[0]].loops[0][0] as usize];
    let (mut lo, mut hi) = (o, o);
    let mut six_volume = 0.0;
    for &k in faces {
        for l in &polys[k].loops {
            let p = |i: usize| sub(pos[l[i] as usize], o);
            for i in 1..l.len() - 1 {
                six_volume += dot(p(0), cross(p(i), p(i + 1)));
            }
            for &v in l {
                for c in 0..3 {
                    lo[c] = lo[c].min(pos[v as usize][c]);
                    hi[c] = hi[c].max(pos[v as usize][c]);
                }
            }
        }
    }
    let size = length(sub(hi, lo));
    six_volume / 6.0 > 1e-9 * size * size * size
}

/// The DATA section as it is written, and what has been written once to be shared.
struct Writer {
    data: String,
    next: u64,
    unit: Scale,
    directions: HashMap<[u64; 3], u64>,
    looks: HashMap<[u8; 3], u64>,
    styled: Vec<u64>,
    /// EDGE_CURVEs drawn smooth (soft edges), for the "Soft edges" layer.
    soft: Vec<u64>,
}

/// The layer that holds the soft edges (made invisible), and what it says about them.
const SOFT_LAYER: &str = "Soft edges";
const SOFT_NOTE: &str = "edges drawn smooth: the faces on either side show as one surface";

/// `(#1,#2,#3)`
fn id_list(ids: &[u64]) -> String {
    let mut s = String::with_capacity(ids.len() * 8 + 2);
    s.push('(');
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "#{id}");
    }
    s.push(')');
    s
}

impl Writer {
    fn new(unit: Scale) -> Writer {
        Writer { data: String::new(), next: 1, unit, directions: HashMap::new(), looks: HashMap::new(), styled: Vec::new(), soft: Vec::new() }
    }

    /// Adds an entity; returns its number.
    fn add(&mut self, e: std::fmt::Arguments) -> u64 {
        let id = self.next;
        self.next += 1;
        let _ = write!(self.data, "#{id}=");
        let _ = self.data.write_fmt(e);
        self.data.push_str(";\n");
        id
    }

    fn point(&mut self, p: V3) -> u64 {
        let u = self.unit;
        self.add(format_args!("CARTESIAN_POINT('',({},{},{}))", real(u.number(p[0])), real(u.number(p[1])), real(u.number(p[2]))))
    }

    /// A unit direction, written once however often it is used.
    fn direction(&mut self, d: V3) -> u64 {
        let d = d.map(|c| if c.abs() < 1e-15 { 0.0 } else { c });
        let key = d.map(f64::to_bits);
        if let Some(&id) = self.directions.get(&key) {
            return id;
        }
        let id = self.add(format_args!("DIRECTION('',({},{},{}))", real(d[0]), real(d[1]), real(d[2])));
        self.directions.insert(key, id);
        id
    }

    /// An AXIS2_PLACEMENT_3D at `origin` (metres), its z along `z` and its x along `x`.
    fn placement(&mut self, origin: V3, z: V3, x: V3) -> u64 {
        let o = self.point(origin);
        let z = unit(z).unwrap_or(Z);
        let (zd, xd) = (self.direction(z), self.direction(across(x, z)));
        self.add(format_args!("AXIS2_PLACEMENT_3D('',#{o},#{zd},#{xd})"))
    }

    /// The length unit the context gives, as STEP writes it.
    fn length_unit(&mut self, units: Units) -> u64 {
        match units {
            Units::Millimetres => self.add(format_args!("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))")),
            Units::Centimetres => self.add(format_args!("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.CENTI.,.METRE.))")),
            Units::Metres => self.add(format_args!("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.))")),
            Units::Inches | Units::FeetInches => {
                let mm = self.add(format_args!("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))"));
                let dims = self.add(format_args!("DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.)"));
                let factor = self.add(format_args!("LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#{mm})"));
                self.add(format_args!("(CONVERSION_BASED_UNIT('INCH',#{factor}) LENGTH_UNIT() NAMED_UNIT(#{dims}))"))
            }
        }
    }

    /// The presentation style for a colour, written once per colour.
    fn look(&mut self, rgb: [u8; 3]) -> u64 {
        if let Some(&id) = self.looks.get(&rgb) {
            return id;
        }
        let [r, g, b] = rgb.map(|c| real(c as f64 / 255.0));
        let colour = self.add(format_args!("COLOUR_RGB('',{r},{g},{b})"));
        let fill = self.add(format_args!("FILL_AREA_STYLE_COLOUR('',#{colour})"));
        let area = self.add(format_args!("FILL_AREA_STYLE('',(#{fill}))"));
        let surface = self.add(format_args!("SURFACE_STYLE_FILL_AREA(#{area})"));
        let side = self.add(format_args!("SURFACE_SIDE_STYLE('',(#{surface}))"));
        let usage = self.add(format_args!("SURFACE_STYLE_USAGE(.BOTH.,#{side})"));
        let style = self.add(format_args!("PRESENTATION_STYLE_ASSIGNMENT((#{usage}))"));
        self.looks.insert(rgb, style);
        style
    }

    fn paint(&mut self, item: u64, rgb: [u8; 3]) {
        let style = self.look(rgb);
        let s = self.add(format_args!("STYLED_ITEM('color',(#{style}),#{item})"));
        self.styled.push(s);
    }

    /// A mesh's faces as solids and other shells (for an ADVANCED_BREP_SHAPE_REPRESENTATION and a surface model), and its
    /// loose lines as a GEOMETRIC_CURVE_SET (for a wireframe representation). Its soft edges are kept for the "Soft edges"
    /// layer.
    fn geometry(&mut self, mesh: &Mesh, tally: &mut Tally) -> (Vec<u64>, Vec<u64>, Option<u64>) {
        let (polys, inner) = polys(mesh, tally);
        // soft: the mesh's own soft edges, and those inside a face written as triangles
        let soft: HashSet<(u32, u32)> = mesh.seams.iter().map(|&[a, b]| (a.min(b), a.max(b))).chain(inner).collect();
        let mut points: HashMap<u32, u64> = HashMap::new();
        let (mut solids, mut others) = (Vec::new(), Vec::new());
        for shell in shells(mesh, &polys) {
            let s = self.shell(mesh, &polys, &shell, &mut points, &soft);
            tally.faces += shell.faces.len();
            if shell.solid {
                solids.push(self.add(format_args!("MANIFOLD_SOLID_BREP('',#{s})")));
                tally.solids += 1;
            } else {
                others.push(s);
                tally.shells += 1;
            }
        }
        let wire = self.wireframe(mesh, &mut points, tally);
        (solids, others, wire)
    }

    /// A mesh's loose lines, as POLYLINEs (chained where they meet end to end) in one GEOMETRIC_CURVE_SET; their corners are
    /// the same CARTESIAN_POINTs as the faces' where they meet them.
    fn wireframe(&mut self, mesh: &Mesh, points: &mut HashMap<u32, u64>, tally: &mut Tally) -> Option<u64> {
        let pos = &mesh.positions;
        let n = pos.len();
        let lines: Vec<[u32; 2]> = mesh.lines.iter().copied().filter(|&[a, b]| (a as usize) < n && (b as usize) < n && pos[a as usize] != pos[b as usize]).collect();
        tally.lines_dropped += mesh.lines.len() - lines.len();
        if lines.is_empty() {
            return None;
        }
        let mut curves = Vec::new();
        for chain in chains(&lines) {
            let corners: Vec<u64> = chain.iter().map(|&v| self.cartesian(mesh, v, points)).collect();
            curves.push(self.add(format_args!("POLYLINE('',{})", id_list(&corners))));
        }
        Some(self.add(format_args!("GEOMETRIC_CURVE_SET('',{})", id_list(&curves))))
    }

    fn cartesian(&mut self, mesh: &Mesh, v: u32, points: &mut HashMap<u32, u64>) -> u64 {
        if let Some(&id) = points.get(&v) {
            return id;
        }
        let id = self.point(mesh.positions[v as usize]);
        points.insert(v, id);
        id
    }

    fn vertex(&mut self, mesh: &Mesh, v: u32, points: &mut HashMap<u32, u64>, vertices: &mut HashMap<u32, u64>) -> u64 {
        if let Some(&id) = vertices.get(&v) {
            return id;
        }
        let p = self.cartesian(mesh, v, points);
        let id = self.add(format_args!("VERTEX_POINT('',#{p})"));
        vertices.insert(v, id);
        id
    }

    /// The edge from vertex `a` to `b` (a < b), a LINE between them.
    fn edge(&mut self, mesh: &Mesh, (a, b): (u32, u32), points: &mut HashMap<u32, u64>, vertices: &mut HashMap<u32, u64>) -> u64 {
        let (va, vb) = (self.vertex(mesh, a, points, vertices), self.vertex(mesh, b, points, vertices));
        let d = sub(mesh.positions[b as usize], mesh.positions[a as usize]);
        let l = length(d);
        let dir = self.direction(times(d, 1.0 / l));
        let magnitude = real(self.unit.number(l));
        let vector = self.add(format_args!("VECTOR('',#{dir},{magnitude})"));
        let start = self.cartesian(mesh, a, points);
        let line = self.add(format_args!("LINE('',#{start},#{vector})"));
        self.add(format_args!("EDGE_CURVE('',#{va},#{vb},#{line},.T.)"))
    }

    /// One shell: its faces, each edge and corner written once (a shell's own; its points are shared with the part's other
    /// shells). Its edges in `soft` go to the soft-edges layer.
    fn shell(&mut self, mesh: &Mesh, polys: &[Poly], shell: &Shell, points: &mut HashMap<u32, u64>, soft: &HashSet<(u32, u32)>) -> u64 {
        let mut vertices: HashMap<u32, u64> = HashMap::new();
        let mut edges: HashMap<(u32, u32), u64> = HashMap::new();
        let mut faces = Vec::with_capacity(shell.faces.len());
        for &k in &shell.faces {
            let p = &polys[k];
            let mut bounds = Vec::with_capacity(p.loops.len());
            for (li, l) in p.loops.iter().enumerate() {
                let mut used = Vec::with_capacity(l.len());
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    let key = (a.min(b), a.max(b));
                    let e = match edges.get(&key) {
                        Some(&e) => e,
                        None => {
                            let e = self.edge(mesh, key, points, &mut vertices);
                            edges.insert(key, e);
                            if soft.contains(&key) {
                                self.soft.push(e);
                            }
                            e
                        }
                    };
                    used.push(self.add(format_args!("ORIENTED_EDGE('',*,*,#{e},{})", if a < b { ".T." } else { ".F." })));
                }
                let lp = self.add(format_args!("EDGE_LOOP('',{})", id_list(&used)));
                let kind = if li == 0 { "FACE_OUTER_BOUND" } else { "FACE_BOUND" };
                bounds.push(self.add(format_args!("{kind}('',#{lp},.T.)")));
            }
            // the plane through the first corner, facing out, its x along the first edge
            let (first, second) = (p.loops[0][0], p.loops[0][1]);
            let origin = self.cartesian(mesh, first, points);
            let x = across(sub(mesh.positions[second as usize], mesh.positions[first as usize]), p.normal);
            let (z, x) = (self.direction(p.normal), self.direction(x));
            let axes = self.add(format_args!("AXIS2_PLACEMENT_3D('',#{origin},#{z},#{x})"));
            let plane = self.add(format_args!("PLANE('',#{axes})"));
            let face = self.add(format_args!("ADVANCED_FACE('',{},#{plane},.T.)", id_list(&bounds)));
            if let Some(rgb) = p.rgb {
                self.paint(face, rgb);
            }
            faces.push(face);
        }
        let kind = if shell.closed { "CLOSED_SHELL" } else { "OPEN_SHELL" };
        self.add(format_args!("{kind}('',{})", id_list(&faces)))
    }
}

/// A PRODUCT's id: its name, or what it is when it has none, made unique in the file (a reader may take two products with
/// one id for the same part).
fn product_id(name: &str, kind: Kind, taken: &mut HashSet<String>) -> String {
    let base = if name.trim().is_empty() { kind.word().to_string() } else { name.to_string() };
    let mut id = base.clone();
    let mut n = 1;
    while !taken.insert(id.clone()) {
        n += 1;
        id = format!("{base} ({n})");
    }
    id
}

/// Writes a whole model to STEP: see the module's notes for how, and [`Written::left_out`] for what it cannot hold.
pub fn write(scene: &Scene, opts: &WriteOptions) -> Result<Written, IoError> {
    let finite = |c: &Context| c.mesh.positions.iter().flatten().all(|v| v.is_finite()) && c.instances.iter().all(|i| i.xf.0.iter().flatten().all(|v| v.is_finite()));
    if !std::iter::once(&scene.root).chain(scene.defs.iter().map(|d| &d.ctx)).all(finite) {
        return Err(IoError::Unsupported("the model has a point or a placement that is not a finite number".into()));
    }
    let stem = std::path::Path::new(&opts.name).file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.trim().is_empty());
    let plan = Plan::new(scene, stem.unwrap_or_else(|| "Model".into()));
    let order = plan.order();
    let unit = Scale::of(opts.units);
    let tolerance = plan.parts.iter().map(|p| p.mesh.tolerance()).fold(1e-10, f64::max);

    let mut w = Writer::new(unit);
    let app = w.add(format_args!("APPLICATION_CONTEXT('core data for automotive mechanical design processes')"));
    w.add(format_args!("APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#{app})"));
    let product_ctx = w.add(format_args!("PRODUCT_CONTEXT('',#{app},'mechanical')"));
    let definition_ctx = w.add(format_args!("PRODUCT_DEFINITION_CONTEXT('part definition',#{app},'design')"));
    let length_unit = w.length_unit(opts.units);
    let angle_unit = w.add(format_args!("(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.))"));
    let solid_angle_unit = w.add(format_args!("(NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT())"));
    let uncertainty = w.add(format_args!(
        "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE({}),#{length_unit},'distance_accuracy_value','confusion accuracy')",
        real(unit.number(tolerance))
    ));
    let ctx = w.add(format_args!(
        "(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{uncertainty})) GLOBAL_UNIT_ASSIGNED_CONTEXT((#{length_unit},#{angle_unit},#{solid_angle_unit})) REPRESENTATION_CONTEXT('','3D'))"
    ));

    struct Ids {
        definition: u64,
        shape: u64,
        origin: u64,
        placed: Vec<u64>,
    }
    let mut ids: Vec<Option<Ids>> = (0..plan.parts.len()).map(|_| None).collect();
    let mut tally = Tally::default();
    let mut products = Vec::with_capacity(order.len());
    let mut taken = HashSet::new();
    for &pi in &order {
        let part = &plan.parts[pi];
        let (solids, shells, wire) = w.geometry(&part.mesh, &mut tally);
        // its shape: its own axes, where each part in it is placed, and its solids and surfaces beside them
        let origin = w.placement([0.0; 3], Z, X);
        let placed: Vec<u64> = part.children.iter().map(|(_, xf)| w.placement(xf.origin(), column(xf, 2), column(xf, 0))).collect();
        let items: Vec<u64> = std::iter::once(origin).chain(placed.iter().copied()).collect();
        let shape = w.add(format_args!("SHAPE_REPRESENTATION('',{},#{ctx})", id_list(&items)));
        if !solids.is_empty() {
            let rep = w.add(format_args!("ADVANCED_BREP_SHAPE_REPRESENTATION('',{},#{ctx})", id_list(&solids)));
            w.add(format_args!("SHAPE_REPRESENTATION_RELATIONSHIP('','',#{shape},#{rep})"));
        }
        if !shells.is_empty() {
            let model = w.add(format_args!("SHELL_BASED_SURFACE_MODEL('',{})", id_list(&shells)));
            let rep = w.add(format_args!("MANIFOLD_SURFACE_SHAPE_REPRESENTATION('',(#{model}),#{ctx})"));
            w.add(format_args!("SHAPE_REPRESENTATION_RELATIONSHIP('','',#{shape},#{rep})"));
        }
        // loose lines are wireframe: what other programs show them as
        if let Some(set) = wire {
            let rep = w.add(format_args!("GEOMETRICALLY_BOUNDED_WIREFRAME_SHAPE_REPRESENTATION('',(#{set}),#{ctx})"));
            w.add(format_args!("SHAPE_REPRESENTATION_RELATIONSHIP('','',#{shape},#{rep})"));
        }
        let id = product_id(&part.name, part.kind, &mut taken);
        let product = w.add(format_args!("PRODUCT({},{},{},(#{product_ctx}))", string(&id), string(&part.name), string(&part.kind.description())));
        let formation = w.add(format_args!("PRODUCT_DEFINITION_FORMATION('','',#{product})"));
        let definition = w.add(format_args!("PRODUCT_DEFINITION('design','',#{formation},#{definition_ctx})"));
        let definition_shape = w.add(format_args!("PRODUCT_DEFINITION_SHAPE('','',#{definition})"));
        w.add(format_args!("SHAPE_DEFINITION_REPRESENTATION(#{definition_shape},#{shape})"));
        products.push(product);
        ids[pi] = Some(Ids { definition, shape, origin, placed });
    }
    // every copy: its part, used in the part holding it, its axes taken to where it is placed
    let mut usage = 0;
    for &pi in &order {
        for (k, &(c, _)) in plan.parts[pi].children.iter().enumerate() {
            let (Some(holder), Some(child)) = (&ids[pi], &ids[c]) else { continue };
            let (holder_def, holder_shape, at) = (holder.definition, holder.shape, holder.placed[k]);
            let (child_def, child_shape, child_origin) = (child.definition, child.shape, child.origin);
            usage += 1;
            let nauo = w.add(format_args!("NEXT_ASSEMBLY_USAGE_OCCURRENCE('{usage}','','',#{holder_def},#{child_def},$)"));
            let used_shape = w.add(format_args!("PRODUCT_DEFINITION_SHAPE('','',#{nauo})"));
            let transform = w.add(format_args!("ITEM_DEFINED_TRANSFORMATION('','',#{child_origin},#{at})"));
            let relation = w.add(format_args!(
                "(REPRESENTATION_RELATIONSHIP('','',#{child_shape},#{holder_shape}) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{transform}) SHAPE_REPRESENTATION_RELATIONSHIP())"
            ));
            w.add(format_args!("CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{relation},#{used_shape})"));
        }
    }
    if !w.styled.is_empty() {
        let styled = std::mem::take(&mut w.styled);
        w.add(format_args!("MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',{},#{ctx})", id_list(&styled)));
    }
    // soft edges: a layer of the edges drawn smooth, made invisible (edges only, never faces)
    if !w.soft.is_empty() {
        let soft = std::mem::take(&mut w.soft);
        let layer = w.add(format_args!("PRESENTATION_LAYER_ASSIGNMENT({},{},{})", string(SOFT_LAYER), string(SOFT_NOTE), id_list(&soft)));
        w.add(format_args!("INVISIBILITY((#{layer}))"));
    }
    w.add(format_args!("PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,{})", id_list(&products)));

    let mut text = String::with_capacity(w.data.len() + 512);
    text.push_str("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('Daedalus CAD model'),'2;1');\n");
    // Daedalus's own version: the VERSION file at the top of the repository, read when this is built
    let program = format!("Daedalus CAD {}", include_str!("../../../VERSION").trim_ascii());
    let _ = writeln!(text, "FILE_NAME({},{},(''),(''),{},'Daedalus CAD','');", string(&opts.name), string(&now()), string(&program));
    let _ = writeln!(text, "FILE_SCHEMA(('{SCHEMA}'));");
    text.push_str("ENDSEC;\nDATA;\n");
    text.push_str(&w.data);
    text.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    let left_out = left_out(scene, &plan, &tally);
    Ok(Written { text, products: products.len(), solids: tally.solids, shells: tally.shells, faces: tally.faces, left_out })
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// What a STEP file of the scene does not hold, one line each.
fn left_out(scene: &Scene, plan: &Plan, tally: &Tally) -> Vec<String> {
    let mut out = Vec::new();
    let n = &scene.notes;
    for (k, one, many) in [(n.guides.len(), "guide", "guides"), (n.dims.len(), "dimension", "dimensions"), (n.labels.len(), "text note", "text notes"), (n.sections.len(), "section plane", "section planes")] {
        if k > 0 {
            out.push(count(k, one, many));
        }
    }
    if !n.axes.is_default() {
        out.push("the drawing axes".into());
    }
    if !scene.textures.is_empty() {
        out.push(format!("{} (the faces keep their colours)", count(scene.textures.len(), "texture", "textures")));
    }
    if plan.dynamic > 0 {
        out.push(format!("Dynamic Component attributes ({} written as they are drawn now)", count(plan.dynamic, "copy", "copies")));
    }
    let parts = scene.defs.iter().enumerate().filter(|(i, d)| d.part.is_some() && plan.reached[*i]).count();
    if parts > 0 {
        out.push(format!("{} (as their faces; File > Export STEP for Fabrication writes them exact)", count(parts, "sheet-metal part", "sheet-metal parts")));
    }
    let unplaced = plan.reached.iter().filter(|r| !**r).count();
    if unplaced > 0 {
        out.push(count(unplaced, "group or component placed nowhere", "groups or components placed nowhere"));
    }
    if plan.reshaped > 0 {
        out.push(format!("{} (each written as a part of its own)", count(plan.reshaped, "scaled or mirrored copy", "scaled or mirrored copies")));
    }
    if plan.broken > 0 {
        out.push(count(plan.broken, "copy of a missing or self-holding definition", "copies of missing or self-holding definitions"));
    }
    if tally.split > 0 {
        out.push(format!("{} (written as triangles)", count(tally.split, "face not flat", "faces not flat")));
    }
    if tally.degenerate > 0 {
        out.push(count(tally.degenerate, "face with no area", "faces with no area"));
    }
    if tally.holes_dropped > 0 {
        out.push(count(tally.holes_dropped, "hole with no area", "holes with no area"));
    }
    if tally.lines_dropped > 0 {
        out.push(count(tally.lines_dropped, "loose line with no length", "loose lines with no length"));
    }
    out
}

/// A real as Part 21 writes one: always with a full stop (`1.`, `-0.25`, `1.5E-12`), in the fewest digits that read back as
/// the same number.
fn real(x: f64) -> String {
    let x = if x == 0.0 { 0.0 } else { x };
    let a = x.abs();
    if a != 0.0 && !(1e-5..1e15).contains(&a) {
        let s = format!("{x:E}");
        return match s.split_once('E') {
            Some((m, e)) if !m.contains('.') => format!("{m}.E{e}"),
            _ => s,
        };
    }
    let s = x.to_string();
    if s.contains('.') {
        s
    } else {
        s + "."
    }
}

/// Text as a Part 21 string: in quotes, a quote or a backslash doubled, and anything outside printable ASCII as `\X2\...\X0\`
/// (four hex digits a character) or, beyond the 16-bit range, `\X4\...\X0\` (eight).
fn string(t: &str) -> String {
    let mut s = String::with_capacity(t.len() + 2);
    s.push('\'');
    // inside a \X2\ run (false) or a \X4\ one (true)
    let mut run: Option<bool> = None;
    for c in t.chars() {
        if (' '..='~').contains(&c) {
            if run.take().is_some() {
                s.push_str("\\X0\\");
            }
            match c {
                '\'' => s.push_str("''"),
                '\\' => s.push_str("\\\\"),
                c => s.push(c),
            }
        } else {
            let wide = c as u32 > 0xFFFF;
            if run != Some(wide) {
                if run.is_some() {
                    s.push_str("\\X0\\");
                }
                s.push_str(if wide { "\\X4\\" } else { "\\X2\\" });
                run = Some(wide);
            }
            let _ = if wide { write!(s, "{:08X}", c as u32) } else { write!(s, "{:04X}", c as u32) };
        }
    }
    if run.is_some() {
        s.push_str("\\X0\\");
    }
    s.push('\'');
    s
}

/// Now, as ISO 8601 says it, in UTC: `2026-11-08T14:03:12+00:00`.
fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, m, d) = civil((secs / 86_400) as i64);
    let s = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00", s / 3600, s % 3600 / 60, s % 60)
}

/// The calendar date `days` after 1970-01-01.
fn civil(days: i64) -> (i64, u32, u32) {
    // counted in 400-year cycles of 146 097 days from 1 March of year 0, so each leap day comes at the end of its year
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_from_march + 2) / 5 + 1) as u32;
    let month = if month_from_march < 10 { month_from_march + 3 } else { month_from_march - 9 } as u32;
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

// ---------------------------------------------------------------------------------------------------------------------
// Part 21 reading: the file as entities and their parameters

/// A parameter of an entity, as Part 21 writes it.
#[derive(Debug, Clone, PartialEq)]
enum P {
    /// `$`, and `*` (a value the schema works out)
    Null,
    Derived,
    Int(i64),
    Real(f64),
    Str(String),
    /// `.T.` as "T"
    Enum(String),
    Ref(u64),
    List(Vec<P>),
    /// `LENGTH_MEASURE(1.E-07)`
    Typed(String, Box<P>),
    /// binary and constants, not used here
    Other,
}

/// An entity: one record (`#1=PLANE(...)`) or several (`#2=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(...))`), by type number.
enum Ent {
    One(u32, Vec<P>),
    Many(Vec<(u32, Vec<P>)>),
}

#[derive(Default)]
struct File {
    names: Vec<String>,
    lookup: HashMap<Box<[u8]>, u32>,
    ents: Vec<(u64, Ent)>,
    index: HashMap<u64, u32>,
}

impl File {
    fn name(&self, n: u32) -> &str {
        &self.names[n as usize]
    }

    fn intern(&mut self, w: &[u8]) -> u32 {
        if let Some(&n) = self.lookup.get(w) {
            return n;
        }
        let n = self.names.len() as u32;
        self.names.push(String::from_utf8_lossy(w).to_ascii_uppercase());
        self.lookup.insert(w.into(), n);
        n
    }

    fn get(&self, id: u64) -> Option<&Ent> {
        self.index.get(&id).map(|&i| &self.ents[i as usize].1)
    }

    /// A one-record entity's type and parameters.
    fn simple(&self, id: u64) -> Option<(&str, &[P])> {
        match self.get(id)? {
            Ent::One(n, p) => Some((self.name(*n), p)),
            Ent::Many(_) => None,
        }
    }

    /// The parameters of the record of that type: the entity itself, or that part of a complex one.
    fn rec(&self, id: u64, name: &str) -> Option<&[P]> {
        self.part(self.get(id)?, name)
    }

    fn part<'a>(&'a self, e: &'a Ent, name: &str) -> Option<&'a [P]> {
        match e {
            Ent::One(n, p) => (self.name(*n) == name).then_some(p.as_slice()),
            Ent::Many(parts) => parts.iter().find(|(n, _)| self.name(*n) == name).map(|(_, p)| p.as_slice()),
        }
    }

    /// An entity's type for a message; of a complex one, its most telling part (RATIONAL_B_SPLINE_SURFACE, not SURFACE).
    fn type_name(&self, id: u64) -> String {
        const PLAIN: [&str; 9] = ["BOUNDED_CURVE", "BOUNDED_SURFACE", "CURVE", "SURFACE", "GEOMETRIC_REPRESENTATION_ITEM", "REPRESENTATION_ITEM", "TOPOLOGICAL_REPRESENTATION_ITEM", "B_SPLINE_CURVE", "B_SPLINE_SURFACE"];
        match self.get(id) {
            None => format!("#{id} (not in the file)"),
            Some(Ent::One(n, _)) => self.name(*n).to_string(),
            Some(Ent::Many(parts)) => {
                let names: Vec<&str> = parts.iter().map(|(n, _)| self.name(*n)).collect();
                let telling = names.iter().find(|n| n.starts_with("RATIONAL_")).or_else(|| names.iter().find(|n| n.ends_with("_WITH_KNOTS"))).or_else(|| names.iter().find(|n| !PLAIN.contains(n)));
                telling.or(names.first()).map(|s| s.to_string()).unwrap_or_default()
            }
        }
    }

    fn instance(&mut self, lx: &mut Lexer) -> Result<(), String> {
        lx.i += 1; // the '#'
        let id = lx.digits()?;
        lx.expect(b'=')?;
        let ent = if lx.eat(b'(') {
            let mut parts = Vec::new();
            while !lx.eat(b')') {
                let name = lx.word();
                if name.is_empty() {
                    return Err(lx.error(&format!("#{id}: a record name expected")));
                }
                let n = self.intern(name);
                parts.push((n, lx.list(0)?));
            }
            Ent::Many(parts)
        } else {
            let name = lx.word();
            if name.is_empty() {
                return Err(lx.error(&format!("#{id} has no entity name")));
            }
            let n = self.intern(name);
            Ent::One(n, lx.list(0)?)
        };
        lx.expect(b';')?;
        if self.index.insert(id, self.ents.len() as u32).is_some() {
            return Err(lx.error(&format!("#{id} is defined twice")));
        }
        self.ents.push((id, ent));
        Ok(())
    }
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

impl<'a> Lexer<'a> {
    /// Past white space and comments.
    fn skip(&mut self) {
        loop {
            while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
                self.i += 1;
            }
            if !self.s[self.i.min(self.s.len())..].starts_with(b"/*") {
                return;
            }
            self.i = match find(&self.s[self.i + 2..], b"*/") {
                Some(k) => self.i + 2 + k + 2,
                None => self.s.len(),
            };
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.error(&format!("'{}' expected", c as char)))
        }
    }

    /// A keyword: an entity or section name (`ISO-10303-21` too).
    fn word(&mut self) -> &'a [u8] {
        self.skip();
        let start = self.i;
        while self.s.get(self.i).is_some_and(|&c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'!')) {
            self.i += 1;
        }
        &self.s[start..self.i]
    }

    fn error(&self, what: &str) -> String {
        let line = 1 + self.s[..self.i.min(self.s.len())].iter().filter(|&&b| b == b'\n').count();
        format!("line {line}: {what}")
    }

    fn digits(&mut self) -> Result<u64, String> {
        let start = self.i;
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok().and_then(|t| t.parse().ok()).ok_or_else(|| self.error("an entity number expected"))
    }

    fn list(&mut self, depth: usize) -> Result<Vec<P>, String> {
        self.expect(b'(')?;
        let mut v = Vec::new();
        if self.eat(b')') {
            return Ok(v);
        }
        loop {
            v.push(self.param(depth)?);
            if self.eat(b',') {
                continue;
            }
            self.expect(b')')?;
            return Ok(v);
        }
    }

    fn param(&mut self, depth: usize) -> Result<P, String> {
        if depth > 64 {
            return Err(self.error("lists nested too deep"));
        }
        match self.peek() {
            Some(b'$') => {
                self.i += 1;
                Ok(P::Null)
            }
            Some(b'*') => {
                self.i += 1;
                Ok(P::Derived)
            }
            Some(b'#') => {
                self.i += 1;
                self.digits().map(P::Ref)
            }
            Some(b'\'') => self.string().map(P::Str),
            Some(b'"') => {
                self.i += 1;
                while self.s.get(self.i).is_some_and(|&c| c != b'"') {
                    self.i += 1;
                }
                self.i += 1;
                Ok(P::Other)
            }
            Some(b'(') => self.list(depth + 1).map(P::List),
            Some(b'.') if self.s.get(self.i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') => {
                let start = self.i + 1;
                let Some(k) = self.s[start..].iter().position(|&c| c == b'.') else { return Err(self.error("an enumeration is not closed")) };
                self.i = start + k + 1;
                Ok(P::Enum(String::from_utf8_lossy(&self.s[start..start + k]).to_ascii_uppercase()))
            }
            Some(b'@') => {
                self.i += 1;
                self.word();
                Ok(P::Other)
            }
            Some(c) if c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.') => self.number(),
            Some(c) if c.is_ascii_alphabetic() || c == b'!' => {
                let name = String::from_utf8_lossy(self.word()).to_ascii_uppercase();
                let mut inner = self.list(depth + 1)?;
                let value = if inner.len() == 1 { inner.pop().unwrap_or(P::Null) } else { P::List(inner) };
                Ok(P::Typed(name, Box::new(value)))
            }
            _ => Err(self.error("a parameter expected")),
        }
    }

    fn number(&mut self) -> Result<P, String> {
        let start = self.i;
        if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
            self.i += 1;
        }
        let mut real = false;
        while let Some(&c) = self.s.get(self.i) {
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == b'.' {
                real = true;
                self.i += 1;
            } else if c == b'E' || c == b'e' {
                real = true;
                self.i += 1;
                if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        let t = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
        let value = if real { t.parse::<f64>().ok().map(P::Real) } else { t.parse::<i64>().ok().map(P::Int).or_else(|| t.parse::<f64>().ok().map(P::Real)) };
        value.ok_or_else(|| self.error(&format!("{t:?} is not a number")))
    }

    /// A string, its quotes doubled and its `\X2\`, `\X4\`, `\X\`, `\S\` and `\\` directives decoded; line breaks inside it
    /// are not part of it. Bytes outside ASCII are taken as UTF-8, else as ISO 8859-1.
    fn string(&mut self) -> Result<String, String> {
        fn flush(out: &mut String, raw: &mut Vec<u8>) {
            match std::str::from_utf8(raw) {
                Ok(s) => out.push_str(s),
                Err(_) => out.extend(raw.iter().map(|&b| b as char)),
            }
            raw.clear();
        }
        fn hex(s: &[u8]) -> Option<u32> {
            u32::from_str_radix(std::str::from_utf8(s).ok()?, 16).ok()
        }
        self.i += 1; // the opening quote
        let mut out = String::new();
        let mut raw: Vec<u8> = Vec::new();
        loop {
            let Some(&b) = self.s.get(self.i) else { return Err(self.error("a text in quotes is not closed")) };
            self.i += 1;
            match b {
                b'\'' if self.s.get(self.i) == Some(&b'\'') => {
                    raw.push(b'\'');
                    self.i += 1;
                }
                b'\'' => {
                    flush(&mut out, &mut raw);
                    return Ok(out);
                }
                b'\r' | b'\n' => {}
                b'\\' => {
                    flush(&mut out, &mut raw);
                    let rest = &self.s[self.i..];
                    if rest.starts_with(b"\\") {
                        out.push('\\');
                        self.i += 1;
                    } else if rest.starts_with(b"X2\\") || rest.starts_with(b"X4\\") {
                        let width = if rest[1] == b'2' { 4 } else { 8 };
                        let Some(end) = find(&rest[3..], b"\\X0\\") else { return Err(self.error("a \\X2\\ or \\X4\\ run is not closed")) };
                        let digits: Vec<u8> = rest[3..3 + end].iter().copied().filter(|c| !c.is_ascii_whitespace()).collect();
                        let codes: Vec<u32> = digits.chunks(width).filter_map(hex).collect();
                        if width == 4 {
                            out.extend(char::decode_utf16(codes.iter().map(|&c| c as u16)).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)));
                        } else {
                            out.extend(codes.iter().map(|&c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER)));
                        }
                        self.i += 3 + end + 4;
                    } else if rest.starts_with(b"X\\") && rest.len() >= 4 {
                        out.push(hex(&rest[2..4]).and_then(char::from_u32).unwrap_or(char::REPLACEMENT_CHARACTER));
                        self.i += 4;
                    } else if rest.starts_with(b"S\\") && rest.len() >= 3 {
                        out.push(char::from_u32(rest[2] as u32 + 128).unwrap_or(char::REPLACEMENT_CHARACTER));
                        self.i += 3;
                    } else if rest.len() >= 3 && rest[0] == b'P' && rest[2] == b'\\' {
                        self.i += 3; // a code page for \S\ (ISO 8859-1 taken)
                    } else {
                        out.push('\\');
                    }
                }
                _ => raw.push(b),
            }
        }
    }
}

/// Reads a Part 21 exchange structure.
fn parse(bytes: &[u8]) -> Result<File, String> {
    let mut lx = Lexer { s: bytes, i: 0 };
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        lx.i = 3;
    }
    if lx.word() != b"ISO-10303-21" {
        return Err("not a STEP file (it does not start with ISO-10303-21)".into());
    }
    lx.expect(b';')?;
    let mut file = File::default();
    loop {
        match lx.word() {
            b"HEADER" => {
                lx.expect(b';')?;
                loop {
                    let name = lx.word();
                    if name == b"ENDSEC" {
                        lx.expect(b';')?;
                        break;
                    }
                    if name.is_empty() {
                        return Err(lx.error("a header entry expected"));
                    }
                    // FILE_DESCRIPTION, FILE_NAME, FILE_SCHEMA: what is in the file says more than they do
                    lx.list(0)?;
                    lx.expect(b';')?;
                }
            }
            b"DATA" => {
                if lx.peek() == Some(b'(') {
                    lx.list(0)?;
                }
                lx.expect(b';')?;
                loop {
                    if lx.peek() == Some(b'#') {
                        file.instance(&mut lx)?;
                        continue;
                    }
                    let w = lx.word();
                    if w == b"ENDSEC" {
                        lx.expect(b';')?;
                        break;
                    }
                    if w.is_empty() && lx.peek().is_none() {
                        return Err(lx.error("the DATA section has no ENDSEC"));
                    }
                    return Err(lx.error(&format!("{:?} where an entity was expected", String::from_utf8_lossy(w))));
                }
            }
            b"END-ISO-10303-21" => break,
            // Part 21's third edition: sections of no use here
            b"ANCHOR" | b"REFERENCE" | b"SIGNATURE" => match find(&bytes[lx.i..], b"ENDSEC") {
                Some(k) => {
                    lx.i += k + 6;
                    lx.expect(b';')?;
                }
                None => break,
            },
            // no END-ISO-10303-21: the end of the file is taken for it
            b"" if lx.peek().is_none() => break,
            other => return Err(lx.error(&format!("{:?} is not a section", String::from_utf8_lossy(other)))),
        }
    }
    Ok(file)
}

fn at(p: &[P], i: usize) -> Option<u64> {
    match p.get(i) {
        Some(P::Ref(id)) => Some(*id),
        _ => None,
    }
}

fn list(p: &[P], i: usize) -> &[P] {
    match p.get(i) {
        Some(P::List(v)) => v,
        _ => &[],
    }
}

fn ids(p: &[P], i: usize) -> impl Iterator<Item = u64> + '_ {
    list(p, i).iter().filter_map(|x| if let P::Ref(id) = x { Some(*id) } else { None })
}

fn num(p: Option<&P>) -> Option<f64> {
    match p? {
        P::Real(x) => Some(*x),
        P::Int(i) => Some(*i as f64),
        P::Typed(_, inner) => num(Some(inner)),
        _ => None,
    }
}

fn flag(p: &[P], i: usize) -> Option<bool> {
    match p.get(i) {
        Some(P::Enum(e)) if e == "T" => Some(true),
        Some(P::Enum(e)) if e == "F" => Some(false),
        _ => None,
    }
}

fn enumeration(p: &[P], i: usize) -> Option<&str> {
    match p.get(i) {
        Some(P::Enum(e)) => Some(e),
        _ => None,
    }
}

fn text(p: &[P], i: usize) -> &str {
    match p.get(i) {
        Some(P::Str(s)) => s,
        _ => "",
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading: products, shapes and colours

/// A STEP file read, and what could not be.
#[derive(Debug, Clone, Default)]
pub struct Read {
    pub scene: Scene,
    /// The file's length unit, when it is one Daedalus shows (millimetres, centimetres, metres, inches; feet as feet and inches).
    pub units: Option<Units>,
    /// What is in the file but was not read, with how many ("24 faces on CYLINDRICAL_SURFACE"); empty when all was.
    pub skipped: Vec<String>,
    /// What was put right or passed over on the way ("2 faces with no area").
    pub notes: Vec<String>,
}

/// Reads a STEP file whole, as Daedalus opens it: refuses one with anything it cannot read (a curved face, say), naming what,
/// rather than open the model with parts of it missing. [`read`] takes what it can and lists the rest.
pub fn read_scene(bytes: &[u8]) -> Result<Scene, IoError> {
    let r = read(bytes)?;
    if !r.skipped.is_empty() {
        return Err(IoError::Unsupported(format!("this STEP file has what Daedalus cannot read yet: {} (only flat faces so far)", r.skipped.join(", "))));
    }
    Ok(r.scene)
}

/// Reads a STEP file: products as groups and components (the top one as the model itself), their faces, placements, colours
/// and units. Faces on curved surfaces and what else is not read are listed in [`Read::skipped`].
pub fn read(bytes: &[u8]) -> Result<Read, IoError> {
    let file = parse(bytes).map_err(IoError::Parse)?;
    let index = Index::of(&file);
    let mut r = Reader::new(&file, &index);
    let mut scene = Scene::default();
    if index.pds.is_empty() {
        // no products: every shape representation's geometry, loose at the top
        let reps: Vec<u64> = file
            .ents
            .iter()
            .filter(|(id, _)| !index.mapped.contains(id))
            .filter(|(_, e)| matches!(e, Ent::One(n, _) if file.name(*n).ends_with("SHAPE_REPRESENTATION") && file.name(*n) != "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION"))
            .map(|(id, _)| *id)
            .collect();
        let mut b = Builder::default();
        for rep in reps {
            r.rep_items(rep, &mut b, 0);
        }
        scene.root = Context { mesh: b.mesh, instances: b.instances };
    } else {
        let children: HashSet<u64> = index.usages.iter().map(|u| u.2).collect();
        let roots: Vec<u64> = index.pds.iter().copied().filter(|pd| !children.contains(pd)).collect();
        // one product at the top is the model; several are each placed in it as they are
        let top = if roots.len() == 1 { Some(roots[0]) } else { None };
        let mut def_of: HashMap<u64, usize> = HashMap::new();
        for &pd in &index.pds {
            if Some(pd) != top {
                def_of.insert(pd, r.defs.len());
                let d = r.product(pd);
                r.defs.push(d);
            }
        }
        for &pd in &index.pds {
            let ctx = r.product_context(pd, &def_of);
            match def_of.get(&pd) {
                Some(&d) => r.defs[d].ctx = ctx,
                None => scene.root = ctx,
            }
        }
        if top.is_none() {
            for pd in roots {
                scene.root.instances.push(Instance { def: def_of[&pd], xf: Xform::IDENTITY });
            }
        }
    }
    scene.defs = std::mem::take(&mut r.defs);
    if let Some(d) = holds_itself(&scene) {
        return Err(IoError::Parse(format!("the assembly is a loop: {:?} holds itself", scene.defs[d].name)));
    }
    let lines = |m: &BTreeMap<String, usize>| m.iter().map(|(what, &n)| counted(n, what)).collect::<Vec<_>>();
    Ok(Read { units: r.first_scale.and_then(Scale::units), skipped: lines(&r.skipped), notes: lines(&r.notes), scene })
}

/// "2 faces on CYLINDRICAL_SURFACE", "1 face on ...": `what` starts with the plural of what is counted.
fn counted(n: usize, what: &str) -> String {
    if n != 1 {
        return format!("{n} {what}");
    }
    let (word, rest) = what.split_once(' ').unwrap_or((what, ""));
    let one = word.strip_suffix('s').unwrap_or(word);
    if rest.is_empty() {
        format!("1 {one}")
    } else {
        format!("1 {one} {rest}")
    }
}

/// A definition that holds itself, through the copies placed in it (only in a damaged file): one walk down from each
/// definition not yet walked, a definition met again while its own walk is still open being in a loop.
fn holds_itself(scene: &Scene) -> Option<usize> {
    let count = scene.defs.len();
    // 0: not walked yet, 1: its walk is open, 2: done
    let mut state = vec![0u8; count];
    for start in 0..count {
        if state[start] != 0 {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        state[start] = 1;
        while let Some(&mut (d, ref mut next)) = stack.last_mut() {
            match scene.defs[d].ctx.instances.get(*next) {
                Some(i) => {
                    *next += 1;
                    match state.get(i.def) {
                        Some(1) => return Some(i.def),
                        Some(0) => {
                            state[i.def] = 1;
                            stack.push((i.def, 0));
                        }
                        _ => {}
                    }
                }
                None => {
                    state[d] = 2;
                    stack.pop();
                }
            }
        }
    }
    None
}

/// Who refers to whom, gathered in one pass over the file.
#[derive(Default)]
struct Index {
    /// Product definitions, in the file's order.
    pds: Vec<u64>,
    /// A product definition's or a usage's PRODUCT_DEFINITION_SHAPEs.
    shapes_of: HashMap<u64, Vec<u64>>,
    /// A PRODUCT_DEFINITION_SHAPE's representations (SHAPE_DEFINITION_REPRESENTATION).
    reps_of: HashMap<u64, Vec<u64>>,
    /// Representations one shape is made of (SHAPE_REPRESENTATION_RELATIONSHIPs with no transformation), both ways.
    linked: HashMap<u64, Vec<u64>>,
    /// Assembly usages: (the usage, the product holding, the product held), and by the product holding: (usage, held).
    usages: Vec<(u64, u64, u64)>,
    used_in: HashMap<u64, Vec<(u64, u64)>>,
    /// A usage's PRODUCT_DEFINITION_SHAPE -> the relationship placing it (CONTEXT_DEPENDENT_SHAPE_REPRESENTATION).
    placed_by: HashMap<u64, u64>,
    /// Styled items, and whether each over-rides another.
    styled: Vec<(u64, bool)>,
    /// Representations used by MAPPED_ITEMs (read through them, not on their own).
    mapped: HashSet<u64>,
    /// What is drawn smooth or not at all: the items of the "Soft edges" layer and of any layer made invisible (of them,
    /// edges become soft edges).
    soft: HashSet<u64>,
}

impl Index {
    fn of(file: &File) -> Index {
        let mut x = Index::default();
        let mut layers: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut hidden: Vec<u64> = Vec::new();
        let link = |x: &mut Index, a: Option<u64>, b: Option<u64>| {
            if let (Some(a), Some(b)) = (a, b) {
                x.linked.entry(a).or_default().push(b);
                x.linked.entry(b).or_default().push(a);
            }
        };
        for (id, e) in &file.ents {
            let id = *id;
            match e {
                Ent::One(n, p) => match file.name(*n) {
                    "PRODUCT_DEFINITION" | "PRODUCT_DEFINITION_WITH_ASSOCIATED_DOCUMENTS" => x.pds.push(id),
                    "PRODUCT_DEFINITION_SHAPE" | "PROPERTY_DEFINITION" => {
                        if let Some(d) = at(p, 2) {
                            x.shapes_of.entry(d).or_default().push(id);
                        }
                    }
                    "SHAPE_DEFINITION_REPRESENTATION" => {
                        if let (Some(d), Some(r)) = (at(p, 0), at(p, 1)) {
                            x.reps_of.entry(d).or_default().push(r);
                        }
                    }
                    "SHAPE_REPRESENTATION_RELATIONSHIP" => link(&mut x, at(p, 2), at(p, 3)),
                    "NEXT_ASSEMBLY_USAGE_OCCURRENCE" | "ASSEMBLY_COMPONENT_USAGE" => {
                        if let (Some(a), Some(b)) = (at(p, 3), at(p, 4)) {
                            x.usages.push((id, a, b));
                            x.used_in.entry(a).or_default().push((id, b));
                        }
                    }
                    "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION" => {
                        if let (Some(rel), Some(shape)) = (at(p, 0), at(p, 1)) {
                            x.placed_by.insert(shape, rel);
                        }
                    }
                    "STYLED_ITEM" => x.styled.push((id, false)),
                    "OVER_RIDING_STYLED_ITEM" => x.styled.push((id, true)),
                    "REPRESENTATION_MAP" => {
                        if let Some(r) = at(p, 1) {
                            x.mapped.insert(r);
                        }
                    }
                    "PRESENTATION_LAYER_ASSIGNMENT" => {
                        layers.insert(id, ids(p, 2).collect::<Vec<u64>>());
                        if text(p, 0).trim().eq_ignore_ascii_case(SOFT_LAYER) {
                            x.soft.extend(ids(p, 2));
                        }
                    }
                    "INVISIBILITY" => hidden.extend(ids(p, 0)),
                    _ => {}
                },
                Ent::Many(_) => {
                    if file.part(e, "PRODUCT_DEFINITION").is_some() {
                        x.pds.push(id);
                    }
                    // a shape made of others; with a transformation it is an assembly placement instead
                    if file.part(e, "SHAPE_REPRESENTATION_RELATIONSHIP").is_some() && file.part(e, "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION").is_none() {
                        if let Some(p) = file.part(e, "REPRESENTATION_RELATIONSHIP") {
                            link(&mut x, at(p, 2), at(p, 3));
                        }
                    }
                }
            }
        }
        // a layer made invisible: its edges are not drawn either
        for l in hidden {
            if let Some(items) = layers.get(&l) {
                x.soft.extend(items.iter().copied());
            }
        }
        x
    }
}

/// An edge read: its first and last vertex and those along it between; or what kind of curve could not be read (None: an
/// edge with nothing to read).
type EdgeRead = Result<(u32, u32, Vec<u32>), Option<String>>;

/// One context's geometry as it is read.
#[derive(Default)]
struct Builder {
    mesh: Mesh,
    instances: Vec<Instance>,
    /// CARTESIAN_POINT -> vertex
    points: HashMap<u64, u32>,
    edges: HashMap<u64, EdgeRead>,
    /// Soft edges already marked (two faces share each).
    soft_done: HashSet<u64>,
}

struct Reader<'f> {
    f: &'f File,
    index: &'f Index,
    /// Styled items' colours, by what they style (a face, a shell, a solid).
    colour: HashMap<u64, [u8; 3]>,
    /// Representation context -> its length unit.
    scales: HashMap<u64, Scale>,
    first_scale: Option<Scale>,
    reps: HashMap<u64, Vec<u64>>,
    defs: Vec<Definition>,
    /// Mapped representations' definitions.
    of_rep: HashMap<u64, usize>,
    building: Vec<u64>,
    skipped: BTreeMap<String, usize>,
    notes: BTreeMap<String, usize>,
}

/// A transform from axes: x, y and z its columns, `o` where its origin lands.
fn frame(x: V3, y: V3, z: V3, o: V3) -> Xform {
    Xform([[x[0], y[0], z[0], o[0]], [x[1], y[1], z[1], o[1]], [x[2], y[2], z[2], o[2]]])
}

impl<'f> Reader<'f> {
    fn new(f: &'f File, index: &'f Index) -> Reader<'f> {
        let mut r = Reader {
            f,
            index,
            colour: HashMap::new(),
            scales: HashMap::new(),
            first_scale: None,
            reps: HashMap::new(),
            defs: Vec::new(),
            of_rep: HashMap::new(),
            building: Vec::new(),
            skipped: BTreeMap::new(),
            notes: BTreeMap::new(),
        };
        // plain styles first, so an over-riding one wins
        let mut styled = index.styled.clone();
        styled.sort_by_key(|&(_, over)| over);
        for (s, _) in styled {
            if let Some((_, p)) = f.simple(s) {
                if let (Some(item), Some(c)) = (at(p, 2), r.style_colour(p)) {
                    r.colour.insert(item, c);
                }
            }
        }
        r
    }

    fn skip(&mut self, what: String) {
        *self.skipped.entry(what).or_default() += 1;
    }

    fn note(&mut self, what: &str) {
        *self.notes.entry(what.to_string()).or_default() += 1;
    }

    /// A STYLED_ITEM's surface colour: through its presentation styles to a COLOUR_RGB or a pre-defined colour.
    fn style_colour(&self, p: &[P]) -> Option<[u8; 3]> {
        let f = self.f;
        for psa in ids(p, 1) {
            // PRESENTATION_STYLE_ASSIGNMENT(styles), or PRESENTATION_STYLE_BY_CONTEXT(styles, context)
            let Some((_, q)) = f.simple(psa) else { continue };
            for style in ids(q, 0) {
                let side = match f.simple(style) {
                    Some(("SURFACE_STYLE_USAGE", s)) => at(s, 1),
                    Some(("SURFACE_SIDE_STYLE", _)) => Some(style),
                    _ => None,
                };
                let Some(("SURFACE_SIDE_STYLE", s)) = side.and_then(|id| f.simple(id)) else { continue };
                for element in ids(s, 1) {
                    let colour = match f.simple(element) {
                        Some(("SURFACE_STYLE_FILL_AREA", e)) => at(e, 0)
                            .and_then(|a| f.simple(a))
                            .and_then(|(_, area)| ids(area, 1).find_map(|fill| f.rec(fill, "FILL_AREA_STYLE_COLOUR").and_then(|c| at(c, 1)))),
                        Some(("SURFACE_STYLE_RENDERING" | "SURFACE_STYLE_RENDERING_WITH_PROPERTIES", e)) => at(e, 1),
                        _ => None,
                    };
                    if let Some(c) = colour.and_then(|c| self.rgb(c)) {
                        return Some(c);
                    }
                }
            }
        }
        None
    }

    fn rgb(&self, c: u64) -> Option<[u8; 3]> {
        let to = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if let Some(p) = self.f.rec(c, "COLOUR_RGB") {
            return Some([to(num(p.get(1))?), to(num(p.get(2))?), to(num(p.get(3))?)]);
        }
        let p = self.f.rec(c, "DRAUGHTING_PRE_DEFINED_COLOUR")?;
        Some(match text(p, 0) {
            "red" => [255, 0, 0],
            "green" => [0, 255, 0],
            "blue" => [0, 0, 255],
            "yellow" => [255, 255, 0],
            "magenta" => [255, 0, 255],
            "cyan" => [0, 255, 255],
            "black" => [0, 0, 0],
            "white" => [255, 255, 255],
            _ => return None,
        })
    }

    /// The length unit of a representation, from its context (millimetres when the file gives none).
    fn rep_scale(&mut self, rep: u64) -> Scale {
        let ctx = self.rep_params(rep).and_then(|p| at(p, 2));
        let Some(ctx) = ctx else { return Scale::MM };
        if let Some(&s) = self.scales.get(&ctx) {
            return s;
        }
        let f = self.f;
        let found = f.rec(ctx, "GLOBAL_UNIT_ASSIGNED_CONTEXT").and_then(|p| ids(p, 0).find_map(|u| self.length_unit(u, 0)));
        let s = found.unwrap_or_else(|| {
            self.note("representations with no length unit (millimetres taken)");
            Scale::MM
        });
        self.first_scale.get_or_insert(s);
        self.scales.insert(ctx, s);
        s
    }

    fn length_unit(&self, u: u64, depth: usize) -> Option<Scale> {
        let f = self.f;
        if depth > 8 {
            return None;
        }
        if let Some(p) = f.rec(u, "SI_UNIT") {
            if enumeration(p, 1) != Some("METRE") {
                return None;
            }
            let (mul, div) = match enumeration(p, 0) {
                Some("MILLI") => (1.0, 1e3),
                Some("CENTI") => (1.0, 1e2),
                Some("DECI") => (1.0, 1e1),
                Some("MICRO") => (1.0, 1e6),
                Some("NANO") => (1.0, 1e9),
                Some("DECA") => (1e1, 1.0),
                Some("HECTO") => (1e2, 1.0),
                Some("KILO") => (1e3, 1.0),
                _ => (1.0, 1.0),
            };
            return Some(Scale { mul, div });
        }
        let p = f.rec(u, "CONVERSION_BASED_UNIT")?;
        // an inch or a foot, say; angles are converted units too
        f.rec(u, "LENGTH_UNIT")?;
        let measure = at(p, 1)?;
        let q = f.simple(measure).map(|(_, q)| q).or_else(|| f.rec(measure, "MEASURE_WITH_UNIT"))?;
        let value = num(q.first())?;
        let base = self.length_unit(at(q, 1)?, depth + 1)?;
        Some(Scale { mul: value * base.mul, div: base.div })
    }

    /// A representation's name, items and context.
    fn rep_params(&self, rep: u64) -> Option<&'f [P]> {
        let f = self.f;
        f.simple(rep).map(|(_, p)| p).or_else(|| f.rec(rep, "REPRESENTATION"))
    }

    /// The representations a product definition's shape is made of: those it names and those linked to them.
    fn reps(&mut self, pd: u64) -> Vec<u64> {
        if let Some(r) = self.reps.get(&pd) {
            return r.clone();
        }
        let x = self.index;
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut queue: VecDeque<u64> = x.shapes_of.get(&pd).into_iter().flatten().flat_map(|s| x.reps_of.get(s).into_iter().flatten()).copied().collect();
        while let Some(r) = queue.pop_front() {
            if !seen.insert(r) {
                continue;
            }
            out.push(r);
            queue.extend(x.linked.get(&r).into_iter().flatten().copied());
        }
        self.reps.insert(pd, out.clone());
        out
    }

    /// A product definition's PRODUCT as a definition: its name (its id when it has none) and, if Daedalus wrote it, what it was.
    fn product(&self, pd: u64) -> Definition {
        let f = self.f;
        let product = f.rec(pd, "PRODUCT_DEFINITION").and_then(|p| at(p, 2)).and_then(|formation| f.simple(formation)).and_then(|(_, q)| at(q, 2)).and_then(|p| f.rec(p, "PRODUCT"));
        let (id, name, description) = product.map(|p| (text(p, 0), text(p, 1), text(p, 2))).unwrap_or(("", "", ""));
        let words: Vec<&str> = description.split_whitespace().collect();
        let ours = words.first() == Some(&"Daedalus");
        Definition {
            name: if name.is_empty() && !ours { id.to_string() } else { name.to_string() },
            component: !(ours && words.get(1) == Some(&"group")),
            glue: ours && words.contains(&"glue"),
            cut: ours && words.contains(&"cut"),
            attributes: Vec::new(),
            part: None,
            ctx: Context::default(),
        }
    }

    /// A product's own faces and the products placed in it.
    fn product_context(&mut self, pd: u64, def_of: &HashMap<u64, usize>) -> Context {
        let mut b = Builder::default();
        let reps = self.reps(pd);
        // a shape given only as a tessellation (AP242) is not read; beside an exact one it is a copy of it
        let (tessellated, exact): (Vec<u64>, Vec<u64>) = reps.into_iter().partition(|&r| self.f.type_name(r).contains("TESSELLATED"));
        for &rep in &exact {
            self.rep_items(rep, &mut b, 0);
        }
        if b.mesh.faces.is_empty() && b.instances.is_empty() && !tessellated.is_empty() {
            for &r in &tessellated {
                let t = self.f.type_name(r);
                self.skip(format!("shapes of type {t}"));
            }
        }
        let x = self.index;
        for &(usage, held) in x.used_in.get(&pd).into_iter().flatten() {
            let Some(&def) = def_of.get(&held) else { continue };
            let xf = self.usage_placement(usage, held).unwrap_or_else(|| {
                self.note("parts with no placement given (put at the origin)");
                Xform::IDENTITY
            });
            b.instances.push(Instance { def, xf });
        }
        Context { mesh: b.mesh, instances: b.instances }
    }

    /// Where a usage places the part it uses, in the coordinates of the one holding it.
    fn usage_placement(&mut self, usage: u64, held: u64) -> Option<Xform> {
        let f = self.f;
        let x = self.index;
        let relation = x.shapes_of.get(&usage)?.iter().find_map(|s| x.placed_by.get(s).copied())?;
        let rel = f.rec(relation, "REPRESENTATION_RELATIONSHIP")?;
        let (r1, r2) = (at(rel, 2)?, at(rel, 3)?);
        let op = f.rec(relation, "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION").and_then(|p| at(p, 0))?;
        let (s1, s2) = (self.rep_scale(r1), self.rep_scale(r2));
        let t = match f.rec(op, "ITEM_DEFINED_TRANSFORMATION") {
            Some(p) => {
                let (a1, a2) = (self.axes(at(p, 2)?, s1), self.axes(at(p, 3)?, s2));
                a2.after(&a1.inverse()?)
            }
            None => self.operator(op, s2)?,
        };
        // the part's own shape first, the holder's second; some programs write them the other way round
        let own = self.reps(held);
        if !own.contains(&r1) && own.contains(&r2) {
            t.inverse()
        } else {
            Some(t)
        }
    }

    /// An AXIS2_PLACEMENT_3D (or 2D) as the transform from its axes to the coordinates it is given in.
    fn axes(&self, id: u64, scale: Scale) -> Xform {
        let f = self.f;
        let Some((_, p)) = f.simple(id) else { return Xform::IDENTITY };
        let o = at(p, 1).and_then(|c| self.point(c)).map(|c| c.map(|v| scale.metres(v))).unwrap_or([0.0; 3]);
        let (z, x) = match f.simple(id) {
            Some(("AXIS2_PLACEMENT_3D", _)) => (at(p, 2).and_then(|d| self.direction(d)).unwrap_or(Z), at(p, 3).and_then(|d| self.direction(d))),
            _ => (Z, at(p, 2).and_then(|d| self.direction(d))),
        };
        let x = x.unwrap_or(if z[0].abs() > 1.0 - 1e-12 { [0.0, 1.0, 0.0] } else { X });
        let x = across(x, z);
        frame(x, cross(z, x), z, o)
    }

    /// A CARTESIAN_TRANSFORMATION_OPERATOR_3D: axes (which may be mirrored), an origin and a scale.
    fn operator(&self, id: u64, scale: Scale) -> Option<Xform> {
        let f = self.f;
        let p = f.rec(id, "CARTESIAN_TRANSFORMATION_OPERATOR_3D").or_else(|| f.rec(id, "CARTESIAN_TRANSFORMATION_OPERATOR"))?;
        // axis1, axis2, local_origin, scale, axis3, found by the origin among them (writers differ in the names before)
        let k = (0..p.len()).find(|&i| at(p, i).is_some_and(|c| f.rec(c, "CARTESIAN_POINT").is_some()))?;
        let dir = |i: Option<usize>| i.and_then(|i| at(p, i)).and_then(|d| self.direction(d));
        let o = self.point(at(p, k)?)?.map(|v| scale.metres(v));
        let s = num(p.get(k + 1)).unwrap_or(1.0);
        let z = dir(Some(k + 2)).unwrap_or(Z);
        let x = across(dir(k.checked_sub(2)).unwrap_or(if z[0].abs() > 1.0 - 1e-12 { [0.0, 1.0, 0.0] } else { X }), z);
        let y = match dir(k.checked_sub(1)) {
            Some(a2) => unit(sub(sub(a2, times(z, dot(a2, z))), times(x, dot(a2, x)))).unwrap_or(cross(z, x)),
            None => cross(z, x),
        };
        Some(frame(times(x, s), times(y, s), times(z, s), o))
    }

    fn point(&self, id: u64) -> Option<V3> {
        let p = self.f.rec(id, "CARTESIAN_POINT")?;
        let c = list(p, 1);
        let v = |i: usize| num(c.get(i));
        Some([v(0)?, v(1)?, v(2).unwrap_or(0.0)])
    }

    fn direction(&self, id: u64) -> Option<V3> {
        let p = self.f.rec(id, "DIRECTION")?;
        let c = list(p, 1);
        let v = |i: usize| num(c.get(i));
        unit([v(0)?, v(1)?, v(2).unwrap_or(0.0)])
    }

    /// A representation's items into `b`.
    fn rep_items(&mut self, rep: u64, b: &mut Builder, depth: usize) {
        let Some(p) = self.rep_params(rep) else { return };
        let scale = self.rep_scale(rep);
        let colour = self.colour.get(&rep).copied();
        for item in ids(p, 1) {
            self.item(item, scale, b, colour, depth);
        }
    }

    fn item(&mut self, id: u64, scale: Scale, b: &mut Builder, colour: Option<[u8; 3]>, depth: usize) {
        let f = self.f;
        let colour = self.colour.get(&id).copied().or(colour);
        let Some((kind, p)) = f.simple(id) else {
            let t = f.type_name(id);
            self.skip(format!("items of type {t}"));
            return;
        };
        match kind {
            "MANIFOLD_SOLID_BREP" | "FACETED_BREP" => {
                if let Some(s) = at(p, 1) {
                    self.shell(s, false, scale, b, colour, 0);
                }
            }
            "BREP_WITH_VOIDS" => {
                for s in at(p, 1).into_iter().chain(ids(p, 2)) {
                    self.shell(s, false, scale, b, colour, 0);
                }
            }
            "SHELL_BASED_SURFACE_MODEL" | "FACE_BASED_SURFACE_MODEL" => {
                for s in ids(p, 1) {
                    self.shell(s, false, scale, b, colour, 0);
                }
            }
            "MAPPED_ITEM" => self.mapped(p, scale, b, depth),
            // wireframe: loose lines
            "GEOMETRIC_CURVE_SET" | "GEOMETRIC_SET" => {
                for c in ids(p, 1) {
                    self.wire(c, scale, b);
                }
            }
            // placements and points: nothing to draw
            "AXIS2_PLACEMENT_3D" | "AXIS2_PLACEMENT_2D" | "CARTESIAN_POINT" | "DIRECTION" => {}
            other => self.skip(format!("items of type {other}")),
        }
    }

    /// One curve of a wireframe as loose lines: a polyline's segments (a B-spline of degree 1 is one), a line trimmed at both
    /// ends, a circle or ellipse whole or trimmed at two points (in segments). A point is passed over.
    fn wire(&mut self, c: u64, scale: Scale, b: &mut Builder) {
        let f = self.f;
        let chain = |b: &mut Builder, run: &[u32]| b.mesh.lines.extend(run.windows(2).filter(|w| w[0] != w[1]).map(|w| [w[0], w[1]]));
        let poly = |r: &mut Self, corners: &[u64], b: &mut Builder| {
            let run: Vec<u32> = corners.iter().filter_map(|&cp| r.vertex_at(cp, scale, b)).collect();
            chain(b, &run);
        };
        match f.simple(c) {
            Some(("POLYLINE", p)) => return poly(self, &ids(p, 1).collect::<Vec<u64>>(), b),
            Some(("B_SPLINE_CURVE_WITH_KNOTS" | "B_SPLINE_CURVE", p)) if num(p.get(1)) == Some(1.0) => return poly(self, &ids(p, 2).collect::<Vec<u64>>(), b),
            Some(("CARTESIAN_POINT", _)) => return,
            Some(("CIRCLE" | "ELLIPSE", p)) => {
                let Some(position) = at(p, 1) else { return };
                let r1 = num(p.get(2)).unwrap_or(0.0);
                let r2 = if f.simple(c).is_some_and(|(k, _)| k == "ELLIPSE") { num(p.get(3)).unwrap_or(0.0) } else { r1 };
                let ax = self.axes(position, scale);
                let start = plus(ax.origin(), times(column(&ax, 0), scale.metres(r1)));
                let first = b.mesh.add_vertex(start);
                let along = self.arc(position, r1, r2, true, start, start, true, scale, b);
                let run: Vec<u32> = std::iter::once(first).chain(along).chain(std::iter::once(first)).collect();
                return chain(b, &run);
            }
            Some(("TRIMMED_CURVE", p)) => {
                // TRIMMED_CURVE(name, basis, trim_1, trim_2, sense_agreement, master): a trim is a point, a parameter, or both
                let point = |trim: &[P]| trim.iter().find_map(|t| if let P::Ref(id) = t { f.rec(*id, "CARTESIAN_POINT").map(|_| *id) } else { None });
                let parameter = |trim: &[P]| trim.iter().find_map(|t| if let P::Typed(name, v) = t { (name == "PARAMETER_VALUE").then(|| num(Some(v))).flatten() } else { None });
                let (t1, t2) = (list(p, 2), list(p, 3));
                let basis = at(p, 1).and_then(|id| f.simple(id).map(|(k, q)| (id, k, q)));
                match basis {
                    Some((_, "LINE", q)) => {
                        // LINE(name, pnt, VECTOR(name, orientation, magnitude))
                        let origin = at(q, 1).and_then(|cp| self.point(cp));
                        let vector = at(q, 2).and_then(|v| f.simple(v)).and_then(|(_, v)| Some((at(v, 1).and_then(|d| self.direction(d))?, num(v.get(2))?)));
                        let end = |r: &mut Self, trim: &[P], b: &mut Builder| match (point(trim), parameter(trim), origin, vector) {
                            (Some(cp), _, _, _) => r.vertex_at(cp, scale, b),
                            (None, Some(t), Some(o), Some((d, m))) => Some(b.mesh.add_vertex(plus(o, times(d, t * m)).map(|x| scale.metres(x)))),
                            _ => None,
                        };
                        if let (Some(a), Some(z)) = (end(self, t1, b), end(self, t2, b)) {
                            return chain(b, &[a, z]);
                        }
                    }
                    Some((id, "CIRCLE" | "ELLIPSE", q)) => {
                        if let (Some(a), Some(z), Some(position)) = (point(t1), point(t2), at(q, 1)) {
                            let r1 = num(q.get(2)).unwrap_or(0.0);
                            let r2 = if f.simple(id).is_some_and(|(k, _)| k == "ELLIPSE") { num(q.get(3)).unwrap_or(0.0) } else { r1 };
                            if let (Some(va), Some(vz)) = (self.vertex_at(a, scale, b), self.vertex_at(z, scale, b)) {
                                let (pa, pz) = (b.mesh.positions[va as usize], b.mesh.positions[vz as usize]);
                                let along = self.arc(position, r1, r2, flag(p, 4).unwrap_or(true), pa, pz, va == vz, scale, b);
                                let run: Vec<u32> = std::iter::once(va).chain(along).chain(std::iter::once(vz)).collect();
                                return chain(b, &run);
                            }
                        }
                    }
                    _ => {}
                }
                let of = at(p, 1).map(|id| f.type_name(id)).unwrap_or_default();
                return self.skip(format!("wireframe curves of type TRIMMED_CURVE of {of}"));
            }
            _ => {}
        }
        let t = f.type_name(c);
        self.skip(format!("wireframe curves of type {t}"));
    }

    /// A MAPPED_ITEM: a copy of another representation, placed; that representation becomes a component, read once.
    fn mapped(&mut self, p: &[P], scale: Scale, b: &mut Builder, depth: usize) {
        let f = self.f;
        let Some(q) = at(p, 1).and_then(|m| f.rec(m, "REPRESENTATION_MAP")) else { return };
        let (Some(origin), Some(rep)) = (at(q, 0), at(q, 1)) else { return };
        let rep_scale = self.rep_scale(rep);
        let def = match self.of_rep.get(&rep) {
            Some(&d) => d,
            None => {
                if depth > 32 || self.building.contains(&rep) {
                    self.note("items mapped into themselves (left out)");
                    return;
                }
                let name = self.rep_params(rep).map(|p| text(p, 0)).filter(|n| !n.is_empty()).unwrap_or("Mapped item").to_string();
                let d = self.defs.len();
                self.defs.push(Definition { name, component: true, glue: false, cut: false, attributes: Vec::new(), part: None, ctx: Context::default() });
                self.of_rep.insert(rep, d);
                self.building.push(rep);
                let mut inner = Builder::default();
                self.rep_items(rep, &mut inner, depth + 1);
                self.building.pop();
                self.defs[d].ctx = Context { mesh: inner.mesh, instances: inner.instances };
                d
            }
        };
        let from = self.axes(origin, rep_scale);
        let to = match at(p, 2) {
            Some(t) if f.rec(t, "AXIS2_PLACEMENT_3D").is_some() => self.axes(t, scale),
            Some(t) => self.operator(t, scale).unwrap_or(Xform::IDENTITY),
            None => Xform::IDENTITY,
        };
        let xf = match from.inverse() {
            Some(inv) => to.after(&inv),
            None => to,
        };
        b.instances.push(Instance { def, xf });
    }

    /// A shell's faces (`flip`: used the other way round, as an inner void or an oriented shell may be).
    fn shell(&mut self, id: u64, flip: bool, scale: Scale, b: &mut Builder, colour: Option<[u8; 3]>, depth: usize) {
        let f = self.f;
        let colour = self.colour.get(&id).copied().or(colour);
        match f.simple(id) {
            Some(("CLOSED_SHELL" | "OPEN_SHELL" | "CONNECTED_FACE_SET", p)) => {
                for face in ids(p, 1) {
                    self.face(face, flip, scale, b, colour, 0);
                }
            }
            Some(("ORIENTED_CLOSED_SHELL" | "ORIENTED_OPEN_SHELL", p)) if depth < 8 => {
                if let Some(e) = at(p, 2) {
                    let along = flag(p, 3).unwrap_or(true);
                    self.shell(e, flip != !along, scale, b, colour, depth + 1);
                }
            }
            _ => {
                let t = f.type_name(id);
                self.skip(format!("shells of type {t}"));
            }
        }
    }

    fn face(&mut self, id: u64, flip: bool, scale: Scale, b: &mut Builder, colour: Option<[u8; 3]>, depth: usize) {
        let f = self.f;
        let colour = self.colour.get(&id).copied().or(colour);
        match f.simple(id) {
            Some(("ORIENTED_FACE", p)) if depth < 8 => {
                if let Some(e) = at(p, 2) {
                    let along = flag(p, 3).unwrap_or(true);
                    self.face(e, flip != !along, scale, b, colour, depth + 1);
                }
            }
            Some(("ADVANCED_FACE" | "FACE_SURFACE", p)) => {
                let normal = match at(p, 2) {
                    Some(s) => match self.plane_normal(s, 0) {
                        Ok(n) => n,
                        Err(t) => return self.skip(format!("faces on {t}")),
                    },
                    None => return self.skip("faces with no surface".into()),
                };
                // the face's own normal: its surface's, turned when it says so, and again when it is used turned
                let same = flag(p, 3).unwrap_or(true);
                let normal = if same != flip { normal } else { times(normal, -1.0) };
                self.add_face(list(p, 1), Some(normal), flip, scale, b, colour);
            }
            Some(("FACE", p)) => self.add_face(list(p, 1), None, flip, scale, b, colour),
            _ => {
                let t = f.type_name(id);
                self.skip(format!("faces of type {t}"));
            }
        }
    }

    /// The normal of a flat surface (a PLANE, maybe trimmed or offset), or the kind of surface it is if it is not flat.
    fn plane_normal(&self, s: u64, depth: usize) -> Result<V3, String> {
        let f = self.f;
        let basis = |p: &[P]| at(p, 1).ok_or_else(|| f.type_name(s));
        match f.simple(s) {
            Some(("PLANE", p)) => Ok(at(p, 1).and_then(|a| f.simple(a)).and_then(|(_, q)| at(q, 2)).and_then(|d| self.direction(d)).unwrap_or(Z)),
            // its first and second parameters may run against the plane's, which turns it over
            Some(("RECTANGULAR_TRIMMED_SURFACE", p)) if depth < 8 => {
                let n = self.plane_normal(basis(p)?, depth + 1)?;
                Ok(if flag(p, 6).unwrap_or(true) == flag(p, 7).unwrap_or(true) { n } else { times(n, -1.0) })
            }
            Some(("OFFSET_SURFACE" | "CURVE_BOUNDED_SURFACE", p)) if depth < 8 => self.plane_normal(basis(p)?, depth + 1),
            _ => Err(f.type_name(s)),
        }
    }

    /// A face from its bounds: the outline wound counter-clockwise about its normal and its holes the other way, as Daedalus
    /// keeps faces (the normal: the surface's; for a FACE with no surface, its outline's).
    fn add_face(&mut self, bounds: &[P], normal: Option<V3>, flip: bool, scale: Scale, b: &mut Builder, colour: Option<[u8; 3]>) {
        let f = self.f;
        let mut outer: Option<Vec<u32>> = None;
        let mut rings: Vec<Vec<u32>> = Vec::new();
        let mut used = Vec::new();
        for bound in bounds.iter().filter_map(|x| if let P::Ref(id) = x { Some(*id) } else { None }) {
            let Some((kind, p)) = f.simple(bound) else { continue };
            let Some(lp) = at(p, 1) else { continue };
            let mut ring = match self.ring(lp, scale, b, &mut used) {
                Ok(r) => r,
                Err(Some(what)) => return self.skip(format!("faces bounded by {what}")),
                Err(None) => continue,
            };
            if flag(p, 2) == Some(false) {
                ring.reverse();
            }
            let pos = &b.mesh.positions;
            ring.dedup_by(|x, y| pos[*x as usize] == pos[*y as usize]);
            while ring.len() > 1 && pos[ring[0] as usize] == pos[ring[ring.len() - 1] as usize] {
                ring.pop();
            }
            if ring.len() < 3 {
                continue;
            }
            if kind == "FACE_OUTER_BOUND" && outer.is_none() {
                outer = Some(ring);
            } else {
                rings.push(ring);
            }
        }
        let pos = &b.mesh.positions;
        let marked = outer.is_some();
        let normal = match normal {
            Some(n) => n,
            None => {
                let n = outer.as_ref().or(rings.first()).and_then(|r| unit(area_normal(pos, r))).unwrap_or(Z);
                if flip {
                    times(n, -1.0)
                } else {
                    n
                }
            }
        };
        let mut outer = match outer {
            Some(o) => o,
            None => {
                // no bound marked as the outline: the one round the most area is
                let along = |r: &Vec<u32>| dot(area_normal(pos, r), normal).abs();
                let Some(k) = (0..rings.len()).max_by(|&i, &j| along(&rings[i]).total_cmp(&along(&rings[j]))) else {
                    return self.note("faces with no area");
                };
                rings.remove(k)
            }
        };
        if dot(area_normal(pos, &outer), normal) < 0.0 {
            outer.reverse();
            if marked {
                self.note("faces whose loops ran against the plane (turned to match)");
            }
        }
        for h in &mut rings {
            if dot(area_normal(pos, h), normal) > 0.0 {
                h.reverse();
            }
        }
        let face = b.mesh.faces.len();
        b.mesh.add_face(outer);
        b.mesh.set_holes(face, rings);
        if let Some(c) = colour {
            b.mesh.set_color(face, Some(Paint::from(c)));
        }
        // its edges drawn smooth: soft edges again, each segment of one (the faces either side share it: marked once)
        for e in used {
            if self.index.soft.contains(&e) && b.soft_done.insert(e) {
                if let Some(Ok((start, end, along))) = b.edges.get(&e) {
                    let run: Vec<u32> = std::iter::once(*start).chain(along.iter().copied()).chain(std::iter::once(*end)).collect();
                    b.mesh.seams.extend(run.windows(2).filter(|w| w[0] != w[1]).map(|w| [w[0], w[1]]));
                }
            }
        }
    }

    /// A loop's vertices in order, or what kind of edge or loop could not be read (None: a loop of no length, a VERTEX_LOOP);
    /// the edges it runs along are added to `used`.
    fn ring(&mut self, lp: u64, scale: Scale, b: &mut Builder, used: &mut Vec<u64>) -> Result<Vec<u32>, Option<String>> {
        let f = self.f;
        match f.simple(lp) {
            Some(("EDGE_LOOP", p)) => {
                let mut out = Vec::new();
                for oe in ids(p, 1) {
                    let (edge, forward) = self.oriented_edge(oe, 0)?;
                    let (start, end, along) = self.edge(edge, scale, b)?;
                    used.push(edge);
                    if forward {
                        out.push(start);
                        out.extend_from_slice(&along);
                    } else {
                        out.push(end);
                        out.extend(along.iter().rev());
                    }
                }
                Ok(out)
            }
            Some(("POLY_LOOP", p)) => ids(p, 1).map(|c| self.vertex_at(c, scale, b).ok_or(None)).collect(),
            Some(("VERTEX_LOOP", _)) => Err(None),
            _ => Err(Some(format!("{} loops", f.type_name(lp)))),
        }
    }

    /// An ORIENTED_EDGE's edge, and whether the loop runs along it (from its start) or against it.
    fn oriented_edge(&self, oe: u64, depth: usize) -> Result<(u64, bool), Option<String>> {
        let f = self.f;
        match f.simple(oe) {
            Some(("ORIENTED_EDGE", p)) if depth < 8 => {
                let e = at(p, 3).ok_or(None)?;
                let along = flag(p, 4).unwrap_or(true);
                if f.simple(e).is_some_and(|(k, _)| k == "ORIENTED_EDGE") {
                    let (inner, a) = self.oriented_edge(e, depth + 1)?;
                    return Ok((inner, a == along));
                }
                Ok((e, along))
            }
            Some(("EDGE_CURVE" | "EDGE", _)) => Ok((oe, true)),
            _ => Err(Some(format!("{} edges", f.type_name(oe)))),
        }
    }

    /// An edge's first and last vertex and the vertices along it between (its curve in segments), read once.
    fn edge(&mut self, id: u64, scale: Scale, b: &mut Builder) -> EdgeRead {
        if let Some(e) = b.edges.get(&id) {
            return e.clone();
        }
        let made = self.read_edge(id, scale, b);
        b.edges.insert(id, made.clone());
        made
    }

    fn read_edge(&mut self, id: u64, scale: Scale, b: &mut Builder) -> EdgeRead {
        let f = self.f;
        let Some((kind, p)) = f.simple(id) else { return Err(Some(format!("{} edges", f.type_name(id)))) };
        let (vs, ve) = (at(p, 1).ok_or(None)?, at(p, 2).ok_or(None)?);
        let start = self.vertex(vs, scale, b).ok_or(None)?;
        let end = self.vertex(ve, scale, b).ok_or(None)?;
        let along = match (kind, at(p, 3)) {
            ("EDGE_CURVE", Some(curve)) => self.curve_points(curve, flag(p, 4).unwrap_or(true), start, end, vs == ve, scale, b)?,
            _ => Vec::new(),
        };
        Ok((start, end, along))
    }

    fn vertex(&mut self, id: u64, scale: Scale, b: &mut Builder) -> Option<u32> {
        let p = self.f.rec(id, "VERTEX_POINT")?;
        self.vertex_at(at(p, 1)?, scale, b)
    }

    /// The vertex at a CARTESIAN_POINT, one per point.
    fn vertex_at(&mut self, cp: u64, scale: Scale, b: &mut Builder) -> Option<u32> {
        if let Some(&v) = b.points.get(&cp) {
            return Some(v);
        }
        let p = self.point(cp)?.map(|c| scale.metres(c));
        let v = b.mesh.add_vertex(p);
        b.points.insert(cp, v);
        Some(v)
    }

    /// The vertices along an edge's curve between its ends, from `start` to `end` (`same`: the edge runs the curve's way):
    /// none for a line, a polyline's corners, a circle or ellipse in segments.
    #[allow(clippy::too_many_arguments)]
    fn curve_points(&mut self, curve: u64, same: bool, start: u32, end: u32, closed: bool, scale: Scale, b: &mut Builder) -> Result<Vec<u32>, Option<String>> {
        let f = self.f;
        let (mut c, mut same) = (curve, same);
        // past what only wraps a curve: on a surface, trimmed (its trimming runs against it, maybe)
        for _ in 0..8 {
            match f.simple(c) {
                Some(("SURFACE_CURVE" | "SEAM_CURVE" | "INTERSECTION_CURVE", p)) => c = at(p, 1).ok_or(None)?,
                Some(("TRIMMED_CURVE", p)) => {
                    if flag(p, 4) == Some(false) {
                        same = !same;
                    }
                    c = at(p, 1).ok_or(None)?;
                }
                _ => break,
            }
        }
        let closed = closed || start == end;
        let (ps, pe) = (b.mesh.positions[start as usize], b.mesh.positions[end as usize]);
        // a B-spline's degree and control points: from its own record, or from the B_SPLINE_CURVE part of a complex one
        let spline = match f.simple(c) {
            Some(("B_SPLINE_CURVE_WITH_KNOTS" | "B_SPLINE_CURVE" | "BEZIER_CURVE" | "UNIFORM_CURVE" | "QUASI_UNIFORM_CURVE", p)) => Some((num(p.get(1)), ids(p, 2).collect::<Vec<u64>>())),
            Some(_) => None,
            None => f.rec(c, "B_SPLINE_CURVE").map(|p| (num(p.first()), ids(p, 1).collect())),
        };
        match f.simple(c) {
            Some(("LINE", _)) => return Ok(Vec::new()),
            Some(("POLYLINE", p)) => {
                let corners: Vec<u64> = ids(p, 1).collect();
                return self.polyline(&corners, same, ps, pe, closed, scale, b);
            }
            Some(("CIRCLE", p)) => {
                let r = num(p.get(2)).ok_or(None)?;
                return Ok(self.arc(at(p, 1).ok_or(None)?, r, r, same, ps, pe, closed, scale, b));
            }
            Some(("ELLIPSE", p)) => {
                let (r1, r2) = (num(p.get(2)).ok_or(None)?, num(p.get(3)).ok_or(None)?);
                return Ok(self.arc(at(p, 1).ok_or(None)?, r1, r2, same, ps, pe, closed, scale, b));
            }
            _ => {}
        }
        // a B-spline of degree 1 is the polyline of its control points
        if let Some((Some(degree), corners)) = spline {
            if degree == 1.0 {
                return self.polyline(&corners, same, ps, pe, closed, scale, b);
            }
        }
        Err(Some(format!("{} edges", f.type_name(c))))
    }

    #[allow(clippy::too_many_arguments)]
    fn polyline(&mut self, corners: &[u64], same: bool, ps: V3, pe: V3, closed: bool, scale: Scale, b: &mut Builder) -> Result<Vec<u32>, Option<String>> {
        let mut pts: Vec<(u64, V3)> = corners.iter().filter_map(|&c| Some((c, self.point(c)?.map(|v| scale.metres(v))))).collect();
        if pts.len() < 2 {
            return Ok(Vec::new());
        }
        if !same {
            pts.reverse();
        }
        let d = |a: V3, b: V3| length(sub(a, b));
        let (first, last) = (pts[0].1, pts[pts.len() - 1].1);
        // the edge's ends tell which way it runs, whatever the file says
        if !closed && d(first, pe) + d(last, ps) < d(first, ps) + d(last, pe) {
            pts.reverse();
        }
        let size = pts.iter().fold(0.0f64, |m, (_, p)| m.max(d(*p, ps)));
        let tol = 1e-9 * size + 1e-12;
        if d(pts[0].1, ps) <= tol {
            pts.remove(0);
        }
        if pts.last().is_some_and(|&(_, p)| d(p, pe) <= tol) {
            pts.pop();
        }
        Ok(pts.into_iter().filter_map(|(c, _)| self.vertex_at(c, scale, b)).collect())
    }

    /// The points along a circle or ellipse from `ps` to `pe`, counter-clockwise about its axis when `same` (the whole way
    /// round when `closed`), in segments of at most 1/24 of a turn.
    #[allow(clippy::too_many_arguments)]
    fn arc(&mut self, position: u64, r1: f64, r2: f64, same: bool, ps: V3, pe: V3, closed: bool, scale: Scale, b: &mut Builder) -> Vec<u32> {
        let ax = self.axes(position, scale);
        let (c, x, y) = (ax.origin(), column(&ax, 0), column(&ax, 1));
        let (r1, r2) = (scale.metres(r1), scale.metres(r2));
        if !(r1 > 0.0 && r2 > 0.0) {
            return Vec::new();
        }
        let angle = |p: V3| {
            let d = sub(p, c);
            (dot(d, y) / r2).atan2(dot(d, x) / r1)
        };
        let tau = std::f64::consts::TAU;
        let a0 = angle(ps);
        let ccw = if closed { tau } else { (angle(pe) - a0).rem_euclid(tau) };
        let ccw = if ccw == 0.0 { tau } else { ccw };
        let sweep = if same { ccw } else { -(if closed { tau } else { tau - ccw }) };
        let n = ((sweep.abs() / tau) * CIRCLE_SEGMENTS).ceil().max(1.0) as usize;
        (1..n)
            .map(|k| {
                let t = a0 + sweep * k as f64 / n as f64;
                b.mesh.add_vertex(plus(c, plus(times(x, r1 * t.cos()), times(y, r2 * t.sin()))))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daedalus_core::dynamic::Attribute;
    use daedalus_core::scene::{Guide, Texture};

    /// A prism from z0 up to z1: `outline` counter-clockwise from above, `holes` clockwise; every face facing out.
    fn prism(outline: &[[f64; 2]], holes: &[Vec<[f64; 2]>], z0: f64, z1: f64) -> Mesh {
        let mut m = Mesh::new();
        let loops: Vec<&[[f64; 2]]> = std::iter::once(outline).chain(holes.iter().map(Vec::as_slice)).collect();
        let (mut lo, mut hi) = (Vec::new(), Vec::new());
        for l in &loops {
            lo.push(l.iter().map(|p| m.add_vertex([p[0], p[1], z0])).collect::<Vec<u32>>());
            hi.push(l.iter().map(|p| m.add_vertex([p[0], p[1], z1])).collect::<Vec<u32>>());
        }
        // the top as given, the bottom the other way round, then the sides
        m.add_face(hi[0].clone());
        m.set_holes(0, hi[1..].to_vec());
        m.add_face(lo[0].iter().rev().copied().collect());
        m.set_holes(1, lo[1..].iter().map(|h| h.iter().rev().copied().collect()).collect());
        for (l, h) in lo.iter().zip(&hi) {
            for i in 0..l.len() {
                let j = (i + 1) % l.len();
                m.add_face(vec![l[i], l[j], h[j], h[i]]);
            }
        }
        m
    }

    fn square(x: f64, y: f64, s: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]]
    }

    fn hole(x: f64, y: f64, s: f64) -> Vec<[f64; 2]> {
        square(x, y, s).into_iter().rev().collect()
    }

    fn cube(s: f64) -> Mesh {
        prism(&square(0.0, 0.0, s), &[], 0.0, s)
    }

    fn def(name: &str, component: bool, mesh: Mesh, instances: Vec<Instance>) -> Definition {
        Definition { name: name.into(), component, glue: false, cut: false, attributes: Vec::new(), part: None, ctx: Context { mesh, instances } }
    }

    /// A turn by `degrees` about `axis` through `at`.
    fn turn(axis: V3, degrees: f64, at: V3) -> Xform {
        let k = unit(axis).unwrap();
        let (s, c) = degrees.to_radians().sin_cos();
        Xform::from_fn(|p| {
            let v = sub(p, at);
            plus(plus(plus(times(v, c), times(cross(k, v), s)), times(k, dot(k, v) * (1.0 - c))), at)
        })
    }

    fn key(p: V3) -> [i64; 3] {
        p.map(|c| (c * 1e7).round() as i64)
    }

    /// A loop as its corners' positions, from its lowest corner on.
    fn corners(m: &Mesh, r: &[u32]) -> Vec<V3> {
        let mut pts: Vec<V3> = r.iter().map(|&v| m.positions[v as usize]).collect();
        let first = (0..pts.len()).min_by_key(|&i| key(pts[i])).unwrap();
        pts.rotate_left(first);
        pts
    }

    type Face = (Vec<V3>, Vec<Vec<V3>>, Option<[u8; 3]>);

    /// A mesh's faces in an order and form that survive a write and a read: outline, holes, colour.
    fn faces(m: &Mesh) -> Vec<Face> {
        let mut out: Vec<Face> = (0..m.faces.len())
            .map(|f| {
                let mut holes: Vec<Vec<V3>> = m.holes_of(f).iter().map(|h| corners(m, h)).collect();
                holes.sort_by_key(|h| key(h[0]));
                (corners(m, &m.faces[f]), holes, m.color_of(f).map(|c| c.rgb))
            })
            .collect();
        out.sort_by_key(|(o, _, _)| o.iter().map(|&p| key(p)).collect::<Vec<_>>());
        out
    }

    /// The same faces: as many, each corner within 1e-9 m, wound the same way, the same holes and colours.
    fn assert_same_faces(a: &Mesh, b: &Mesh) {
        let (fa, fb) = (faces(a), faces(b));
        assert_eq!(fa.len(), fb.len(), "how many faces");
        let near = |p: &[V3], q: &[V3]| p.len() == q.len() && p.iter().zip(q).all(|(x, y)| length(sub(*x, *y)) < 1e-9);
        for (x, y) in fa.iter().zip(&fb) {
            assert!(near(&x.0, &y.0), "outline {:?} came back as {:?}", x.0, y.0);
            assert_eq!(x.1.len(), y.1.len(), "holes of {:?}", x.0);
            for (h, k) in x.1.iter().zip(&y.1) {
                assert!(near(h, k), "hole {h:?} came back as {k:?}");
            }
            assert_eq!(x.2, y.2, "colour of {:?}", x.0);
        }
    }

    fn assert_same_xf(a: &Xform, b: &Xform) {
        assert!((0..3).all(|r| (0..4).all(|c| (a.0[r][c] - b.0[r][c]).abs() < 1e-9)), "{a:?} came back as {b:?}");
    }

    /// How many entities of a type the text defines.
    fn n(text: &str, entity: &str) -> usize {
        text.matches(&format!("={entity}(")).count()
    }

    /// Every entity defined once and every one referred to defined; the header Part 21 asks for; the end.
    fn check_structure(text: &str) {
        assert!(text.starts_with("ISO-10303-21;\nHEADER;\n"), "the start");
        assert!(text.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"), "the end");
        let (header, data) = text.split_once("DATA;\n").expect("a DATA section");
        assert!(header.contains("FILE_DESCRIPTION(("));
        assert!(header.contains("FILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));"));
        let file_name = header.lines().find(|l| l.starts_with("FILE_NAME(")).expect("FILE_NAME");
        assert!(file_name.ends_with(",'Daedalus CAD','');"), "{file_name}");
        assert!(file_name.contains(&format!("'Daedalus CAD {}'", include_str!("../../../VERSION").trim())), "the program and Daedalus's VERSION: {file_name}");
        // the time it was written, in ISO 8601: 2026-11-08T14:03:12+00:00
        let stamp = file_name.split('\'').nth(3).unwrap();
        assert_eq!(stamp.len(), 25, "{stamp}");
        for (i, c) in stamp.chars().enumerate() {
            let want = match i {
                4 | 7 => '-',
                10 => 'T',
                13 | 16 | 22 => ':',
                19 => '+',
                _ => '0',
            };
            assert!(if want == '0' { c.is_ascii_digit() } else { c == want }, "{stamp}");
        }
        let data = data.strip_suffix("ENDSEC;\nEND-ISO-10303-21;\n").unwrap();
        let mut defined = HashSet::new();
        let mut used = Vec::new();
        for line in data.lines() {
            assert!(line.ends_with(';'), "{line}");
            let (id, body) = line.split_once('=').expect("#n=...");
            let id: u64 = id.strip_prefix('#').and_then(|i| i.parse().ok()).expect("#n");
            assert!(defined.insert(id), "#{id} is written twice");
            // the references, outside quoted text
            let b = body.as_bytes();
            let (mut quoted, mut i) = (false, 0);
            while i < b.len() {
                match b[i] {
                    b'\'' => quoted = !quoted,
                    b'#' if !quoted => {
                        let start = i + 1;
                        i = start;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                        used.push(body[start..i].parse::<u64>().unwrap());
                        continue;
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        for r in used {
            assert!(defined.contains(&r), "#{r} is referred to but not written");
        }
    }

    fn written(scene: &Scene) -> Written {
        let w = write(scene, &WriteOptions::default()).unwrap();
        check_structure(&w.text);
        w
    }

    fn read_back(w: &Written) -> Read {
        let r = read(w.text.as_bytes()).unwrap();
        assert!(r.skipped.is_empty(), "{:?}", r.skipped);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        r
    }

    #[test]
    fn a_cube_is_one_solid() {
        let m = cube(1.0);
        let w = written(&Scene::from_mesh(m.clone()));
        assert_eq!((w.products, w.solids, w.shells, w.faces), (1, 1, 0, 6));
        assert_eq!((n(&w.text, "MANIFOLD_SOLID_BREP"), n(&w.text, "CLOSED_SHELL"), n(&w.text, "OPEN_SHELL"), n(&w.text, "SHELL_BASED_SURFACE_MODEL")), (1, 1, 0, 0));
        // each face once, each edge and corner once though two or three faces share it
        assert_eq!((n(&w.text, "ADVANCED_FACE"), n(&w.text, "PLANE"), n(&w.text, "EDGE_CURVE"), n(&w.text, "VERTEX_POINT"), n(&w.text, "ORIENTED_EDGE")), (6, 6, 12, 8, 24));
        assert_eq!((n(&w.text, "FACE_OUTER_BOUND"), n(&w.text, "FACE_BOUND")), (6, 0));
        assert!(w.left_out.is_empty(), "{:?}", w.left_out);
        let back = read_back(&w);
        assert_eq!(back.units, Some(Units::Millimetres));
        assert_same_faces(&m, &back.scene.root.mesh);
        assert!((daedalus_core::inspect::signed_volume(&back.scene.root.mesh) - 1.0).abs() < 1e-9, "faces out round the volume");
        assert!(back.scene.defs.is_empty() && back.scene.root.instances.is_empty());
    }

    #[test]
    fn an_open_box_is_a_surface_model() {
        let mut m = cube(1.0);
        m.remove_face(0);
        let w = written(&Scene::from_mesh(m.clone()));
        assert_eq!((w.solids, w.shells, w.faces), (0, 1, 5));
        assert_eq!((n(&w.text, "SHELL_BASED_SURFACE_MODEL"), n(&w.text, "OPEN_SHELL"), n(&w.text, "MANIFOLD_SURFACE_SHAPE_REPRESENTATION")), (1, 1, 1));
        assert_eq!((n(&w.text, "MANIFOLD_SOLID_BREP"), n(&w.text, "CLOSED_SHELL"), n(&w.text, "ADVANCED_BREP_SHAPE_REPRESENTATION")), (0, 0, 0));
        assert_same_faces(&m, &read_back(&w).scene.root.mesh);
    }

    #[test]
    fn an_inside_out_box_is_no_solid() {
        let mut m = cube(1.0);
        for f in &mut m.faces {
            f.reverse();
        }
        let w = written(&Scene::from_mesh(m.clone()));
        assert_eq!((w.solids, w.shells), (0, 1), "closed, but round no volume");
        assert_eq!((n(&w.text, "CLOSED_SHELL"), n(&w.text, "SHELL_BASED_SURFACE_MODEL")), (1, 1));
        assert_same_faces(&m, &read_back(&w).scene.root.mesh);
    }

    #[test]
    fn a_face_keeps_both_its_holes() {
        // a plate with two square holes through it: a solid whose top and bottom each have two holes
        let plate = prism(&square(0.0, 0.0, 10.0), &[hole(2.0, 1.5, 1.0), hole(6.0, 1.5, 1.0)], 0.0, 1.0);
        let w = written(&Scene::from_mesh(plate.clone()));
        assert_eq!((w.solids, w.faces), (1, 14));
        assert_eq!((n(&w.text, "FACE_OUTER_BOUND"), n(&w.text, "FACE_BOUND")), (14, 4));
        let back = read_back(&w).scene.root.mesh;
        assert_same_faces(&plate, &back);
        assert_eq!(back.holes.iter().filter(|h| h.len() == 2).count(), 2);
        assert!((daedalus_core::inspect::signed_volume(&back) - 98.0).abs() < 1e-9);
        // and a lone face with two holes
        let mut face = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 2.0, 0.0], [0.0, 2.0, 0.0], [1.0, 0.5, 0.0], [1.0, 1.5, 0.0], [1.5, 1.5, 0.0], [1.5, 0.5, 0.0], [2.5, 0.5, 0.0], [2.5, 1.5, 0.0], [3.0, 1.0, 0.0]] {
            face.add_vertex(p);
        }
        face.add_face(vec![0, 1, 2, 3]);
        face.set_holes(0, vec![vec![4, 5, 6, 7], vec![8, 9, 10]]);
        let w = written(&Scene::from_mesh(face.clone()));
        assert_eq!((w.shells, n(&w.text, "FACE_BOUND")), (1, 2));
        assert_same_faces(&face, &read_back(&w).scene.root.mesh);
    }

    #[test]
    fn a_component_used_five_times_is_written_once() {
        let mut s = Scene::default();
        s.defs.push(def("Bolt", true, cube(0.1), Vec::new()));
        let places = [
            Xform::IDENTITY,
            Xform::translation([1.0, 0.0, 0.0]),
            turn([0.0, 0.0, 1.0], 30.0, [2.0, 0.0, 0.0]),
            turn([1.0, 2.0, 3.0], 137.0, [0.5, -0.25, 3.0]),
            Xform::translation([0.0, 0.0, -4.5]).after(&turn([1.0, 0.0, 0.0], 90.0, [0.0; 3])),
        ];
        s.root.instances = places.iter().map(|&xf| Instance { def: 0, xf }).collect();
        let w = written(&s);
        assert_eq!((n(&w.text, "PRODUCT"), n(&w.text, "NEXT_ASSEMBLY_USAGE_OCCURRENCE"), n(&w.text, "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION")), (2, 5, 5));
        assert_eq!((n(&w.text, "ADVANCED_FACE"), n(&w.text, "MANIFOLD_SOLID_BREP")), (6, 1), "the bolt's faces once");
        let back = read_back(&w).scene;
        assert_eq!(back.defs.len(), 1);
        assert_eq!((back.defs[0].name.as_str(), back.defs[0].component), ("Bolt", true));
        assert_same_faces(&s.defs[0].ctx.mesh, &back.defs[0].ctx.mesh);
        assert_eq!(back.root.instances.len(), 5);
        for (a, b) in s.root.instances.iter().zip(&back.root.instances) {
            assert_eq!(b.def, 0);
            assert_same_xf(&a.xf, &b.xf);
        }
    }

    #[test]
    fn a_model_comes_back_whole() {
        let mut leg = prism(&square(0.0, 0.0, 0.05), &[], 0.0, 0.7);
        for f in 0..leg.faces.len() {
            leg.set_color(f, Some([120, 80, 40].into()));
        }
        leg.set_color(1, Some([30, 30, 30].into()));
        let top = prism(&square(0.0, 0.0, 1.2), &[], 0.7, 0.74);
        let shelf = prism(&square(0.0, 0.0, 0.8), &[hole(0.1, 0.1, 0.2), hole(0.5, 0.3, 0.2)], 0.0, 0.02);
        let mut triangle = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [0.3, 0.0, 0.1], [0.0, 0.3, 0.2]] {
            triangle.add_vertex(p);
        }
        triangle.add_face(vec![0, 1, 2]);
        // the model's own geometry: an open box, painted in part, and a face with two holes above it
        let mut root = cube(1.0);
        root.remove_face(0);
        root.set_color(2, Some([10, 200, 30].into()));
        root.set_color(3, Some([10, 200, 30].into()));
        let at = root.positions.len() as u32;
        for p in [[0.0, 0.0, 2.0], [4.0, 0.0, 2.0], [4.0, 2.0, 2.0], [0.0, 2.0, 2.0], [1.0, 0.5, 2.0], [1.0, 1.5, 2.0], [1.5, 1.5, 2.0], [1.5, 0.5, 2.0], [2.5, 0.5, 2.0], [2.5, 1.5, 2.0], [3.0, 1.0, 2.0]] {
            root.add_vertex(p);
        }
        root.add_face(vec![at, at + 1, at + 2, at + 3]);
        let f = root.faces.len() - 1;
        root.set_holes(f, vec![vec![at + 4, at + 5, at + 6, at + 7], vec![at + 8, at + 9, at + 10]]);
        root.set_color(f, Some([0, 0, 255].into()));

        let mut s = Scene::from_mesh(root);
        s.defs.push(def("Leg", true, leg, Vec::new()));
        let legs = vec![
            Instance { def: 0, xf: Xform::translation([0.05, 0.05, 0.0]) },
            Instance { def: 0, xf: Xform::translation([1.1, 0.05, 0.0]) },
            Instance { def: 0, xf: turn([0.0, 0.0, 1.0], 90.0, [1.125, 1.125, 0.0]).after(&Xform::translation([1.1, 1.1, 0.0])) },
            Instance { def: 0, xf: Xform::translation([0.05, 1.1, 0.0]) },
        ];
        s.defs.push(def("Table", false, top, legs));
        let mut shelf_def = def("Shelf 'A' \"Tür\" \\ #1 € 😀", true, shelf, Vec::new());
        shelf_def.glue = true;
        shelf_def.cut = true;
        s.defs.push(shelf_def);
        s.defs.push(def("", false, triangle, Vec::new()));
        s.root.instances = vec![
            Instance { def: 1, xf: Xform::translation([3.0, 0.0, 0.0]) },
            Instance { def: 0, xf: turn([1.0, 2.0, 3.0], 37.0, [0.5, 0.5, 0.0]) },
            Instance { def: 2, xf: Xform::translation([0.0, 5.0, 1.0]).after(&turn([0.0, 0.0, 1.0], 90.0, [0.0; 3])) },
            Instance { def: 3, xf: Xform::IDENTITY },
        ];

        let w = written(&s);
        assert_eq!(w.products, 5, "the model and four definitions, each once");
        assert_eq!(n(&w.text, "NEXT_ASSEMBLY_USAGE_OCCURRENCE"), 8);
        assert!(w.left_out.is_empty(), "{:?}", w.left_out);
        let back = read_back(&w).scene;
        assert_same_faces(&s.root.mesh, &back.root.mesh);
        assert_eq!(back.defs.len(), s.defs.len());
        for (a, b) in s.defs.iter().zip(&back.defs) {
            assert_eq!((&a.name, a.component, a.glue, a.cut), (&b.name, b.component, b.glue, b.cut));
            assert_same_faces(&a.ctx.mesh, &b.ctx.mesh);
            assert_eq!(a.ctx.instances.len(), b.ctx.instances.len());
            for (x, y) in a.ctx.instances.iter().zip(&b.ctx.instances) {
                assert_eq!(x.def, y.def);
                assert_same_xf(&x.xf, &y.xf);
            }
        }
        assert_eq!(back.root.instances.len(), s.root.instances.len());
        for (x, y) in s.root.instances.iter().zip(&back.root.instances) {
            assert_eq!(x.def, y.def);
            assert_same_xf(&x.xf, &y.xf);
        }
        assert_same_faces(&s.flatten(), &back.flatten());
    }

    #[test]
    fn scaled_and_mirrored_copies_become_parts_of_their_own() {
        let bracket = prism(&[[0.0, 0.0], [0.3, 0.0], [0.3, 0.1], [0.1, 0.1], [0.1, 0.4], [0.0, 0.4]], &[], 0.0, 0.05);
        let mut s = Scene::default();
        s.defs.push(def("Bracket", true, bracket, Vec::new()));
        let mirror = Xform::from_fn(|p| [1.0 - p[0], p[1], p[2]]);
        let stretch = turn([0.0, 0.0, 1.0], 30.0, [0.0; 3]).after(&Xform::from_fn(|p| [2.0 * p[0], p[1] + 1.0, 0.5 * p[2]]));
        let slant = Xform::from_fn(|p| [p[0] + 0.5 * p[1], p[1] - 2.0, p[2]]);
        s.root.instances = [Xform::IDENTITY, mirror, stretch, slant].iter().map(|&xf| Instance { def: 0, xf }).collect();
        let w = written(&s);
        assert_eq!(w.products, 5, "the model, the bracket, and three brackets reshaped");
        assert_eq!(w.left_out, vec!["3 scaled or mirrored copies (each written as a part of its own)".to_string()]);
        let back = read_back(&w).scene;
        assert_same_faces(&s.flatten(), &back.flatten());
        for i in &back.root.instances {
            assert!((i.xf.det() - 1.0).abs() < 1e-9, "placed by a turn and a move");
        }
        let volume = daedalus_core::inspect::signed_volume(&s.flatten());
        assert!((daedalus_core::inspect::signed_volume(&back.flatten()) - volume).abs() < 1e-9, "faces still out");
    }

    #[test]
    fn a_dynamic_component_is_written_as_it_is_drawn() {
        let a = |name: &str, formula: &str| Attribute { name: name.into(), formula: formula.into(), label: None };
        let mut post = def("Post", true, cube(1.0), Vec::new());
        post.attributes = vec![a("Copies", "2"), a("X", "=COPY*1.5"), a("Material", "#00ff00")];
        let mut ghost = def("Ghost", true, cube(1.0), Vec::new());
        ghost.attributes = vec![a("Hidden", "true")];
        let mut s = Scene { defs: vec![post, ghost], ..Default::default() };
        s.root.instances = vec![Instance { def: 0, xf: Xform::IDENTITY }, Instance { def: 1, xf: Xform::translation([5.0, 0.0, 0.0]) }];
        let drawn = s.flatten();
        assert_eq!(drawn.faces.len(), 18, "three posts, no ghost");
        let w = written(&s);
        assert_eq!(w.left_out, vec!["Dynamic Component attributes (2 copies written as they are drawn now)".to_string()]);
        assert_same_faces(&drawn, &read_back(&w).scene.flatten());
    }

    #[test]
    fn lengths_are_in_the_units_asked_for() {
        let m = cube(0.0254);
        for (units, unit) in [
            (Units::Millimetres, "(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))"),
            (Units::Centimetres, "(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.CENTI.,.METRE.))"),
            (Units::Metres, "(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.))"),
            (Units::Inches, "(CONVERSION_BASED_UNIT('INCH',#"),
            (Units::FeetInches, "(CONVERSION_BASED_UNIT('INCH',#"),
        ] {
            let w = write(&Scene::from_mesh(m.clone()), &WriteOptions { units, name: "cube.step".into() }).unwrap();
            check_structure(&w.text);
            assert!(w.text.contains(unit), "{units:?}");
            assert!(w.text.contains("FILE_NAME('cube.step',"));
            assert!(w.text.contains("=PRODUCT('cube','cube','Daedalus model',"));
            let back = read_back(&w);
            assert_eq!(back.units, Some(if units == Units::FeetInches { Units::Inches } else { units }));
            assert_same_faces(&m, &back.scene.root.mesh);
        }
        let inches = write(&Scene::from_mesh(m.clone()), &WriteOptions { units: Units::Inches, name: String::new() }).unwrap().text;
        assert!(inches.contains("LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#"));
        assert!(inches.contains("CARTESIAN_POINT('',(1.,1.,1.))"), "an inch is 1.");
        let mm = write_scene(&Scene::from_mesh(m)).unwrap();
        assert!(String::from_utf8(mm).unwrap().contains("CARTESIAN_POINT('',(25.4,25.4,25.4))"));
    }

    #[test]
    fn a_face_not_flat_is_written_as_its_triangles() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.2], [0.0, 1.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        let w = written(&Scene::from_mesh(m));
        assert_eq!(w.faces, 2);
        assert_eq!(w.left_out, vec!["1 face not flat (written as triangles)".to_string()]);
        let back = read_back(&w).scene.root.mesh;
        assert_eq!(back.faces.len(), 2);
        assert_eq!(back.seams.len(), 1, "the edge between the two triangles is soft: it still looks one face");
    }

    /// Loose lines or soft edges as pairs of corner positions, in an order that survives a write and a read.
    fn pairs(m: &Mesh, list: &[[u32; 2]]) -> Vec<[[i64; 3]; 2]> {
        let mut out: Vec<[[i64; 3]; 2]> = list
            .iter()
            .map(|&[a, b]| {
                let (p, q) = (key(m.positions[a as usize]), key(m.positions[b as usize]));
                if p <= q { [p, q] } else { [q, p] }
            })
            .collect();
        out.sort_unstable();
        out
    }

    #[test]
    fn loose_lines_and_soft_edges_come_back() {
        let mut m = cube(1.0);
        // a chain of three lines from a corner of the cube, a ring of three, three lines meeting at a point, one of no length
        let chain = [[2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [3.0, 1.0, 0.0]].map(|p| m.add_vertex(p));
        m.lines.extend([[0, chain[0]], [chain[0], chain[1]], [chain[1], chain[2]]]);
        let ring = [[5.0, 0.0, 0.0], [6.0, 0.0, 0.0], [5.0, 1.0, 0.0]].map(|p| m.add_vertex(p));
        m.lines.extend([[ring[0], ring[1]], [ring[1], ring[2]], [ring[2], ring[0]]]);
        let hub = m.add_vertex([8.0, 0.0, 0.0]);
        let spokes = [[9.0, 0.0, 0.0], [8.0, 1.0, 0.0], [8.0, 0.0, 1.0]].map(|p| m.add_vertex(p));
        m.lines.extend(spokes.map(|s| [hub, s]));
        m.lines.push([hub, hub]);
        // two edges of the cube drawn smooth
        m.seams = vec![[0, 1], [5, 6]];
        let w = written(&Scene::from_mesh(m.clone()));
        assert_eq!(w.left_out, vec!["1 loose line with no length".to_string()]);
        let t = &w.text;
        assert_eq!((n(t, "GEOMETRICALLY_BOUNDED_WIREFRAME_SHAPE_REPRESENTATION"), n(t, "GEOMETRIC_CURVE_SET"), n(t, "POLYLINE")), (1, 1, 5), "a chain, a ring, three spokes");
        let layer = t.lines().find(|l| l.contains("=PRESENTATION_LAYER_ASSIGNMENT('Soft edges',")).expect("the soft edges' layer");
        assert_eq!(layer.matches('#').count(), 3, "its own number and the two edges: {layer}");
        assert_eq!(n(t, "INVISIBILITY"), 1);
        let back = read_back(&w).scene.root.mesh;
        m.lines.pop();
        assert_eq!(pairs(&back, &back.lines), pairs(&m, &m.lines));
        assert_eq!(pairs(&back, &back.seams), pairs(&m, &m.seams));
        assert_eq!(back.positions.len(), 8 + 3 + 3 + 4, "a line meeting the cube shares its corner");
        assert_same_faces(&m, &back);
    }

    #[test]
    fn a_scaled_copy_keeps_its_soft_edges_and_lines() {
        let mut m = cube(1.0);
        m.seams = vec![[0, 1], [5, 6], [2, 6]];
        let v = m.add_vertex([2.0, 0.0, 0.0]);
        m.lines.push([1, v]);
        let mut s = Scene::default();
        s.defs.push(def("Box", true, m, Vec::new()));
        s.root.instances.push(Instance { def: 0, xf: Xform::from_fn(|p| [2.0 * p[0], p[1], 3.0 * p[2]]) });
        s.root.instances.push(Instance { def: 0, xf: Xform::from_fn(|p| [-p[0], p[1], p[2]]) });
        let w = written(&s);
        let back = read_back(&w).scene.flatten();
        let flat = s.flatten();
        assert_eq!(pairs(&back, &back.seams), pairs(&flat, &flat.seams));
        assert_eq!(pairs(&back, &back.lines), pairs(&flat, &flat.lines));
    }

    #[test]
    fn wireframe_and_hidden_edges_from_another_program_read() {
        // a square face whose edge along x is in a layer made invisible, a line trimmed by parameters, a whole circle, a
        // polyline, and a point
        let text = format!(
            "{HEAD}#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#2=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))REPRESENTATION_CONTEXT('',''));
#3=MANIFOLD_SURFACE_SHAPE_REPRESENTATION('',(#4),#2);
#4=SHELL_BASED_SURFACE_MODEL('',(#5));
#5=OPEN_SHELL('',(#10));
#10=ADVANCED_FACE('',(#11),#12,.T.);
#11=FACE_OUTER_BOUND('',#13,.T.);
#12=PLANE('',#14);
#13=EDGE_LOOP('',(#20,#21,#22,#23));
#14=AXIS2_PLACEMENT_3D('',#40,$,$);
#20=ORIENTED_EDGE('',*,*,#30,.T.);
#21=ORIENTED_EDGE('',*,*,#31,.T.);
#22=ORIENTED_EDGE('',*,*,#32,.T.);
#23=ORIENTED_EDGE('',*,*,#33,.T.);
#30=EDGE_CURVE('',#50,#51,#60,.T.);
#31=EDGE_CURVE('',#51,#52,#60,.T.);
#32=EDGE_CURVE('',#52,#53,#60,.T.);
#33=EDGE_CURVE('',#53,#50,#60,.T.);
#40=CARTESIAN_POINT('',(0.,0.,0.));
#41=CARTESIAN_POINT('',(10.,0.,0.));
#42=CARTESIAN_POINT('',(10.,10.,0.));
#43=CARTESIAN_POINT('',(0.,10.,0.));
#50=VERTEX_POINT('',#40);
#51=VERTEX_POINT('',#41);
#52=VERTEX_POINT('',#42);
#53=VERTEX_POINT('',#43);
#60=LINE('',#40,#61);
#61=VECTOR('',#62,1.);
#62=DIRECTION('',(1.,0.,0.));
#70=PRESENTATION_LAYER_ASSIGNMENT('construction','',(#30));
#71=INVISIBILITY((#70));
#80=GEOMETRICALLY_BOUNDED_WIREFRAME_SHAPE_REPRESENTATION('',(#81),#2);
#81=GEOMETRIC_SET('',(#82,#84,#86,#43));
#82=TRIMMED_CURVE('',#60,(PARAMETER_VALUE(20.)),(PARAMETER_VALUE(25.)),.T.,.PARAMETER.);
#84=CIRCLE('',#85,2.);
#85=AXIS2_PLACEMENT_3D('',#42,$,$);
#86=POLYLINE('',(#41,#87,#88));
#87=CARTESIAN_POINT('',(20.,10.,0.));
#88=CARTESIAN_POINT('',(20.,20.,0.));
{TAIL}"
        );
        let r = read(text.as_bytes()).unwrap();
        assert!(r.skipped.is_empty(), "{:?}", r.skipped);
        let m = &r.scene.root.mesh;
        assert_eq!(m.faces.len(), 1);
        assert_eq!(pairs(m, &m.seams), vec![[key([0.0; 3]), key([0.01, 0.0, 0.0])]], "the hidden edge is soft");
        // 1 line + 24 round the circle + 2 of the polyline
        assert_eq!(m.lines.len(), 1 + 24 + 2);
        let line = pairs(m, &m.lines[..1]);
        assert_eq!(line, vec![[key([0.02, 0.0, 0.0]), key([0.025, 0.0, 0.0])]]);
        assert!(m.lines[1..25].iter().flatten().all(|&v| (length(sub(m.positions[v as usize], [0.01, 0.01, 0.0])) - 0.002).abs() < 1e-12), "round the circle");
    }

    #[test]
    fn a_face_listing_a_hole_first_is_written_the_right_way_round() {
        // as some converted faces are: a small square given as the outline, the big one round it as the "hole"
        let mut m = Mesh::new();
        for p in [[4.0, 4.0, 0.0], [5.0, 4.0, 0.0], [5.0, 5.0, 0.0], [4.0, 5.0, 0.0], [0.0, 0.0, 0.0], [0.0, 9.0, 0.0], [9.0, 9.0, 0.0], [9.0, 0.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        m.set_holes(0, vec![vec![4, 5, 6, 7]]);
        let w = written(&Scene::from_mesh(m));
        // the big square, clockwise from above, is the outline: the face faces down, the small square its hole
        assert!(w.text.contains("DIRECTION('',(0.,0.,-1.))"));
        let back = read_back(&w).scene.root.mesh;
        let mut want = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [0.0, 9.0, 0.0], [9.0, 9.0, 0.0], [9.0, 0.0, 0.0], [4.0, 4.0, 0.0], [5.0, 4.0, 0.0], [5.0, 5.0, 0.0], [4.0, 5.0, 0.0]] {
            want.add_vertex(p);
        }
        want.add_face(vec![0, 1, 2, 3]);
        want.set_holes(0, vec![vec![4, 5, 6, 7]]);
        assert_same_faces(&want, &back);
    }

    #[test]
    fn a_face_a_few_micrometres_off_flat_stays_one_face() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 5e-6], [0.0, 1.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        let w = written(&Scene::from_mesh(m.clone()));
        assert!(w.left_out.is_empty(), "{:?}", w.left_out);
        assert_same_faces(&m, &read_back(&w).scene.root.mesh);
    }

    #[test]
    fn what_step_cannot_hold_is_listed() {
        let mut m = cube(1.0);
        let v = m.add_vertex([3.0, 0.0, 0.0]);
        m.lines.push([0, v]);
        m.seams.push([0, 1]);
        m.set_color(0, Some(Paint { rgb: [10, 20, 30], texture: Some(0) }));
        let mut s = Scene::from_mesh(m);
        s.notes.guides.push(Guide::Point([1.0, 2.0, 3.0]));
        s.textures.push(Texture { name: "Brick".into(), path: "/brick.png".into(), size: [1.0, 1.0] });
        s.defs.push(def("Spare", true, cube(1.0), Vec::new()));
        let w = written(&s);
        assert_eq!(w.left_out, vec!["1 guide", "1 texture (the faces keep their colours)", "1 group or component placed nowhere"]);
        assert_eq!(w.products, 1, "the spare component is not written");
        let back = read_back(&w).scene.root.mesh;
        assert_eq!(back.color_of(0).map(|c| c.rgb), Some([10, 20, 30]), "a textured face keeps its colour");
        assert_eq!((back.lines.len(), back.seams.len()), (1, 1), "the loose line and the soft edge are kept");
    }

    #[test]
    fn numbers_and_text_read_back_as_written() {
        assert_eq!([real(1.0), real(-0.25), real(0.0), real(-0.0), real(100.0)], ["1.", "-0.25", "0.", "0.", "100."]);
        assert_eq!([real(1.5e-12), real(1e-12), real(2.5e20)], ["1.5E-12", "1.E-12", "2.5E20"]);
        for x in [1.0, -0.25, 0.1 + 0.2, 1.0 / 3.0, 1.5e-12, 1e-300, 6.02e23, -123456.789, 25.4] {
            let t = real(x);
            assert_eq!(Lexer { s: t.as_bytes(), i: 0 }.param(0).unwrap(), P::Real(x), "{t}");
        }
        for t in ["plain", "it's", "back\\slash", "Tür", "€ and 😀", "a line\nbreak", ""] {
            let s = string(t);
            assert!(s.bytes().all(|b| (0x20..0x7f).contains(&b)), "{s}");
            assert_eq!(Lexer { s: s.as_bytes(), i: 0 }.param(0).unwrap(), P::Str(t.into()));
        }
        assert_eq!(string("Tür €😀"), "'T\\X2\\00FC\\X0\\r \\X2\\20AC\\X0\\\\X4\\0001F600\\X0\\'");
        // as other programs write it: \X\ and \S\ for ISO 8859-1, and plain UTF-8
        let other = "'caf\\X\\E9 caf\\S\\i caf\u{e9}'";
        assert_eq!(Lexer { s: other.as_bytes(), i: 0 }.param(0).unwrap(), P::Str("café café café".into()));
        assert_eq!(Lexer { s: b"LENGTH_MEASURE(1.E-07)", i: 0 }.param(0).unwrap(), P::Typed("LENGTH_MEASURE".into(), Box::new(P::Real(1e-7))));
    }

    #[test]
    fn dates_count_from_1970() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(10_957), (2000, 1, 1));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(civil(20_765), (2026, 11, 8));
    }

    #[test]
    fn step_files_save_and_open_by_their_ending() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("daedalus-step-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Scene::from_mesh(cube(1.0));
        s.defs.push(def("Box", true, cube(0.5), Vec::new()));
        s.root.instances.push(Instance { def: 0, xf: Xform::translation([2.0, 0.0, 0.0]) });
        for name in ["model.step", "model.STP"] {
            let path = dir.join(name);
            crate::save_scene(&s, &path).unwrap();
            let back = crate::open_scene(&path).unwrap();
            assert_eq!(back.defs.len(), 1);
            assert_same_faces(&s.flatten(), &back.flatten());
            assert_eq!(crate::import_mesh(&path).unwrap().faces.len(), 12);
            crate::export_mesh(&cube(2.0), &path).unwrap();
            assert_eq!(crate::open_scene(&path).unwrap().root.mesh.faces.len(), 6);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    const HEAD: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n";
    const TAIL: &str = "ENDSEC;\nEND-ISO-10303-21;\n";

    /// A tetrahedron (an inch along each axis) placed twice in an assembly, written in other programs' ways: inches, an
    /// AP203 schema, a FACE_SURFACE, a bound reversed, a plane facing in with same_sense false, a loop the wrong way round, a
    /// polyline edge stored the other way, a usage naming its shapes the other way round, a colour on the solid and another
    /// over-riding it on one face, comments and an entity over two lines.
    const FOREIGN: &str = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('a part from elsewhere'),'2;1');
FILE_NAME('tri.stp','2020-01-01T00:00:00',('someone'),(''),'other','other','');
FILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));
ENDSEC;
DATA;
/* lengths in inches */
#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#2=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#1);
#3=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);
#4=(CONVERSION_BASED_UNIT('INCH',#2)LENGTH_UNIT()NAMED_UNIT(#3));
#5=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));
#6=(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT());
#7=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-06),#4,'distance_accuracy_value','');
#8=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#7))
   GLOBAL_UNIT_ASSIGNED_CONTEXT((#4,#5,#6))REPRESENTATION_CONTEXT('',''));
#10=APPLICATION_CONTEXT('configuration controlled 3d designs of mechanical parts and assemblies');
#11=PRODUCT_CONTEXT('',#10,'mechanical');
#12=PRODUCT_DEFINITION_CONTEXT('part definition',#10,'design');
#13=PRODUCT('ASM-001','Assembly','',(#11));
#14=PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE('','',#13,.NOT_KNOWN.);
#15=PRODUCT_DEFINITION('design','',#14,#12);
#16=PRODUCT_DEFINITION_SHAPE('','',#15);
#17=SHAPE_DEFINITION_REPRESENTATION(#16,#18);
#18=SHAPE_REPRESENTATION('',(#19,#20,#21),#8);
#19=AXIS2_PLACEMENT_3D('',#22,$,$);
#20=AXIS2_PLACEMENT_3D('',#22,$,$);
#21=AXIS2_PLACEMENT_3D('',#23,#24,#25);
#22=CARTESIAN_POINT('',(0.,0.,0.));
#23=CARTESIAN_POINT('',(10.,0.,0.));
#24=DIRECTION('',(0.,0.,1.));
#25=DIRECTION('',(0.,1.,0.));
#30=PRODUCT('TRI-7','','',(#11));
#31=PRODUCT_DEFINITION_FORMATION('','',#30);
#32=PRODUCT_DEFINITION('design','',#31,#12);
#33=PRODUCT_DEFINITION_SHAPE('','',#32);
#34=SHAPE_DEFINITION_REPRESENTATION(#33,#35);
#35=SHAPE_REPRESENTATION('',(#36),#8);
#36=AXIS2_PLACEMENT_3D('',#22,$,$);
#37=SHAPE_REPRESENTATION_RELATIONSHIP('','',#38,#35);
#38=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#40,#36),#8);
/* placed twice; the first usage names its two shapes the other way round */
#50=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#15,#32,$);
#51=PRODUCT_DEFINITION_SHAPE('','',#50);
#52=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#53,#51);
#53=(REPRESENTATION_RELATIONSHIP('','',#18,#35)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#54)SHAPE_REPRESENTATION_RELATIONSHIP());
#54=ITEM_DEFINED_TRANSFORMATION('','',#20,#36);
#55=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#15,#32,$);
#56=PRODUCT_DEFINITION_SHAPE('','',#55);
#57=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#58,#56);
#58=(REPRESENTATION_RELATIONSHIP('','',#35,#18)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#59)SHAPE_REPRESENTATION_RELATIONSHIP());
#59=ITEM_DEFINED_TRANSFORMATION('','',#36,#21);
#40=MANIFOLD_SOLID_BREP('tri',#41);
#41=CLOSED_SHELL('',(#200,#210,#220,#230));
#60=VERTEX_POINT('',#22);
#61=VERTEX_POINT('',#62);
#62=CARTESIAN_POINT('',(1.,0.,0.));
#63=VERTEX_POINT('',#64);
#64=CARTESIAN_POINT('',(0.,1.,0.));
#65=VERTEX_POINT('',#66);
#66=CARTESIAN_POINT('',(0.,0.,1.));
#67=CARTESIAN_POINT('',(0.5,0.5,0.));
#70=DIRECTION('',(1.,0.,0.));
#71=DIRECTION('',(-1.,0.,1.));
#72=DIRECTION('',(0.,-1.,1.));
#80=EDGE_CURVE('',#60,#61,#81,.T.);
#81=LINE('',#22,#82);
#82=VECTOR('',#70,1.);
#83=EDGE_CURVE('',#60,#63,#84,.T.);
#84=LINE('',#22,#85);
#85=VECTOR('',#25,1.);
#86=EDGE_CURVE('',#60,#65,#87,.T.);
#87=LINE('',#22,#88);
#88=VECTOR('',#24,1.);
/* from the third corner to the second, along a polyline that runs the other way */
#89=EDGE_CURVE('',#63,#61,#90,.F.);
#90=POLYLINE('',(#62,#67,#64));
#91=EDGE_CURVE('',#61,#65,#92,.T.);
#92=LINE('',#62,#93);
#93=VECTOR('',#71,1.4142135623730951);
#94=EDGE_CURVE('',#63,#65,#95,.T.);
#95=LINE('',#64,#96);
#96=VECTOR('',#72,1.4142135623730951);
/* the bottom: its plane faces up, the face down; its loop runs counter-clockwise from above and its bound reverses it */
#200=ADVANCED_FACE('',(#201),#203,.F.);
#201=FACE_OUTER_BOUND('',#202,.F.);
#202=EDGE_LOOP('',(#300,#301,#302));
#300=ORIENTED_EDGE('',*,*,#80,.T.);
#301=ORIENTED_EDGE('',*,*,#89,.F.);
#302=ORIENTED_EDGE('',*,*,#83,.F.);
#203=PLANE('',#204);
#204=AXIS2_PLACEMENT_3D('',#22,#24,#70);
/* the front (y = 0): a FACE_SURFACE with a plain FACE_BOUND */
#210=FACE_SURFACE('',(#211),#213,.T.);
#211=FACE_BOUND('',#212,.T.);
#212=EDGE_LOOP('',(#310,#311,#312));
#310=ORIENTED_EDGE('',*,*,#80,.T.);
#311=ORIENTED_EDGE('',*,*,#91,.T.);
#312=ORIENTED_EDGE('',*,*,#86,.F.);
#213=PLANE('',#214);
#214=AXIS2_PLACEMENT_3D('',#22,#215,#70);
#215=DIRECTION('',(0.,-1.,0.));
/* the side (x = 0): its loop runs the wrong way round for its plane */
#220=ADVANCED_FACE('',(#221),#223,.T.);
#221=FACE_OUTER_BOUND('',#222,.T.);
#222=EDGE_LOOP('',(#320,#321,#322));
#320=ORIENTED_EDGE('',*,*,#83,.T.);
#321=ORIENTED_EDGE('',*,*,#94,.T.);
#322=ORIENTED_EDGE('',*,*,#86,.F.);
#223=PLANE('',#224);
#224=AXIS2_PLACEMENT_3D('',#22,#225,#25);
#225=DIRECTION('',(-1.,0.,0.));
/* the slanted face */
#230=ADVANCED_FACE('',(#231),#233,.T.);
#231=FACE_OUTER_BOUND('',#232,.T.);
#232=EDGE_LOOP('',(#330,#331,#332));
#330=ORIENTED_EDGE('',*,*,#89,.F.);
#331=ORIENTED_EDGE('',*,*,#94,.T.);
#332=ORIENTED_EDGE('',*,*,#91,.F.);
#233=PLANE('',#234);
#234=AXIS2_PLACEMENT_3D('',#62,#235,$);
#235=DIRECTION('',(1.,1.,1.));
/* red all over, the slanted face blue */
#120=STYLED_ITEM('',(#121),#40);
#121=PRESENTATION_STYLE_ASSIGNMENT((#122));
#122=SURFACE_STYLE_USAGE(.BOTH.,#123);
#123=SURFACE_SIDE_STYLE('',(#124));
#124=SURFACE_STYLE_FILL_AREA(#125);
#125=FILL_AREA_STYLE('',(#126));
#126=FILL_AREA_STYLE_COLOUR('',#127);
#127=DRAUGHTING_PRE_DEFINED_COLOUR('red');
#130=OVER_RIDING_STYLED_ITEM('',(#131),#230,#120);
#131=PRESENTATION_STYLE_ASSIGNMENT((#132));
#132=SURFACE_STYLE_USAGE(.BOTH.,#133);
#133=SURFACE_SIDE_STYLE('',(#134));
#134=SURFACE_STYLE_RENDERING(.NORMAL_SHADING.,#135);
#135=COLOUR_RGB('',0.,0.,1.);
#136=MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',(#120,#130),#8);
ENDSEC;
END-ISO-10303-21;
"#;

    #[test]
    fn a_flat_faced_assembly_from_another_program_reads() {
        let r = read(FOREIGN.as_bytes()).unwrap();
        assert!(r.skipped.is_empty(), "{:?}", r.skipped);
        assert_eq!(r.notes, vec!["1 face whose loops ran against the plane (turned to match)".to_string()]);
        assert_eq!(r.units, Some(Units::Inches));
        let s = &r.scene;
        assert!(s.root.mesh.faces.is_empty());
        assert_eq!(s.defs.len(), 1);
        assert_eq!((s.defs[0].name.as_str(), s.defs[0].component), ("TRI-7", true), "a product with no name goes by its id");
        // placed as they were: where it was drawn, and turned a quarter about z and moved 10 inches along x
        assert_eq!(s.root.instances.len(), 2);
        assert_same_xf(&s.root.instances[0].xf, &Xform::IDENTITY);
        assert_same_xf(&s.root.instances[1].xf, &Xform([[0.0, -1.0, 0.0, 0.254], [1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]]));
        let inch = 0.0254;
        let red = Some([255, 0, 0]);
        let mut tri = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [inch, 0.0, 0.0], [0.0, inch, 0.0], [0.0, 0.0, inch], [inch / 2.0, inch / 2.0, 0.0]] {
            tri.add_vertex(p);
        }
        // the polyline's middle corner is on the bottom and on the slanted face
        for (f, c) in [(vec![2, 4, 1, 0], red), (vec![0, 1, 3], red), (vec![3, 2, 0], red), (vec![1, 4, 2, 3], Some([0, 0, 255]))] {
            tri.add_face(f);
            let k = tri.faces.len() - 1;
            tri.set_color(k, c.map(Paint::from));
        }
        assert_same_faces(&tri, &s.defs[0].ctx.mesh);
        assert!((daedalus_core::inspect::signed_volume(&s.defs[0].ctx.mesh) - inch.powi(3) / 6.0).abs() < 1e-15, "facing out");
        assert_eq!(read_scene(FOREIGN.as_bytes()).unwrap().flatten().faces.len(), 8);
    }

    #[test]
    fn curved_faces_are_named_not_dropped() {
        let text = format!(
            "{HEAD}#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#2=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))REPRESENTATION_CONTEXT('',''));
#3=MANIFOLD_SURFACE_SHAPE_REPRESENTATION('',(#4),#2);
#4=SHELL_BASED_SURFACE_MODEL('',(#5));
#5=OPEN_SHELL('',(#10,#20,#30,#40));
#10=ADVANCED_FACE('',(#11),#12,.T.);
#11=FACE_OUTER_BOUND('',#13,.T.);
#12=PLANE('',#14);
#13=POLY_LOOP('',(#50,#51,#52,#53));
#14=AXIS2_PLACEMENT_3D('',#50,$,$);
#20=ADVANCED_FACE('',(#21),#22,.T.);
#21=FACE_OUTER_BOUND('',#13,.T.);
#22=CYLINDRICAL_SURFACE('',#14,5.);
#30=ADVANCED_FACE('',(#21),#31,.T.);
#31=(BOUNDED_SURFACE()B_SPLINE_SURFACE(1,1,((#50,#51),(#53,#52)),.UNSPECIFIED.,.F.,.F.,.F.)B_SPLINE_SURFACE_WITH_KNOTS((2,2),(2,2),(0.,1.),(0.,1.),.UNSPECIFIED.)GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_SURFACE(((1.,1.),(1.,1.)))REPRESENTATION_ITEM('')SURFACE());
#40=ADVANCED_FACE('',(#21),#41,.T.);
#41=CYLINDRICAL_SURFACE('',#14,7.);
#50=CARTESIAN_POINT('',(0.,0.,0.));
#51=CARTESIAN_POINT('',(10.,0.,0.));
#52=CARTESIAN_POINT('',(10.,10.,0.));
#53=CARTESIAN_POINT('',(0.,10.,0.));
{TAIL}"
        );
        let r = read(text.as_bytes()).unwrap();
        assert_eq!(r.scene.root.mesh.faces.len(), 1, "the flat face is read");
        assert_eq!(r.skipped, vec!["2 faces on CYLINDRICAL_SURFACE".to_string(), "1 face on RATIONAL_B_SPLINE_SURFACE".to_string()]);
        let e = read_scene(text.as_bytes()).unwrap_err().to_string();
        assert!(e.contains("2 faces on CYLINDRICAL_SURFACE") && e.contains("1 face on RATIONAL_B_SPLINE_SURFACE"), "{e}");
    }

    #[test]
    fn circles_and_arcs_come_in_segments() {
        // a disc bounded by one whole circle, and a half disc whose arc is written twice: along the circle, and against it
        let text = format!(
            "{HEAD}#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#2=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))REPRESENTATION_CONTEXT('',''));
#3=MANIFOLD_SURFACE_SHAPE_REPRESENTATION('',(#4),#2);
#4=SHELL_BASED_SURFACE_MODEL('',(#5,#6,#7));
#5=OPEN_SHELL('',(#10));
#6=OPEN_SHELL('',(#30));
#7=OPEN_SHELL('',(#40));
#10=ADVANCED_FACE('',(#11),#12,.T.);
#11=FACE_OUTER_BOUND('',#13,.T.);
#12=PLANE('',#14);
#13=EDGE_LOOP('',(#15));
#14=AXIS2_PLACEMENT_3D('',#20,#21,#22);
#15=ORIENTED_EDGE('',*,*,#16,.T.);
#16=EDGE_CURVE('',#17,#17,#18,.T.);
#17=VERTEX_POINT('',#19);
#18=CIRCLE('',#14,5.);
#19=CARTESIAN_POINT('',(15.,10.,0.));
#20=CARTESIAN_POINT('',(10.,10.,0.));
#21=DIRECTION('',(0.,0.,1.));
#22=DIRECTION('',(1.,0.,0.));
#23=VERTEX_POINT('',#24);
#24=CARTESIAN_POINT('',(5.,10.,0.));
#25=EDGE_CURVE('',#23,#17,#26,.T.);
#26=LINE('',#24,#27);
#27=VECTOR('',#22,10.);
#30=ADVANCED_FACE('',(#31),#12,.T.);
#31=FACE_OUTER_BOUND('',#32,.T.);
#32=EDGE_LOOP('',(#33,#34));
#33=ORIENTED_EDGE('',*,*,#35,.T.);
#34=ORIENTED_EDGE('',*,*,#25,.T.);
#35=EDGE_CURVE('',#17,#23,#18,.T.);
#40=ADVANCED_FACE('',(#41),#12,.T.);
#41=FACE_OUTER_BOUND('',#42,.T.);
#42=EDGE_LOOP('',(#43,#44));
#43=ORIENTED_EDGE('',*,*,#45,.F.);
#44=ORIENTED_EDGE('',*,*,#25,.T.);
#45=EDGE_CURVE('',#23,#17,#18,.F.);
{TAIL}"
        );
        let r = read(text.as_bytes()).unwrap();
        assert!(r.skipped.is_empty() && r.notes.is_empty(), "{:?} {:?}", r.skipped, r.notes);
        let m = &r.scene.root.mesh;
        assert_eq!(m.faces.len(), 3);
        let at = |v: u32| m.positions[v as usize];
        let centre = [0.01, 0.01, 0.0];
        assert_eq!(m.faces[0].len(), 24, "a whole circle in 24 segments");
        assert!(m.faces[0].iter().all(|&v| (length(sub(at(v), centre)) - 0.005).abs() < 1e-12));
        assert!(area_normal(&m.positions, &m.faces[0])[2] > 0.0, "counter-clockwise about its plane");
        // half a turn in 12 segments, over the top, the same both ways of writing it
        for f in [1, 2] {
            assert_eq!(m.faces[f].len(), 13);
            assert!(m.faces[f].iter().all(|&v| at(v)[1] >= 0.01 - 1e-12));
            assert!(area_normal(&m.positions, &m.faces[f])[2] > 0.0);
        }
        let (one, other) = (corners(m, &m.faces[1]), corners(m, &m.faces[2]));
        assert!(one.iter().zip(&other).all(|(p, q)| length(sub(*p, *q)) < 1e-15), "{one:?} {other:?}");
    }

    #[test]
    fn mapped_items_become_copies_of_a_component() {
        // a triangle's representation mapped twice into a product's shape: moved 10 mm along x, and doubled in size at y = 10 mm
        let text = format!(
            "{HEAD}#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#2=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))REPRESENTATION_CONTEXT('',''));
#3=PRODUCT('P','Plate','',());
#4=PRODUCT_DEFINITION_FORMATION('','',#3);
#5=PRODUCT_DEFINITION('design','',#4,$);
#6=PRODUCT_DEFINITION_SHAPE('','',#5);
#7=SHAPE_DEFINITION_REPRESENTATION(#6,#8);
#8=SHAPE_REPRESENTATION('',(#9,#10),#2);
#9=MAPPED_ITEM('',#11,#13);
#10=MAPPED_ITEM('',#11,#20);
#11=REPRESENTATION_MAP(#12,#30);
#12=AXIS2_PLACEMENT_3D('',#40,$,$);
#13=AXIS2_PLACEMENT_3D('',#41,$,$);
#20=CARTESIAN_TRANSFORMATION_OPERATOR_3D('','','',$,$,#42,2.,$);
#30=MANIFOLD_SURFACE_SHAPE_REPRESENTATION('Tile',(#31),#2);
#31=SHELL_BASED_SURFACE_MODEL('',(#32));
#32=OPEN_SHELL('',(#33));
#33=FACE_SURFACE('',(#34),#35,.T.);
#34=FACE_OUTER_BOUND('',#36,.T.);
#35=PLANE('',#12);
#36=POLY_LOOP('',(#40,#43,#44));
#40=CARTESIAN_POINT('',(0.,0.,0.));
#41=CARTESIAN_POINT('',(10.,0.,0.));
#42=CARTESIAN_POINT('',(0.,10.,0.));
#43=CARTESIAN_POINT('',(1.,0.,0.));
#44=CARTESIAN_POINT('',(0.,1.,0.));
{TAIL}"
        );
        let r = read(text.as_bytes()).unwrap();
        assert!(r.skipped.is_empty() && r.notes.is_empty(), "{:?} {:?}", r.skipped, r.notes);
        let s = &r.scene;
        assert_eq!(s.defs.len(), 1, "the mapped representation, read once");
        assert_eq!((s.defs[0].name.as_str(), s.defs[0].component, s.defs[0].ctx.mesh.faces.len()), ("Tile", true, 1));
        assert_eq!(s.root.instances.len(), 2);
        assert_same_xf(&s.root.instances[0].xf, &Xform::translation([0.01, 0.0, 0.0]));
        assert_same_xf(&s.root.instances[1].xf, &Xform([[2.0, 0.0, 0.0, 0.0], [0.0, 2.0, 0.0, 0.01], [0.0, 0.0, 2.0, 0.0]]));
    }

    #[test]
    fn broken_files_are_refused_saying_where() {
        let e = |t: &str| read(t.as_bytes()).unwrap_err().to_string();
        assert!(e("solid x\n").contains("not a STEP file"));
        let point = "#1=CARTESIAN_POINT('',(0.,0.,0.));\n";
        assert!(e(&format!("{HEAD}{point}{point}{TAIL}")).contains("line 9: #1 is defined twice"));
        assert!(e(&format!("{HEAD}#1=CARTESIAN_POINT('',(0.,0.,0.);\n{TAIL}")).contains("line 8"));
        assert!(e(&format!("{HEAD}#1=PRODUCT('open);\n")).contains("not closed"));
        assert!(e(&format!("{HEAD}{point}")).contains("ENDSEC"));
        // two products each holding the other
        let products = "#1=PRODUCT('a','a','',());\n#2=PRODUCT_DEFINITION_FORMATION('','',#1);\n#3=PRODUCT_DEFINITION('','',#2,$);\n\
#4=PRODUCT('b','b','',());\n#5=PRODUCT_DEFINITION_FORMATION('','',#4);\n#6=PRODUCT_DEFINITION('','',#5,$);\n\
#7=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#3,#6,$);\n#8=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#6,#3,$);\n";
        assert!(e(&format!("{HEAD}{products}{TAIL}")).contains("holds itself"));
    }
}
