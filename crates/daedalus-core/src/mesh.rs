/// Indexed polygon mesh. Faces may have any number of vertices (>= 3).
///
/// This is the interchange representation used by importers/exporters.
/// An editable half-edge structure will be built on top of it later.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub faces: Vec<Vec<u32>>,
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

    /// Fan-triangulates every face. Correct for convex faces only.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut out = Vec::new();
        for f in &self.faces {
            for i in 1..f.len().saturating_sub(1) {
                out.push([f[0], f[i], f[i + 1]]);
            }
        }
        out
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
        assert_eq!(m.bounds(), Some(([0.0; 3], [1.0, 1.0, 0.0])));
    }
}
