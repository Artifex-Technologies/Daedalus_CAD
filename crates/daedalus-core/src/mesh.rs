use crate::halfedge::ray_triangle;
use crate::triangulate::{triangulate, triangulate_with_holes};

/// Indexed polygon mesh. Faces may have any number of vertices (>= 3).
///
/// This is the interchange representation used by importers/exporters.
/// An editable half-edge structure will be built on top of it later.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f64; 3]>,
    pub faces: Vec<Vec<u32>>,
    /// Loose edges: lines drawn that do not border a face (yet), as SketchUp keeps them. Vertex pairs.
    pub lines: Vec<[u32; 2]>,
    /// Face edges that are not drawn: the seams that join the two faces around a hole (see `imprint::cut_hole`).
    pub seams: Vec<[u32; 2]>,
    /// Paint Bucket colours, one per face (None: the default material). May be shorter than `faces`: the rest are unpainted.
    pub colors: Vec<Option<[u8; 3]>>,
    /// The openings in each face (a wall's window): loops of vertices wound the other way round from the face's outline, in
    /// the face's plane and inside it. May be shorter than `faces`: the rest have none.
    pub holes: Vec<Vec<Vec<u32>>>,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_vertex(&mut self, p: [f64; 3]) -> u32 {
        self.positions.push(p);
        (self.positions.len() - 1) as u32
    }

    pub fn add_face(&mut self, indices: Vec<u32>) {
        debug_assert!(indices.len() >= 3);
        self.faces.push(indices);
    }

    /// The colour face `f` is painted, if any.
    pub fn color_of(&self, f: usize) -> Option<[u8; 3]> {
        self.colors.get(f).copied().flatten()
    }

    /// Paints face `f` (None: back to the default material).
    pub fn set_color(&mut self, f: usize, c: Option<[u8; 3]>) {
        if self.colors.len() <= f {
            if c.is_none() {
                return;
            }
            self.colors.resize(f + 1, None);
        }
        self.colors[f] = c;
    }

    /// The openings in face `f` (none for most faces).
    pub fn holes_of(&self, f: usize) -> &[Vec<u32>] {
        self.holes.get(f).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Sets the openings in face `f`.
    pub fn set_holes(&mut self, f: usize, holes: Vec<Vec<u32>>) {
        if self.holes.len() <= f {
            if holes.is_empty() {
                return;
            }
            self.holes.resize(f + 1, Vec::new());
        }
        self.holes[f] = holes;
    }

    /// Face `f`'s outline, then its holes.
    pub fn loops(&self, f: usize) -> impl Iterator<Item = &Vec<u32>> {
        std::iter::once(&self.faces[f]).chain(self.holes_of(f))
    }

    /// Each face's edges as directed vertex pairs, outline and holes: (face, from, to).
    pub fn face_edges(&self) -> impl Iterator<Item = (usize, u32, u32)> + '_ {
        (0..self.faces.len()).flat_map(move |f| self.loops(f).flat_map(move |r| (0..r.len()).map(move |i| (f, r[i], r[(i + 1) % r.len()]))))
    }

    /// Removes face `f`, its colour and its holes; later faces move down one. Returns its outline (its holes are in
    /// [`Mesh::remove_face_loops`]).
    pub fn remove_face(&mut self, f: usize) -> Vec<u32> {
        self.remove_face_loops(f).0
    }

    /// Removes face `f`, its colour and its holes; later faces move down one. Returns its outline and its holes.
    pub fn remove_face_loops(&mut self, f: usize) -> (Vec<u32>, Vec<Vec<u32>>) {
        if f < self.colors.len() {
            self.colors.remove(f);
        }
        let holes = if f < self.holes.len() { self.holes.remove(f) } else { Vec::new() };
        (self.faces.remove(f), holes)
    }

    /// The triangles of face `f`, holes left open.
    pub fn triangulate_face(&self, f: usize) -> Vec<[u32; 3]> {
        let holes = self.holes_of(f);
        if holes.is_empty() {
            triangulate(&self.positions, &self.faces[f])
        } else {
            triangulate_with_holes(&self.positions, &self.faces[f], holes)
        }
    }

    /// How close two points or planes must be to count as the same, for this mesh: its size x 1e-7 (a 10 m house: 1 um; a
    /// 10 mm part: 1 nm), so small models keep their detail and large ones are not split by rounding. At least 1e-10 m.
    pub fn tolerance(&self) -> f64 {
        let size = self.bounds().map_or(0.0, |(lo, hi)| (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f64>().sqrt());
        let reach = self.positions.iter().flatten().fold(0.0f64, |m, c| m.max(c.abs()));
        // far from the origin, rounding grows with the coordinates themselves
        (size * 1e-7).max(reach * 1e-12).max(1e-10)
    }

    /// Axis-aligned bounds as (min, max), or None if the mesh has no vertices.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let first = *self.positions.first()?;
        Some(self.positions.iter().fold((first, first), |(mut lo, mut hi), p| {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
            (lo, hi)
        }))
    }

    /// Undirected edges as (low, high) vertex pairs, each listed once, sorted.
    pub fn edges(&self) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = self
            .face_edges()
            .map(|(_, a, b)| (a.min(b), a.max(b)))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Every edge to draw and snap to: the faces' edges and the loose lines, (low, high), each once, sorted.
    pub fn all_edges(&self) -> Vec<(u32, u32)> {
        let mut out = self.edges();
        out.extend(self.lines.iter().map(|&[a, b]| (a.min(b), a.max(b))));
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The edges to draw and snap to: [`Mesh::all_edges`] without the hidden seams.
    pub fn visible_edges(&self) -> Vec<(u32, u32)> {
        let hidden: std::collections::HashSet<(u32, u32)> = self.seams.iter().map(|&[a, b]| (a.min(b), a.max(b))).collect();
        self.all_edges().into_iter().filter(|e| !hidden.contains(e)).collect()
    }

    /// Nearest face hit by a ray, as (face index, distance along the ray).
    /// Faces are treated as two-sided. Works on any mesh, including
    /// non-manifold imports that cannot become a half-edge mesh.
    pub fn pick_face(&self, origin: [f64; 3], dir: [f64; 3]) -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for f in 0..self.faces.len() {
            for tri in self.triangulate_face(f) {
                if let Some(t) = ray_triangle(origin, dir, tri.map(|v| self.positions[v as usize])) {
                    if best.is_none_or(|(_, bt)| t < bt) {
                        best = Some((f, t));
                    }
                }
            }
        }
        best
    }

    /// Each face's triangles (ear clipping), in face order: worked out once and kept by a caller that picks or draws often.
    pub fn face_triangles(&self) -> Vec<Vec<[u32; 3]>> {
        (0..self.faces.len()).map(|f| self.triangulate_face(f)).collect()
    }

    /// [`Mesh::pick_face`] with the triangles already worked out ([`Mesh::face_triangles`] of this mesh): no ear clipping
    /// on every call, so it is quick enough for every mouse move on a large model.
    pub fn pick_triangles(&self, tris: &[Vec<[u32; 3]>], origin: [f64; 3], dir: [f64; 3]) -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for (f, ts) in tris.iter().enumerate() {
            for tri in ts {
                if let Some(t) = ray_triangle(origin, dir, tri.map(|v| self.positions[v as usize])) {
                    if best.is_none_or(|(_, bt)| t < bt) {
                        best = Some((f, t));
                    }
                }
            }
        }
        best
    }

    /// Every face as a plain polygon, for formats that cannot hold holes: a face without holes as it is, one with holes as
    /// its triangles. Colours are not carried. Returns the polygons and, for each, the face it came from.
    pub fn polygons(&self) -> (Vec<Vec<u32>>, Vec<usize>) {
        let mut out = Vec::new();
        let mut from = Vec::new();
        for f in 0..self.faces.len() {
            if self.holes_of(f).is_empty() {
                out.push(self.faces[f].clone());
                from.push(f);
            } else {
                for t in self.triangulate_face(f) {
                    out.push(t.to_vec());
                    from.push(f);
                }
            }
        }
        (out, from)
    }

    /// Triangulates every face (ear clipping, so concave faces are correct).
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        (0..self.faces.len()).flat_map(|f| self.triangulate_face(f)).collect()
    }

    /// This mesh with every point sent through `xf`. A mirroring transform (negative determinant) turns the faces round, so
    /// they still face outward.
    pub fn transformed(&self, xf: &crate::scene::Xform) -> Mesh {
        let mut m = self.clone();
        for p in &mut m.positions {
            *p = xf.apply(*p);
        }
        if xf.det() < 0.0 {
            for f in m.faces.iter_mut().chain(m.holes.iter_mut().flatten()) {
                f.reverse();
            }
        }
        m
    }

    /// Adds `other`'s vertices, faces, lines, seams and colours after this mesh's own (nothing is joined).
    pub fn append(&mut self, other: &Mesh) {
        let base = self.positions.len() as u32;
        let faces_before = self.faces.len();
        self.positions.extend_from_slice(&other.positions);
        self.faces.extend(other.faces.iter().map(|f| f.iter().map(|v| v + base).collect()));
        self.lines.extend(other.lines.iter().map(|l| l.map(|v| v + base)));
        self.seams.extend(other.seams.iter().map(|l| l.map(|v| v + base)));
        for (i, c) in other.colors.iter().enumerate() {
            if c.is_some() {
                self.set_color(faces_before + i, *c);
            }
        }
        for (i, h) in other.holes.iter().enumerate() {
            self.set_holes(faces_before + i, h.iter().map(|r| r.iter().map(|v| v + base).collect()).collect());
        }
    }

    /// Like [`Mesh::append`], but a vertex of `other` within `tol` of one already here becomes that vertex, so the two
    /// share their touching corners and edges.
    pub fn append_welded(&mut self, other: &Mesh, tol: f64) {
        let key = |p: [f64; 3]| p.map(|c| (c / tol).round() as i64);
        let mut at: std::collections::HashMap<[i64; 3], u32> = std::collections::HashMap::new();
        for (i, &p) in self.positions.iter().enumerate() {
            at.entry(key(p)).or_insert(i as u32);
        }
        let map: Vec<u32> = other
            .positions
            .iter()
            .map(|&p| match at.get(&key(p)) {
                Some(&v) => v,
                None => {
                    let v = self.add_vertex(p);
                    at.insert(key(p), v);
                    v
                }
            })
            .collect();
        let faces_before = self.faces.len();
        self.faces.extend(other.faces.iter().map(|f| f.iter().map(|&v| map[v as usize]).collect()));
        self.lines.extend(other.lines.iter().map(|l| l.map(|v| map[v as usize])));
        self.seams.extend(other.seams.iter().map(|l| l.map(|v| map[v as usize])));
        for (i, c) in other.colors.iter().enumerate() {
            if c.is_some() {
                self.set_color(faces_before + i, *c);
            }
        }
        for (i, h) in other.holes.iter().enumerate() {
            self.set_holes(faces_before + i, h.iter().map(|r| r.iter().map(|&v| map[v as usize]).collect()).collect());
        }
    }

    /// Each face made of pieces joined by hidden seams (how a face with a hole was kept before faces had holes) becomes one
    /// face with its holes again. Only pieces lying in one plane are joined: a seam between faces at an angle is a soft edge
    /// (a curved wall) and stays. Returns how many faces were joined up.
    pub fn join_seamed_faces(&mut self) -> usize {
        use std::collections::{BTreeMap, HashMap, HashSet};
        if self.seams.is_empty() {
            return 0;
        }
        let key = |a: u32, b: u32| (a.min(b), a.max(b));
        let seams: HashSet<(u32, u32)> = self.seams.iter().map(|&[a, b]| key(a, b)).collect();
        let tol = self.tolerance() * 10.0;
        let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (f, a, b) in self.face_edges() {
            if seams.contains(&key(a, b)) {
                by_edge.entry(key(a, b)).or_default().push(f);
            }
        }
        let mut parent: Vec<usize> = (0..self.faces.len()).collect();
        fn find(p: &mut [usize], mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        for faces in by_edge.values() {
            if let [a, b] = faces[..] {
                if a != b && crate::merge::coplanar(self, a, b, tol) && self.color_of(a) == self.color_of(b) {
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                    parent[ra] = rb;
                }
            }
        }
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for f in 0..self.faces.len() {
            let r = find(&mut parent, f);
            groups.entry(r).or_default().push(f);
        }
        let mut joined = 0;
        let mut gone: Vec<usize> = Vec::new();
        for members in groups.values().filter(|g| g.len() > 1) {
            // the group's border: directed edges whose reverse is not in it (the seams inside cancel out)
            let directed: HashSet<(u32, u32)> = members.iter().flat_map(|&f| self.loops(f).flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()])))).collect();
            let mut next: HashMap<u32, u32> = HashMap::new();
            let mut pinched = false;
            for &(a, b) in &directed {
                if !directed.contains(&(b, a)) && next.insert(a, b).is_some() {
                    pinched = true;
                }
            }
            if pinched || next.is_empty() {
                continue;
            }
            // the border in loops
            let mut loops: Vec<Vec<u32>> = Vec::new();
            let mut left: HashSet<u32> = next.keys().copied().collect();
            while let Some(&start) = left.iter().min() {
                let mut lp = vec![start];
                left.remove(&start);
                let mut at = next[&start];
                while at != start {
                    if !left.remove(&at) {
                        break;
                    }
                    lp.push(at);
                    at = match next.get(&at) {
                        Some(&n) => n,
                        None => break,
                    };
                }
                loops.push(lp);
            }
            // the outline is the loop wound like the pieces (largest area that way); the rest are holes
            let n = crate::imprint::face_plane(self, &self.faces[members[0]]).map(|(n, _)| n).unwrap_or([0.0, 0.0, 1.0]);
            // twice the area, signed by the way round it runs about the pieces' normal
            let area = |r: &Vec<u32>| {
                let mut s = [0.0f64; 3];
                for i in 0..r.len() {
                    let (p, q) = (self.positions[r[i] as usize], self.positions[r[(i + 1) % r.len()] as usize]);
                    s[0] += (p[1] - q[1]) * (p[2] + q[2]);
                    s[1] += (p[2] - q[2]) * (p[0] + q[0]);
                    s[2] += (p[0] - q[0]) * (p[1] + q[1]);
                }
                s[0] * n[0] + s[1] * n[1] + s[2] * n[2]
            };
            let Some(outer) = (0..loops.len()).max_by(|&a, &b| area(&loops[a]).total_cmp(&area(&loops[b]))) else { continue };
            if area(&loops[outer]) <= 0.0 || loops.iter().enumerate().any(|(i, l)| i != outer && area(l) >= 0.0) {
                continue; // not one outline round holes: left as it is
            }
            let first = members[0];
            let outline = loops.remove(outer);
            self.faces[first] = outline;
            self.set_holes(first, loops);
            gone.extend(members[1..].iter().copied());
            joined += 1;
        }
        gone.sort_unstable_by(|a, b| b.cmp(a));
        for f in gone {
            self.remove_face(f);
        }
        let edges: HashSet<(u32, u32)> = self.edges().into_iter().collect();
        self.seams.retain(|&[a, b]| edges.contains(&key(a, b)));
        joined
    }

    /// Splits off `faces` (Make Group): returns (what is left, the faces taken). The faces taken keep their colours and
    /// seams; an edge they share with a face left behind stays with that face too, so each side keeps a whole outline.
    /// Both come back without unused vertices.
    pub fn extract(&self, faces: &[usize]) -> (Mesh, Mesh) {
        let take: std::collections::HashSet<usize> = faces.iter().copied().collect();
        let mut rest = Mesh { positions: self.positions.clone(), lines: self.lines.clone(), seams: self.seams.clone(), ..Default::default() };
        let mut part = Mesh { positions: self.positions.clone(), seams: self.seams.clone(), ..Default::default() };
        for (i, f) in self.faces.iter().enumerate() {
            let side = if take.contains(&i) { &mut part } else { &mut rest };
            side.faces.push(f.clone());
            let n = side.faces.len() - 1;
            side.set_color(n, self.color_of(i));
            side.set_holes(n, self.holes_of(i).to_vec());
        }
        for m in [&mut rest, &mut part] {
            let edges: std::collections::HashSet<(u32, u32)> = m.edges().into_iter().collect();
            m.seams.retain(|&[a, b]| edges.contains(&(a.min(b), a.max(b))));
            m.compact();
        }
        (rest, part)
    }

    /// Drops vertices no face, line or seam uses, renumbering the rest in order.
    pub fn compact(&mut self) {
        let mut used = vec![false; self.positions.len()];
        for v in self.faces.iter().flatten().chain(self.holes.iter().flatten().flatten()).chain(self.lines.iter().flatten()).chain(self.seams.iter().flatten()) {
            used[*v as usize] = true;
        }
        let mut map = vec![u32::MAX; self.positions.len()];
        let mut kept = Vec::new();
        for (i, p) in self.positions.iter().enumerate() {
            if used[i] {
                map[i] = kept.len() as u32;
                kept.push(*p);
            }
        }
        self.positions = kept;
        for f in self.faces.iter_mut().chain(self.holes.iter_mut().flatten()) {
            for v in f.iter_mut() {
                *v = map[*v as usize];
            }
        }
        for l in self.lines.iter_mut().chain(self.seams.iter_mut()) {
            *l = l.map(|v| map[v as usize]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_triangulates_to_two() {
        let mut m = Mesh::new();
        let a = m.add_vertex([0.0, 0.0, 0.0]);
        let b = m.add_vertex([1.0, 0.0, 0.0]);
        let c = m.add_vertex([1.0, 1.0, 0.0]);
        let d = m.add_vertex([0.0, 1.0, 0.0]);
        m.add_face(vec![a, b, c, d]);
        assert_eq!(m.triangles().len(), 2);
        assert_eq!(m.edges(), vec![(0, 1), (0, 3), (1, 2), (2, 3)]);
        assert_eq!(m.bounds(), Some(([0.0; 3], [1.0, 1.0, 0.0])));
    }

    #[test]
    fn an_old_ring_of_two_seamed_faces_becomes_one_face_with_a_hole() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [9.0, 0.0, 0.0], [9.0, 9.0, 0.0], [0.0, 9.0, 0.0], [2.0, 2.0, 0.0], [7.0, 2.0, 0.0], [7.0, 7.0, 0.0], [2.0, 7.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        // the old way: the ring as two faces joined by two seams, the inside a face of its own
        crate::imprint::cut_hole_avoiding(&mut m, 0, &[4, 5, 6, 7], &[]).unwrap();
        assert_eq!((m.faces.len(), m.seams.len()), (3, 2));
        assert_eq!(m.join_seamed_faces(), 1);
        assert_eq!(m.faces.len(), 2, "the ring and the square inside it");
        assert!(m.seams.is_empty());
        let ring = (0..2).find(|&f| !m.holes_of(f).is_empty()).expect("one face has the hole");
        assert_eq!(m.faces[ring].len(), 4);
        assert_eq!(m.holes_of(ring)[0].len(), 4);
        let tris = m.triangulate_face(ring);
        let area: f64 = tris.iter().map(|t| {
            let [a, b, c] = t.map(|v| m.positions[v as usize]);
            ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.0
        }).sum();
        assert!((area - 56.0).abs() < 1e-9, "{area}");
    }
}
