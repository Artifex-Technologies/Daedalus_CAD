//! Solid inspection and repair.
//!
//! [`inspect`] reports why a mesh is not a valid solid; [`repair`] fixes what
//! can be fixed automatically and returns a new mesh. Both work on the
//! interchange [`Mesh`] because broken geometry (non-manifold edges, mixed
//! winding) cannot be represented by `HalfEdgeMesh`.

use crate::Mesh;
use std::collections::{HashMap, HashSet, VecDeque};

pub const DEFAULT_TOLERANCE: f64 = 1e-4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueKind {
    /// An edge belongs to exactly one face: the surface has a hole or open border.
    SurfaceBorder,
    /// An edge is shared by more than two faces (includes internal faces).
    NonManifoldEdge,
    /// Two faces use a shared edge in the same direction: one is flipped.
    InconsistentWinding,
    /// A closed, consistent surface whose faces point inward.
    InvertedSolid,
    /// Two faces use exactly the same vertices.
    DuplicateFace,
    /// A face with (almost) no area, or a repeated corner.
    ZeroAreaFace,
    /// An edge shorter than the tolerance.
    ShortEdge,
    /// A vertex that no face uses.
    UnusedVertex,
    /// Distinct vertices at (nearly) the same position.
    CoincidentVertices,
}

impl IssueKind {
    pub fn label(self) -> &'static str {
        match self {
            IssueKind::SurfaceBorder => "surface border (open edge)",
            IssueKind::NonManifoldEdge => "non-manifold edge / internal face",
            IssueKind::InconsistentWinding => "reversed face (inconsistent winding)",
            IssueKind::InvertedSolid => "inverted solid",
            IssueKind::DuplicateFace => "duplicate face",
            IssueKind::ZeroAreaFace => "zero-area face",
            IssueKind::ShortEdge => "short edge",
            IssueKind::UnusedVertex => "unused vertex",
            IssueKind::CoincidentVertices => "coincident vertices",
        }
    }

    /// Kinds that stop a mesh from being a valid solid. Short edges, unused
    /// vertices and coincident vertices are reported but do not by themselves.
    pub fn breaks_solid(self) -> bool {
        !matches!(self, IssueKind::ShortEdge | IssueKind::UnusedVertex | IssueKind::CoincidentVertices)
    }
}

pub const ALL_KINDS: [IssueKind; 9] = [
    IssueKind::SurfaceBorder,
    IssueKind::NonManifoldEdge,
    IssueKind::InconsistentWinding,
    IssueKind::InvertedSolid,
    IssueKind::DuplicateFace,
    IssueKind::ZeroAreaFace,
    IssueKind::ShortEdge,
    IssueKind::UnusedVertex,
    IssueKind::CoincidentVertices,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub kind: IssueKind,
    pub faces: Vec<usize>,
    pub vertices: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub issues: Vec<Issue>,
    /// Signed volume, when the surface is closed and consistently wound.
    pub volume: Option<f64>,
}

impl Report {
    pub fn count(&self, kind: IssueKind) -> usize {
        self.issues.iter().filter(|i| i.kind == kind).count()
    }

    pub fn is_solid(&self) -> bool {
        self.issues.iter().all(|i| !i.kind.breaks_solid())
    }

    /// Human-readable multi-line summary: one line per issue kind found, then a verdict.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        for kind in ALL_KINDS {
            let n = self.count(kind);
            if n > 0 {
                s.push_str(&format!("  {n:>6}  {}\n", kind.label()));
            }
        }
        match (self.is_solid(), self.volume) {
            (true, Some(v)) => s.push_str(&format!("valid solid, volume {v}")),
            (true, None) => s.push_str("no solid-breaking issues"),
            (false, _) => s.push_str("NOT a valid solid"),
        }
        s
    }

    /// Indices of every face involved in an issue that breaks the solid.
    pub fn flagged_faces(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self
            .issues
            .iter()
            .filter(|i| i.kind.breaks_solid())
            .flat_map(|i| i.faces.iter().copied())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

fn ring_edges(f: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    (0..f.len()).map(move |i| (f[i], f[(i + 1) % f.len()]))
}

fn key(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

fn quantize(p: [f64; 3], tol: f64) -> (i64, i64, i64) {
    ((p[0] / tol).round() as i64, (p[1] / tol).round() as i64, (p[2] / tol).round() as i64)
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    dot(d, d).sqrt()
}

/// Polygon area via the magnitude of the Newell vector.
fn area(positions: &[[f64; 3]], ring: &[u32]) -> f64 {
    let mut n = [0.0f64; 3];
    for (a, b) in ring_edges(ring) {
        let (a, b) = (positions[a as usize], positions[b as usize]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    dot(n, n).sqrt() * 0.5
}

fn face_volume(positions: &[[f64; 3]], ring: &[u32]) -> f64 {
    let p = |i: usize| positions[ring[i] as usize];
    (1..ring.len() - 1).map(|i| dot(p(0), cross(p(i), p(i + 1)))).sum::<f64>() / 6.0
}

/// A ring's (unnormalised) Newell normal.
fn ring_normal(positions: &[[f64; 3]], ring: &[u32]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    for i in 0..ring.len() {
        let (a, b) = (positions[ring[i] as usize], positions[ring[(i + 1) % ring.len()] as usize]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

/// Signed volume of a mesh (positive when faces point outward). Only meaningful when closed.
pub fn signed_volume(mesh: &Mesh) -> f64 {
    // holes, wound the other way, take their part off
    (0..mesh.faces.len()).flat_map(|f| mesh.loops(f)).map(|r| face_volume(&mesh.positions, r)).sum()
}

pub fn inspect(mesh: &Mesh, tol: f64) -> Report {
    let mut issues = Vec::new();

    // Undirected edge -> [(face, stored direction matches (min, max))]
    let mut edges: HashMap<(u32, u32), Vec<(usize, bool)>> = HashMap::new();
    for (fi, a, b) in mesh.face_edges() {
        if a != b {
            edges.entry(key(a, b)).or_default().push((fi, a < b));
        }
    }
    let mut keys: Vec<_> = edges.keys().copied().collect();
    keys.sort_unstable();

    let (mut open, mut non_manifold, mut inconsistent) = (false, false, false);
    for &(a, b) in &keys {
        let uses = &edges[&(a, b)];
        let faces: Vec<usize> = uses.iter().map(|u| u.0).collect();
        match uses.len() {
            1 => {
                open = true;
                issues.push(Issue { kind: IssueKind::SurfaceBorder, faces, vertices: vec![a, b] });
            }
            2 => {
                if uses[0].1 == uses[1].1 {
                    inconsistent = true;
                    issues.push(Issue { kind: IssueKind::InconsistentWinding, faces, vertices: vec![a, b] });
                }
            }
            _ => {
                non_manifold = true;
                issues.push(Issue { kind: IssueKind::NonManifoldEdge, faces, vertices: vec![a, b] });
            }
        }
        if dist(mesh.positions[a as usize], mesh.positions[b as usize]) < tol {
            issues.push(Issue { kind: IssueKind::ShortEdge, faces: vec![], vertices: vec![a, b] });
        }
    }

    let mut by_vertices: HashMap<Vec<u32>, Vec<usize>> = HashMap::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        let repeated = ring_edges(f).any(|(a, b)| a == b);
        if repeated || f.len() < 3 || area(&mesh.positions, f) < tol * tol {
            issues.push(Issue { kind: IssueKind::ZeroAreaFace, faces: vec![fi], vertices: f.clone() });
        }
        let mut k = f.clone();
        k.sort_unstable();
        by_vertices.entry(k).or_default().push(fi);
    }
    let mut dupes: Vec<_> = by_vertices.into_values().filter(|g| g.len() > 1).collect();
    dupes.sort();
    for faces in dupes {
        issues.push(Issue { kind: IssueKind::DuplicateFace, faces, vertices: vec![] });
    }

    let mut used = vec![false; mesh.positions.len()];
    for f in mesh.faces.iter().chain(mesh.holes.iter().flatten()) {
        for &v in f {
            if let Some(u) = used.get_mut(v as usize) {
                *u = true;
            }
        }
    }
    for (v, &u) in used.iter().enumerate() {
        if !u {
            issues.push(Issue { kind: IssueKind::UnusedVertex, faces: vec![], vertices: vec![v as u32] });
        }
    }

    let mut at: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    for (i, &p) in mesh.positions.iter().enumerate() {
        at.entry(quantize(p, tol)).or_default().push(i as u32);
    }
    let mut groups: Vec<_> = at.into_values().filter(|g| g.len() > 1).collect();
    groups.sort();
    for vertices in groups {
        issues.push(Issue { kind: IssueKind::CoincidentVertices, faces: vec![], vertices });
    }

    let mut volume = None;
    if !mesh.faces.is_empty() && !open && !non_manifold && !inconsistent {
        let v = signed_volume(mesh);
        volume = Some(v);
        if v < 0.0 {
            issues.push(Issue { kind: IssueKind::InvertedSolid, faces: (0..mesh.faces.len()).collect(), vertices: vec![] });
        }
    }
    if mesh.faces.is_empty() {
        issues.push(Issue { kind: IssueKind::ZeroAreaFace, faces: vec![], vertices: vec![] });
    }

    Report { issues, volume }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairSummary {
    pub welded_vertices: usize,
    pub removed_degenerate_faces: usize,
    pub removed_duplicate_faces: usize,
    pub removed_unused_vertices: usize,
    pub reoriented_faces: usize,
    pub filled_holes: usize,
    pub inverted_solids_flipped: usize,
}

impl RepairSummary {
    pub fn changed_anything(&self) -> bool {
        *self != RepairSummary::default()
    }
}

/// Repairs what can be fixed automatically: welds coincident vertices, drops
/// degenerate and duplicate faces and unused vertices, makes winding
/// consistent, fills simple holes, and flips inverted closed solids.
/// Non-manifold edges are left alone; run [`inspect`] on the result.
pub fn repair(mesh: &Mesh, tol: f64) -> (Mesh, RepairSummary) {
    let mut s = RepairSummary::default();

    // 1. Weld vertices that coincide within the tolerance.
    let mut first: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut remap = Vec::with_capacity(mesh.positions.len());
    for (i, &p) in mesh.positions.iter().enumerate() {
        let id = *first.entry(quantize(p, tol)).or_insert(i as u32);
        if id != i as u32 {
            s.welded_vertices += 1;
        }
        remap.push(id);
    }

    // 2. Rebuild faces, dropping degenerate ones (each keeps its holes, welded the same way).
    let weld = |f: &Vec<u32>| {
        let mut ring: Vec<u32> = f.iter().filter_map(|&v| remap.get(v as usize).copied()).collect();
        ring.dedup();
        while ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        ring
    };
    let mut faces: Vec<Vec<u32>> = Vec::new();
    let mut holes: Vec<Vec<Vec<u32>>> = Vec::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        let ring = weld(f);
        if ring.len() < 3 || area(&mesh.positions, &ring) < tol * tol {
            s.removed_degenerate_faces += 1;
        } else {
            faces.push(ring);
            holes.push(mesh.holes_of(fi).iter().map(weld).filter(|h| h.len() >= 3).collect());
        }
    }

    // 3. Drop duplicate faces, keeping the first.
    let mut seen = HashSet::new();
    let fresh: Vec<bool> = faces
        .iter()
        .map(|f| {
            let mut k = f.clone();
            k.sort_unstable();
            seen.insert(k)
        })
        .collect();
    s.removed_duplicate_faces = fresh.iter().filter(|&&f| !f).count();
    let mut keep = fresh.iter();
    faces.retain(|_| *keep.next().expect("one each"));
    let mut keep = fresh.iter();
    holes.retain(|_| *keep.next().expect("one each"));

    // 4. Compact vertices to those still used.
    // Original order is kept so repairing a clean mesh returns it unchanged.
    let mut used = vec![false; mesh.positions.len()];
    faces.iter().chain(holes.iter().flatten()).flatten().for_each(|&v| used[v as usize] = true);
    mesh.lines.iter().flatten().for_each(|&v| used[v as usize] = true);
    let mut new_index = vec![u32::MAX; mesh.positions.len()];
    let mut positions = Vec::new();
    for (i, _) in used.iter().enumerate().filter(|(_, &u)| u) {
        new_index[i] = positions.len() as u32;
        positions.push(mesh.positions[i]);
    }
    faces.iter_mut().chain(holes.iter_mut().flatten()).flatten().for_each(|v| *v = new_index[*v as usize]);
    let lines: Vec<[u32; 2]> = mesh.lines.iter().map(|l| l.map(|v| new_index[v as usize])).filter(|l| l[0] != u32::MAX && l[1] != u32::MAX).collect();
    let seams: Vec<[u32; 2]> = mesh.seams.iter().map(|l| l.map(|v| new_index[v as usize])).filter(|l| l[0] != u32::MAX && l[1] != u32::MAX).collect();
    s.removed_unused_vertices = mesh.positions.len() - s.welded_vertices - positions.len();

    // 5-7. Orientation, gaps in the surface, inverted solids. A face turned round turns its holes round with it.
    let before: Vec<[f64; 3]> = faces.iter().map(|f| ring_normal(&positions, f)).collect();
    s.reoriented_faces = make_consistent(&mut faces);
    s.filled_holes = fill_holes(&positions, &mut faces, tol);
    s.inverted_solids_flipped = fix_inverted(&positions, &mut faces);
    for (i, n) in before.iter().enumerate() {
        if dot(*n, ring_normal(&positions, &faces[i])) < 0.0 {
            for h in &mut holes[i] {
                h.reverse();
            }
        }
    }

    // repair renumbers and drops faces, so colours are not carried (it is for imported meshes, which have none)
    let mut out = Mesh { positions, faces, lines, seams, colors: Vec::new(), holes: Vec::new() };
    for (i, h) in holes.into_iter().enumerate() {
        out.set_holes(i, h);
    }
    (out, s)
}

fn edge_faces(faces: &[Vec<u32>]) -> HashMap<(u32, u32), Vec<usize>> {
    let mut adj: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (fi, f) in faces.iter().enumerate() {
        for (a, b) in ring_edges(f) {
            adj.entry(key(a, b)).or_default().push(fi);
        }
    }
    adj
}

/// Flood-fills across manifold edges so neighbouring faces agree on direction.
/// Each connected piece keeps the winding of its lowest-numbered face.
fn make_consistent(faces: &mut [Vec<u32>]) -> usize {
    let adj = edge_faces(faces);
    let mut visited = vec![false; faces.len()];
    let mut flipped = 0;
    for start in 0..faces.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut queue = VecDeque::from([start]);
        while let Some(f) = queue.pop_front() {
            let edges: Vec<(u32, u32)> = ring_edges(&faces[f]).collect();
            for (a, b) in edges {
                let Some(list) = adj.get(&key(a, b)) else { continue };
                if list.len() != 2 {
                    continue;
                }
                let g = if list[0] == f { list[1] } else { list[0] };
                if g == f || visited[g] {
                    continue;
                }
                visited[g] = true;
                // f runs a->b, so g must run b->a.
                if ring_edges(&faces[g]).any(|e| e == (a, b)) {
                    faces[g].reverse();
                    flipped += 1;
                }
                queue.push_back(g);
            }
        }
    }
    flipped
}

/// Closes simple boundary loops with a polygon (or a fan when non-planar).
fn fill_holes(positions: &[[f64; 3]], faces: &mut Vec<Vec<u32>>, tol: f64) -> usize {
    let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
    for f in faces.iter() {
        for e in ring_edges(f) {
            *directed.entry(e).or_default() += 1;
        }
    }
    // Boundary edges: used once and with no partner running the other way.
    let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut starts: Vec<(u32, u32)> = directed
        .iter()
        .filter(|&(&(a, b), &n)| n == 1 && !directed.contains_key(&(b, a)))
        .map(|(&e, _)| e)
        .collect();
    starts.sort_unstable();
    for &(a, b) in &starts {
        next.entry(a).or_default().push(b);
    }

    let mut done: HashSet<(u32, u32)> = HashSet::new();
    let mut filled = 0;
    let mut new_faces = Vec::new();
    for &(a0, b0) in &starts {
        if done.contains(&(a0, b0)) {
            continue;
        }
        let mut ring = vec![a0];
        let (mut a, mut b) = (a0, b0);
        let mut closed = false;
        loop {
            done.insert((a, b));
            if b == a0 {
                closed = true;
                break;
            }
            ring.push(b);
            let Some(n) = next.get(&b) else { break };
            if n.len() != 1 || ring.len() > starts.len() {
                break;
            }
            a = b;
            b = n[0];
        }
        if !closed || ring.len() < 3 {
            continue;
        }
        // The existing surface runs along the loop; the patch must run against it.
        ring.reverse();
        if is_planar(positions, &ring, tol) {
            new_faces.push(ring);
        } else {
            for i in 1..ring.len() - 1 {
                new_faces.push(vec![ring[0], ring[i], ring[i + 1]]);
            }
        }
        filled += 1;
    }
    faces.extend(new_faces);
    filled
}

fn is_planar(positions: &[[f64; 3]], ring: &[u32], tol: f64) -> bool {
    let mut n = [0.0f64; 3];
    for (a, b) in ring_edges(ring) {
        let (a, b) = (positions[a as usize], positions[b as usize]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let len = dot(n, n).sqrt();
    if len == 0.0 {
        return false;
    }
    let n = [n[0] / len, n[1] / len, n[2] / len];
    let p0 = positions[ring[0] as usize];
    ring.iter().all(|&v| {
        let p = positions[v as usize];
        dot(n, [p[0] - p0[0], p[1] - p0[1], p[2] - p0[2]]).abs() <= tol * 10.0
    })
}

/// Flips every closed connected piece whose signed volume is negative.
fn fix_inverted(positions: &[[f64; 3]], faces: &mut [Vec<u32>]) -> usize {
    let adj = edge_faces(faces);
    let mut visited = vec![false; faces.len()];
    let mut flipped = 0;
    for start in 0..faces.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let (mut component, mut queue) = (vec![start], VecDeque::from([start]));
        while let Some(f) = queue.pop_front() {
            for (a, b) in ring_edges(&faces[f]) {
                for &g in adj.get(&key(a, b)).into_iter().flatten() {
                    if !visited[g] {
                        visited[g] = true;
                        component.push(g);
                        queue.push_back(g);
                    }
                }
            }
        }
        let closed = component
            .iter()
            .all(|&f| ring_edges(&faces[f]).all(|(a, b)| adj.get(&key(a, b)).map_or(0, Vec::len) == 2));
        let volume: f64 = component.iter().map(|&f| face_volume(positions, &faces[f])).sum();
        if closed && volume < 0.0 {
            for f in component {
                faces[f].reverse();
            }
            flipped += 1;
        }
    }
    flipped
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = DEFAULT_TOLERANCE;

    fn cube() -> Mesh {
        let mut m = Mesh::new();
        for z in [0.0, 1.0] {
            for y in [0.0, 1.0] {
                for x in [0.0, 1.0] {
                    m.add_vertex([x, y, z]);
                }
            }
        }
        for f in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
            m.add_face(f.to_vec());
        }
        m
    }

    #[test]
    fn clean_cube_is_a_solid() {
        let r = inspect(&cube(), TOL);
        assert!(r.is_solid(), "{:?}", r.issues);
        assert!((r.volume.unwrap() - 1.0).abs() < 1e-5);
        assert!(r.flagged_faces().is_empty());
    }

    #[test]
    fn missing_face_shows_four_borders() {
        let mut m = cube();
        m.faces.remove(1);
        let r = inspect(&m, TOL);
        assert_eq!(r.count(IssueKind::SurfaceBorder), 4);
        assert!(!r.is_solid());
        assert!(r.volume.is_none());
    }

    #[test]
    fn flipped_face_is_inconsistent_winding() {
        let mut m = cube();
        m.faces[1].reverse();
        let r = inspect(&m, TOL);
        assert_eq!(r.count(IssueKind::InconsistentWinding), 4);
    }

    #[test]
    fn inverted_cube_detected_and_fixed() {
        let mut m = cube();
        m.faces.iter_mut().for_each(|f| f.reverse());
        assert_eq!(inspect(&m, TOL).count(IssueKind::InvertedSolid), 1);
        let (fixed, s) = repair(&m, TOL);
        assert_eq!(s.inverted_solids_flipped, 1);
        assert!(inspect(&fixed, TOL).is_solid());
        assert!((signed_volume(&fixed) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn internal_face_is_non_manifold() {
        let mut m = cube();
        m.faces.push(vec![0, 1, 3, 2]); // shares edges, forms a third face on some edges
        m.faces.push(vec![0, 3, 7]);
        let r = inspect(&m, TOL);
        assert!(r.count(IssueKind::NonManifoldEdge) > 0);
    }

    #[test]
    fn duplicate_and_zero_area_faces() {
        let mut m = cube();
        m.faces.push(m.faces[0].clone());
        m.faces.push(vec![0, 1, 1]);
        let r = inspect(&m, TOL);
        assert_eq!(r.count(IssueKind::DuplicateFace), 1);
        assert_eq!(r.count(IssueKind::ZeroAreaFace), 1);
    }

    #[test]
    fn short_edge_unused_and_coincident_vertices() {
        let mut m = cube();
        m.add_vertex([0.00001, 0.0, 0.0]); // unused, and coincident with vertex 0
        let r = inspect(&m, TOL);
        assert_eq!(r.count(IssueKind::UnusedVertex), 1);
        assert_eq!(r.count(IssueKind::CoincidentVertices), 1);
        assert!(r.is_solid(), "these do not break the solid");

        let mut s = Mesh::new();
        let (a, b, c) = (s.add_vertex([0.0; 3]), s.add_vertex([0.00001, 0.0, 0.0]), s.add_vertex([0.0, 1.0, 0.0]));
        s.add_face(vec![a, b, c]);
        assert!(inspect(&s, TOL).count(IssueKind::ShortEdge) > 0);
    }

    #[test]
    fn repair_fixes_everything_it_can_in_one_pass() {
        let mut m = cube();
        m.faces[2].reverse(); // flipped face
        m.faces.remove(1); // hole in the top
        m.faces.push(m.faces[0].clone()); // duplicate
        m.faces.push(vec![0, 1, 1]); // degenerate
        m.add_vertex([9.0, 9.0, 9.0]); // unused
        let (fixed, s) = repair(&m, TOL);
        assert_eq!(s.removed_duplicate_faces, 1);
        assert_eq!(s.removed_degenerate_faces, 1);
        assert_eq!(s.removed_unused_vertices, 1);
        assert_eq!(s.reoriented_faces, 1);
        assert_eq!(s.filled_holes, 1);
        let r = inspect(&fixed, TOL);
        assert!(r.is_solid(), "{:?}", r.issues);
        assert!((r.volume.unwrap() - 1.0).abs() < 1e-5);
        assert_eq!(fixed.positions.len(), 8);
    }

    #[test]
    fn repair_welds_unwelded_vertices_into_a_solid() {
        // Every face has its own vertices, like a raw STL export.
        let c = cube();
        let mut m = Mesh::new();
        for f in &c.faces {
            let ring: Vec<u32> = f.iter().map(|&v| m.add_vertex(c.positions[v as usize])).collect();
            m.add_face(ring);
        }
        assert_eq!(inspect(&m, TOL).count(IssueKind::SurfaceBorder), 24);
        let (fixed, s) = repair(&m, TOL);
        assert_eq!(s.welded_vertices, 16);
        assert!(inspect(&fixed, TOL).is_solid());
        assert_eq!(fixed.positions.len(), 8);
    }

    #[test]
    fn clean_mesh_needs_no_repair() {
        let (fixed, s) = repair(&cube(), TOL);
        assert!(!s.changed_anything(), "{s:?}");
        assert_eq!(fixed, cube());
    }
}
