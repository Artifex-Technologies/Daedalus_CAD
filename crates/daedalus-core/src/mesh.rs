use crate::halfedge::ray_triangle;
use crate::triangulate::triangulate;

/// Indexed polygon mesh. Faces may have any number of vertices (>= 3).
///
/// This is the interchange representation used by importers/exporters.
/// An editable half-edge structure will be built on top of it later.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub faces: Vec<Vec<u32>>,
    /// Loose edges: lines drawn that do not border a face (yet), as SketchUp keeps them. Vertex pairs.
    pub lines: Vec<[u32; 2]>,
    /// Face edges that are not drawn: the seams that join the two faces around a hole (see `imprint::cut_hole`).
    pub seams: Vec<[u32; 2]>,
    /// Paint Bucket colours, one per face (None: the default material). May be shorter than `faces`: the rest are unpainted.
    pub colors: Vec<Option<[u8; 3]>>,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_vertex(&mut self, p: [f32; 3]) -> u32 {
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

    /// Removes face `f` and its colour; later faces move down one.
    pub fn remove_face(&mut self, f: usize) -> Vec<u32> {
        if f < self.colors.len() {
            self.colors.remove(f);
        }
        self.faces.remove(f)
    }

    /// Axis-aligned bounds as (min, max), or None if the mesh has no vertices.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
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
            .faces
            .iter()
            .flat_map(|f| (0..f.len()).map(move |i| (f[i], f[(i + 1) % f.len()])))
            .map(|(a, b)| (a.min(b), a.max(b)))
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
    pub fn pick_face(&self, origin: [f32; 3], dir: [f32; 3]) -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (f, ring) in self.faces.iter().enumerate() {
            for tri in triangulate(&self.positions, ring) {
                if let Some(t) = ray_triangle(origin, dir, tri.map(|v| self.positions[v as usize])) {
                    if best.is_none_or(|(_, bt)| t < bt) {
                        best = Some((f, t));
                    }
                }
            }
        }
        best
    }

    /// Triangulates every face (ear clipping, so concave faces are correct).
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        self.faces.iter().flat_map(|f| triangulate(&self.positions, f)).collect()
    }

    /// This mesh with every point sent through `xf`. A mirroring transform (negative determinant) turns the faces round, so
    /// they still face outward.
    pub fn transformed(&self, xf: &crate::scene::Xform) -> Mesh {
        let mut m = self.clone();
        for p in &mut m.positions {
            *p = xf.apply(*p);
        }
        if xf.det() < 0.0 {
            for f in &mut m.faces {
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
    }

    /// Like [`Mesh::append`], but a vertex of `other` within `tol` of one already here becomes that vertex, so the two
    /// share their touching corners and edges.
    pub fn append_welded(&mut self, other: &Mesh, tol: f32) {
        let key = |p: [f32; 3]| p.map(|c| (c / tol).round() as i64);
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
        for v in self.faces.iter().flatten().chain(self.lines.iter().flatten()).chain(self.seams.iter().flatten()) {
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
        for f in &mut self.faces {
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
}
