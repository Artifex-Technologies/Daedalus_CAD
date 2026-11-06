use crate::Mesh;
use std::collections::HashMap;
use std::fmt;

const NONE: u32 = u32::MAX;

#[derive(Debug, Clone, Copy)]
struct HalfEdge {
    /// Vertex this half-edge starts at.
    origin: u32,
    /// Next half-edge around the same face (counter-clockwise).
    next: u32,
    /// Opposite half-edge, or NONE on a boundary.
    twin: u32,
    face: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TopologyError {
    /// A directed edge is used by more than one face: non-manifold geometry or
    /// inconsistent face winding.
    NonManifoldEdge { from: u32, to: u32 },
    DegenerateFace(usize),
    VertexOutOfRange(u32),
    /// The two vertices are not distinct, non-adjacent corners of the face.
    InvalidSplit,
    FaceOutOfRange(usize),
    /// The face has no usable normal (zero area), so it has no direction to push along.
    DegenerateNormal,
    ZeroDistance,
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopologyError::NonManifoldEdge { from, to } => {
                write!(f, "edge {from}->{to} is shared by more than one face with the same direction")
            }
            TopologyError::DegenerateFace(i) => write!(f, "face {i} has fewer than 3 distinct vertices"),
            TopologyError::VertexOutOfRange(v) => write!(f, "vertex index {v} out of range"),
            TopologyError::InvalidSplit => write!(f, "split vertices must be distinct, non-adjacent corners of the face"),
            TopologyError::FaceOutOfRange(i) => write!(f, "face {i} does not exist"),
            TopologyError::DegenerateNormal => write!(f, "face has zero area"),
            TopologyError::ZeroDistance => write!(f, "distance must be non-zero"),
        }
    }
}

impl std::error::Error for TopologyError {}

/// Editable half-edge mesh. Build one from an interchange [`Mesh`] with
/// [`HalfEdgeMesh::from_mesh`] and turn it back with [`HalfEdgeMesh::to_mesh`].
#[derive(Debug, Clone, Default)]
pub struct HalfEdgeMesh {
    pub positions: Vec<[f32; 3]>,
    half_edges: Vec<HalfEdge>,
    /// One half-edge per face.
    faces: Vec<u32>,
    /// Loose edges (lines that are no face's edge), as vertex pairs.
    pub lines: Vec<[u32; 2]>,
}

impl HalfEdgeMesh {
    pub fn from_mesh(mesh: &Mesh) -> Result<Self, TopologyError> {
        let mut he = HalfEdgeMesh { positions: mesh.positions.clone(), ..Default::default() };
        if let Some(&[a, b]) = mesh.lines.iter().find(|l| l.iter().any(|&v| v as usize >= mesh.positions.len())) {
            return Err(TopologyError::VertexOutOfRange(a.max(b)));
        }
        he.lines = mesh.lines.clone();
        let mut directed: HashMap<(u32, u32), u32> = HashMap::new();

        for (fi, face) in mesh.faces.iter().enumerate() {
            if face.len() < 3 {
                return Err(TopologyError::DegenerateFace(fi));
            }
            if let Some(&bad) = face.iter().find(|&&v| v as usize >= mesh.positions.len()) {
                return Err(TopologyError::VertexOutOfRange(bad));
            }
            let base = he.half_edges.len() as u32;
            let n = face.len() as u32;
            for (k, &origin) in face.iter().enumerate() {
                let to = face[(k + 1) % face.len()];
                if origin == to {
                    return Err(TopologyError::DegenerateFace(fi));
                }
                let id = base + k as u32;
                if directed.insert((origin, to), id).is_some() {
                    return Err(TopologyError::NonManifoldEdge { from: origin, to });
                }
                he.half_edges.push(HalfEdge {
                    origin,
                    next: base + (k as u32 + 1) % n,
                    twin: NONE,
                    face: fi as u32,
                });
            }
            he.faces.push(base);
        }

        for (&(a, b), &id) in &directed {
            if let Some(&t) = directed.get(&(b, a)) {
                he.half_edges[id as usize].twin = t;
            }
        }
        Ok(he)
    }

    pub fn to_mesh(&self) -> Mesh {
        let mut m = Mesh { positions: self.positions.clone(), faces: Vec::new(), lines: self.lines.clone() };
        for f in 0..self.faces.len() {
            m.faces.push(self.face_vertices(f));
        }
        m
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Number of undirected edges.
    pub fn edge_count(&self) -> usize {
        self.half_edges
            .iter()
            .enumerate()
            .filter(|(i, h)| h.twin == NONE || (*i as u32) < h.twin)
            .count()
    }

    /// Half-edges without a twin.
    pub fn boundary_edge_count(&self) -> usize {
        self.half_edges.iter().filter(|h| h.twin == NONE).count()
    }

    pub fn is_closed(&self) -> bool {
        self.boundary_edge_count() == 0
    }

    /// V - E + F, counting only vertices that belong to a face.
    pub fn euler_characteristic(&self) -> i64 {
        let mut used = vec![false; self.positions.len()];
        for h in &self.half_edges {
            used[h.origin as usize] = true;
        }
        let v = used.iter().filter(|&&u| u).count() as i64;
        v - self.edge_count() as i64 + self.face_count() as i64
    }

    pub fn face_vertices(&self, face: usize) -> Vec<u32> {
        self.face_loop(face).into_iter().map(|h| self.half_edges[h as usize].origin).collect()
    }

    fn face_loop(&self, face: usize) -> Vec<u32> {
        let start = self.faces[face];
        let mut out = vec![start];
        let mut h = self.half_edges[start as usize].next;
        while h != start {
            out.push(h);
            h = self.half_edges[h as usize].next;
        }
        out
    }

    /// Unit normal by Newell's method; zero vector for a degenerate face.
    pub fn face_normal(&self, face: usize) -> [f32; 3] {
        let vs = self.face_vertices(face);
        let mut n = [0.0f32; 3];
        for i in 0..vs.len() {
            let a = self.positions[vs[i] as usize];
            let b = self.positions[vs[(i + 1) % vs.len()] as usize];
            n[0] += (a[1] - b[1]) * (a[2] + b[2]);
            n[1] += (a[2] - b[2]) * (a[0] + b[0]);
            n[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 0.0 {
            n.map(|c| c / len)
        } else {
            [0.0; 3]
        }
    }

    /// Signed volume of the surface (positive when faces point outward).
    /// Only meaningful for closed meshes.
    pub fn signed_volume(&self) -> f32 {
        let mut vol = 0.0;
        for f in 0..self.faces.len() {
            let vs = self.face_vertices(f);
            let p = |i: usize| self.positions[vs[i] as usize];
            for i in 1..vs.len() - 1 {
                vol += dot(p(0), cross(p(i), p(i + 1)));
            }
        }
        vol / 6.0
    }

    /// Adds a free-standing polygon (new vertices, no welding) and returns its face index.
    pub fn add_polygon(&mut self, points: &[[f32; 3]]) -> Result<usize, TopologyError> {
        if points.len() < 3 {
            return Err(TopologyError::DegenerateFace(self.face_count()));
        }
        let mut m = self.to_mesh();
        let base = m.positions.len() as u32;
        m.positions.extend_from_slice(points);
        m.faces.push((base..base + points.len() as u32).collect());
        *self = Self::from_mesh(&m)?;
        let face = self.face_count() - 1;
        if self.face_normal(face) == [0.0; 3] {
            return Err(TopologyError::DegenerateNormal);
        }
        Ok(face)
    }

    /// Removes a face (SketchUp's Eraser on a face). Its edges stay: as a
    /// neighbour's edges, or else as loose lines. Vertices are kept so indices stay valid.
    pub fn erase_face(&mut self, face: usize) -> Result<(), TopologyError> {
        if face >= self.faces.len() {
            return Err(TopologyError::FaceOutOfRange(face));
        }
        let mut m = self.to_mesh();
        let ring = m.faces.remove(face);
        let still: std::collections::HashSet<(u32, u32)> = m.edges().into_iter().collect();
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            if !still.contains(&(a.min(b), a.max(b))) {
                m.lines.push([a, b]);
            }
        }
        *self = Self::from_mesh(&m)?;
        Ok(())
    }

    /// The vertex at `p` (within `tol`), or a new one there: lines drawn from a corner share it.
    fn weld_vertex(&mut self, p: [f32; 3], tol: f32) -> u32 {
        let found = self.positions.iter().position(|q| (0..3).all(|i| (q[i] - p[i]).abs() <= tol));
        match found {
            Some(i) => i as u32,
            None => {
                self.positions.push(p);
                (self.positions.len() - 1) as u32
            }
        }
    }

    /// Line tool: a loose edge from `a` to `b`, sharing vertices with what is already there. When it closes a loop of loose
    /// lines that lie in one plane, the loop becomes a face, as in SketchUp; that face's index is returned. A line that is
    /// already there (or has no length) changes nothing.
    pub fn add_line(&mut self, a: [f32; 3], b: [f32; 3]) -> Result<Option<usize>, TopologyError> {
        let scale = a.iter().chain(b.iter()).fold(1.0f32, |m, c| m.max(c.abs()));
        let tol = 1e-5 * scale;
        let (ia, ib) = (self.weld_vertex(a, tol), self.weld_vertex(b, tol));
        if ia == ib {
            return Ok(None);
        }
        let key = (ia.min(ib), ia.max(ib));
        let exists = self.lines.iter().any(|&[x, y]| (x.min(y), x.max(y)) == key) || self.to_mesh().edges().binary_search(&key).is_ok();
        if exists {
            return Ok(None);
        }
        self.lines.push([ia, ib]);
        Ok(self.close_loop(ia, ib, tol * 10.0))
    }

    /// Erases loose line `i` (the Eraser on a line). Its vertices stay so indices stay valid.
    pub fn erase_line(&mut self, i: usize) -> Result<(), TopologyError> {
        if i >= self.lines.len() {
            return Err(TopologyError::FaceOutOfRange(i));
        }
        self.lines.remove(i);
        Ok(())
    }

    /// After the line `a`-`b` was added: the shortest path of other loose lines from `b` back to `a`, if it lies in one
    /// plane with at least three corners, becomes a face (its lines turn into the face's edges).
    fn close_loop(&mut self, a: u32, b: u32, tol: f32) -> Option<usize> {
        let new_line = self.lines.len() - 1;
        let mut adjacent: HashMap<u32, Vec<(u32, usize)>> = HashMap::new();
        for (i, &[x, y]) in self.lines.iter().enumerate() {
            if i != new_line {
                adjacent.entry(x).or_default().push((y, i));
                adjacent.entry(y).or_default().push((x, i));
            }
        }
        // breadth first from b to a
        let mut came: HashMap<u32, (u32, usize)> = HashMap::new();
        let mut queue = std::collections::VecDeque::from([b]);
        came.insert(b, (b, usize::MAX));
        while let Some(v) = queue.pop_front() {
            if v == a {
                break;
            }
            for &(w, line) in adjacent.get(&v).map(|n| n.as_slice()).unwrap_or(&[]) {
                if !came.contains_key(&w) {
                    came.insert(w, (v, line));
                    queue.push_back(w);
                }
            }
        }
        if !came.contains_key(&a) {
            return None;
        }
        let (mut ring, mut used) = (vec![a], vec![new_line]);
        let mut v = a;
        while v != b {
            let (prev, line) = came[&v];
            ring.push(prev);
            used.push(line);
            v = prev;
        }
        if ring.len() < 3 {
            return None;
        }
        // one plane: every corner within tol of the plane through the first, by the ring's normal
        let pts: Vec<[f32; 3]> = ring.iter().map(|&v| self.positions[v as usize]).collect();
        let mut n = [0.0f32; 3];
        for i in 0..pts.len() {
            let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
            n[0] += (p[1] - q[1]) * (p[2] + q[2]);
            n[1] += (p[2] - q[2]) * (p[0] + q[0]);
            n[2] += (p[0] - q[0]) * (p[1] + q[1]);
        }
        let len = dot(n, n).sqrt();
        if len < 1e-12 {
            return None;
        }
        let n = n.map(|c| c / len);
        if pts.iter().any(|&p| dot(sub(p, pts[0]), n).abs() > tol.max(1e-6)) {
            return None;
        }
        // which way it faces: up if it lies flat, else toward the front (-Y), else toward +X
        let want = if n[2].abs() > 0.1 { n[2] > 0.0 } else if n[1].abs() > 0.1 { n[1] < 0.0 } else { n[0] > 0.0 };
        if !want {
            ring.reverse();
        }
        used.sort_unstable_by(|x, y| y.cmp(x));
        let mut m = self.to_mesh();
        for &i in &used {
            m.lines.remove(i);
        }
        // the other winding when this one runs the same way as a neighbouring face's edge
        for attempt in 0..2 {
            let mut trial = m.clone();
            let face: Vec<u32> = if attempt == 0 { ring.clone() } else { ring.iter().rev().copied().collect() };
            trial.faces.push(face);
            if let Ok(h) = Self::from_mesh(&trial) {
                *self = h;
                return Some(self.face_count() - 1);
            }
        }
        None
    }

    /// Extrudes `face` along its normal by `distance` (negative pushes inward).
    ///
    /// * A face that borders other faces (part of a solid) is replaced by side
    ///   walls and a moved copy, so a closed solid stays closed.
    /// * A free-standing face stays behind as the base and becomes a solid.
    ///
    /// Face indices are not stable across this call; the new far face is returned.
    pub fn push_pull(&mut self, face: usize, distance: f32) -> Result<usize, TopologyError> {
        if face >= self.faces.len() {
            return Err(TopologyError::FaceOutOfRange(face));
        }
        if distance == 0.0 {
            return Err(TopologyError::ZeroDistance);
        }
        let n = self.face_normal(face);
        if n == [0.0; 3] {
            return Err(TopologyError::DegenerateNormal);
        }
        let attached = self.face_loop(face).iter().any(|&h| self.half_edges[h as usize].twin != NONE);
        let ring = self.face_vertices(face);

        let mut m = self.to_mesh();
        let top: Vec<u32> = ring
            .iter()
            .map(|&v| {
                let p = m.positions[v as usize];
                m.add_vertex([p[0] + n[0] * distance, p[1] + n[1] * distance, p[2] + n[2] * distance])
            })
            .collect();

        let reversed = |v: &[u32]| -> Vec<u32> { v.iter().rev().copied().collect() };
        // `outward` is true when the walls keep the original edge direction a->b.
        let (base, far, outward) = if attached {
            (None, top.clone(), true)
        } else if distance > 0.0 {
            (Some(reversed(&ring)), top.clone(), true)
        } else {
            (Some(ring.clone()), reversed(&top), false)
        };

        m.faces.remove(face);
        if let Some(b) = base {
            m.faces.push(b);
        }
        let n_ring = ring.len();
        for i in 0..n_ring {
            let j = (i + 1) % n_ring;
            let (a, b, a2, b2) = (ring[i], ring[j], top[i], top[j]);
            m.faces.push(if outward { vec![a, b, b2, a2] } else { vec![b, a, a2, b2] });
        }
        m.faces.push(far);

        *self = Self::from_mesh(&m)?;
        Ok(self.face_count() - 1)
    }

    /// Nearest face hit by a ray, as (face index, distance along the ray).
    /// Faces are treated as two-sided.
    pub fn pick_face(&self, origin: [f32; 3], dir: [f32; 3]) -> Option<(usize, f32)> {
        self.to_mesh().pick_face(origin, dir)
    }

    /// Splits `face` along the chord between vertices `a` and `b`, which must be
    /// distinct, non-adjacent corners of the face. The original face keeps the
    /// `a..b` side; the returned index is the new face on the `b..a` side.
    pub fn split_face(&mut self, face: usize, a: u32, b: u32) -> Result<usize, TopologyError> {
        let lp = self.face_loop(face);
        let n = lp.len();
        let pos_a = lp.iter().position(|&h| self.half_edges[h as usize].origin == a);
        let pos_b = lp.iter().position(|&h| self.half_edges[h as usize].origin == b);
        let (Some(ia), Some(ib)) = (pos_a, pos_b) else {
            return Err(TopologyError::InvalidSplit);
        };
        let adjacent = (ia + 1) % n == ib || (ib + 1) % n == ia;
        if ia == ib || adjacent {
            return Err(TopologyError::InvalidSplit);
        }

        let he_a = lp[ia];
        let he_b = lp[ib];
        let last_before_b = lp[(ib + n - 1) % n]; // ends at b, on the a..b side
        let last_before_a = lp[(ia + n - 1) % n]; // ends at a, on the b..a side

        let new_face = self.faces.len() as u32;
        let chord_ba = self.half_edges.len() as u32; // b -> a, closes the a..b side
        let chord_ab = chord_ba + 1; // a -> b, closes the b..a side

        self.half_edges.push(HalfEdge { origin: b, next: he_a, twin: chord_ab, face: face as u32 });
        self.half_edges.push(HalfEdge { origin: a, next: he_b, twin: chord_ba, face: new_face });
        self.half_edges[last_before_b as usize].next = chord_ba;
        self.half_edges[last_before_a as usize].next = chord_ab;

        // Reassign the b..a side to the new face.
        let mut h = he_b;
        while h != chord_ab {
            self.half_edges[h as usize].face = new_face;
            h = self.half_edges[h as usize].next;
        }
        self.faces[face] = he_a;
        self.faces.push(he_b);
        Ok(new_face as usize)
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Moller-Trumbore ray/triangle intersection, two-sided. Returns the ray parameter.
pub(crate) fn ray_triangle(origin: [f32; 3], dir: [f32; 3], tri: [[f32; 3]; 3]) -> Option<f32> {
    let e1 = sub(tri[1], tri[0]);
    let e2 = sub(tri[2], tri[0]);
    let p = cross(dir, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let t0 = sub(origin, tri[0]);
    let u = dot(t0, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(t0, e1);
    let v = dot(dir, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    (t > 1e-6).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Mesh {
        let mut m = Mesh::new();
        for z in [0.0, 1.0] {
            for y in [0.0, 1.0] {
                for x in [0.0, 1.0] {
                    m.add_vertex([x, y, z]);
                }
            }
        }
        // Outward-facing, counter-clockwise quads.
        for f in [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ] {
            m.add_face(f.to_vec());
        }
        m
    }

    #[test]
    fn cube_is_a_closed_sphere() {
        let h = HalfEdgeMesh::from_mesh(&cube()).unwrap();
        assert!(h.is_closed());
        assert_eq!(h.edge_count(), 12);
        assert_eq!(h.euler_characteristic(), 2);
        let n = h.face_normal(0);
        assert!((n[2] + 1.0).abs() < 1e-6, "bottom face should point -Z, got {n:?}");
    }

    #[test]
    fn round_trips_through_mesh() {
        let c = cube();
        assert_eq!(HalfEdgeMesh::from_mesh(&c).unwrap().to_mesh(), c);
    }

    #[test]
    fn rejects_inconsistent_winding() {
        let mut c = cube();
        c.faces[1] = vec![4, 6, 7, 5]; // flipped: now shares directed edges with a neighbour
        c.faces[2] = vec![0, 1, 5, 4];
        c.faces[0] = vec![0, 1, 3, 2]; // flipped
        assert!(matches!(
            HalfEdgeMesh::from_mesh(&c),
            Err(TopologyError::NonManifoldEdge { .. })
        ));
    }

    #[test]
    fn split_quad_into_two_triangles() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        let mut h = HalfEdgeMesh::from_mesh(&m).unwrap();
        let new = h.split_face(0, 0, 2).unwrap();

        assert_eq!(h.face_count(), 2);
        assert_eq!(h.face_vertices(0), vec![0, 1, 2]);
        assert_eq!(h.face_vertices(new), vec![2, 3, 0]);
        assert_eq!(h.edge_count(), 5);
        assert_eq!(h.boundary_edge_count(), 4);
        assert_eq!(h.euler_characteristic(), 1); // a disc
        for f in 0..2 {
            assert!((h.face_normal(f)[2] - 1.0).abs() < 1e-6);
        }
    }

    fn square() -> HalfEdgeMesh {
        let mut h = HalfEdgeMesh::default();
        h.add_polygon(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]).unwrap();
        h
    }

    fn assert_solid(h: &HalfEdgeMesh, volume: f32) {
        assert!(h.is_closed(), "mesh should be closed");
        assert_eq!(h.euler_characteristic(), 2);
        assert!((h.signed_volume() - volume).abs() < 1e-5, "volume {} != {volume}", h.signed_volume());
    }

    #[test]
    fn push_free_square_up_makes_a_box() {
        let mut h = square();
        h.push_pull(0, 2.0).unwrap();
        assert_eq!(h.face_count(), 6);
        assert_solid(&h, 2.0);
    }

    #[test]
    fn push_free_square_down_makes_a_box() {
        let mut h = square();
        h.push_pull(0, -2.0).unwrap();
        assert_solid(&h, 2.0);
    }

    #[test]
    fn push_cube_top_out_and_in() {
        let top = 1; // [4,5,7,6], the z = 1 face
        let mut out = HalfEdgeMesh::from_mesh(&cube()).unwrap();
        assert!((out.signed_volume() - 1.0).abs() < 1e-5);
        out.push_pull(top, 1.0).unwrap();
        assert_solid(&out, 2.0);

        let mut inward = HalfEdgeMesh::from_mesh(&cube()).unwrap();
        inward.push_pull(top, -0.5).unwrap();
        assert_solid(&inward, 0.5);
    }

    #[test]
    fn push_pull_errors() {
        let mut h = square();
        assert_eq!(h.push_pull(5, 1.0), Err(TopologyError::FaceOutOfRange(5)));
        assert_eq!(h.push_pull(0, 0.0), Err(TopologyError::ZeroDistance));
    }

    #[test]
    fn picks_nearest_face_from_both_sides() {
        let h = HalfEdgeMesh::from_mesh(&cube()).unwrap();
        let (face, t) = h.pick_face([0.5, 0.5, 5.0], [0.0, 0.0, -1.0]).unwrap();
        assert_eq!(face, 1);
        assert!((t - 4.0).abs() < 1e-5);
        let (face, _) = h.pick_face([0.5, 0.5, -5.0], [0.0, 0.0, 1.0]).unwrap();
        assert_eq!(face, 0);
        assert!(h.pick_face([5.0, 5.0, 5.0], [0.0, 0.0, -1.0]).is_none());
    }

    #[test]
    fn edges_and_erase() {
        let mut h = HalfEdgeMesh::from_mesh(&cube()).unwrap();
        h.erase_face(1).unwrap();
        assert_eq!(h.face_count(), 5);
        assert_eq!(h.boundary_edge_count(), 4, "open box");
        assert_eq!(h.edge_count(), 12, "the rim edges stay");
        assert_eq!(h.erase_face(9), Err(TopologyError::FaceOutOfRange(9)));
    }

    #[test]
    fn add_polygon_rejects_degenerate_input() {
        let mut h = HalfEdgeMesh::default();
        assert!(h.add_polygon(&[[0.0; 3], [1.0, 0.0, 0.0]]).is_err());
        let line = [[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        assert_eq!(h.add_polygon(&line), Err(TopologyError::DegenerateNormal));
    }

    #[test]
    fn picks_inside_the_notch_of_a_concave_face_correctly() {
        let mut h = HalfEdgeMesh::default();
        // L-shape: 2 x 2 with the top-right quarter missing.
        let l = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 2.0, 0.0]];
        h.add_polygon(&l).unwrap();
        assert!(h.pick_face([0.5, 1.5, 1.0], [0.0, 0.0, -1.0]).is_some());
        assert!(h.pick_face([1.5, 1.5, 1.0], [0.0, 0.0, -1.0]).is_none(), "the missing quarter");
    }

    #[test]
    fn split_rejects_adjacent_vertices() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        let mut h = HalfEdgeMesh::from_mesh(&m).unwrap();
        assert_eq!(h.split_face(0, 0, 1), Err(TopologyError::InvalidSplit));
    }
}
