use crate::{HalfEdgeMesh, TopologyError};

/// Within this distance (model units, metres) two faces' planes count as one when coplanar faces are merged.
pub const MERGE_TOLERANCE: f32 = 1e-5;

/// Merges coplanar faces of `m` in place; returns where `face` went (or `face` when nothing could be rebuilt).
fn merge_in_place(m: &mut HalfEdgeMesh, face: usize) -> usize {
    let (merged, map) = crate::merge::merge_coplanar(&m.to_mesh(), MERGE_TOLERANCE);
    match HalfEdgeMesh::from_mesh(&merged) {
        Ok(h) => {
            *m = h;
            map.get(face).copied().unwrap_or(face)
        }
        // never expected; the push stays as it was rather than being undone
        Err(_) => face,
    }
}

/// The editable document: a mesh plus an undo/redo history.
///
/// Every edit is one atomic history step; a failed edit leaves the model
/// untouched. History currently stores whole-mesh snapshots, which is simple
/// and correct; it should move to diffs/commands once models get large.
#[derive(Debug, Clone, Default)]
pub struct Model {
    mesh: HalfEdgeMesh,
    undo: Vec<(HalfEdgeMesh, u64)>,
    redo: Vec<(HalfEdgeMesh, u64)>,
    /// Which state the mesh is in: a new number for every edit, carried along by undo and redo, so "unchanged since saving"
    /// is a comparison with the number at the save (undoing back to it counts as unchanged).
    state: u64,
    next_state: u64,
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

    /// The state the model is in now (see the field). Keep it at a save; the model is unchanged while this equals it.
    pub fn state(&self) -> u64 {
        self.state
    }

    fn edit<R>(
        &mut self,
        f: impl FnOnce(&mut HalfEdgeMesh) -> Result<R, TopologyError>,
    ) -> Result<R, TopologyError> {
        let before = self.mesh.clone();
        match f(&mut self.mesh) {
            Ok(r) => {
                self.undo.push((before, self.state));
                self.redo.clear();
                self.next_state += 1;
                self.state = self.next_state;
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

    /// Adds a closed outline as a new face (Line, Rectangle, Circle and Polygon tools).
    pub fn add_polygon(&mut self, points: &[[f32; 3]]) -> Result<usize, TopologyError> {
        self.edit(|m| m.add_polygon(points))
    }

    pub fn erase_face(&mut self, face: usize) -> Result<(), TopologyError> {
        self.edit(|m| m.erase_face(face))
    }

    /// Line tool: one segment, as one undo step. Returns the face it closed, if it closed one.
    pub fn add_line(&mut self, a: [f32; 3], b: [f32; 3]) -> Result<Option<usize>, TopologyError> {
        self.edit(|m| m.add_line(a, b))
    }

    /// A shape's outline (Rectangle, Circle, Polygon, Pie ...) drawn as its lines in one undo step, so it joins and cuts what
    /// is there like lines do: on a face it splits it, or cuts a hole that it fills. Returns the face it made, if any.
    pub fn add_outline(&mut self, points: &[[f32; 3]]) -> Result<Option<usize>, TopologyError> {
        if points.len() < 3 {
            return Err(TopologyError::DegenerateFace(self.mesh.face_count()));
        }
        self.edit(|m| {
            let mut made = None;
            for i in 0..points.len() {
                if let Some(f) = m.add_line(points[i], points[(i + 1) % points.len()])? {
                    made = Some(f);
                }
            }
            Ok(made)
        })
    }

    pub fn erase_line(&mut self, i: usize) -> Result<(), TopologyError> {
        self.edit(|m| m.erase_line(i))
    }

    /// Erases several faces as one undo step (Delete on a selection). Highest numbers first, so the others keep theirs.
    pub fn erase_faces(&mut self, faces: &[usize]) -> Result<(), TopologyError> {
        let mut faces = faces.to_vec();
        faces.sort_unstable_by(|a, b| b.cmp(a));
        faces.dedup();
        self.edit(|m| faces.iter().try_for_each(|&f| m.erase_face(f)))
    }

    /// Pushes or pulls a face, then merges coplanar faces as SketchUp does (pushing a face of a solid again leaves no seam).
    /// Returns the pushed face's index after the merge.
    pub fn push_pull(&mut self, face: usize, distance: f32) -> Result<usize, TopologyError> {
        self.edit(|m| {
            let f = m.push_pull(face, distance)?;
            Ok(merge_in_place(m, f))
        })
    }

    /// Merges coplanar neighbouring faces (Tools > Merge Coplanar Faces). Returns how many faces went.
    pub fn merge_coplanar(&mut self) -> Result<usize, TopologyError> {
        let before = self.mesh.face_count();
        let n = self.edit(|m| {
            merge_in_place(m, 0);
            Ok(before - m.face_count())
        })?;
        if n == 0 {
            // nothing merged: not a step worth undoing
            self.undo();
            self.redo.clear();
        }
        Ok(n)
    }

    /// Move, Rotate and Scale: puts each of `vertices` where `f` sends it, as one undo step. Faces keep their numbers.
    /// Faces that share some of the vertices but not all of them stretch to follow, as in SketchUp.
    pub fn transform_vertices(&mut self, vertices: &[u32], f: impl Fn([f32; 3]) -> [f32; 3]) -> Result<(), TopologyError> {
        self.edit(|m| {
            for &v in vertices {
                if let Some(p) = m.positions.get_mut(v as usize) {
                    *p = f(*p);
                }
            }
            Ok(())
        })
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
            Some((prev, state)) => {
                self.redo.push((std::mem::replace(&mut self.mesh, prev), self.state));
                self.state = state;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some((next, state)) => {
                self.undo.push((std::mem::replace(&mut self.mesh, next), self.state));
                self.state = state;
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
    fn pushing_a_solid_face_again_merges_the_seam() {
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [3.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        let top = m.push_pull(f, 1.0).unwrap();
        let top = m.push_pull(top, 0.5).unwrap();
        assert_eq!(m.mesh().face_count(), 6);
        assert_eq!(m.mesh().vertex_count(), 8);
        assert!((m.mesh().signed_volume() - 9.0).abs() < 1e-4);
        assert_eq!(m.mesh().face_vertices(top).len(), 4);
        // the top can be pushed once more, by its returned index
        m.push_pull(top, -0.5).unwrap();
        assert!((m.mesh().signed_volume() - 6.0).abs() < 1e-4);
        m.undo();
        assert!((m.mesh().signed_volume() - 9.0).abs() < 1e-4, "push and merge are one undo step");
        let state = m.state();
        assert_eq!(m.merge_coplanar().unwrap(), 0);
        assert_eq!(m.state(), state, "merging nothing is no step");
    }

    #[test]
    fn the_state_follows_undo_and_redo() {
        let mut m = Model::new();
        let saved = m.state();
        let f = m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        let drawn = m.state();
        assert_ne!(drawn, saved);
        m.undo();
        assert_eq!(m.state(), saved, "undone back to the save: unchanged");
        m.redo();
        assert_eq!(m.state(), drawn);
        m.push_pull(f, 1.0).unwrap();
        m.undo();
        m.undo();
        m.add_rectangle([5.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert_ne!(m.state(), saved, "a different edit from the same start is a change");
        assert_ne!(m.state(), drawn);
        assert!(m.push_pull(99, 1.0).is_err());
        let before = m.state();
        assert_eq!(m.state(), before, "a failed edit changes nothing");
    }

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
    fn degenerate_outline_is_rejected_without_a_history_step() {
        let mut m = Model::new();
        assert!(m.add_polygon(&[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]).is_err());
        assert_eq!(m.mesh().face_count(), 0);
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

    #[test]
    fn lines_stay_until_they_close_a_face() {
        let mut m = Model::new();
        assert_eq!(m.add_line([0.0, 0.0, 0.0], [3.0, 0.0, 0.0]).unwrap(), None);
        assert_eq!(m.add_line([3.0, 0.0, 0.0], [3.0, 2.0, 0.0]).unwrap(), None);
        assert_eq!(m.add_line([3.0, 2.0, 0.0], [0.0, 2.0, 0.0]).unwrap(), None);
        assert_eq!(m.mesh().lines.len(), 3, "three loose lines, no face yet");
        assert_eq!(m.mesh().vertex_count(), 4, "corners are shared");
        let face = m.add_line([0.0, 2.0, 0.0], [0.0, 0.0, 0.0]).unwrap().expect("the fourth line closes the face");
        assert!(m.mesh().lines.is_empty(), "the lines became the face's edges");
        assert_eq!(m.mesh().face_normal(face), [0.0, 0.0, 1.0], "a floor faces up");
        m.push_pull(face, 1.0).unwrap();
        assert!((m.mesh().signed_volume() - 6.0).abs() < 1e-4);
        // each line was its own step
        m.undo();
        m.undo();
        assert_eq!(m.mesh().lines.len(), 3);
    }

    #[test]
    fn a_wall_from_lines_and_a_crooked_loop() {
        let mut m = Model::new();
        let pts = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 0.0, 3.0], [0.0, 0.0, 3.0]];
        let mut face = None;
        for i in 0..4 {
            face = m.add_line(pts[i], pts[(i + 1) % 4]).unwrap();
        }
        let n = m.mesh().face_normal(face.unwrap());
        assert!((n[1] + 1.0).abs() < 1e-6, "an upright face looks to the front: {n:?}");

        // four lines that do not lie in one plane stay lines
        let mut m = Model::new();
        let pts = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 2.0, 1.0], [0.0, 2.0, 0.0]];
        for i in 0..4 {
            assert_eq!(m.add_line(pts[i], pts[(i + 1) % 4]).unwrap(), None);
        }
        assert_eq!(m.mesh().lines.len(), 4);
        assert_eq!(m.mesh().face_count(), 0);
        // the same line twice is one line
        m.add_line([0.0, 0.0, 0.0], [2.0, 0.0, 0.0]).unwrap();
        assert_eq!(m.mesh().lines.len(), 4);
    }

    #[test]
    fn erasing_a_face_leaves_its_edges_and_lines_survive_edits() {
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.erase_face(f).unwrap();
        assert_eq!(m.mesh().face_count(), 0);
        assert_eq!(m.mesh().lines.len(), 4, "SketchUp keeps the edges");
        // a box elsewhere, pushed (which merges coplanar faces): the lines stay
        let b = m.add_rectangle([5.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.push_pull(b, 1.0).unwrap();
        assert_eq!(m.mesh().lines.len(), 4);
        let mesh = m.mesh().to_mesh();
        assert!(mesh.lines.iter().all(|l| l.iter().all(|&v| mesh.positions[v as usize][0] < 2.0)), "still the old square");
        m.erase_line(0).unwrap();
        assert_eq!(m.mesh().lines.len(), 3);
    }

    #[test]
    fn a_line_across_a_face_splits_it() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        // from the middle of one side to the middle of the other: two faces, the sides get a corner each
        m.add_line([1.0, 0.0, 0.0], [1.0, 2.0, 0.0]).unwrap();
        assert_eq!(m.mesh().face_count(), 2);
        assert!(m.mesh().lines.is_empty());
        assert_eq!(m.mesh().vertex_count(), 6);
        // corner to corner: a triangle splits off
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        m.add_line([0.0, 0.0, 0.0], [2.0, 2.0, 0.0]).unwrap();
        assert_eq!(m.mesh().face_count(), 2);
        assert!((0..2).all(|f| m.mesh().face_vertices(f).len() == 3));
        // one half pulls up on its own
        let half = (0..2).find(|&f| m.mesh().face_normal(f)[2] > 0.9).unwrap();
        m.push_pull(half, 1.0).unwrap();
        assert!((m.mesh().signed_volume() - 2.0).abs() < 1e-4, "a triangular prism of 2 x 2 / 2 x 1");
    }

    #[test]
    fn crossing_lines_meet() {
        let mut m = Model::new();
        m.add_line([0.0, 1.0, 0.0], [2.0, 1.0, 0.0]).unwrap();
        m.add_line([1.0, 0.0, 0.0], [1.0, 2.0, 0.0]).unwrap();
        assert_eq!(m.mesh().lines.len(), 4, "both lines broke where they cross");
        assert_eq!(m.mesh().vertex_count(), 5);
        // a line ending on another's middle breaks it too
        m.add_line([0.0, 1.0, 0.0], [0.0, 3.0, 0.0]).unwrap();
        m.add_line([-1.0, 2.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        assert_eq!(m.mesh().lines.len(), 7);
    }

    #[test]
    fn lines_close_a_face_against_a_face_edge() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        m.add_line([2.0, 0.0, 0.0], [3.0, 0.0, 0.0]).unwrap();
        m.add_line([3.0, 0.0, 0.0], [3.0, 2.0, 0.0]).unwrap();
        let f = m.add_line([3.0, 2.0, 0.0], [2.0, 2.0, 0.0]).unwrap();
        assert!(f.is_some(), "the square's side closes the new face");
        assert_eq!(m.mesh().face_count(), 2);
        assert!(m.mesh().lines.is_empty());
        // the two faces share the side: pulled together they make one closed box
        let up = m.mesh().to_mesh();
        assert_eq!(up.edges().len(), 7);
    }

    #[test]
    fn a_square_drawn_inside_a_face_becomes_a_hole_that_pushes_in() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]).unwrap();
        let top = m.push_pull(0, 2.0).unwrap();
        assert_eq!(m.mesh().face_normal(top), [0.0, 0.0, 1.0]);
        let sq = [[1.0, 1.0, 2.0], [3.0, 1.0, 2.0], [3.0, 3.0, 2.0], [1.0, 3.0, 2.0]];
        let mut inner = None;
        for i in 0..4 {
            inner = m.add_line(sq[i], sq[(i + 1) % 4]).unwrap();
        }
        let inner = inner.expect("the square closed into a face");
        assert!(m.mesh().lines.is_empty());
        assert_eq!(m.mesh().face_vertices(inner).len(), 4);
        assert!((m.mesh().face_normal(inner)[2] - 1.0).abs() < 1e-6, "faces up like the top around it");
        assert!(m.mesh().is_closed(), "still a closed solid");
        assert!((m.mesh().signed_volume() - 32.0).abs() < 1e-3);
        // the two seams of the ring around it are not drawn; the square's own edges are
        let mesh = m.mesh().to_mesh();
        assert_eq!(mesh.seams.len(), 2);
        assert_eq!(mesh.all_edges().len() - mesh.visible_edges().len(), 2);
        // pushed in by 1: a pocket of 2 x 2 x 1
        m.push_pull(inner, -1.0).unwrap();
        assert!(m.mesh().is_closed());
        assert!((m.mesh().signed_volume() - 28.0).abs() < 1e-3, "{}", m.mesh().signed_volume());
    }
}
