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
    pub positions: Vec<[f64; 3]>,
    half_edges: Vec<HalfEdge>,
    /// One half-edge per face.
    faces: Vec<u32>,
    /// Per face, one half-edge of each hole's loop (a face with openings, like a wall with a window).
    holes: Vec<Vec<u32>>,
    /// Loose edges (lines that are no face's edge), as vertex pairs.
    pub lines: Vec<[u32; 2]>,
    /// Hidden face edges (seams around holes).
    pub seams: Vec<[u32; 2]>,
    /// Face colours (see `Mesh::colors`).
    pub colors: Vec<Option<crate::Paint>>,
}

impl HalfEdgeMesh {
    pub fn from_mesh(mesh: &Mesh) -> Result<Self, TopologyError> {
        let mut he = HalfEdgeMesh { positions: mesh.positions.clone(), ..Default::default() };
        if let Some(&[a, b]) = mesh.lines.iter().find(|l| l.iter().any(|&v| v as usize >= mesh.positions.len())) {
            return Err(TopologyError::VertexOutOfRange(a.max(b)));
        }
        he.lines = mesh.lines.clone();
        he.colors = mesh.colors.iter().copied().take(mesh.faces.len()).collect();
        while he.colors.last() == Some(&None) {
            he.colors.pop();
        }
        // a seam is kept only while it is still some face's edge
        let face_edges: std::collections::HashSet<(u32, u32)> = mesh.edges().into_iter().collect();
        he.seams = mesh.seams.iter().copied().filter(|&[a, b]| face_edges.contains(&(a.min(b), a.max(b)))).collect();
        let mut directed: HashMap<(u32, u32), u32> = HashMap::new();

        for fi in 0..mesh.faces.len() {
            let mut starts = Vec::new();
            for face in mesh.loops(fi) {
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
                    he.half_edges.push(HalfEdge { origin, next: base + (k as u32 + 1) % n, twin: NONE, face: fi as u32 });
                }
                starts.push(base);
            }
            he.faces.push(starts[0]);
            he.holes.push(starts[1..].to_vec());
        }

        for (&(a, b), &id) in &directed {
            if let Some(&t) = directed.get(&(b, a)) {
                he.half_edges[id as usize].twin = t;
            }
        }
        Ok(he)
    }

    pub fn to_mesh(&self) -> Mesh {
        let mut m = Mesh { positions: self.positions.clone(), faces: Vec::new(), lines: self.lines.clone(), seams: self.seams.clone(), colors: self.colors.clone(), holes: Vec::new() };
        for f in 0..self.faces.len() {
            m.faces.push(self.face_vertices(f));
            m.set_holes(f, self.face_holes(f));
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

    /// V - E + F - H (H: holes in faces), counting only vertices that belong to a face: 2 for a closed solid without a
    /// tunnel through it (Euler-Poincare), whatever openings its faces have.
    pub fn euler_characteristic(&self) -> i64 {
        let mut used = vec![false; self.positions.len()];
        for h in &self.half_edges {
            used[h.origin as usize] = true;
        }
        let v = used.iter().filter(|&&u| u).count() as i64;
        let holes = self.holes.iter().map(Vec::len).sum::<usize>() as i64;
        v - self.edge_count() as i64 + self.face_count() as i64 - holes
    }

    pub fn face_vertices(&self, face: usize) -> Vec<u32> {
        self.face_loop(face).into_iter().map(|h| self.half_edges[h as usize].origin).collect()
    }

    /// The openings in `face`, as vertex loops (wound against its outline).
    pub fn face_holes(&self, face: usize) -> Vec<Vec<u32>> {
        self.holes.get(face).map_or(Vec::new(), |hs| hs.iter().map(|&s| self.walk(s).into_iter().map(|h| self.half_edges[h as usize].origin).collect()).collect())
    }

    /// Whether `face` has openings.
    pub fn has_holes(&self, face: usize) -> bool {
        self.holes.get(face).is_some_and(|h| !h.is_empty())
    }

    fn face_loop(&self, face: usize) -> Vec<u32> {
        self.walk(self.faces[face])
    }

    /// Every loop of `face`'s half-edges: the outline, then the holes.
    fn all_loops(&self, face: usize) -> Vec<Vec<u32>> {
        std::iter::once(self.faces[face]).chain(self.holes.get(face).into_iter().flatten().copied()).map(|s| self.walk(s)).collect()
    }

    /// The half-edges of one loop, from `start`.
    fn walk(&self, start: u32) -> Vec<u32> {
        let mut out = vec![start];
        let mut h = self.half_edges[start as usize].next;
        while h != start {
            out.push(h);
            h = self.half_edges[h as usize].next;
        }
        out
    }

    /// Unit normal by Newell's method; zero vector for a degenerate face.
    pub fn face_normal(&self, face: usize) -> [f64; 3] {
        let vs = self.face_vertices(face);
        let mut n = [0.0f64; 3];
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
    pub fn signed_volume(&self) -> f64 {
        // each loop's fan: a hole, wound the other way, takes its part off again
        let mut vol = 0.0;
        for f in 0..self.faces.len() {
            for lp in self.all_loops(f) {
                let vs: Vec<u32> = lp.into_iter().map(|h| self.half_edges[h as usize].origin).collect();
                let p = |i: usize| self.positions[vs[i] as usize];
                for i in 1..vs.len() - 1 {
                    vol += dot(p(0), cross(p(i), p(i + 1)));
                }
            }
        }
        vol / 6.0
    }

    /// Adds a free-standing polygon (new vertices, no welding) and returns its face index.
    pub fn add_polygon(&mut self, points: &[[f64; 3]]) -> Result<usize, TopologyError> {
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
        let (ring, holes) = m.remove_face_loops(face);
        let still: std::collections::HashSet<(u32, u32)> = m.edges().into_iter().collect();
        for r in std::iter::once(&ring).chain(&holes) {
          for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            if !still.contains(&(a.min(b), a.max(b))) {
                m.lines.push([a, b]);
            }
          }
        }
        *self = Self::from_mesh(&m)?;
        Ok(())
    }

    /// Line tool: a line from `a` to `b` drawn into what is there (see [`crate::imprint`]): it joins the corners and edges it
    /// meets, splits a face it crosses from edge to edge, and is otherwise a loose line. A closed loop it completes, of
    /// loose lines and face edges lying in one plane, becomes a face, as in SketchUp. Returns the face that was made or
    /// split off, if any. A line that is already there (or has no length) changes nothing.
    pub fn add_line(&mut self, a: [f64; 3], b: [f64; 3]) -> Result<Option<usize>, TopologyError> {
        let mut m = self.to_mesh();
        let tol = line_tolerance(m.tolerance(), a, b);
        let (added, split) = crate::imprint::add_segment(&mut m, a, b, tol);
        *self = Self::from_mesh(&m)?;
        let mut made = split.then(|| self.face_count() - 1);
        // each new loose line may close a loop; later ones first, as closing removes lines and shifts the numbers
        for &line in added.iter().rev() {
            if line < self.lines.len() {
                if let Some(f) = self.close_loop(line, tol * 10.0) {
                    made = Some(f);
                }
            }
        }
        Ok(made)
    }

    /// Erases loose line `i` (the Eraser on a line). Its vertices stay so indices stay valid.
    pub fn erase_line(&mut self, i: usize) -> Result<(), TopologyError> {
        if i >= self.lines.len() {
            return Err(TopologyError::FaceOutOfRange(i));
        }
        self.lines.remove(i);
        Ok(())
    }

    /// Erases edges, given as vertex pairs (Delete on selected edges), as SketchUp does: a loose line goes; an edge between
    /// two faces lying in one plane joins them into one face; any other edge takes the faces on it with it (and the faces
    /// joined to those by hidden seams, which are one face with a hole), their other edges staying as loose lines where no
    /// face is left. Returns how many edges were kept because the two faces either side could not become one simple face.
    /// Vertices are kept so indices stay valid.
    pub fn erase_edges(&mut self, edges: &[[u32; 2]]) -> Result<usize, TopologyError> {
        use std::collections::HashSet;
        let key = |a: u32, b: u32| (a.min(b), a.max(b));
        let mut gone: Vec<(u32, u32)> = edges.iter().map(|&[a, b]| key(a, b)).collect();
        gone.sort_unstable();
        gone.dedup();
        let gone_set: HashSet<(u32, u32)> = gone.iter().copied().collect();
        let mut m = self.to_mesh();
        let tol = m.tolerance();
        m.lines.retain(|&[a, b]| !gone_set.contains(&key(a, b)));
        let seams: HashSet<(u32, u32)> = m.seams.iter().map(|&[a, b]| key(a, b)).collect();
        let on_edge = |m: &Mesh, e: (u32, u32)| -> Vec<usize> {
            (0..m.faces.len()).filter(|&f| m.loops(f).any(|r| (0..r.len()).any(|i| key(r[i], r[(i + 1) % r.len()]) == e))).collect()
        };
        let mut kept = 0;
        for &e in &gone {
            let faces = on_edge(&m, e);
            if faces.is_empty() {
                continue;
            }
            if let [a, b] = faces[..] {
                let joined = crate::merge::join_pair(&m, a, b, tol);
                if let Some(ring) = joined {
                    let (lo, hi) = (a.min(b), a.max(b));
                    let (_, theirs) = m.remove_face_loops(hi);
                    m.faces[lo] = ring;
                    let mut holes = m.holes_of(lo).to_vec();
                    holes.extend(theirs);
                    m.set_holes(lo, holes);
                    continue;
                }
                // in one plane but not one simple face (it would have a hole): left as it is, rather than lose both
                if crate::merge::coplanar(&m, a, b, tol) {
                    kept += 1;
                    continue;
                }
            }
            // the faces on it go, with the faces seamed to them
            let mut take: HashSet<usize> = faces.into_iter().collect();
            loop {
                let more: Vec<usize> = (0..m.faces.len())
                    .filter(|f| !take.contains(f))
                    .filter(|&f| {
                        let r = &m.faces[f];
                        (0..r.len()).any(|i| {
                            let s = key(r[i], r[(i + 1) % r.len()]);
                            seams.contains(&s) && take.iter().any(|&t| { let q = &m.faces[t]; (0..q.len()).any(|j| key(q[j], q[(j + 1) % q.len()]) == s) })
                        })
                    })
                    .collect();
                if more.is_empty() {
                    break;
                }
                take.extend(more);
            }
            let mut take: Vec<usize> = take.into_iter().collect();
            take.sort_unstable_by(|x, y| y.cmp(x));
            let rings: Vec<Vec<u32>> = take
                .iter()
                .flat_map(|&f| {
                    let (outline, holes) = m.remove_face_loops(f);
                    std::iter::once(outline).chain(holes)
                })
                .collect();
            let still: HashSet<(u32, u32)> = m.edges().into_iter().collect();
            let mut lines: HashSet<(u32, u32)> = m.lines.iter().map(|&[a, b]| key(a, b)).collect();
            for r in rings {
                for i in 0..r.len() {
                    let s = key(r[i], r[(i + 1) % r.len()]);
                    if !still.contains(&s) && !seams.contains(&s) && !gone_set.contains(&s) && lines.insert(s) {
                        m.lines.push([s.0, s.1]);
                    }
                }
            }
        }
        let face_edges: HashSet<(u32, u32)> = m.edges().into_iter().collect();
        m.seams.retain(|&[a, b]| face_edges.contains(&key(a, b)));
        *self = Self::from_mesh(&m)?;
        Ok(kept)
    }

    /// Loose line number `new_line` may close a loop: the shortest path of other loose lines and face edges from one of
    /// its ends back to the other, if it lies in one plane with at least three corners and covers no face already there,
    /// becomes a face (its loose lines turn into the face's edges). What the loop becomes is [`closing`]'s choice.
    pub(crate) fn close_loop(&mut self, new_line: usize, tol: f64) -> Option<usize> {
        let [a, b] = self.lines[new_line];
        // a loop can only close where both ends already meet something (another loose line or a face): drawing an outline
        // line by line, that is every line but the last, and skipping the search keeps a long outline from taking seconds
        // (each search lists and sorts every edge)
        let meets = |v: u32| {
            self.lines.iter().enumerate().any(|(i, l)| i != new_line && (l[0] == v || l[1] == v))
                || self.half_edges.iter().any(|h| h.origin == v)
        };
        if !meets(a) || !meets(b) {
            return None;
        }
        let mesh = self.to_mesh();
        let mut adjacent: HashMap<u32, Vec<(u32, usize)>> = HashMap::new();
        for (i, &[x, y]) in self.lines.iter().enumerate() {
            if i != new_line {
                adjacent.entry(x).or_default().push((y, i));
                adjacent.entry(y).or_default().push((x, i));
            }
        }
        // face edges can close a loop too (usize::MAX marks them: not a loose line)
        for (x, y) in mesh.edges() {
            adjacent.entry(x).or_default().push((y, usize::MAX));
            adjacent.entry(y).or_default().push((x, usize::MAX));
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
                if let std::collections::hash_map::Entry::Vacant(e) = came.entry(w) {
                    e.insert((v, line));
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
            if line != usize::MAX {
                used.push(line);
            }
            v = prev;
        }
        let plan = closing(&mesh, ring, tol)?;
        used.sort_unstable_by(|x, y| y.cmp(x));
        let mut m = self.to_mesh();
        for &i in &used {
            m.lines.remove(i);
        }
        match plan {
            // drawn inside a face (a rectangle on a box's top): the face gets a hole there, filled by the new face
            Closing::Hole { host, ring } => {
                let inner = crate::imprint::cut_hole(&mut m, host, &ring)?;
                let h = Self::from_mesh(&m).ok()?;
                *self = h;
                Some(inner)
            }
            Closing::Face(ring) => {
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
        }
    }

    /// Extrudes `face` along its normal by `distance` (negative pushes inward).
    ///
    /// * A face that borders other faces (part of a solid) is replaced by side
    ///   walls and a moved copy, so a closed solid stays closed.
    /// * A free-standing face stays behind as the base and becomes a solid.
    ///
    /// Face indices are not stable across this call; the new far face is returned.
    pub fn push_pull(&mut self, face: usize, distance: f64) -> Result<usize, TopologyError> {
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
        let attached = self.all_loops(face).iter().flatten().any(|&h| self.half_edges[h as usize].twin != NONE);
        // the outline and the holes: each gets walls (a hole's run round the opening, facing into it)
        let loops: Vec<Vec<u32>> = std::iter::once(self.face_vertices(face)).chain(self.face_holes(face)).collect();

        let mut m = self.to_mesh();
        let tops: Vec<Vec<u32>> = loops
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|&v| {
                        let p = m.positions[v as usize];
                        m.add_vertex([p[0] + n[0] * distance, p[1] + n[1] * distance, p[2] + n[2] * distance])
                    })
                    .collect()
            })
            .collect();

        let reversed = |v: &[u32]| -> Vec<u32> { v.iter().rev().copied().collect() };
        // `outward` is true when the walls keep the original edge direction a->b.
        let (base, far, outward) = if attached {
            (None, tops.clone(), true)
        } else if distance > 0.0 {
            (Some(loops.iter().map(|r| reversed(r)).collect::<Vec<_>>()), tops.clone(), true)
        } else {
            (Some(loops.clone()), tops.iter().map(|r| reversed(r)).collect(), false)
        };

        // the pushed face keeps its colour (and so does the base it leaves behind); the new side walls start unpainted
        let color = m.color_of(face);
        m.remove_face(face);
        let put = |m: &mut Mesh, mut rings: Vec<Vec<u32>>, color: Option<crate::Paint>| {
            let outline = rings.remove(0);
            m.faces.push(outline);
            let f = m.faces.len() - 1;
            m.set_color(f, color);
            m.set_holes(f, rings);
        };
        if let Some(b) = base {
            put(&mut m, b, color);
        }
        for (ring, top) in loops.iter().zip(&tops) {
            let n_ring = ring.len();
            for i in 0..n_ring {
                let j = (i + 1) % n_ring;
                let (a, b, a2, b2) = (ring[i], ring[j], top[i], top[j]);
                m.faces.push(if outward { vec![a, b, b2, a2] } else { vec![b, a, a2, b2] });
            }
        }
        put(&mut m, far, color);

        *self = Self::from_mesh(&m)?;
        Ok(self.face_count() - 1)
    }

    /// Nearest face hit by a ray, as (face index, distance along the ray).
    /// Faces are treated as two-sided.
    pub fn pick_face(&self, origin: [f64; 3], dir: [f64; 3]) -> Option<(usize, f64)> {
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
        self.holes.push(Vec::new());
        // each hole goes with the half it lies in
        let holes = std::mem::take(&mut self.holes[face]);
        if !holes.is_empty() {
            let normal = self.face_normal(face);
            let outline_new: Vec<[f64; 3]> = self.face_vertices(new_face as usize).iter().map(|&v| self.positions[v as usize]).collect();
            for start in holes {
                let p = self.positions[self.half_edges[start as usize].origin as usize];
                let owner = if point_in_ring(&outline_new, p, normal) { new_face } else { face as u32 };
                for h in self.walk(start) {
                    self.half_edges[h as usize].face = owner;
                }
                self.holes[owner as usize].push(start);
            }
        }
        // the new half keeps the face's colour
        if let Some(&c @ Some(_)) = self.colors.get(face) {
            self.colors.resize(new_face as usize + 1, None);
            self.colors[new_face as usize] = c;
        }
        Ok(new_face as usize)
    }
}

/// How close a new line's ends must come to what is there to join it (`HalfEdgeMesh::add_line`): the model's own
/// tolerance (`model`, [`Mesh::tolerance`]), with the new line in it (a first line on an empty model has only itself).
pub(crate) fn line_tolerance(model: f64, a: [f64; 3], b: [f64; 3]) -> f64 {
    let span = (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt();
    let reach = a.iter().chain(b.iter()).fold(0.0f64, |r, c| r.max(c.abs()));
    model.max(span * 1e-7).max(reach * 1e-12).max(1e-10)
}

/// What a closed loop of lines becomes (see [`closing`]).
pub(crate) enum Closing {
    /// A new face, wound as given.
    Face(Vec<u32>),
    /// A hole cut in face `host` and a new face filling it (`imprint::cut_hole`), wound as given.
    Hole { host: usize, ring: Vec<u32> },
}

/// What the loop `ring` (corners, from the new line's first end back round to its other end, as `close_loop` finds it) makes
/// in `mesh`: nothing when it has under three corners, does not lie in one plane (within `tol`) or overlaps a face's edge
/// without meeting it; a hole in the face it lies on (wound as that face is); else a face of its own, facing up if it lies
/// flat, else toward the front (-Y), else toward +X.
pub(crate) fn closing(mesh: &Mesh, mut ring: Vec<u32>, tol: f64) -> Option<Closing> {
    if ring.len() < 3 {
        return None;
    }
    // one plane: every corner within tol of the plane through the first, by the ring's normal
    let pts: Vec<[f64; 3]> = ring.iter().map(|&v| mesh.positions[v as usize]).collect();
    let mut n = [0.0f64; 3];
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
    if pts.iter().any(|&p| dot(sub(p, pts[0]), n).abs() > tol) {
        return None;
    }
    // not over a face already there: the middle of the new face (a point just inside its first corner) is in none
    let probe = {
        let k = pts.len();
        let (p0, p1, pl) = (pts[0], pts[1], pts[k - 1]);
        [(p0[0] + p1[0] + pl[0]) / 3.0, (p0[1] + p1[1] + pl[1]) / 3.0, (p0[2] + p1[2] + pl[2]) / 3.0]
    };
    let host = (0..mesh.faces.len()).find(|&f| crate::imprint::on_face(mesh, f, probe, tol));

    // which way it faces: up if it lies flat, else toward the front (-Y), else toward +X
    let want = if n[2].abs() > 0.1 { n[2] > 0.0 } else if n[1].abs() > 0.1 { n[1] < 0.0 } else { n[0] > 0.0 };
    if !want {
        ring.reverse();
    }
    // drawn inside a face (a rectangle on a box's top): the face gets a hole there, filled by the new face
    if let Some(f) = host {
        let all_inside = crate::imprint::on_face_all(mesh, f, &pts, tol);
        if !all_inside {
            return None; // it overlaps the face's edge without meeting it: leave the lines
        }
        let (hn, _) = crate::imprint::face_plane(mesh, &mesh.faces[f])?;
        // the ring's normal now (it may have been turned round above), to wind as the face around it does
        let now = if want { n } else { n.map(|c| -c) };
        if dot(hn, now) < 0.0 {
            ring.reverse();
        }
        return Some(Closing::Hole { host: f, ring });
    }
    Some(Closing::Face(ring))
}

/// Whether `p` lies inside the polygon `ring` (in the plane with this normal), seen along the normal's main axis.
fn point_in_ring(ring: &[[f64; 3]], p: [f64; 3], normal: [f64; 3]) -> bool {
    let a = normal.map(f64::abs);
    let (x, y) = if a[0] >= a[1] && a[0] >= a[2] { (1, 2) } else if a[1] >= a[2] { (2, 0) } else { (0, 1) };
    let mut inside = false;
    for i in 0..ring.len() {
        let (u, v) = (ring[i], ring[(i + 1) % ring.len()]);
        if (u[y] > p[y]) != (v[y] > p[y]) && p[x] < u[x] + (p[y] - u[y]) * (v[x] - u[x]) / (v[y] - u[y]) {
            inside = !inside;
        }
    }
    inside
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Moller-Trumbore ray/triangle intersection, two-sided. Returns the ray parameter.
pub fn ray_triangle(origin: [f64; 3], dir: [f64; 3], tri: [[f64; 3]; 3]) -> Option<f64> {
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

    fn assert_solid(h: &HalfEdgeMesh, volume: f64) {
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

    #[test]
    fn erasing_the_edge_between_two_faces_in_one_plane_joins_them() {
        // a 2x1 rectangle split down the middle: two squares sharing the edge 1-4
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 4, 5]);
        m.add_face(vec![1, 2, 3, 4]);
        let mut h = HalfEdgeMesh::from_mesh(&m).unwrap();
        assert_eq!(h.erase_edges(&[[4, 1]]), Ok(0));
        assert_eq!(h.face_count(), 1);
        assert!(h.lines.is_empty());
        let mut ring = h.face_vertices(0);
        ring.sort_unstable();
        assert_eq!(ring, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn erasing_an_edge_of_a_box_takes_the_two_faces_on_it() {
        let mut h = HalfEdgeMesh::from_mesh(&cube()).unwrap();
        // the edge 0-1 is on the bottom (0 2 3 1) and the front (0 1 5 4)
        assert_eq!(h.erase_edges(&[[0, 1]]), Ok(0));
        assert_eq!(h.face_count(), 4);
        // every other edge of the two faces still borders a face that is left: no loose lines
        assert!(h.lines.is_empty());
        assert!(!h.to_mesh().edges().contains(&(0, 1)));
    }

    #[test]
    fn erasing_the_outline_edge_of_a_lone_face_leaves_its_other_edges_as_lines() {
        let mut h = square();
        assert_eq!(h.erase_edges(&[[0, 1]]), Ok(0));
        assert_eq!(h.face_count(), 0);
        let mut lines: Vec<(u32, u32)> = h.lines.iter().map(|&[a, b]| (a.min(b), a.max(b))).collect();
        lines.sort_unstable();
        assert_eq!(lines, vec![(0, 3), (1, 2), (2, 3)]);
        // and a loose line goes when erased
        assert_eq!(h.erase_edges(&[[3, 2]]), Ok(0));
        assert_eq!(h.lines.len(), 2);
    }

    #[test]
    fn small_models_keep_detail_a_micrometre_apart() {
        // a 2 mm square, split by a line 1 um in from its edge: the old fixed 10 um tolerance joined that line to the edge
        let mut h = HalfEdgeMesh::default();
        h.add_polygon(&[[0.0, 0.0, 0.0], [0.002, 0.0, 0.0], [0.002, 0.002, 0.0], [0.0, 0.002, 0.0]]).unwrap();
        let made = h.add_line([0.000001, 0.0, 0.0], [0.000001, 0.002, 0.0]).unwrap();
        assert!(made.is_some(), "the line splits the face");
        assert_eq!(h.face_count(), 2);
        let mesh = h.to_mesh();
        assert!(mesh.tolerance() < 1e-9, "a 2 mm part's tolerance is under a nanometre: {}", mesh.tolerance());
    }
}
