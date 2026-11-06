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
