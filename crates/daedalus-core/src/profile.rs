//! Flat outlines made into faces, and faces made into solids: what 3D Text is built from (and any other profile given as
//! closed outlines, such as an imported drawing later).
//!
//! Outlines may nest: a letter's outside, the holes in it (the inside of an O), an island in a hole. What lies inside an
//! odd number of outlines is solid; the rest is open.

use crate::halfedge::{closing, line_tolerance, Closing};
use crate::imprint::{crossing, cut_hole, len, on_segment, sub};
use crate::sketch::Plane;
use crate::{HalfEdgeMesh, Mesh, TopologyError};
use std::collections::HashSet;

/// Whether `p` lies inside the closed outline `ring` (even-odd).
pub fn inside(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut c = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                c = !c;
            }
        }
    }
    c
}

/// Faces for the solid parts of `outlines` (closed, each listed once round, any winding), laid on `plane`, all facing the
/// plane's normal. A face with a hole (the inside of an O) is one face with that hole, as the Line tool makes it; so is an
/// opening that an outline reaches along a bridge of lines, out and back the same way (as some SketchUp faces come).
///
/// The outlines are drawn as the Line tool would draw them line by line ([`faces_line_by_line`] does exactly that), with the
/// same result, but in one pass: a logo of over 5,000 lines in a SketchUp model took over 6 s line by line.
pub fn faces(outlines: &[Vec<[f64; 2]>], plane: &Plane) -> Result<Mesh, TopologyError> {
    let (rings, order) = in_order(outlines);
    let (drawn, _) = draw(&order, plane)?;
    Ok(cut_out(drawn, &rings, plane))
}

/// [`faces`] the plain way: every line of every outline drawn with `HalfEdgeMesh::add_line` into one half-edge mesh. Each
/// line is checked against the whole drawing, so the time grows with the square of the lines; kept as the reference
/// `faces` must match.
pub fn faces_line_by_line(outlines: &[Vec<[f64; 2]>], plane: &Plane) -> Result<Mesh, TopologyError> {
    let (rings, order) = in_order(outlines);
    let mut h = HalfEdgeMesh::default();
    for r in &order {
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            h.add_line(plane.to_world(a[0], a[1]), plane.to_world(b[0], b[1]))?;
        }
    }
    Ok(cut_out(h.to_mesh(), &rings, plane))
}

type Ring<'a> = &'a Vec<[f64; 2]>;

/// The outlines worth drawing (three corners or more), and the order to draw them in: by how many other outlines hold each
/// one, outsides first, then holes, then islands, so each is drawn into what holds it.
fn in_order(outlines: &[Vec<[f64; 2]>]) -> (Vec<Ring<'_>>, Vec<Ring<'_>>) {
    let rings: Vec<&Vec<[f64; 2]>> = outlines.iter().filter(|r| r.len() >= 3).collect();
    let depth = |r: &Vec<[f64; 2]>| rings.iter().filter(|o| !std::ptr::eq(**o, r) && inside(o, r[0])).count();
    let mut order: Vec<(usize, &Vec<[f64; 2]>)> = rings.iter().map(|r| (depth(r), *r)).collect();
    order.sort_by_key(|(d, _)| *d);
    let order = order.into_iter().map(|(_, r)| r).collect();
    (rings, order)
}

/// What is left of the drawn outlines (`m`) once the faces inside an even number of outlines (holes) are taken out, every
/// face turned toward the plane's normal, and the loose lines dropped. A face drawn round faces already there gets its
/// holes there first ([`uncover`]).
fn cut_out(m: Mesh, rings: &[Ring<'_>], plane: &Plane) -> Mesh {
    let mut m = m;
    let tol = m.tolerance();
    let mut inner: Vec<Option<[f64; 2]>> = (0..m.faces.len()).map(|f| point_inside(&m, f, plane, tol)).collect();
    for f in uncover(&mut m, plane, &inner) {
        inner[f] = point_inside(&m, f, plane, tol);
    }
    let even = |q: [f64; 2]| rings.iter().filter(|r| inside(r, q)).count() % 2 == 0;
    let open: Vec<usize> = (0..m.faces.len()).filter(|&f| inner[f].is_some_and(even)).collect();
    for &f in open.iter().rev() {
        m.remove_face(f);
    }
    m.lines.clear();
    // every face toward the plane's normal
    for f in 0..m.faces.len() {
        let n = newell(&m.positions, &m.faces[f]);
        if n[0] * plane.normal[0] + n[1] * plane.normal[1] + n[2] * plane.normal[2] < 0.0 {
            m.faces[f].reverse();
            let holes: Vec<Vec<u32>> = m.holes_of(f).iter().map(|h| h.iter().rev().copied().collect()).collect();
            m.set_holes(f, holes);
        }
    }
    let edges: HashSet<(u32, u32)> = m.edges().into_iter().collect();
    m.seams.retain(|&[a, b]| edges.contains(&(a.min(b), a.max(b))));
    m.compact();
    m
}

/// A point inside face `f`, in the plane: the middle of its first triangle that is not a sliver (its middle at least ten
/// times `tol` from its sides), else of its first (None if it has none). A sliver along a straight edge (corners in a row, a
/// hair off straight on a plane far from the origin) has its middle on the face's edge, and a whole face was taken for open.
fn point_inside(m: &Mesh, f: usize, plane: &Plane, tol: f64) -> Option<[f64; 2]> {
    let tri = m.triangulate_face(f);
    let clear = |t: &&[u32; 3]| {
        let [a, b, c] = t.map(|v| m.positions[v as usize]);
        let (u, w) = (sub(b, a), sub(c, a));
        let twice = len([u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]]);
        // the middle is a third of the shortest height from the nearest side
        twice / (3.0 * len(u).max(len(w)).max(len(sub(c, b)))) > 10.0 * tol
    };
    let t = tri.iter().find(clear).or(tri.first())?;
    let c = t.iter().fold([0.0f64; 3], |s, &v| {
        let p = m.positions[v as usize];
        [s[0] + p[0] / 3.0, s[1] + p[1] / 3.0, s[2] + p[2] / 3.0]
    });
    Some(plane.to_local(c))
}

/// Faces that an outline closed round after they were drawn, covering them with no hole there. An outline that runs out
/// along a bridge of lines to an opening and back the same way (some SketchUp faces come so) closes the opening's loop
/// first, as a face of its own, and its own loop later; drawn line by line, the outline then filled the opening in.
///
/// A face lies on another when a point inside it (`inner`) and all its corners are in that face's area (inside its outline,
/// in none of its holes) and none of its corners is that face's. Each face lying on another, and not inside the outline of
/// another face lying on the same one (an island in an opening goes with the opening), becomes a hole in the smallest face
/// it lies on, as the Line tool cuts a hole round an outline drawn inside a face (`halfedge::closing`); holes of that face
/// inside it go with it. Returns the faces whose `inner` point lies in one of their new holes.
fn uncover(m: &mut Mesh, plane: &Plane, inner: &[Option<[f64; 2]>]) -> Vec<usize> {
    let n = m.faces.len();
    let flat: Vec<[f64; 2]> = m.positions.iter().map(|&p| plane.to_local(p)).collect();
    let ring = |r: &[u32]| -> Vec<[f64; 2]> { r.iter().map(|&v| flat[v as usize]).collect() };
    let outlines: Vec<Vec<[f64; 2]>> = m.faces.iter().map(|r| ring(r)).collect();
    let holes: Vec<Vec<Vec<[f64; 2]>>> = (0..n).map(|f| m.holes_of(f).iter().map(|h| ring(h)).collect()).collect();
    let boxes: Vec<[f64; 4]> = outlines
        .iter()
        .map(|r| r.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]))
        .collect();
    // in face g's own area
    let on = |g: usize, q: [f64; 2]| {
        let b = boxes[g];
        q[0] >= b[0] && q[0] <= b[2] && q[1] >= b[1] && q[1] <= b[3] && inside(&outlines[g], q) && !holes[g].iter().any(|h| inside(h, q))
    };
    let size = |r: &[[f64; 2]]| (0..r.len()).map(|i| r[i][0] * r[(i + 1) % r.len()][1] - r[(i + 1) % r.len()][0] * r[i][1]).sum::<f64>().abs();
    // the smallest face each face lies on
    let mut corners: Vec<Option<HashSet<u32>>> = vec![None; n];
    let mut under: Vec<Option<usize>> = vec![None; n];
    for f in 0..n {
        let Some(q) = inner[f] else { continue };
        let mut best: Option<(f64, usize)> = None;
        for g in (0..n).filter(|&g| g != f) {
            if !on(g, q) {
                continue;
            }
            let theirs = corners[g].get_or_insert_with(|| m.loops(g).flatten().copied().collect());
            if m.faces[f].iter().any(|v| theirs.contains(v)) || !m.faces[f].iter().all(|&v| on(g, flat[v as usize])) {
                continue;
            }
            let s = size(&outlines[g]);
            if best.is_none_or(|(b, _)| s < b) {
                best = Some((s, g));
            }
        }
        under[f] = best.map(|(_, g)| g);
    }
    // (face, the face it lies on, the point inside it)
    let lying: Vec<(usize, usize, [f64; 2])> = (0..n).filter_map(|f| Some((f, under[f]?, inner[f]?))).collect();
    let mut cut: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(f, g, q) in &lying {
        if !lying.iter().any(|&(h, on_g, _)| h != f && on_g == g && inside(&outlines[h], q)) {
            cut[g].push(f);
        }
    }
    let mut moved = Vec::new();
    for g in 0..n {
        if cut[g].is_empty() {
            continue;
        }
        let up = newell(&m.positions, &m.faces[g]);
        let within = |p: [f64; 2]| cut[g].iter().any(|&f| inside(&outlines[f], p));
        let mut hs: Vec<Vec<u32>> = m.holes_of(g).iter().filter(|h| !within(flat[h[0] as usize])).cloned().collect();
        for &f in &cut[g] {
            // wound against the face's outline
            let r = &m.faces[f];
            let along = newell(&m.positions, r);
            let same = along[0] * up[0] + along[1] * up[1] + along[2] * up[2] > 0.0;
            hs.push(if same { r.iter().rev().copied().collect() } else { r.clone() });
        }
        m.set_holes(g, hs);
        if inner[g].is_some_and(within) {
            moved.push(g);
        }
    }
    moved
}

/// The outlines (in drawing order) drawn line by line, as `HalfEdgeMesh::add_line` draws them, into what
/// `HalfEdgeMesh::to_mesh` then gives; and how many lines and loops were left to `add_line` and `close_loop` themselves.
fn draw(order: &[Ring<'_>], plane: &Plane) -> Result<(Mesh, Counts), TopologyError> {
    draw_within(order, plane, None)
}

/// [`draw`], the grid's budget set (the tests make it small, to see the rest drawn by `add_line`).
fn draw_within(order: &[Ring<'_>], plane: &Plane, budget: Option<usize>) -> Result<(Mesh, Counts), TopologyError> {
    let world: Vec<[f64; 3]> = order.iter().flat_map(|r| r.iter().map(|p| plane.to_world(p[0], p[1]))).collect();
    let mut d = Drawing::new(&world, plane);
    if let Some(b) = budget {
        d.grid.budget = b;
    }
    let mut at = 0;
    for r in order {
        d.start_outline();
        for i in 0..r.len() {
            d.line(world[at + i], world[at + (i + 1) % r.len()])?;
        }
        at += r.len();
    }
    d.finish()
}

/// Lines and loops of a [`draw`] that went through `HalfEdgeMesh::add_line` / `close_loop` (the tests check what is quick).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Counts {
    lines: usize,
    loops: usize,
}

/// What the next line will do, worked out by [`Drawing::quick`]: its two corners (`new`: not there yet), and whether it is
/// drawn (not when it has no length or is there already).
struct Plan {
    ia: u32,
    ib: u32,
    new_a: bool,
    new_b: bool,
    line: bool,
}

/// Outlines drawn into a [`Mesh`] with, line for line, exactly what `HalfEdgeMesh::add_line` makes of them, but without
/// going over the whole drawing for every line. Almost every line of an outline starts where the last one ended and meets
/// nothing else (no corner on it, no edge across it, no face it could split): such a line is put in directly, its ends
/// found in a grid. An outline that closes on its own lines alone becomes a face, or a hole in the face it lies in, by the
/// Line tool's own rule (`halfedge::closing`). Anything else (a line crossing another, a corner on a line, a loop closed
/// through earlier lines or faces) is left to `add_line` and `close_loop` themselves.
struct Drawing<'a> {
    plane: &'a Plane,
    /// the drawing as `HalfEdgeMesh::to_mesh` would give it after the same lines
    m: Mesh,
    /// its corners' bounds (for its tolerance)
    bounds: Option<([f64; 3], [f64; 3])>,
    grid: Grid,
    /// per corner: where it lies in the plane (the grid's coordinates), the loose lines ending there, whether a face's
    /// outline or hole runs through it, and the faces whose outlines do
    flat: Vec<[f64; 2]>,
    lines_at: Vec<u32>,
    in_face: Vec<bool>,
    outline_of: Vec<Vec<u32>>,
    /// every edge (face edges and loose lines) as (low, high), numbered as the grid lists them, and as a set
    edges: Vec<(u32, u32)>,
    edge_set: HashSet<(u32, u32)>,
    /// the outline being drawn: its corners so far, and whether they are all new and joined one after the other by its own
    /// lines alone (the last lines drawn)
    run: Vec<u32>,
    fresh: bool,
    counts: Counts,
    /// the grid outgrown (many long lines across the drawing): the rest is drawn by `add_line` itself
    plain: Option<HalfEdgeMesh>,
}

impl<'a> Drawing<'a> {
    /// A drawing for these points (every corner of every outline, as drawn).
    fn new(points: &[[f64; 3]], plane: &'a Plane) -> Drawing<'a> {
        // no line's tolerance can exceed the tolerance of all the points together (its corners are among them, or where lines
        // cross between them), so the grid finds everything within any line's tolerance by looking a few times that far
        let most = Mesh { positions: points.to_vec(), ..Default::default() }.tolerance();
        let flat: Vec<[f64; 2]> = points.iter().map(|&p| plane.to_local(p)).collect();
        let (lo, hi) = flat.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| {
            ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])])
        });
        let (lo, hi) = if points.is_empty() { ([0.0; 2], [0.0; 2]) } else { (lo, hi) };
        Drawing {
            plane,
            m: Mesh::default(),
            bounds: None,
            grid: Grid::new(lo, hi, points.len(), most * 4.0),
            flat: Vec::new(),
            lines_at: Vec::new(),
            in_face: Vec::new(),
            outline_of: Vec::new(),
            edges: Vec::new(),
            edge_set: HashSet::new(),
            run: Vec::new(),
            fresh: false,
            counts: Counts::default(),
            plain: None,
        }
    }

    fn start_outline(&mut self) {
        self.run.clear();
        self.fresh = true;
    }

    /// The drawing's tolerance now (`Mesh::tolerance`: the two corners of its bounds have the same size and reach as all
    /// its corners).
    fn tolerance(&self) -> f64 {
        let positions = self.bounds.map(|(lo, hi)| vec![lo, hi]).unwrap_or_default();
        Mesh { positions, ..Default::default() }.tolerance()
    }

    /// Draws the line a-b, as `HalfEdgeMesh::add_line(a, b)` would.
    fn line(&mut self, a: [f64; 3], b: [f64; 3]) -> Result<(), TopologyError> {
        if self.plain.is_none() && self.grid.full() {
            self.plain = Some(HalfEdgeMesh::from_mesh(&self.m)?);
        }
        if let Some(h) = &mut self.plain {
            self.counts.lines += 1;
            h.add_line(a, b)?;
            return Ok(());
        }
        let tol = line_tolerance(self.tolerance(), a, b);
        match self.quick(a, b, tol) {
            Some(plan) => self.put(plan, a, b, tol),
            None => {
                // something else on its way: drawn by add_line itself
                self.fresh = false;
                self.counts.lines += 1;
                let mut h = HalfEdgeMesh::from_mesh(&self.m)?;
                h.add_line(a, b)?;
                self.m = h.to_mesh();
                self.rebuild();
                Ok(())
            }
        }
    }

    /// What `imprint::add_segment` would do with the line a-b, if it only joins its ends to corners there (or new ones) and
    /// adds a loose line or nothing; None if more could happen (a corner on it or on an edge, an edge across it, a face it
    /// may split).
    fn quick(&mut self, a: [f64; 3], b: [f64; 3], tol: f64) -> Option<Plan> {
        let n = self.m.positions.len() as u32;
        let (fa, fb) = (self.plane.to_local(a), self.plane.to_local(b));
        // `vertex_at`: the first corner within tol, else a new one (which must not fall on an edge: the edge would break)
        let ia = match self.weld(a, fa, tol) {
            Some(v) => v,
            None if self.on_an_edge(a, fa, tol) => return None,
            None => n,
        };
        let new_a = ia == n;
        let (ib, new_b) = match self.weld(b, fb, tol) {
            Some(v) => (v, false),
            None if new_a && len(sub(a, b)) <= tol => (n, false),
            None if self.on_an_edge(b, fb, tol) => return None,
            None => (n + new_a as u32, true),
        };
        if ia == ib {
            return Some(Plan { ia, ib, new_a, new_b, line: false });
        }
        let (pa, qa) = if new_a { (a, fa) } else { (self.m.positions[ia as usize], self.flat[ia as usize]) };
        let (pb, qb) = if new_b { (b, fb) } else { (self.m.positions[ib as usize], self.flat[ib as usize]) };
        // `add_segment`: no other corner on the line, and no edge across it (but those from its own ends)
        if self.corner_on(pa, pb, qa, qb, [ia, ib], tol) || self.crossed(pa, pb, qa, qb, [ia, ib], tol) {
            return None;
        }
        if !new_a && !new_b {
            if self.edge_set.contains(&(ia.min(ib), ia.max(ib))) {
                return Some(Plan { ia, ib, new_a, new_b, line: false });
            }
            // between two corners of one face's outline it may split that face
            if self.outline_of[ia as usize].iter().any(|f| self.outline_of[ib as usize].contains(f)) {
                return None;
            }
        }
        Some(Plan { ia, ib, new_a, new_b, line: true })
    }

    /// The first corner (by number) within `tol` of `p`, as `imprint::vertex_at` finds it.
    fn weld(&self, p: [f64; 3], q: [f64; 2], tol: f64) -> Option<u32> {
        let mut best: Option<u32> = None;
        self.grid.corners_near(q, q, |v| {
            if best.is_none_or(|b| v < b) && len(sub(self.m.positions[v as usize], p)) <= tol {
                best = Some(v);
            }
            false
        });
        best
    }

    /// Whether a new corner at `p` would lie on an edge, between its ends (`vertex_at` would break that edge there).
    fn on_an_edge(&mut self, p: [f64; 3], q: [f64; 2], tol: f64) -> bool {
        let (edges, positions) = (&self.edges, &self.m.positions);
        self.grid.edges_near(q, q, |e| {
            let (u, v) = edges[e as usize];
            on_segment(p, positions[u as usize], positions[v as usize], tol).is_some_and(|t| t > 0.0 && t < 1.0)
        })
    }

    /// Whether a corner other than `ends` lies on the line pa-pb, between its ends.
    fn corner_on(&self, pa: [f64; 3], pb: [f64; 3], qa: [f64; 2], qb: [f64; 2], ends: [u32; 2], tol: f64) -> bool {
        let (lo, hi) = ([qa[0].min(qb[0]), qa[1].min(qb[1])], [qa[0].max(qb[0]), qa[1].max(qb[1])]);
        self.grid.corners_near(lo, hi, |v| {
            !ends.contains(&v) && on_segment(self.m.positions[v as usize], pa, pb, tol).is_some_and(|t| t > 0.0 && t < 1.0)
        })
    }

    /// Whether an edge not ending at either of `ends` crosses the line pa-pb.
    fn crossed(&mut self, pa: [f64; 3], pb: [f64; 3], qa: [f64; 2], qb: [f64; 2], ends: [u32; 2], tol: f64) -> bool {
        let (lo, hi) = ([qa[0].min(qb[0]), qa[1].min(qb[1])], [qa[0].max(qb[0]), qa[1].max(qb[1])]);
        let (edges, positions) = (&self.edges, &self.m.positions);
        self.grid.edges_near(lo, hi, |e| {
            let (u, v) = edges[e as usize];
            !ends.contains(&u) && !ends.contains(&v) && crossing(pa, pb, positions[u as usize], positions[v as usize], tol).is_some()
        })
    }

    /// Carries out `plan` for the line a-b: its new corners, its line, and the loop it may close.
    fn put(&mut self, plan: Plan, a: [f64; 3], b: [f64; 3], tol: f64) -> Result<(), TopologyError> {
        let Plan { ia, ib, new_a, new_b, line } = plan;
        if new_a {
            self.add_corner(a);
        }
        if new_b {
            self.add_corner(b);
        }
        let first = self.run.is_empty();
        let last = self.run.last().copied();
        if !line {
            // nothing drawn: the outline goes on from the same corner, or it is no longer all its own
            if ia == ib && first && new_a {
                self.run.push(ia);
            } else if !(ia == ib && last == Some(ia)) {
                self.fresh = false;
            }
            return Ok(());
        }
        let k = self.m.lines.len();
        self.m.lines.push([ia, ib]);
        self.lines_at[ia as usize] += 1;
        self.lines_at[ib as usize] += 1;
        let key = (ia.min(ib), ia.max(ib));
        self.edge_set.insert(key);
        self.edges.push(key);
        self.grid.add_edge(self.edges.len() as u32 - 1, self.flat[key.0 as usize], self.flat[key.1 as usize]);
        // back to the outline's first corner, after new corners joined only by its own lines: the loop it closes is that
        // outline, as `close_loop`'s search would find it
        let closes = self.fresh && last == Some(ia) && self.run.first() == Some(&ib) && self.run.len() >= 3;
        if first {
            self.run = vec![ia, ib];
            self.fresh &= new_a && new_b;
        } else if last == Some(ia) && new_b {
            self.run.push(ib);
        } else if !closes {
            self.fresh = false;
        }
        // `close_loop` looks for a loop only where both ends meet another line or a face
        let meets = |v: u32| self.lines_at[v as usize] > 1 || self.in_face[v as usize];
        if !(meets(ia) && meets(ib)) {
            return Ok(());
        }
        self.fresh = false;
        if closes {
            self.close_outline(tol * 10.0);
            return Ok(());
        }
        // any other loop: `close_loop` itself
        self.counts.loops += 1;
        let mut h = HalfEdgeMesh::from_mesh(&self.m)?;
        let faces = h.face_count();
        h.close_loop(k, tol * 10.0);
        if h.face_count() != faces {
            self.m = h.to_mesh();
            self.rebuild();
        }
        Ok(())
    }

    /// The outline being drawn has come back to its first corner: the loop is its corners from the last back to the first,
    /// and its lines the last ones drawn, as `close_loop` finds them; what it becomes is `close_loop`'s choice
    /// (`halfedge::closing`). Its corners are new, so the face or hole always fits.
    fn close_outline(&mut self, tol: f64) {
        let ring: Vec<u32> = self.run.iter().rev().copied().collect();
        let face = match closing(&self.m, ring, tol) {
            None => return,
            Some(Closing::Face(ring)) => {
                self.m.faces.push(ring);
                self.m.faces.len() - 1
            }
            Some(Closing::Hole { host, ring }) => match cut_hole(&mut self.m, host, &ring) {
                None => return,
                Some(f) => f,
            },
        };
        // its lines (one per corner) are the face's edges now
        let keep = self.m.lines.len() - self.run.len();
        for [x, y] in self.m.lines.drain(keep..) {
            self.lines_at[x as usize] -= 1;
            self.lines_at[y as usize] -= 1;
        }
        for &v in &self.m.faces[face] {
            self.in_face[v as usize] = true;
            self.outline_of[v as usize].push(face as u32);
        }
        debug_assert!(HalfEdgeMesh::from_mesh(&self.m).is_ok(), "a closed outline always fits");
    }

    /// A new corner at `p`.
    fn add_corner(&mut self, p: [f64; 3]) {
        let v = self.m.add_vertex(p);
        self.note_corner(v);
        self.lines_at.push(0);
        self.in_face.push(false);
        self.outline_of.push(Vec::new());
    }

    /// Corner `v` (already in the mesh) in the bounds and the grid.
    fn note_corner(&mut self, v: u32) {
        let p = self.m.positions[v as usize];
        self.bounds = Some(match self.bounds {
            None => (p, p),
            Some((mut lo, mut hi)) => {
                for i in 0..3 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
                (lo, hi)
            }
        });
        let q = self.plane.to_local(p);
        self.flat.push(q);
        self.grid.add_corner(v, q);
    }

    /// Everything known about the drawing worked out again from `m` (after `add_line` or `close_loop` changed it).
    fn rebuild(&mut self) {
        for v in self.flat.len()..self.m.positions.len() {
            self.note_corner(v as u32);
        }
        let n = self.m.positions.len();
        self.edges = self.m.all_edges();
        self.edge_set = self.edges.iter().copied().collect();
        self.grid.clear_edges();
        for (e, &(u, v)) in self.edges.iter().enumerate() {
            self.grid.add_edge(e as u32, self.flat[u as usize], self.flat[v as usize]);
        }
        self.lines_at = vec![0; n];
        for &[x, y] in &self.m.lines {
            self.lines_at[x as usize] += 1;
            self.lines_at[y as usize] += 1;
        }
        self.in_face = vec![false; n];
        for (_, x, _) in self.m.face_edges() {
            self.in_face[x as usize] = true;
        }
        self.outline_of = vec![Vec::new(); n];
        for (f, ring) in self.m.faces.iter().enumerate() {
            for &v in ring {
                self.outline_of[v as usize].push(f as u32);
            }
        }
    }

    /// The drawing, as `HalfEdgeMesh::to_mesh` gives it.
    fn finish(self) -> Result<(Mesh, Counts), TopologyError> {
        let h = match self.plain {
            Some(h) => h,
            None => HalfEdgeMesh::from_mesh(&self.m)?,
        };
        Ok((h.to_mesh(), self.counts))
    }
}

/// A grid over the plane for finding the corners and edges near a point or a line: a corner is listed in the cell it lies in,
/// an edge in every cell its box (grown by `pad`) touches, so a lookup finds everything within `pad` of what it asks about
/// (and more, for the caller's exact test).
struct Grid {
    at: [f64; 2],
    cell: f64,
    size: [usize; 2],
    pad: f64,
    corners: Vec<Vec<u32>>,
    edges: Vec<Vec<u32>>,
    /// edge entries in all cells, and how many are too many (long edges across a large grid)
    entries: usize,
    budget: usize,
    /// per edge: the lookup that last listed it (each edge once per lookup)
    seen: Vec<u32>,
    stamp: u32,
}

impl Grid {
    /// About one cell per point over the box lo-hi, none smaller than `pad`.
    fn new(lo: [f64; 2], hi: [f64; 2], points: usize, pad: f64) -> Grid {
        let at = [lo[0] - 2.0 * pad, lo[1] - 2.0 * pad];
        let span = [hi[0] - lo[0] + 4.0 * pad, hi[1] - lo[1] + 4.0 * pad];
        let side = ((points as f64).sqrt().ceil() as usize).clamp(1, 512);
        let cell = (span[0].max(span[1]) / side as f64).max(pad);
        let count = |s: f64| ((s / cell) as usize + 1).min(side + 1);
        let size = [count(span[0]), count(span[1])];
        let cells = size[0] * size[1];
        Grid { at, cell, size, pad, corners: vec![Vec::new(); cells], edges: vec![Vec::new(); cells], entries: 0, budget: 64 * points + 65536, seen: Vec::new(), stamp: 0 }
    }

    fn index(&self, x: f64, axis: usize) -> usize {
        let k = ((x - self.at[axis]) / self.cell).floor();
        if k >= 0.0 { (k as usize).min(self.size[axis] - 1) } else { 0 }
    }

    /// The cells that hold anything within `pad` of the box lo-hi: x from-to, y from-to.
    fn cells(&self, lo: [f64; 2], hi: [f64; 2]) -> [usize; 4] {
        [self.index(lo[0] - self.pad, 0), self.index(hi[0] + self.pad, 0), self.index(lo[1] - self.pad, 1), self.index(hi[1] + self.pad, 1)]
    }

    fn add_corner(&mut self, v: u32, p: [f64; 2]) {
        let c = self.index(p[1], 1) * self.size[0] + self.index(p[0], 0);
        self.corners[c].push(v);
    }

    fn add_edge(&mut self, e: u32, p: [f64; 2], q: [f64; 2]) {
        let [x0, x1, y0, y1] = self.cells([p[0].min(q[0]), p[1].min(q[1])], [p[0].max(q[0]), p[1].max(q[1])]);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.edges[y * self.size[0] + x].push(e);
            }
        }
        self.entries += (x1 - x0 + 1) * (y1 - y0 + 1);
        if self.seen.len() <= e as usize {
            self.seen.resize(e as usize + 1, 0);
        }
    }

    fn clear_edges(&mut self) {
        for c in &mut self.edges {
            c.clear();
        }
        self.entries = 0;
    }

    fn full(&self) -> bool {
        self.entries > self.budget
    }

    /// Calls `f` with each corner near the box lo-hi, until it says true; whether it did.
    fn corners_near(&self, lo: [f64; 2], hi: [f64; 2], mut f: impl FnMut(u32) -> bool) -> bool {
        let [x0, x1, y0, y1] = self.cells(lo, hi);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if self.corners[y * self.size[0] + x].iter().any(|&v| f(v)) {
                    return true;
                }
            }
        }
        false
    }

    /// Calls `f` with each edge near the box lo-hi (once each), until it says true; whether it did.
    fn edges_near(&mut self, lo: [f64; 2], hi: [f64; 2], mut f: impl FnMut(u32) -> bool) -> bool {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.seen.fill(0);
            self.stamp = 1;
        }
        let [x0, x1, y0, y1] = self.cells(lo, hi);
        for y in y0..=y1 {
            for x in x0..=x1 {
                for &e in &self.edges[y * self.size[0] + x] {
                    if self.seen[e as usize] != self.stamp {
                        self.seen[e as usize] = self.stamp;
                        if f(e) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

fn newell(positions: &[[f64; 3]], ring: &[u32]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    for i in 0..ring.len() {
        let (p, q) = (positions[ring[i] as usize], positions[ring[(i + 1) % ring.len()] as usize]);
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    n
}

/// A flat set of faces made solid: the faces, a copy of them moved by `offset`, and walls round their outer edges. Walls
/// along a curve (bending less than `smooth` radians from the next) are joined by hidden seams, so a round letter shows
/// no stripes.
pub fn extrude(cap: &Mesh, offset: [f64; 3], smooth: f64) -> Mesh {
    let n = cap.positions.len() as u32;
    let mut m = Mesh { positions: cap.positions.clone(), ..Default::default() };
    m.positions.extend(cap.positions.iter().map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]));
    let up = cap.faces.first().map(|f| {
        let nn = newell(&cap.positions, f);
        nn[0] * offset[0] + nn[1] * offset[1] + nn[2] * offset[2] >= 0.0
    });
    let up = up.unwrap_or(true);
    // the bottom and the moved top, each with its holes
    for f in 0..cap.faces.len() {
        let loops: Vec<Vec<u32>> = cap.loops(f).cloned().collect();
        let moved: Vec<Vec<u32>> = loops.iter().map(|r| r.iter().map(|v| v + n).collect()).collect();
        let rev = |rs: &[Vec<u32>]| -> Vec<Vec<u32>> { rs.iter().map(|r| r.iter().rev().copied().collect()).collect() };
        let (bottom, top) = if up { (rev(&loops), moved) } else { (loops, rev(&moved)) };
        for mut rings in [bottom, top] {
            let outline = rings.remove(0);
            m.faces.push(outline);
            let k = m.faces.len() - 1;
            m.set_holes(k, rings);
        }
    }
    // the outline's edges, round the outside and round each hole: those only one face uses
    let directed: std::collections::HashSet<(u32, u32)> = cap.face_edges().map(|(_, a, b)| (a, b)).collect();
    let border: Vec<(u32, u32)> = directed.iter().copied().filter(|&(a, b)| !directed.contains(&(b, a))).collect();
    let mut walls: Vec<(u32, u32)> = Vec::new();
    for &(a, b) in &border {
        if up {
            m.faces.push(vec![a, b, b + n, a + n]);
        } else {
            m.faces.push(vec![b, a, a + n, b + n]);
        }
        walls.push((a, b));
    }
    for s in &cap.seams {
        m.seams.push(*s);
        m.seams.push([s[0] + n, s[1] + n]);
    }
    // an upright edge between two walls that hardly bend is hidden
    let dir = |(a, b): (u32, u32)| {
        let (p, q) = (cap.positions[a as usize], cap.positions[b as usize]);
        let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-12);
        d.map(|c| c / l)
    };
    let into: std::collections::HashMap<u32, (u32, u32)> = walls.iter().map(|&(a, b)| (b, (a, b))).collect();
    for &(a, b) in &walls {
        if let Some(&prev) = into.get(&a) {
            let (d0, d1) = (dir(prev), dir((a, b)));
            let cos = d0[0] * d1[0] + d0[1] * d1[1] + d0[2] * d1[2];
            if cos > smooth.cos() {
                m.seams.push([a, a + n]);
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]]
    }

    #[test]
    fn a_square_with_a_hole_and_an_island() {
        let outlines = vec![square(0.0, 0.0, 9.0), square(2.0, 2.0, 5.0), square(4.0, 4.0, 1.0)];
        let cap = faces(&outlines, &Plane::ground()).unwrap();
        let area: f64 = cap
            .faces
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let n = newell(&cap.positions, f);
                assert!(n[2] > 0.0, "every face looks up");
                // its holes, wound the other way, take their area off
                n[2] / 2.0 + cap.holes_of(i).iter().map(|h| newell(&cap.positions, h)[2] / 2.0).sum::<f64>()
            })
            .sum();
        assert!((area - (81.0 - 25.0 + 1.0)).abs() < 1e-3, "area {area}");
        // made solid: closed, and as big as its area times the depth
        let solid = extrude(&cap, [0.0, 0.0, 2.0], 0.4);
        let h = HalfEdgeMesh::from_mesh(&solid).unwrap();
        assert!(h.is_closed());
        assert!((h.signed_volume() - 2.0 * 57.0).abs() < 1e-2, "{}", h.signed_volume());
        // the other way, down from the plane
        let down = HalfEdgeMesh::from_mesh(&extrude(&cap, [0.0, 0.0, -1.0], 0.4)).unwrap();
        assert!((down.signed_volume() - 57.0).abs() < 1e-2, "{}", down.signed_volume());
    }

    #[test]
    fn a_round_wall_hides_its_upright_edges() {
        let ring: Vec<[f64; 2]> = (0..24).map(|i| {
            let a = i as f64 / 24.0 * std::f64::consts::TAU;
            [a.cos(), a.sin()]
        }).collect();
        let cap = faces(&[ring], &Plane::ground()).unwrap();
        let solid = extrude(&cap, [0.0, 0.0, 1.0], 0.5);
        assert_eq!(solid.seams.len(), 24, "every upright edge of a 24-sided round");
        let square = extrude(&faces(&[square(0.0, 0.0, 1.0)], &Plane::ground()).unwrap(), [0.0, 0.0, 1.0], 0.5);
        assert!(square.seams.is_empty(), "a box keeps its corners");
    }

    /// `faces` and `faces_line_by_line` give the same result to the last bit: the same corners, faces (outline and holes,
    /// corner for corner, from the same first corner) and edges, or the same error. Returns what the one-pass drawing left
    /// to `add_line` and `close_loop`.
    fn same_both_ways(outlines: &[Vec<[f64; 2]>], plane: &Plane) -> Counts {
        let (quick, plain) = (faces(outlines, plane), faces_line_by_line(outlines, plane));
        if let (Ok(q), Ok(p)) = (&quick, &plain) {
            assert_eq!(q.faces, p.faces, "faces");
            assert_eq!(q.holes, p.holes, "holes");
            assert_eq!(q.edges(), p.edges(), "edges");
        }
        assert_eq!(format!("{quick:?}"), format!("{plain:?}"));
        let (_, order) = in_order(outlines);
        draw(&order, plane).map(|d| d.1).unwrap_or_default()
    }

    fn round(c: [f64; 2], r: f64, n: usize) -> Vec<[f64; 2]> {
        (0..n).map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        }).collect()
    }

    /// Planes to draw on: the ground, and a sloping one far from the origin (larger coordinates, so a larger tolerance).
    fn planes() -> [Plane; 2] {
        [Plane::ground(), Plane::from_point_normal([1234.5, -67.8, 9.1], [0.3, -0.8, 0.52]).unwrap()]
    }

    #[test]
    fn a_face_with_holes_is_drawn_in_one_pass_as_line_by_line() {
        // a plate with nine round holes and a square one with an island in it
        let mut outlines = vec![square(0.0, 0.0, 20.0)];
        for i in 0..3 {
            for j in 0..3 {
                outlines.push(round([3.0 + 4.0 * i as f64, 3.0 + 4.0 * j as f64], 1.2, 12));
            }
        }
        outlines.push(square(14.0, 2.0, 5.0));
        outlines.push(square(15.5, 3.5, 2.0));
        for plane in planes() {
            let counts = same_both_ways(&outlines, &plane);
            assert_eq!(counts, Counts::default(), "every line and loop drawn in the one pass");
            let cap = faces(&outlines, &plane).unwrap();
            assert_eq!(cap.faces.len(), 2, "the plate and the island");
            assert_eq!((0..2).map(|f| cap.holes_of(f).len()).sum::<usize>(), 10);
        }
        // the holes wound either way round, and a letter-like outline of many corners
        let mut flipped: Vec<Vec<[f64; 2]>> = outlines.iter().map(|r| r.iter().rev().copied().collect()).collect();
        flipped.push(round([10.0, 17.0], 2.0, 40));
        for plane in planes() {
            assert_eq!(same_both_ways(&flipped, &plane), Counts::default());
        }
    }

    #[test]
    fn a_self_touching_outline_is_drawn_as_line_by_line() {
        // a keyhole, as the logos in my SketchUp models come: the outline runs in along a bridge of three lines to a loop
        // inside, round it, and back out the same way
        let keyhole = vec![
            [0.0, 0.0], [10.0, 0.0], [10.0, 1.0], [10.0, 2.0], [10.0, 3.0],
            [8.0, 3.0], [8.0, 6.0], [12.0, 6.0], [12.0, 3.0], [10.0, 3.0],
            [10.0, 2.0], [10.0, 1.0], [10.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0],
        ];
        // two squares touching at one corner, as one outline
        let eight = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0], [2.0, 2.0], [0.0, 2.0]];
        // a hole touching the outline's corner, and one touching another hole's corner
        let touching = vec![square(0.0, 0.0, 10.0), vec![[0.0, 0.0], [3.0, 1.0], [1.0, 3.0]], square(5.0, 5.0, 2.0), vec![[7.0, 7.0], [9.0, 7.5], [8.0, 9.0]]];
        for plane in planes() {
            for (outlines, what) in [(vec![keyhole.clone()], "keyhole"), (vec![keyhole.clone(), round([4.0, 12.0], 2.0, 9)], "keyhole with a hole"), (vec![eight.clone()], "figure eight"), (touching.clone(), "touching holes")] {
                let counts = same_both_ways(&outlines, &plane);
                assert_eq!(counts.lines, 0, "{what}: no line meets anything but corners");
                assert!(counts.loops > 0, "{what}: a loop closed through earlier lines");
            }
        }
        // the loop at the end of the bridge is an opening: a hole, not filled in (its loop closed first, as a face of its own,
        // and the outline round it later)
        let cap = faces(&[keyhole], &Plane::ground()).unwrap();
        assert_eq!((cap.faces.len(), cap.holes_of(0).len()), (1, 1));
        assert!((area(&cap, &Plane::ground()) - (400.0 - 12.0)).abs() < 1e-9);
    }

    /// The area `cap`'s faces cover, worked out in the plane: each outline's, less its holes' (wound the other way); every
    /// face must face the plane's normal.
    fn area(cap: &Mesh, plane: &Plane) -> f64 {
        let signed = |r: &[u32]| {
            let p: Vec<[f64; 2]> = r.iter().map(|&v| plane.to_local(cap.positions[v as usize])).collect();
            (0..p.len()).map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1]).sum::<f64>() / 2.0
        };
        (0..cap.faces.len())
            .map(|f| {
                assert!(signed(&cap.faces[f]) > 0.0, "face {f} faces the plane's normal");
                cap.loops(f).map(|r| signed(r)).sum::<f64>()
            })
            .sum()
    }

    /// The holes of face `f`, each as its corners in the plane (rounded to a millionth, sorted): to compare with the openings
    /// drawn.
    fn holes_seen(cap: &Mesh, f: usize, plane: &Plane) -> Vec<Vec<[i64; 2]>> {
        let flat = |h: &Vec<u32>| h.iter().map(|&v| plane.to_local(cap.positions[v as usize])).collect::<Vec<_>>();
        let mut out: Vec<Vec<[i64; 2]>> = cap.holes_of(f).iter().map(|h| corners_of(&flat(h))).collect();
        out.sort();
        out
    }

    fn corners_of(r: &[[f64; 2]]) -> Vec<[i64; 2]> {
        let mut c: Vec<[i64; 2]> = r.iter().map(|p| p.map(|x| (x * 1e6).round() as i64)).collect();
        c.sort();
        c.dedup();
        c
    }

    /// Every way of drawing `ring`: from each corner, either way round.
    fn every_start(ring: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>> {
        let mut out = Vec::new();
        for k in 0..ring.len() {
            let mut r = ring.to_vec();
            r.rotate_left(k);
            out.push(r.clone());
            r.reverse();
            out.push(r);
        }
        out
    }

    #[test]
    fn an_opening_reached_along_a_doubled_bridge_stays_open() {
        // the keyhole: in along a bridge of three lines to a 4 x 3 loop, round it and back; drawn from a corner before the
        // bridge, the loop closed first and the outline filled it in
        let keyhole = vec![
            [0.0, 0.0], [10.0, 0.0], [10.0, 1.0], [10.0, 2.0], [10.0, 3.0],
            [8.0, 3.0], [8.0, 6.0], [12.0, 6.0], [12.0, 3.0], [10.0, 3.0],
            [10.0, 2.0], [10.0, 1.0], [10.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0],
        ];
        let opening = corners_of(&[[10.0, 3.0], [8.0, 3.0], [8.0, 6.0], [12.0, 6.0], [12.0, 3.0]]);
        // the loop wound against the outline (as a hole is) or with it: by the even-odd rule an opening either way
        let mut with = keyhole.clone();
        with[5..9].reverse();
        for plane in planes() {
            for (ring, how) in [(&keyhole, "against"), (&with, "with")] {
                for r in every_start(ring) {
                    same_both_ways(&[r.clone()], &plane);
                    let cap = faces(&[r.clone()], &plane).unwrap();
                    assert_eq!(cap.faces.len(), 1, "{how}, from {:?}", r[0]);
                    assert_eq!(holes_seen(&cap, 0, &plane), vec![opening.clone()], "{how}, from {:?}", r[0]);
                    assert!((area(&cap, &plane) - 388.0).abs() < 1e-6, "{how}, from {:?}: {}", r[0], area(&cap, &plane));
                }
            }
        }
    }

    #[test]
    fn several_openings_reached_along_doubled_bridges_stay_open() {
        // a 30 x 20 plate: a bridge of two lines up from its bottom edge to opening A (4 x 2), from A's top on to opening B
        // (2 x 2), a bridge of two lines in from the right edge to a diamond C, and a hole D of its own
        let a = [[5.0, 4.0], [3.0, 4.0], [3.0, 6.0], [5.0, 6.0], [7.0, 6.0], [7.0, 4.0]];
        let b = [[5.0, 9.0], [4.0, 9.0], [4.0, 11.0], [6.0, 11.0], [6.0, 9.0]];
        let c = [[24.0, 10.0], [22.0, 8.0], [20.0, 10.0], [22.0, 12.0]];
        let d = vec![[12.0, 12.0], [16.0, 12.0], [16.0, 16.0], [12.0, 16.0]];
        let mut ring = vec![[0.0, 0.0], [5.0, 0.0], [5.0, 2.0]];
        // round A (wound as a hole) from its bottom middle to its top middle, up to B, round B, back down, on round A
        ring.extend(&a[..4]);
        ring.push([5.0, 7.5]);
        ring.extend(b);
        ring.extend([b[0], [5.0, 7.5]]);
        ring.extend(&a[3..]);
        ring.extend([a[0], [5.0, 2.0], [5.0, 0.0], [30.0, 0.0], [30.0, 10.0], [27.0, 10.0]]);
        ring.extend(c);
        ring.extend([c[0], [27.0, 10.0], [30.0, 10.0], [30.0, 20.0], [0.0, 20.0]]);
        let openings = {
            let mut o = vec![corners_of(&a), corners_of(&b), corners_of(&c), corners_of(&d)];
            o.sort();
            o
        };
        let want = 600.0 - 8.0 - 4.0 - 8.0 - 16.0;
        for plane in planes() {
            for r in every_start(&ring) {
                let outlines = vec![r.clone(), d.clone()];
                same_both_ways(&outlines, &plane);
                let cap = faces(&outlines, &plane).unwrap();
                assert_eq!(cap.faces.len(), 1, "from {:?}", r[0]);
                assert_eq!(holes_seen(&cap, 0, &plane), openings, "from {:?}", r[0]);
                assert!((area(&cap, &plane) - want).abs() < 1e-6, "from {:?}: {}", r[0], area(&cap, &plane));
            }
        }
        // a bridge out to a square of its own: two faces, as the loops enclose
        let pair = vec![
            [0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [7.0, 2.0], [7.0, 0.0], [9.0, 0.0],
            [9.0, 4.0], [7.0, 4.0], [7.0, 2.0], [4.0, 2.0], [4.0, 4.0], [0.0, 4.0],
        ];
        for plane in planes() {
            for r in every_start(&pair) {
                same_both_ways(&[r.clone()], &plane);
                let cap = faces(&[r.clone()], &plane).unwrap();
                assert_eq!((cap.faces.len(), cap.holes.iter().flatten().count()), (2, 0), "from {:?}", r[0]);
                assert!((area(&cap, &plane) - 24.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn plates_that_came_out_empty_have_their_face() {
        // two of the random plates below, as they failed: nothing came out at all
        // in along a bridge of three lines to an opening wound with the outline: the outline's loop closed last, and the
        // point `closing` tries (just inside its first corner) lay in the opening, closed first, so its lines were left
        let ring = vec![
            [120.0, 390.0], [120.0, 0.0], [60.0, 0.0], [60.0, 80.0], [60.0, 160.0], [60.0, 240.0], [90.0, 240.0], [90.0, 300.0],
            [60.0, 300.0], [30.0, 300.0], [30.0, 240.0], [60.0, 240.0], [60.0, 160.0], [60.0, 80.0], [60.0, 0.0], [0.0, 0.0], [0.0, 390.0],
        ];
        let plane = Plane::ground();
        same_both_ways(&[ring.clone()], &plane);
        let cap = faces(&[ring], &plane).unwrap();
        assert_eq!((cap.faces.len(), cap.holes_of(0).len()), (1, 1));
        assert!((area(&cap, &plane) - (120.0 * 390.0 - 60.0 * 60.0)).abs() < 1e-6);
        // 4 x 13 mm with two openings, on a sloping plane far from the origin: the plate's first triangle was a sliver along
        // its bottom edge (corners in a row there, a hair off straight so far out), and the plate was taken for open
        let ring: Vec<[f64; 2]> = [
            [1.0, 3.0], [2.0, 3.0], [2.0, 1.5], [2.0, 0.0], [0.0, 0.0], [0.0, 13.0], [2.0, 13.0], [2.0, 10.0], [3.0, 10.0], [3.0, 8.0],
            [2.0, 8.0], [1.0, 8.0], [1.0, 10.0], [2.0, 10.0], [2.0, 13.0], [4.0, 13.0], [4.0, 0.0], [2.0, 0.0], [2.0, 1.5], [2.0, 3.0],
            [3.0, 3.0], [3.0, 5.0], [2.0, 5.0], [1.0, 5.0],
        ]
        .iter()
        .map(|p| [p[0] * 0.001, p[1] * 0.001])
        .collect();
        let plane = Plane {
            origin: [709.2129494994879, 205.93247283250093, 176.2825476936996],
            u: [-0.42598999065417575, 0.904727875033402, 0.0],
            v: [0.6838495430777648, 0.3219897037590494, 0.654730809650418],
            normal: [0.5923532141339216, 0.2789087714839825, -0.7558621348463674],
        };
        same_both_ways(&[ring.clone()], &plane);
        let cap = faces(&[ring], &plane).unwrap();
        assert_eq!((cap.faces.len(), cap.holes_of(0).len()), (1, 2));
        assert!((area(&cap, &plane) - 44e-6).abs() < 1e-15, "{}", area(&cap, &plane));
    }

    #[test]
    fn past_the_grid_budget_the_rest_is_drawn_by_add_line() {
        let mut outlines = vec![square(0.0, 0.0, 20.0), square(1.0, 1.0, 3.0)];
        outlines.extend((0..6).map(|i| round([6.0 + 2.0 * i as f64, 10.0], 0.8, 7)));
        let (_, order) = in_order(&outlines);
        let plane = planes()[1];
        let mut h = HalfEdgeMesh::default();
        for r in &order {
            for i in 0..r.len() {
                let (a, b) = (r[i], r[(i + 1) % r.len()]);
                h.add_line(plane.to_world(a[0], a[1]), plane.to_world(b[0], b[1])).unwrap();
            }
        }
        let plain = format!("{:?}", h.to_mesh());
        // the grid outgrown after a few lines, or a few outlines: the same drawing
        for budget in [0, 20, 60] {
            let (m, counts) = draw_within(&order, &plane, Some(budget)).unwrap();
            assert_eq!(format!("{m:?}"), plain, "budget {budget}");
            assert!(counts.lines > 0, "budget {budget}: the rest went to add_line");
        }
        assert_eq!(draw_within(&order, &plane, None).unwrap().1, Counts::default());
    }

    #[test]
    fn crossing_and_near_outlines_are_drawn_as_line_by_line() {
        let plane = Plane::ground();
        // outlines crossing each other: those lines break where they cross (add_line itself)
        let crossing = vec![square(0.0, 0.0, 4.0), square(2.0, 2.0, 4.0)];
        assert!(same_both_ways(&crossing, &plane).lines > 0);
        // a corner on another outline's edge, corners a hair apart, a corner given twice, corners along a straight edge
        let t = vec![square(0.0, 0.0, 4.0), vec![[4.0, 1.0], [6.0, 1.0], [6.0, 3.0]]];
        let hair = vec![square(0.0, 0.0, 4.0), vec![[4.0 + 1e-9, 4.0], [6.0, 4.0], [6.0, 6.0]]];
        let twice = vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]];
        let straight = vec![vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]], vec![[0.5, 0.25], [1.5, 0.25], [1.0, 0.75]]];
        for outlines in [t, hair, twice, straight] {
            for plane in planes() {
                same_both_ways(&outlines, &plane);
            }
        }
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn unit(&mut self) -> f64 {
            self.next() as f64 / (1u64 << 31) as f64
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    #[test]
    fn random_plates_keep_every_opening_reached_along_a_bridge() {
        // a plate in columns 4 wide, each with up to two 2 x 2 openings, one above the other: the lower reached along a
        // bridge up from the bottom edge, the upper from the top edge, from the bottom edge (no lower one) or on from the
        // lower one; each wound as a hole or the other way, bridges of one to three lines; plain triangular holes in some
        // free places; drawn from any corner, either way round, at any size, on the ground or a sloping plane far away:
        // one face, a hole for each opening, and the area the outlines enclose. PROFILE_CASES=n for more
        let cases: usize = std::env::var("PROFILE_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(200);
        let mut rng = Rng(11);
        let (mut openings_seen, mut chained) = (0, 0);
        for case in 0..cases {
            let cols = 1 + rng.below(5);
            let (w, h) = (4.0 * cols as f64, 13.0);
            // the opening with its bottom middle at (x, y), from there round (clockwise: as a hole), its top middle 4th
            let opening = |x: f64, y: f64, clockwise: bool| {
                let mut r = vec![[x, y], [x - 1.0, y], [x - 1.0, y + 2.0], [x, y + 2.0], [x + 1.0, y + 2.0], [x + 1.0, y]];
                if !clockwise {
                    r[1..].reverse();
                }
                r
            };
            // a bridge from (x, y0) to (x, y1), both ends included
            let bridge = |rng: &mut Rng, x: f64, y0: f64, y1: f64| {
                let k = rng.below(3);
                (0..=k + 1).map(|i| [x, y0 + (y1 - y0) * i as f64 / (k + 1) as f64]).collect::<Vec<_>>()
            };
            let mut walk = vec![[0.0, 0.0]];
            let mut tops = Vec::new();
            let (mut holes, mut plain) = (Vec::new(), Vec::new());
            for c in 0..cols {
                let x = 4.0 * c as f64 + 2.0;
                let (lower, upper) = (rng.below(3) > 0, rng.below(2) == 0);
                // the upper one reached from below (the bottom edge, or on from the lower one), or else from the top edge
                let from_below = upper && rng.below(2) == 0;
                if lower || from_below {
                    let first = if lower { 3.0 } else { 8.0 };
                    let up = bridge(&mut rng, x, 0.0, first);
                    walk.extend(&up);
                    let o = opening(x, first, rng.below(4) > 0);
                    holes.push(corners_of(&o));
                    for (i, &p) in o.iter().enumerate().skip(1) {
                        walk.push(p);
                        if i == 3 && lower && from_below {
                            // at its top middle, on to the upper one and back
                            chained += 1;
                            let on = bridge(&mut rng, x, 5.0, 8.0);
                            walk.extend(&on[1..]);
                            let o2 = opening(x, 8.0, rng.below(4) > 0);
                            holes.push(corners_of(&o2));
                            walk.extend(&o2[1..]);
                            walk.extend(on.iter().rev());
                        }
                    }
                    walk.extend(up.iter().rev());
                }
                if upper && !from_below {
                    tops.push(x);
                }
                // plain holes where nothing else is
                if !upper && rng.below(3) == 0 {
                    plain.push(vec![[x - 1.0, 8.0], [x + 1.0, 8.0], [x, 10.0]]);
                }
                if !lower && !from_below && rng.below(3) == 0 {
                    plain.push(vec![[x - 1.0, 3.0], [x + 1.0, 3.0], [x, 5.0]]);
                }
            }
            walk.extend([[w, 0.0], [w, h]]);
            for &x in tops.iter().rev() {
                let down = bridge(&mut rng, x, h, 10.0);
                walk.extend(&down);
                let mut o = opening(x, 8.0, rng.below(4) > 0);
                holes.push(corners_of(&o));
                o.rotate_left(3);
                walk.extend(&o[1..]);
                walk.extend(down.iter().rev());
            }
            walk.push([0.0, h]);
            let n = holes.len();
            openings_seen += n;
            holes.extend(plain.iter().map(|r| corners_of(r)));
            let want = w * h - 4.0 * n as f64 - 2.0 * plain.len() as f64;
            // any size, from any corner, either way round
            let s = [0.001, 0.05, 1.0, 30.0][rng.below(4)];
            let k = rng.below(walk.len());
            walk.rotate_left(k);
            if rng.below(2) == 0 {
                walk.reverse();
            }
            let mut outlines: Vec<Vec<[f64; 2]>> = vec![walk];
            outlines.extend(plain.iter().map(|r| if rng.below(2) == 0 { r.iter().rev().copied().collect() } else { r.clone() }));
            let outlines: Vec<Vec<[f64; 2]>> = outlines.iter().map(|r| r.iter().map(|p| [p[0] * s, p[1] * s]).collect()).collect();
            let mut holes: Vec<Vec<[i64; 2]>> = holes.iter().map(|r| r.iter().map(|p| p.map(|c| (c as f64 * s).round() as i64)).collect()).collect();
            for r in &mut holes {
                r.sort();
            }
            holes.sort();
            let plane = if rng.below(2) == 0 {
                Plane::ground()
            } else {
                let o = [rng.unit() * 2000.0 - 1000.0, rng.unit() * 2000.0 - 1000.0, rng.unit() * 200.0];
                Plane::from_point_normal(o, [rng.unit() - 0.5, rng.unit() - 0.5, rng.unit() - 0.5]).unwrap_or(Plane::ground())
            };
            let what = format!("case {case}: {outlines:?} on {plane:?}");
            same_both_ways(&outlines, &plane);
            let cap = faces(&outlines, &plane).unwrap();
            assert_eq!(cap.faces.len(), 1, "{what}");
            assert_eq!(holes_seen(&cap, 0, &plane), holes, "{what}");
            let got = area(&cap, &plane);
            assert!((got - want * s * s).abs() <= 1e-9 * want * s * s, "{what}: area {got}, not {}", want * s * s);
        }
        assert!(openings_seen > cases && chained > cases / 10, "{openings_seen} openings, {chained} reached from another");
    }

    #[test]
    fn many_random_outlines_are_drawn_as_line_by_line() {
        // squares, triangles, stars and keyholes, on a half-unit lattice (so corners land on corners and edges, edges along
        // edges) or anywhere, nested, touching and crossing, on the ground or on a sloping plane far away;
        // PROFILE_CASES=n for more
        fn shape(rng: &mut Rng) -> Vec<[f64; 2]> {
            let lattice = rng.below(2) == 0;
            let at = |rng: &mut Rng, span: f64| if lattice { (rng.below((span * 2.0) as usize + 1) as f64) * 0.5 } else { rng.unit() * span };
            let mut r = match rng.below(4) {
                0 => {
                    let (x, y) = (at(rng, 8.0), at(rng, 8.0));
                    let (w, h) = (0.5 + at(rng, 6.0), 0.5 + at(rng, 6.0));
                    vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
                }
                1 => (0..3).map(|_| [at(rng, 10.0), at(rng, 10.0)]).collect(),
                2 => {
                    let (c, n) = ([at(rng, 10.0), at(rng, 10.0)], 5 + rng.below(8));
                    let size = 0.5 + rng.unit() * 4.0;
                    (0..n).map(|i| {
                        let (a, r) = (i as f64 / n as f64 * std::f64::consts::TAU, size * (0.4 + 0.6 * rng.unit()));
                        [c[0] + r * a.cos(), c[1] + r * a.sin()]
                    }).collect()
                }
                _ => {
                    // a keyhole: in along a bridge to a loop inside (wound either way), round it and back out
                    let (x, y) = (at(rng, 4.0), at(rng, 4.0));
                    let (w, h) = (4.0 + at(rng, 4.0), 4.0 + at(rng, 4.0));
                    let m = x + w / 2.0;
                    let (b0, b1) = (y + 1.0, y + 1.5 + (rng.below(3) as f64) * 0.5);
                    let (i0, i1, top) = (m - 1.0, m + 1.0, b1 + 1.0);
                    let mut inner = vec![[i0, b1], [i0, top], [i1, top], [i1, b1]];
                    if rng.below(2) == 0 {
                        inner.reverse();
                    }
                    let mut r = vec![[x, y], [m, y], [m, b0], [m, b1]];
                    r.extend(inner);
                    r.extend([[m, b1], [m, b0], [m, y], [x + w, y], [x + w, y + h], [x, y + h]]);
                    r
                }
            };
            // either way round, from any corner; now and then a corner twice or a hair off
            if rng.below(2) == 0 {
                r.reverse();
            }
            let k = rng.below(r.len());
            r.rotate_left(k);
            if rng.below(8) == 0 {
                let i = rng.below(r.len());
                r.insert(i, r[i]);
            }
            if rng.below(6) == 0 {
                // a hair off, or about as far off as the tolerance (1e-7 of the drawing's size), either side of it
                let i = rng.below(r.len());
                r[i][rng.below(2)] += [1e-10, 5e-7, 1e-6, 1.5e-6, 3e-6][rng.below(5)];
            }
            r
        }
        let cases: usize = std::env::var("PROFILE_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
        let mut rng = Rng(7);
        let (mut quick, mut lines, mut loops) = (0, 0, 0);
        for case in 0..cases {
            let mut outlines: Vec<Vec<[f64; 2]>> = Vec::new();
            if rng.below(6) == 0 {
                // a plate with many holes, some maybe touching or crossing
                outlines.push(square(-1.0, -1.0, 12.0));
                for _ in 0..4 + rng.below(12) {
                    let (c, s) = ([rng.unit() * 10.0, rng.unit() * 10.0], 0.2 + rng.unit() * 0.8);
                    outlines.push(round(c, s, 3 + rng.below(9)));
                }
            }
            for _ in 0..1 + rng.below(5) {
                // now and then the last outline again on top of itself (either way round)
                let again = rng.below(8) == 0 && !outlines.is_empty();
                let next = if again { outlines[outlines.len() - 1].clone() } else { shape(&mut rng) };
                outlines.push(if again && rng.below(2) == 0 { next.into_iter().rev().collect() } else { next });
            }
            let plane = if rng.below(2) == 0 {
                Plane::ground()
            } else {
                let o = [rng.unit() * 2000.0 - 1000.0, rng.unit() * 2000.0 - 1000.0, rng.unit() * 200.0];
                let n = [rng.unit() - 0.5, rng.unit() - 0.5, rng.unit() - 0.5];
                Plane::from_point_normal(o, n).unwrap_or(Plane::ground())
            };
            let result = std::panic::catch_unwind(|| same_both_ways(&outlines, &plane));
            let counts = result.unwrap_or_else(|_| panic!("case {case} differs: {outlines:?} on {plane:?}"));
            quick += (counts == Counts::default()) as usize;
            lines += counts.lines;
            loops += counts.loops;
        }
        // the cases exercised both what the one pass draws itself and what it leaves to add_line and close_loop
        assert!(quick > cases / 10 && lines > 0 && loops > 0, "{quick} wholly quick, {lines} lines and {loops} loops left over");
    }
}
