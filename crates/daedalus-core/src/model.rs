use crate::{HalfEdgeMesh, TopologyError};

/// The editable document: a mesh plus an undo/redo history.
///
/// Every edit is one atomic history step; a failed edit leaves the model
/// untouched. History currently stores whole-mesh snapshots, which is simple
/// and correct; it should move to diffs/commands once models get large.
#[derive(Debug, Clone, Default)]
pub struct Model {
    mesh: HalfEdgeMesh,
    undo: Vec<HalfEdgeMesh>,
    redo: Vec<HalfEdgeMesh>,
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_mesh(mesh: HalfEdgeMesh) -> Self {
        Model { mesh, ..Default::default() }
    }

    pub fn mesh(&self) -> &HalfEdgeMesh {
        &self.mesh
    }

    fn edit<R>(
        &mut self,
        f: impl FnOnce(&mut HalfEdgeMesh) -> Result<R, TopologyError>,
    ) -> Result<R, TopologyError> {
        let before = self.mesh.clone();
        match f(&mut self.mesh) {
            Ok(r) => {
                self.undo.push(before);
                self.redo.clear();
                Ok(r)
            }
            Err(e) => {
                self.mesh = before;
                Err(e)
            }
        }
    }

    /// Draws a rectangle spanned by edge vectors `u` and `v` from `origin`.
    pub fn add_rectangle(&mut self, origin: [f32; 3], u: [f32; 3], v: [f32; 3]) -> Result<usize, TopologyError> {
        let add = |a: [f32; 3], b: [f32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let pts = [origin, add(origin, u), add(add(origin, u), v), add(origin, v)];
        self.edit(|m| m.add_polygon(&pts))
    }

    pub fn push_pull(&mut self, face: usize, distance: f32) -> Result<usize, TopologyError> {
        self.edit(|m| m.push_pull(face, distance))
    }

    pub fn split_face(&mut self, face: usize, a: u32, b: u32) -> Result<usize, TopologyError> {
        self.edit(|m| m.split_face(face, a, b))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.mesh, prev));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.mesh, next));
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_pull_undo_redo() {
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        m.push_pull(f, 3.0).unwrap();
        assert_eq!(m.mesh().face_count(), 6);
        assert!((m.mesh().signed_volume() - 12.0).abs() < 1e-4);

        assert!(m.undo());
        assert_eq!(m.mesh().face_count(), 1);
        assert!(m.undo());
        assert_eq!(m.mesh().face_count(), 0);
        assert!(!m.undo());

        assert!(m.redo() && m.redo());
        assert_eq!(m.mesh().face_count(), 6);
        assert!(!m.redo());
    }

    #[test]
    fn failed_edit_changes_nothing_and_adds_no_history() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert!(m.push_pull(9, 1.0).is_err());
        assert_eq!(m.mesh().face_count(), 1);
        assert!(m.undo());
        assert!(!m.can_undo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.undo();
        m.add_rectangle([5.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert!(!m.can_redo());
    }
}
